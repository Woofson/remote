use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ssh2::{Session, Sftp};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SftpFileInfo {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub modified: u64,
    pub permissions: u32,
}

pub struct SshConnectionParams {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: Option<String>,
    pub private_key: Option<String>,
    pub certificate: Option<String>,
    pub passphrase: Option<String>,
}

enum SshChannelCommand {
    Data(Vec<u8>),
    Resize(u32, u32),
}

pub fn create_ssh_session(params: &SshConnectionParams) -> Result<Session, String> {
    let addr = format!("{}:{}", params.host, params.port);
    let tcp = TcpStream::connect_timeout(
        &addr
            .parse()
            .map_err(|e| format!("Invalid host address {}: {}", addr, e))?,
        Duration::from_secs(10),
    )
    .map_err(|e| format!("Failed to connect to {}: {}", addr, e))?;

    let mut sess = Session::new().map_err(|e| format!("SSH session init failed: {}", e))?;
    sess.set_tcp_stream(tcp);
    sess.handshake().map_err(|e| format!("SSH handshake failed with {}: {}", addr, e))?;

    if let Some(key) = &params.private_key {
        if !key.trim().is_empty() {
            let mut cert_opt = params.certificate.as_deref().filter(|c| !c.trim().is_empty());
            let extracted_cert;

            // Auto-detect if user certificate is embedded in the key text
            if cert_opt.is_none() && key.contains("-cert-v01@openssh.com") {
                let cert_line = key
                    .lines()
                    .find(|line| line.contains("-cert-v01@openssh.com"))
                    .map(|l| l.trim().to_string());
                if let Some(cl) = cert_line {
                    extracted_cert = cl;
                    cert_opt = Some(&extracted_cert);
                }
            }

            sess.userauth_pubkey_memory(
                &params.username,
                cert_opt,
                key,
                params.passphrase.as_deref(),
            )
            .map_err(|e| format!("SSH private key / certificate authentication failed: {}", e))?;
            return Ok(sess);
        }
    }

    if let Some(pw) = &params.password {
        sess.userauth_password(&params.username, pw)
            .map_err(|e| format!("SSH password authentication failed: {}", e))?;
        return Ok(sess);
    }

    // Try agent
    if let Ok(_) = sess.userauth_agent(&params.username) {
        return Ok(sess);
    }

    Err("No valid authentication method provided for SSH".into())
}

