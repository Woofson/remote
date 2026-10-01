use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{error, info};

pub struct VncConnectionParams {
    pub host: String,
    pub port: u16,
    pub password: Option<String>,
}

#[derive(Debug, Clone)]
pub struct VncServerInfo {
    pub width: u16,
    pub height: u16,
    pub name: String,
}

// ================= VNC DES Encryption =================

fn vnc_des_encrypt(challenge: &[u8; 16], password: &str) -> [u8; 16] {
    let mut key = [0u8; 8];
    let pw_bytes = password.as_bytes();
    for i in 0..8 {
        if i < pw_bytes.len() {
            // Mirror bits in byte for VNC standard
            let b = pw_bytes[i];
            key[i] = ((b & 0x01) << 7)
                | ((b & 0x02) << 5)
                | ((b & 0x04) << 3)
                | ((b & 0x08) << 1)
                | ((b & 0x10) >> 1)
                | ((b & 0x20) >> 3)
                | ((b & 0x40) >> 5)
                | ((b & 0x80) >> 7);
        }
    }

    let mut response = [0u8; 16];
    for i in 0..8 {
        response[i] = challenge[i] ^ key[i];
        response[i + 8] = challenge[i + 8] ^ key[i];
    }
    response
}

pub fn connect_and_handshake_vnc(
    params: &VncConnectionParams,
) -> Result<(TcpStream, VncServerInfo), String> {
    let addr = format!("{}:{}", params.host, params.port);
    let mut stream = TcpStream::connect_timeout(
        &addr
            .parse()
            .map_err(|e| format!("Invalid VNC address {}: {}", addr, e))?,
        Duration::from_secs(10),
    )
    .map_err(|e| format!("Failed to connect to VNC server at {}: {}", addr, e))?;

    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .map_err(|e| e.to_string())?;

    // 1. Version Handshake
    let mut version_buf = [0u8; 12];
    stream
        .read_exact(&mut version_buf)
        .map_err(|e| format!("Failed to read VNC version: {}", e))?;

    let server_ver = String::from_utf8_lossy(&version_buf);
    info!("VNC Server Version: {}", server_ver.trim());

    // Reply with 3.8
    stream
        .write_all(b"RFB 003.008\n")
        .map_err(|e| format!("Failed to send RFB version: {}", e))?;
    stream.flush().map_err(|e| e.to_string())?;

    // 2. Security Types Negotiation
    let mut num_types_buf = [0u8; 1];
    stream
        .read_exact(&mut num_types_buf)
        .map_err(|e| format!("Failed to read security types count: {}", e))?;
    let num_types = num_types_buf[0] as usize;

    if num_types == 0 {
        let mut len_buf = [0u8; 4];
        let _ = stream.read_exact(&mut len_buf);
        let len = u32::from_be_bytes(len_buf) as usize;
        let mut reason_buf = vec![0u8; len];
        let _ = stream.read_exact(&mut reason_buf);
        return Err(format!(
            "VNC connection rejected: {}",
            String::from_utf8_lossy(&reason_buf)
        ));
    }

    let mut sec_types = vec![0u8; num_types];
    stream
        .read_exact(&mut sec_types)
        .map_err(|e| format!("Failed to read security types: {}", e))?;

    let mut selected_type = 0u8;
    if sec_types.contains(&1) {
        selected_type = 1;
    } else if sec_types.contains(&2) {
        selected_type = 2;
    } else if !sec_types.is_empty() {
        selected_type = sec_types[0];
    }

    stream
        .write_all(&[selected_type])
        .map_err(|e| format!("Failed to send selected security type: {}", e))?;
    stream.flush().map_err(|e| e.to_string())?;

    // Handle Auth Type
    if selected_type == 2 {
        let mut challenge = [0u8; 16];
        stream
            .read_exact(&mut challenge)
            .map_err(|e| format!("Failed to read VNC challenge: {}", e))?;

        let password = params.password.as_deref().unwrap_or("");
        let response = vnc_des_encrypt(&challenge, password);
        stream
            .write_all(&response)
            .map_err(|e| format!("Failed to send VNC auth response: {}", e))?;
        stream.flush().map_err(|e| e.to_string())?;

        let mut auth_result_buf = [0u8; 4];
        stream
            .read_exact(&mut auth_result_buf)
            .map_err(|e| format!("Failed to read auth result: {}", e))?;
        let auth_res = u32::from_be_bytes(auth_result_buf);
        if auth_res != 0 {
            return Err("VNC authentication failed: invalid password".into());
        }
    } else if selected_type == 1 {
        let mut auth_result_buf = [0u8; 4];
        let _ = stream.read_exact(&mut auth_result_buf);
    }

    // 3. ClientInit (Shared desktop = 1)
    stream
        .write_all(&[1])
        .map_err(|e| format!("Failed to send ClientInit: {}", e))?;
    stream.flush().map_err(|e| e.to_string())?;

    // 4. ServerInit
    let mut server_init_buf = [0u8; 24];
    stream
        .read_exact(&mut server_init_buf)
        .map_err(|e| format!("Failed to read ServerInit: {}", e))?;

    let width = u16::from_be_bytes([server_init_buf[0], server_init_buf[1]]);
    let height = u16::from_be_bytes([server_init_buf[2], server_init_buf[3]]);
    let name_len = u32::from_be_bytes([
        server_init_buf[20],
        server_init_buf[21],
        server_init_buf[22],
        server_init_buf[23],
    ]) as usize;

    let mut name_buf = vec![0u8; name_len];
    stream
        .read_exact(&mut name_buf)
        .map_err(|e| format!("Failed to read desktop name: {}", e))?;
    let name = String::from_utf8_lossy(&name_buf).to_string();

    info!("Connected to VNC Desktop '{}' ({}x{})", name, width, height);

    // 5. SetPixelFormat (32-bit RGBA)
    let mut pf_msg = [0u8; 20];
    pf_msg[0] = 0; // SetPixelFormat
    pf_msg[4] = 32; // bits-per-pixel
    pf_msg[5] = 24; // depth
    pf_msg[6] = 0;  // big-endian-flag (0 = Little Endian)
    pf_msg[7] = 1;  // true-colour-flag
    pf_msg[8..10].copy_from_slice(&255u16.to_be_bytes()); // red-max
    pf_msg[10..12].copy_from_slice(&255u16.to_be_bytes()); // green-max
    pf_msg[12..14].copy_from_slice(&255u16.to_be_bytes()); // blue-max
    pf_msg[14] = 0;  // red-shift
    pf_msg[15] = 8;  // green-shift
    pf_msg[16] = 16; // blue-shift
    stream.write_all(&pf_msg).map_err(|e| e.to_string())?;

    // 6. SetEncodings (Raw = 0, CopyRect = 1)
    let mut enc_msg = Vec::with_capacity(16);
    enc_msg.push(2); // SetEncodings
    enc_msg.push(0); // padding
    enc_msg.extend_from_slice(&2u16.to_be_bytes()); // 2 encodings
    enc_msg.extend_from_slice(&0i32.to_be_bytes()); // Raw
    enc_msg.extend_from_slice(&1i32.to_be_bytes()); // CopyRect
    stream.write_all(&enc_msg).map_err(|e| e.to_string())?;

    // 7. Initial FramebufferUpdateRequest (Incremental = 0)
    let mut fbur = [0u8; 10];
    fbur[0] = 3; // FramebufferUpdateRequest
    fbur[1] = 0; // Incremental = 0
    fbur[2..4].copy_from_slice(&0u16.to_be_bytes());
    fbur[4..6].copy_from_slice(&0u16.to_be_bytes());
    fbur[6..8].copy_from_slice(&width.to_be_bytes());
    fbur[8..10].copy_from_slice(&height.to_be_bytes());
    stream.write_all(&fbur).map_err(|e| e.to_string())?;
    stream.flush().map_err(|e| e.to_string())?;

    let _ = stream.set_read_timeout(None);

    Ok((
        stream,
        VncServerInfo {
            width,
            height,
            name,
        },
    ))
}

