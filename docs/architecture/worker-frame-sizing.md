# Worker Frame Sizing (design)

> Status: **Design A implemented** (statement_text bounded at extraction; regression test
> `extracted_statement_text_is_bounded_for_oversized_types` over the `large-type` fixture). Designs B and C are not
> adopted; the `statement_digest` follow-up remains future work, tied to [probe-cache-scoping.md](probe-cache-scoping.md).
> Authored against lean-dup `0.1.0` on lean-rs `0.2.4`. Companion to
> [worker-protocol.md](worker-protocol.md), which owns the command contract; this doc owns **how large an extraction
> declaration row may be** — what it should carry, and the transport envelope that catches it when it carries too much.
> Independent of [probe-cache-scoping.md](probe-cache-scoping.md) — that is verdict-cache lifecycle; the two share no
> state and change separately.

## Symptom

`index-mathlib` on a project whose mathlib dependency is real (KanProofs, `v4.31.0-rc2`) fails fast:

```
error: worker protocol violation: worker protocol failed:
worker protocol frame too large: 1198811 bytes exceeds 1048576
```

The same failure blocks `audit --compare-mathlib`, which builds the same project-pinned mathlib index. A plain workspace
`audit` (KanProofs' own 10,789 declarations) succeeds — no workspace declaration row exceeds 1 MiB; only mathlib carries
rows that large.

## Root cause

The worker streams one `declaration_row` per protocol frame, and a frame is rejected before transmission if its
serialized JSON exceeds the negotiated `max_frame_bytes` (default `MAX_FRAME_BYTES` = **1 MiB**, negotiable per
connection to a 256 MiB hard cap). So the failing frame is **one declaration row** of ~1.14 MiB, not a batch.

What makes one row that large is the real finding. A declaration row carries **display + identity facts only**; semantic
comparison runs on the *separate* feature-key rows (`worker-protocol.md`: callers "may not rely on `statement_text` … as
semantic inputs"). `statement_text` has **no** consumer in any ranking, retrieval, or comparison path — it is
display-only, and the probe re-elaborates declarations by `declaration_id`, never from this text.

In `lean/LeanDup/Extract.lean`, the body and docstring are already bounded to an actionable summary
(`boundedSemanticText`, `maxDefinitionBodyChars = 4000`). But `statement_text` interpolates
`(← ppExpr decl.constInfo.type).pretty` — the **full** pretty-printed type — with **no bound**:

```lean
let typeText := (← ppExpr decl.constInfo.type).pretty   -- unbounded
-- statement_text = s!"{kind} {displayName} : {typeText}"
```

For a monster mathlib type that pretty-prints past 1 MiB, this one display field produces an over-cap frame and the
stream aborts. The full type is the internal `Expr` representation leaking through a *display* interface (§7.4: internal
representation ≠ interface) — and a megabyte type is not human-actionable. The field was simply omitted from the
bounding policy the body and docstring already follow.

This is a lean-dup **extraction defect**, not a lean-rs frame-cap problem: the 1 MiB default is correct, and the fix is
to stop emitting an unbounded display payload — not to widen the channel so it fits.

## Design it twice

| Design | Fixes root cause | Lossless for comparison | Bounds row size | Surface / cost |
| --- | --- | --- | --- | --- |
| A. Bound `statement_text` at extraction (match body policy) | **yes** | yes (display-only field) | yes, structurally | ~1 line of Lean |
| B. Raise the negotiated frame cap | no — admits the bloat | yes | no | one builder call; defense-in-depth only |
| C. Chunk over-cap rows into continuation frames | no — transports the bloat | yes | per-frame only | lean-rs protocol change + version bump |

### A — bound `statement_text` at extraction (recommended)

Pass the statement through `boundedSemanticText`, exactly as `definition_body_summary` and `docstring_text` already are.
Rows drop back under the 1 MiB default for every corpus; no cap change, no protocol change. This is the "pull complexity
down" fix at the source: the row interface is *display facts*, so it should carry a bounded display snippet, not the
full pretty-printed type. Beyond unblocking `index-mathlib` and `audit --compare-mathlib`, it shrinks index size,
worker→parent I/O, and memory across the whole pipeline.

Bound to apply: reuse `maxDefinitionBodyChars` (4000) or a sibling `maxStatementChars` if the statement warrants a
different budget; the value is a display-legibility choice, not a correctness one.

**Digest caveat (for the future probe cache).** The probe-cache content key in
[probe-cache-scoping.md](probe-cache-scoping.md) must address the declaration's *full* content. With a bounded display,
that key cannot be derived from `statement_text` — two declarations differing only past the bound would collide. The
extraction must therefore emit a separate `statement_digest` computed Lean-side over the **full** `typeText` *before*
truncation. Net shape: bounded display snippet + full-fidelity digest, with the full type never crossing the wire. This
is strictly better than streaming the full text and hashing it Rust-side.

### B — raise the frame cap (defense-in-depth, not the fix)

lean-rs exposes `LeanWorkerCapabilityBuilder::max_frame_bytes`; lean-dup never calls it, so connections run at 1 MiB.
Raising it to a finite generous value (e.g. 16 MiB, well under the 256 MiB hard cap; lean-dup's child is trusted, so the
memory-safety rationale for the low default does not bind) would also admit the current rows. But on its own it is the
wrong fix: it widens the channel to carry a megabyte of display text nobody reads, and a still-larger type would
re-break it. Worth keeping only as a **margin** behind A — so a row modestly over the *bounded* size (e.g. a long
docstring plus a near-budget statement) does not abort — not as the primary remedy.

### C — chunk over-cap rows (rejected here)

Splitting an over-cap row into reassembled continuation frames is the right transport design *when the payload is both
needed and genuinely unbounded*. Display text is neither — it is display, and it should be bounded — so chunking here
would pay a lean-rs protocol change and version bump to faithfully transport data we should not be sending. Reserve C
for a future payload that is genuinely large *and* required in full; it does not apply to `statement_text`.

## Recommendation

1. **Bound `statement_text` in `lean/LeanDup/Extract.lean`** (Design A) — the root-cause fix, consistent with the
   existing body/docstring policy. When the probe-cache work lands, add the `statement_digest` field so content
   addressing uses full fidelity.
2. **Optionally** raise lean-dup's negotiated `max_frame_bytes` to a modest finite value as defense-in-depth (Design B).
   Not required once A lands.
3. **Do not** reach for chunking (C) or an unbounded cap for this — both transport bloat the fix should eliminate.

## Performance notes

- Bounding the statement removes work everywhere downstream: smaller worker→parent frames, smaller `declarations` rows
  in SQLite, less memory held during a bulk index. The win scales with corpus size and is largest on mathlib.
- A huge pretty-printed type is *pretty-printer output* size, unrelated to elaboration cost, so the heartbeat budget
  that skips slow declarations does not address it. Bounding the emitted text is the right lever.
- If Design B is also applied, the cap bounds the parent's largest single `read_frame` allocation; keep it finite (≈16
  MiB), never the 256 MiB hard cap.

## Tests

- Regression (Rust/extraction, **landed**): `extracted_statement_text_is_bounded_for_oversized_types` in
  `crates/worker/src/worker/mod.rs` extracts the `tests/fixtures/large-type` module through the real worker and asserts
  `oversizedType`'s `statement_text` is bounded (≤ 4100 chars and ends with the `" ..."` truncation marker) while a sibling
  `smallType` in the same module is left intact — proving the bound is size-triggered, not blanket truncation.
- Integration (lean-dup): `index-mathlib` against a fixture whose mathlib-like dependency contains an oversized type
  completes (before Design A it aborted at the frame cap).
- When the digest field lands: two declarations whose types share the first `maxStatementChars` but differ later produce
  **different** `statement_digest` (guards the digest caveat).
