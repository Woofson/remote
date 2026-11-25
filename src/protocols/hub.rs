//! Session relay ("hub", #32): sits between a running session and the browsers watching it.
//!
//! Every session gets one. The protocol code (RDP, VNC, SSH, Telnet, local shell) talks to a
//! [`HubSocket`] exactly as it used to talk to the browser's WebSocket. The hub passes
//! everything to the owner's browser and, when the owner shares the session, copies the
//! picture or terminal output to the guests watching through a share link. Input from guests
//! is only passed on when their link allows control, and never resizes the session.

use axum::extract::ws::{CloseFrame, Message, WebSocket};
use futures_util::{Sink, SinkExt, Stream, StreamExt};
use parking_lot::RwLock;
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::sync::mpsc;
use tokio_util::sync::PollSender;
use tracing::info;

/// Messages queued for one guest before it counts as lagging (and gets a fresh picture later).
const GUEST_QUEUE: usize = 512;
/// Recent terminal output replayed to a guest who joins (bytes).
const TERM_REPLAY_BYTES: usize = 256 * 1024;
/// At most this many open share links per session.
const MAX_LINKS: usize = 10;
/// At most this many guests per session.
const MAX_GUESTS: usize = 20;

// ---------------------------------------------------------------------------------------------
// The protocol side
// ---------------------------------------------------------------------------------------------

/// What the protocol code talks to instead of the browser's WebSocket.
pub struct HubSocket {
    rx: mpsc::Receiver<Message>,
    tx: PollSender<Message>,
}

impl HubSocket {
    pub async fn send(&mut self, msg: Message) -> Result<(), axum::Error> {
        SinkExt::send(self, msg).await
    }

    pub async fn recv(&mut self) -> Option<Result<Message, axum::Error>> {
        self.rx.recv().await.map(Ok)
    }
}

impl Stream for HubSocket {
    type Item = Result<Message, axum::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().rx.poll_recv(cx).map(|m| m.map(Ok))
    }
}

impl Sink<Message> for HubSocket {
    type Error = axum::Error;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.get_mut().tx.poll_reserve(cx).map_err(axum::Error::new)
    }

    fn start_send(self: Pin<&mut Self>, item: Message) -> Result<(), Self::Error> {
        self.get_mut().tx.send_item(item).map_err(axum::Error::new)
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.get_mut().tx.close();
        Poll::Ready(Ok(()))
    }
}

// ---------------------------------------------------------------------------------------------
// Share links
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShareMode {
    View,
    Control,
}

impl ShareMode {
    pub fn as_str(self) -> &'static str {
        match self {
            ShareMode::View => "view",
            ShareMode::Control => "control",
        }
    }
}

/// A link that lets someone join a session. Lives only as long as the session.
#[derive(Clone)]
pub struct ShareLink {
    pub id: String,
    pub token: String,
    pub session_id: String,
    pub mode: ShareMode,
    /// Unix seconds; `None` means until the session ends.
    pub expires_at: Option<i64>,
    pub owner_name: String,
    pub conn_name: String,
    pub conn_icon: Option<String>,
    pub protocol: String,
    cmd: mpsc::Sender<HubCmd>,
}

impl ShareLink {
    fn expired(&self, now: i64) -> bool {
        self.expires_at.is_some_and(|t| now >= t)
    }

    /// What a guest may see before joining.
    pub fn public_view(&self) -> Value {
        json!({
            "owner": self.owner_name,
            "connection": self.conn_name,
            "icon": self.conn_icon,
            "protocol": self.protocol,
            "mode": self.mode.as_str(),
            "expires_at": self.expires_at,
        })
    }

    /// Hand a guest's WebSocket to the session. Fails when the session has ended.
    pub async fn attach(&self, guest: Guest) -> Result<(), ()> {
        self.cmd.send(HubCmd::Attach(guest, self.id.clone(), self.mode)).await.map_err(|_| ())
    }
}

/// All open share links, by token.
#[derive(Default)]
pub struct ShareRegistry {
    links: RwLock<HashMap<String, ShareLink>>,
}

