# Running Remote behind a reverse proxy

Remote works behind Nginx, Caddy, Traefik, HAProxy and Cloudflare Tunnel, at its own address (`https://remote.example.com/`) or under a sub-path (`https://example.com/remote/`).

## What Remote needs

- **WebSockets.** Every session is a WebSocket on `<base>/ws/tunnel/...`. The proxy must pass the `Upgrade` and `Connection` headers through. Caddy, Traefik, HAProxy and Cloudflare do this on their own; Nginx needs the lines shown below.
- **Nothing else for idle sessions.** Remote sends a small ping on every open session every 25 seconds, so proxy idle limits (Nginx 60 s, Cloudflare 100 s, HAProxy `timeout tunnel`) do not close quiet terminals. If a remote desktop (RDP or VNC) is dropped anyway, the browser reconnects it up to three times; the desktop keeps running on the other side.
- **Large uploads.** If you upload big files, raise the proxy's body size limit (Nginx `client_max_body_size`). Remote's own limit is `max_upload_size_mb` in `config.toml`.

## Settings in `config.toml`

```toml
[server]
bind_address = "127.0.0.1:8088"   # only the proxy can reach Remote directly
base_path = "/remote"             # only when using a sub-path; leave "" at the root
trusted_proxies = ["127.0.0.1"]   # address(es) the proxy connects from
```

- **`base_path`**: set it whenever Remote lives under a sub-path. Remote then answers under that path whether or not the proxy strips it, and `/remote` redirects to `/remote/`. Open Remote at the address with the trailing slash.
- **`trusted_proxies`**: IP addresses or ranges (`10.0.0.0/8`, `fd00::/8`) of your proxies. Only for these does Remote believe `X-Forwarded-For`, `X-Real-IP` and `CF-Connecting-IP`; everyone else's are ignored, because visitors can send them too. With it set, the activity log and the brake on repeated failed sign-ins see the visitor's real address instead of the proxy's. For a proxy in Docker, use the Docker network range, for example `172.16.0.0/12`.
- **Single sign-on (OpenID Connect):** set `redirect_uri` in `[oidc]` to the public address, for example `https://example.com/remote/api/auth/oidc/callback`.

## Health checks

Both need no login and reveal nothing about the installation:

- `GET <base>/api/health`: `200 {"status":"ok"}` while Remote runs.
- `GET <base>/api/ready`: `200 {"status":"ready"}` when the database answers, otherwise `503`.

## Example configurations

The examples serve Remote under `/remote/` and forward to Remote on `127.0.0.1:8088`. For Remote at its own address, drop the path parts and leave `base_path` empty.

The Nginx, Caddy, Traefik and HAProxy examples were tested with Remote 0.12 (sign-in, assets, terminal and idle sessions past the proxy's timeout). The Cloudflare Tunnel and Authelia examples follow those projects' documentation but were not tested here.

### Nginx

```nginx
map $http_upgrade $connection_upgrade {
    default upgrade;
    ''      close;
}

server {
    listen 443 ssl;
    server_name example.com;
    # ssl_certificate ...; ssl_certificate_key ...;

    location /remote/ {
        proxy_pass http://127.0.0.1:8088;   # no trailing slash: the path is passed on as is
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection $connection_upgrade;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        client_max_body_size 2g;
    }
}
```

### Caddy

```caddy
example.com {
    handle /remote* {
        reverse_proxy 127.0.0.1:8088
    }
}
```

Caddy handles HTTPS, WebSockets and the forwarded headers on its own. `handle_path` (which strips the prefix) works as well.

### Traefik (file provider)

```yaml
http:
  routers:
    remote:
      rule: "Host(`example.com`) && PathPrefix(`/remote`)"
      service: remote
      entryPoints: [websecure]
      tls: {}
  services:
    remote:
      loadBalancer:
        servers:
          - url: "http://127.0.0.1:8088"
```

With Docker labels, the same rule goes in `traefik.http.routers.remote.rule` and the port in `traefik.http.services.remote.loadbalancer.server.port=8088`.

### HAProxy

```haproxy
defaults
    mode http
    timeout connect 5s
    timeout client  60s
    timeout server  60s
    timeout tunnel  1h

frontend web
    bind :443 ssl crt /etc/haproxy/example.com.pem
    option forwardfor
    use_backend remote if { path_beg /remote }

backend remote
    server remote 127.0.0.1:8088
```

### Cloudflare Tunnel (not tested here)

```yaml
# /etc/cloudflared/config.yml
tunnel: <tunnel-id>
credentials-file: /etc/cloudflared/<tunnel-id>.json
ingress:
  - hostname: remote.example.com
    service: http://127.0.0.1:8088
  - service: http_status:404
```

`cloudflared` connects from the same machine, so `trusted_proxies = ["127.0.0.1"]`. Cloudflare closes WebSockets that are idle for 100 seconds; Remote's pings prevent that.

## Signing in through the proxy (Authelia, Authentik forward auth)

If your proxy already signs people in, Remote can trust it instead of asking again:

```toml
[server]
trusted_proxies = ["127.0.0.1"]

[auth.header_login]
enabled = true
username_header = "Remote-User"      # Authentik: "X-authentik-username"
email_header = "Remote-Email"        # Authentik: "X-authentik-email"
name_header = "Remote-Name"          # Authentik: "X-authentik-name"
groups_header = "Remote-Groups"      # Authentik: "X-authentik-groups"
admin_group = "Remote-Admins"
operator_group = "Remote-Operators"
```

- Remote signs in the user named in `username_header`, creating the account on first visit. Groups decide the role (admin, operator, otherwise `default_role`) on every sign-in.
- Only requests from `trusted_proxies` count. **The proxy must remove these headers from what visitors send** and set them only after a successful check; Authelia and Authentik do this when configured as forward auth.
- Accounts made this way are separate from local and OpenID Connect accounts. If a local account already has the same name, Remote refuses instead of taking it over.
- Two-factor authentication for these users is the proxy's job. Remote's session rules (login lifetime, idle timeout) still apply.
- After **Log out**, Remote waits for a click on "Continue with your proxy sign-in" instead of signing straight back in.

Nginx with Authelia (not tested here), inside the `location /remote/` block above:

```nginx
auth_request /authelia;
auth_request_set $user   $upstream_http_remote_user;
auth_request_set $groups $upstream_http_remote_groups;
auth_request_set $email  $upstream_http_remote_email;
auth_request_set $name   $upstream_http_remote_name;
proxy_set_header Remote-User   $user;
proxy_set_header Remote-Groups $groups;
proxy_set_header Remote-Email  $email;
proxy_set_header Remote-Name   $name;
```

Setting the headers with `proxy_set_header` replaces anything the visitor sent.
