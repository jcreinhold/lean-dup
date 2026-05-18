# Search Labels And Adjudication Corpus

This document defines the typed label contract introduced by Prompt 29. It extends the production-gate label files from
plain positive and hard-negative pairs into task-specific adjudications that later search-quality prompts can measure by
match class and expected stage behavior.

## Design Note

The label/adjudication boundary owns label taxonomy, adjudication provenance, confidence, expected stage visibility,
semantic-evidence requirements, static-evidence allowance, legacy migration, and skipped class accounting.

Its smallest public interface is still the named eval suites and their JSON/table metrics. Internally, the label loader
normalizes typed adjudications into unordered positive and hard-negative pairs for the current scorer while preserving
typed metadata for Prompt 30 and later stage-level metrics.

These decisions must not leak upward or sideways:

- JSON label-file layout, legacy cluster expansion, and typed-pair migration mechanics;
- private KanProofs path policy and manual-suite execution policy;
- retrieval keys, ranking thresholds, cache layout, SQLite tables, source scanning, probe chunking, or worker JSONL;
- fixture construction details and temporary Prompt 29 migration choices.

The preserved user-facing capability is measurable read-only duplicate audit quality. Existing `eval --suite default`
and `eval --suite hard-negatives` commands continue to report raw recall, precision, hard-negative leakage, runtime, and
memory counts without requiring users to know the label storage format.

Python-era behavior intentionally discarded:

- treating Python ranking behavior as a compatibility target;
- using anecdotal inspection as label evidence;
- collapsing exact duplicates, replacement candidates, weak related theorems, and hard negatives into one undifferentiated
  duplicate bucket;
- accepting labels with no provenance or confidence.

## Design It Twice

**Rejected: replace all labels with one new typed-only file format.** That would force a broad migration before the eval
gates could keep running, and it would turn Prompt 29 into a scoring or suite rewrite. The current scorer only needs
positive and hard-negative sets, so removing legacy clusters now would add risk without improving the immediate quality
oracle.

**Chosen: typed adjudication layer around legacy labels.** The loader accepts legacy clusters and direct pairs, then
adds typed `typed_pairs` with match class, expected visibility, source, confidence, and evidence requirements. This is
deeper because label-file compatibility, contradiction handling, and task taxonomy stay inside `eval::labels`; scoring
continues to see normalized unordered pairs, while later prompts can consume typed metadata without rediscovering label
provenance.

## Label Schema Contract

Legacy fields remain supported during migration:

- `positive_clusters`
- `positive_pairs`
- `hard_negative_clusters`
- `hard_negative_pairs`

Typed labels use `typed_pairs`:

```json
{
  "left": "Tiny.same_left",
  "right": "Tiny.same_right",
  "polarity": "positive",
  "match_class": "exact-theorem-duplicate",
  "expected_stage_visibility": "visible",
  "adjudication_source": "fixture-intent",
  "confidence": "high",
  "semantic_verification_required": true,
  "static_evidence_acceptable": true
}
```

The supported `polarity` values are `positive` and `hard-negative`.

The supported `match_class` values mirror
[search-quality.md](/Users/jcreinhold/Code/lean-dup/docs/architecture/search-quality.md):

- `exact-theorem-duplicate`
- `binder-permutation-duplicate`
- `reducible-definition-duplicate`
- `replacement-candidate`
- `specialization-generalization`
- `local-cleanup-duplicate`
- `static-structural-similarity`
- `non-actionable-related-theorem`
- `hard-negative`

The supported `expected_stage_visibility` values are:

- `candidate`: the pair should be generated but need not rank highly yet;
- `ranked`: the pair should survive first-stage ranking;
- `visible`: the pair should appear in the default visible queue when the suite reaches the relevant stage;
- `hidden`: the pair is intentionally not default-visible.

The supported `adjudication_source` values are:

- `fixture-intent`: the fixture was written to encode this relation;
- `manual-inspection`: a human inspected the declarations;
- `prompt27-evidence`: the label preserves the Prompt 27 production-gate evidence;
- `python-era-regression`: retained historical evidence from the retired implementation.

The supported `confidence` values are `high`, `medium`, and `low`. New production-gate labels should not use `low`
unless the architecture document records why the uncertain label is still useful.

## Validation Rules

The loader rejects typed labels that omit any required typed field. A typed label must state polarity, match class,
expected visibility, adjudication source, confidence, semantic-verification requirement, and static-evidence allowance.