impl ShareRegistry {
    /// A link that is still valid (expired ones are forgotten on the way).
    pub fn lookup(&self, token: &str) -> Option<ShareLink> {
        let now = now_secs();
        let mut links = self.links.write();
        links.retain(|_, l| !l.expired(now));
        links.get(token).cloned()
    }

    fn insert(&self, link: ShareLink) {
        self.links.write().insert(link.token.clone(), link);
    }

    fn remove(&self, token: &str) {
        self.links.write().remove(token);
    }

    fn remove_session(&self, session_id: &str) {
        self.links.write().retain(|_, l| l.session_id != session_id);
    }
}

/// Someone joining through a share link.
pub struct Guest {
    pub user_id: String,
    pub name: String,
    pub ws: WebSocket,
}

enum HubCmd {
    Attach(Guest, String, ShareMode),
}

/// Something worth recording in the activity log.
pub enum ShareEvent {
    Created { mode: ShareMode, minutes: Option<i64> },
    Revoked,
    Joined { guest_id: String, guest_name: String, mode: ShareMode },
    Left { guest_id: String, guest_name: String },
    Removed { guest_name: String },
}

// ---------------------------------------------------------------------------------------------
// The hub
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionKind {
    /// RDP and VNC: a picture.
    Graphics,
    /// SSH, Telnet, local shell: a text stream.
    Terminal,
}

pub struct HubConfig {
    pub owner_name: String,
    pub conn_name: String,
    pub conn_icon: Option<String>,
    pub protocol: String,
    pub kind: SessionKind,
    pub cols: u16,
    pub rows: u16,
    /// Guests with control may paste into the session (the connection allows browser to remote).
    pub guest_clipboard: bool,
    /// Sharing allowed at all, and with control: asked each time, so admin changes apply
    /// to sessions that are already open.
    pub share_rights: Box<dyn Fn() -> (bool, bool) + Send + Sync>,
    /// Called for share events (activity log).
    pub on_event: Box<dyn Fn(ShareEvent) + Send + Sync>,
}

/// Put a hub between the owner's WebSocket and the session. The protocol code gets the
/// returned socket.
pub fn start(owner_ws: WebSocket, cfg: HubConfig, registry: Arc<ShareRegistry>) -> HubSocket {
    let (proto_in_tx, proto_in_rx) = mpsc::channel::<Message>(256);
    let (proto_out_tx, proto_out_rx) = mpsc::channel::<Message>(64);
    tokio::spawn(run(owner_ws, cfg, registry, proto_in_tx, proto_out_rx));
    HubSocket { rx: proto_in_rx, tx: PollSender::new(proto_out_tx) }
}

struct GuestConn {
    user_id: String,
    name: String,
    mode: ShareMode,
    link_id: String,
    tx: mpsc::Sender<Message>,
    visible: bool,
    lagging: bool,
}

struct Hub {
    session_id: String,
    cfg: HubConfig,
    registry: Arc<ShareRegistry>,
    proto_in: mpsc::Sender<Message>,
    owner_tx: mpsc::Sender<Message>,
    cmd_tx: mpsc::Sender<HubCmd>,
    guests: HashMap<u64, GuestConn>,
    next_guest: u64,
    links: Vec<ShareLink>,
    owner_visible: bool,
    sent_visible: bool,
    last_init: Option<Message>,
    term_buf: VecDeque<Message>,
    term_buf_bytes: usize,
    cols: u16,
    rows: u16,
}

