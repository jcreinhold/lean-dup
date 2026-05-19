# Search Labels And Adjudication Corpus

Production-gate labels carry typed adjudications: match class, expected stage visibility, source, confidence, and
evidence requirements. The current scorer still sees normalized unordered pairs; stage-level metrics consume the
typed metadata.

## Label schema

Legacy fields stay supported during migration:

- `positive_clusters`, `positive_pairs`
- `hard_negative_clusters`, `hard_negative_pairs`

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

### Enumerations

| Field | Values |
| --- | --- |
| `polarity` | `positive`, `hard-negative` |
| `match_class` | mirrors [search-quality.md](../search-quality.md): `exact-theorem-duplicate`, `binder-permutation-duplicate`, `reducible-definition-duplicate`, `replacement-candidate`, `specialization-generalization`, `local-cleanup-duplicate`, `static-structural-similarity`, `non-actionable-related-theorem`, `hard-negative` |
| `expected_stage_visibility` | `candidate` (generated, not necessarily high-ranked), `ranked` (survives first-stage), `visible` (default queue), `hidden` (intentionally not default-visible) |
| `adjudication_source` | `fixture-intent`, `manual-inspection`, `prompt27-evidence`, `python-era-regression` |
| `confidence` | `high`, `medium`, `low` (only with documented reason) |

## Validation

- Typed labels must include every typed field above; partial typed labels are rejected.
- Typed labels for the same unordered pair must agree on polarity and match class.
- Legacy positive/hard-negative contradictions keep the pre-Prompt-29 behavior: the hard-negative entry is dropped
  after pair normalization. Typed labels do not get that escape hatch.
- Label identity is direction-insensitive: `A/B` and `B/A` are the same pair.

## Fixture coverage

Every current core match class is represented; `skipped_classes` is `[]`.

| Match class | Fixture pair |
| --- | --- |
| exact theorem duplicate | `Tiny.same_left` ↔ `Tiny.same_right` |
| binder/permutation duplicate | `Tiny.reordered_left` ↔ `Tiny.reordered_right` |
| reducible-definition duplicate | `Tiny.probe_small_def_left` ↔ `Tiny.probe_small_def_right` |
| replacement candidate | `Tiny.same_left` ↔ `External.same_as_tiny` |
| specialization/generalization | `Tiny.specialization_general` ↔ `Tiny.specialization_specific` |
| local cleanup duplicate | `Tiny.clone_one` ↔ `Tiny.clone_two` |
| static structural similarity | `Tiny.connective_and_left` ↔ `Tiny.connective_and_right` |
| non-actionable related theorem | `Tiny.related_left` ↔ `Tiny.related_right` |
| hard negative | `Tiny.same_conclusion_nat_domain` ↔ `Tiny.same_conclusion_bool_domain`; `Tiny.broad_eq_only` ↔ `Tiny.symmetric_eq_left` |

## KanProofs labels

KanProofs suites stay manual and private-path aware. Their typed labels are adjudication targets, not a claim that
the current search stack finds them. Files: `kanproofs-internal.json`, `kanproofs-mathlib.json`.

Current evidence (full table in [production-gates.md](production-gates.md)):

| Suite | Recall@10 | Hard-negative leakage |
| --- | ---: | ---: |
| KanProofs/mathlib | 0/11 | 3/4 |
| KanProofs internal | 0/6 | — |
| production-gate aggregate | 15/32 | 3/16 |

## Why a typed layer instead of a new format

A full migration to a typed-only file format would force broad rewrites before the eval gates could keep running. The
current scorer only needs positive and hard-negative sets, so removing legacy clusters now adds risk without
improving the oracle. The typed layer wraps the legacy files: `eval::labels` accepts clusters and direct pairs, then
adds typed `typed_pairs` with the schema above. Compatibility, contradiction handling, and task taxonomy stay inside
`eval::labels`; scoring sees normalized unordered pairs; consumers read typed metadata without rediscovering label
provenance.

## Commands

```sh
cargo run -p lean-dup-cli -- eval --suite default --format json \
  --output target/eval/default.json
cargo run -p lean-dup-cli -- eval --suite hard-negatives --format json \
  --output target/eval/hard-negatives.json
cargo run -p lean-dup-cli -- eval --suite production-gate --format json \
  --output target/eval/production-gate.json
```
