# Worker Migration Slice -- Capability `extract`

> **Status: superseded.** This slice converted only `extract`. The full cut-over (all five commands, subprocess
> substrate deleted, substrate facts in the cache key) is complete — see
> [validation/worker-migration-validation.md](validation/worker-migration-validation.md). This document is retained for
> historical context; statements below about commands "still on the subprocess path" or the surviving
> `lean_exe lean_dup_worker` target no longer hold.

This document records the migration gate that follows the Phase 0 spike
([guide](../../../prompts/guides/lean-dup-lean-rs-worker-migration-guide.md)). The slice converts one command,
`extract`, from lean-dup's subprocess JSONL worker to a `LeanDup` `sharedFacet` capability loaded by the
`lean-rs-worker-child` runtime and driven through a `lean-rs-worker-parent` pool.

The semantic boundary is unchanged: Lean computes declaration facts from the elaborated environment, and Rust owns
workspace discovery, lifecycle, persistence, retrieval, ranking, reporting, and evaluation. This slice changes the
transport substrate beneath `WorkerClient::extract_batch`; it does not change row schemas, ranking, retrieval, reports,
eval behavior, or the `lean-dup.worker.v1` semantic protocol.

## Design Record

The capability-load substrate hides child binary discovery, capability build and manifest lookup, the audited workspace
import root supplied by `ExtractBatch.workspace_root`, host-owned search-path construction, pool lease lifetime, and the
typed command boundary. Callers still ask `WorkerClient` for semantic facts. There is no new public engine trait, no new
`WorkerClient` constructor, no new worker-planning API, and no project-to-worker wiring surface.

The worker boundary must not leak pool internals, worker ids, process ids, lease ids, capability manifest paths,
`LEAN_PATH` contents, child environment details, Lean `Expr`s, transport frames, or subprocess request ids into `index`,
`search`, `cli`, reports, or eval. The validated user-facing capability is declaration extraction: the capability path
streams the same declaration-row payloads that the subprocess worker emits, so the existing `DeclarationRow` DTO
deserializes unchanged.

The subprocess-era implementation behavior intentionally discarded for capability-mode `extract` is the stdin/stdout
JSONL driver, Rust request ids, and the worker's own `initSearchPath` call. Those remain for the five commands still on
the subprocess path until the later migration prompts convert them.

## Search Path Design

Three designs were considered:

- Keep `LeanDup.Extract.importRequestedModules` calling `initSearchPath (← getBuildDir)` and pass the audited workspace
  through `LEAN_PATH` via a child environment knob. This matches the old executable but exposes environment names and
  bypasses the typed import-root seam added in `lean-rs-worker-parent`.
- Let the host session own imports through `LeanWorkerCapabilityBuilder::import_workspace_root`, and parameterize
  `LeanDup.Extract.importRequestedModules` so capability mode skips search-path initialization. This is the chosen
  design.
- Merge the capability dylib's Lake project root with the audited workspace's roots in one search path. This is rejected
  because it can shadow the audited workspace's declared dependency closure with lean-dup's dependencies.

The chosen design treats the audited workspace as one typed session input. The capability project still supplies the
`LeanDup` dylib and manifest; the audited workspace supplies the import search path for requested modules. The two roots
are not merged.

## Spike Evidence Kept

Phase 0 proved the migration preconditions:

- The audited-workspace import path is the production mechanism. The subprocess worker already runs under `lake env` in
  the audited workspace, so target modules resolve through the target workspace's Lake search path.
- The `LeanDup` shared facet builds. `lean/lakefile.lean` keeps both `defaultFacets := #[LeanLib.sharedFacet]` for
  `LeanDup` and the still-present `lean_exe lean_dup_worker` target.
- `lean/LeanDup/Capability.lean` exports `lean_dup_capability_extract` over unchanged extraction semantics, streaming
  rows through `LeanRsInterop.Worker.Stream`.
- `lean-rs-worker-parent` exposes `LeanWorkerCapabilityBuilder::import_workspace_root(path)`, and the import root is
  folded into `LeanWorkerSessionKey` so warm sessions do not alias across audited workspaces.
- The worker child preserves caller-set runtime facts needed by the worker stack; no generic child-env passthrough is
  used by lean-dup for audited workspace imports.

## Slice Shape

`crates/worker-child` is the app-owned child binary. Its only job is to delegate to
`lean_rs_worker_child::run_worker_child_stdio()`, and it is the only lean-dup crate that depends on
`lean-rs-worker-child` and links the Lean runtime.

`crates/worker/build.rs` builds the `LeanDup` shared capability with `lean_toolchain::CargoLeanCapability`, records the
trusted streaming export ABI for `lean_dup_capability_extract`, and emits the manifest environment variable consumed by
the parent-side worker crate. The Lean package uses the Lake identifier `lean_dup_worker` so the dylib name and module
initializer agree with the worker loader's identifier rules.

`crates/worker` keeps the public `WorkerClient` facade. Internally, `extract_batch` uses a private concrete adapter over
`LeanWorkerPool`, `LeanWorkerCapabilityBuilder`, `LeanWorkerChild::sibling("lean-dup-worker-child")`,
`import_workspace_root(batch.workspace_root)`, and `LeanWorkerStreamingCommand`. The other five commands keep using the
subprocess transport in this slice.

## Red Lines

- Do not merge the capability project's search paths with the audited workspace's search paths.
- Do not use the capability project's root as the audited workspace import root.
- Do not put `lean-rs-worker-child`, `lean-rs-host`, `lean-rs`, or `libleanshared` in the parent worker or CLI runtime
  graph. `lean-toolchain` and its metadata-only `lean-rs-sys` dependency are allowed for manifest and fingerprint
  handling, but must not make parent binaries link the Lean runtime.
- Do not remove `lean_exe lean_dup_worker` or subprocess code in this slice.
- Do not change `lean-dup.worker.v1`, the `DeclarationRow` payload shape, ranking, retrieval, reports, or eval.
- Do not expose pool internals, manifest paths, transport frames, child env, or Lean expressions through interface
  comments or public APIs.

## Proof Obligation

The gate is an integration-style worker test against `tests/fixtures/tiny`, a Lake workspace distinct from lean-dup's
own Lean package. It runs the same `ExtractBatch` through the old subprocess worker and the new pool-backed capability
path, sorts rows by declaration id, and asserts both `DeclarationRow` equality and identical `serde_json` serialization
for the row list.
