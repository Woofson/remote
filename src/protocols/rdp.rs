use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use ironrdp_client::config::{ConfigBuilder, Destination};
use ironrdp_client::rdp::{RdpClient, RdpInputEvent, RdpOutputEvent};
use ironrdp_pdu::input::fast_path::{FastPathInputEvent, KeyboardFlags};
use ironrdp_pdu::input::mouse::{MousePdu, PointerFlags};
use ironrdp_pdu::rdp::capability_sets::MajorPlatformType;
use serde_json::json;
use smallvec::smallvec;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

pub struct RdpConnectionParams {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    pub domain: Option<String>,
    pub ignore_cert: bool,
    pub width: u16,
    pub height: u16,
    pub color_depth: u32,
    pub enable_audio: bool,
    pub disable_wallpaper: bool,
    pub disable_full_window_drag: bool,
    pub disable_menu_animations: bool,
    pub disable_themes: bool,
    pub font_smoothing: bool,
    pub staging_dir: Option<String>,
    pub enable_drive_redirection: bool,
    pub keyboard_layout: Option<String>,
}

/// Map incoming browser mouse events (bitmask + coordinates) to IronRDP FastPath mouse events
fn map_mouse_events(
    mask: u8,
    prev_mask: u8,
    x: u16,
    y: u16,
    prev_x: u16,
    prev_y: u16,
) -> smallvec::SmallVec<[FastPathInputEvent; 2]> {
    let mut events = smallvec::SmallVec::new();

    let pos_changed = x != prev_x || y != prev_y;
    let button_changed = mask != prev_mask;

    // 1. If position moved, send MOVE event first
    if pos_changed {
        events.push(FastPathInputEvent::MouseEvent(MousePdu {
            flags: PointerFlags::MOVE,
            number_of_wheel_rotation_units: 0,
            x_position: x,
            y_position: y,
        }));
    }

    // 2. Left Button transition (PTRFLAGS_BUTTON1 | PTRFLAGS_DOWN on press, PTRFLAGS_BUTTON1 on release)
    if (mask & 1) != (prev_mask & 1) {
        let mut flags = PointerFlags::LEFT_BUTTON;
        if (mask & 1) != 0 {
            flags |= PointerFlags::DOWN;
        }
        events.push(FastPathInputEvent::MouseEvent(MousePdu {
            flags,
            number_of_wheel_rotation_units: 0,
            x_position: x,
            y_position: y,
        }));
    }

    // 3. Right Button transition (PTRFLAGS_BUTTON2)
    if (mask & 4) != (prev_mask & 4) {
        let mut flags = PointerFlags::RIGHT_BUTTON;
        if (mask & 4) != 0 {
            flags |= PointerFlags::DOWN;
        }
        events.push(FastPathInputEvent::MouseEvent(MousePdu {
            flags,
            number_of_wheel_rotation_units: 0,
            x_position: x,
            y_position: y,
        }));
    }

    // 4. Middle Button transition (PTRFLAGS_BUTTON3)
    if (mask & 2) != (prev_mask & 2) {
        let mut flags = PointerFlags::MIDDLE_BUTTON_OR_WHEEL;
        if (mask & 2) != 0 {
            flags |= PointerFlags::DOWN;
        }
        events.push(FastPathInputEvent::MouseEvent(MousePdu {
            flags,
            number_of_wheel_rotation_units: 0,
            x_position: x,
            y_position: y,
        }));
    }

    // 5. Vertical Wheel (PTRFLAGS_WHEEL)
    if (mask & 8) != 0 {
        events.push(FastPathInputEvent::MouseEvent(MousePdu {
            flags: PointerFlags::VERTICAL_WHEEL,
            number_of_wheel_rotation_units: 120,
            x_position: x,
            y_position: y,
        }));
    } else if (mask & 16) != 0 {
        events.push(FastPathInputEvent::MouseEvent(MousePdu {
            flags: PointerFlags::VERTICAL_WHEEL | PointerFlags::WHEEL_NEGATIVE,
            number_of_wheel_rotation_units: -120,
            x_position: x,
            y_position: y,
        }));
    }

    // Fallback: if nothing was emitted, emit a pointer move
    if events.is_empty() && !pos_changed && !button_changed {
        events.push(FastPathInputEvent::MouseEvent(MousePdu {
            flags: PointerFlags::MOVE,
            number_of_wheel_rotation_units: 0,
            x_position: x,
            y_position: y,
        }));
    }

    events
}