The loader rejects contradictory typed labels for the same unordered declaration pair. A duplicate typed label is valid
only if polarity and match class agree.

Legacy positive and hard-negative contradictions keep the pre-Prompt-29 compatibility behavior: if a legacy pair is both
positive and hard-negative, the hard-negative entry is dropped after pair normalization. Typed labels do not get that
escape hatch because typed adjudications are the production-quality source of truth.

Label identity is direction-insensitive. `A/B` and `B/A` describe the same pair for scoring and validation.

## Fixture Coverage

Prompt 29 represents every current core match class using existing fixture declarations, so `skipped_classes[]` is
empty.

```json
"skipped_classes": []
```

Current fixture typed coverage:

| Match class | Fixture pair |
| --- | --- |
| exact theorem duplicate | `Tiny.same_left` / `Tiny.same_right` |
| binder/permutation duplicate | `Tiny.reordered_left` / `Tiny.reordered_right` |
| reducible-definition duplicate | `Tiny.probe_small_def_left` / `Tiny.probe_small_def_right` |
| replacement candidate | `Tiny.same_left` / `External.same_as_tiny` |
| specialization/generalization | `Tiny.specialization_general` / `Tiny.specialization_specific` |
| local cleanup duplicate | `Tiny.clone_one` / `Tiny.clone_two` |
| static structural similarity | `Tiny.connective_and_left` / `Tiny.connective_and_right` |
| non-actionable related theorem | `Tiny.related_left` / `Tiny.related_right` |
| hard negative | `Tiny.same_conclusion_nat_domain` / `Tiny.same_conclusion_bool_domain`; `Tiny.broad_eq_only` / `Tiny.symmetric_eq_left` |

If a future class cannot be represented in one session, this document must add a `skipped_classes[]` entry with the
class, reason, and exact future fixture requirement before the prompt can finish.

## KanProofs Labels

The KanProofs suites remain manual and private-path aware. Their typed labels preserve Prompt 27 evidence instead of
claiming that current retrieval or ranking is good.

Prompt 27 evidence recorded in
[production-gates.md](/Users/jcreinhold/Code/lean-dup/docs/architecture/evaluation/production-gates.md):

- KanProofs/mathlib recall@10: `0/11`;
- KanProofs/mathlib hard-negative leakage: `3/4`;
- KanProofs internal recall@10: `0/6`;
- production-gate aggregate recall@10: `15/32`.

The labels in `kanproofs-internal.json` and `kanproofs-mathlib.json` are therefore adjudication targets, not a claim
that the current search stack finds them.

## Evidence Commands

Compatibility and fast-suite evidence:

```sh
cargo run -p lean-dup-cli -- eval --suite default --format json \
  --output target/eval/default.json

cargo run -p lean-dup-cli -- eval --suite hard-negatives --format json \
  --output target/eval/hard-negatives.json
```

Manual production evidence:

```sh
cargo run -p lean-dup-cli -- eval --suite production-gate --format json \
  --output target/eval/production-gate.json
```

Prompt 29 does not tune retrieval, ranking, probe planning, or thresholds. Any quality failure after this prompt is
evidence for Prompts 30 through 37, not a reason to weaken labels.

## Red Flag Review

- **Shallow module:** mitigated. The label loader hides legacy migration, typed validation, and normalized scoring pairs
  behind one eval boundary.
- **Pass-through wrapper:** avoided. Typed labels add adjudication metadata and validation; they are not aliases for
  existing clusters.
- **Temporal decomposition:** mitigated. The schema is organized by label meaning and expected stage behavior, not by the
  order in which retrieval, ranking, and probes currently run.
- **Information leakage:** mitigated. Label files do not expose retrieval keys, SQLite rows, worker transport, or probe
  chunks.
- **Special-general mixture:** contained. Fixture and KanProofs labels share one schema, while private KanProofs suite
  execution remains owned by suite orchestration.
- **Conjoined methods:** mitigated. Scoring consumes normalized pairs; typed metadata is preserved for later stage
  metrics without forcing scorer changes.
- **Hard-to-describe public API:** mitigated. Users still run named eval suites. The richer schema is internal evidence
  for production gates.
- **Implementation details contaminating interface comments:** mitigated. Interface comments describe label contracts
  and caller-visible meaning, not JSON parsing steps or storage details.
