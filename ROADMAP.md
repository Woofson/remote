# Roadmap

Remote aims to be a modern replacement for Apache Guacamole: lightweight, fast and secure. Many terminals and remote desktops can be open at once, arranged in panes, in tabs, or as floating windows.

Day-to-day planning lives in [GitHub Issues](https://github.com/Woofson/remote/issues). This page gives the overall direction. Items below without an issue number become issues when work on them starts.

---

## Done

- RDP, VNC, SSH, Telnet and local shell sessions in a 1 to 4 pane grid.
- Local accounts, roles (Admin, Operator, Viewer), per-connection permissions, global and personal connections, OpenID Connect login, audit log.
- Clipboard sync for RDP, VNC and SSH; SFTP file browsing; upload staging.
- Per-pane keyboard layouts, RDP dynamic resolution and performance presets.
- Full window mode and session lock screen.

## Open bugs

- Windows drive redirection (`\\tsclient\Dropbox`) was lost in a refactor and needs restoring ([#5](https://github.com/Woofson/remote/issues/5)).
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

## Later

- **Web proxy** for homelab web interfaces such as Proxmox, TrueNAS and Portainer ([#2](https://github.com/Woofson/remote/issues/2)).
- **Session limits**: maximum concurrent sessions per user or connection, time-of-day access windows, idle timeout.
- **Richer audit log**: session duration, client IP, disconnect reason, clipboard and file transfer records.
- **Accounts**: password reset by email, passkeys/hardware keys.
- **Session recording**: terminal recordings and RDP/VNC capture with an in-browser player.
- **High availability**: multiple gateway nodes sharing one database.
