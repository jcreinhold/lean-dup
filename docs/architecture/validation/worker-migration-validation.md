# Worker migration validation

This records the closure of the Lean↔Rust worker migration (prompts 78–81): the cut-over from the per-call subprocess
JSONL transport to the `lean-rs-worker-parent` pool loading a `LeanDup` shared-facet capability dylib through the
`lean-dup-worker-child` binary. After this change Rust drives Lean entirely through the pool; no subprocess /
`WorkerTransport` / `ProtocolItem` / `lean_exe lean_dup_worker` code remains.

For the contract this validates, see [worker-protocol.md](../worker-protocol.md). For the spike that proved the seam on
the single `extract` command, see [worker-migration-spike.md](../worker-migration-spike.md).

## What changed

- All five worker commands run through the pool capability, not a subprocess: `version` is a json-command export
  (`lean_dup_capability_version`); `extract`, `features`, `probe`, and `index` are streaming exports
  (`lean_dup_capability_{extract,features,probe,index}`). The dylib
  (`lean/.lake/build/lib/liblean__dup__worker_LeanDup.dylib`) was confirmed to export all five symbols.
- The Lean subprocess driver (`lean/LeanDup/Worker.lean`) and the `lean_exe lean_dup_worker` target are deleted; the
  `LeanDup` shared-facet `lean_lib` is the only worker build and the package default target.
- The Rust subprocess substrate (`transport.rs`, `subprocess.rs`, `protocol.rs`, `capability_extract.rs`) is replaced by
  a private `engine/` seam: `WorkerEngine` (a concrete `Pool`/`Fake` enum, never a trait object), `PoolEngine`,
  `LeanDupCapabilityRuntime`, and a `#[cfg(test)] FakeEngine`.
- Worker substrate facts (`lean-rs-worker-parent` handshake transport-protocol version + pooled worker runtime version,
  via `lease.runtime_metadata()`) are folded into the index cache key. Pool ids, pids, queue counters, and lease keys
  are excluded.

## Parity and test results

Run on the pinned toolchain against the `tiny` / `external` / `source-backed` Lean fixtures.

| Suite | Result |
| --- | --- |
| `cargo build --workspace --locked` | clean (exit 0) |
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | clean |
| `cargo test -p lean-dup-worker` | 8 passed (Fake-engine unit + pool-backed `version`/`worker_identity`/`extract`/`features`/`probe` integration) |
| `cargo test -p lean-dup-index` | 17 passed (incl. new `cache_key_tracks_worker_substrate_facts`) |
| `cargo test -p lean-dup-cli --test boundaries` | 7 passed |
| `cargo test -p lean-dup-cli --test cli` | 48 passed — **byte-parity gate** |
| `cargo test --workspace --locked` | 205 passed, 0 failed |

**Byte-parity:** the CLI integration suite passed with **no golden refreshes**. The audit/index/show/diff report bytes
are identical to the pre-migration output; no row body or substrate-version metadata delta surfaced in any golden.

## Cache-key substrate deltas

`cache_key_tracks_worker_substrate_facts` asserts the intended invalidation surface:

- identical substrate facts → identical cache key (no ephemeral pool state leaks in);
- a transport-protocol-version bump → distinct key (invalidates);
- a pooled-worker runtime-version bump → distinct key (invalidates).

The semantic algorithm versions, Lake inputs, toolchain, and selected roots continue to drive invalidation exactly as
before; substrate facts are additive to the key.

## Performance note

`cargo run -p lean-dup-cli -- perf --workload fixture-audit` reports cost classes `sqlite-index`, `retrieval-ranking`,
and `reporting` only. The subprocess `worker-startup` / `transport` cost classes that existed pre-migration are gone:
the warm pooled session is reused across commands per workspace, so there is no per-call process spawn or stdin/stdout
framing cost to account for.

## Deviations from the prompt plan

- **`doctor` and `metadata` are not capability exports.** The audit-path `doctor` CLI subcommand was always composed
  Rust-side from `version` plus workspace/cache checks — it never called a worker `doctor` command — so no Lean `doctor`
  export was added. Substrate facts come from the parent pool handshake (`lease.runtime_metadata()`), not a Lean
  `metadata` export. Both omissions remove protocol surface with no Rust consumer.

## Deferred follow-ups

- **Mathlib-scale A/B benchmark.** The fixture-scale perf note above shows the subprocess startup/transport cost class
  vanishing, but a full mathlib-scale before/after wall-clock and RSS comparison needs an operator-supplied Lake
  workspace and is left as an operator follow-up.
- **Capability packaging debt.** `crates/worker/build.rs` still patches the dependency manifest by hand and resolves
  sibling-checkout Lake paths (`../../lean-rs`, `../../lean-semantic-search`). Per the
  [capability-runtime design](../../../../prompts/designs/2026-06-08-lean-capability-runtime-architecture.md) this is the
  flagged anti-pattern. It is isolated behind the private `LeanDupCapabilityRuntime` seam so the steady-state fix —
  replacing `from_build_manifest` with a `build_cached(...)` call into a package-owned runtime crate — touches only that
  one module, not the command path. Tracked as documented debt, not done in this migration.
</content>
</invoke>
