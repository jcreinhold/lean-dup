# Candidate Generation

Candidate generation is an explicit private search stage with named observability. The goal is high-recall visibility
into where positives are lost, without changing ranking thresholds, semantic-probe policy, report JSON, command names,
or ordinary mathlib hydration limits.

## Generation policies

Every generated candidate carries one of four diagnostic policy labels. They name the source, not the visibility
decision.

| Policy | Source |
| --- | --- |
| `local_duplicate_audit` | pairs generated within the audited workspace corpus |
| `mathlib_comparison` | pairs generated from the project mathlib index |
| `static_external_comparison` | pairs from external indexes without current source-backed provenance |
| `source_backed_external_comparison` | pairs from external indexes with source-backed provenance |
| `vector_local_duplicate_audit` | hidden vector pairs generated within the audited workspace corpus |
| `vector_mathlib_comparison` | hidden vector pairs generated from the project mathlib vector corpus |
| `vector_static_external_comparison` | hidden vector pairs from external vector corpora without current source-backed provenance |
| `vector_source_backed_external_comparison` | hidden vector pairs from external vector corpora with source-backed provenance |

Generation may be noisy. The noise must be measured by feature family, origin, and hard-negative survival. Final
visibility remains a later review-policy decision.

Ordinary audits still hydrate only selected external handles. Eval may request tracked declaration pairs by qualified
name so search can report whether labeled mathlib/external pairs were generated, without hydrating all of mathlib.

Hidden vector experiments are explicit exceptions for measurement. They may build or reuse a persisted
declaration-vector corpus and query it for nearest neighbors, but this work happens only when the hidden vector flag is
set. The default audit path remains symbolic and keeps the existing selected-candidate hydration limit.

## Vector eligibility and top-k policy

Search owns vector corpus and query eligibility. The default hidden policy is `actionable-public-statement`: it excludes
generated, private, synthetic, low-signal, missing-statement, non-actionable, and unsupported-kind declarations before
any embedding or vector-index work. A `broad` policy may include normally excluded declarations for a named experiment,
but artifacts must record that choice.

Eligibility is not document policy. Eligibility decides whether a declaration can be a query or corpus row. Document
policy decides which stable declaration facts are selected for embedding. Embedding still owns model-specific wrapping
such as query/document prefixes; vector-index still owns persistence and nearest-neighbor mechanics. Hidden vector
requests record a stable input-format id (`asymmetric-query-document` or `symmetric-document`) so validation can compare
role formatting without teaching search the model-specific strings or runtime choices behind that id.

## Semantic document policy

Search owns semantic document construction for hidden vector experiments. Lean worker and index expose stable
declaration facts: name, module, kind, statement/signature text, optional docstring text, optional definition body
summary, visibility/generated facts, and content hashes. Search selects text from those facts and sends plain text plus
an embedding role to `lean-dup-embedding`. Embedding must not learn Lean actionability, definition-body policy,
proof-body exclusions, retrieval keys, or worker/index row shape.

Design Note: this document owns the candidate-generation observation vocabulary and the search-owned semantic-document
policy. The smallest public interface is policy ids, policy versions, availability counters, content hashes, and stage
denominators. Backend names, model prefixes, tokenizer/runtime details, raw declaration text, final model input strings,
source snippets, worker rows, retrieval keys, and database vocabulary must not leak into eval/report artifacts. The
default symbolic audit remains unchanged. The discarded behavior is embedding a weak ad hoc string while preserving
policy names that claim unavailable informal content.

Design It Twice: keeping name plus statement is too weak for definitions; letting embedding assemble Lean-aware inputs
leaks duplicate-search policy into the model runtime crate; search-owned policies over worker/index declaration facts
are deeper because each volatile decision has one owner.

Current policies:

