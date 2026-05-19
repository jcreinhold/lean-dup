# Candidate Generation

Candidate generation is now an explicit private search stage with named observability. The goal is high-recall
visibility into where positives are lost, without changing ranking thresholds, semantic-probe policy, report JSON,
command names, or ordinary mathlib hydration limits.

## Generation policies

Every generated candidate carries one of four diagnostic policy labels. They name the source, not the visibility
decision.

| Policy | Source |
| --- | --- |
| `local_duplicate_audit` | pairs generated within the audited workspace corpus |
| `mathlib_comparison` | pairs generated from the project mathlib index |
| `static_external_comparison` | pairs from external indexes without current source-backed provenance |
| `source_backed_external_comparison` | pairs from external indexes with source-backed provenance |

Generation may be noisy. The noise must be measured by feature family, origin, and hard-negative survival. Final
visibility remains a later review-policy decision.

Ordinary audits still hydrate only selected external handles. Eval may request tracked declaration pairs by qualified
name so search can report whether labeled mathlib/external pairs were generated, without hydrating all of mathlib.

## Metrics

`metrics.stage_metrics.candidate_generation_recall` counts labeled positives known to the generated stage, including
tracked generated-only pairs that did not survive first-stage selection.

| Metric | Counts |
| --- | --- |
| `candidate_generation_recall` | labeled positives present at generation (including generated-only survivors) |
| `top_k_recall_before_final_ranking` | labeled positives surviving into ranked observations at each `k` |
| `ranked_recall` | the previous ranked-recall vocabulary, kept for compatibility |
| `visible_queue_precision` | shown true positives / shown pairs |
| `hard_negative_survival` | hard negatives at generated, top-k, and visible stages |
| `generated_candidate_count_by_policy` | generated observations by private policy label |
| `generated_candidate_count_by_feature_family` | generated observations by stable feature family |
| `hard_negative_generated_by_feature_family` | generated hard negatives by feature family |
| `retrieval.generated_candidate_count` | total generated |
| `retrieval.ranked_candidate_count` | total survived ranking |
| `retrieval.pruned_feature_fanouts` | fanouts dropped before ranking |

These names are stable observation vocabulary. They are not retrieval keys, posting-table names, or scorer features.

## Why a private stage plus observation DTOs

Exposing retrieval internals directly would make eval and reports depend on retrieval structs, semantic key strings,
heap pruning, and SQLite-shaped vocabulary. Every retrieval refactor would become a JSON/API migration.

A private generation stage plus stable observation facts means candidate-generation policy can change without teaching
eval or report how retrieval works. Search keeps feature planning, fanout checks, and first-stage selection private;
eval receives generated/ranked facts, policy labels, and feature-family diagnostics.

## Evidence commands

```sh
# Fast fixture evidence
cargo run -p lean-dup-cli -- eval --suite default --format json --write-search-dataset

# Production-gate evidence
cargo run -p lean-dup-cli -- eval --suite production-gate --format json \
  --output target/eval/prompt32-production-gate.json

# Leak check for dataset artifacts. Any match must be intentional stable
# vocabulary, not a leaked internal key.
rg -n 'sqlite|posting|IndexQuery|FeatureMatch|/Users/|statement_text|raw' \
  target/search-quality/default-dataset.json
```
