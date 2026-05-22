#!/usr/bin/env bash
#
# aivyx local dev launcher — Phase 99 Task 1.
#
# Phase 99 keeps all builds local while repo infrastructure is still
# being decided (no CI, no remote runners). This script is the
# local-testing entry point: it builds the `aivyx` binary and
# launches it against a **fully local Ollama backend** with a
# disposable, gitignored state directory — so a `run Aivyx locally`
# loop is one command and leaves no footprint outside `.dev-run/`.
#
# What it does:
#   1. Preflights the Ollama server (reachable + requested model
#      pulled) so a misconfigured backend fails fast with an
#      actionable message instead of mid-session.
#   2. Builds `aivyx` locally (`cargo build --bin aivyx`).
#   3. Execs the binary with the Ollama provider selected and every
#      path pinned under `.dev-run/` — sandbox FS root, encrypted
#      store, and a throwaway dev passphrase.
#
# Why Ollama: no API key, no network egress, no per-run cost — the
# truest fit for a local testing loop. The Anthropic provider is
# deliberately out of scope here (Phase 99 decision).
#
# Why a fixed dev passphrase: the redb store is encrypted, so the
# passphrase must be stable across runs or each launch would fail to
# reopen the store. `.dev-run/` is disposable scratch state, never
# real data, so a hardcoded throwaway is correct here — do NOT reuse
# this pattern for a real install.
#
# Usage:
#   ./scripts/dev-run.sh [options] [-- <args passed to aivyx>]
#
# Options:
#   --model <name>      Ollama model to use      (default: llama3.1)
#   --ollama-url <url>  Ollama base URL          (default: http://localhost:11434)
#   --release           Build with --release     (default: debug)
#   --reset             Wipe .dev-run/ first for a clean store/sandbox
#   -h, --help          Show this help and exit
#
# Anything after `--` is forwarded verbatim to the binary, e.g.:
#   ./scripts/dev-run.sh -- --verify-only      # forensic audit walk
#   ./scripts/dev-run.sh -- --role coder       # start in a named role
#
# Examples:
#   ./scripts/dev-run.sh                       # interactive session
#   ./scripts/dev-run.sh --model llama3.2 --reset
#   ./scripts/dev-run.sh --release

set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
DEV_DIR="$REPO_ROOT/.dev-run"

# --- defaults (each overridable via the matching flag) ---------------
MODEL="${AIVYX_MODEL:-llama3.1}"
OLLAMA_URL="${AIVYX_OLLAMA_URL:-http://localhost:11434}"
# Dev-only throwaway. See header — never a real secret.
PASSPHRASE="${AIVYX_DEV_PASSPHRASE:-aivyx-dev-throwaway}"
BUILD_PROFILE="debug"
DO_RESET=0
PASSTHROUGH=()

usage() { sed -n '2,49p' "$0" | sed 's/^# \{0,1\}//'; }

# --- argument parsing ------------------------------------------------
while [[ $# -gt 0 ]]; do
    case "$1" in
        --model)      MODEL="${2:?--model requires a value}"; shift 2 ;;
        --ollama-url) OLLAMA_URL="${2:?--ollama-url requires a value}"; shift 2 ;;
        --release)    BUILD_PROFILE="release"; shift ;;
        --reset)      DO_RESET=1; shift ;;
        -h|--help)    usage; exit 0 ;;
        --)           shift; PASSTHROUGH+=("$@"); break ;;
        *)            PASSTHROUGH+=("$1"); shift ;;
    esac
done

# --- preflight: Ollama reachable ------------------------------------
echo "dev-run: checking Ollama at $OLLAMA_URL"
if ! TAGS="$(curl -fsS --max-time 4 "$OLLAMA_URL/api/tags" 2>/dev/null)"; then
    echo "dev-run: ERROR — Ollama is not reachable at $OLLAMA_URL" >&2
    echo "dev-run:   start it with:  ollama serve" >&2
    echo "dev-run:   or point elsewhere with:  --ollama-url <url>" >&2
    exit 1
fi

# --- preflight: requested model pulled ------------------------------
# /api/tags reports names as e.g. "llama3.1:latest"; a bare "llama3.1"
# request must match the "llama3.1:" prefix or an exact-with-tag name.
if ! grep -qE "\"name\"[[:space:]]*:[[:space:]]*\"${MODEL}(:|\")" <<<"$TAGS"; then
    echo "dev-run: ERROR — model '$MODEL' is not pulled on this Ollama instance" >&2
    echo "dev-run:   pull it with:  ollama pull $MODEL" >&2
    echo "dev-run:   or pick another with:  --model <name>" >&2
    exit 1
fi

# --- disposable state directory -------------------------------------
if [[ $DO_RESET -eq 1 ]]; then
    echo "dev-run: --reset — wiping $DEV_DIR"
    rm -rf "$DEV_DIR"
fi
mkdir -p "$DEV_DIR/sandbox"

# --- build (local only — Phase 99) ----------------------------------
echo "dev-run: building aivyx ($BUILD_PROFILE)"
if [[ "$BUILD_PROFILE" == "release" ]]; then
    cargo build --bin aivyx --release
else
    cargo build --bin aivyx
fi
BIN="$REPO_ROOT/target/$BUILD_PROFILE/aivyx"

# --- launch ----------------------------------------------------------
echo "dev-run: launching"
echo "dev-run:   provider = ollama   model = $MODEL"
echo "dev-run:   ollama   = $OLLAMA_URL"
echo "dev-run:   sandbox  = $DEV_DIR/sandbox"
echo "dev-run:   store    = $DEV_DIR/store.redb"
echo "dev-run:   args     = ${PASSTHROUGH[*]:-(none)}"
echo

# CWD is .dev-run/ so the binary's default ./aivyx.toml lookup is
# isolated from the repo root — drop a .dev-run/aivyx.toml later if a
# dev role config is wanted; nothing in the repo gets picked up by
# accident.
cd "$DEV_DIR"
exec env \
    AIVYX_PROVIDER=ollama \
    AIVYX_MODEL="$MODEL" \
    AIVYX_OPENAI_BASE_URL="$OLLAMA_URL" \
    AIVYX_FS_ROOT="$DEV_DIR/sandbox" \
    AIVYX_STORAGE_PATH="$DEV_DIR/store.redb" \
    AIVYX_PASSPHRASE="$PASSPHRASE" \
    "$BIN" ${PASSTHROUGH[@]+"${PASSTHROUGH[@]}"}