pub async fn handle_ssh_session(
    mut socket: WebSocket,
    params: SshConnectionParams,
    initial_cols: u16,
    initial_rows: u16,
) {
    let sess = match tokio::task::spawn_blocking(move || create_ssh_session(&params)).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            let _ = socket
                .send(Message::Text(
                    serde_json::json!({
                        "type": "error",
                        "message": e
                    })
                    .to_string(),
                ))
                .await;
            return;
        }
        Err(e) => {
            let _ = socket
                .send(Message::Text(
                    serde_json::json!({
                        "type": "error",
                        "message": format!("Internal SSH connection task error: {}", e)
                    })
                    .to_string(),
                ))
                .await;
            return;
        }
    };

    let cols = if initial_cols == 0 { 80 } else { initial_cols };
    let rows = if initial_rows == 0 { 24 } else { initial_rows };

    let mut channel = match sess.channel_session() {
        Ok(c) => c,
        Err(e) => {
            let _ = socket
                .send(Message::Text(
                    serde_json::json!({
                        "type": "error",
                        "message": format!("Failed to open SSH channel: {}", e)
                    })
                    .to_string(),
                ))
                .await;
            return;
        }
    };

    if let Err(e) = channel.request_pty("xterm-256color", None, Some((cols as u32, rows as u32, 0, 0))) {
        let _ = socket
            .send(Message::Text(
                serde_json::json!({
                    "type": "error",
                    "message": format!("Failed to request PTY: {}", e)
                })
                .to_string(),
            ))
            .await;
        return;
    }

    if let Err(e) = channel.shell() {
        let _ = socket
            .send(Message::Text(
                serde_json::json!({
                    "type": "error",
                    "message": format!("Failed to start remote shell: {}", e)
                })
                .to_string(),
            ))
            .await;
        return;
    }

    // Set non-blocking mode on the SSH session for high-throughput, non-deadlocking async streaming
    sess.set_blocking(false);

    let (ws_sender_tx, mut ws_sender_rx) = mpsc::channel::<Message>(512);
    let (ssh_cmd_tx, mut ssh_cmd_rx) = mpsc::channel::<SshChannelCommand>(512);
    let is_running = Arc::new(AtomicBool::new(true));

    // Task 1: Dedicated Non-Blocking SSH Worker (Both Read & Write concurrently with zero lock contention)
    let is_running_clone = is_running.clone();
    let ssh_worker_handle = tokio::task::spawn_blocking(move || {
        let mut buf = [0u8; 8192];

        while is_running_clone.load(Ordering::Relaxed) {
            let mut did_work = false;

            // 1. Process all pending outgoing commands/writes to SSH channel
            while let Ok(cmd) = ssh_cmd_rx.try_recv() {
                did_work = true;
                match cmd {
                    SshChannelCommand::Data(data) => {
                        let mut offset = 0;
                        while offset < data.len() && is_running_clone.load(Ordering::Relaxed) {
                            match channel.write(&data[offset..]) {
                                Ok(0) => break,
                                Ok(n) => {
                                    offset += n;
                                }
                                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                                    std::thread::sleep(Duration::from_millis(1));
                                }
                                Err(e) => {
                                    warn!("SSH write error: {}", e);
                                    break;
                                }
                            }
                        }
                        let _ = channel.flush();
                    }
                    SshChannelCommand::Resize(c, r) => {
                        let _ = channel.request_pty_size(c, r, None, None);
                    }
                }
            }

            // 2. Read incoming data from SSH channel -> send to WebSocket
            match channel.read(&mut buf) {
                Ok(0) => {
                    if channel.eof() {
                        break;
                    }
                }
                Ok(n) => {
                    did_work = true;
                    let data = buf[..n].to_vec();
                    if ws_sender_tx.blocking_send(Message::Binary(data)).is_err() {
                        break;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // No data waiting currently
                }
                Err(e) => {
                    warn!("SSH read error: {}", e);
                    break;
                }
            }

            // If neither read nor write had activity, yield CPU for 2ms
            if !did_work {
                std::thread::sleep(Duration::from_millis(2));
            }
        }

        is_running_clone.store(false, Ordering::Relaxed);
    });

    // Task 2: Dispatch outgoing WebSocket messages
    let (mut ws_sink, mut ws_stream) = socket.split();
    let is_running_ws = is_running.clone();
    let mut ws_send_handle = tokio::spawn(async move {
        while let Some(msg) = ws_sender_rx.recv().await {
            if ws_sink.send(msg).await.is_err() {
                break;
            }
        }
        let _ = ws_sink.close().await;
        is_running_ws.store(false, Ordering::Relaxed);
    });

    // Task 3: Read from WebSocket -> Send commands to SSH worker
    loop {
        tokio::select! {
            _ = &mut ws_send_handle => {
                break;
            }
            msg_opt = ws_stream.next() => {
                match msg_opt {
                    Some(Ok(msg)) => {
                        match msg {
                            Message::Binary(bin) => {
                                if ssh_cmd_tx.send(SshChannelCommand::Data(bin)).await.is_err() {
                                    break;
                                }
                            }
                            Message::Text(txt) => {
                                if let Ok(val) = serde_json::from_str::<Value>(&txt) {
                                    if let Some(msg_type) = val.get("type").and_then(|t| t.as_str()) {
                                        match msg_type {
                                            "resize" => {
                                                let c = val.get("cols").and_then(|v| v.as_u64()).unwrap_or(80) as u32;
                                                let r = val.get("rows").and_then(|v| v.as_u64()).unwrap_or(24) as u32;
                                                let _ = ssh_cmd_tx.send(SshChannelCommand::Resize(c, r)).await;
                                                continue;
                                            }
                                            "clipboard_push" => {
                                                if let Some(content) = val.get("text").and_then(|t| t.as_str()) {
                                                    let _ = ssh_cmd_tx.send(SshChannelCommand::Data(content.as_bytes().to_vec())).await;
                                                }
                                                continue;
                                            }
                                            "ping" => continue,
                                            _ => {}
                                        }
                                    }
                                }
                                if ssh_cmd_tx.send(SshChannelCommand::Data(txt.into_bytes())).await.is_err() {
                                    break;
                                }
                            }
                            Message::Close(_) => break,
                            _ => {}
                        }
                    }
                    _ => break,
                }
            }
        }
    }

    is_running.store(false, Ordering::Relaxed);
    let _ = ws_send_handle.abort();
    let _ = ssh_worker_handle.abort();
    info!("SSH session closed");
}

