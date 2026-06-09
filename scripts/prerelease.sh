#!/usr/bin/env bash
# scripts/prerelease.sh — run every pre-release gate locally.
#
# This is the local mirror of `.github/workflows/ci.yml`'s `check` job: it
# verifies the pinned Lean toolchain, builds the Lake packages and
# fixtures, then runs the same Rust gates and `jq` schema/eval/report
# assertions CI runs. Passing locally is the fast feedback loop before a
# release tag — CI is a ~20-minute round trip.
#
# All gates are attempted even if an earlier one fails; the run ends with
# a pass/fail/skip summary and a non-zero exit if anything failed.
#
# Usage:
#   scripts/prerelease.sh            # all gates
#   scripts/prerelease.sh --quick    # skip the slow eval + report-contract fixture gates
#   scripts/prerelease.sh --help

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

QUICK=0

# -- logging ----------------------------------------------------------------

if [[ -t 1 ]]; then
	BOLD=$'\033[1m'
	GREEN=$'\033[32m'
	RED=$'\033[31m'
	YELLOW=$'\033[33m'
	RESET=$'\033[0m'
else
	BOLD="" GREEN="" RED="" YELLOW="" RESET=""
fi

log_step() { printf '\n%s==>%s %s%s%s\n' "$BOLD" "$RESET" "$BOLD" "$*" "$RESET"; }
log_ok() { printf '%s✓%s %s\n' "$GREEN" "$RESET" "$*"; }
log_warn() { printf '%s!%s %s\n' "$YELLOW" "$RESET" "$*" >&2; }
log_err() { printf '%s✗%s %s\n' "$RED" "$RESET" "$*" >&2; }

usage() { sed -n '2,/^$/p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; }

# -- arg parsing ------------------------------------------------------------

while [[ $# -gt 0 ]]; do
	case "$1" in
	--quick)
		QUICK=1
		shift
		;;
	-h | --help)
		usage
		exit 0
		;;
	*)
		log_err "unknown argument: $1"
		usage >&2
		exit 2
		;;
	esac
done

# -- preflight --------------------------------------------------------------

require_cmd() {
	if ! command -v "$1" >/dev/null 2>&1; then
		log_err "required command not found: $1${2:+ ($2)}"
		exit 2
	fi
}

require_cmd cargo "install via https://rustup.rs"
require_cmd jq "https://jqlang.github.io/jq/"
require_cmd lake "install via elan + leanprover/lean4"

# CI faithfulness: the workflow sets RUSTFLAGS="-D warnings" globally, so
# rustc warnings are hard errors during build/test, not just clippy lints.
export RUSTFLAGS="${RUSTFLAGS:--D warnings}"
export CARGO_TERM_COLOR="${CARGO_TERM_COLOR:-always}"

# -- toolchain check --------------------------------------------------------

TOOLCHAIN="$(tr -d '[:space:]' <lean/lean-toolchain)"
log_step "Pinned Lean toolchain: ${TOOLCHAIN}"
if command -v elan >/dev/null 2>&1; then
	if ! elan toolchain list 2>/dev/null | grep -qF "$TOOLCHAIN"; then
		log_err "Lean toolchain '${TOOLCHAIN}' is not installed."
		log_err "install it with: elan toolchain install ${TOOLCHAIN}"
		exit 2
	fi
	log_ok "toolchain installed"
else
	log_warn "elan not found; assuming '${TOOLCHAIN}' is on PATH via 'lake'"
fi

# -- gate runner ------------------------------------------------------------

declare -a PASSED=() FAILED=() SKIPPED=()

run_gate() {
	local name="$1"
	shift
	log_step "$name"
	local start=$SECONDS
	if "$@"; then
		log_ok "$name ($((SECONDS - start))s)"
		PASSED+=("$name")
	else
		local rc=$?
		log_err "$name FAILED in $((SECONDS - start))s (exit $rc)"
		FAILED+=("$name")
	fi
}

# -- Lake builds (prerequisite for the Rust tests) --------------------------

build_lake() {
	# The lean/ default target is the worker exe and does not pull in the
	# top-level LeanDup.olean the audit pipeline imports; build the library
	# explicitly alongside the exe, exactly as ci.yml does.
	(cd lean && lake build LeanDup lean_dup_worker)
	local fixture
	for fixture in tests/fixtures/tiny tests/fixtures/external tests/fixtures/source-backed; do
		(cd "$fixture" && lake build)
	done
}
run_gate "lake build (lean/ + fixtures)" build_lake

