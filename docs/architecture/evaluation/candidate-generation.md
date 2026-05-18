# Candidate Generation

Prompt 32 makes candidate generation an explicit private search stage. The goal is high-recall observability without
changing ranking thresholds, semantic-probe policy, report JSON, command names, or ordinary mathlib hydration limits.

## Design Note

The candidate-generation boundary owns semantic feature planning, fanout policy, source/provenance-specific generation
policy, generated candidate counts, broad-feature pruning diagnostics, and tracked-pair generation facts.

Its smallest public interface is additive search/eval observation data: generated counts, ranked counts, generation
policy labels, feature-family diagnostics, and generated-stage facts for labeled pairs. Callers do not learn retrieval
keys, SQLite tables, storage postings, heap selection mechanics, raw Lean expressions, source text, or worker transport.

These design decisions must not leak upward or sideways:

- semantic feature key construction and fanout thresholds;
- the order in which generation contributions are accumulated;
- heap selection limits and ranking weights;
- mathlib hydration strategy and named-declaration lookup details;
- source-backed versus static provenance detection mechanics.

The validated user-facing capability preserved is read-only local duplicate auditing with bounded candidate volume,
cached indexes, optional mathlib comparison, semantic evidence, reports, `show`, `diff`, eval suites, and hidden dataset
artifacts.

Python-era behavior intentionally discarded: treating the bounded ranked queue as the only evidence that a pair was
retrieved, tuning from terminal inspection, and hiding positive loss points behind aggregate recall numbers.

## Design It Twice

**Rejected: expose retrieval internals as candidate-generation output.** That would make eval and reports depend on
retrieval structs, semantic key strings, heap pruning, and SQLite-shaped vocabulary. It would be easy to debug once but
would turn every retrieval refactor into a JSON/API migration.

**Chosen: private generation stage plus observation DTO facts.** Search keeps feature planning, fanout checks, and
first-stage selection private. Eval receives stable generated/ranked facts, generation policy labels, and feature-family
diagnostics. This is deeper because candidate-generation policy can change without teaching eval or report how retrieval
works.

## Policy Contract

Candidate generation has four private policy labels:

- `local_duplicate_audit`: pairs generated within the audited workspace corpus;
- `mathlib_comparison`: pairs generated from the project mathlib index;
- `static_external_comparison`: pairs generated from external indexes without current source-backed proof context;
- `source_backed_external_comparison`: pairs generated from external indexes with source-backed provenance.

The policy labels are diagnostic facts, not ranking thresholds. Candidate generation may be noisy, but the noise must be
measured by feature family, origin, and hard-negative survival. Final visibility remains a later review-policy decision.

Ordinary audits still hydrate only selected external handles. Eval may request tracked declaration pairs by qualified
name so search can report whether labeled mathlib/external pairs were generated without hydrating all of mathlib.

## Metric Contract

Prompt 32 changes the meaning of `metrics.stage_metrics.candidate_generation_recall`: it now counts labeled positives
known to the generated stage, including tracked generated-only pairs that did not survive first-stage selection.

The top-k and visible metrics keep their previous meanings:

- `top_k_recall_before_final_ranking`: labeled positives that survived into ranked observations at each `k`;
- `ranked_recall`: backward-compatible ranked recall vocabulary;
- `visible_queue_precision`: shown true positives over shown pairs;
- `hard_negative_survival`: hard negatives at generated, top-k, and visible stages.

Additional additive diagnostics:

- `generated_candidate_count_by_policy`;
- `generated_candidate_count_by_feature_family`;
- `hard_negative_generated_by_feature_family`;
- `retrieval.generated_candidate_count`;
- `retrieval.ranked_candidate_count`;
- `retrieval.pruned_feature_fanouts`.

These names are stable observation vocabulary. They are not retrieval keys, posting-table names, or scorer features.

## Evidence Commands

Fast fixture evidence:

```sh
cargo run -p lean-dup-cli -- eval --suite default --format json --write-search-dataset
```

Production-gate evidence:

```sh
cargo run -p lean-dup-cli -- eval --suite production-gate --format json \
  --output target/eval/prompt32-production-gate.json
```

Leak check for dataset artifacts:

```sh
rg -n 'sqlite|posting|IndexQuery|FeatureMatch|/Users/|statement_text|raw' \
  target/search-quality/default-dataset.json
```

Any remaining match must be intentional stable vocabulary, not leaked internals.

## Red Flag Review

- **Shallow module:** mitigated. Search now hides generation policy and fanout handling behind workflow observations
  instead of exposing retrieval structs.
- **Pass-through wrapper:** avoided. The boundary adds generated/ranked distinction, policy counts, pruning summaries,
  and tracked-pair facts.
- **Temporal decomposition:** mitigated. The public observation is organized around stage facts, not "call retrieval,
  then rank" sequencing.
- **Information leakage:** mitigated. DTOs expose feature families and policy labels, not SQLite, postings, raw keys,
  worker rows, or heap mechanics.
- **Special-general mixture:** contained. KanProofs/mathlib labels use the same tracked-pair mechanism as fixtures.
- **Conjoined methods:** mitigated. Candidate generation owns fanout and generation evidence; eval owns denominators and
  label joins.
- **Hard-to-describe public API:** mitigated. The normal audit API is unchanged; the additive observation fields have
  one job.
- **Implementation details contaminating interface comments:** mitigated. Comments describe generated/ranked facts and
  bounded hydration promises, not SQL or transport internals.