async fn run(
    owner_ws: WebSocket,
    cfg: HubConfig,
    registry: Arc<ShareRegistry>,
    proto_in: mpsc::Sender<Message>,
    mut proto_out: mpsc::Receiver<Message>,
) {
    let (mut owner_sink, mut owner_stream) = owner_ws.split();
    let (owner_tx, mut owner_rx) = mpsc::channel::<Message>(256);
    tokio::spawn(async move {
        while let Some(m) = owner_rx.recv().await {
            let closing = matches!(m, Message::Close(_));
            if owner_sink.send(m).await.is_err() || closing {
                break;
            }
        }
        let _ = owner_sink.close().await;
    });

    let (cmd_tx, mut cmd_rx) = mpsc::channel::<HubCmd>(16);
    let (guest_in_tx, mut guest_in_rx) = mpsc::channel::<(u64, Option<Message>)>(256);

    let mut hub = Hub {
        session_id: new_token(12),
        cols: cfg.cols,
        rows: cfg.rows,
        cfg,
        registry,
        proto_in,
        owner_tx,
        cmd_tx,
        guests: HashMap::new(),
        next_guest: 1,
        links: Vec::new(),
        owner_visible: true,
        sent_visible: true,
        last_init: None,
        term_buf: VecDeque::new(),
        term_buf_bytes: 0,
    };

    let (can_share, can_share_control) = (hub.cfg.share_rights)();
    let _ = hub
        .owner_tx
        .send(Message::Text(
            json!({
                "type": "session",
                "id": hub.session_id,
                "can_share": can_share,
                "can_share_control": can_share_control,
            })
            .to_string(),
        ))
        .await;

    let mut expiry_tick = tokio::time::interval(std::time::Duration::from_secs(20));
    loop {
        tokio::select! {
            out = proto_out.recv() => match out {
                Some(m) => {
                    if hub.from_session(m).await {
                        break;
                    }
                }
                None => break,
            },
            inp = owner_stream.next() => match inp {
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => {
                    // The owner left: the session ends (the protocol sees its input close).
                    break;
                }
                Some(Ok(m)) => hub.from_owner(m).await,
            },
            Some((gid, m)) = guest_in_rx.recv() => hub.from_guest(gid, m).await,
            Some(cmd) = cmd_rx.recv() => match cmd {
                HubCmd::Attach(guest, link_id, mode) => hub.attach(guest, link_id, mode, guest_in_tx.clone()).await,
            },
            _ = expiry_tick.tick() => hub.prune_links().await,
        }
        hub.resync_lagging().await;
    }

    hub.finish();
}

impl Hub {
    // ---- output from the session ----

    /// Returns true when the session has closed.
    async fn from_session(&mut self, m: Message) -> bool {
        let mut owner_only = false;
        let mut closing = false;
        match &m {
            Message::Text(t) => match control_type(t).as_deref() {
                Some("init") => self.last_init = Some(m.clone()),
                // Clipboard contents and password prompts belong to the owner.
                Some("clipboard_sync") | Some("auth_required") => owner_only = true,
                Some(_) => {}
                None => self.remember_terminal_output(&m),
            },
            Message::Binary(_) => {
                if self.cfg.kind == SessionKind::Terminal {
                    self.remember_terminal_output(&m);
                }
            }
            Message::Close(_) => closing = true,
            _ => {}
        }
        if !owner_only {
            self.to_guests(&m);
        }
        if closing {
            for g in self.guests.values() {
                let _ = g.tx.try_send(close_msg("The shared session has ended"));
            }
        }
        let _ = self.owner_tx.send(m).await;
        closing
    }

    fn remember_terminal_output(&mut self, m: &Message) {
        if self.cfg.kind != SessionKind::Terminal {
            return;
        }
        let len = match m {
            Message::Text(t) => t.len(),
            Message::Binary(b) => b.len(),
            _ => return,
        };
        self.term_buf.push_back(m.clone());
        self.term_buf_bytes += len;
        while self.term_buf_bytes > TERM_REPLAY_BYTES {
            match self.term_buf.pop_front() {
                Some(Message::Text(t)) => self.term_buf_bytes -= t.len(),
                Some(Message::Binary(b)) => self.term_buf_bytes -= b.len(),
                Some(_) => {}
                None => break,
            }
        }
    }

    fn to_guests(&mut self, m: &Message) {
        for g in self.guests.values_mut() {
            if g.lagging {
                continue;
            }
            if g.tx.try_send(m.clone()).is_err() {
                // Too slow to follow: skip ahead and give it a fresh picture once it catches up.
                g.lagging = true;
            }
        }
    }

    /// Guests that fell behind get a fresh start once their queue has emptied.
    async fn resync_lagging(&mut self) {
        let ready: Vec<u64> = self
            .guests
            .iter()
            .filter(|(_, g)| g.lagging && g.tx.capacity() == GUEST_QUEUE)
            .map(|(id, _)| *id)
            .collect();
        for id in ready {
            if let Some(g) = self.guests.get_mut(&id) {
                g.lagging = false;
            }
            self.send_start_state(id, true).await;
        }
    }

