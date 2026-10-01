# <img src="assets/Remoteiconsmall.png" alt="Remote Logo" height="40" style="vertical-align: -6px; margin-right: 8px;" /> Remote

<div align="center">
  <p><em>High-Performance, Ultra-Low-Resource Remote Gateway (RDP, VNC, SSH) in Rust & HTML5 — By Woofson</em></p>
</div>

---

## Overview

**Remote** is a modern, single-binary replacement for Apache Guacamole built from the ground up in **Rust & HTML5**. It eliminates the heavy Java Tomcat/MySQL/`guacd` stack, replacing it with an ultra-lightweight, memory-safe, sub-millisecond asynchronous gateway using **Tokio, Axum, and pure HTML5 Canvas/WebGL**.

Adhering strictly to the **Woofson Design System** (`CommanderDog`, `NoteDog`, `DotDog`), Remote features multi-pane remote viewports, bi-directional clipboard synchronization, drag-and-drop file staging, and native user RBAC ready for **Authentik / OIDC single sign-on**.

---

## ⚡ Key Features

- **Ultra-Low Resource Footprint**:
  - Base memory footprint of **~15–30 MB RAM** (vs 500 MB–1.2 GB in Apache Guacamole).
  - Sub-30ms cold start time.
  - Zero stop-the-world garbage collection pauses (smooth 60 FPS streaming).
- **User Management & Granular RBAC from Day 1**:
  - Backed by an embedded SQLite database (`rusqlite`) with **Argon2id** password hashing and **JWT** session tokens.
  - Role-based access control (`Admin`, `Operator`, `Viewer`).
  - Granular per-connection permissions (`can_connect`, `can_edit`, `can_clipboard`, `can_transfer`).
  - Comprehensive access and connection audit logging.
- **Authentik & OpenID Connect (OIDC / OAuth2) Integration**:
  - Built-in pluggable auth provider architecture.
  - Support for **Authentik**, Keycloak, Authelia, Google, and standard OIDC identity providers.
  - Automated user provisioning and group-to-role synchronization.
- **Flawless Bi-Directional Clipboard Engine**:
  - **Auto-Sync**: Synchronizes text directly with `navigator.clipboard` when remote sessions are active.
  - **Quick Clipboard Slide-Out Drawer (`Ctrl+Shift+V`)**: Instant access to push or pull text buffers to/from remote hosts.
  - **Protocol Native Support**: RFB `ClientCutText`/`ServerCutText`, SSH ANSI OSC 52, and RDP clipboard channels.
- **Dropboxes & Bi-Directional File Transfers**:
  - **Global Session Drag & Drop**: Drag files from your desktop onto any active remote viewport to trigger the glowing amber drop overlay.
  - **SFTP Remote Explorer**: Browse remote directories, upload staged files, and download remote files with 1 click.
  - **Staging Cache**: Secure local caching in `./data/staging` with automatic tracking and download endpoints.
- **Dynamic 1-to-4 Multi-Pane Grid (`Alt+1` to `Alt+4`)**:
  - Toggle seamlessly between Single, Dual-Vertical, Triple, and 2x2 Quad layouts.
  - View up to 4 simultaneous active SSH, VNC, RDP, or Local PTY sessions side-by-side.
- **Protocols Supported**:
  - **RDP (Windows Remote Desktop)**:
    - Native `IronRDP` 0.1.0 engine with **Network Level Authentication (NLA / CredSSP)**, TLS, and RDPGFX graphics decoding.
    - **64×64 Dirty Tile Diffing**: Sub-rect tile caching reducing bandwidth by 99.8% for silky smooth 60 FPS remote interaction.
    - **Live Dynamic Resolution (`MS-RDPEDISP`)**: Real-time remote resolution adaptation when resizing browser viewports or switching multi-pane grid layouts.
    - **Experience Presets**: Configurable performance modes (`High Speed`, `Balanced`, `High Quality`, `Custom`) for wallpaper, window dragging, menu animations, themes, font smoothing, and color depth (32-bit / 16-bit).
    - **Input Precision**: Hardware PS/2 Set 1 scancodes and dedicated MS-RDPBCGR mouse click transition framing.
  - **SSH**: Remote shell with PTY allocation, resize handling, and integrated SFTP file subsystem.
  - **VNC / RFB (3.8)**: Full RFB client supporting Raw and CopyRect tile decoding, DES auth, mouse events, and keysym mappings.
  - **Local PTY**: Direct native shell execution (`bash`/`sh` or `powershell.exe`).

