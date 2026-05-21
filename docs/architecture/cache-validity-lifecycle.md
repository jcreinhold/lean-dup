# Cache Validity Lifecycle

## Design note

This document owns the release-facing cache lifecycle contract. Project resolution owns workspace,
Lake, toolchain, and source identity; the index crate owns cache keys, latest-pointer interpretation,
schema/protocol compatibility, and cleanup eligibility; worker owns protocol and semantic-version
facts; report/doctor owns concise diagnostics. The smallest public interface is cache status,
schema/provenance kind, declaration counts, disk-cost facts, path-role fingerprints, and actionable
reasons.

Absolute filesystem paths, cache directory layout, latest-pointer storage, SQLite table names,
worker rows, and retrieval keys must not leak upward or sideways. The preserved user-facing
capability is deterministic cache reuse and a `doctor` report that explains stale, missing,
corrupt, unchecked, or reusable entries. The Python-era behavior intentionally discarded is using
project-wide dirtiness or operator path inspection as the cache validity rule.

## Design it twice

Three designs were considered:

- expose cache roots, index files, and table-level details in reports;
- invalidate every cache on any workspace dirtiness;
- make project/index own precise lifecycle facts while doctor/report project redacted diagnostics.

The third design is deeper. It keeps cache mechanics below the report boundary, avoids false rebuilds
from unrelated files, and still gives operators stable status and next-action reasons.

`lean-dup` reuses indexes across audits. This document defines when an index is still good, when it must be rebuilt,
and how `doctor` and the hidden `cache-cleanup` keep the cache directory honest.

For the pipeline that uses the cache, see [end-to-end-architecture.md](end-to-end-architecture.md).

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

- redacted cache-root and workspace path references as `{ kind, fingerprint }`;
- total indexed disk bytes;
- one entry per cache label;
- latest-pointer status: `ok | missing | target-missing | corrupt-pointer`;
- per-entry status: `current | stale | corrupt | missing | unchecked`;
- schema version when readable;
- static vs source-backed provenance;
- declaration count when readable;
- disk bytes and reasons.

The JSON projection does not expose absolute private paths, cache-entry directory names, file names such as
`index.sqlite` or `latest.json`, SQLite table names, posting-list vocabulary, or worker-row details. The index crate
continues to own concrete paths internally so it can open, reuse, invalidate, and clean up caches.

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
(cd lean && lake build)

cargo test -p lean-dup-cli cache_lifecycle
cargo test -p lean-dup-cli cache_key_ignores_unrelated_files_and_tracks_lake_inputs
cargo test -p lean-dup-cli hidden_cache_cleanup
cargo test -p lean-dup-cli doctor_json_reports_cache_lifecycle_diagnostics

cargo run -p lean-dup-cli -- doctor \
  --workspace tests/fixtures/tiny --module Tiny --format json \
  > target/cache/doctor-production.json
```

Prompt 57 generated `target/cache/doctor-production.json` with `LEAN_DUP_CACHE_DIR=target/cache/doctor-cache`.
Observed facts:

- `status = ok`;
- `lean_version = Lean 4.30.0-rc2`;
- requested workspace, Lake root, Lake file, cache root, cache labels, cache entries, and cache stores are redacted path
  references;
- the expected audit-workspace entry is `missing` before an index has been built and reports `missing cache store`;
- leak check over `target/cache/doctor-production.json` and redacted provenance summaries found no absolute private
  paths, storage file names, SQLite/posting vocabulary, or worker-row text.
