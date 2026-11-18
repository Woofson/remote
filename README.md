# <img src="assets/RemoteIcon-256.png" alt="Remote logo" height="40" style="vertical-align: -6px; margin-right: 8px;" /> Remote

A browser-based remote access gateway for RDP, VNC, SSH, Telnet and local shells. One Rust binary with the web interface built in. No Java, Tomcat or `guacd`.

By Bolt J Woofson @ Woofsons Lab. Free and open source; if it is useful to you, you can [support development on Ko-fi](https://ko-fi.com/J7P327BXTR).

---

## Features

**Protocols**
- **RDP**: built on IronRDP, with NLA/CredSSP and TLS, `DOMAIN\user` logins, clipboard sync, dynamic resolution on resize, and performance presets (High Speed, Balanced, High Quality, Custom).
- **VNC (RFB 3.8)**: Raw, CopyRect, Hextile, Zlib and ZRLE encodings, VNC password authentication, VeNCrypt TLS/X.509, and clipboard.
- **SSH**: terminal with password, key or OpenSSH certificate login, plus SFTP file browsing.
- **Telnet**.
- **Local shell** on the server host (bash/sh, or PowerShell on Windows).

**Workspace**
- 1 to 4 panes side by side (Single, Dual, Triple, Quad), switched with `Alt+1` to `Alt+4`.
- Full window mode (`F10` or `Alt+W`) and a session lock screen.
- Terminal based on xterm.js with a bundled JetBrainsMono Nerd Font.
- Per-pane keyboard layout switcher (NO, US, GB, SE, DK, FI, DE, FR, ES, IT, Auto).
- Clipboard drawer for pushing text to and pulling text from remote sessions.
- **Themes**: the same theme files as Brum (Woofsons Amber, Catppuccin, Nord, Dracula, Tokyo Night, Gruvbox, Solarized and more), chosen per user under Settings > Appearance and followed by terminals. Extra `*.toml` theme files can be dropped into `themes/` or `data/themes/`.
- **Files**: every user has a private folder ("My files"), and there is a **Shared** folder everyone can read and only admins can change, e.g. for scripts and installers. Browse, upload, download and organise both from the browser. RDP sessions with drive redirection see them as `\\tsclient\Files` and `\\tsclient\Shared` (read-only for non-admins).

**Users and access**
- Local accounts in an embedded SQLite database, with Argon2id password hashing and JWT sessions.
- Roles: Admin, Operator, Viewer. Per-connection permissions for connecting, editing, clipboard direction, file transfer and view-only mode.
- Global and private connections; admins choose a connection's owner and assign it to users and groups. Stored connection passwords are encrypted with AES-256-GCM.
- **Connections directory** with search, protocol/tag/scope filters and sorting, and an editor in tabs (General, Login, Display, Access) with a **Test connection** button. Connection **icons** can be picked from [selfh.st/icons](https://selfh.st/icons/) (downloaded and cached by the server, so browsers never contact selfh.st) or uploaded.
- OpenID Connect login (Authentik, Keycloak and others) with automatic user creation and group-to-role mapping.
- **Admin Control Panel**: users and groups, all connections (with a warning for global ones), the activity log with filters, and server status. Personal **Settings**: account, appearance and keyboard shortcuts.

---

## Quick start

```bash
cargo run --release
```

Open `http://localhost:8088` and log in as `admin`. On first start Remote generates a random password for it, shows it in the console and saves it to `data/initial_admin_credentials.txt` (readable only by you). You choose your own password right after signing in, and the file is then deleted.

Lost the admin password? On the Remote machine run `remoted reset-admin-password` (or `cargo run --release -- reset-admin-password`): it prints a new password and saves it to the same file.

### Docker (Alpine)

```bash
docker run -d --name remote --restart unless-stopped \
  -p 8088:8088 -e PUID=$(id -u) -e PGID=$(id -g) \
  -v ./remote:/remote ghcr.io/woofson/remote:latest
```

Or use the included [`docker-compose.yml`](docker-compose.yml). The `./remote` folder holds everything Remote keeps: `config.toml` (created on first start), `data/` (database, `secret.key`, Files, icons) and an optional `themes/` folder, so back up that one folder. The first admin password is shown by `docker logs remote` and saved in `remote/data/initial_admin_credentials.txt`. Lost it: `docker exec remote remoted reset-admin-password`.

- Remote runs as the user `PUID`:`PGID` (default 1000:1000), not root.
- Images are built for **linux/amd64**. On arm64, build it yourself with `docker build -t remote .`.
- The built-in *Local Host Terminal* opens a shell inside the container, not on the Docker host; delete it or make it private if you do not need it.
- The machines you connect to must be reachable from the container (normal Docker networking is enough when the Docker host can reach them).

---

## Configuration

Settings live in `config.toml` in the working directory. Start from `config.example.toml`; if there is no `config.toml`, Remote creates one with defaults. `config.toml` is ignored by git.

| Section | Key settings |
| :--- | :--- |
| `[server]` | `bind_address` (default `0.0.0.0:8088`), `data_dir`, `token_expiry_hours`, optional `jwt_secret` |
| `[storage]` | `staging_dir`, `max_upload_size_mb`, `auto_cleanup_hours` |
| `[auth]` | `allow_local_login`, `default_role` (default `viewer`), `min_password_length` (default `12`) |
| `[oidc]` | `enabled`, `issuer_url`, `client_id`, `client_secret`, `redirect_uri`, `scopes`, `auto_provision_users`, `admin_group`, `operator_group` |
| `[clipboard]` | `default_mode` (`bidirectional`, `server_to_client`, `client_to_server`, `disabled`), `max_text_size_bytes` |
| `[logging]` | `level` |

### Secret key and backups

On first start Remote generates a random secret in `data/secret.key` (readable only by its owner). It signs login sessions and encrypts saved connection passwords and keys. Back it up together with `data/remote.db`: without it, saved connection credentials cannot be unlocked and must be re-entered. To use your own secret instead, set a private `jwt_secret` in `config.toml`.

Older installations that still used the published default secret are switched to `data/secret.key` automatically; saved credentials are re-encrypted and everyone is logged out once.

### Passwords

Admins can reset a user's password from the Users dialog (generate one, shown once, or set one) and require a new password at next login; new users choose their own password at first login by default. Changing your own password asks for the current one. Every password change, reset, disable or role change ends that user's other logins immediately.

Before exposing Remote to a network, review `default_role`.

---

## Project layout

```
src/
  main.rs            Startup and first-run admin account
  config/            Configuration loader
  db/                SQLite storage: users, groups, connections, audit log
  auth/              Password hashing, sessions, encryption, OIDC
  server/            HTTP API and WebSocket session tunnels
  transfer/          Files folder (browser file manager and RDP drive)
  protocols/         rdp.rs, vnc.rs, ssh.rs, telnet.rs, local_pty.rs
frontend/            Web interface (index.html, app.js, app.css), built into the binary
```

---

## Status and roadmap

Planned work and known bugs are tracked in [GitHub Issues](https://github.com/Woofson/remote/issues). Known gaps:
- Running behind a reverse proxy is not fully supported yet ([#3](https://github.com/Woofson/remote/issues/3)).

Release history: [CHANGELOG.md](CHANGELOG.md).

---

## License

MIT, see [LICENSE](LICENSE). Copyright (c) 2026 Bolt J Woofson.