/// Convert standard keysyms to PS/2 Set 1 scancodes (and extended flag)
fn keysym_to_scancode(keysym: u32) -> Option<(u8, bool)> {
    match keysym {
        // Special & Navigation keys
        0xff08 => Some((0x0e, false)), // Backspace
        0xff09 => Some((0x0f, false)), // Tab
        0xff0d => Some((0x1c, false)), // Enter / Return
        0xff1b => Some((0x01, false)), // Escape
        0xffff => Some((0x53, true)),  // Delete
        0xff50 => Some((0x47, true)),  // Home
        0xff51 => Some((0x4b, true)),  // Arrow Left
        0xff52 => Some((0x48, true)),  // Arrow Up
        0xff53 => Some((0x4d, true)),  // Arrow Right
        0xff54 => Some((0x50, true)),  // Arrow Down
        0xff55 => Some((0x49, true)),  // Page Up
        0xff56 => Some((0x51, true)),  // Page Down
        0xff57 => Some((0x4f, true)),  // End
        0xff63 => Some((0x52, true)),  // Insert

        // Modifiers
        0xffe1 => Some((0x2a, false)), // Shift_L
        0xffe2 => Some((0x36, false)), // Shift_R
        0xffe3 => Some((0x1d, false)), // Control_L
        0xffe4 => Some((0x1d, true)),  // Control_R
        0xffe9 => Some((0x38, false)), // Alt_L
        0xffea => Some((0x38, true)),  // Alt_R
        0xffeb => Some((0x5b, true)),  // Super_L / Windows key
        0xffec => Some((0x5c, true)),  // Super_R
        0xffe5 => Some((0x3a, false)), // Caps_Lock
        0xff7f => Some((0x45, false)), // Num_Lock
        0xff14 => Some((0x46, false)), // Scroll_Lock

        // Function Keys F1 - F12
        0xffbe => Some((0x3b, false)), // F1
        0xffbf => Some((0x3c, false)), // F2
        0xffc0 => Some((0x3d, false)), // F3
        0xffc1 => Some((0x3e, false)), // F4
        0xffc2 => Some((0x3f, false)), // F5
        0xffc3 => Some((0x40, false)), // F6
        0xffc4 => Some((0x41, false)), // F7
        0xffc5 => Some((0x42, false)), // F8
        0xffc6 => Some((0x43, false)), // F9
        0xffc7 => Some((0x44, false)), // F10
        0xffc8 => Some((0x57, false)), // F11
        0xffc9 => Some((0x58, false)), // F12

        // Space and Numbers
        0x20 => Some((0x39, false)), // Space
        0x30 | 0x29 => Some((0x0b, false)), // '0' / ')'
        0x31 | 0x21 => Some((0x02, false)), // '1' / '!'
        0x32 | 0x40 => Some((0x03, false)), // '2' / '@'
        0x33 | 0x23 => Some((0x04, false)), // '3' / '#'
        0x34 | 0x24 => Some((0x05, false)), // '4' / '$'
        0x35 | 0x25 => Some((0x06, false)), // '5' / '%'
        0x36 | 0x5e => Some((0x07, false)), // '6' / '^'
        0x37 | 0x26 => Some((0x08, false)), // '7' / '&'
        0x38 | 0x2a => Some((0x09, false)), // '8' / '*'
        0x39 | 0x28 => Some((0x0a, false)), // '9' / '('

        // Letters (both lower and upper case map to the same hardware scancode)
        0x61 | 0x41 => Some((0x1e, false)), // A
        0x62 | 0x42 => Some((0x30, false)), // B
        0x63 | 0x43 => Some((0x2e, false)), // C
        0x64 | 0x44 => Some((0x20, false)), // D
        0x65 | 0x45 => Some((0x12, false)), // E
        0x66 | 0x46 => Some((0x21, false)), // F
        0x67 | 0x47 => Some((0x22, false)), // G
        0x68 | 0x48 => Some((0x23, false)), // H
        0x69 | 0x49 => Some((0x17, false)), // I
        0x6a | 0x4a => Some((0x24, false)), // J
        0x6b | 0x4b => Some((0x25, false)), // K
        0x6c | 0x4c => Some((0x26, false)), // L
        0x6d | 0x4d => Some((0x32, false)), // M
        0x6e | 0x4e => Some((0x31, false)), // N
        0x6f | 0x4f => Some((0x18, false)), // O
        0x70 | 0x50 => Some((0x19, false)), // P
        0x71 | 0x51 => Some((0x10, false)), // Q
        0x72 | 0x52 => Some((0x13, false)), // R
        0x73 | 0x53 => Some((0x1f, false)), // S
        0x74 | 0x54 => Some((0x14, false)), // T
        0x75 | 0x55 => Some((0x16, false)), // U
        0x76 | 0x56 => Some((0x2f, false)), // V
        0x77 | 0x57 => Some((0x11, false)), // W
        0x78 | 0x58 => Some((0x2d, false)), // X
        0x79 | 0x59 => Some((0x15, false)), // Y
        0x7a | 0x5a => Some((0x2c, false)), // Z

        // Symbols / Punctuation
        0x2d | 0x5f => Some((0x0c, false)), // '-' / '_'
        0x3d | 0x2b => Some((0x0d, false)), // '=' / '+'
        0x5b | 0x7b => Some((0x1a, false)), // '[' / '{'
        0x5d | 0x7d => Some((0x1b, false)), // ']' / '}'
        0x3b | 0x3a => Some((0x27, false)), // ';' / ':'
        0x27 | 0x22 => Some((0x28, false)), // '\'' / '"'
        0x60 | 0x7e => Some((0x29, false)), // '`' / '~'
        0x5c | 0x7c => Some((0x2b, false)), // '\\' / '|'
        0x2c | 0x3c => Some((0x33, false)), // ',' / '<'
        0x2e | 0x3e => Some((0x34, false)), // '.' / '>'
        0x2f | 0x3f => Some((0x35, false)), // '/' / '?'

        _ => None,
    }
}

