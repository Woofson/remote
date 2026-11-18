#!/bin/sh
# Starts Remote as an ordinary user (PUID/PGID, default 1000:1000) instead of root,
# with /remote (config.toml, data/, themes/) as its working folder.
# Set REMOTE_RUN_AS_ROOT=true only if you really need root inside the container.
set -e

cd /remote

# First start: a config.toml listening on all interfaces, from the example file.
new_config() {
    if [ ! -f /remote/config.toml ]; then
        echo "remote: creating /remote/config.toml from the example"
        cp /usr/local/share/remote/config.example.toml /remote/config.toml
    fi
}

PUID="${PUID:-1000}"
PGID="${PGID:-1000}"

# Already started as a non-root user (e.g. `user:` in compose): nothing to switch
if [ "$(id -u)" != "0" ]; then
    new_config
    exec /usr/local/bin/remoted "$@"
fi

case "${REMOTE_RUN_AS_ROOT:-false}" in
    true|1|yes)
        echo "remote: running as root because REMOTE_RUN_AS_ROOT is set"
        new_config
        exec /usr/local/bin/remoted "$@"
        ;;
esac

case "$PUID$PGID" in
    *[!0-9]*) echo "remote: PUID and PGID must be numbers" >&2; exit 1 ;;
esac
if [ "$PUID" = "0" ]; then
    echo "remote: PUID=0 would run as root; use REMOTE_RUN_AS_ROOT=true for that" >&2
    exit 1
fi

# Account for the chosen ids, reusing any that already exist in the image
group="$(getent group "$PGID" | cut -d: -f1)"
if [ -z "$group" ]; then
    group=remote
    addgroup -g "$PGID" "$group"
fi
user="$(getent passwd "$PUID" | cut -d: -f1)"
if [ -z "$user" ]; then
    user=remote
    adduser -D -H -h /remote -s /bin/sh -G "$group" -u "$PUID" "$user"
fi

new_config

# Hand Remote's folder to that user when ownership does not match (first start, or a changed PUID)
if [ "$(stat -c %u /remote)" != "$PUID" ] || [ "$(stat -c %u /remote/config.toml)" != "$PUID" ] \
    || { [ -e /remote/data ] && [ "$(stat -c %u /remote/data)" != "$PUID" ]; }; then
    echo "remote: giving /remote to user $PUID:$PGID"
    chown -R "$PUID:$PGID" /remote
fi

export HOME=/remote
exec su-exec "$PUID:$PGID" /usr/local/bin/remoted "$@"
