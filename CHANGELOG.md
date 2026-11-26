# Changelog

All notable changes to Remote are documented here. The project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html) and the [Keep a Changelog](https://keepachangelog.com/en/1.0.0/) format.

---

## [0.22.0] - 2026-11-26

### Added
- **Remember this device for two-factor sign-in.** Admins choose under Admin > Security how many days (1 to 90; off by default). With it on, the code step offers "Remember this device", and that browser then skips the code at sign-in and on the lock screen; the password is always needed. A remembered device ends when the days run out, the password changes, two-factor is turned off or reset, or the setting is turned off. Settings > Security lists your remembered devices with Forget and Forget all; admins can forget a user's devices from the Users page. Skipped codes are recorded in the activity log.

### Fixed
- Refreshing the page no longer briefly shows the signed-in app before the login or lock screen.
- A login that has ended (lifetime reached, password changed or reset, account disabled, signed out elsewhere) now always returns to the login screen with the reason. An open page checks every minute and when you come back to it.
- The lock screen appears straight away in a new tab when the login is locked.
- Signing out and locking now survive a server restart.

---

## [0.21.1] - 2026-11-25

Covers everything since 0.14.0 (versions 0.15.0 to 0.21.0 were pushed to git without a release).

### Upgrading from 0.14.0: read first
- **Session sharing is allowed by default.** Users can share a session they have open through a link (viewers sign in with their own Remote login). Change it under Admin > Security: watch-only links, or off.
- **Session recording is off until an admin turns it on**, per connection (connection editor, Access tab) or per group (Users & Groups). Sharing recordings is off by default too.
- **New optional `[recording]` section** in `config.toml`: folder (default `data/recordings`), days to keep (30) and total size limit (20 GB). Without it these defaults apply.
- **Docker:** recordings live in `./remote/data/recordings`, inside the folder you already back up. `docker-compose.yml` shows how to put them on another disk or NAS.

### Added
- **Tabs inside a pane** (#7): several sessions per pane, "+" for a new tab, drag tabs to reorder them or move them to another pane without reconnecting. Open tabs are remembered on your account and come back unconnected after sign-in.
- **Floating windows** (#8): float any session out of the grid into a window that can be moved, resized, minimised and maximised, and dock it back as a tab. Window positions are remembered.
- **Share a live session** (#32): from the pane's share menu, watch only or with control, with a time limit; see and remove viewers. Viewers join the same session with their own login. Admin rule under Security.
- **Session recording** (#31): terminal and desktop (RDP, VNC; picture only) sessions are recorded as the browser showed them, never keystrokes, and stored encrypted. A REC marker shows while recording. Admin > Recordings lists every recording with search inside terminal recordings, play, download, share and delete; Settings > Recordings shows your own and those shared with you. The player has a timeline, 1x to 8x speed, skip pauses and find text; desktop recordings can be saved as a video (.webm). Sessions that must be recorded are refused if recording cannot start. Everything is written to the activity log.
- **Phones and tablets** (#33): touchpad or touchscreen mode for remote desktops, pinch to zoom, and a key bar with Esc, Tab, Ctrl, Alt, Win, arrows, F-keys and Ctrl+Alt+Del, also for terminals.
- **Wake-on-LAN** (#35): wake a sleeping machine before connecting, or with the Wake button in the connections list.
- **Keyboard layout for RDP** (#6): sessions start with the chosen layout ("Auto" follows the browser language); the pane's layout button switches it.
- A waiting notice while a remote screen connects or redraws after a resize, instead of a black screen.
- Hidden remote desktops (background tab, hidden pane) pause their screen updates to save bandwidth.

### Fixed
- Start menu > Disconnect in Windows no longer makes Remote reconnect, also when many screen updates were still on their way.
- RDP sessions no longer drop after a resize (F10, F11, pane or window size).
- VNC: typing after a pane resize, dead keys and stuck keys after Shift.
- The connection editor kept replacing a saved custom port with the protocol's default.

---

## [0.14.0] - 2026-11-25

### Added
- **Sound from RDP sessions** plays in the browser (#25). Turn on "Audio" in the connection settings; a speaker button in the pane header mutes it. If the browser blocks sound at first, one click or key press starts it.
- **Direct links to connections** (#29): "Copy link" on a connection gives an address that opens it straight away, for bookmarks and desktop shortcuts. You are asked to sign in first if needed.
- **Login and lock screen branding** (#30, Admin > Branding): your own logo, title, welcome text, accent colour and wallpapers (one picked at random per visit), with a one-click reset to Remote's defaults. Changes are recorded in the activity log.
- **Movable and hideable toolbar** in full window and full screen: drag it by its grip (double-click the grip to put it back), or hide it completely. Bring it back with Alt+T or by holding the mouse at the top edge of the screen for a second. Remembered on your account.
- The version in the corner now includes the build number.

### Fixed
- RDP display settings were ignored (#28): wallpaper, showing window contents while dragging, menu animations, themes and font smoothing now follow the connection settings.

### Changed
- The login and lock screens match Brum's look.
- The RDP client code is now part of Remote (adapted from IronRDP, MIT licensed) so these settings and sound can be controlled; see `docs/rdp-client.md`.

---

## [0.13.1] - 2026-11-19

Covers everything since 0.8.1 (versions 0.9.0 to 0.13.0 were pushed to git without a release).

### Upgrading from 0.8.1: read first
- **Login lifetime moves to Admin > Security.** `token_expiry_hours` in `config.toml` is now only the starting value; change it in the web interface.
- **Logging out now ends the login on the server.** A copied login token stops working at log out.
- **The lock screen is enforced by the server.** While locked, nothing new can be opened; running sessions keep going.
- **Cross-site access (CORS) is switched off.** Remote's own pages are unaffected; scripts on other websites can no longer call Remote's API from a browser.
- **The login cookie is limited to Remote's own path** and marked secure on HTTPS. You may need to sign in once.
- **New optional settings** in `config.toml`: `base_path` and `trusted_proxies` under `[server]`, and `[auth.header_login]`. Without them Remote behaves as before; see `config.example.toml` and `docs/reverse-proxy.md`.

### Security
- **Two-factor authentication** (TOTP) for local accounts: setup with a QR code in Settings > Security, one-time recovery codes, codes cannot be reused, secrets encrypted at rest.
- **Admin session rules** (Admin > Security): require 2FA per role, login lifetime, idle timeout that locks the screen or signs out, optional 2FA code on the lock screen.
- Repeated wrong passwords or codes are slowed down, per account and per address.
- Failed sign-ins, 2FA changes, policy changes and email settings changes are recorded in the activity log, now with the visitor's IP address.
- Admins can reset a user's 2FA (lost phone), which also ends that user's logins.
- Local shells no longer inherit SSH and tmux markers from the Remote server process.

### Added
- **Email** (Admin > Email): SMTP settings with a test email; "Forgot password?" on the login screen with a single-use link (30 minutes); admins can email set-your-password links; users are told when their password or 2FA settings change.
- **Reverse proxy support**: hosting under a sub-path, keepalive pings so proxies do not close idle sessions, automatic reconnect for dropped RDP/VNC sessions, real client IPs from trusted proxies, optional sign-in from Authelia/Authentik forward-auth headers, `/api/health` and `/api/ready`. Guide with tested Nginx, Caddy, Traefik and HAProxy examples: `docs/reverse-proxy.md`.
- **Shell choice** for local shell connections (fish, bash, zsh and other installed login shells).
- **Layout menu** in the header, like Brum's; the last layout is remembered on your account.
- Settings follow you between devices (stored on your account).
- New app icon; README with screenshots.

### Changed
- Toasts appear top-centre with a themed glass look.
- Pane header buttons and the pane badge match Brum's style; outline icons throughout.
- The pane settings popup sizes itself to the window instead of scrolling.
- Pane headers show "Local shell" instead of LOCAL_PTY.
- Plain wording in the About dialog, page title and package description.

### Fixed
- Terminal sizing for full-screen programs (tmux, editors, Claude Code): no stretched text, correct rows and columns, font measured after loading.
- Behind a slow proxy, the shell was told a rough first guess of the terminal size, garbling tmux and similar programs; the real size is now sent once the session connects.
- The built-in "Local Host Terminal" on fresh installs was refused after the shell-choice change.

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