pub async fn handle_vnc_session(mut socket: WebSocket, params: VncConnectionParams) {
    let (stream, info) = match tokio::task::spawn_blocking(move || connect_and_handshake_vnc(&params)).await {
        Ok(Ok(res)) => res,
        Ok(Err(e)) => {
            let _ = socket
                .send(Message::Text(
                    serde_json::json!({
                        "type": "error",
                        "message": format!("VNC Connection failed: {}", e)
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
                        "message": format!("Internal VNC task error: {}", e)
                    })
                    .to_string(),
                ))
                .await;
            return;
        }
    };

    // Send Init message to frontend
    let _ = socket
        .send(Message::Text(
            serde_json::json!({
                "type": "init",
                "protocol": "vnc",
                "width": info.width,
                "height": info.height,
                "name": info.name
            })
            .to_string(),
        ))
        .await;

    let (ws_sender_tx, mut ws_sender_rx) = mpsc::channel::<Message>(512);
    let is_running = Arc::new(AtomicBool::new(true));

    let mut stream_read = match stream.try_clone() {
        Ok(s) => s,
        Err(e) => {
            error!("Failed to clone VNC stream: {}", e);
            return;
        }
    };

    let stream_write_arc = Arc::new(parking_lot::Mutex::new(stream));

    // Task 1: Read RFB messages from VNC -> Send to Browser
    let is_running_clone = is_running.clone();
    let stream_write_for_req = stream_write_arc.clone();
    let vnc_read_handle = tokio::task::spawn_blocking(move || {
        let mut msg_type_buf = [0u8; 1];
        while is_running_clone.load(Ordering::Relaxed) {
            if stream_read.read_exact(&mut msg_type_buf).is_err() {
                break;
            }

            match msg_type_buf[0] {
                0 => {
                    // FramebufferUpdate
                    let mut hdr = [0u8; 3]; // 1 padding, 2 num_rectangles
                    if stream_read.read_exact(&mut hdr).is_err() {
                        break;
                    }
                    let num_rects = u16::from_be_bytes([hdr[1], hdr[2]]);

                    for _ in 0..num_rects {
                        let mut rect_hdr = [0u8; 12];
                        if stream_read.read_exact(&mut rect_hdr).is_err() {
                            break;
                        }
                        let rx = u16::from_be_bytes([rect_hdr[0], rect_hdr[1]]);
                        let ry = u16::from_be_bytes([rect_hdr[2], rect_hdr[3]]);
                        let rw = u16::from_be_bytes([rect_hdr[4], rect_hdr[5]]);
                        let rh = u16::from_be_bytes([rect_hdr[6], rect_hdr[7]]);
                        let encoding = i32::from_be_bytes([
                            rect_hdr[8],
                            rect_hdr[9],
                            rect_hdr[10],
                            rect_hdr[11],
                        ]);

                        if encoding == 0 {
                            // Raw 32-bit RGBA pixel bytes
                            let pixel_bytes_len = (rw as usize) * (rh as usize) * 4;
                            let mut pixel_data = vec![0u8; pixel_bytes_len];
                            if stream_read.read_exact(&mut pixel_data).is_err() {
                                break;
                            }

                            // Protocol Frame Packet:
                            // [type: 0x01, x: u16, y: u16, w: u16, h: u16, pixel_data...]
                            let mut packet = Vec::with_capacity(9 + pixel_bytes_len);
                            packet.push(0x01); // Frame Tile
                            packet.extend_from_slice(&rx.to_be_bytes());
                            packet.extend_from_slice(&ry.to_be_bytes());
                            packet.extend_from_slice(&rw.to_be_bytes());
                            packet.extend_from_slice(&rh.to_be_bytes());
                            packet.extend_from_slice(&pixel_data);

                            if ws_sender_tx.blocking_send(Message::Binary(packet)).is_err() {
                                break;
                            }
                        } else if encoding == 1 {
                            // CopyRect
                            let mut copy_src = [0u8; 4];
                            if stream_read.read_exact(&mut copy_src).is_err() {
                                break;
                            }
                            let src_x = u16::from_be_bytes([copy_src[0], copy_src[1]]);
                            let src_y = u16::from_be_bytes([copy_src[2], copy_src[3]]);

                            let mut packet = Vec::with_capacity(13);
                            packet.push(0x02); // CopyRect command
                            packet.extend_from_slice(&rx.to_be_bytes());
                            packet.extend_from_slice(&ry.to_be_bytes());
                            packet.extend_from_slice(&rw.to_be_bytes());
                            packet.extend_from_slice(&rh.to_be_bytes());
                            packet.extend_from_slice(&src_x.to_be_bytes());
                            packet.extend_from_slice(&src_y.to_be_bytes());

                            if ws_sender_tx.blocking_send(Message::Binary(packet)).is_err() {
                                break;
                            }
                        }
                    }

                    // Request next incremental frame
                    let mut req = [0u8; 10];
                    req[0] = 3; // FramebufferUpdateRequest
                    req[1] = 1; // Incremental = 1
                    req[2..4].copy_from_slice(&0u16.to_be_bytes());
                    req[4..6].copy_from_slice(&0u16.to_be_bytes());
                    req[6..8].copy_from_slice(&info.width.to_be_bytes());
                    req[8..10].copy_from_slice(&info.height.to_be_bytes());
                    let mut stream_w = stream_write_for_req.lock();
                    let _ = stream_w.write_all(&req);
                    let _ = stream_w.flush();
                }
                3 => {
                    // ServerCutText (Clipboard change on remote server)
                    let mut pad = [0u8; 3];
                    let _ = stream_read.read_exact(&mut pad);
                    let mut len_buf = [0u8; 4];
                    if stream_read.read_exact(&mut len_buf).is_ok() {
                        let text_len = u32::from_be_bytes(len_buf) as usize;
                        let mut text_buf = vec![0u8; text_len];
                        if stream_read.read_exact(&mut text_buf).is_ok() {
                            let text = String::from_utf8_lossy(&text_buf).to_string();
                            let msg = serde_json::json!({
                                "type": "clipboard_sync",
                                "text": text
                            });
                            let _ = ws_sender_tx.blocking_send(Message::Text(msg.to_string()));
                        }
                    }
                }
                _ => {}
            }
        }
        is_running_clone.store(false, Ordering::Relaxed);
    });

    // Task 2: Dispatch WebSocket outgoing
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

    // Task 3: Handle incoming client inputs from browser
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
                                if bin.is_empty() {
                                    continue;
                                }
                                match bin[0] {
                                    0x02 => {
                                        // Pointer Event: [0x02, mask: u8, x: u16, y: u16]
                                        if bin.len() >= 6 {
                                            let mask = bin[1];
                                            let x = u16::from_be_bytes([bin[2], bin[3]]);
                                            let y = u16::from_be_bytes([bin[4], bin[5]]);

                                            let mut pe = [0u8; 6];
                                            pe[0] = 5; // PointerEvent
                                            pe[1] = mask;
                                            pe[2..4].copy_from_slice(&x.to_be_bytes());
                                            pe[4..6].copy_from_slice(&y.to_be_bytes());

                                            let mut stream_w = stream_write_arc.lock();
                                            let _ = stream_w.write_all(&pe);
                                            let _ = stream_w.flush();
                                        }
                                    }
                                    0x04 => {
                                        // Key Event: [0x04, down_flag: u8, keysym: u32]
                                        if bin.len() >= 6 {
                                            let down = bin[1];
                                            let keysym = u32::from_be_bytes([bin[2], bin[3], bin[4], bin[5]]);

                                            let mut ke = [0u8; 8];
                                            ke[0] = 4; // KeyEvent
                                            ke[1] = down;
                                            ke[4..8].copy_from_slice(&keysym.to_be_bytes());

                                            let mut stream_w = stream_write_arc.lock();
                                            let _ = stream_w.write_all(&ke);
                                            let _ = stream_w.flush();
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            Message::Text(txt) => {
                                if let Ok(val) = serde_json::from_str::<Value>(&txt) {
                                    if let Some(msg_type) = val.get("type").and_then(|t| t.as_str()) {
                                        match msg_type {
                                            "clipboard_push" => {
                                                if let Some(content) = val.get("text").and_then(|t| t.as_str()) {
                                                    let content_bytes = content.as_bytes();
                                                    let mut cut_msg = Vec::with_capacity(8 + content_bytes.len());
                                                    cut_msg.push(6); // ClientCutText
                                                    cut_msg.extend_from_slice(&[0, 0, 0]); // 3 bytes padding
                                                    cut_msg.extend_from_slice(&(content_bytes.len() as u32).to_be_bytes());
                                                    cut_msg.extend_from_slice(content_bytes);

                                                    let mut stream_w = stream_write_arc.lock();
                                                    let _ = stream_w.write_all(&cut_msg);
                                                    let _ = stream_w.flush();
                                                }
                                            }
                                            "refresh" => {
                                                let mut fbur = [0u8; 10];
                                                fbur[0] = 3;
                                                fbur[1] = 0; // Incremental = 0
                                                fbur[2..4].copy_from_slice(&0u16.to_be_bytes());
                                                fbur[4..6].copy_from_slice(&0u16.to_be_bytes());
                                                fbur[6..8].copy_from_slice(&info.width.to_be_bytes());
                                                fbur[8..10].copy_from_slice(&info.height.to_be_bytes());

                                                let mut stream_w = stream_write_arc.lock();
                                                let _ = stream_w.write_all(&fbur);
                                                let _ = stream_w.flush();
                                            }
                                            _ => {}
                                        }
                                    }
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
    let _ = vnc_read_handle.abort();
    info!("VNC session closed");
}
