#!/usr/bin/env bash
# .claude/hooks/contract-guard.sh — PostToolUse advisory (Edit|Write|MultiEdit).
#
# Surfaces two repo rules that CI catches late (or not at all) as
# *non-blocking* reminders. The edit has already happened; we only print
# guidance to stderr and exit 2 so the text is returned to Claude to act
# on. We never undo anything.
#
#   1. Schema/protocol versions are contracts (CLAUDE.md). Touching a
#      `lean-dup.<x>.vN` literal or a *_SCHEMA_VERSION / protocol_version
#      constant means the matching jq assertion in .github/workflows/ci.yml
#      and the relevant docs/architecture/*.md must move in lockstep.
#   2. stdout stays parseable (CLAUDE.md). `println!`/`print!` outside the
#      CLI/render output path can corrupt `--format json` stdout.
#
# KNOWN TRADEOFF: check 2 is a plain grep, so it can false-positive on
# #[cfg(test)] code or doc examples that legitimately print. It is
# advisory only, so a rare nudge is cheap; an AST-aware check is the wrong
# altitude for a hook. If it gets noisy, scope it to non-`tests/` paths.
set -euo pipefail

command -v jq >/dev/null 2>&1 || exit 0
input="$(cat)"
file="$(printf '%s' "$input" | jq -r '.tool_input.file_path // empty')"
[ -n "$file" ] && [ -f "$file" ] || exit 0

# Only Rust sources under crates/ carry these contracts.
case "$file" in
*/crates/*.rs) : ;;
*) exit 0 ;;
esac

msgs=()

# 1. Schema / protocol contract touch.
if grep -Eq '"lean-dup\.[a-z]+\.v[0-9]|REPORT_SCHEMA_VERSION|CACHE_KEY_VERSION|protocol_version' "$file"; then
	msgs+=("• You touched a schema/protocol version in $file. Per CLAUDE.md these are contracts: update the matching jq assertion in .github/workflows/ci.yml AND the relevant docs/architecture/*.md in the same change. Audit JSON must stay additive.")
fi

# 2. stdout cleanliness — printing is only allowed on the CLI/render output
#    path. Everywhere else, progress/display must go to stderr.
case "$file" in
*/crates/cli/src/* | */crates/report/src/render.rs) : ;;
*)
	if grep -Eq '\b(println!|print!)[[:space:]]*\(' "$file"; then
		msgs+=("• $file uses println!/print! outside the CLI/render output path. This can corrupt --format json stdout (CLAUDE.md: 'stdout stays parseable'). Route user-facing output through the report renderer; send progress to stderr via eprintln!/the diagnostics crate.")
	fi
	;;
esac

if [ "${#msgs[@]}" -gt 0 ]; then
	printf 'contract-guard:\n' >&2
	printf '%s\n' "${msgs[@]}" >&2
	exit 2
fi

exit 0
