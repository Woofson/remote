# Changelog

All notable changes to Remote are documented here. The project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html) and the [Keep a Changelog](https://keepachangelog.com/en/1.0.0/) format.

---

## [0.2.0] - 2026-09-01

### RDP
- New RDP engine built on IronRDP: NLA/CredSSP, TLS and RDPGFX graphics decoding.
- Only changed 64×64 screen tiles are sent to the browser; a full frame is sent when more than 40% of the screen changes. Intermediate frames are dropped under load so the view stays current.
- Mouse: pointer movement is sent separately from button presses, fixing unreliable clicks, double-clicks and dragging. Pointer updates are throttled to 60 per second.
- Keyboard: browser keys are mapped to hardware scancodes, including Nordic ISO keys and AltGr combinations. Falls back to Unicode input for virtual and touch keyboards.
- Dynamic resolution: sessions start at the pane's size and resize live when panes or the browser window change size.
- Drive redirection: the staging folder appears in Windows as `\\tsclient\Dropbox`, toggled per connection.
- Performance presets: High Speed, Balanced, High Quality and Custom (wallpaper, window drag, animations, themes, font smoothing, audio, 16/32-bit colour).
- Disconnect screen with a one-click Reconnect button.

### Keyboard layouts
- Per-pane layout switcher: NO, US, GB, SE, DK, FI, DE, FR, ES, IT and Auto.
- Default layout per connection, remembered in the browser.

---

## [0.1.1] - 2026-09-01

### Interface
- Header toolbar redone with icon buttons and right-aligned layout switchers, matching the Woofson design used in Brum.
- Profile, user management, user edit and About dialogs reworked to match.
- Badge on the Dropboxes button showing active transfers.

### Users
- Profile pictures: cropped and resized to 160×160 in the browser and stored in the database, so backups and moves keep them.
- Users can change display name, email, password and picture; the username is fixed.
- Admins can disable and re-enable accounts, including the built-in `admin`. Disabled accounts are refused at login, session renewal and when opening sessions.

### Connections and access control
- Global (shared, admin-managed) and personal (private, per-user) connections.
- Per-connection policies: view-only mode; clipboard direction (both ways, to remote only, from remote only, off); file transfer (full, upload only, download only, off).
- Connection list with Global/Personal filters, search, and policy badges.

---

## [0.1.0] - 2026-09-01

### Initial release
- Single Rust binary (Axum, Tokio, bundled SQLite) with a browser interface using HTML5 canvas and WebSockets.
- Protocols: RDP, VNC (RFB 3.8), SSH and local shell.
- 1 to 4 pane layouts, switched with `Alt+1` to `Alt+4`.
- Argon2id password hashing, JWT sessions, role and permission tables, OpenID Connect login.
- Clipboard drawer, SFTP file browser and upload staging.
