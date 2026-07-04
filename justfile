# Aivyx task runner. Most work is plain `cargo`; this captures the few flows
# that need extra toolchains (the wasm web bundle).

# Build the aivyx-web Dioxus bundle into crates/aivyx-web/dist/ — the directory
# the daemon's build.rs (Chapter M.5) embeds. The daemon then serves it at `/`,
# `/<app>.wasm`, etc.; without it the daemon serves the legacy fallback page, so
# a plain `cargo build` never needs the wasm toolchain.
#
# Prereqs (once):
#   cargo install dioxus-cli            # the `dx` CLI
#   rustup target add wasm32-unknown-unknown
#
# NOTE: dx's output layout is version-dependent. The CONTRACT is simply that
# crates/aivyx-web/dist/ ends up holding index.html + the wasm + the JS glue.
# Adjust the copy below to your installed dx version if its public/ path differs.
build-web:
    rustup target add wasm32-unknown-unknown
    cd crates/aivyx-web && dx bundle --release --platform web
    rm -rf crates/aivyx-web/dist && mkdir -p crates/aivyx-web/dist
    cp -r target/dx/aivyx-web/release/web/public/. crates/aivyx-web/dist/
    # Drop the brotli pre-compressed twins — the daemon serves the plain assets
    # and embeds everything under dist/, so the .br files are dead weight.
    find crates/aivyx-web/dist -name '*.br' -delete
    @echo "bundle → crates/aivyx-web/dist/; rebuild the daemon to embed it:"
    @echo "  cargo build -p aivyx-cli --bin aivyx --release"

# Verify the web app compiles to wasm (the cheap guard CI runs; no dx needed).
check-web:
    rustup target add wasm32-unknown-unknown
    cargo build -p aivyx-web --target wasm32-unknown-unknown

# Drop the built bundle so the daemon reverts to the fallback page.
clean-web:
    rm -rf crates/aivyx-web/dist

# Chapter Freight (FR.3) — build + sign the Kitchen pack bundle, the free
# worked example of the signed pack format. Uses a DEV key (generated on
# first run into .pack-dev/, git-ignored); the v1.0 web presence will
# establish the real publisher key ceremony.
#
#   just pack-kitchen
#
# Output: .pack-dev/kitchen-<version>-<host-triple>.aivyxpack plus the dev
# verifying key to paste into the target machine's [pack] trusted_publishers.
pack-kitchen:
    #!/usr/bin/env bash
    set -euo pipefail
    ver="$(grep '^version' Cargo.toml | head -1 | cut -d'"' -f2)"
    triple="$(rustc -vV | grep host | cut -d' ' -f2)"
    cargo build --release -p aivyx-kitchen-toolkit
    mkdir -p .pack-dev
    if [ ! -f .pack-dev/dev-signing.key ]; then
        cargo run --release -p aivyx-cli --bin aivyx -- pack keygen .pack-dev/dev-signing.key
    fi
    stage=".pack-dev/stage-kitchen"
    rm -rf "$stage" && mkdir -p "$stage/bin" "$stage/config"
    cp target/release/aivyx-kitchen-toolkit "$stage/bin/"
    cp crates/verticals/aivyx-kitchen/assets/kitchen-boh.toml "$stage/config/"
    cat > "$stage/manifest.toml" <<MANIFEST
    name = "kitchen"
    version = "$ver"
    target = "$triple"
    min_daemon_version = "0.8.0"
    publisher = "Aivyx (dev key)"
    team_config = "kitchen-boh.toml"

    [[tool_process]]
    name = "kitchen-toolkit"
    bin = "aivyx-kitchen-toolkit"
    MANIFEST
    out=".pack-dev/kitchen-$ver-$triple.aivyxpack"
    target/release/aivyx pack build "$stage" --key .pack-dev/dev-signing.key --out "$out"
    echo "bundle: $out"
