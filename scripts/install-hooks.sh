#!/usr/bin/env bash
#
# aivyx pre-commit hook installer — Phase 9 Task 3.
#
# Copies `scripts/pre-commit.sh` into `.git/hooks/pre-commit` and
# marks it executable. Run once after cloning the repo, or any time
# `scripts/pre-commit.sh` is updated.
#
# Why a separate installer instead of committing the hook directly
# to `.git/hooks/`? Because `.git/` is not version-controlled — hooks
# cannot live there directly. The canonical-source-of-truth approach
# is: the hook body lives in `scripts/` (tracked), and each developer
# runs this installer to copy it into their own `.git/hooks/`.
#
# Usage: ./scripts/install-hooks.sh
#
# This script is idempotent — running it twice is a no-op.

set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
HOOK_SRC="$REPO_ROOT/scripts/pre-commit.sh"
HOOK_DST="$REPO_ROOT/.git/hooks/pre-commit"

if [ ! -f "$HOOK_SRC" ]; then
    echo "install-hooks: missing source $HOOK_SRC" >&2
    exit 1
fi

if [ ! -d "$REPO_ROOT/.git/hooks" ]; then
    echo "install-hooks: .git/hooks not found; is this a git worktree?" >&2
    exit 1
fi

cp "$HOOK_SRC" "$HOOK_DST"
chmod +x "$HOOK_DST"

echo "install-hooks: installed $HOOK_DST"
echo "install-hooks: the pre-commit hook will now run"
echo "install-hooks:   cargo clippy --workspace --all-targets -- -D warnings"
echo "install-hooks: before every commit. Skip once with --no-verify"
echo "install-hooks: (discouraged — see PHASE_8.md Task 4)."
