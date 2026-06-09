#!/usr/bin/env bash
# scripts/portability-smoke.sh — prove lean-dup builds from published crates
# with no sibling source checkout.
#
# Exports a committed tree into a throwaway directory *outside* the workspace,
# so `../lean-rs` and `../lean-semantic-search` are unreachable, then resolves
# and builds the worker. If any dependency still needs a sibling path (a stray
# `[patch.crates-io]` or path dependency), resolution or the build fails there
# instead of silently succeeding against the developer's local checkouts.
#
# The build runs the worker's build.rs, which materializes the semantic-search
# and interop-shims Lean sources and runs `lake build LeanDup`, so an installed
# Lean toolchain (elan) is required — the check is heavy by design.
#
# `git archive` only sees committed files, so run this after committing the
# change under test (or pass an explicit ref).
#
# Usage:
#   scripts/portability-smoke.sh            # archive HEAD
#   scripts/portability-smoke.sh <ref>      # archive a specific commit/tag

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REF="${1:-HEAD}"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "==> exporting $REF into a sibling-free tree: $tmp"
git -C "$REPO_ROOT" archive --format=tar "$REF" | tar -C "$tmp" -xf -

cd "$tmp"

echo "==> cargo metadata (must resolve from crates.io, no patch/path deps)"
cargo metadata --format-version 1 --no-deps >/dev/null

echo "==> cargo build -p lean-dup-worker (from published crates only)"
cargo build -p lean-dup-worker

echo "✓ portability smoke check passed: lean-dup-worker built with no sibling checkout"
