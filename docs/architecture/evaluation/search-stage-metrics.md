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
| `candidate_stage_recall` | labeled positives at vector-generated, symbolic-generated, merged-generated, ranked, and visible stages |
| `top_k_recall_before_final_ranking` | recall at requested `k` over the current retrieval ordering |
| `ranked_recall` | current public recall metric, repeated under stage vocabulary |
| `visible_queue_precision` | shown true positives / shown candidates |
| `hard_negative_survival` | hard negatives at generated, top-k, and visible stages |
| `hard_negative_stage_survival` | hard negatives at vector-generated, symbolic-generated, merged-generated, ranked, and visible stages |
| `candidate_count_by_origin` | candidates grouped by `workspace`, `mathlib`, `external:<label>`, … |
| `candidate_count_by_feature_family` | candidates grouped by stable retrieval-evidence family |
| `generated_candidate_count_by_policy` | generated observations grouped by private search policy label |
| `generated_candidate_count_by_feature_family` | generated observations grouped by feature family |
| `hard_negative_generated_by_feature_family` | generated hard negatives grouped by feature family |
| `semantic_verification` | planned, cached, worker, and unavailable probe counts (zeros for retrieval-only suites) |

Generated observations are tracked separately from the bounded ranked queue. Candidate generation means "the pair
was created by the private generation stage"; top-k and visible metrics describe later survival.

For hidden vector experiments, candidate generation has two sources. `symbolic_generated` is the existing symbolic
retrieval stage. `vector_generated` is nearest-neighbor vector search over a persisted declaration-vector corpus.
`merged_generated` is the union after search-owned merge policy. Vector-only candidates may be ranked for measurement,
but they are not shown merely because a vector score exists.

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
vector_similarity
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
denominators, especially manual-corpus mathlib recall and hard-negative survival.

## Known limitations

- Stage metrics measure existing behavior. They reveal bad behavior; they do not fix it.
- The semantic-verification counters are zero for retrieval-only suites; the slots exist so
  artifact shape stays stable when audit-backed observations are added.
- Top-k recall before final ranking uses the current first-stage selection order.
- Hidden vector-search metrics measure candidate generation over the observed corpus and
  do not by themselves justify changing default visibility thresholds.

## 35N Vector Artifact Truth

The vector-search artifact is an eval-owned truth summary, not a replay log of generation events. Search reports stable
stage facts. Eval joins those facts with labels, deduplicates unordered declaration pairs, and writes one row per pair
with the best facts observed across symbolic, vector, merged, ranked, and visible stages.

Design Note:

- Hidden knowledge: eval owns label expansion, conflict resolution, row deduplication, and denominators; search owns
  stage facts; embedding and vector-index own runtime and persistence mechanics.
- Smallest public interface: vector artifacts expose stable row truth, label facts, raw count denominators, eligibility
  summaries, document policy facts, and model/profile summaries.
- Non-leaking decisions: raw statements, source snippets, final model inputs, retrieval keys, worker rows, backend
  names, storage vocabulary, paths, and model prefixes stay out of artifacts.
- Preserved capability: default symbolic audit and ordinary eval remain unchanged and embedding-free.
- Discarded behavior: treating duplicate pair events, unlabeled expanded clusters, and hidden top-k saturation as
  usable validation evidence.

Design It Twice:

- *One row per generation event.* Rejected: downstream readers would have to repair duplicate unordered pairs and
  resolve conflicting facts.
- *Typed labels only.* Rejected: legacy cluster expansion is still part of the scoring oracle, so omitting it makes
  expanded positives and hard negatives look unlabeled.
- *Eval-owned truth-preserving builder.* Chosen: search supplies stage facts, labels supply oracle facts, and eval
  produces a stable artifact with raw denominators.

Additional vector artifact metrics:

