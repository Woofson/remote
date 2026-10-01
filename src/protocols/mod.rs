pub mod local_pty;
pub mod ssh;
pub mod telnet;
pub mod vnc;
pub mod rdp;

use serde::{Deserialize, Serialize};

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
