#!/usr/bin/env bash
#
# aivyx pre-commit hook — Phase 9 Task 3.
#
# Phase 8 Task 8 established the per-task `-D warnings` policy by
# convention. This hook enforces it at commit time so a dirty clippy
# run cannot slip into `main` the way it did in Phase 8 Task 4. See
# PHASE_9.md Q4 for the "Level 1: pre-commit hook" rationale.
#
# What it does:
#   1. Runs `cargo clippy --workspace --all-targets -- -D warnings`.
#   2. If clippy fails, the commit is aborted and the diagnostic is
#      left on the terminal so the developer can fix it.
#
# What it does *not* do:
#   - Run the test suite. Tests are much slower than clippy and this
#     hook should stay fast enough that developers do not habitually
#     `git commit --no-verify`. Tests belong in CI (Phase 10+).
#   - Catch `unsafe_code` hygiene. `#![forbid(unsafe_code)]` /
#     `#![deny(unsafe_code)]` in each crate's lib.rs already catches
#     that at compile time.
#
# Install with: ./scripts/install-hooks.sh
# Skip once:    git commit --no-verify   (frowned upon — see
#               PHASE_8.md Task 4 retrospective for why)

set -euo pipefail

echo "pre-commit: cargo clippy --workspace --all-targets -- -D warnings"

if ! cargo clippy --workspace --all-targets -- -D warnings; then
    echo ""
    echo "pre-commit: clippy failed. Fix warnings above before committing."
    echo "pre-commit: run \`cargo clippy --workspace --all-targets -- -D warnings\`"
    echo "pre-commit: to see the same output locally."
    exit 1
fi

echo "pre-commit: clippy clean."