/// Map incoming RFB/X11 keysyms to IronRDP FastPath keyboard events
fn map_key_event(
    down: bool,
    keysym: u32,
) -> smallvec::SmallVec<[FastPathInputEvent; 2]> {
    let mut kbd_flags = if down {
        KeyboardFlags::empty()
    } else {
        KeyboardFlags::RELEASE
    };

    if let Some((code, extended)) = keysym_to_scancode(keysym) {
        if extended {
            kbd_flags |= KeyboardFlags::EXTENDED;
        }
        smallvec![FastPathInputEvent::KeyboardEvent(kbd_flags, code)]
    } else if down && keysym <= 0xffff && keysym >= 0x20 {
        // Fallback for non-ASCII Unicode characters: ONLY send on KEY DOWN!
        // Sending UnicodeKeyboardEvent on key release causes Windows to type the character twice.
        smallvec![FastPathInputEvent::UnicodeKeyboardEvent(kbd_flags, keysym as u16)]
    } else {
        smallvec![]
    }
}

/// Handle a full RDP session using IronRDP with NLA (CredSSP), TLS, and RDPGFX decoding
pub async fn handle_rdp_session(socket: WebSocket, params: RdpConnectionParams) {
    let target_display = format!("{}:{}", params.host, params.port);
    info!(
        "RDP Gateway: Initializing connection to {} (user: {:?}, domain: {:?}, size: {}x{})",
        target_display, params.username, params.domain, params.width, params.height
    );

    let destination = Destination::from_parts(params.host.clone(), params.port);
    let username = params.username.as_deref().unwrap_or("").trim();
    let password = params.password.as_deref().unwrap_or("");

    let mut config_builder = ConfigBuilder::new()
        .with_destination(destination)
        .with_desktop_width(params.width.max(640))
        .with_desktop_height(params.height.max(480))
        .with_color_depth(if params.color_depth == 16 { 16 } else { 32 })
        .with_credssp(true)
        .with_tls(true)
        .with_pointer_software_rendering(false)
        .with_compression(true)
        .with_compression_level(2)
        .with_client_build(2600)
        .with_client_dir("C:\\Windows\\System32")
        .with_client_name("Remote")
        .with_platform(MajorPlatformType::WINDOWS)
        .with_username(username)
        .with_password(password);

    if let Some(dom) = &params.domain {
        if !dom.trim().is_empty() {
            config_builder = config_builder.with_domain(dom.trim());
        }
    }


    let config = match config_builder.build() {
        Ok(c) => c,
        Err(e) => {
            error!("RDP Gateway: Invalid configuration: {:#}", e);
            let mut s = socket;
            let _ = s
                .send(Message::Text(
                    json!({
                        "type": "error",
                        "message": format!("Invalid RDP configuration: {}", e)
                    })
                    .to_string(),
                ))
                .await;
            return;
        }
    };

    let (output_tx, mut output_rx) = mpsc::channel::<RdpOutputEvent>(64);
    let client = RdpClient::new(config, output_tx);
    let input_sender = client.input_sender();

    // Spawn IronRDP client on a dedicated thread with a current_thread tokio runtime
    let thread_target = target_display.clone();
    let _rdp_thread = std::thread::Builder::new()
        .name(format!("rdp-{}", thread_target))
        .spawn(move || {
            let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(r) => r,
                Err(e) => {
                    error!("RDP Gateway: Failed to create thread runtime: {}", e);
                    return;
                }
            };
            rt.block_on(async move {
                client.run().await;
            });
        });

    let (mut ws_tx, mut ws_rx) = socket.split();
    let (ws_out_tx, mut ws_out_rx) = mpsc::channel::<Message>(512);

    let is_running = Arc::new(AtomicBool::new(true));
    let is_running_ws_writer = Arc::clone(&is_running);

    // WebSocket Outbound Writer Task
    let ws_writer_task = tokio::spawn(async move {
        while let Some(msg) = ws_out_rx.recv().await {
            if !is_running_ws_writer.load(Ordering::Relaxed) {
                break;
            }
            if ws_tx.send(msg).await.is_err() {
                break;
            }
        }
    });

    // Send initial "init" message to prepare client canvas
    let _ = ws_out_tx
        .send(Message::Text(
            json!({
                "type": "init",
                "protocol": "rdp",
                "width": params.width,
                "height": params.height,
                "name": format!("RDP ({})", target_display)
            })
            .to_string(),
        ))
        .await;

    let input_sender_rx = input_sender.clone();
    let is_running_reader = Arc::clone(&is_running);
    let ws_out_tx_reader = ws_out_tx.clone();

    // Browser Inbound Input Task
    let ws_reader_task = tokio::spawn(async move {
        let mut prev_mouse_mask = 0u8;
        let mut prev_x = 0u16;
        let mut prev_y = 0u16;

        while let Some(Ok(msg)) = ws_rx.next().await {
            if !is_running_reader.load(Ordering::Relaxed) {
                break;
            }
            match msg {
                Message::Binary(data) => {
                    if data.is_empty() {
                        continue;
                    }
                    let packet_type = data[0];
                    if packet_type == 0x02 && data.len() >= 6 {
                        // Pointer event: [0x02, mask: u8, x: u16 (be), y: u16 (be)]
                        let mask = data[1];
                        let x = u16::from_be_bytes([data[2], data[3]]);
                        let y = u16::from_be_bytes([data[4], data[5]]);

                        let events = map_mouse_events(mask, prev_mouse_mask, x, y, prev_x, prev_y);
                        prev_mouse_mask = mask;
                        prev_x = x;
                        prev_y = y;

                        if !events.is_empty() {
                            let _ = input_sender_rx.send(RdpInputEvent::FastPath(events));
                        }
                    } else if packet_type == 0x04 && data.len() >= 6 {
                        // Key event: [0x04, down: u8, scancode: u8, extended: u8, unicode: u16 (be)]
                        let down = data[1] != 0;
                        let scancode = data[2];
                        let extended = data[3] != 0;
                        let unicode = u16::from_be_bytes([data[4], data[5]]);

                        if scancode != 0 {
                            let mut flags = if down {
                                KeyboardFlags::empty()
                            } else {
                                KeyboardFlags::RELEASE
                            };
                            if extended {
                                flags |= KeyboardFlags::EXTENDED;
                            }
                            let _ = input_sender_rx.send(RdpInputEvent::FastPath(smallvec![
                                FastPathInputEvent::KeyboardEvent(flags, scancode)
                            ]));
                        } else if unicode != 0 {
                            let flags = if down {
                                KeyboardFlags::empty()
                            } else {
                                KeyboardFlags::RELEASE
                            };
                            let _ = input_sender_rx.send(RdpInputEvent::FastPath(smallvec![
                                FastPathInputEvent::UnicodeKeyboardEvent(flags, unicode)
                            ]));
                        } else {
                            let keysym = u32::from_be_bytes([data[2], data[3], data[4], data[5]]);
                            let events = map_key_event(down, keysym);
                            if !events.is_empty() {
                                let _ = input_sender_rx.send(RdpInputEvent::FastPath(events));
                            }
                        }
                    }
                }
                Message::Text(txt) => {
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&txt) {
                        let msg_type = val.get("type").and_then(|v| v.as_str());
                        if msg_type == Some("ping") {
                            let _ = ws_out_tx_reader
                                .send(Message::Text(json!({"type": "pong"}).to_string()))
                                .await;
                        } else if msg_type == Some("resize") {
                            if let (Some(w), Some(h)) = (
                                val.get("width").and_then(|v| v.as_u64()),
                                val.get("height").and_then(|v| v.as_u64()),
                            ) {
                                let w = (w as u16).clamp(640, 3840);
                                let h = (h as u16).clamp(480, 2160);
                                info!("RDP Gateway: Dynamic resolution resize requested: {}x{}", w, h);
                                let _ = input_sender_rx.send(RdpInputEvent::Resize {
                                    width: w,
                                    height: h,
                                    scale_factor: 100,
                                    physical_size: None,
                                });
                            }
                        }
                    }
                }
                Message::Close(_) => {
                    let _ = input_sender_rx.send(RdpInputEvent::Close);
                    break;
                }
                _ => {}
            }
        }
        is_running_reader.store(false, Ordering::Relaxed);
    });

