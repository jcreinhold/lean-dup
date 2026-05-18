# Search Dataset Artifacts

Prompt 31 separates feature extraction from scoring. The dataset path records what the current search stack observed
for candidate pairs, but it does not change retrieval, ranking, semantic-probe policy, report JSON, or eval scoring.

## Design Note

The pair-feature boundary owns search-quality feature facts for candidate pairs: stable retrieval feature families,
declaration kinds, evidence mode, structural-fingerprint family matches, role-overlap counts, module relation,
semantic-evidence state, cheap blockers, label joins, stage position, final visibility, and deterministic artifact
ordering.

Its smallest public interface is the hidden `eval --write-search-dataset` mode plus root-exported search observation
DTOs. Search owns feature extraction; eval owns label joins and artifact writing. Callers do not import retrieval,
ranking, semantic verification, report rendering, project submodules, SQLite details, or worker transport.

These design decisions must not leak upward or sideways:

- retrieval keys, posting-list layout, SQLite rows, table names, score constants, or heap pruning mechanics;
- raw Lean expressions, raw statement text, source snippets, source paths, or absolute private paths;
- worker JSONL framing, probe chunking, cache keys, or transport diagnostics;
- label-file parsing details or private KanProofs path policy.

The preserved capability is read-only duplicate-audit evaluation. Existing eval and audit commands keep their current
metrics and report behavior; the new artifact only records the current observations for later scorer and ablation work.

Python-era behavior intentionally discarded: computing features from text in the scorer, treating label files as the
feature source, and inspecting ad hoc terminal output as the search dataset.

## Design It Twice

**Rejected: eval reconstructs feature vectors from retrieval output.** That would force eval to learn retrieval
contributions, key families, structural fingerprints, and blocker policy. It would make search-quality artifacts easy
to add, but it would reintroduce the same information leakage the crate split removed.

**Chosen: search-owned pair features plus eval-owned artifacts.** Search translates private retrieval and declaration
facts into stable feature DTOs. Eval joins those DTOs to typed labels and writes deterministic artifacts. This is deeper
because feature extraction changes with search internals, while dataset layout changes with evaluation and offline
analysis needs.

## Dataset Contract

The hidden command is:

```sh
cargo run -p lean-dup-cli -- eval --suite default --format json --write-search-dataset
```

It writes:

```text
target/search-quality/<suite>-dataset.json
```

The artifact schema version is `lean-dup.search-dataset.v1`. Top-level fields are:

- `schema_version`
- `suite`
- `pairs`

Each pair row contains:

- `left` and `right`: normalized declaration names;
- `label_status`: `positive`, `hard-negative`, or `unlabeled`;
- `label`: typed label metadata when the pair is adjudicated;
- `stage_position`: whether the pair was generated and its current rank;
- `final_visibility`: current shown-queue facts;
- `features`: search-owned stable feature facts.

Rows are sorted by `(left, right, rank)`. Unlabeled retrieved candidates remain in the artifact because later prompts
need to inspect false-positive and background-candidate distributions, not only the gold pairs.

## Feature Contract

Feature facts are intentionally coarser than retrieval internals:

- `retrieval_feature_families`: stable names such as `statement_fingerprint`, `safe_permutation_fingerprint`,
  `connective_fingerprint`, `conclusion_fingerprint`, `role_conclusion_const`, `role_hypothesis_const`, `role_head`,
  `role_other`, `other`, or `unknown`;
- `declaration_kinds`: declaration kind labels;
- `evidence_mode`: `local`, `source-backed`, or `static`;
- `structural_fingerprint_families`: fingerprint families that match without exposing fingerprint values;
- `role_overlap`: counts by role-feature family, not role keys;
- `module_relation`: module names only;
- `semantic_evidence_state`: currently `not-run` for retrieval-only eval observations;
- `cheap_blockers`: stable blocker labels such as generated, non-public, low-signal, or role-head-only evidence.

Prompt 34 may enrich semantic-evidence states after semantic reranking exists. Prompt 33 may consume these facts for a
crate-private scorer config, but the dataset must still avoid raw keys and source payloads.

## Privacy Rules

Dataset artifacts must not contain:

- absolute private paths;
- raw Lean expressions;
- raw statement text;
- source snippets;
- raw retrieval keys;
- SQLite row ids, table names, or posting records;
- worker JSONL rows or transport diagnostics.

If a future feature appears to need one of those fields, the feature boundary is wrong. Add a stable family, count,
mode, or relation instead.

## Evidence Commands

Fast fixture evidence:

```sh
cargo run -p lean-dup-cli -- eval --suite default --format json --write-search-dataset
test -f target/search-quality/default-dataset.json
```

Leak check:

```sh
rg -n 'sqlite|posting|IndexQuery|FeatureMatch|/Users/|statement_text|raw' \
  target/search-quality/default-dataset.json
```

Any remaining match must be intentional stable vocabulary, not leaked implementation data.

## Red Flag Review

- **Shallow module:** mitigated. Search extracts feature facts from multiple private search/index declarations behind
  one observation DTO.
- **Pass-through wrapper:** avoided. Eval does more than forward observations: it joins labels, normalizes pair
  identity, sorts rows, and writes artifacts.
- **Temporal decomposition:** mitigated. The boundary is organized around pair facts and labels, not retrieval step
  order.
- **Information leakage:** mitigated. Artifacts expose stable families and counts, not raw keys, postings, SQL, Lean
  expressions, source text, or worker rows.
- **Special-general mixture:** contained. Fixture and KanProofs suites share one dataset shape; private KanProofs paths
  remain suite policy.
- **Conjoined methods:** mitigated. Search owns feature extraction; eval owns dataset assembly. Neither reconstructs the
  other crate's internals.
- **Hard-to-describe public API:** mitigated. The normal public API is unchanged; the new surface is one hidden eval
  flag and one optional artifact path.
- **Implementation details contaminating interface comments:** mitigated. Interface comments describe stable dataset and
  feature facts, not storage layout or transport mechanics.