    // ---- input from the owner ----

    async fn from_owner(&mut self, m: Message) {
        if let Message::Text(t) = &m {
            if let Some(v) = parse_control(t) {
                match v.get("type").and_then(|x| x.as_str()) {
                    Some("visibility") => {
                        self.owner_visible = v.get("visible").and_then(|x| x.as_bool()).unwrap_or(true);
                        self.update_visibility().await;
                        return;
                    }
                    Some("resize") => {
                        if let (Some(c), Some(r)) = (v.get("cols").and_then(|x| x.as_u64()), v.get("rows").and_then(|x| x.as_u64())) {
                            self.cols = c.min(u16::MAX as u64) as u16;
                            self.rows = r.min(u16::MAX as u64) as u16;
                            if self.cfg.kind == SessionKind::Terminal {
                                let size = self.term_size_msg();
                                self.to_guests(&size);
                            }
                        }
                    }
                    Some("share_create") => {
                        self.create_link(&v).await;
                        return;
                    }
                    Some("share_revoke") => {
                        if let Some(id) = v.get("link").and_then(|x| x.as_str()) {
                            self.revoke_link(id);
                            self.push_share_state().await;
                        }
                        return;
                    }
                    Some("share_kick") => {
                        if let Some(id) = v.get("viewer").and_then(|x| x.as_u64()) {
                            if let Some(g) = self.guests.remove(&id) {
                                let _ = g.tx.try_send(close_msg("The owner removed you from this session"));
                                (self.cfg.on_event)(ShareEvent::Removed { guest_name: g.name });
                                self.update_visibility().await;
                                self.push_share_state().await;
                            }
                        }
                        return;
                    }
                    _ => {}
                }
            }
        }
        if matches!(m, Message::Ping(_) | Message::Pong(_)) {
            return;
        }
        let _ = self.proto_in.send(m).await;
    }

    // ---- input from guests ----

    async fn from_guest(&mut self, gid: u64, m: Option<Message>) {
        let Some(m) = m.filter(|m| !matches!(m, Message::Close(_))) else {
            if let Some(g) = self.guests.remove(&gid) {
                (self.cfg.on_event)(ShareEvent::Left { guest_id: g.user_id, guest_name: g.name });
                self.update_visibility().await;
                self.push_share_state().await;
            }
            return;
        };
        let Some(g) = self.guests.get_mut(&gid) else { return };
        let control = g.mode == ShareMode::Control;
        match &m {
            Message::Text(t) => {
                if let Some(v) = parse_control(t) {
                    match v.get("type").and_then(|x| x.as_str()) {
                        Some("visibility") => {
                            g.visible = v.get("visible").and_then(|x| x.as_bool()).unwrap_or(true);
                            self.update_visibility().await;
                            return;
                        }
                        // Only the owner sizes the session, answers password prompts and shares.
                        Some("resize") | Some("auth") | Some("auth_cancel") => return,
                        Some(t) if t.starts_with("share_") => return,
                        Some("clipboard_push") if !(control && self.cfg.guest_clipboard) => return,
                        Some("refresh") => {}
                        _ if !control => return,
                        _ => {}
                    }
                } else if !control {
                    return;
                }
            }
            Message::Binary(_) if !control => return,
            Message::Binary(_) => {}
            _ => return,
        }
        let _ = self.proto_in.send(m).await;
    }

    /// Screen updates pause only when nobody can see the session (#7).
    async fn update_visibility(&mut self) {
        if self.cfg.kind != SessionKind::Graphics {
            return;
        }
        let visible = self.owner_visible || self.guests.values().any(|g| g.visible);
        if visible != self.sent_visible {
            self.sent_visible = visible;
            let _ = self.proto_in.send(Message::Text(json!({ "type": "visibility", "visible": visible }).to_string())).await;
        }
    }

    // ---- guests joining ----

