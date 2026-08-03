# lean-dup: memory hardening + native-Lean4 worker transport

## Context

lean-rs 0.5/0.6 and lean-host-mcp 0.7/0.8 established the memory retention
model for long-lived Lean hosts (`lean-rs/docs/safety/long-session-memory.md`):

- Fresh `importModules` calls in one process grow **physical footprint** without
  bound (imported compacted regions are process-global; `Drop` never returns
  them). Pool imported environments; import once per process; cycle the process
  past a byte budget.
- RSS is the wrong metric (anti-correlated under memory pressure; counts clean
  file-backed olean pages). Use phys_footprint (macOS `proc_pid_rusage`,
  Linux `smaps_rollup Private_Dirty`) or Lean-side
  `non_memory_mapped_region_bytes` attribution.
- Bound import breadth per session; bracketed lightweight import queries; LRU
  brokers with idle reaper + dead-actor eviction (lean-host-mcp).

lean-dup already hit this exact wall: commit `8028780` fixed the **probe** path
(single-entry env cache, import once per session). Remaining problems:

1. `Extract.importRequestedModules` (lean/LeanDup/Extract.lean:423) and
   `Features.runProfiled` still re-import **per command** — same OOM
   antipattern, unfixed outside probe.
2. Worker stack pins lean-rs-worker-* **0.4** and carries the whole FFI
   apparatus: install-worker, capability dylib, dlopen chain, worker pool,
   cancellation-token bridges, frame-size negotiation.
3. Rust retrieval/verification (~10.5k lines in crates/search) is unprofiled
   for clone/allocation antipatterns.
4. lean-fmt proves the native model: a pure Lean 4 tool, no FFI, run against
   the target project's toolchain.

**Decision (user-approved): phased.** Phase 1 hardens memory + profiles Rust.
Phase 2 replaces the FFI worker transport with a **native Lean 4 subprocess
server** (JSONL over stdin/stdout), built as a **per-toolchain prebuilt exe**
by `install-worker`. Rust keeps CLI, SQLite index, retrieval, ranking,
reporting. Vector/embedding/eval crates stay as external extensions.

## Approach

### Phase 1 — hardening (current architecture, independently shippable)

1. **Shared session environment in Lean.** Generalize `Probe.probeEnvCache`
   (lean/LeanDup/Probe.lean:498-566) into one session-environment cache keyed
   by `(modules signature, options)` shared by extract, features, and probe.
   `Extract.withAcceptedDeclarationsProfiled` and `Features.runProfiled` take
   an env from the cache instead of calling `importRequestedModules` per
   command. Single-entry, replace-never-accumulate (the probe idiom). This
   code carries over unchanged to Phase 2.
2. **Profile the Rust hot paths before touching them** (rust-performance
   skill): workload = the 31k-decl Mathlib-importing workspace audit from the
   `8028780` commit message. Reuse `crates/eval` stage metrics + `samply`.
   Targets from a first read: `retrieval.rs` (candidate/feature clones),
   `semantic_verification.rs` (chunk payload rebuilds), `index.rs` hydration.
   Remove only *measured* phase-local clones/allocations; report numbers.
3. **Probe-cache scoping, design C** (docs/architecture/probe-cache-scoping.md
   is already the approved design): key probe verdicts by the transitive
   import-closure digest of the two declarations' modules so one unrelated
   leaf edit no longer discards all 226 cached probes (172 s of a 226 s
   audit). Needs a worker query for per-module import closures.
4. Skip the lean-rs-worker 0.6.1 bump: Phase 2 deletes that dependency;
   the Phase-1 env cache removes the OOM without it.

### Phase 2 — native Lean 4 worker transport

**Lean side** (in the existing `lean/` Lake package):

- New `Main.lean`: a JSONL server. Reads one request line
  `{export, payload}` per command, dispatches to the existing
  Extract/Features/Probe/Index logic, writes framed response lines
  `{type: row|progress|diagnostic|summary, ...}` on stdout. Reuses the
  Phase-1 session env cache → imports once per process.
- `Index.lean`'s `LeanRsInterop` callback trampoline is replaced by direct
  stdout frame writes (same row payloads, same stream names
  `"declarations"`/`"features"`).
- `Capability.lean` (FFI export shims) is deleted; the request/response JSON
  schemas in `Protocol.lean` and docs/architecture/worker-protocol.md are the
  protocol — unchanged semantically, new framing.
- Cancellation: server checks an abort flag between declarations; Rust kills
  the child process for hard cancel/timeout (simpler than token bridges).

**Rust side**:

