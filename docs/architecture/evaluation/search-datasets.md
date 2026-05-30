# Search Dataset Artifacts

Feature extraction is owned by search; scoring artifacts are owned by eval. The dataset records what the search stack
observed for each candidate pair, without changing retrieval, ranking, semantic-probe policy, report JSON, or eval
scoring.

Search translates private retrieval and declaration facts into stable feature DTOs. Eval joins those DTOs to typed
labels and writes deterministic artifacts. A reconstruction-from-retrieval design was rejected: it would have forced
eval to learn retrieval contributions, key families, structural fingerprints, and blocker policy: the same leakage the
crate split removed.

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

Rows are sorted by `(left, right, rank)`; generated-only rows carry no rank. Unlabeled retrieved candidates remain in
the artifact so consumers can inspect false-positive and background distributions, not only the gold pairs.

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

The dataset must avoid raw keys and source payloads even as `semantic_evidence_state` or scorer-config consumers grow
richer.

## Privacy

Allowed in dataset artifacts: stable family names, typed counts, evidence modes, module names, label provenance.

Forbidden: absolute private paths, raw Lean expressions, raw statement text, source snippets, raw retrieval keys, SQLite
row ids or table names or posting records, worker JSONL rows, transport diagnostics.

If a future feature looks like it needs one of the forbidden items, the feature boundary is wrong. Add a stable family,
count, mode, or relation instead.

## Verification

```sh
cargo run -p lean-dup-cli -- eval --suite default --format json --write-search-dataset
test -f target/search-quality/default-dataset.json

# leak check: any match must be intentional stable vocabulary
rg -n 'sqlite|posting|IndexQuery|FeatureMatch|/Users/|statement_text|raw' \
  target/search-quality/default-dataset.json
```

## 35P realistic vector validation corpora

Prompt 35P adds a workload contract for vector-search validation. The earlier vector fixtures were too small: `top_k`
covered the whole eligible corpus, and symbolic retrieval already found every positive. Those fixtures remain useful
plumbing checks, but they cannot prove or disprove vector candidate generation.

Design Note:

- Hidden knowledge: eval owns workload denominators, label classes, manual-suite blocker reporting, and the distinction
  between fixture evidence and mathlib-scale evidence. Search owns corpus/query eligibility and top-k policy. Embedding
  owns model wrapping; vector-index owns persistence and nearest-neighbor mechanics.
- Smallest public interface: dataset and vector artifacts record workload id, model profile id, declaration-document
  policy id, eligibility policy id, eligible corpus size, query count, `top_k`, saturation status, raw label
  denominators, and cache reuse status.
- Non-leaking decisions: raw formal statements, source snippets, final model input, model prefixes, worker rows, backend
  names, table or row vocabulary, vector-cache paths, and absolute private paths stay out of artifacts.
- Preserved capability: ordinary eval and audit remain symbolic, embedding-free, and vector-index-free unless hidden
  vector flags are explicitly supplied.
- Discarded behavior: treating tiny saturated corpora or manual-suite skips as quality evidence.

Design It Twice:

- *Keep the tiny fixtures and interpret saturation carefully.* Rejected: careful prose cannot turn
  `top_k >= eligible_corpus_size` into nearest-neighbor evidence.
- *Rely only on KanProofs/mathlib manual runs.* Rejected: manual prerequisites are operator-local, so they cannot
  provide deterministic regression coverage.
- *Add a realistic deterministic fixture and still run manual suites when available.* Chosen: fixtures protect the
  evaluation contract; manual suites provide scale evidence when the local environment can run them.

The deterministic vector workload must include:

| Requirement | Contract |
| --- | --- |
| Non-saturated corpus | `top_k < eligible_corpus_size`; the fixture target is at least 2x top-k |
| Vector-only positive | at least one labeled positive where symbolic generation is absent and vector generation is present |
| Symbolic-only positive | at least one labeled positive where symbolic generation is present and vector generation is absent |
| Lexical/name hard negative | at least one labeled hard negative likely to be retrieved by semantic similarity |
| Eligibility skips | generated, private, synthetic, low-signal, missing-statement, non-actionable, and unsupported-kind rows exercise stable skip reasons |
| Manual blockers | KanProofs/mathlib prerequisites are checked, and any missing `.olean`, workspace, or mathlib artifact is recorded as a blocker, not a pass |

As of this prompt, the local KanProofs workspace, build library directory, and mathlib package directory are present.
Prompt 35Q must still run the manual suites and record the actual command result; presence of directories is not
validation evidence.

