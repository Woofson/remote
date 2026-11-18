# Changelog

All notable changes to Remote are documented here. The project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html) and the [Keep a Changelog](https://keepachangelog.com/en/1.0.0/) format.

---

## [0.8.1] - 2026-11-18

### Docker
- Official Alpine Linux container image: `ghcr.io/woofson/remote` (tags `latest`, `alpine`, `0.8.1`, `0.8`), linux/amd64. On arm64, build it yourself with `docker build -t remote .`.
- Built from source in a multi-stage `Dockerfile`; runs as `PUID`:`PGID` (default 1000:1000) instead of root, with a health check.
- One volume, `/remote`, holds `config.toml` (created on first start), `data/` and optional `themes/`. Example `docker-compose.yml` included; see the README's Docker section.
- Published automatically when a release tag is pushed.

### Maintenance
- Updated `crypto-bigint` (used by the RDP library) from a withdrawn version to 0.7.11.

---

## [0.8.0] - 2026-11-18

Covers everything since 0.2.0 (versions 0.3.0 to 0.7.0 were pushed to git without a release).

### Upgrading from 0.2.0: read first
- **Secret key moves to `data/secret.key`.** On first start Remote generates a private secret, re-encrypts saved connection passwords with it, and everyone is logged out once. **Back up `data/secret.key` together with `data/remote.db`**; without it saved connection passwords cannot be unlocked.
- **`config.toml` is no longer in the repository.** It was replaced by `config.example.toml`; your existing `config.toml` keeps working. On `git pull`, git removes a tracked `config.toml`: copy yours aside first, or Remote recreates one with defaults.
- **Admin password:** fresh installs generate a random admin password (shown in the console and saved to `data/initial_admin_credentials.txt`). An existing `admin` still using a published default (`remote`, or `remotedog` from the RemoteDog days) must choose a new password at next login. Lost it? Run `remoted reset-admin-password`.
- **Files:** the old shared "Dropbox" folder becomes **Files**. Its contents move into the new **Shared** folder on first start; every user gets a private **My files** folder.
- **Connections stay global** (visible to every user) until you make them private in the Admin Control Panel.
- New single sign-on users now get the **viewer** role by default.

### Security
- RDP drive redirection is guarded: paths sent by the remote machine can no longer escape the drive folder (`..`, alternate data streams, symlinks), so a compromised remote host cannot reach server files. Read-only drives refuse every write.
- Private, randomly generated secret instead of the published default; hardcoded fallback removed.
- SFTP checks connection permissions, the connection's file-transfer setting, view-only and role; refusals are audited.
- Explicit "may not connect" assignments are honoured.
- Unauthenticated file download and settings endpoints closed; the connection test needs Operator or Admin.
- Every request checks that the account is active and the login current; password changes, resets, disabling and role changes end other logins.
- Login no longer reveals whether an account exists or is disabled.
- Theme files and connection icons are validated (no CSS injection, PNG only, no SVG).

### Users and access
- Admins assign connections to users and groups with Connect, Edit, Clipboard and File transfer rights, and choose a connection's owner.
- Groups: create, rename, delete, members.
- Password reset (generated or set), change on next login, entered twice, minimum length (`min_password_length`, default 12), own change needs the current password.
- SSH asks for a password when none is saved; session errors are shown on the disconnected screen.

### Interface
- **Admin Control Panel:** Users & Groups, Connections (global warning, make private), Activity Log with filters, Server status.
- **Settings:** Account, Appearance, Shortcuts.
- **Themes:** Brum's 38 themes, per user, followed by terminals; extra themes from `themes/`.
- **Pane appearance** like Brum: name, two-tone borders, fade direction, width, ring and glow, saved per user.
- **Connections directory:** search, protocol/tag/scope filters, sorting; editor in tabs with checks before saving and a Test connection button.
- **Connection icons** from selfh.st/icons (downloaded and cached by the server) or uploaded.
- In-app dialogs and coloured toasts replace browser pop-ups.
- Version shown from the server; new Remote icon.

### Files
- Private **My files** per user and an admin-managed **Shared** folder (e.g. scripts), in the browser and in RDP as `\\tsclient\Files` and `\\tsclient\Shared`.
- Real file names, folders, upload/download/delete, large uploads streamed to disk.

### Protocols
- RDP drive redirection restored (Windows needs a sound channel to start drive sharing).
- SSH, Telnet and VNC accept hostnames and IPv6.
- VNC clipboard uses Latin-1 as the standard requires, so æ, ø and å work with UltraVNC and others.

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
