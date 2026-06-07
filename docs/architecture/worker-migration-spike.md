# Worker Migration Spike — Workspace-Import Mechanism

Phase 0 of the `lean-rs-worker-*` migration
([guide](../../../prompts/guides/lean-dup-lean-rs-worker-migration-guide.md)). Goal: prove a pooled
`lean-rs-worker-child` running a `sharedFacet` `LeanDup` capability can import an **arbitrary audited workspace's**
`.olean`s, or pin down the exact `lean-rs` change required. This note records the findings and the resulting Phase 1
seam design.

## Conclusion

The migration is feasible. The audited-workspace import path is **already the production mechanism**, the `sharedFacet`
capability **builds**, and the worker child **preserves a caller-set environment**. The one true gap is that
`lean-rs-worker-parent`'s capability/pool builder does not *surface* a per-session import root, even though
`lean-rs-host` already computes one. Phase 1 is a **surfacing** job plus a session-key fold, not new import machinery.

## Evidence

### (a) The import mechanism is production-proven — not a new unknown

`LeanDup.Extract.importRequestedModules` (`lean/LeanDup/Extract.lean:364`) does
`Lean.enableInitializersExecution; initSearchPath (← getBuildDir)` then `importModules`. `initSearchPath` augments the
search path with the **`LEAN_PATH`** environment variable. The current subprocess worker is invoked via `lake env <bin>`
with `cwd` = the audited workspace, so `LEAN_PATH` already carries the target's `.lake/build/lib` (plus deps). Target
modules resolve through `LEAN_PATH` today; the worker's own code is statically linked and needs no import. So "child
imports the target via `LEAN_PATH`" is how lean-dup works now.

### (b) The `sharedFacet` capability dylib builds (empirically verified)

Spike changes on branch `adopt-lean-rs-worker`:

- `lean/lakefile.lean`: added `defaultFacets := #[LeanLib.sharedFacet]` to `lean_lib LeanDup` and
  `require «lean_rs_interop_shims» from ".." / ".." / "lean-rs" / "crates" / "lean-rs" / "shims" / "lean-rs-interop-shims"`
  (kept the `lean_exe` target so both coexist during migration).
- `lean/LeanDup/Capability.lean`: one streaming `@[export] lean_dup_capability_extract` over the unchanged
  `LeanDup.Extract.runProfiled`, emitting rows via `LeanRsInterop.Worker.Stream.{diagnostic,row,metadata,emitAll}`.

`lake build LeanDup` produces `.lake/build/lib/liblean-dup-worker_LeanDup.dylib` with a global
`_lean_dup_capability_extract` symbol; `lean-semantic-search` (shared) and the interop-shims C trampoline link cleanly
(the trampoline resolves transitively via `LeanRsInterop.dylib`, matching the `LeanRsInteropConsumer` fixture layering).
`lake build lean_dup_worker LeanDup` builds both targets.

### (c) The worker child preserves a caller-set environment

`LeanWorker::spawn` (`lean-rs/crates/lean-rs-worker-parent/src/supervisor.rs:752`) applies `config.env` and
`config.current_dir` to the child `Command` and does **not** `env_clear()`. Neither `lean-rs-worker-child` nor
`lean-rs-host` resets/overrides `LEAN_PATH` at startup (no `env_clear`/`set_var`/`remove_var`). So a `LEAN_PATH` set
through `LeanWorkerConfig::env("LEAN_PATH", …)` reaches the loaded capability's `initSearchPath`.

### The gap, and what already exists

The production *pool* path goes through `LeanWorkerCapabilityBuilder`, which **deliberately refuses a generic child-env
passthrough** (`capability.rs:207,1424-1432`) and keys sessions on `(project_root, package, lib_name, imports)`. There
is no current way to point a pooled session at an arbitrary audited workspace's search path.

But the computation already exists one layer down: `lean-rs-host`'s `LakeProject::olean_search_paths`
(`lean-rs/crates/lean-rs-host/src/host/lake.rs:106`) walks a project root's `lake-manifest.json` and returns every
package's olean dir for `initSearchPath` — exactly what importing an audited workspace needs. It is `pub(crate)`, driven
by the host session's root, and not surfaced through the worker boundary.

## Phase 1 seam design (refined)

Surface a **typed audited-workspace import root** through the capability/pool API, distinct from the capability dylib's
own `project_root` (the capability is lean-dup's `LeanDup`; the modules to import live in the audited workspace). Two
viable mechanisms — Phase 1 picks based on which the host session accepts most directly:

1. **Host-root mechanism (preferred).** Let the session carry an import root for the audited workspace and reuse
   `olean_search_paths` to `initSearchPath`. No env var; reuses existing manifest-walking.
2. **`LEAN_PATH` mechanism.** A typed `workspace_search_path(roots)` builder that sets the child's `LEAN_PATH` via
   `LeanWorkerConfig::env` (matches today's `lake env` exactly; lean-dup already computes the target `LEAN_PATH`).

Either way: keep the "no generic `env()`" rule (this is one typed concept — the audited workspace), and **fold the
import root into `LeanWorkerSessionKey`** (`pool.rs`) so different audited workspaces get distinct sessions and don't
alias a warm child. Update `docs/api-review/lean-rs-worker-parent-public.txt`, add tests (key distinctness,
out-of-package import fixture).

## Spike artifacts (kept as Phase 2 foundation)

The lakefile `sharedFacet` change and `LeanDup/Capability.lean` build and are retained as the starting point for Phase 2
(full capability conversion). A throwaway end-to-end Rust harness was judged unnecessary: (a) is production-proven,
(b)+(c) are verified above, and Phase 3's integration test against `tests/fixtures/tiny` will be the end-to-end gate
once the Phase 1 seam exists.