Red Flag Review:

- Shallow module: the workload contract carries denominators and label classes, not just a command name.
- Pass-through wrapper: eval does not replay search internals; it records the facts search exposes.
- Temporal decomposition: fixture and manual evidence are separated by evidence role, not by command order.
- Information leakage: raw text, worker rows, model prefixes, and vector storage details stay out of artifacts.
- Special-general mixture: fixture design stays in eval documentation and tests; model and persistence mechanics stay in
  their crates.
- Conjoined methods: corpus eligibility, document policy, vector search, and label joining remain separate surfaces with
  explicit joining facts.
- Hard-to-describe public API: workload facts are corpus size, query count, top-k, saturation, labels, and cache
  provenance.
- Implementation-detail comments: this section describes validation obligations, not backend layout or runtime
  algorithms.

## 35W command-level vector fixture

Prompt 35W promotes the realistic fixture requirement into the same CLI/eval path used by later validation decisions.
Unit tests can prove helper behavior, but they cannot prove that hidden flags, search eligibility, text-vector caching,
corpus reuse, scorer variants, artifact rows, and leak checks work together.

Design Note:

- Hidden knowledge: eval owns the command-level workload, labels, artifact truth checks, and raw denominators. Search
  owns eligibility, top-k, vector evidence, and scorer stage facts. Embedding owns runtime/profile wrapping, and
  vector-index owns corpus persistence and nearest-neighbor mechanics.
- Smallest public interface: a hidden suite id plus stable artifact facts: policy ids, top-k, eligible corpus size,
  saturation status, skip counts, label classes, scorer variant facts, cache reuse status, and privacy-safe hashes.
- Non-leaking decisions: fixture vectors, model formatting, vector-cache layout, backend storage, raw statements, source
  snippets, worker rows, retrieval keys, model prefixes, and absolute private paths stay out of artifacts.
- Preserved capability: ordinary audit and ordinary eval remain symbolic and unchanged; the fixture runs only when the
  hidden vector experiment is explicitly requested.
- Discarded behavior: using unit-only fixtures or saturated command-level runs as evidence for semantic retrieval
  quality.

Design It Twice:

- *Keep realistic cases as unit tests only.* Rejected because unit tests do not exercise CLI flags, artifact writing,
  corpus reuse, or leak checks.
- *Rely on KanProofs/mathlib manual validation.* Rejected as the only regression surface because manual prerequisites
  and run cost are operator-local.
- *Add a deterministic command-level fixture while keeping manual workloads as scale evidence.* Chosen because it gives
  repeatable end-to-end evidence for the validation machinery without claiming mathlib-scale quality.

The hidden `vector-fixture` suite has this contract:

| Requirement | Contract |
| --- | --- |
| Non-saturated top-k | eligible corpus size is greater than the private search top-k, and artifacts record `top_k_saturated = false` |
| Vector-only positive | at least one positive pair is generated by the vector path and not by symbolic retrieval |
| Symbolic-only positive | at least one positive pair is generated by symbolic retrieval and not by vector top-k |
| Lexical/name hard negative | at least one hard negative is semantically close enough to be vector-generated |
| Eligibility skip coverage | generated, private, synthetic, low-signal, missing-statement, not-actionable, and unsupported-kind declarations are skipped and counted |
| Artifact truth | rows are deduplicated by unordered declaration pair and expanded labels are not reported as unlabeled |
| Leak checks | artifacts contain stable ids, counters, hashes, and stage facts only |

The fixture is deliberately not a mathlib-quality claim. It proves that a non-saturated, labeled, hidden vector workload
can be driven through the command surface and that its artifacts are interpretable.

35W Red Flag Review:

- *Shallow module:* the fixture covers hidden CLI/eval/search/embedding/vector-index integration, not only a helper.
- *Pass-through wrapper:* eval writes truth summaries from search facts and labels; it does not replay vector search.
- *Temporal decomposition:* fixture validation is separated from manual scale evidence by evidence role.
- *Information leakage:* raw text, prefixes, backend names, storage vocabulary, worker rows, retrieval keys, and private
  paths are forbidden in artifacts.
- *Special-general mixture:* deterministic fixture behavior is a validation workload, not a production model or search
  default.
- *Conjoined methods:* eligibility, document policy, embedding runtime, corpus persistence, scoring, and labels remain
  owned by separate crates.
- *Hard-to-describe public API:* the fixture surface is a suite id plus stable artifact facts and denominators.
- *Implementation-detail comments:* this section describes validation contracts, not vector storage or model internals.
