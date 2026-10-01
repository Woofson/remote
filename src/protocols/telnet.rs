use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tracing::{info, warn};

// Telnet Protocol Constants (RFC 854, RFC 855, RFC 1091, RFC 1073)
const IAC: u8 = 255; // Interpret As Command
const DONT: u8 = 254;
const DO: u8 = 253;
const WONT: u8 = 252;
const WILL: u8 = 251;
const SB: u8 = 250; // Subnegotiation Begin
const SE: u8 = 240; // Subnegotiation End

// Telnet Option Codes
const OPT_BINARY: u8 = 0;
const OPT_ECHO: u8 = 1;
const OPT_SGA: u8 = 3; // Suppress Go Ahead
const OPT_TTYPE: u8 = 24; // Terminal Type
const OPT_NAWS: u8 = 31; // Negotiate About Window Size

pub struct TelnetConnectionParams {
    pub host: String,
    pub port: u16,
}

pub async fn handle_telnet_session(
    mut socket: WebSocket,
    params: TelnetConnectionParams,
    initial_cols: u16,
    initial_rows: u16,
) {
    let addr = format!("{}:{}", params.host, if params.port == 0 { 23 } else { params.port });
    info!("Connecting to Telnet endpoint at {}", addr);

    let stream = match tokio::time::timeout(Duration::from_secs(10), TcpStream::connect(&addr)).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            let _ = socket
                .send(Message::Text(
                    serde_json::json!({
                        "type": "error",
                        "message": format!("Failed to connect to Telnet host {}: {}", addr, e)
                    })
                    .to_string(),
                ))
                .await;
            return;
        }
        Err(_) => {
            let _ = socket
                .send(Message::Text(
                    serde_json::json!({
                        "type": "error",
                        "message": format!("Connection timed out to Telnet host {}", addr)
                    })
                    .to_string(),
                ))
                .await;
            return;
        }
    };

    let (mut tcp_read, mut tcp_write) = stream.into_split();
    let (ws_sender_tx, mut ws_sender_rx) = mpsc::channel::<Message>(256);
    let (tcp_sender_tx, mut tcp_sender_rx) = mpsc::channel::<Vec<u8>>(256);

    let is_running = Arc::new(AtomicBool::new(true));

    // Send initial client options (WILL NAWS, WILL TTYPE, DO SGA, DO ECHO)
    let initial_naws = naws_packet(initial_cols, initial_rows);
    let _ = tcp_write.write_all(&[IAC, WILL, OPT_NAWS]).await;
    let _ = tcp_write.write_all(&initial_naws).await;
    let _ = tcp_write.write_all(&[IAC, WILL, OPT_TTYPE]).await;
    let _ = tcp_write.write_all(&[IAC, DO, OPT_SGA]).await;
    let _ = tcp_write.write_all(&[IAC, DO, OPT_BINARY]).await;
    let _ = tcp_write.flush().await;

    // Task 1: Read from TCP -> Process Telnet IAC negotiations & forward clean terminal data to WebSocket
    let is_running_clone = is_running.clone();
    let ws_tx = ws_sender_tx.clone();
    let tcp_tx = tcp_sender_tx.clone();
    let tcp_read_handle = tokio::spawn(async move {
        let mut raw_buf = [0u8; 8192];
        let mut iac_state = IacParserState::Normal;

        while is_running_clone.load(Ordering::Relaxed) {
            match tcp_read.read(&mut raw_buf).await {
                Ok(0) => break,
                Ok(n) => {
                    let mut text_output = Vec::with_capacity(n);

                    for &b in &raw_buf[..n] {
                        match iac_state {
                            IacParserState::Normal => {
                                if b == IAC {
                                    iac_state = IacParserState::Iac;
                                } else {
                                    text_output.push(b);
                                }
                            }
                            IacParserState::Iac => {
                                match b {
                                    IAC => {
                                        // Escaped 255 byte
                                        text_output.push(255);
                                        iac_state = IacParserState::Normal;
                                    }
                                    DO => iac_state = IacParserState::Do,
                                    DONT => iac_state = IacParserState::Dont,
                                    WILL => iac_state = IacParserState::Will,
                                    WONT => iac_state = IacParserState::Wont,
                                    SB => iac_state = IacParserState::Subnegotiation(Vec::new()),
                                    _ => {
                                        iac_state = IacParserState::Normal;
                                    }
                                }
                            }
                            IacParserState::Do => {
                                match b {
                                    OPT_NAWS => {
                                        let _ = tcp_tx.send(vec![IAC, WILL, OPT_NAWS]).await;
                                        let _ = tcp_tx.send(naws_packet(initial_cols, initial_rows)).await;
                                    }
                                    OPT_TTYPE => {
                                        let _ = tcp_tx.send(vec![IAC, WILL, OPT_TTYPE]).await;
                                    }
                                    OPT_SGA => {
                                        let _ = tcp_tx.send(vec![IAC, WILL, OPT_SGA]).await;
                                    }
                                    OPT_BINARY => {
                                        let _ = tcp_tx.send(vec![IAC, WILL, OPT_BINARY]).await;
                                    }
                                    other => {
                                        let _ = tcp_tx.send(vec![IAC, WONT, other]).await;
                                    }
                                }
                                iac_state = IacParserState::Normal;
                            }
                            IacParserState::Dont => {
                                iac_state = IacParserState::Normal;
                            }
                            IacParserState::Will => {
                                match b {
                                    OPT_ECHO => {
                                        let _ = tcp_tx.send(vec![IAC, DO, OPT_ECHO]).await;
                                    }
                                    OPT_SGA => {
                                        let _ = tcp_tx.send(vec![IAC, DO, OPT_SGA]).await;
                                    }
                                    OPT_BINARY => {
                                        let _ = tcp_tx.send(vec![IAC, DO, OPT_BINARY]).await;
                                    }
                                    other => {
                                        let _ = tcp_tx.send(vec![IAC, DONT, other]).await;
                                    }
                                }
                                iac_state = IacParserState::Normal;
                            }
                            IacParserState::Wont => {
                                iac_state = IacParserState::Normal;
                            }
                            IacParserState::Subnegotiation(ref mut sub_buf) => {
                                if b == SE && sub_buf.last() == Some(&IAC) {
                                    sub_buf.pop(); // Remove IAC
                                    handle_subnegotiation(sub_buf, &tcp_tx, initial_cols, initial_rows).await;
                                    iac_state = IacParserState::Normal;
                                } else {
                                    sub_buf.push(b);
                                }
                            }
                        }
                    }

                    if !text_output.is_empty() {
                        if ws_tx.send(Message::Binary(text_output)).await.is_err() {
                            break;
                        }
                    }
                }
                Err(e) => {
                    warn!("Telnet TCP read error: {}", e);
                    break;
                }
            }
        }
        is_running_clone.store(false, Ordering::Relaxed);
    });

    // Task 2: Dispatch outgoing packets to Telnet TCP stream
    let is_running_writer = is_running.clone();
    let tcp_write_handle = tokio::spawn(async move {
        while let Some(bytes) = tcp_sender_rx.recv().await {
            if tcp_write.write_all(&bytes).await.is_err() {
                break;
            }
            let _ = tcp_write.flush().await;
        }
        is_running_writer.store(false, Ordering::Relaxed);
    });

    // Task 3: Dispatch WebSocket messages to client
    let (mut ws_sink, mut ws_stream) = socket.split();
    let is_running_ws = is_running.clone();
    let ws_send_handle = tokio::spawn(async move {
        while let Some(msg) = ws_sender_rx.recv().await {
            if ws_sink.send(msg).await.is_err() {
                break;
            }
        }
        is_running_ws.store(false, Ordering::Relaxed);
    });

    // Task 4: Handle WebSocket incoming messages from client (keystrokes, resize, clipboard)
    let tcp_sender = tcp_sender_tx.clone();
    while let Some(Ok(msg)) = ws_stream.next().await {
        match msg {
            Message::Binary(bin) => {
                let _ = tcp_sender.send(bin).await;
            }
            Message::Text(txt) => {
                if let Ok(val) = serde_json::from_str::<Value>(&txt) {
                    if let Some(msg_type) = val.get("type").and_then(|t| t.as_str()) {
                        match msg_type {
                            "resize" => {
                                let c = val.get("cols").and_then(|v| v.as_u64()).unwrap_or(80) as u16;
                                let r = val.get("rows").and_then(|v| v.as_u64()).unwrap_or(24) as u16;
                                let _ = tcp_sender.send(naws_packet(c, r)).await;
                                continue;
                            }
                            "clipboard_push" => {
                                if let Some(content) = val.get("text").and_then(|t| t.as_str()) {
                                    let _ = tcp_sender.send(content.as_bytes().to_vec()).await;
                                }
                                continue;
                            }
                            "ping" => continue,
                            _ => {}
                        }
                    }
                }
                let _ = tcp_sender.send(txt.into_bytes()).await;
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    is_running.store(false, Ordering::Relaxed);
    let _ = tcp_read_handle.abort();
    let _ = tcp_write_handle.abort();
    let _ = ws_send_handle.abort();
    info!("Telnet session closed for {}", addr);
}

enum IacParserState {
    Normal,
    Iac,
    Do,
    Dont,
    Will,
    Wont,
    Subnegotiation(Vec<u8>),
}

pub fn naws_packet(cols: u16, rows: u16) -> Vec<u8> {
    let c = if cols == 0 { 80 } else { cols };
    let r = if rows == 0 { 24 } else { rows };
    vec![
        IAC,
        SB,
        OPT_NAWS,
        (c >> 8) as u8,
        (c & 0xFF) as u8,
        (r >> 8) as u8,
        (r & 0xFF) as u8,
        IAC,
        SE,
    ]
}

async fn handle_subnegotiation(
    buf: &[u8],
    tcp_tx: &mpsc::Sender<Vec<u8>>,
    _cols: u16,
    _rows: u16,
) {
    if buf.is_empty() {
        return;
    }

    match buf[0] {
        OPT_TTYPE => {
            // Server asks: IAC SB TTYPE SEND IAC SE (1 byte 0x01)
            if buf.len() > 1 && buf[1] == 1 {
                // Respond: IAC SB TTYPE IS "xterm-256color" IAC SE
                let mut resp = vec![IAC, SB, OPT_TTYPE, 0];
                resp.extend_from_slice(b"xterm-256color");
                resp.extend_from_slice(&[IAC, SE]);
                let _ = tcp_tx.send(resp).await;
            }
        }
        _ => {}
    }
}
