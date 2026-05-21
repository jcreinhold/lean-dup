# High-Recall Fanout Policy

Date: 2026-05-21

This document records the Prompt 70 fanout and top-k repair. The change does not raise default budgets. It makes the
current bounded retrieval policy named, observable, and label-aware so future recall work can say exactly whether a
labeled pair was lost to fanout pruning, per-anchor top-k, ranking, or visibility.

## Design Note

Fanout policy owns posting limits, broad-head pruning, and per-source top-k selection. Search owns candidate-source
merge policy and emits stable pruning/saturation facts. Eval owns label joins and denominators. Report owns aggregate
projection. Performance evidence stays attached to named workloads rather than to intuition about constants.

The smallest public interface is a leak-safe policy summary plus candidate facts and loss facts: policy id, source id,
source family, top-k saturation, feature family, loss stage, reason, and count. Callers do not receive retrieval keys,
posting layout, raw feature keys, scorer weights, worker rows, private paths, cache layout, or vector facts.

The preserved user-facing capability is the conservative symbolic audit queue. The Python-era behavior discarded is
treating pruning constants as invisible implementation accidents and asking users to infer recall loss from missing
pairs.

## Design It Twice

Three designs were considered.

1. Raise per-anchor and role posting constants until recall improves. Rejected: this spends runtime and memory before
   proving which labels are actually cap-lost.
2. Remove broad-head pruning and rely on ranking to clean up noise. Rejected: Prompt 67 already showed large KanProofs
   candidate volume, and unbounded broad fanout would obscure both performance and precision failures.
3. Make fanout/top-k policy explicit, measured, label-aware in eval, and bounded by release targets. Chosen: retrieval
   keeps its mechanics private while eval can locate recall loss without learning retrieval keys or posting layout.

## Active Policy

The current policy id is `lean-dup.fanout-policy.v1`.

| Setting | Value |
| --- | ---: |
| symbolic per-anchor top-k | 80 |
| Lean semantic lane per-anchor top-k | 24 |
| role-feature posting limit | 512 |
| broad-head posting limit | 64 |

Search now projects this policy through `SearchRetrievalObservation.fanout_policy` and ordinary audit report
`retrieval.fanout_policy_id`. It also records:

- `top_k_saturation_by_source_id`, counted by source id;
- `pruned_feature_fanout_by_family`, summed by stable feature family;
- per-pair `SearchCandidateLossFact` rows for tracked labels lost to fanout pruning.

Eval derives top-k losses from generated-but-unranked tracked pairs and reports all loss denominators under
`metrics.stage_metrics.candidate_loss_metrics`.

## Prompt 67 Baseline

Prompt 67 recorded the KanProofs private audit before this repair:

| Fact | Value |
| --- | ---: |
| retrieval candidates | 558,109 |
| review candidate pairs | 13,581 |
| visible groups with `--private` | 5 |
| pruned feature fanouts | 39,387 |
| per-anchor heap truncations | 6,522 |

Those numbers are now observable as bounded-policy facts instead of opaque counters. The current manual label corpus was
still not sufficient to decide whether those KanProofs fanout prunes are label-affecting loss, so this session added a
focused fixture proving the loss-accounting path.

## Focused Fixture

`crates/search/src/observation.rs` includes a role-fanout fixture with 520 declarations sharing one role feature and
unique statement fingerprints. The tracked positive shares only that overwide role feature. Search records no generated
pair and emits one fanout-pruned candidate-loss fact:

| Field | Value |
| --- | --- |
| source id | `symbolic-retrieval` |
| policy | `local_duplicate_audit` |
| reason | `overwide-posting` |
| feature family | `role_conclusion_const` |
| count | 520 |

The fixture proves eval can distinguish "not generated because the pair has no symbolic evidence" from "not generated
because bounded fanout policy pruned the only shared feature family."

## Fast Evidence

Commands run after the repair:

```sh
cargo run -p lean-dup-cli -- eval --suite default --format json --output target/fanout-policy/default.json
cargo run -p lean-dup-cli -- eval --suite hard-negatives --format json --output target/fanout-policy/hard-negatives.json
```

Default suite:

| Metric | Value |
| --- | ---: |
| candidate generation recall | 16 / 16 |
| visible precision | 8 / 8 |
| hard-negative visible hits | 0 / 3 |
| positive fanout-pruned | 0 / 16 |
| hard-negative fanout-pruned | 0 / 3 |
| positive top-k dropped | 0 / 16 |
| hard-negative top-k dropped | 0 / 3 |
| candidate count | 299 |
| retrieval time | 10 ms |
| RSS status | 12,877,824 bytes |

Hard-negative suite:

| Metric | Value |
| --- | ---: |
| candidate generation recall | 1 / 1 |
| visible precision | 1 / 8 |
| hard-negative visible hits | 0 / 5 |
| positive fanout-pruned | 0 / 1 |
| hard-negative fanout-pruned | 0 / 5 |
| positive top-k dropped | 0 / 1 |
| hard-negative top-k dropped | 0 / 5 |
| candidate count | 299 |
| retrieval time | 9 ms |
| RSS status | 12,976,128 bytes |

No default cap was raised from fixture-only evidence. The fast suites show that existing labeled fixture positives and
hard negatives are not currently lost to fanout or top-k policy.

## KanProofs Interpretation

The Prompt 67 KanProofs `39,387` fanout prunes and `6,522` heap truncations remain expected bounded behavior unless a
current resolved manual label is traced to one of those losses. Prompt 65 rebuilt the manual corpus conservatively;
future KanProofs release claims must rerun the manual suites and use `candidate_loss_metrics` to distinguish:

- expected high-fanout pruning with no labeled loss;
- positive recall loss due to `fanout-pruned`;
- positive recall loss due to generated-but-unranked top-k drops;
- hard-negative survival introduced by any larger budget.

## Red Flag Review

- Shallow module: the change records policy summaries and label-affecting losses, not just raw counters.
- Pass-through wrapper: eval receives stable loss facts and derives top-k loss denominators; it does not forward
  retrieval rows.
- Temporal decomposition: the split follows ownership: retrieval policy, search facts, eval denominators, report
  projection.
- Information leakage: retrieval keys, posting names, raw feature keys, scorer internals, worker rows, private paths,
  cache layout, backend vocabulary, and vector facts stay out of artifacts.
- Special-general mixture: the policy is concrete and versioned; the loss fact vocabulary is general enough for current
  symbolic and Lean semantic sources.
- Conjoined methods: pruning, ranking, label scoring, and reporting remain separately owned.
- Hard-to-describe public API: a candidate loss fact is one stable reason a tracked pair did not reach generation or
  top-k.
- Implementation-detail comments: public comments describe pruning and saturation facts, not posting-table shape or
  feature-key encodings.
