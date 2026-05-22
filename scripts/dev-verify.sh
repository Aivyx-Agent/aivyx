#!/usr/bin/env bash
#
# aivyx local verification pass — Phase 99 Task 3.
#
# `dev-run.sh` proves an interactive chat session works. This script
# is its non-interactive sibling: a scripted battery that drives the
# real `aivyx` binary against a real local Ollama backend and checks
# the subsystems a human cannot reliably smoke-test by hand — the
# audit chain, the encrypted store, the memory/fs tool paths, and
# daemon mode.
#
# It shares dev-run.sh's posture exactly: Ollama backend, all state
# under a gitignored `.dev-run/`, a throwaway dev passphrase. The
# daemon socket is additionally isolated under `.dev-run/run/` (an
# overridden `XDG_RUNTIME_DIR`) so this pass never collides with a
# real daemon the operator may be running.
#
# Two classes of check:
#   * SUBSTRATE checks are deterministic — store, audit chain,
#     daemon lifecycle, subcommands. A failure here is a real defect
#     and exits the script non-zero.
#   * TOOL-PATH probes depend on the local LLM actually choosing to
#     call a tool (memory.write, fs.write). Local models are not
#     reliable tool-callers, so a miss is reported as WARN, not
#     FAIL — it indicts the model, not the substrate.
#
# Usage:
#   ./scripts/dev-verify.sh [--model <name>] [--ollama-url <url>] [--keep]
#
#   --keep   Do not wipe .dev-run/ first (default: wipe for a clean
#            baseline — the 0-events audit assertion needs it).
#
# Also reachable as `./scripts/dev-run.sh --verify`.

# No `set -e`: every check must run so the summary is complete.
set -uo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
DEV_DIR="$REPO_ROOT/.dev-run"
RUN_DIR="$DEV_DIR/run"

MODEL="${AIVYX_MODEL:-llama3.1}"
OLLAMA_URL="${AIVYX_OLLAMA_URL:-http://localhost:11434}"
PASSPHRASE="${AIVYX_DEV_PASSPHRASE:-aivyx-dev-throwaway}"
KEEP=0
# Generous — a cold local model can take a while on the first turn.
TURN_TIMEOUT=180

while [[ $# -gt 0 ]]; do
    case "$1" in
        --model)      MODEL="${2:?--model requires a value}"; shift 2 ;;
        --ollama-url) OLLAMA_URL="${2:?--ollama-url requires a value}"; shift 2 ;;
        --keep)       KEEP=1; shift ;;
        -h|--help)    sed -n '2,33p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *)            echo "dev-verify: unknown argument: $1" >&2; exit 2 ;;
    esac
done

PASS=0; FAIL=0; WARN=0
pass()    { echo "  PASS  $*"; PASS=$((PASS + 1)); }
fail()    { echo "  FAIL  $*" >&2; FAIL=$((FAIL + 1)); }
warn()    { echo "  WARN  $*"; WARN=$((WARN + 1)); }
section() { echo; echo "--- $* ---"; }

# --- preflight: Ollama -----------------------------------------------
echo "dev-verify: checking Ollama at $OLLAMA_URL"
if ! TAGS="$(curl -fsS --max-time 4 "$OLLAMA_URL/api/tags" 2>/dev/null)"; then
    echo "dev-verify: ERROR — Ollama not reachable at $OLLAMA_URL (ollama serve)" >&2
    exit 1
fi
if ! grep -qE "\"name\"[[:space:]]*:[[:space:]]*\"${MODEL}(:|\")" <<<"$TAGS"; then
    echo "dev-verify: ERROR — model '$MODEL' not pulled (ollama pull $MODEL)" >&2
    exit 1
fi

# --- clean baseline + build ------------------------------------------
if [[ $KEEP -eq 0 ]]; then
    echo "dev-verify: wiping $DEV_DIR for a clean baseline"
    rm -rf "$DEV_DIR"
fi
mkdir -p "$DEV_DIR/sandbox" "$RUN_DIR"

echo "dev-verify: building aivyx (debug)"
if ! cargo build --bin aivyx; then
    echo "dev-verify: ERROR — build failed" >&2
    exit 1
fi
BIN="$REPO_ROOT/target/debug/aivyx"
echo "dev-verify: model=$MODEL  ollama=$OLLAMA_URL"

# The whole dev environment, exported once so every invocation below
# inherits it. CWD is pinned to .dev-run/ inside the helpers.
export AIVYX_PROVIDER=ollama
export AIVYX_MODEL="$MODEL"
export AIVYX_OPENAI_BASE_URL="$OLLAMA_URL"
export AIVYX_FS_ROOT="$DEV_DIR/sandbox"
export AIVYX_STORAGE_PATH="$DEV_DIR/store.redb"
export AIVYX_PASSPHRASE="$PASSPHRASE"
export XDG_RUNTIME_DIR="$RUN_DIR"

# Fast, non-LLM invocations — flags and subcommands. stdin is the
# caller's; redirect /dev/null for the no-session paths.
aivyx_run()  { ( cd "$DEV_DIR" && "$BIN" "$@" ); }
# LLM-driven turns — timeout-wrapped. Pipe the prompt in on stdin.
aivyx_turn() { ( cd "$DEV_DIR" && timeout "$TURN_TIMEOUT" "$BIN" "$@" ); }

# Audit event count parsed from a `--verify-only` run.
audit_count() {
    aivyx_run --verify-only </dev/null 2>&1 \
        | grep -oE 'verified [0-9]+' | grep -oE '[0-9]+' | head -1
}

# =====================================================================
# SUBSTRATE — deterministic
# =====================================================================
section "introspection (no session)"