    async fn attach(&mut self, guest: Guest, link_id: String, mode: ShareMode, guest_in: mpsc::Sender<(u64, Option<Message>)>) {
        let Guest { user_id, name, ws } = guest;
        let (mut sink, mut stream) = ws.split();
        if self.guests.len() >= MAX_GUESTS || !self.links.iter().any(|l| l.id == link_id) {
            let _ = sink.send(close_msg("This share link is no longer valid")).await;
            return;
        }
        let gid = self.next_guest;
        self.next_guest += 1;

        let (tx, mut rx) = mpsc::channel::<Message>(GUEST_QUEUE);
        tokio::spawn(async move {
            while let Some(m) = rx.recv().await {
                let closing = matches!(m, Message::Close(_));
                if sink.send(m).await.is_err() || closing {
                    break;
                }
            }
            let _ = sink.close().await;
        });
        tokio::spawn(async move {
            while let Some(m) = stream.next().await {
                match m {
                    Ok(m) => {
                        if guest_in.send((gid, Some(m))).await.is_err() {
                            return;
                        }
                    }
                    Err(_) => break,
                }
            }
            let _ = guest_in.send((gid, None)).await;
        });

        (self.cfg.on_event)(ShareEvent::Joined { guest_id: user_id.clone(), guest_name: name.clone(), mode });
        self.guests.insert(gid, GuestConn { user_id, name, mode, link_id, tx, visible: true, lagging: false });
        self.send_start_state(gid, false).await;
        self.update_visibility().await;
        self.push_share_state().await;
    }

    /// What a guest needs to see the session as it is now.
    async fn send_start_state(&mut self, gid: u64, resync: bool) {
        let Some(g) = self.guests.get(&gid) else { return };
        let tx = g.tx.clone();
        if !resync {
            let _ = tx.try_send(Message::Text(
                json!({
                    "type": "shared",
                    "owner": self.cfg.owner_name,
                    "mode": g.mode.as_str(),
                    "protocol": self.cfg.protocol,
                })
                .to_string(),
            ));
        }
        match self.cfg.kind {
            SessionKind::Graphics => {
                if let Some(init) = &self.last_init {
                    let _ = tx.try_send(init.clone());
                }
                // A full redraw reaches everyone, including the new guest.
                let _ = self.proto_in.send(Message::Text(json!({ "type": "refresh" }).to_string())).await;
            }
            SessionKind::Terminal => {
                if resync {
                    let _ = tx.try_send(Message::Text(json!({ "type": "term_reset" }).to_string()));
                }
                let _ = tx.try_send(self.term_size_msg());
                for m in &self.term_buf {
                    if tx.try_send(m.clone()).is_err() {
                        break;
                    }
                }
                // Nudge full-screen programs (tmux, editors) into redrawing.
                let (cols, rows) = (self.cols, self.rows);
                if rows > 2 {
                    let proto = self.proto_in.clone();
                    tokio::spawn(async move {
                        let resize = |r: u16| Message::Text(json!({ "type": "resize", "cols": cols, "rows": r }).to_string());
                        let _ = proto.send(resize(rows - 1)).await;
                        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
                        let _ = proto.send(resize(rows)).await;
                    });
                }
            }
        }
    }

    fn term_size_msg(&self) -> Message {
        Message::Text(json!({ "type": "term_size", "cols": self.cols, "rows": self.rows }).to_string())
    }

    // ---- links ----

    async fn create_link(&mut self, v: &Value) {
        let control = v.get("mode").and_then(|x| x.as_str()) == Some("control");
        let (can_share, can_share_control) = (self.cfg.share_rights)();
        let refuse = if !can_share {
            Some("Sharing is turned off by the administrator")
        } else if control && !can_share_control {
            Some("Only watch-only links are allowed by the administrator")
        } else if self.links.len() >= MAX_LINKS {
            Some("Too many share links for this session")
        } else {
            None
        };
        if let Some(msg) = refuse {
            let _ = self.owner_tx.send(Message::Text(json!({ "type": "share_error", "message": msg }).to_string())).await;
            return;
        }
        let mode = if control { ShareMode::Control } else { ShareMode::View };
        let minutes = v.get("minutes").and_then(|x| x.as_i64()).filter(|m| *m > 0).map(|m| m.min(7 * 24 * 60));
        let link = ShareLink {
            id: new_token(6),
            token: new_token(24),
            session_id: self.session_id.clone(),
            mode,
            expires_at: minutes.map(|m| now_secs() + m * 60),
            owner_name: self.cfg.owner_name.clone(),
            conn_name: self.cfg.conn_name.clone(),
            conn_icon: self.cfg.conn_icon.clone(),
            protocol: self.cfg.protocol.clone(),
            cmd: self.cmd_tx.clone(),
        };
        self.registry.insert(link.clone());
        self.links.push(link);
        (self.cfg.on_event)(ShareEvent::Created { mode, minutes });
        self.push_share_state().await;
    }

