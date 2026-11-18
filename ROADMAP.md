# Roadmap

Remote aims to be a modern replacement for Apache Guacamole: lightweight, fast and secure. Many terminals and remote desktops can be open at once, arranged in panes, in tabs, or as floating windows.

Longer term, Remote grows into a small-scale VDI for around 10 users: each user gets one or more Proxmox VMs, first as whole desktops, later as individual apps from different platforms in one place.

Day-to-day planning lives in [GitHub Issues](https://github.com/Woofson/remote/issues). This page gives the overall direction. Items below without an issue number become issues when work on them starts.

---

## Done

- RDP, VNC, SSH, Telnet and local shell sessions in a 1 to 4 pane grid.
- Local accounts, roles (Admin, Operator, Viewer), per-connection permissions, global and personal connections, OpenID Connect login, audit log.
- Clipboard sync for RDP, VNC and SSH; SFTP file browsing.
- Files: shared folder managed from the browser and offered to RDP sessions as `\\tsclient\Files` ([#5](https://github.com/Woofson/remote/issues/5), [#13](https://github.com/Woofson/remote/issues/13)).
- Per-pane keyboard layouts, RDP dynamic resolution and performance presets.
- Full window mode and session lock screen.

## Open bugs

- fish/zsh terminal support needs confirming ([#4](https://github.com/Woofson/remote/issues/4)).
- Keyboard layout "Auto" mode and non-RDP protocols need confirming ([#6](https://github.com/Woofson/remote/issues/6)).

---

## Next: Workspace

- Tabs: several sessions inside one pane ([#7](https://github.com/Woofson/remote/issues/7)).
- Floating session windows ([#8](https://github.com/Woofson/remote/issues/8)).

## Next: Reverse proxy readiness ([#3](https://github.com/Woofson/remote/issues/3))

- Hosting under a sub-path.
- WebSocket keepalive so proxies do not drop idle sessions; automatic reconnect.
- Trusted proxy list and correct client IP and scheme from forwarded headers.
- Optional header-based login behind Authentik/Authelia.
- Example configs for Nginx, Caddy, Traefik, Cloudflare Tunnel and HAProxy.
- Health check endpoints.

## Then: VDI

1. **Proxmox VM integration**: assign one or more VMs per user, show power state, start on connect, optional shutdown when idle ([#9](https://github.com/Woofson/remote/issues/9)).
2. **Launcher page**: desktops and apps as tiles, following existing permissions ([#10](https://github.com/Woofson/remote/issues/10)).
3. **Individual apps**: Windows apps first (one app per session), Linux apps next, seamless Windows RemoteApp windows later ([#11](https://github.com/Woofson/remote/issues/11)).

Builds on tabs (#7) and floating windows (#8).

## Later

- **Web proxy** for homelab web interfaces such as Proxmox, TrueNAS and Portainer ([#2](https://github.com/Woofson/remote/issues/2)).
- **Session limits**: maximum concurrent sessions per user or connection, time-of-day access windows, idle timeout.
- **Richer audit log**: session duration, client IP, disconnect reason, clipboard and file transfer records.
- **Accounts**: password reset by email, passkeys/hardware keys.
- **Session recording**: terminal recordings and RDP/VNC capture with an in-browser player.
- **High availability**: multiple gateway nodes sharing one database.