// ================= SFTP Remote File Transfer Subsystem =================

pub fn sftp_list_directory(sftp: &Sftp, remote_path: &str) -> Result<Vec<SftpFileInfo>, String> {
    let path = Path::new(remote_path);
    let entries = sftp.readdir(path).map_err(|e| format!("Failed to read remote dir {}: {}", remote_path, e))?;

    let mut list = Vec::new();
    for (entry_path, stat) in entries {
        let name = entry_path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();

        if name == "." || name == ".." {
            continue;
        }

        let full_path = format!("{}/{}", remote_path.trim_end_matches('/'), name);
        list.push(SftpFileInfo {
            name,
            path: full_path,
            size: stat.size.unwrap_or(0),
            is_dir: stat.is_dir(),
            is_symlink: stat.file_type().is_symlink(),
            modified: stat.mtime.unwrap_or(0),
            permissions: stat.perm.unwrap_or(0),
        });
    }

    list.sort_by(|a, b| {
        if a.is_dir != b.is_dir {
            b.is_dir.cmp(&a.is_dir)
        } else {
            a.name.to_lowercase().cmp(&b.name.to_lowercase())
        }
    });

    Ok(list)
}

pub fn sftp_upload_file(
    sftp: &Sftp,
    local_path: &Path,
    remote_path: &str,
) -> Result<u64, String> {
    let mut local_file = std::fs::File::open(local_path)
        .map_err(|e| format!("Failed to open local file {}: {}", local_path.display(), e))?;

    let mut remote_file = sftp
        .create(Path::new(remote_path))
        .map_err(|e| format!("Failed to create remote file {}: {}", remote_path, e))?;

    let mut buf = [0u8; 65536];
    let mut total_bytes = 0u64;

    loop {
        let n = local_file
            .read(&mut buf)
            .map_err(|e| format!("Read local error: {}", e))?;
        if n == 0 {
            break;
        }
        remote_file
            .write_all(&buf[..n])
            .map_err(|e| format!("Write SFTP error: {}", e))?;
        total_bytes += n as u64;
    }

    remote_file.flush().map_err(|e| format!("Flush SFTP error: {}", e))?;
    Ok(total_bytes)
}

pub fn sftp_download_file(
    sftp: &Sftp,
    remote_path: &str,
    local_path: &Path,
) -> Result<u64, String> {
    let mut remote_file = sftp
        .open(Path::new(remote_path))
        .map_err(|e| format!("Failed to open remote file {}: {}", remote_path, e))?;

    let mut local_file = std::fs::File::create(local_path)
        .map_err(|e| format!("Failed to create local file {}: {}", local_path.display(), e))?;

    let mut buf = [0u8; 65536];
    let mut total_bytes = 0u64;

    loop {
        let n = remote_file
            .read(&mut buf)
            .map_err(|e| format!("Read SFTP error: {}", e))?;
        if n == 0 {
            break;
        }
        local_file
            .write_all(&buf[..n])
            .map_err(|e| format!("Write local error: {}", e))?;
        total_bytes += n as u64;
    }

    local_file.flush().map_err(|e| format!("Flush local error: {}", e))?;
    Ok(total_bytes)
}