# -- Rust gates -------------------------------------------------------------

run_gate "cargo fmt --all -- --check" \
	cargo fmt --all -- --check

run_gate "cargo clippy --workspace --all-targets --locked -- -D warnings" \
	cargo clippy --workspace --all-targets --locked -- -D warnings

run_gate "cargo test --workspace --locked" \
	cargo test --workspace --locked

run_gate "boundaries (cargo test -p lean-dup-cli --test boundaries --locked)" \
	cargo test -p lean-dup-cli --test boundaries --locked

# -- Release diagnostics: schema + protocol contracts -----------------------

gate_diagnostics() {
	mkdir -p target
	cargo run -p lean-dup-cli --locked -- --version
	cargo run -p lean-dup-cli --locked -- doctor \
		--workspace tests/fixtures/tiny --module Tiny --format json \
		>target/doctor-ci.json
	jq -e '.report_schema_version == "lean-dup.report.v3"' target/doctor-ci.json >/dev/null
	jq -e '.worker.protocol_version == "lean-dup.worker.v1"' target/doctor-ci.json >/dev/null
}
run_gate "release diagnostics (report.v3 / worker.v1)" gate_diagnostics

# -- Slow fixture gates (skippable with --quick) ----------------------------

gate_evals() {
	mkdir -p target/eval
	cargo run -p lean-dup-cli --locked -- eval \
		--suite default --format json --output target/eval/default.json \
		>target/eval/default.stdout
	cargo run -p lean-dup-cli --locked -- eval \
		--suite hard-negatives --format json --output target/eval/hard-negatives.json \
		>target/eval/hard-negatives.stdout
	jq -e '.status == "ok"' target/eval/default.json >/dev/null
	jq -e '.metrics.hard_negative_hits.found == 0' target/eval/hard-negatives.json >/dev/null
}

gate_report_contract() {
	mkdir -p target/report-contract
	cargo run -p lean-dup-cli --locked -- audit \
		--workspace tests/fixtures/tiny --module Tiny \
		--no-semantic-probes --format json \
		>target/report-contract/ordinary-audit.json
	jq -e '.report_schema_version == "lean-dup.report.v3"' target/report-contract/ordinary-audit.json >/dev/null
	jq -e '.visible_groups_emitted <= .visible_group_limit' target/report-contract/ordinary-audit.json >/dev/null
	jq -e '.review.groups == null' target/report-contract/ordinary-audit.json >/dev/null
}

# Release portability: prove the worker builds from published crates with no
# sibling `../lean-rs` / `../lean-semantic-search` checkout. Archives HEAD into a
# throwaway tree, so a stray patch/path dependency fails here. Slow (runs a full
# Lake build via the worker's build.rs), hence skippable with --quick.
gate_portability() {
	scripts/portability-smoke.sh
}

if [[ "$QUICK" == 1 ]]; then
	SKIPPED+=("fixture evals (--quick)")
	SKIPPED+=("report contract fixture (--quick)")
	SKIPPED+=("portability smoke (--quick)")
else
	run_gate "fixture evals (default ok / hard-negatives clean)" gate_evals
	run_gate "report contract fixture" gate_report_contract
	run_gate "portability smoke (no sibling checkout)" gate_portability
fi

# -- summary ----------------------------------------------------------------

printf '\n%s====== Pre-release summary ======%s\n' "$BOLD" "$RESET"
printf 'Lean toolchain: %s\n' "$TOOLCHAIN"
printf '\npassed (%d):\n' "${#PASSED[@]}"
for name in "${PASSED[@]}"; do printf '  %s✓%s %s\n' "$GREEN" "$RESET" "$name"; done

if ((${#SKIPPED[@]} > 0)); then
	printf '\nskipped (%d):\n' "${#SKIPPED[@]}"
	for name in "${SKIPPED[@]}"; do printf '  %s-%s %s\n' "$YELLOW" "$RESET" "$name"; done
fi

if ((${#FAILED[@]} > 0)); then
	printf '\n%sfailed (%d):%s\n' "$RED" "${#FAILED[@]}" "$RESET"
	for name in "${FAILED[@]}"; do printf '  %s✗%s %s\n' "$RED" "$RESET" "$name"; done
	exit 1
fi

printf '\n%sAll gates passed.%s\n' "$GREEN" "$RESET"
