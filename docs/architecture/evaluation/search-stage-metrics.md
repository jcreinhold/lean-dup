# Search Stage Metrics And Retrieval Observability

Prompt 30 adds stage-level denominators to evaluation output. The goal is to locate quality failures before changing
retrieval, ranking, semantic verification, or visibility policy.

## Design Note

The stage-metrics boundary owns search-stage denominators, hard-negative survival, candidate origin counts, stable
retrieval feature-family names, semantic-verification counters, and aggregation across production-gate child suites.

Its smallest public interface is additive eval JSON under `metrics.stage_metrics`. Existing `metrics.recall`,
`shown_queue_precision`, `hard_negative_hits`, `visible_groups`, `probe_unavailable`, candidate counts, timings, and
table output remain supported.

These decisions must not leak upward or sideways:

- SQLite table names, posting keys, raw Lean-owned feature keys, and hydration mechanics;
- ranking thresholds, scorer constants, review profile internals, and report visibility policy;
- Lean worker rows, probe chunks, JSONL framing, and probe-cache keys;
- private KanProofs paths or prompt-specific artifact layout.

The preserved capability is measurable read-only duplicate-audit quality. Users can keep running the same eval suites
while JSON consumers gain enough stage information to tell whether positives are lost during candidate generation,
top-k ranking, semantic verification, or visible-queue filtering.

Python-era behavior intentionally discarded:

- aggregate command completion as a quality pass;
- anecdotal "the candidate looked related" inspection;
- exposing broad retrieval key dumps as observability;
- measuring one mixed duplicate bucket without stage denominators.

## Design It Twice

**Rejected: expose raw retrieval contributions in eval JSON.** Raw contribution keys would make debugging easy in the
short term, but they would leak Lean-owned encodings, SQLite query shape, and retrieval internals into the report
contract. Later retrieval refactors would become JSON migrations.

**Chosen: stage metrics with stable feature families.** Eval records where each labeled pair survives and groups
retrieval evidence into stable families such as `statement_fingerprint`, `safe_permutation_fingerprint`, and
`role_conclusion_const`. This is deeper because the eval boundary owns observability vocabulary while retrieval keeps
its internal key representation private.

## Metric Contract

`metrics.stage_metrics` is additive and JSON-safe:

- `candidate_generation_recall`: labeled positives present anywhere in retrieved candidates;
- `top_k_recall_before_final_ranking`: recall at requested `k` values over the current retrieval ordering;
- `ranked_recall`: the current public recall metric repeated under the stage vocabulary;
- `visible_queue_precision`: shown true positives over shown candidates;
- `hard_negative_survival`: hard negatives present at candidate generation, at requested top-k values, and in the
  visible queue;
- `candidate_count_by_origin`: candidate observations grouped by stable origin labels such as `workspace`, `mathlib`,
  or `external:fixture`;
- `candidate_count_by_feature_family`: candidate observations grouped by stable retrieval evidence families;
- `semantic_verification`: planned, cached, worker, and unavailable probe counts. Retrieval-only eval suites report
  zeros until an audit-backed observation path supplies probe diagnostics.

The current eval observation surface is retrieval output. That means candidate generation currently means "candidate
appeared in the bounded retrieval result." Prompt 32 will split candidate generation from first-stage ranking more
cleanly; Prompt 30 records the current limitation rather than hiding it.

## Feature Families

Feature families are intentionally coarser than retrieval keys:

- `statement_fingerprint`
- `safe_permutation_fingerprint`
- `connective_fingerprint`
- `conclusion_fingerprint`
- `role_conclusion_const`
- `role_hypothesis_const`
- `role_head`
- `role_other`
- `other`
- `unknown`

The family names are stable diagnostic vocabulary. They are not a promise about storage tables, posting keys, or Lean
feature encodings.

## Evidence Commands

Fast fixture evidence:

```sh
cargo run -p lean-dup-rs -- eval --suite default --format json
cargo run -p lean-dup-rs -- eval --suite hard-negatives --format json
```

Production-gate evidence:

```sh
cargo run -p lean-dup-rs -- eval --suite production-gate --format json \
  --output target/eval/prompt30-production-gate.json
```

The aggregate `status` still reports command/gate execution status. Release-quality claims must use the raw stage
denominators, especially KanProofs/mathlib recall and hard-negative survival.

## Current Limitations

The stage metrics do not tune retrieval or ranking. They may therefore reveal existing bad behavior, such as positives
missing from bounded retrieval output or hard negatives surviving into the visible queue.

The semantic-verification counters are zero for retrieval-only suites. Audit-backed eval observations are a later
extension; this prompt only creates the stable metric slots.

Top-k recall before final ranking currently mirrors retrieval rank because the calibrated scorer and pair-feature
boundary do not exist yet. Prompt 31 creates pair feature artifacts, Prompt 32 separates candidate generation from
ranking, and Prompt 33 introduces calibrated symbolic scoring.

## Red Flag Review

- **Shallow module:** mitigated. The stage-metrics module computes stage denominators and feature-family diagnostics; it
  is not a pass-through alias for existing recall.
- **Pass-through wrapper:** mitigated. Existing recall remains for compatibility, but the new object adds generated,
  top-k, visible, origin, feature-family, and semantic counters.
- **Temporal decomposition:** residual and documented. Current eval observes retrieval output as the candidate stage
  because retrieval has not yet been split internally; Prompt 32 removes this limitation.
- **Information leakage:** mitigated. JSON exposes stable feature families, not raw retrieval keys, SQLite tables, Lean
  worker rows, or JSONL frames.
- **Special-general mixture:** mitigated. KanProofs evidence uses the same stage schema as fixtures; private path policy
  remains in suite orchestration.
- **Conjoined methods:** mitigated. Scoring still consumes normalized pairs and shown membership; stage metrics consume
  richer observation facts without changing scorer thresholds.
- **Hard-to-describe public API:** mitigated. The public addition is one `metrics.stage_metrics` object.
- **Implementation details contaminating interface comments:** mitigated. Interface comments describe stage meanings and
  stable family labels, not storage layout or retrieval algorithms.
