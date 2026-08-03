# Worker Frame Sizing (design)

> Status: **implemented; transport superseded.** The `lean-rs-worker` FFI pool this doc's frame-cap negotiation refers
> to has been replaced by the native JSONL subprocess transport (no frame-size negotiation: lines are unbounded
> length-prefixed JSON). The payload-hoisting fix (`modules_payload`) remains load-bearing for request size. The
> remainder of this document is retained for historical context. The `index-mathlib` frame overrun is fixed by hoisting
> the repeated per-module constants out
> of the request manifest (`modules_payload` + the `Capability.lean` / `Protocol.lean` parsers), with the negotiated
> frame cap raised to a finite 16 MiB default as headroom. A separate, earlier change bounds `statement_text` at
> extraction — valid display-row hardening, but **not** the mathlib fix (see "A correction" below). Authored against
> lean-dup `0.1.0` on lean-rs `0.2.4`. Companion to [worker-protocol.md](worker-protocol.md), which owns the command
> contract; this doc owns **how large a single worker frame may be** — both the request manifest the parent sends and the
> rows the child streams back. Independent of [probe-cache-scoping.md](probe-cache-scoping.md).

## Symptom

`index-mathlib` on a project whose mathlib dependency is real (KanProofs, `v4.31.0-rc2`, 8180 modules) fails — and fails
in **~1 second**, before mathlib is even imported:

```
error: worker protocol violation: worker protocol failed:
worker protocol frame too large: 1288802 bytes exceeds 1048576
```

The same failure blocks `audit --compare-mathlib`, which builds the same project-pinned mathlib index. A plain workspace
`audit` (KanProofs' own ~10k declarations, ~hundreds of modules) succeeds.

## Root cause: the request manifest, not a row

The "~1 second, before import" timing is the key clue: import + extraction take minutes, so the over-cap frame is sent
at **session setup**, not during streaming. It is the **index request** (parent → child), not any declaration or feature
row (child → parent).

`modules_payload` (`crates/worker/src/worker/engine/payload.rs`) historically serialized one full descriptor per module:

```json
{ "module": "Mathlib.Analysis.…", "origin": "mathlib",
  "source_root": "/Users/…/.lake/packages/mathlib" }
```

`modules_for` (`crates/index/src/index.rs`) sets the **same** `origin` and `source_root` on every descriptor — they
describe the one audited corpus, not the individual module. So for mathlib's 8180 modules the request frame repeats those
two constant strings 8180 times. Measured on KanProofs' mathlib: the request is ~1.16–1.29 MiB, and **~61% of it
(~720 KB) is the repeated `source_root` + `origin`.** That single frame exceeds the 1 MiB default cap and the parent
aborts before sending it.

This was confirmed empirically by raising the cap to 64 MiB and completing the index: it produced 317,340 declarations
with **every stored row ≤ 25 KB** and zero declarations skipped — i.e. no row was ever near 1 MiB. The oversize was
entirely in the setup manifest.

### A correction

An earlier change bounded `statement_text` at extraction (the `large-type` fixture and the
`extracted_statement_text_is_bounded_for_oversized_types` regression). That is a genuine hardening — an unbounded
pretty-printed type *is* a display-only field leaking the internal `Expr` representation (§7.4), and a single monster
declaration's row could overrun the frame on some corpus. But it is **not** what broke `index-mathlib`: with that bound
in place the index still failed at the identical `1288802` bytes, because the offending frame is the request manifest,
not a row. The `statement_text` bound stands on its own merit; this doc no longer claims it fixes the mathlib overrun.

## Design

| Design | Fixes root cause | Lossless | Bounds frame | Cost |
| --- | --- | --- | --- | --- |
| **A. Hoist the uniform `origin`/`source_root` out of the per-module entries** | **yes** | yes | yes (≈ −61%) | request encoder + two Lean parsers |
| B. Raise the negotiated frame cap | no — transports the redundancy | yes | no | one builder call |
| C. Batch the manifest across several requests | partly | yes | per-request | re-imports per batch; loses the single-import win |

**A (implemented) — hoist the constants.** The request now carries `modules_origin` and (when present)
`modules_source_root` **once** at the top level, and `modules` is an array of bare name strings. The Lean parsers
(`Capability.lean` for the live capability path, `Protocol.lean` for the legacy subprocess path) read those defaults and
still accept per-entry objects with overrides, so the change is backward compatible and any future non-uniform caller
still round-trips. This drops the mathlib request from ~1.29 MiB to ~0.45 MiB — comfortably under even the old 1 MiB
default — with no data loss. It is the "don't repeat a per-request constant per element" fix at the source.

**B (also applied, as headroom not the fix).** The negotiated cap default is raised from the protocol's 1 MiB to a finite
**16 MiB** (`DEFAULT_MAX_FRAME_BYTES` in `runtime.rs`, overridable via `LEAN_DUP_MAX_FRAME_BYTES`, still clamped below the
256 MiB hard cap). With A in place mathlib fits under 1 MiB; B is margin so a corpus several times mathlib's size — or a
single large but legitimate row — does not re-break the stream. It is kept finite so the parent's largest single
`read_frame` allocation stays bounded.

**C (rejected).** Splitting the manifest into multiple requests would bound the frame regardless of corpus size, but each
request re-imports its modules, forfeiting the single-import optimization the streaming index depends on. Reserve it for a
future payload that is genuinely unbounded *and* required in full; the manifest is neither once A removes the redundancy.

## Performance note

The manifest fix is orthogonal to throughput, but the same investigation measured that the Lean worker is ~99.5% of an
`index-mathlib` build (472 s of 474 s for 317k declarations) and ran single-threaded by default. Extraction parallelism
now defaults to `min(available_parallelism, MAX_MATHLIB_INDEX_THREADS)` (cap raised 2 → 4) instead of 1; each task
elaborates a disjoint chunk over the shared, read-only environment. Measured on KanProofs' mathlib (default settings,
4 threads): the full build dropped **474 s → 176 s (2.7×)** while peak RSS rose only **5.9 GB → 6.6 GB (+12%)** — the
extra threads cost one chunk's working set each, not another copy of the multi-GB environment. See
`crates/index/src/index.rs` (`mathlib_index_threads`, `MAX_MATHLIB_INDEX_THREADS`).

## Tests

- Unit (`crates/worker/src/worker/engine/payload.rs`): `modules_payload` hoists uniform `origin`/`source_root` and streams
  bare names; omits `modules_source_root` when absent; falls back to per-object entries for a non-uniform set; and the
  hoisted encoding is strictly smaller than the per-object one.
- Integration (round-trip): the existing `public_client_{extract,features,probe}` worker tests exercise the hoisted
  request through the real `Capability.lean` parser on the `tiny` fixture.
- Regression (display rows, separate concern): `extracted_statement_text_is_bounded_for_oversized_types` over the
  `large-type` fixture keeps a single monster declaration's display row bounded.
- Manual: `index-mathlib` against KanProofs' real mathlib completes at the **default** frame cap and default parallelism
  (previously aborted in ~1 s).
