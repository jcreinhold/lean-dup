# Changelog

All notable changes to lean-dup are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). It is pre-1.0, so breaking changes bump the minor version.

## [Unreleased]

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
  *published* upstream sources pinned to release tags (`v0.3.0`, `v0.2.2`) via Lake git requires, instead of a sibling
  `../../lean-rs` checkout and a pre-materialized `LEAN_DUP_SEMANTIC_SEARCH_ROOT`. A clean `lake build LeanDup` now
  resolves from any checkout, so CI builds `lean/` without vendoring sibling repos. This mirrors the worker `build.rs`
  path, which already materializes the same crates' Lean sources.
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
