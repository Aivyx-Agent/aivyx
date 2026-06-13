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
