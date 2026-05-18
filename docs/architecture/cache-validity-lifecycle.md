# Cache Validity Lifecycle

This document records the production cache-validity boundary from prompt 24.

For the current end-to-end architecture around cache use, `doctor`, and hidden cleanup, see
[06-end-to-end-architecture.md](/Users/jcreinhold/Code/lean-dup/docs/architecture/06-end-to-end-architecture.md).

## Design Note

The cache lifecycle boundary owns the hidden knowledge for cache validity: source-relevant fingerprint ingredients,
expected index entries, latest-pointer interpretation, stale/corrupt/missing classification, disk-usage accounting, and
safe cleanup rules.

Its smallest public interface is `doctor` diagnostics plus one hidden cleanup command. Audit, index, retrieval, ranking,
and reporting callers continue to request indexes by label or path; they do not learn cache-key JSON, SQLite metadata
keys, latest-pointer layout, source digest ordering, or deletion policy.

These decisions must not leak upward or sideways:

- cache-key serialization and digest construction;
- SQLite metadata table layout and schema checks;
- latest-pointer file shape;
- the distinction between active, expected, stale, corrupt, and unchecked entries;
- disk traversal and cleanup safety policy.

The validated user-facing capability preserved here is read-only local duplicate auditing with reusable workspace,
external, and project-pinned mathlib indexes. The common cache root remains `~/.cache/lean-dup` unless
`LEAN_DUP_CACHE_DIR` overrides it.

Python-era behavior intentionally discarded: cache freshness no longer depends on broad repository dirtiness or Python
cache layout. Python artifacts may inform regression labels, but they are not production cache identity.

## Design It Twice

**Rejected: git-status invalidation plus ad hoc cleanup.** It is easy to add `git status --porcelain` or cleanup shell
scripts, but that leaks project-wide state into every index cache. It invalidates on unrelated files, makes reuse
unpredictable, and turns cleanup into an operator responsibility.

**Chosen: source-relevant cache lifecycle boundary.** The index store still owns cache-key construction. The lifecycle
module receives only expected index entries and reports whether cached entries are current, stale, corrupt, missing, or
unchecked. Cleanup protects every active `latest.json` target and every current expected entry before considering
deletion. This is deeper because callers ask for cache health, not for table rows, digests, git state, or directory
deletion steps.

## Validity Contract

Index freshness is determined by the inputs that can change Lean semantic rows:

- index schema and provenance versions;
- worker protocol, worker, extraction, feature, and probe semantic versions;
- Lean worker source digest;
- Lean toolchain text;
- Lake file and Lake manifest digests;
- selected module roots and include policies;
- selected Lean source file digests;
- project-pinned mathlib source digests for `index-mathlib` and `audit --compare-mathlib`.

Index freshness is not determined by unrelated non-Lean files or workspace git dirtiness. This preserves reuse when a
README, note, or unrelated generated artifact changes.

Project-pinned mathlib indexes remain content-addressed and shared under the normal cache root. Their cache key excludes
the audited project's absolute root and includes the pinned mathlib source content and the project execution toolchain.

## Doctor Diagnostics

`doctor --format json` reports:

- cache root and total indexed disk bytes;
- one entry per cache label;
- latest-pointer status: `ok`, `missing`, `target-missing`, or `corrupt-pointer`;
- per-entry status: `current`, `stale`, `corrupt`, `missing`, or `unchecked`;
- schema version when readable;
- static versus source-backed provenance;
- declaration count when readable;
- disk bytes and reasons.

`unchecked` means the index is readable, but the current `doctor` invocation did not provide enough source context to
judge freshness. This is normal for arbitrary external labels.

## Cleanup Contract

The hidden `cache-cleanup` command is dry-run by default. `--execute` is required to remove anything. When passed a
workspace and module, it also protects the cache entry that the current workspace request would publish.

Cleanup may remove only index directories that are not:

- the target of any readable `latest.json` pointer;
- an expected current index entry for the command's current requests.

Active latest entries are protected even when stale. Rebuilding should publish a new latest pointer before old active
entries become cleanup candidates.

## Evidence Commands

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cd /Users/jcreinhold/Code/lean-dup/lean && lake build
```

Focused cache evidence:

```sh
cargo test -p lean-dup-cli cache_lifecycle
cargo test -p lean-dup-cli cache_key_ignores_unrelated_files_and_tracks_lake_inputs
cargo test -p lean-dup-cli hidden_cache_cleanup
cargo test -p lean-dup-cli doctor_json_reports_cache_lifecycle_diagnostics
```

Manual diagnostic artifact:

```sh
cargo run -p lean-dup-cli -- doctor \
  --workspace /Users/jcreinhold/Code/lean-dup/tests/fixtures/tiny \
  --module Tiny --format json \
  > target/cache/doctor-production.json
```

## Red Flag Review

- **Shallow module:** avoided. The lifecycle boundary classifies cache health and owns cleanup safety; it is not a thin
  wrapper over directory listing.
- **Pass-through wrapper:** avoided. `doctor` receives summarized cache health rather than forwarding raw index-store or
  SQLite facts.
- **Temporal decomposition:** avoided. The design is organized around cache validity and lifecycle state, not around the
  order of resolving, opening, checking, and cleaning files.
- **Information leakage:** avoided. Callers do not inspect cache-key JSON, SQLite metadata keys, latest-pointer layout,
  or deletion rules.
- **Special-general mixture:** contained. Project-pinned mathlib is one source-relevant cache-key policy inside the
  index boundary, while lifecycle diagnostics remain label-general.
- **Conjoined methods:** no remaining red flag. Cache-key construction, lifecycle diagnosis, and cleanup communicate
  through expected index entries rather than shared mutable phase state.
- **Hard-to-describe public API:** no remaining red flag. The public behavior is: `doctor` explains cache health, and
  hidden cleanup removes only unprotected entries when explicitly executed.
- **Implementation details contaminating interface comments:** avoided. Interface comments describe cache lifecycle
  guarantees and caller obligations, not SQL tables or digest serialization.