- `crates/worker`: replace `engine/pool.rs` + `engine/runtime.rs` with a
  subprocess engine: resolve installed exe per toolchain (existing
  `toolchain.rs` machinery unchanged), spawn it with `lake env` from the
  audited workspace root (Lake installs the project's olean search path — the
  same thing lean-rs's `import_workspace_root` does internally), read framed
  stdout lines, drive the same `WorkerCall`/sink API. `WorkerClient` public
  API is unchanged; the private engine seam (`engine/mod.rs`) already exists
  for exactly this swap.
- `crates/cli/src/install_worker.rs`: build the Lean exe per toolchain
  (`elan run <toolchain> lake build` on a staged copy of `lean/`), install to
  the same `<data_local>/lean-dup/workers/<toolchain-id>/` layout, keep the
  provenance sidecar + smoke test (now: spawn exe, run `version` command).
- Worker lifecycle: one child per audited workspace; restart on crash or when
  the module signature changes. Import-once per process + process exit =
  the entire memory problem class gone (no compacted-region retention across
  commands, no pool, no LRU).

**Deleted in Phase 2**: crates/worker-child, crates/capability-source (incl.
its vendored LeanDup copy — the byte-identical-copy maintenance burden in
`8028780` disappears), lean-rs-interop-shims / lean-rs-worker-* /
lean-semantic-search-runtime deps from workspace Cargo.toml, FFI docs.

**Docs**: update docs/architecture/overview.md (Lean/Rust boundary rule —
boundary moves from FFI capability to JSONL subprocess), worker-protocol.md,
worker-migration-spike.md, README/AGENTS/CLAUDE.

## Files to modify

Phase 1:
- `lean/LeanDup/{Extract,Features,Probe}.lean` (+ vendored copy until Phase 2)
- `crates/search/src/{retrieval,semantic_verification}.rs`, `crates/index/src/index.rs` (measured only)
- `crates/index` probe-cache schema + `semantic_verification.rs` key scoping

Phase 2:
- `lean/Main.lean` (new), `lean/lakefile.lean` (exe target), `lean/LeanDup/{Index,Protocol}.lean`, delete `Capability.lean`
- `crates/worker/src/worker/engine/{pool,runtime}.rs` → new `engine/subprocess.rs`; `crates/worker/src/toolchain.rs` (exe name/layout)
- `crates/cli/src/install_worker.rs`, `crates/cli/src/commands.rs` (doctor)
- workspace `Cargo.toml`; delete `crates/{worker-child,capability-source}`
- docs + changelogs

## Reuse

- `Probe.lean` env-cache idiom (lean/LeanDup/Probe.lean:498-606) → Phase 1 shared cache.
- `Protocol.lean` (542 lines) request/response JSON — the wire schema stays.
- `crates/worker/src/toolchain.rs` — install layout, ToolchainId, sidecar: unchanged.
- `WorkerClient` + private `WorkerEngine` seam (engine/mod.rs) — built for this swap; `FakeEngine` keeps unit tests Lean-free.
- payload.rs request builders + `decode_stream_item` — same payloads, new framing.
- lean-rs long-session-memory doc's measurement methodology for Phase 1 verification.
- eval stage metrics (`crates/eval/src/eval/stage_metrics.rs`) as the perf harness.

## Steps

Phase 1:
- [ ] Extract shared session-env cache from Probe into `LeanDup.Extract` (or new `LeanDup/Session.lean`); route extract/features/probe through it; keep signature-replace semantics; mirror into vendored copy
- [ ] Measure audit on the 31k-decl workspace: worker phys_footprint flat across extract+features+probe commands; record numbers
- [ ] Profile Rust retrieval/verification (samply + stage metrics); remove measured antipatterns only; re-measure
- [ ] Implement probe-cache scoping design C (import-closure digest in probe key)
- [ ] `cargo nextest run --workspace`, `lake build` in `lean/`, full audit regression (64-pair expected result from 8028780)

Phase 2:
- [ ] `lean/Main.lean` JSONL server: dispatch loop, framing, abort flag, reuse session env
- [ ] `Index.lean`: trampoline → stdout frames; delete `Capability.lean`; delete vendored copy + capability-source crate
- [ ] Rust subprocess engine behind `WorkerEngine`; delete pool.rs/runtime.rs + lean-rs deps
- [ ] install-worker: per-toolchain `lake build` of the exe; sidecar + smoke test; doctor
- [ ] Worker restart-on-signature-change; kill-on-timeout/cancel
- [ ] Docs/charter update; CHANGELOG; README

## Verification

- **Memory**: 31k-decl Mathlib-importing workspace audit; worker
  phys_footprint (not RSS — see lean-rs doc) flat across all commands;
  no per-command import growth. Compare against the 8028780 baseline
  (~1.5–2 GB steady).
- **Perf**: cold/warm audit wall time before/after each phase, same workload;
  probe-cache scoping: one unrelated leaf edit → warm probe reuse (target:
  ~12.5 s warm path from probe-cache-scoping.md instead of 186 s).
- **Correctness**: audit results byte-identical on the regression workspace
  (64 pairs, 38 verified / 26 rejected); `cargo nextest run --workspace`;
  `lake build`; smoke test in install-worker.
