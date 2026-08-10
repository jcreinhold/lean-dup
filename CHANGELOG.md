# Changelog

All notable changes to lean-dup are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). It is pre-1.0, so breaking changes bump the minor version.

## [Unreleased]

## [0.3.1] - 2026-08-10

### Changed

- Bumped the Lean toolchain pin from `leanprover/lean4:v4.33.0-rc2` to `leanprover/lean4:v4.33.0` (the final
  release, header-identical to rc2). The upstream `lean-semantic-search` release stays at tag `v0.7.0` (its
  `lean-toolchain` pins rc2; the final is ABI-identical, so the shared package compiles cleanly under it). All six
  `lean-toolchain` files and `PINNED_TOOLCHAIN` moved together; no Cargo, Rust-floor, or protocol changes.

## [0.3.0] - 2026-08-04

### Changed

- **Native Lean worker transport.** The `lean-rs-worker` FFI pool (a dlopen'd `LeanDup` capability dylib behind
  `lean-rs-worker-parent`/`lean-dup-worker-child`) is replaced by a native Lean 4 executable, `lean-dup-worker`,
  spawned under `lake env` in the audited workspace and driven over line-framed JSONL (`lean/LeanDup/Server.lean`).
  Command set, request payloads, row schemas, and stream names are unchanged; only the framing moves. One warm child
  serves every command of an audit; timeouts and cancellation kill it (the next command respawns, bounded by the
  Lean-side session cache). `install-worker` now builds the executable per toolchain with that toolchain's own `lake`
  — no Rust toolchain needed — and the smoke test spawns it and answers `version`. The index cache substrate facts are
  now Rust-owned transport constants (`2` = JSONL subprocess), so caches re-warm once across the swap. Deleted: the
  `lean-dup-worker-child` crate, the FFI capability exports (`LeanDup.Capability`), the `lean-rs-*`/`lean-toolchain`
  dependencies, and the worker-pool machinery.
- **Import-once everywhere.** The probe-only environment cache (from `8028780`) is generalized into a shared
  session-environment cache (`LeanDup.Extract.sessionEnv`) used by `extract`, `features`, `probe`, and `index`: one
  import per module signature per worker session. Previously `extract` and `features` re-imported the full
  (Mathlib-scale) environment per command, and each audit pipeline stage spawned a fresh worker that re-imported
  again; one `WorkerClient` engine is now shared across all stages of an audit. Measured on a Mathlib-importing
  workspace (`Proofs.Topology` audit): worker physical footprint 4.5 GiB → 0.6 GiB, cold audit 21 s → 16 s,
  identical findings.
- **Probe cache scoping (design C).** Semantic probe verdicts moved from the per-`cache_id` index SQLite into a shared
  store at `<cache_root>/probes/<label>.sqlite`, keyed by the two declarations' content digests plus the transitive
  import-closure digests of their modules (parsed from `.ilean` headers and Lean sources, no worker round-trip).
  Editing a file outside both closures no longer discards every cached verdict — measured: adding an unrelated file
  reused 54/54 probes (0 re-run) where the whole cache previously invalidated. `doctor` reports the shared store;
  `cache-cleanup` treats it as a managed artifact.
- Bumped the Lean toolchain pin to `leanprover/lean4:v4.33.0-rc2` and moved the upstream dependencies in lockstep:
  the `lean-semantic-search-*` crates and the `lean/lakefile.lean` Lake git require to release tag `v0.7.0`, which
  advances the transitive `lean-rs` line from `0.4` to `0.7` (`lean-rs-worker-protocol`, `lean-rs-abi`,
  `lean-toolchain`). lean-rs 0.7.0 adds 4.33.0-rc2 (byte-identical `lean.h` ABI with rc1) to its supported window;
  the wire protocol is unchanged. All six `lean-toolchain` files and `PINNED_TOOLCHAIN` moved together; the Rust
  floor stays 1.91.
- The `Report` enum's `Doctor` and `Perf` variants are now boxed (matching the already-boxed `Audit`/`Eval`/`Show`
  variants) to satisfy the `large_enum_variant` lint under current stable clippy.

### Removed

- The `lean-rs` worker stack dependency and the FFI capability transport (see above).

### Fixed

- `install-worker` failed its own smoke test on a fresh install dir: the smoke run resolves the worker through the
  parent's runtime path, which refuses a worker without a provenance sidecar, and the sidecar was only written
  *after* the smoke test. `install-worker` now writes a pending sidecar (no smoke outcome) before the smoke run and
  overwrites it with the real outcome.

## [0.2.4] - 2026-07-19

### Changed

