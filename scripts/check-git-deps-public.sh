#!/usr/bin/env bash
# Confirms every git dependency declared in the workspace Cargo.toml is
# anonymously cloneable — no credential helper, no cached token, no
# interactive prompt fallback. This is the exact property whose silent
# failure broke the v0.9.0 release pipeline (see
# docs/superpowers/specs/2026-09-05-release-distribution-integrity-design.md):
# a git dependency pinned to a private repo passes `cargo check` on any
# machine that already has cached access to it, but fails for CI and for
# any real outside contributor. Exits non-zero, naming every offending
# URL, if any dependency requires authentication to fetch.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CARGO_TOML="$REPO_ROOT/Cargo.toml"

if [ ! -f "$CARGO_TOML" ]; then
  echo "error: $CARGO_TOML not found" >&2
  exit 2
fi

urls=$(grep -oE 'git = "https://[^"]+"' "$CARGO_TOML" | sed -E 's/git = "(.*)"/\1/' | sort -u) || true

if [ -z "$urls" ]; then
  echo "No git dependencies found in $CARGO_TOML — nothing to check."
  exit 0
fi

failed=0
while IFS= read -r url; do
  echo "Checking anonymous clone access: $url"
  # -c credential.helper= clears any configured credential helper for
  # this invocation only (an empty value resets the helper list — this
  # is documented git behavior, not a no-op). GIT_TERMINAL_PROMPT=0 makes
  # git fail immediately instead of hanging on an interactive
  # username/password prompt when a repo needs auth. Together these make
  # the check genuinely credential-less regardless of what's cached on
  # the machine running it.
  if GIT_TERMINAL_PROMPT=0 git -c credential.helper= ls-remote "$url" > /dev/null 2>&1; then
    echo "  OK: anonymously cloneable"
  else
    echo "  FAIL: could not be cloned without credentials" >&2
    failed=1
  fi
done <<< "$urls"

echo ""
if [ "$failed" -ne 0 ]; then
  echo "One or more git dependencies require authentication to clone." >&2
  echo "This breaks CI (the default GITHUB_TOKEN can't authenticate against" >&2
  echo "a different repo) and any outside contributor's build-from-source path." >&2
  exit 1
fi

echo "All git dependencies are anonymously cloneable."
