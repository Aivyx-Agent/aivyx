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

exec aivyx daemon run "$@"