- Bumped the Lean toolchain pin to `leanprover/lean4:v4.33.0-rc1` and moved the upstream dependencies in lockstep: the
  `lean-semantic-search-*` crates to release tag `v0.4.3` and the `lean-rs` worker crates (`lean-rs-worker-parent` /
  `-child`, `-protocol`, `lean-rs-interop-shims`) to release tag `v0.4.0`, with the matching `lean/lakefile.lean` Lake
  git requires updated to `v0.4.3` / `v0.4.0`. Both upstream tags ship under `v4.33.0-rc1`. The Cargo pins already
  resolved to these crate versions (`0.4.3` / `0.4.0`), so only the toolchain pins, Lake requires, and
  `PINNED_TOOLCHAIN` moved. The Rust floor stays 1.91 (`rust-version` unchanged) and the full workspace test suite
  passes with no golden refreshes.

## [0.2.3] - 2026-07-15

### Changed

- Bumped the Lean toolchain pin to `leanprover/lean4:v4.32.0` (the final stable release, ABI-identical to the previous
  `v4.32.0-rc1` pin) and moved the upstream dependencies in lockstep: the `lean-rs` worker crates
  (`lean-rs-worker-parent` / `-child`, `-protocol`, `lean-rs-interop-shims`, `lean-toolchain`) to release tag `v0.3.1`
  and the `lean-semantic-search-*` crates to release tag `v0.4.2`, with the matching `lean/lakefile.lean` Lake git
  requires updated to `v0.4.2` / `v0.3.1`. The Rust floor stays 1.91 (`rust-version` unchanged) and the full workspace
  test suite passes with no golden refreshes.

## [0.2.2] - 2026-06-26

### Added

- **`cargo install lean-dup`.** The CLI package is renamed `lean-dup-cli` → `lean-dup` and is published to crates.io
  along with its library crates. The parent installs as pure Rust — it does not link `libleanshared` — so a user can
  `cargo install lean-dup` with no Lean toolchain on the build path.
- **`lean-dup install-worker`.** A new command builds the toolchain-specific worker (the `lean-dup-worker-child` binary
  plus the `LeanDup` capability dylib and its dependency dylibs) on the user's machine, into
  `<data_local>/lean-dup/workers/<toolchain-id>/`. It defaults to the current project's `lean-toolchain` (override with
  `--toolchain`), runs a post-build smoke test that loads the capability through the real dlopen chain, and records a
  `worker.json` provenance sidecar (header digest, host version, smoke outcome). Flags: `--toolchain`, `--force`,
  `--source-dir`.
- **`lean-dup-capability-source` crate.** Packages the `LeanDup` Lean source (`lean/LeanDup*`) so it survives a
  crates.io unpack, and exposes the runtime capability build lifted out of the old `crates/worker/build.rs`. A drift
  test keeps the vendored copy byte-identical to the editable `lean/` dev project.

### Changed

- The worker is now resolved per audited workspace: the parent reads the project's `lean-toolchain` and loads the
  matching installed worker, or fails with the exact `lean-dup install-worker --toolchain <id>` command to run. Audits
  on a toolchain with no installed worker print an actionable hint instead of an opaque bootstrap error.
- `crates/worker/build.rs` is removed. `cargo install lean-dup` no longer builds Lean at crate-compile time; the
  capability + worker-child are built per-toolchain by `install-worker`. The `LEAN_DUP_WORKER_CHILD` dev override is
  replaced by `LEAN_DUP_WORKERS_DIR`, which points the parent at an install dir (CI and `scripts/prerelease.sh` use it).
- Added a tag-triggered `release.yml` that gates on the parent ⊥ `libleanshared` link invariant and publishes every
  crate to crates.io in dependency order (idempotent: versions already published are skipped).

## [0.2.1] - 2026-06-26

### Changed

- Bumped the Lean toolchain pin to `leanprover/lean4:v4.32.0-rc1` and moved the upstream dependencies in lockstep: the
  `lean-rs` worker crates (`lean-rs-worker-parent` / `-child`, `-protocol`, `lean-rs-interop-shims`, `lean-toolchain`)
  to 0.3.x (release tag `v0.3.0`) and the `lean-semantic-search-*` crates to 0.4.x (release tag `v0.4.1`), with the
  matching `lean/lakefile.lean` Lake git requires updated to `v0.4.1` / `v0.3.0`. The new tags ship under `v4.32.0-rc1`,
  so `lake build LeanDup` compiles the shared package cleanly under the root toolchain. The Rust floor stays 1.91
  (`rust-version` unchanged) and the full workspace test suite passes with no golden refreshes.

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
