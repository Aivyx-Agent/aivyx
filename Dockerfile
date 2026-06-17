# Aivyx — server-appliance image (Chapter Harbor, HB.1).
#
# Multi-stage: a Rust builder compiles the daemon + every tool-process binary,
# then a slim Debian runtime carries just the binaries + the appliance config.
# The Studio web bundle is already embedded in the `aivyx` binary at compile
# time (from the committed crates/aivyx-web/dist/), so no Node/dx is needed here.
#
# TLS is rustls (pure-Rust) and crypto is RustCrypto — no OpenSSL — so the
# runtime needs only ca-certificates. A musl/distroless static image is a future
# size optimization (docs/DOCKER.md F-2); debian-slim is the reliable default.
#
# Build:  DOCKER_BUILDKIT=1 docker build -t aivyx:dev .
# Run:    docker compose up    (see docker-compose.yml)

# ---- builder ----------------------------------------------------------------
FROM rust:1-bookworm AS builder
WORKDIR /app

# Copy the whole workspace. aivyx-web (wasm) is excluded from default-members,
# so the binary builds below never try to compile it; the daemon embeds the
# pre-built, committed dist/ bundle.
COPY . .

# Build the daemon + all Chapter F/G tool-process binaries in one pass. Cache
# the cargo registry + target dir across builds (BuildKit), and copy the release
# artifacts out within the same RUN (cache mounts don't survive the step).
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release \
        -p aivyx-cli \
        -p aivyx-gmail -p aivyx-calendar -p aivyx-drive -p aivyx-contacts \
        -p aivyx-notion -p aivyx-obsidian -p aivyx-n8n -p aivyx-toolkit \
    && mkdir -p /out \
    && for b in aivyx aivyx-gmail aivyx-calendar aivyx-drive aivyx-contacts \
                aivyx-notion aivyx-obsidian aivyx-n8n aivyx-toolkit; do \
         cp "target/release/$b" /out/; \
       done

# ---- runtime ----------------------------------------------------------------
FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Daemon + tool binaries on PATH (so [[tool_process]] `command` entries resolve).
COPY --from=builder /out/ /usr/local/bin/

# The baked appliance default config + the entrypoint that bridges the
# passphrase secret and seeds ~/.aivyx on first boot.
COPY deploy/docker/aivyx.appliance.toml /etc/aivyx/aivyx.appliance.toml
COPY deploy/docker/entrypoint.sh /usr/local/bin/aivyx-entrypoint
RUN chmod +x /usr/local/bin/aivyx-entrypoint

# Studio + state live here; mount a named volume on /root/.aivyx to persist.
ENV HOME=/root
VOLUME ["/root/.aivyx", "/work"]
EXPOSE 7843

ENTRYPOINT ["/usr/local/bin/aivyx-entrypoint"]
