use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use serde_json::Value;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

pub async fn handle_local_pty_session(mut socket: WebSocket, initial_cols: u16, initial_rows: u16) {
    let pty_system = native_pty_system();
    let size = PtySize {
        rows: if initial_rows == 0 { 24 } else { initial_rows },
        cols: if initial_cols == 0 { 80 } else { initial_cols },
        pixel_width: 0,
        pixel_height: 0,
    };

    let pair = match pty_system.openpty(size) {
        Ok(p) => p,
        Err(e) => {
            let _ = socket
                .send(Message::Text(
                    serde_json::json!({
                        "type": "error",
                        "message": format!("Failed to allocate PTY: {}", e)
                    })
                    .to_string(),
                ))
                .await;
            return;
        }
    };

    #[cfg(target_os = "windows")]
    let default_shell = "powershell.exe";
    #[cfg(not(target_os = "windows"))]
    let default_shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());

    let mut cmd = CommandBuilder::new(&default_shell);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");

    if let Ok(home) = std::env::var("HOME") {
        cmd.cwd(home);
    }

    let mut child = match pair.slave.spawn_command(cmd) {
        Ok(c) => c,
        Err(e) => {
            let _ = socket
                .send(Message::Text(
                    serde_json::json!({
                        "type": "error",
                        "message": format!("Failed to spawn shell '{}': {}", default_shell, e)
                    })
                    .to_string(),
                ))
                .await;
            return;
        }
    };

    // Release slave handle in master process
    drop(pair.slave);

    let mut reader = match pair.master.try_clone_reader() {
        Ok(r) => r,
        Err(e) => {
            error!("Failed to clone PTY reader: {}", e);
            return;
        }
    };

    let mut writer = match pair.master.take_writer() {
        Ok(w) => w,
        Err(e) => {
            error!("Failed to take PTY writer: {}", e);
            return;
        }
    };

    let (ws_sender_tx, mut ws_sender_rx) = mpsc::channel::<Message>(256);
    let is_running = Arc::new(AtomicBool::new(true));

    // Task 1: Read from PTY -> Send to WebSocket
    let is_running_clone = is_running.clone();
    let pty_read_handle = tokio::task::spawn_blocking(move || {
        let mut buf = [0u8; 8192];
        while is_running_clone.load(Ordering::Relaxed) {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let data = buf[..n].to_vec();
                    if ws_sender_tx.blocking_send(Message::Binary(data)).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    warn!("PTY read finished/error: {}", e);
                    break;
                }
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

    // Task 3: Read from WebSocket -> Write to PTY or handle control
    let master_mutex = Arc::new(parking_lot::Mutex::new(pair.master));
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
                                let _ = writer.write_all(&bin);
                                let _ = writer.flush();
                            }
                            Message::Text(txt) => {
                                if let Ok(val) = serde_json::from_str::<Value>(&txt) {
                                    if let Some(msg_type) = val.get("type").and_then(|t| t.as_str()) {
                                        match msg_type {
                                            "resize" => {
                                                let cols = val.get("cols").and_then(|c| c.as_u64()).unwrap_or(80) as u16;
                                                let rows = val.get("rows").and_then(|r| r.as_u64()).unwrap_or(24) as u16;
                                                let _ = master_mutex.lock().resize(PtySize {
                                                    rows,
                                                    cols,
                                                    pixel_width: 0,
                                                    pixel_height: 0,
                                                });
                                                continue;
                                            }
                                            "clipboard_push" => {
                                                if let Some(content) = val.get("text").and_then(|t| t.as_str()) {
                                                    let _ = writer.write_all(content.as_bytes());
                                                    let _ = writer.flush();
                                                }
                                                continue;
                                            }
                                            "ping" => {
                                                continue;
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                                // Fallback: write text directly
                                let _ = writer.write_all(txt.as_bytes());
                                let _ = writer.flush();
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
    let _ = child.kill();
    let _ = ws_send_handle.abort();
    let _ = pty_read_handle.abort();
    info!("Local PTY session closed");
}