const TILE_SIZE: usize = 64;

/// Efficiently diff the current frame against the previous frame and send only dirty 64x64 tiles
async fn process_and_send_frame(
    ws_tx: &mpsc::Sender<Message>,
    prev_frame: &mut Vec<u32>,
    curr_frame: &[u32],
    w: usize,
    h: usize,
) -> Result<(), ()> {
    let total_pixels = w * h;
    if curr_frame.len() < total_pixels {
        return Ok(());
    }

    if prev_frame.len() != total_pixels {
        // First frame or size changed: send full frame and cache
        *prev_frame = curr_frame[..total_pixels].to_vec();

        let mut payload = Vec::with_capacity(9 + total_pixels * 4);
        payload.push(0x01); // Frame tile type
        payload.extend_from_slice(&0u16.to_be_bytes()); // x = 0
        payload.extend_from_slice(&0u16.to_be_bytes()); // y = 0
        payload.extend_from_slice(&(w as u16).to_be_bytes()); // width
        payload.extend_from_slice(&(h as u16).to_be_bytes()); // height

        for &pixel in &curr_frame[..total_pixels] {
            let bytes = pixel.to_be_bytes();
            payload.push(bytes[1]);
            payload.push(bytes[2]);
            payload.push(bytes[3]);
            payload.push(255);
        }

        if ws_tx.send(Message::Binary(payload)).await.is_err() {
            return Err(());
        }
        return Ok(());
    }

    let tiles_x = (w + TILE_SIZE - 1) / TILE_SIZE;
    let tiles_y = (h + TILE_SIZE - 1) / TILE_SIZE;

    let mut dirty_tiles: Vec<(usize, usize, usize, usize)> = Vec::new();

    for ty in 0..tiles_y {
        let tile_y = ty * TILE_SIZE;
        let tile_h = (h - tile_y).min(TILE_SIZE);

        for tx in 0..tiles_x {
            let tile_x = tx * TILE_SIZE;
            let tile_w = (w - tile_x).min(TILE_SIZE);

            let mut tile_changed = false;
            for row in 0..tile_h {
                let offset = (tile_y + row) * w + tile_x;
                if curr_frame[offset..offset + tile_w] != prev_frame[offset..offset + tile_w] {
                    tile_changed = true;
                    break;
                }
            }

            if tile_changed {
                dirty_tiles.push((tile_x, tile_y, tile_w, tile_h));
            }
        }
    }

    if dirty_tiles.is_empty() {
        return Ok(());
    }

    let total_tiles = tiles_x * tiles_y;

    // If more than 40% of the screen changed at once, send a single full frame
    if dirty_tiles.len() > (total_tiles * 2 / 5) {
        prev_frame[..total_pixels].copy_from_slice(&curr_frame[..total_pixels]);

        let mut payload = Vec::with_capacity(9 + total_pixels * 4);
        payload.push(0x01);
        payload.extend_from_slice(&0u16.to_be_bytes());
        payload.extend_from_slice(&0u16.to_be_bytes());
        payload.extend_from_slice(&(w as u16).to_be_bytes());
        payload.extend_from_slice(&(h as u16).to_be_bytes());

        for &pixel in &curr_frame[..total_pixels] {
            let bytes = pixel.to_be_bytes();
            payload.push(bytes[1]);
            payload.push(bytes[2]);
            payload.push(bytes[3]);
            payload.push(255);
        }

        if ws_tx.send(Message::Binary(payload)).await.is_err() {
            return Err(());
        }
        return Ok(());
    }

    // Otherwise, stream all dirty tiles packed into ONE single atomic batch packet (type 0x03)
    let mut total_tile_bytes = 3;
    for &(_, _, tw, th) in &dirty_tiles {
        total_tile_bytes += 8 + tw * th * 4;
    }

    let mut batch_pkt = Vec::with_capacity(total_tile_bytes);
    batch_pkt.push(0x03); // Batch Tiles Packet Type
    batch_pkt.extend_from_slice(&(dirty_tiles.len() as u16).to_be_bytes());

    for (tile_x, tile_y, tile_w, tile_h) in dirty_tiles {
        batch_pkt.extend_from_slice(&(tile_x as u16).to_be_bytes());
        batch_pkt.extend_from_slice(&(tile_y as u16).to_be_bytes());
        batch_pkt.extend_from_slice(&(tile_w as u16).to_be_bytes());
        batch_pkt.extend_from_slice(&(tile_h as u16).to_be_bytes());

        for row in 0..tile_h {
            let offset = (tile_y + row) * w + tile_x;
            for &pixel in &curr_frame[offset..offset + tile_w] {
                let bytes = pixel.to_be_bytes();
                batch_pkt.push(bytes[1]);
                batch_pkt.push(bytes[2]);
                batch_pkt.push(bytes[3]);
                batch_pkt.push(255);
            }
            prev_frame[offset..offset + tile_w].copy_from_slice(&curr_frame[offset..offset + tile_w]);
        }
    }

    if ws_tx.send(Message::Binary(batch_pkt)).await.is_err() {
        return Err(());
    }

    Ok(())
}

    let ws_out_tx_events = ws_out_tx.clone();
    let is_running_events = Arc::clone(&is_running);

    let target_display_clone = target_display.clone();

    // Process output events from the IronRDP client
    let output_loop = async move {
        let mut initial_size_sent = false;
        let mut current_w = 0u16;
        let mut current_h = 0u16;
        let mut prev_frame: Vec<u32> = Vec::new();

        while let Some(event) = output_rx.recv().await {
            if !is_running_events.load(Ordering::Relaxed) {
                break;
            }

            // Frame coalescing: drain intermediate graphics updates to immediately process latest frame
            let mut latest_event = event;
            while let Ok(newer) = output_rx.try_recv() {
                latest_event = newer;
            }

            match latest_event {
                RdpOutputEvent::Image { buffer, width, height } => {
                    let w = width.get() as usize;
                    let h = height.get() as usize;

                    if !initial_size_sent || (w as u16) != current_w || (h as u16) != current_h {
                        current_w = w as u16;
                        current_h = h as u16;
                        let _ = ws_out_tx_events
                            .send(Message::Text(
                                json!({
                                    "type": "init",
                                    "protocol": "rdp",
                                    "width": w,
                                    "height": h,
                                    "name": format!("RDP ({})", target_display_clone)
                                })
                                .to_string(),
                            ))
                            .await;
                        initial_size_sent = true;
                    }

                    if process_and_send_frame(&ws_out_tx_events, &mut prev_frame, &buffer, w, h).await.is_err() {
                        break;
                    }
                }
                RdpOutputEvent::ConnectionFailure(err) => {
                    error!("RDP Gateway: Connection failure: {}", err);
                    let _ = ws_out_tx_events
                        .send(Message::Text(
                            json!({
                                "type": "error",
                                "message": format!("RDP Connection Failed: {}", err)
                            })
                            .to_string(),
                        ))
                        .await;
                    break;
                }
                RdpOutputEvent::Terminated(reason) => {
                    match reason {
                        Ok(graceful) => {
                            info!("RDP Gateway: Session disconnected gracefully: {:?}", graceful);
                        }
                        Err(err) => {
                            warn!("RDP Gateway: Session terminated with error: {}", err);
                            let _ = ws_out_tx_events
                                .send(Message::Text(
                                    json!({
                                        "type": "error",
                                        "message": format!("RDP Session Terminated: {}", err)
                                    })
                                    .to_string(),
                                ))
                                .await;
                        }
                    }
                    break;
                }
                _ => {}
            }
        }
    };

    output_loop.await;

    is_running.store(false, Ordering::Relaxed);
    let _ = input_sender.send(RdpInputEvent::Close);

    ws_reader_task.abort();
    ws_writer_task.abort();

    info!("RDP Gateway: Session ended for {}", target_display);
}
