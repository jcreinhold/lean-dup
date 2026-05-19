# Cache Validity Lifecycle

`lean-dup` reuses indexes across audits. This document defines when an index is still good, when it must be rebuilt,
and how `doctor` and the hidden `cache-cleanup` keep the cache directory honest.

For the pipeline that uses the cache, see
[06-end-to-end-architecture.md](/Users/jcreinhold/Code/lean-dup/docs/architecture/06-end-to-end-architecture.md).

## What makes an index stale

Freshness is determined by the inputs that can change Lean semantic rows. Nothing else.

### Every index

| Ingredient | Why |
| --- | --- |
| index schema and provenance versions | row layout might have changed |
| worker protocol version | wire contract might have changed |
| worker, extraction, feature, probe semantic versions | algorithm might have changed |
| Lean worker source digest | local worker code might have changed |
| Lean toolchain text | elaboration might differ |
| Lake file and manifest digests | dependencies or build settings might have changed |
| selected module roots and include policies | the requested universe might differ |
| selected Lean source file digests | the audited code itself might have changed |

### External and mathlib indexes only

| Ingredient | Why |
| --- | --- |
| project-pinned mathlib source digests | the pinned mathlib might have moved |
| external workspace package, manifest, Git state when available | external corpus identity |
| compiled-artifact stamps when `require_oleans` is in the workflow | oleans are part of the contract |

Freshness is **not** determined by unrelated non-Lean files or workspace git dirtiness. A README change, a note, or
an unrelated generated artifact does not invalidate the cache.

Project-pinned mathlib indexes are content-addressed and shared under the normal cache root. Their key excludes the
audited project's absolute root and includes the pinned mathlib source content and the project execution toolchain.

The cache root defaults to `~/.cache/lean-dup`; `LEAN_DUP_CACHE_DIR` overrides it.

## Doctor diagnostics

`doctor --format json` reports:

- cache root and total indexed disk bytes;
- one entry per cache label;
- latest-pointer status: `ok | missing | target-missing | corrupt-pointer`;
- per-entry status: `current | stale | corrupt | missing | unchecked`;
- schema version when readable;
- static vs source-backed provenance;
- declaration count when readable;
- disk bytes and reasons.

`unchecked` means the index is readable but the current `doctor` invocation did not provide enough source context to
judge freshness. That is normal for arbitrary external labels.

## Cleanup

The hidden `cache-cleanup` command is dry-run by default. `--execute` is required to remove anything. When passed a
workspace and module, it also protects the cache entry the current workspace request would publish.

Cleanup may remove only index directories that are not:

- the target of any readable `latest.json` pointer;
- an expected current index entry for the command's current requests.

Active latest entries are protected even when stale. Rebuilds publish a new latest pointer before old active entries
become cleanup candidates.

## Why this shape

The tempting design—`git status --porcelain` plus an ad hoc cleanup script—leaks project-wide state into every
index cache, invalidates on unrelated files, and turns cleanup into an operator responsibility. The chosen design
keeps the index store as the authoritative cache-key constructor; a separate lifecycle module receives only expected
index entries and reports `current | stale | corrupt | missing | unchecked`. Cleanup protects every active
`latest.json` target and every current expected entry before considering deletion. Callers ask for cache health, not
for table rows, digests, or deletion steps.

## Evidence commands

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cd /Users/jcreinhold/Code/lean-dup/lean && lake build

cargo test -p lean-dup-cli cache_lifecycle
cargo test -p lean-dup-cli cache_key_ignores_unrelated_files_and_tracks_lake_inputs
cargo test -p lean-dup-cli hidden_cache_cleanup
cargo test -p lean-dup-cli doctor_json_reports_cache_lifecycle_diagnostics

cargo run -p lean-dup-cli -- doctor \
  --workspace /Users/jcreinhold/Code/lean-dup/tests/fixtures/tiny \
  --module Tiny --format json \
  > target/cache/doctor-production.json
```
