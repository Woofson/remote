# Roadmap

Remote aims to be a modern replacement for Apache Guacamole: lightweight, fast and secure. Many terminals and remote desktops can be open at once, arranged in panes, in tabs, or as floating windows.

Longer term, Remote grows into a small-scale VDI for around 10 users: each user gets one or more Proxmox VMs, first as whole desktops, later as individual apps from different platforms in one place.

Day-to-day planning lives in [GitHub Issues](https://github.com/Woofson/remote/issues). This page gives the overall direction. Items below without an issue number become issues when work on them starts.

---

## Done

- RDP, VNC, SSH, Telnet and local shell sessions in a 1 to 4 pane grid.
- Local accounts, roles (Admin, Operator, Viewer), OpenID Connect login, audit log.
- Connections assigned to users and groups, with an owner and per-assignment rights ([#18](https://github.com/Woofson/remote/issues/18)).
- Password reset, change on next login, password policy, logout everywhere ([#20](https://github.com/Woofson/remote/issues/20)).
- Admin Control Panel, personal Settings and Brum themes ([#21](https://github.com/Woofson/remote/issues/21), [#22](https://github.com/Woofson/remote/issues/22)).
- Connections directory and editor, connection icons ([#17](https://github.com/Woofson/remote/issues/17), [#19](https://github.com/Woofson/remote/issues/19)).
- Clipboard sync for RDP, VNC and SSH; SFTP file browsing.
- Files: private "My files" per user and an admin-managed Shared folder, also offered to RDP sessions as `\\tsclient\Files` and `\\tsclient\Shared` ([#5](https://github.com/Woofson/remote/issues/5), [#12](https://github.com/Woofson/remote/issues/12), [#13](https://github.com/Woofson/remote/issues/13)).
- Security hardening: private secret key, SFTP permission checks, guarded RDP drives ([#14](https://github.com/Woofson/remote/issues/14), [#15](https://github.com/Woofson/remote/issues/15)).
- Per-pane keyboard layouts, RDP dynamic resolution and performance presets.
- Full window mode and session lock screen.
- Terminal sizing for full-screen programs (tmux, editors), shell choice per local shell, layout menu with the last layout remembered per user ([#4](https://github.com/Woofson/remote/issues/4)).
- Email: SMTP settings, "Forgot password?" with single-use links, set-your-password invitations, security notices ([#27](https://github.com/Woofson/remote/issues/27)).
- Reverse proxy support: sub-path hosting, session keepalive and reconnect, trusted proxies and real client IPs, forward-auth sign-in, health checks, tested Nginx/Caddy/Traefik/HAProxy examples ([#3](https://github.com/Woofson/remote/issues/3)).
- Two-factor authentication (TOTP) with recovery codes; admin session rules: 2FA per role, login lifetime, idle lock or sign-out, server-enforced lock screen ([#26](https://github.com/Woofson/remote/issues/26)).

## Open bugs

- Keyboard layout "Auto" mode and non-RDP protocols need confirming ([#6](https://github.com/Woofson/remote/issues/6)).

---

## Next: Workspace

- Tabs: several sessions inside one pane ([#7](https://github.com/Woofson/remote/issues/7)).
- Floating session windows ([#8](https://github.com/Woofson/remote/issues/8)).

## Then: VDI

1. **Proxmox VM integration**: assign one or more VMs per user, show power state, start on connect, optional shutdown when idle ([#9](https://github.com/Woofson/remote/issues/9)).
2. **Launcher page**: desktops and apps as tiles, following existing permissions ([#10](https://github.com/Woofson/remote/issues/10)).
3. **Individual apps**: Windows apps first (one app per session), Linux apps next, Windows RemoteApp windows later ([#11](https://github.com/Woofson/remote/issues/11)).

Builds on tabs (#7) and floating windows (#8).

## Later

- **Web proxy** for homelab web interfaces such as Proxmox, TrueNAS and Portainer ([#2](https://github.com/Woofson/remote/issues/2)).
- **Session limits**: maximum concurrent sessions per user or connection, time-of-day access windows.
- **Richer audit log**: session duration, client IP, disconnect reason, clipboard and file transfer records.
- **Accounts**: passkeys and hardware keys.
- **Session recording**: terminal recordings and RDP/VNC capture with an in-browser player.
- **High availability**: multiple gateway nodes sharing one database.
