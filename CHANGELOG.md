# Changelog

All notable changes to lean-dup are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). It is pre-1.0, so breaking changes bump the minor version.

## [Unreleased]

## [0.2.0] - 2026-06-11

### Changed

- Upgraded the `lean-rs` worker crates (`lean-rs-worker-parent` / `-child`, `-protocol`, `lean-rs-interop-shims`,
  `lean-toolchain`) to 0.2.2 and bumped the Lean toolchain pin to `leanprover/lean4:v4.31.0-rc2` (header-identical to
  `-rc1`).
- The Lean worker now runs entirely through the `lean-rs-worker-parent` pool, loading the `LeanDup` shared-facet
  capability dylib via the new `lean-dup-worker-child` binary, instead of a per-call subprocess JSONL transport. Only
  the child links `libleanshared`; the audit process stays free of the Lean runtime ABI. The `lean-dup.worker.v1` schema
  and command semantics are unchanged, and CLI output is byte-identical (full integration suite passes with no golden
  refreshes).
- The index cache key now folds in worker *substrate* facts (the pool transport-protocol version and pooled worker
  runtime version from the handshake), so a worker-runtime change invalidates stale entries. Ephemeral pool state (ids,
  pids, queue counters, lease keys) is excluded.
- `LeanDup` capability build-root materialization now uses the shared `lean-toolchain` source-package helper instead of
  a hand-written cache/copy path. The worker command exports and `LeanDup` package behavior are unchanged, but generated
  roots now share the same lock, provenance, generated-toolchain, and manifest-validation mechanics as the other
  packaged Lean capabilities.
- `lean/lakefile.lean` now requires its Lean dependencies (`lean-semantic-search`, `lean_rs_interop_shims`) from the
  *published* upstream sources pinned to release tags (`v0.3.1`, `v0.2.2`) via Lake git requires, instead of a sibling
  `../../lean-rs` checkout and a pre-materialized `LEAN_DUP_SEMANTIC_SEARCH_ROOT`. A clean `lake build LeanDup` now
  resolves from any checkout, so CI builds `lean/` without vendoring sibling repos. This mirrors the worker `build.rs`
  path, which already materializes the same crates' Lean sources.
- Bumped the `lean-semantic-search-*` runtime crates (and the matching `lean/lakefile.lean` Lake git require) to 0.3.1,
  which deduplicates role features in O(n) via a hash set instead of the previous per-insert linear scan. The emitted
  feature rows are byte-identical, so `features.roles.v3` and the index cache key are unchanged — a pure upstream
  performance fix.
- `index-mathlib` no longer overruns the worker frame limit on large workspaces. The parent→child index request now
  hoists the uniform `origin`/`source_root` out of the per-module list and streams bare module names (≈1.29 MiB → 0.45
  MiB for an 8k-module mathlib) instead of repeating identical metadata on every entry; parsers still accept the
  per-object form. Extraction also parallelizes across a bounded thread pool by default (one disjoint module chunk per
  task over the shared read-only environment), cutting a full mathlib index ~2.7× (474 s → 176 s at 4 threads) for ~12%
  more resident memory. Tunable via `LEAN_DUP_MAX_FRAME_BYTES` and the mathlib index thread cap. See
  `docs/architecture/worker-frame-sizing.md`.
- MSRV floor raised to Rust 1.91, matching the adopted lean-rs 0.2.0 crates.

### Added

- `WorkerIdentity` / `WorkerSubstrateFacts` worker DTOs and the private `WorkerEngine` / `PoolEngine` /
  `LeanDupCapabilityRuntime` engine seam in `lean-dup-worker`.
- `docs/architecture/validation/worker-migration-validation.md` recording parity, substrate-key behavior, and deferred
  follow-ups.

### Removed

- The Lean subprocess driver (`lean/LeanDup/Worker.lean`), the `lean_exe lean_dup_worker` target, and the Rust
  subprocess transport modules (`transport.rs`, `subprocess.rs`, `protocol.rs`, `capability_extract.rs`).

## [0.1.0]

- Initial pre-release: read-only duplication auditor for Lean 4 Lake workspaces.
