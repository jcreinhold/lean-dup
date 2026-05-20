# Search Dataset Artifacts

Feature extraction is owned by search; scoring artifacts are owned by eval. The dataset records what the search
stack observed for each candidate pair, without changing retrieval, ranking, semantic-probe policy, report JSON, or
eval scoring.

Search translates private retrieval and declaration facts into stable feature DTOs. Eval joins those DTOs to typed
labels and writes deterministic artifacts. A reconstruction-from-retrieval design was rejected: it would have forced
eval to learn retrieval contributions, key families, structural fingerprints, and blocker policy: the same leakage
the crate split removed.

## Hidden command

```sh
cargo run -p lean-dup-cli -- eval --suite default --format json --write-search-dataset
# writes target/search-quality/<suite>-dataset.json
```

## Artifact shape

Schema version: `lean-dup.search-dataset.v1`.

Top-level: `schema_version`, `suite`, `pairs`. Each pair row:

| Field | Contents |
| --- | --- |
| `left`, `right` | normalized declaration names |
| `label_status` | `positive | hard-negative | unlabeled` |
| `label` | typed label metadata when adjudicated |
| `stage_position` | generated? survived ranking? rank when ranked |
| `final_visibility` | current shown-queue facts |
| `features` | search-owned stable feature facts (table below) |

Rows are sorted by `(left, right, rank)`; generated-only rows carry no rank. Unlabeled retrieved candidates remain
in the artifact so consumers can inspect false-positive and background distributions, not only the gold pairs.

## Feature facts

Coarser than retrieval internals, by design.

| Feature | Values |
| --- | --- |
| `retrieval_feature_families` | `statement_fingerprint`, `safe_permutation_fingerprint`, `connective_fingerprint`, `conclusion_fingerprint`, `role_conclusion_const`, `role_hypothesis_const`, `role_head`, `role_other`, `other`, `unknown` |
| `declaration_kinds` | declaration kind labels |
| `evidence_mode` | `local`, `source-backed`, `static` |
| `structural_fingerprint_families` | fingerprint families that match (no fingerprint values) |
| `role_overlap` | counts by role-feature family (no role keys) |
| `module_relation` | module names only |
| `semantic_evidence_state` | currently `not-run` for retrieval-only eval observations |
| `cheap_blockers` | `generated`, `non-public`, `low-signal`, `role-head-only-evidence`, … |

The dataset must avoid raw keys and source payloads even as `semantic_evidence_state` or scorer-config consumers
grow richer.

## Privacy

Allowed in dataset artifacts: stable family names, typed counts, evidence modes, module
names, label provenance.

Forbidden: absolute private paths, raw Lean expressions, raw statement text, source
snippets, raw retrieval keys, SQLite row ids or table names or posting records, worker
JSONL rows, transport diagnostics.

If a future feature looks like it needs one of the forbidden items, the feature boundary
is wrong. Add a stable family, count, mode, or relation instead.

## Verification

```sh
cargo run -p lean-dup-cli -- eval --suite default --format json --write-search-dataset
test -f target/search-quality/default-dataset.json

# leak check: any match must be intentional stable vocabulary
rg -n 'sqlite|posting|IndexQuery|FeatureMatch|/Users/|statement_text|raw' \
  target/search-quality/default-dataset.json
```
