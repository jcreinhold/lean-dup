# Search Labels And Adjudication Corpus

Production-gate labels carry typed adjudications: match class, expected stage visibility, source, confidence, and
evidence requirements. The current scorer still sees normalized unordered pairs; stage-level metrics consume the
typed metadata.

## Label schema

Labels are typed at the file boundary. The eval parser rejects unknown fields so
old untyped label keys cannot silently change scoring denominators.

Direct labels use `typed_pairs`:

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

Same-class groups use `typed_clusters`; eval owns expanding the members into
unordered typed pair facts:

```json
{
  "id": "exact-internal-aliases",
  "members": ["Tiny.same_left", "Tiny.use_same_left", "Tiny.same_right"],
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
| `adjudication_source` | `fixture-intent`, `manual-inspection`, `prompt27-evidence` |
| `confidence` | `high`, `medium`, `low` (only with documented reason) |

## Validation

- Typed labels and typed clusters must include every typed field above; partial typed labels are rejected.
- Typed labels for the same unordered pair must be identical. Contradictions are fixture errors.
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

## Manual labels

The manual suites stay slow and private-path aware. Their typed labels are adjudication targets,
not a claim that the current search stack finds them. Files: `manual-internal.json`,
`manual-mathlib.json`.

## Why typed clusters stay inside eval

Fixture authors may group declarations when every expanded pair has the same
adjudication. Eval expands those groups into normalized unordered pairs before
scoring. Search, report, and vector artifacts consume pair facts and do not need
to know label-file layout.

## Commands

```sh
cargo run -p lean-dup-cli -- eval --suite default --format json \
  --output target/eval/default.json
cargo run -p lean-dup-cli -- eval --suite hard-negatives --format json \
  --output target/eval/hard-negatives.json
cargo run -p lean-dup-cli -- eval --suite production-gate --format json \
  --output target/eval/production-gate.json
```