out="$(aivyx_run --version </dev/null 2>&1)"
if [[ $? -eq 0 && "$out" == aivyx\ * ]]; then
    pass "--version → $out"
else
    fail "--version unexpected: $out"
fi

out="$(aivyx_run --print-role default </dev/null 2>&1)"; rc=$?
if [[ $rc -eq 0 && -n "$out" ]]; then
    pass "--print-role default rendered an envelope"
else
    # No committed TOML in the dev setup → 'default' may not resolve.
    warn "--print-role default rc=$rc (no role config without a TOML — expected)"
fi

section "encrypted store + audit chain"

base="$(audit_count)"
if [[ "$base" == "0" ]]; then
    pass "fresh encrypted store verifies — 0 events (chain empty)"
else
    fail "fresh store baseline expected 0 events, got '${base:-<none>}'"
fi

# A plain chat turn writes audit entries regardless of tool calls, so
# this exercises the store + HMAC chain deterministically.
echo "dev-verify: running a scripted chat turn (model may be slow)..."
turn="$(printf 'Reply with exactly the single word: READY\n' | aivyx_turn 2>&1)"
rc=$?
if [[ $rc -eq 0 ]] && grep -qi 'turn completed' <<<"$turn"; then
    pass "scripted chat turn completed"
else
    fail "scripted chat turn rc=$rc (no 'turn completed' marker)"
fi

after="$(audit_count)"
if [[ -n "$after" && -n "$base" ]] && (( after > base )); then
    pass "audit chain grew ${base} → ${after} events, integrity OK"
else
    fail "audit chain did not grow (base=$base after=$after)"
fi

# =====================================================================
# TOOL-PATH TURNS — LLM-dependent, run standalone (no daemon).
# The fs.write result is checked here directly off the filesystem;
# the memory.write result is verified in the daemon section below,
# because the `memory` subcommand that reads it back is a daemon
# client.
# =====================================================================
section "tool-path turns (LLM-dependent — WARN on miss)"

mem_prompt='Call the memory.write tool now. Store, under topic "verify99", the exact text: codeword VERIFY-BANANA-7723. After the tool call returns, reply DONE.'
echo "dev-verify: running the memory.write turn..."
printf '%s\n' "$mem_prompt" | aivyx_turn >/dev/null 2>&1

fs_prompt='Call the fs.write tool now to create a file named probe99.txt with the exact contents: phase99-fs-ok. After the tool call returns, reply DONE.'
echo "dev-verify: running the fs.write turn..."
printf '%s\n' "$fs_prompt" | aivyx_turn >/dev/null 2>&1
if [[ -f "$DEV_DIR/sandbox/probe99.txt" ]] \
    && grep -q 'phase99-fs-ok' "$DEV_DIR/sandbox/probe99.txt" 2>/dev/null; then
    pass "fs.write tool path works (sandbox file created)"
else
    warn "fs.write probe: sandbox file absent — local model likely skipped the tool call"
fi

# =====================================================================
# DAEMON lifecycle + memory query. The `memory` subcommand is a
# daemon client, so the daemon must be up to (a) substrate-check
# `memory list` and (b) verify the memory.write turn above.
# =====================================================================
section "daemon lifecycle + memory query"
DAEMON_LOG="$RUN_DIR/daemon.log"
aivyx_run daemon run >"$DAEMON_LOG" 2>&1 </dev/null &
dpid=$!
SOCK="$RUN_DIR/aivyx/daemon.sock"
for _ in $(seq 1 40); do
    [[ -S "$SOCK" ]] && break
    kill -0 "$dpid" 2>/dev/null || break
    sleep 0.5
done
if [[ -S "$SOCK" ]]; then
    pass "daemon run started, socket is live"

    if aivyx_run daemon status </dev/null >/dev/null 2>&1; then
        pass "daemon status reports the running daemon"
    else
        fail "daemon status failed against a running daemon"
    fi

    if out="$(aivyx_run memory list </dev/null 2>&1)"; then
        pass "aivyx memory list ran against the daemon (rc=0)"
    else
        fail "aivyx memory list failed: $out"
    fi

    search="$(aivyx_run memory search VERIFY-BANANA-7723 </dev/null 2>&1)"
    if grep -q 'VERIFY-BANANA-7723' <<<"$search"; then
        pass "memory.write → memory.read tool path works (codeword recalled)"
    else
        warn "memory.write probe: codeword not found — local model likely skipped the tool call"
    fi

    if aivyx_run daemon stop </dev/null >/dev/null 2>&1; then
        pass "daemon stop accepted"
    else
        fail "daemon stop failed"
    fi
    for _ in $(seq 1 20); do
        kill -0 "$dpid" 2>/dev/null || break
        sleep 0.5
    done
    if kill -0 "$dpid" 2>/dev/null; then
        fail "daemon process still alive after stop — killing"
        kill "$dpid" 2>/dev/null
    else
        pass "daemon process exited cleanly after stop"
    fi
else
    fail "daemon run never produced a socket (see $DAEMON_LOG)"
    kill "$dpid" 2>/dev/null
fi
wait "$dpid" 2>/dev/null

# =====================================================================
# SUMMARY
# =====================================================================
echo
echo "=== dev-verify summary ==="
echo "  PASS: $PASS   WARN: $WARN   FAIL: $FAIL"
if [[ $FAIL -gt 0 ]]; then
    echo "dev-verify: FAILED — $FAIL substrate check(s) failed" >&2
    exit 1
fi
echo "dev-verify: OK — substrate verified ($WARN tool-path warning(s))"
exit 0
