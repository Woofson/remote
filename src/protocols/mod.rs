pub mod hub;
pub mod local_pty;
pub mod net;
pub mod ssh;
pub mod telnet;
pub mod vnc;
pub mod rdp;
pub mod rdp_client;
pub mod rdp_drive;

use serde::{Deserialize, Serialize};

/// Seconds between keepalive pings on a session's WebSocket.
const KEEPALIVE_SECS: u64 = 25;

/// Send a WebSocket ping every 25 seconds while the session lives, so reverse proxies
/// and tunnels (Nginx 60 s, Cloudflare 100 s idle limits) do not close quiet sessions.
/// Holds only a weak handle: it never keeps a finished session open.
pub fn spawn_keepalive(tx: &tokio::sync::mpsc::Sender<axum::extract::ws::Message>) {
    let weak = tx.downgrade();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(KEEPALIVE_SECS));
        tick.tick().await;
        loop {
            tick.tick().await;
            let Some(tx) = weak.upgrade() else { break };
            if tx.send(axum::extract::ws::Message::Ping(Vec::new())).await.is_err() {
                break;
            }
        }
    });
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ClientControlMessage {
    #[serde(rename = "resize")]
    Resize { cols: u16, rows: u16 },
    #[serde(rename = "clipboard_push")]
    ClipboardPush { text: String },
    #[serde(rename = "key")]
    Key { down: bool, keysym: u32, key: Option<String> },
    #[serde(rename = "mouse")]
    Mouse { x: u16, y: u16, mask: u8 },
    #[serde(rename = "refresh")]
    Refresh,
    #[serde(rename = "ping")]
    Ping,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ServerControlMessage {
    #[serde(rename = "init")]
    Init {
        protocol: String,
        width: u32,
        height: u32,
        name: String,
    },
    #[serde(rename = "clipboard_sync")]
    ClipboardSync { text: String },
    #[serde(rename = "error")]
    Error { message: String },
    #[serde(rename = "pong")]
    Pong,
    #[serde(rename = "status")]
    Status { connected: bool, message: String },
}
