#!/bin/sh
# Aivyx container entrypoint (Chapter Harbor, HB.1).
#
# Bridges a Docker *secret* (mounted as a file) to the AIVYX_PASSPHRASE env var
# the daemon reads to unlock the encrypted store — so the passphrase never has
# to live in plain `environment:` (visible in `docker inspect`). Honors the
# conventional `_FILE` indirection.
set -eu

# If AIVYX_PASSPHRASE isn't already set, source it from a file:
#   1. $AIVYX_PASSPHRASE_FILE if set, else
#   2. the default Docker secret path.
if [ -z "${AIVYX_PASSPHRASE:-}" ]; then
    _pass_file="${AIVYX_PASSPHRASE_FILE:-/run/secrets/aivyx_passphrase}"
    if [ -f "$_pass_file" ]; then
        AIVYX_PASSPHRASE="$(cat "$_pass_file")"
        export AIVYX_PASSPHRASE
    fi
fi

if [ -z "${AIVYX_PASSPHRASE:-}" ]; then
    echo "aivyx (entrypoint): no passphrase — set the aivyx_passphrase secret" >&2
    echo "  (or AIVYX_PASSPHRASE_FILE / AIVYX_PASSPHRASE). See docs/DOCKER.md." >&2
    exit 2
fi

# First boot seeds ~/.aivyx with the baked appliance config unless the operator
# mounted their own (or a prior run already wrote one).
if [ ! -f /root/.aivyx/aivyx.toml ]; then
    mkdir -p /root/.aivyx
    cp /etc/aivyx/aivyx.appliance.toml /root/.aivyx/aivyx.toml
    echo "aivyx (entrypoint): seeded /root/.aivyx/aivyx.toml from the appliance default" >&2
fi

# Chapter Gatehouse — the daemon refuses an off-host bind with no auth token
# (the exposure interlock), and the appliance binds beyond loopback by design.
# If the active config exposes the web UI but sets neither a token nor the
# explicit insecure escape hatch, generate a Studio token once, insert it into
# the existing [daemon] section (a second [daemon] table would be invalid
# TOML), and print it so the operator can log in. Idempotent: subsequent boots
# see the token in the file.
_cfg=/root/.aivyx/aivyx.toml
if grep -qE '^[[:space:]]*web_ui_host' "$_cfg" \
    && ! grep -qE '^[[:space:]]*web_ui_host[[:space:]]*=[[:space:]]*"(127\.|::1)' "$_cfg" \
    && ! grep -qE '^[[:space:]]*web_ui_(auth_token|insecure_no_auth)' "$_cfg"; then
    _tok="$(tr -dc 'a-zA-Z0-9' < /dev/urandom | head -c 43)"
    awk -v tok="$_tok" '
        { print }
        /^[[:space:]]*web_ui_host/ && !done {
            print "web_ui_auth_token = \"" tok "\"  # generated at first boot (Gatehouse)"
            done = 1
        }' "$_cfg" > "$_cfg.new" && mv "$_cfg.new" "$_cfg"
    echo "aivyx (entrypoint): generated a Studio auth token (Gatehouse):" >&2
    echo "aivyx (entrypoint):   $_tok" >&2
    echo "aivyx (entrypoint): browsers: any username, this token as the password." >&2
    echo "aivyx (entrypoint): it is saved in /root/.aivyx/aivyx.toml (web_ui_auth_token)." >&2
fi

exec aivyx daemon run "$@"
