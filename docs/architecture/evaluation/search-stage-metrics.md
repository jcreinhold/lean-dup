# Search Stage Metrics

Evaluation output carries stage-level denominators. Without them a quality failure cannot be located: a positive
could be lost at candidate generation, ranking, semantic verification, or visibility, and those are different bugs.

## Metric contract

`metrics.stage_metrics` is additive and JSON-safe. Existing `metrics.recall`, `shown_queue_precision`,
`hard_negative_hits`, `visible_groups`, `probe_unavailable`, candidate counts, timings, and table output stay
supported.

| Metric | What it counts |
| --- | --- |
| `candidate_generation_recall` | labeled positives present anywhere in retrieved candidates |
| `top_k_recall_before_final_ranking` | recall at requested `k` over the current retrieval ordering |
| `ranked_recall` | current public recall metric, repeated under stage vocabulary |
| `visible_queue_precision` | shown true positives / shown candidates |
| `hard_negative_survival` | hard negatives at generated, top-k, and visible stages |
| `candidate_count_by_origin` | candidates grouped by `workspace`, `mathlib`, `external:<label>`, … |
| `candidate_count_by_feature_family` | candidates grouped by stable retrieval-evidence family |
| `generated_candidate_count_by_policy` | generated observations grouped by private search policy label |
| `generated_candidate_count_by_feature_family` | generated observations grouped by feature family |
| `hard_negative_generated_by_feature_family` | generated hard negatives grouped by feature family |
| `semantic_verification` | planned, cached, worker, and unavailable probe counts (zeros for retrieval-only suites) |

Generated observations are tracked separately from the bounded ranked queue. Candidate generation means "the pair
was created by the private generation stage"; top-k and visible metrics describe later survival.

## Feature families

Stable diagnostic vocabulary. Not retrieval keys, posting-table names, or Lean feature encodings.

```
statement_fingerprint
safe_permutation_fingerprint
connective_fingerprint
conclusion_fingerprint
role_conclusion_const
role_hypothesis_const
role_head
role_other
other
unknown
```

## Why stage metrics, not raw retrieval contributions

Raw contribution keys make short-term debugging easy and leak Lean-owned encodings, SQLite query shape, and retrieval
internals into the report contract. Later retrieval refactors become JSON migrations. Stable family vocabulary keeps
the eval boundary owning observability while retrieval keeps its internal keys private.

## Commands

```sh
# Fast fixture evidence
cargo run -p lean-dup-cli -- eval --suite default --format json
cargo run -p lean-dup-cli -- eval --suite hard-negatives --format json

# Production-gate evidence
cargo run -p lean-dup-cli -- eval --suite production-gate --format json \
  --output target/eval/production-gate.json
```

The aggregate `status` reports command/gate execution. Release-quality claims use the raw stage
denominators, especially KanProofs/mathlib recall and hard-negative survival.

## Known limitations

- Stage metrics measure existing behavior. They reveal bad behavior; they do not fix it.
- The semantic-verification counters are zero for retrieval-only suites; the slots exist so
  artifact shape stays stable when audit-backed observations are added.
- Top-k recall before final ranking uses the current first-stage selection order.
