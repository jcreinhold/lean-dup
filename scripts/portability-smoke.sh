#!/usr/bin/env bash
# scripts/portability-smoke.sh — prove `cargo install lean-dup` builds the parent
# CLI from published crates with no sibling source checkout, Lean-free.
#
# Exports a committed tree into a throwaway directory *outside* the workspace,
# so `../lean-rs` and `../lean-semantic-search` are unreachable, then resolves
# and builds the parent CLI. If any dependency still needs a sibling path (a
# stray `[patch.crates-io]` or path dependency), resolution or the build fails
# there instead of silently succeeding against the developer's local checkouts.
#
# Two invariants are checked: the parent builds without a Lean toolchain
# (`cargo install lean-dup` is pure Rust), and the resulting binary does not link
# `libleanshared` (the per-toolchain worker that links Lean is built later by
# `lean-dup install-worker`).
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

echo "==> cargo build --release -p lean-dup (parent CLI, must be Lean-free)"
cargo build --release -p lean-dup

echo "==> assert the parent CLI does not link libleanshared"
bin="target/release/lean-dup"
if command -v otool >/dev/null 2>&1; then
	linked="$(otool -L "$bin" | grep -i libleanshared || true)"
elif command -v ldd >/dev/null 2>&1; then
	linked="$(ldd "$bin" | grep -i libleanshared || true)"
else
	echo "!! neither otool nor ldd found; skipping link-invariant check" >&2
	linked=""
fi
if [[ -n "$linked" ]]; then
	echo "✗ parent CLI links libleanshared — cargo install lean-dup would require a Lean runtime" >&2
	echo "$linked" >&2
	exit 1
fi

echo "✓ portability smoke check passed: lean-dup built Lean-free with no sibling checkout"