| Policy | Search-selected content |
| --- | --- |
| `statement` | statement/signature only |
| `name-and-statement` | declaration name plus statement/signature |
| `definition-aware` | name plus statement/signature plus definition body summary when available |
| `docstring-augmented` | docstring when available, then name plus statement/signature, plus definition body summary when available |

The worker/index boundary supplies real optional docstring and definition-summary facts. Search records content
availability counters in hidden vector facts. Theorem proof bodies are not embedded by default. A future proof-body
experiment must be a named non-default policy with its own privacy and quality checks.

Red Flag Review: the document policy surface is not a pass-through wrapper because it selects content, hashes it, and
records availability counters. It avoids temporal decomposition by assigning content facts to worker/index, policy to
search, and role wrapping to embedding. It avoids information leakage by keeping raw content and model formatting out of
serialized artifacts. The API is describable as policy, counters, hashes, and stage facts.

Hidden vector artifacts record:

| Fact | Meaning |
| --- | --- |
| `query_eligibility` | policy id/version, total query declarations, eligible query declarations, skip reasons |
| `corpus_eligibility` | policy id/version, total corpus declarations, eligible corpus declarations, skip reasons |
| `top_k` | search-owned nearest-neighbor request size |
| `eligible_corpus_size` | corpus size after eligibility filtering |
| `top_k_saturated` | `true` when `top_k >= eligible_corpus_size` |

Saturated `top_k` runs can prove plumbing, but they are not vector retrieval-quality evidence. A quality claim needs
non-saturated denominators, vector-only positives, and hard-negative survival.

## Metrics

`metrics.stage_metrics.candidate_generation_recall` counts labeled positives known to the generated stage, including
tracked generated-only pairs that did not survive first-stage selection.

| Metric | Counts |
| --- | --- |
| `candidate_generation_recall` | labeled positives present at generation (including generated-only survivors) |
| `candidate_stage_recall` | labeled positives at vector-generated, symbolic-generated, merged-generated, ranked, and visible stages |
| `top_k_recall_before_final_ranking` | labeled positives surviving into ranked observations at each `k` |
| `ranked_recall` | labeled positives that survive ranked observation at each configured cutoff |
| `visible_queue_precision` | shown true positives / shown pairs |
| `hard_negative_survival` | hard negatives at generated, top-k, and visible stages |
| `hard_negative_stage_survival` | hard negatives at vector-generated, symbolic-generated, merged-generated, ranked, and visible stages |
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

Vector candidate generation follows the same rule. Eval sees stable source facts and raw denominators; it does not call
embedding or vector-index directly to reconstruct search. Vector backend names, corpus storage layout, model runtime
details, and final model input strings remain private to their owning crates.

Design Note: this document owns the stable observation vocabulary for candidate generation. The smallest public
interface is named policy facts and stage denominators. Backend names, storage rows, model prefixes, raw statement text,
worker rows, and retrieval keys must not leak into eval/report JSON. The default symbolic audit capability is preserved;
the discarded behavior is unrecorded semantic/vector probing without explicit corpus/query eligibility or top-k
saturation facts.

Design It Twice: letting embedding/vector-index reject declarations would leak Lean actionability into lower crates;
letting eval filter after the observation would make eval reconstruct search. Search-owned named eligibility policies
are deeper because search already owns declaration meaning and candidate-generation policy.

Red Flag Review: the eligibility surface is not a pass-through wrapper because it records policy counts and skip
reasons, not rows; it avoids temporal decomposition by keeping filtering in search before runtime work; it avoids
information leakage by keeping backend and model vocabulary out of public artifacts; and its API is describable as
policy, counts, top-k, and saturation.

## Evidence commands

```sh
# Fast fixture evidence
cargo run -p lean-dup-cli -- eval --suite default --format json --write-search-dataset

# Production-gate evidence
cargo run -p lean-dup-cli -- eval --suite production-gate --format json \
  --output target/eval/production-gate.json
```

Dataset artifact privacy is verified by the leak-check rule in [search-datasets.md](search-datasets.md).
