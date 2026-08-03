# Probe Cache Scoping (design)

> Status: **implemented** (post-`0.2.4`). Companion to
> [cache-validity-lifecycle.md](cache-validity-lifecycle.md), which owns the *index* lifecycle; this doc owns the
> *semantic probe* lifecycle, which is currently coupled to the index lifecycle and over-invalidates.

## Problem

Semantic probe results — the verdicts from running the Lean worker to decide whether two declarations are
definitionally/statement equal — are the most expensive artifact lean-dup produces. On KanProofs (`v4.31.0-rc2`, 10,789
declarations) the probe run dominates an audit:

| Audit (measured, static tree) | probes run | wall time |
| --- | --- | --- |
| cold (`--no-semantic-probes`, floor) | 0 | 54 s |
| cold (full) | 226 | 226 s |
| warm, no source change | 0 (226/226 cached) | 12.5 s |
| warm, **one unrelated leaf file edited** | **226 (0 cached)** | **186 s** |

The probe run is ~172 s of the 226 s cold audit. The warm path proves the cache works and is highly effective (18×
speedup, 226/226 reuse). The last row is the defect: editing a single source file — even a leaf that appears in **none**
of the probed pairs — discards the entire probe cache and re-runs all 226 probes.

### Root cause

The probe cache is a `probe_cache(pair_key, payload_json)` table stored *inside* each per-`cache_id` `index.sqlite`. The
`cache_id` is `hex_digest(index_cache_key)`, and `index_cache_key` includes the content digest of **every** workspace
source file (see [cache-validity-lifecycle.md](cache-validity-lifecycle.md) — "selected Lean source file digests"). So
any source edit produces a new `cache_id`, a new index directory, and a fresh **empty** `probe_cache`. Probe lookup
(`semantic_verification.rs`, `cached_probe_result`) consults only the current entry; there is no carry-forward. The
probe key (`probe_cache_key = sha256({pair_id, left_declaration_id, right_declaration_id})`) is already independent of
the index key — the coupling is purely a storage-location accident, not a correctness requirement of the key itself.

This is **sound but coarse**: the index lifecycle's whole-corpus invalidation is correct for the *index* (the corpus
identity genuinely changed), but the probe cache inherits it needlessly.

## Why the obvious fix is wrong

The tempting fix — move the probe cache to a shared store keyed by
`sha256(left_declaration_content, right_declaration_content, versions)` — is **unsound**. The probes are `exact-theorem`
and `reducible-definition` obligations: they unfold reducible definitions and consult instances, so the verdict for a
pair `(A, B)` can depend on the **ambient environment**, not just the text of `A` and `B`. Editing a reducible
definition or an instance `C` elsewhere can change whether `A` and `B` are judged equal. A probe store keyed only on `A`
and `B` would serve a stale verdict after such an edit.

The current whole-workspace key is conservative precisely because it captures every declaration that *could* be in
scope. The correct optimization is not to drop ambient dependencies from the key, but to scope them to what a given
probe can actually reach.

## Design it twice

| Design | Sound? | Survives unrelated edit? | Cost |
| --- | --- | --- | --- |
| A. whole-workspace digest (current) | yes | no — re-probes everything | none (status quo) |
| B. `(A,B)`-content only | **no** | yes (but wrongly) | low; rejected |
| C. `(A,B)`-content + transitive-import-closure digest | yes | yes (correctly) | worker import-graph extraction |

**Design C** is the target: a probe verdict is fully determined by the two declarations' content, the prover semantics
(probe/lean versions), and the **transitive import closure** of the two declarations' modules — that closure is exactly
the set of declarations the elaborator can bring into scope. Editing a file outside both closures cannot change the
verdict, so the cached result stays valid. Editing anything inside a closure changes that closure's digest and correctly
invalidates the affected probes only.

## Design C — specification

### Probe key

```
probe_key = sha256(json({
    probe_version,                 // prover algorithm identity
    lean_version,                  // elaboration identity
    obligation_kind,               // exact-theorem | reducible-definition | ...
    left  : { content_digest, closure_digest },
    right : { content_digest, closure_digest },
}))
```

- `content_digest` = `sha256(kind, visibility, modifiers, statement_text, definition_body_summary)` — all already
  columns in the `declarations` table.
- `closure_digest` = digest over the transitive import closure of the declaration's module: the set of
  `(module, module_content_digest)` reachable from that module. Normalize pair order (`left ≤ right`) so `(A,B)` and
  `(B,A)` share a key.

### Storage

A single shared store per cache root, `<cache_root>/probes/<label>.sqlite`, table
`probe_cache(probe_key, payload_json)`. It is **not** under any `cache_id` directory, so it survives index rebuilds. It
is consulted and written during `audit` regardless of which index `cache_id` is current.

### Worker import graph (the new capability)

`closure_digest` needs each module's transitive import set. As implemented, option 1 (pure Rust) carries this:
`crates/index/src/import_graph.rs` parses each built module's `.ilean` `directImports` JSON, computes the transitive
closure as a fixpoint, and folds each member's Lean **source** digest into the closure digest (falling back to the
`.ilean` bytes when a source is not visible). Resolution searches the workspace build directory and every
`.lake/packages/*` build directory, so dependency modules resolve alongside workspace modules. No worker round-trip,
no protocol change. Option 2 (a worker `import_graph` command) remains the fallback if header parsing proves brittle
across toolchains.

### Lifecycle integration

- **`cache-cleanup`** must treat `probes/<label>.sqlite` as a managed artifact, not an orphan to delete. Pruning policy:
  drop rows whose `probe_version` / `lean_version` no longer match the current worker identity (cheap GC), and an
  optional size cap. The per-`cache_id` `probe_cache` tables disappear with their index dirs as today; no migration of
  old rows is needed (they re-warm once).
- **`doctor`** reports the shared store's size and row count alongside the existing per-label cache facts.
- **`cache-validity-lifecycle.md`** gains a "Probe cache" subsection stating the probe lifecycle is keyed independently
  of the index lifecycle, on content + closure + prover versions.

### Tests

- Unit: `probe_key` changes with each input (content, closure, probe_version, lean_version, obligation_kind) and is
  pair-order stable.
- Unit: editing a module **outside** both closures leaves `probe_key` unchanged; editing one **inside** a closure
  changes it.
- Integration (fixture): two audits with an unrelated-file edit between them reuse probes (`cached_hits == planned`,
  `worker_pairs == 0`); an edit to a module in a probed pair's closure re-probes exactly the affected pairs.
- Soundness regression: edit a reducible def in a pair's closure → the pair is re-probed (guards against the Design-B
  unsoundness).

## Expected payoff

The edit→re-audit loop drops from ~186 s toward the ~54 s index-rebuild floor plus the handful of probes whose closures
actually changed — roughly **3×** on KanProofs, and larger on projects with more probed pairs. The index rebuild itself
(extract + features over all modules) is mandatory and unchanged; only the preventable probe re-run is eliminated.

## Out of scope (related findings)

- **`index-mathlib` frame-size failure.** Indexing a real mathlib dependency fails fast with
  `worker protocol frame too large`. This is a worker-transport concern, independent of probe caching, with its own
  design in [worker-frame-sizing.md](worker-frame-sizing.md).
- **Whole-workspace re-extract.** The index rebuild re-extracts all modules on any edit. Extraction reads compiled
  oleans (cheap relative to probes), so this is lower priority, but the same closure machinery could later make
  extraction incremental.
