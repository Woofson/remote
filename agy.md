# Architectural Specification: Adaptive Remote Session Orchestrator & Gateway

**Project:** `remote` (Binary: `remote`)  
**Design Paradigm:** Client-first orchestration with fallback web bastion  
**Target Runtime:** Rust (Tokio async runtime)

---

## 1. System Vision & Problem Statement

Modern remote access tools exist as polarized extremes:
- **Native clients (`xfreerdp3`, Moonlight):** Offer near-zero latency, hardware acceleration, and full multi-monitor integration over modern P2P mesh overlays (NetBird, Tailscale), but fail entirely when traversing hostile corporate firewalls blocking non-443/UDP traffic.
- **Web gateways (Apache Guacamole):** Provide firewall traversal over HTTPS/WebSockets, but suffer from CPU-intensive server-side transcoding, high input latency, lossy video artifacts, and legacy deployment stacks (Java servlets, `guacd`).

`remote` is an **adaptive hybrid system**:
1. **Primary Operational Mode (Orchestrator):** A lightweight terminal binary running locally on trusted endpoints. It resolves endpoints via local routing or mesh API peering, auto-provisions session geometry/credentials, and spawns native high-performance clients directly over P2P tunnels.
2. **Fallback Operational Mode (Bastion Gateway):** A containerized daemon deployed within the private network behind standard reverse proxies (Nginx/Traefik). When accessed via browser or restricted environments, it streams remote displays via WebRTC/WebSocket directly onto an HTML5 canvas.

---

## 2. Core Architecture & Route Decision Tree

                   [ Invocation: `remote <target>` ]
                                  |
                                  v
                 +---------------------------------+
                 |    Target Resolution Engine     |
                 |  - Check NetBird / Tailscale IP |
                 |  - Test TCP/UDP Direct Route    |
                 +---------------------------------+
                                  |
            +---------------------+---------------------+
            |                                           |
  [ Direct Reachable ]                        [ Route Blocked / Web ]
            |                                           |
            v                                           v
+-------------------------------+           +-------------------------------+
|     Native Client Runner      |           |    Gateway Proxy / Web UI     |
| - Inspect local monitors/DPI  |           | - Establish WSS / WebRTC link |
| - Pull credentials from vault |           | - Authenticate via mTLS/OIDC  |
| - Execute xfreerdp3 / client|           | - Render to in-browser canvas |
+-------------------------------+           +-------------------------------+


---

## 3. Component Breakdown

### 3.1. Target Resolution & Mesh Discovery
- **Local Routing Probe:** Fast TCP SYN check against target service ports (3389 for RDP, 22 for SSH, 47990 for Sunshine).
- **Mesh Integration:** Read-only bindings against local NetBird (`/var/run/netbird.sock`) or Tailscale Unix sockets to match human aliases (`hawk`, `comet`, `buzzard`) to their internal virtual IPs.
- **WOL / State Broker:** If a host is marked offline or asleep, trigger Wake-on-LAN packets or API power calls to the hypervisor (e.g., Proxmox VE API) prior to connection negotiation.

### 3.2. Native Execution Engine (Local Client Mode)
- Generates precise, hardware-optimized CLI arguments dynamically:
  - **RDP:** Spawns `xfreerdp3` with native Wayland/X11 flags (`/gfx:avc444`, `/sound:sys:pulse`, `/clipboard`, `/dynamic-resolution`).
  - **Screen Adaptation:** Reads active monitor setups (via `wlr-output-management` or XRandR) to enforce exact aspect ratios and fractional scaling factors.
- Avoids manual script management; wraps execution in a managed sub-process with terminal log forwarding.

### 3.3. Embedded Web Gateway (Bastion Daemon Mode)
- **Engine:** Built using `axum` and asynchronous WebSockets/WebRTC data channels.
- **Protocol Interop:** Direct Rust bindings to FreeRDP/libvnc or minimal proxy bridges; avoids legacy C-daemon wrappers like `guacd`.
- **Client Surface:** Minimalist WASM or Vanilla JS/WebGL canvas client served on a single port for seamless Nginx/Traefik reverse proxying.

---

## 4. Configuration Schema (`remote.toml`)

```toml
[general]
default_mode = "auto" # Options: "auto", "native", "gateway"
credential_store = "system" # Options: "system", "pass", "keepassxc", "env"

[mesh]
provider = "netbird" # Options: "netbird", "tailscale", "static"
socket_path = "/var/run/netbird.sock"

[gateway]
listen_addr = "0.0.0.0:8443"
reverse_proxy_header = "X-Forwarded-For"
tls_enabled = false # Assumes Traefik/Nginx handles TLS termination

[hosts.hawk]
alias = "hawk"
hostname = "hawk.netbird.cloud"
ip = "172.16.100.25"
protocol = "rdp"
port = 3389
user = "administrator"
client_args = [
    "/cert:ignore",
    "/gfx:avc444",
    "/sound",
    "/clipboard",
    "/floatbar"
]
wol_mac = "AA:BB:CC:DD:EE:01"

[hosts.comet]
alias = "comet"
hostname = "comet.netbird.cloud"
ip = "172.16.100.2"
protocol = "ssh"
port = 22
user = "root"
5. CLI Interface Specification
Bash
# Direct connection (evaluates path automatically)
remote hawk

# Force native execution pipeline
remote hawk --mode native

# Force tunnel through self-hosted web gateway
remote hawk --mode gateway

# Launch embedded gateway service (server deployment)
remote serve --config /etc/remote/config.toml

# Probe and list known mesh endpoints and reachability status
remote list
6. Implementation Roadmap
Phase 1: Local Runner & Discovery CLI
Implement remote.toml configuration parser.

Build host resolver with direct TCP probe and NetBird local peer table extraction.

Implement process spawn abstractions for xfreerdp3 and OpenSSH.

Phase 2: Session Broker & Power Integration
Implement Wake-on-LAN broadcasting.

Add Proxmox VE API integration to monitor VM state and send start triggers when connecting to stopped endpoints.

Phase 3: Web Bastion Daemon
Build axum HTTP/WSS server.

Integrate WebGL/HTML5 canvas frontend for corporate browser fallback.

Implement RDP/VNC-to-canvas rendering loop over encrypted WebSockets.