| Metric | What it counts |
| --- | --- |
| `vector_top_k_recall` | labeled positives returned by vector top-k / all positives |
| `vector_top_k_precision` | vector-generated positives / vector-generated pairs |
| `top_k_saturation` | saturated vector queries / vector queries |
| `vector_only_positives` | positives generated by vector but not symbolic |
| `vector_only_hard_negatives` | hard negatives generated by vector but not symbolic |
| `symbolic_only_positives` | positives generated by symbolic but not vector |
| `symbolic_only_hard_negatives` | hard negatives generated by symbolic but not vector |
| `merged_generated_recall` | positives present after symbolic/vector merge |
| `ranked_recall` | positives present in ranked observations |
| `visible_precision` | visible positives / visible pairs |
| `visible_hard_negative_count` | visible hard negatives / all hard negatives |

Red Flag Review:

- Shallow module: eval does real label joining, pair deduplication, and denominator construction rather than forwarding
  search rows.
- Pass-through wrapper: the artifact builder changes shape from event facts to unordered-pair truth.
- Temporal decomposition: artifact truth is defined by ownership of facts, not by the order search produced events.
- Information leakage: private model, backend, storage, source, and retrieval details are forbidden in artifacts.
- Special-general mixture: vector-specific metrics live in the hidden vector artifact; ordinary eval metrics stay
  stable.
- Conjoined methods: search generation, label parsing, and artifact rendering remain separate.
- Hard-to-describe API: the artifact contract is one row per unordered pair plus raw stage denominators.
- Implementation-detail comments: comments describe artifact and metric contracts, not storage or runtime mechanics.

## 35O Vector Scorer Variants

Hidden vector artifacts now measure ranking separately from candidate generation. Search reports vector-generated,
symbolic-generated, merged-generated, ranked, and visible stages as before, but hidden artifacts also include scorer
variant metrics for `symbolic-only`, `vector-evidence-only`, and `symbolic-plus-vector`.

Design Note:

- Hidden knowledge: search owns the conversion from nearest-neighbor facts to scorer features; eval owns denominators
  and variant artifact rows.
- Smallest public interface: scorer variant id, vector evidence feature version, and raw `found/total` stage metrics.
- Non-leaking decisions: raw backend distance semantics, model-specific normalization, tokenizer details, model input
  prefixes, vector-cache filenames, and database storage vocabulary do not appear in metrics.
- Preserved capability: ordinary eval still reports symbolic metrics and does not request embeddings or vector indexes.
- Discarded behavior: treating vector score as metadata that cannot affect ranking while asking validation whether
  vector ranking helps.

Design It Twice:

- *Eval re-ranks from vector scores.* Rejected because it would create an eval-only ranking pipeline.
- *Artifacts expose raw vector distances.* Rejected because it would couple metric interpretation to backend/model
  mechanics.
- *Search-owned vector evidence.* Chosen because scoring already belongs to search and eval can measure variants
  without reconstructing private search policy.

Variant metrics do not replace candidate-generation denominators. `vector_top_k_recall` answers whether vector search
found a labeled pair. Variant `ranked_recall` and `visible_precision` answer whether a scorer placed generated pairs
above hidden ranking and visibility thresholds. A vector-generated pair that stays invisible under `symbolic-only` but
becomes visible under `symbolic-plus-vector` is ranking evidence, not candidate-generation evidence.

Red Flag Review:

- Shallow module: the metric surface records scorer behavior, not command success.
- Pass-through wrapper: search owns feature conversion; eval records the result.
- Temporal decomposition: the split follows ownership of scoring and denominators, not execution order.
- Information leakage: metrics carry stable variant ids and feature versions, not backend/model details.
- Special-general mixture: hidden vector scorer variants stay out of ordinary eval metrics.
- Conjoined methods: generation, scoring, and validation remain separate surfaces.
- Hard-to-describe API: variant id plus raw denominators.
- Implementation-detail comments: comments describe metric meaning, not vector storage or runtime mechanics.
