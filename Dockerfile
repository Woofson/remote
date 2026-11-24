# Remote — container image (Alpine Linux)
#
# Build:  docker build -t remote .
# Run:    docker run -d -p 8088:8088 -v ./remote:/remote --name remote remote
#
# Everything Remote keeps (config.toml, data/ with the database, secret.key and Files,
# optional themes/) lives in the /remote volume.

# ---------- Build stage: compile Remote natively for Alpine (musl) ----------
FROM rust:1-alpine3.23 AS build

RUN apk add --no-cache build-base cmake perl pkgconf linux-headers openssl-dev zlib-dev

WORKDIR /build
COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src
COPY frontend ./frontend
COPY assets ./assets
COPY themes ./themes

# Link against Alpine's shared libraries (openssl, zlib) instead of a static build.
ENV RUSTFLAGS="-C target-feature=-crt-static"
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/build/target \
    cargo build --release --locked \
    && cp target/release/remoted /usr/local/bin/remoted \
    && strip /usr/local/bin/remoted

# ---------- Run stage: small Alpine image with only what Remote needs ----------
FROM alpine:3.23

LABEL org.opencontainers.image.title="Remote" \
      org.opencontainers.image.description="Lightweight web gateway for RDP, VNC, SSH, Telnet and local shells - By Woofson" \
      org.opencontainers.image.vendor="Woofsons Lab" \
      org.opencontainers.image.url="https://www.arf.ac" \
      org.opencontainers.image.source="https://github.com/woofson/remote" \
      org.opencontainers.image.licenses="MIT"

# libgcc/openssl/zlib: runtime libraries; su-exec: drop root; curl: health check;
# tzdata: TZ support; bash: nicer local shell inside the container
RUN apk add --no-cache ca-certificates libgcc libssl3 libcrypto3 zlib su-exec curl tzdata bash

COPY --from=build /usr/local/bin/remoted /usr/local/bin/remoted
COPY docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
COPY config.example.toml /usr/local/share/remote/config.example.toml

RUN mkdir -p /remote && chmod 755 /usr/local/bin/docker-entrypoint.sh

# Remote runs as user PUID:PGID (default 1000:1000); see docker-entrypoint.sh
ENV PUID=1000 PGID=1000 \
    RUST_LOG=remoted=info,tower_http=info

WORKDIR /remote
VOLUME /remote
EXPOSE 8088

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD curl -fs http://localhost:8088/api/version || exit 1

ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh"]