    /// Cancel a link: nobody can join with it any more, and its guests are disconnected.
    fn revoke_link(&mut self, id: &str) {
        let Some(pos) = self.links.iter().position(|l| l.id == id) else { return };
        let link = self.links.remove(pos);
        self.registry.remove(&link.token);
        let gone: Vec<u64> = self.guests.iter().filter(|(_, g)| g.link_id == link.id).map(|(k, _)| *k).collect();
        for gid in gone {
            if let Some(g) = self.guests.remove(&gid) {
                let _ = g.tx.try_send(close_msg("The share link was cancelled"));
            }
        }
        (self.cfg.on_event)(ShareEvent::Revoked);
    }

    /// Expired links can no longer be used to join (guests already in stay).
    async fn prune_links(&mut self) {
        let now = now_secs();
        let before = self.links.len();
        let registry = self.registry.clone();
        self.links.retain(|l| {
            let keep = !l.expired(now);
            if !keep {
                registry.remove(&l.token);
            }
            keep
        });
        if self.links.len() != before {
            self.push_share_state().await;
        }
    }

    async fn push_share_state(&mut self) {
        let links: Vec<Value> = self
            .links
            .iter()
            .map(|l| json!({ "id": l.id, "token": l.token, "mode": l.mode.as_str(), "expires_at": l.expires_at }))
            .collect();
        let mut viewers: Vec<(u64, &GuestConn)> = self.guests.iter().map(|(k, g)| (*k, g)).collect();
        viewers.sort_by_key(|(k, _)| *k);
        let viewers: Vec<Value> = viewers
            .iter()
            .map(|(id, g)| json!({ "id": id, "name": g.name, "mode": g.mode.as_str() }))
            .collect();
        let _ = self
            .owner_tx
            .send(Message::Text(json!({ "type": "share_state", "links": links, "viewers": viewers }).to_string()))
            .await;
    }

    fn finish(&mut self) {
        self.registry.remove_session(&self.session_id);
        for (_, g) in self.guests.drain() {
            let _ = g.tx.try_send(close_msg("The shared session has ended"));
        }
        if !self.links.is_empty() {
            info!("Session relay: shared session ended, {} link(s) closed", self.links.len());
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------------

/// The `type` of a JSON control message, or `None` for terminal output / input.
fn control_type(t: &str) -> Option<String> {
    parse_control(t).and_then(|v| v.get("type").and_then(|x| x.as_str()).map(str::to_string))
}

fn parse_control(t: &str) -> Option<Value> {
    if !t.starts_with('{') {
        return None;
    }
    serde_json::from_str::<Value>(t).ok().filter(|v| v.get("type").is_some_and(|x| x.is_string()))
}

fn close_msg(reason: &'static str) -> Message {
    Message::Close(Some(CloseFrame { code: 4000, reason: reason.into() }))
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Random URL-safe token of `bytes` random bytes.
fn new_token(bytes: usize) -> String {
    use base64::Engine as _;
    let mut buf = vec![0u8; bytes];
    getrandom::getrandom(&mut buf).expect("random source");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_messages_are_told_apart_from_terminal_text() {
        assert_eq!(control_type(r#"{"type":"init","width":1}"#).as_deref(), Some("init"));
        assert_eq!(control_type("ls -la\r\n"), None);
        assert_eq!(control_type(r#"{"no_type":1}"#), None);
        assert_eq!(control_type("{ not json"), None);
    }

    #[test]
    fn tokens_are_url_safe_and_unique() {
        let a = new_token(24);
        let b = new_token(24);
        assert_ne!(a, b);
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }
}