---

## 🎨 Design & Color Palette

Remote is styled with the official **Woofson Golden Amber Palette**:

| Token | Hex | Usage |
| :--- | :--- | :--- |
| **`bg-void`** | `#121214` | Main canvas background |
| **`bg-panel`** | `#18181b` | Panes, sidebars, modal surfaces |
| **`bg-header`** | `#202024` | Toolbar headers and status bars |
| **`bg-active`** | `#27272a` | Active tabs and hover states |
| **`accent-core`** | `#f59e0b` | Golden Amber primary accent |
| **`border-focus`** | `#f59e0b` | Focused pane active rings |
| **`text-main`** | `#f4f4f5` | High-contrast body text |

---

## 🚀 Quick Start

### 1. Build and Run

```bash
cd /home/bolt/projects/remote
cargo run --release
```

### 2. Initial Login Credentials

On first launch, Remote automatically seeds a default administrator:

* **URL:** `http://localhost:8088`
* **Username:** `admin`
* **Password:** `remote`

*(Please change this password upon first login in User Management!)*

---

## ⚙️ Configuration (`config.toml`)

```toml
[server]
bind_address = "0.0.0.0:8088"
data_dir = "./data"
jwt_secret = "remote-super-secret-jwt-key-woofson-2026"
token_expiry_hours = 24

[storage]
staging_dir = "./data/staging"
max_upload_size_mb = 2048

[auth]
allow_local_login = true
default_role = "operator"

[oidc]
enabled = false
provider_name = "Authentik"
issuer_url = "https://authentik.example.com/application/o/remote/"
client_id = "remote-client"
client_secret = "your-secret"
redirect_uri = "http://localhost:8088/api/auth/oidc/callback"
scopes = ["openid", "profile", "email", "groups"]
auto_provision_users = true
admin_group = "Remote-Admins"

[clipboard]
default_mode = "bidirectional"
max_text_size_bytes = 1048576
```

---

## 📂 Project Structure

```
remote/
├── Cargo.toml               # Rust package & dependency definitions
├── config.toml              # Server, OIDC, storage, and clipboard settings
├── src/
│   ├── main.rs              # Entry point & banner initialization
│   ├── lib.rs               # Library export declarations
│   ├── config/mod.rs        # App configuration loader
│   ├── db/mod.rs            # SQLite database manager & RBAC tables
│   ├── auth/mod.rs          # Argon2id, JWT, AES-GCM encryption & OIDC service
│   ├── protocols/           # Gateway protocol handlers
│   │   ├── local_pty.rs     # Native shell PTY tunnel
│   │   ├── ssh.rs           # SSH terminal & SFTP file transfer engine
│   │   ├── vnc.rs           # Pure RFB 3.8 client & tile streaming
│   │   └── rdp.rs           # RDP gateway engine
│   ├── transfer/mod.rs      # Dropbox staging & upload manager
│   └── server/mod.rs        # Axum REST routes & WebSocket tunnels
└── frontend/                # Single-page web client (embedded into binary)
    ├── index.html           # Woofson multi-pane UI layout & modals
    ├── app.css              # Golden Amber design tokens & animations
    ├── app.js               # Canvas/WebGL renderer, terminal & clipboard engine
    └── assets/              # Logos and icons
```

---

## 🗺️ Roadmap & Changelog

* **Roadmap**: See [ROADMAP.md](ROADMAP.md) for the detailed architectural plan, including granular RBAC connection policies, clipboard & file transfer permissions, session recording, and Authentik directory sync.
* **Changelog**: See [CHANGELOG.md](CHANGELOG.md) for release notes and version history.

---

## 📜 License

MIT License — Copyright (c) 2026 Bolt J Woofson
