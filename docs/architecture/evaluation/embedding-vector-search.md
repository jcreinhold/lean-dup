# Embedding-backed vector search

Hidden architecture: an optional experiment slice that owns persisted declaration-vector
corpora, embedding model use, vector candidate generation, vector scorer variants, and
vector validation artifacts. The default audit path stays read-only, deterministic,
embedding-free, vector-index-free, and authoritative. Vector candidate facts may enter
threshold calibration only after a labeled validation shows recall gain (see
[semantic-search-validation-decision.md](semantic-search-validation-decision.md)).

The prior rerank-only implementation over the symbolic candidate pool has been removed.
It was a useful negative probe, but it did not test candidate-generation recall and is
not part of the supported architecture.

## Crate boundaries

- *`lean-dup-embedding`* owns model profiles, model acquisition, model-specific input
  wrapping, CPU inference, vector normalization, runtime counters, and the per-text
  vector cache.
- *`lean-dup-vector-index`* owns persisted declaration-vector corpora, backend choice,
  corpus identity, index/cache invalidation, nearest-neighbor lookup, and corpus
  diagnostics.
- *`lean-dup-vector-search`* owns declaration-document construction for vector runs,
  vector candidate-generation policy, vector evidence/scorer variants, vector-stage
  metrics, hidden validation artifacts, validation bounds, progress, and cache/corpus
  cost accounting.
- *`lean-dup-search`* owns symbolic retrieval, symbolic pair facts, review ranking,
  semantic probes, and source/replacement facts. It does not depend on embedding,
  vector-index, or vector-search.
- *`lean-dup-eval`* owns ordinary labels, symbolic metrics, and quality gates. Vector-
  specific truth joins and artifact rows belong to `lean-dup-vector-search`.
- *`lean-dup-report`* projects ordinary eval/audit reports only; vector artifacts are
  written by the vector experiment crate.
- *`lean-dup-cli`* owns ordinary symbolic command dispatch. Hidden vector command wiring
  belongs to `lean-dup-vector-search`.

Public surfaces are three small facades: model-profile (prepare a supported model, embed
a batch, report stable model/input/runtime facts); vector-corpus (build, open, reuse,
query a persisted declaration-vector corpus while reporting stable corpus/build/query
facts); vector-validation workflow (`VectorValidationRequest`, `VectorValidationOutcome`,
`run_vector_validation`). Ordinary search/eval/report APIs expose none of these vector
facts.

### Deletion contract

The vector experiment is removable as a vertical slice. Deleting `crates/vector-search`,
`crates/embedding`, and `crates/vector-index` must not require edits to `crates/search`,
`crates/eval`, or `crates/report`. Core crates must not re-export `SearchVector*`,
`VectorSearch*`, embedding-document DTOs, vector scorer variants, vector generated
counters, or vector artifact paths. Boundary tests enforce that only `lean-dup-vector-search`
depends on `lean-dup-embedding` and `lean-dup-vector-index`.

Model/profile experiments are selected in
[embedding-model-selection.md](embedding-model-selection.md), not by search or eval. The
35T decision keeps the candidate matrix small: BGE-small as the hidden baseline, BGE-base
as the only additional local CPU profile for capacity ablation, and two profile-private
role-format variants. Search/eval/report artifacts may record profile id, role-format id,
dimension, model/cache/runtime facts, and denominators; they must not record FastEmbed
enum names, model prefixes, ONNX/ORT details, tokenizer files, or model paths.

Prompt 35U makes that decision executable. Hidden vector experiments request stable
profile ids and input-format ids. The embedding crate maps those ids to canonical model
ids, runtime support, role wrapping, dimensions, normalization, and vector-cache identity.
Search and eval never choose prefixes or runtime types. Vector corpus provenance includes
the input-format id/version because corpus vectors are formatting-dependent experiment
data, not just model data.

Decisions that do not cross those boundaries: tokenizer mechanics, pooling, normalization,
model-file layout, backend names, persistence format, ANN parameters, database cache
layout, raw declaration-document text, vector-cache filenames, and vector-database query
syntax. Search, eval, report, and CLI must not be able to tell whether the current
backend is LanceDB, sqlite-vec, Qdrant, HNSW, or a future replacement.

## Vector-corpus contract

`lean-dup-vector-index` exposes only declaration-corpus operations:

- build or reuse a persisted corpus from declaration identities, metadata, content
  hashes, and normalized vectors;
- open a previously built corpus only when schema and provenance match;
- query nearest declarations and return stable declaration facts with scores where higher
  means closer;
- report corpus status as `built`, `reused`, `missing`, `stale`, or `unusable`.

Public provenance: source corpus fingerprint, embedding model profile and fingerprint,
embedding input-format id and version, declaration-document policy id and version, vector
dimension, normalization contract. The schema version is owned by the vector-index crate.
If any provenance fact changes, the crate reports `stale` rather than letting search
inspect database files.

LanceDB is the first persistent local backend. Lance dependencies use vendored protobuf
so the crate is self-contained without a machine-level `protoc` prerequisite. Fixture
corpora may use exact backend search when they are too small for an ANN index to pay off;
production-sized corpora create the backend vector index during build. The distinction is
private—callers get the same corpus summary and nearest-declaration facts either way.

## Hidden vector candidate generation

The hidden workflow builds or reuses a persisted vector corpus once per observation. It
embeds workspace declarations as queries and comparison declarations as documents. When
comparison indexes are present, the corpus is built from those comparison declarations;
otherwise it is built from the local workspace so fixture and local experiments can run.
Provenance controls reuse: a valid corpus is reopened, not rebuilt.

Inside `lean-dup-vector-search`, vector candidates merge with symbolic observations before
hidden scorer variants run. Existing symbolic pairs may gain vector facts when the
nearest-neighbor query also finds them. Vector-only pairs are generated and ranked for
experiment measurement, but they are not part of production visibility.

### 35M design note: eligibility and top-k

Historically this policy sat in `lean-dup-search`; after the deletability refactor it
belongs to `lean-dup-vector-search`. The hidden knowledge is Lean
actionability: generated declarations, private declarations, synthetic fixtures,
low-signal declarations, missing statements, non-actionable declarations, and unsupported
kinds. Embedding must not learn those concepts, and vector-index must not learn them
either; both crates receive only already-eligible declaration documents and vectors.

The smallest public interface is a named eligibility policy plus counts: policy id,
policy version, total declarations, eligible declarations, skip reasons, `top_k`,
eligible corpus size, and `top_k_saturated`. The skip labels are stable diagnostic facts,
not retrieval keys. Document policy remains separate: eligibility decides which
declarations enter a vector corpus or query set; declaration-document policy decides which
text fields are embedded for those declarations.

Non-leaking decisions include raw statement text, final model input text, model prefixes,
tokenizer/runtime details, database paths, table names, ANN parameters, and vector-cache
layout. The preserved user-facing capability is the default symbolic duplicate audit:
ordinary audit and eval do not build models, query vector corpora, or change visibility.
The discarded behavior is the Python-era/ad hoc habit of embedding whatever row set is
convenient without recording whether the corpus is actionable or whether `top_k` saturated
the corpus.

Design It Twice:

- *Embedding or vector-index rejects noisy declarations.* Rejected: it would force model
  runtime and vector persistence crates to know Lean actionability policy.
- *Eval filters rows after the observation.* Rejected: eval would reconstruct candidate
  generation and could not report what search actually queried.
- *A vector experiment slice owns named corpus/query eligibility policies.* Chosen:
  vector validation needs Lean actionability facts and vector-specific cost/provenance
  rules, but symbolic search should not expose vector-shaped branches. This keeps the
  public surface small and prevents temporal decomposition across search, embedding,
  vector-index, and eval.

The default hidden policy is `actionable-public-statement`. It excludes declarations with
stable reasons: `generated`, `private`, `synthetic`, `low-signal`, `missing-statement`,
`not-actionable`, and `unsupported-kind`. A named `broad` policy may include normally
excluded declarations for experiments, but the artifact must record that policy choice.
Saturated runs (`top_k >= eligible_corpus_size`) are smoke evidence only; they cannot
support vector-search quality claims.

`lean-dup-vector-search` controls declaration-document selection, per-query top-k,
symbolic/vector merge ordering for hidden variants, and the rule that vector-only
candidates are ranked for measurement but not shown by vector score alone. Model profile
internals, FastEmbed runtime mechanics, final model input strings, LanceDB storage, ANN
parameters, vector database paths, table names, and vector-cache filenames stay out of
search, eval, and report artifacts.

Stage counters in hidden artifacts:

| Counter | Meaning |
| --- | --- |
| `vector_generated` | pair produced by nearest-neighbor vector search |
| `symbolic_generated` | pair produced by symbolic retrieval |
| `merged_generated` | pair exists after the hidden search merge stage |
| `ranked` | pair survived into ranked observation facts |
| `visible` | pair entered the shown queue |

### 35N artifact truthfulness

Vector artifacts use schema `lean-dup.vector-search.v2`. They are eval-owned summaries of pair truth, not logs of
generation events. Each row is keyed by the normalized unordered declaration pair. If symbolic and vector generation
both observe the same pair, eval emits one row with the best facts: any symbolic generation, any vector generation, any
merged generation, any ranked survival, any visible survival, minimum rank, minimum vector rank, maximum vector score,
sorted generation policies, sorted feature families, and privacy-safe declaration hashes.

Design Note:

- Hidden knowledge: eval owns label expansion, conflict resolution, row deduplication, and validation denominators;
  search owns stage facts and privacy-safe content hashes; embedding and vector-index own model/runtime/storage
  mechanics.
- Smallest public interface: artifact rows expose declaration names, content hashes, explicit label facts, stage facts,
  and raw metric denominators.
- Non-leaking decisions: raw formal statements, source snippets, final model inputs, model prefixes, backend names,
  persistence vocabulary, retrieval keys, worker rows, and absolute paths must not appear in artifacts.
- Preserved capability: default symbolic audit and ordinary eval remain unchanged and do not build or query vector
  corpora.
- Discarded behavior: treating expanded cluster positives or hard negatives as unlabeled, treating duplicate event rows
  as evidence, and hiding top-k saturation.

Design It Twice:

- *Store one row per generation event.* Rejected: readers would have to repair unordered-pair duplicates and merge
  contradictory row facts.
- *Join only direct typed-pair labels.* Rejected: typed clusters still define expanded scoring denominators.
- *Eval-owned truth-preserving artifact builder.* Chosen: search supplies stage facts; eval supplies label truth and
  writes one stable row per unordered pair.

The artifact records label facts as `positive`, `hard-negative`, `expanded-positive`, `expanded-hard-negative`, or
`unlabeled`. A row can carry multiple typed label facts when a direct typed pair and an expanded typed cluster agree.
Contradictory typed labels are fixture errors, not skipped compatibility rows. Scoring denominators still come from the
normalized positive and hard-negative label sets.

Additional metrics in `vector_stage_metrics` report raw `found/total` counts for vector top-k recall, vector top-k
precision, top-k saturation, vector-only positives, vector-only hard negatives, symbolic-only positives, symbolic-only
hard negatives, merged-generated recall, ranked recall, visible precision, and visible hard-negative count. These
metrics are artifact-local; Prompt 35O is responsible for testing vector score as hidden ranking evidence.

### 35O vector score as search evidence

Vector similarity is now a search-owned pair feature for hidden experiments, not inert artifact metadata. Search
converts nearest-neighbor facts into stable vector evidence: feature version, score bucket, rank bucket, and bounded
reciprocal-rank fact. The scorer consumes those stable facts; eval measures the resulting variants. Eval never
reconstructs rank order from raw vector scores.

Design Note:

- Hidden knowledge: search owns vector evidence construction, scorer variants, and symbolic/vector merge policy;
  embedding owns model/runtime behavior; vector-index owns nearest-neighbor mechanics and score conversion; eval owns
  denominators and artifacts.
- Smallest public interface: scorer variant id, vector evidence feature version, stable vector evidence facts, and raw
  stage denominators.
- Non-leaking decisions: backend distance conventions, database score semantics, model-specific normalization,
  tokenizer/runtime details, model prefixes, cache filenames, and vector database layout stay below their owning crate
  boundaries.
- Preserved capability: default audit and ordinary eval continue to use the symbolic scorer and remain embedding-free.
- Discarded behavior: recording vector similarity only as metadata while asking validation to infer whether vector
  evidence would have helped ranking.

Design It Twice:

- *Eval adjusts ranks from vector similarities after search.* Rejected: eval would own a second ranking pipeline and
  would need to understand vector score semantics.
- *Expose raw vector distances and tune thresholds in artifacts.* Rejected: backend/model semantics would leak upward
  and validation could optimize against storage details.
- *Search converts vector similarity into stable pair features and hidden scorer variants.* Chosen: search already owns
  pair features and ranking policy, so vector evidence enters the same abstraction as symbolic evidence.

Hidden scorer variants:

| Variant | Meaning |
| --- | --- |
| `symbolic-only` | current symbolic baseline; vector facts may be present but do not affect score |
| `vector-evidence-only` | ranks hidden vector candidates using only stable vector evidence facts |
| `symbolic-plus-vector` | combines symbolic feature facts with stable vector evidence facts |

Vector-generated recall remains candidate-generation evidence. Ranked recall and visible precision are reported
separately for each scorer variant, so validation can distinguish "vector found the pair" from "vector evidence ranked
the pair well enough to be useful." The default scorer, thresholds, report JSON, semantic-probe policy, and CLI command
names do not change.

### 35P realistic validation corpora

Vector-search validation now requires workloads that can actually distinguish candidate-generation behavior. A tiny
fixture where `top_k` covers the entire corpus remains a smoke test only. A validation workload counts as vector-search
quality evidence only when `top_k < eligible_corpus_size` for the relevant queries and the labels include vector-only
positives, symbolic-only positives, and lexical/name hard negatives.

Design Note:

- Hidden knowledge: eval owns workload suitability, label denominators, manual-suite blocker reporting, and the final
  validation decision; search owns top-k, eligibility, and candidate-policy facts; embedding and vector-index keep model
  and storage mechanics private.
- Smallest public interface: workload id, model profile id, declaration-document policy id, eligibility policy id,
  query count, eligible corpus size, top-k, saturation status, vector-only/symbolic-only label counts, runtime/cache
  facts, and manual-suite blocker status.
- Non-leaking decisions: raw statements, source snippets, final model input strings, model prefixes, backend names,
  table or row vocabulary, worker rows, cache paths, and ANN parameters stay out of artifacts.
- Preserved capability: default symbolic audit/eval still do not build models, build vector corpora, or change
  visibility.
- Discarded behavior: treating saturated fixture runs, manual-suite skips, or successful vector-index build/query as
  quality evidence.

Design It Twice:

- *Keep using current tiny fixtures.* Rejected: they cannot show vector-only recall because symbolic retrieval already
  finds the positives and the vector top-k is saturated.
- *Use only KanProofs/mathlib manual suites.* Rejected: they provide scale evidence when present but are not reliable
  regression fixtures.
- *Use a deterministic realistic fixture plus optional manual validation.* Chosen: the fixture pins non-saturated
  denominators and label classes; manual suites supply mathlib-scale evidence when local prerequisites exist.

Required workload facts for Prompt 35Q:

| Fact | Why it matters |
| --- | --- |
| `eligible_corpus_size` and `top_k` | proves whether nearest-neighbor selection is non-saturated |
| `top_k_saturated` | prevents smoke tests from being promoted as quality evidence |
| query count | prevents one lucky query from looking like corpus-level evidence |
| vector-only positives | measures candidate-generation value unavailable to symbolic retrieval |
| symbolic-only positives | prevents vector-only tunnel vision |
| lexical/name hard negatives | measures semantic-neighbor false-positive risk |
| eligibility skip reasons | verifies noisy declarations do not enter the default vector corpus |
| corpus/cache reuse facts | separates cold-build cost from warm search behavior |

Prompt 35Q may not claim mathlib-scale quality from fixture-only evidence, and skipped manual suites do not count as
passes. On this machine the KanProofs workspace, compiled library directory, and mathlib package directory are present;
35Q must still record the actual manual command result and exact blocker if a suite fails.

Red Flag Review:

- Shallow module: the workload contract records the facts needed to judge vector search, not just the fact that a
  command ran.
- Pass-through wrapper: eval measures search-produced facts instead of reconstructing vector retrieval.
- Temporal decomposition: fixture, manual, cold-build, and warm-reuse evidence are separated by validation meaning.
- Information leakage: workload artifacts carry stable policy ids and hashes, not raw text or backend/runtime details.
- Special-general mixture: hidden vector validation stays out of ordinary eval and report semantics.
- Conjoined methods: candidate generation, ranking variants, labels, and validation decisions remain separate.
- Hard-to-describe public API: the evidence is corpus/query size, top-k saturation, label classes, and raw denominators.
- Implementation-detail comments: backend names appear only as architecture evidence, not artifact schema.

Cache-only missing model preparation is a skipped vector experiment, not an eval failure.
The symbolic baseline remains in the artifact, and existing suite gates are evaluated
against the symbolic baseline, not against experimental vector output.

## Invalidation

A corpus is reusable only when provenance matches. Identity must include at least: source
corpus identity and declaration handles, embedding model profile and model fingerprint,
declaration-document input-policy version, vector dimension and normalization contract,
vector-index schema version, and backend-compatible build facts. On any mismatch, the
vector-index crate reports the corpus as missing or stale and the hidden preparation
workflow rebuilds it. Search must not inspect backend-specific manifests.

## Backend choices

| Backend | Status | Rationale |
| --- | --- | --- |
| FastEmbed | first runtime | provides BGE-small as default text model and hides ONNX/tokenizer details behind a batch API |
| LanceDB | first persistence | embedded local filesystem path, Rust-native, persistent storage, metadata filtering, vector indexes |
| sqlite-vec | fallback persistence | small, SQLite-shaped, fits beside existing cache; pre-v1 makes it a fallback not first choice |
| Qdrant | future persistence | strong for service deployments and extended filtering; first CLI path should not require a service |
| HNSW (in-memory) | rejected as default | a backend may use HNSW internally, but an in-memory-only graph fails the persistent-corpus requirement |

Backend identity is architecture evidence only. References:
[FastEmbed](https://docs.rs/crate/fastembed/latest),
[LanceDB](https://docs.rs/lancedb/latest/lancedb/index.html),
[sqlite-vec](https://github.com/asg017/sqlite-vec),
[Qdrant](https://qdrant.tech/documentation/quick-start/),
[hnsw_rs](https://docs.rs/hnsw_rs/latest/hnsw_rs/hnsw/index.html).

## Design alternatives considered

- *Tune the existing rerank-only experiment.* Rejected: reranking the symbolic candidate
  pool cannot discover pairs that symbolic retrieval missed, so it cannot measure
  candidate-generation recall.
- *Vector storage and ANN inside `lean-dup-search`.* Rejected: mixes search policy with
  persistence and would make every backend change a search change.
- *Rebuild an in-memory vector graph per run.* Rejected: at mathlib scale this is a
  performance bug; persistence and reuse are part of the abstraction, not a later
  optimization.
- *Require a vector-database service (e.g., Qdrant) for the first CLI backend.* Rejected:
  a service or Docker prerequisite would make the default developer workflow heavier than
  the capability needs. Qdrant may become a future backend behind the same facade.
- *Eval calls embedding and vector-index directly to assemble pair rows.* Rejected:
  exposes runtime and corpus mechanics to eval and measures an eval-only workflow rather
  than search behavior.
- *A separate vector-search CLI command, unrelated to audit/eval.* Rejected: it can
  inspect nearest neighbors but cannot measure stage survival through the same scoring
  and visibility pipeline.
- *Open-ended `EmbeddingModelSpec { id, revision }`.* Rejected: a false abstraction when
  the runtime still assumes BERT/MiniLM shape. Private model profiles inside
  `lean-dup-embedding` are the extension point; supported ids resolve to profiles before
  acquisition or runtime.
- *Search emits final model-input strings.* Rejected: leaks BGE/FastEmbed prefix policy
  into search. Search provides structured declaration documents; embedding owns model-
  specific query/document wrapping.

## 35S Semantic Declaration Documents

Design Note:

- Hidden knowledge: search owns which declaration content is useful for duplicate-audit
  candidate generation; Lean worker/index own stable declaration facts; embedding owns
  only model-profile role wrapping.
- Smallest public interface: a document policy id/version, content availability counters,
  privacy-safe content hashes, and in-memory document text for hidden embedding calls.
- Non-leaking decisions: raw statements, docstrings, definition bodies, final model input
  strings, model prefixes, tokenizer/runtime details, worker rows, source snippets,
  retrieval keys, database paths, and vector storage vocabulary stay out of public
  search/eval/report artifacts.
- Preserved capability: ordinary symbolic audit and ordinary eval remain unchanged,
  embedding-free, and vector-index-free.
- Discarded behavior: the retired name/module/kind/features input string, policies that
  claim informal text without supplying it, and compatibility aliases for removed document
  policies.

Design It Twice:

- *Keep name plus formal statement only.* Rejected: it omits definition bodies and
  docstrings, so many definition-like declarations are embedded with too little
  distinguishing content.
- *Let embedding build final model input from Lean internals.* Rejected: embedding would
  learn Lean declaration kinds, proof-body exclusions, docstring availability, and
  duplicate-audit actionability.
- *Search owns semantic document policies over stable declaration facts.* Chosen:
  worker/index expose declaration facts; search chooses which facts are useful for the
  hidden semantic-search policy; embedding applies only profile-specific role formatting.

Current semantic document policies:

| Policy | Embedded text selected by search |
| --- | --- |
| `statement` | theorem/lemma/axiom statements and definition signatures only |
| `name-and-statement` | declaration name plus statement/signature; default hidden policy |
| `definition-aware` | name plus statement/signature plus definition body summary when available |
| `docstring-augmented` | docstring when available, then name plus statement/signature, plus definition body summary when available |

The worker/index boundary now supplies optional `docstring_text` and
`definition_body_summary` declaration facts. Search records availability counters for
those facts before embedding, and artifacts receive policy ids, versions, counters, and
content hashes. A theorem proof body is not a declaration-document source for the default
hidden semantic-search path. If future experiments need proof text, they must introduce a
named non-default policy and separate leak checks.

Search may fall back within `docstring-augmented` for declarations without docstrings
because the field is genuinely supplied by the worker/index boundary and availability is
counted. A policy that claims informal text while the worker never supplies informal facts
is not allowed.

35S Red Flag Review:

- *Shallow module:* the search document policy now hides meaningful content selection
  rather than forwarding a hardcoded string shape.
- *Pass-through wrapper:* worker/index expose stable facts; search transforms them into
  policy-specific document text and hashes.
- *Temporal decomposition:* the split follows ownership of declaration facts, document
  policy, and model wrapping, not worker/index/search execution order.
- *Information leakage:* Lean content policy stays in search; model prefixes stay in
  embedding; raw content and worker rows stay out of artifacts.
- *Special-general mixture:* model runtime remains general text embedding; Lean-specific
  duplicate-search semantics stay in search.
- *Conjoined methods:* docstring extraction, definition body summary, document selection,
  embedding, and vector-index query remain separately owned.
- *Hard-to-describe public API:* policy id/version, counters, and hashes describe the
  artifact surface.
- *Implementation-detail comments:* comments should describe semantic-document facts and
  privacy constraints, not Lean expression internals or model runtime details.

## Red-flag checklist

- *Shallow module:* the vector-index crate hides persistence, ANN, and invalidation
  behind a corpus facade rather than exposing backend operations.
- *Pass-through wrapper:* the crate provides corpus build/open/query facts; it does not
  re-export LanceDB, sqlite-vec, Qdrant, or HNSW APIs.
- *Temporal decomposition:* organized by hidden knowledge, not by build/embed/index/query
  steps.
- *Information leakage:* model profiles, backend names, ANN parameters, vector-cache
  layout, and raw document text each have one owning crate.
- *Special-general mixture:* lean-dup-specific declaration documents stay in search;
  general vector persistence stays in vector-index; model runtime stays in embedding.
- *Conjoined methods:* preparing models, building corpora, querying candidates, and
  evaluating metrics are not exposed as one public workflow object.
- *Hard-to-describe public API:* three facades—model profile, vector corpus, stage
  facts—with stable facts.
- *Implementation details in interface comments:* backend names appear in this document
  as architecture evidence; they must not become search, eval, or report interface
  comments.

35M Red Flag Review:

- *Shallow module:* eligibility pulls concrete actionability checks behind one search
  policy instead of scattering boolean filters across eval and embedding.
- *Pass-through wrapper:* the public facts are counts and stable skip reasons; they do
  not forward declaration rows or backend status through another name.
- *Temporal decomposition:* eligibility runs before embedding and corpus work because it
  is a search policy, not because it is a pipeline step owned by another crate.
- *Information leakage:* Lean actionability stays in search; model/runtime and
  persistence details stay in their owning crates.
- *Special-general mixture:* default policy is lean-dup-specific; document policy and
  vector persistence remain separate abstractions.
- *Conjoined methods:* selecting eligible declarations, formatting document text,
  embedding vectors, and querying corpora remain separate crate responsibilities.
- *Hard-to-describe public API:* the API is policy plus counts plus top-k saturation.
- *Implementation-detail comments:* skip reasons describe stable search facts, not
  worker rows, storage fields, or model input mechanics.

35N Red Flag Review:

- *Shallow module:* eval does artifact truth work—deduplication, label expansion, and
  denominator construction—instead of forwarding event rows.
- *Pass-through wrapper:* vector artifact v2 changes the representation from generation
  events to unordered-pair summaries.
- *Temporal decomposition:* row truth is owned by eval because eval owns labels and
  validation denominators, not because eval happens after search.
- *Information leakage:* backend names, storage vocabulary, model prefixes, raw text,
  worker rows, retrieval keys, and absolute paths remain forbidden artifact content.
- *Special-general mixture:* vector-specific denominators live in the hidden vector
  artifact; ordinary eval metrics stay unchanged.
- *Conjoined methods:* search generation, label parsing, vector runtime, vector storage,
  and artifact writing remain separately owned.
- *Hard-to-describe public API:* one row per unordered pair plus `found/total` vector
  stage metrics.
- *Implementation-detail comments:* interface comments describe artifact facts and
  privacy constraints, not database or runtime mechanics.

35O Red Flag Review:

- *Shallow module:* vector evidence is real scorer input, not a report-only wrapper around a stored score.
- *Pass-through wrapper:* search converts nearest-neighbor facts into buckets and rank facts instead of forwarding
  backend score conventions.
- *Temporal decomposition:* ranking evidence is owned by search because search owns scoring, not because it runs after
  vector-index queries.
- *Information leakage:* backend distances, model prefixes, tokenizer/runtime details, cache filenames, and database
  layout remain below their owning crates.
- *Special-general mixture:* vector evidence variants are hidden scorer variants; the default symbolic scorer remains
  separate.
- *Conjoined methods:* eval measures scorer variants but does not construct their scores.
- *Hard-to-describe public API:* variant id plus vector evidence version plus raw denominators.
- *Implementation-detail comments:* comments describe stable evidence and artifact facts, not backend or model
  mechanics.

## 35V scorer and artifact repair

Prompt 35V repairs hidden vector scorer variants before new validation can
interpret them. The rule is strict: search converts nearest-neighbor output into
stable vector evidence facts, and eval records artifact truth from search stage
facts plus labels. Eval must not reverse-engineer ranking from vector
similarities.

Design Note:

- Hidden knowledge: search owns vector evidence construction, hidden scorer
  rankability rules, and visible-stage group accounting; eval owns row truth,
  denominators, label joining, and artifact consistency checks.
- Smallest public interface: scorer variant id, vector evidence feature version,
  rank bucket, bounded score bucket, reciprocal-rank fact, top-k membership, and
  raw denominators for candidate generation, ranked survival, visible precision,
  and hard-negative survival.
- Non-leaking decisions: raw nearest-neighbor score direction, backend distance
  conventions, model normalization behavior, model prefixes, tokenizer/runtime
  details, vector-cache filenames, database layout, and storage vocabulary stay
  below their owning crate boundaries.
- Preserved capability: default audit and ordinary eval still use the symbolic
  scorer and do not prepare models, build corpora, query vector indexes, or
  change visibility.
- Discarded behavior: reporting impossible visible group counts, ranking
  vector-only pairs under the symbolic baseline, and serializing raw vector
  score values in pair artifacts.

Design It Twice:

- *Eval recomputes ranks from vector scores.* Rejected because it creates a
  second ranking implementation and forces eval to know search scoring policy.
- *Artifacts expose raw vector scores and tune visibility there.* Rejected
  because backend/model score semantics would leak upward and become calibration
  inputs.
- *Search-owned vector evidence with eval-owned truth summaries.* Chosen because
  search already owns ranking policy and pair features, while eval owns labels
  and denominators. The interface is deeper: eval sees stable evidence facts and
  variant outcomes, not the mechanics that produced them.

Hidden scorer variants now have distinct obligations:

| Variant | Obligation |
| --- | --- |
| `symbolic-only` | match the current symbolic baseline; vector-generated pairs remain generated evidence but are not ranked by vector facts |
| `vector-evidence-only` | rank only pairs with search-owned vector evidence facts |
| `symbolic-plus-vector` | combine symbolic scorer facts and stable vector evidence facts without changing the default scorer |

Artifact rows and search observation JSON no longer expose raw vector score
values. They carry stable vector evidence facts instead: feature version, score
bucket, rank bucket, bounded reciprocal-rank value, top-k membership, and vector
rank. Variant artifacts count visible groups by distinct query/anchor group
rather than by visible pair row, so `visible_groups.found <=
visible_groups.total` is an invariant.

Candidate-generation recall stays separate from ranking. `vector_generated`
records whether vector top-k produced a pair. Variant `ranked_recall`,
`visible_precision`, and visible hard-negative counts record what the scorer did
with generated pairs.

35V Red Flag Review:

- *Shallow module:* search now owns real vector evidence conversion and group
  accounting instead of forwarding raw scores to eval.
- *Pass-through wrapper:* eval records truth summaries from stage facts; it does
  not wrap or replay vector-index output.
- *Temporal decomposition:* the split follows hidden knowledge: search ranks,
  eval joins labels, vector-index searches, embedding embeds.
- *Information leakage:* raw score semantics, backend names, model formatting,
  storage paths, and cache filenames are not public search/eval/report facts.
- *Special-general mixture:* hidden vector scorer variants stay out of ordinary
  audit and ordinary eval behavior.
- *Conjoined methods:* eval no longer needs to understand scorer internals to
  explain variant artifacts.
- *Hard-to-describe public API:* the variant surface is describable as variant
  id, stable vector evidence facts, raw denominators, and label truth.
- *Implementation-detail comments:* public comments describe caller-visible
  facts, not vector storage, model files, or temporary migration details.

## 35W command-level non-saturated fixture

Prompt 35W adds a deterministic command-level fixture for hidden vector validation. The fixture exists because earlier
quality claims were too easy to overread: a unit test can prove that a helper returns a candidate, but it cannot prove
that the command path records eligibility, top-k saturation, cache reuse, scorer variants, artifact truth, and leak
checks together.

Design Note:

- Hidden knowledge: search owns corpus/query eligibility, vector top-k, vector evidence, symbolic/vector merging, and
  scorer stage facts. Eval owns workload labels, denominator truth, artifact writing, and leak checks. Embedding owns
  model/profile/input wrapping, and vector-index owns persistent corpus reuse.
- Smallest public interface: a hidden suite and stable artifact facts: model/profile/input-format ids, document and
  eligibility policy ids, top-k, eligible corpus size, saturation status, skip counts, vector-only and symbolic-only
  denominators, scorer variant ids, and privacy-safe hashes.
- Non-leaking decisions: deterministic fixture vectors, model prefixes, tokenizer/runtime details, vector-cache
  layout, corpus storage, backend names, raw statements, worker rows, retrieval keys, and absolute private paths do not
  appear in search/eval/report public facts or artifacts.
- Preserved capability: default audit and ordinary eval remain symbolic; vector fixtures require hidden vector flags.
- Discarded behavior: treating saturated command runs or unit-only vector cases as semantic-search quality evidence.

Design It Twice:

- *Keep realistic vector cases as eval unit tests.* Rejected because they would not exercise hidden CLI flags, artifact
  paths, text-vector cache behavior, persistent corpus reuse, or command-level leak checks.
- *Wait for KanProofs/mathlib manual runs.* Rejected as the only validation surface because long manual workloads need
  separate operational controls and cannot provide cheap deterministic regression coverage.
- *Add a deterministic command-level fixture and keep manual workloads for scale evidence.* Chosen because it is deeper:
  the command path exposes only stable workload and artifact facts while each crate keeps its hidden mechanics.

The `vector-fixture` suite is a validation fixture, not a production retrieval source. Its eligible corpus is larger
than the private search top-k, so `top_k < eligible_corpus_size` and the artifact records a non-saturated run. It
contains one vector-only positive, one symbolic-only positive, lexical/name hard negatives, and declarations that
exercise every default eligibility skip reason without entering the vector corpus.

The fixture proves machinery, not mathlib-scale quality. Prompt 35Y may use it as evidence that the repaired command
surface is truthful and non-saturated, but any mathlib-scale claim still requires a completed non-saturated
KanProofs/mathlib workload.

35W Red Flag Review:

- *Shallow module:* the fixture runs through CLI/eval/search/embedding/vector-index rather than wrapping a unit helper.
- *Pass-through wrapper:* eval joins labels to search stage facts; it does not reconstruct candidates from embedding or
  vector-index internals.
- *Temporal decomposition:* command fixture evidence and manual scale evidence are separate because they answer
  different validation questions.
- *Information leakage:* raw text, model prefixes, backend names, storage vocabulary, worker rows, retrieval keys, and
  private paths are forbidden in artifacts.
- *Special-general mixture:* deterministic fixture behavior is hidden validation infrastructure, not a general model
  profile for quality claims.
- *Conjoined methods:* eligibility, document policy, embedding, persistence, scoring, and artifact truth remain separate
  ownership boundaries.
- *Hard-to-describe public API:* hidden validation exposes suite id, policy ids, top-k facts, denominators, label
  classes, and cache reuse status.
- *Implementation-detail comments:* public comments describe validation facts and privacy rules, not backend or runtime
  layout.

## 35X bounded large-workload validation and progress

Prompt 35X makes hidden large semantic/vector validation observable and bounded. It does not decide whether vector
search is useful; it makes long KanProofs/mathlib-scale runs measurable enough for Prompt 35Y to decide.

Design Note:

- Hidden knowledge: CLI/eval own operator interaction, workflow budgets, phase progress, partial status artifacts,
  runtime/RSS/cache-size accounting, and validation lifecycle. Search owns candidate-generation progress counters and
  stable vector summaries. Embedding owns model/runtime/cache mechanics; vector-index owns corpus persistence and
  nearest-neighbor mechanics.
- Smallest public interface: hidden validation bounds plus stable vector artifact fields for phase runtimes, RSS
  availability, model/text-vector/vector-corpus cache sizes, corpus reuse status, query count, eligible corpus size,
  top-k, saturation status, cold-build timing, warm-open/query timing, and artifact path.
- Non-leaking decisions: model files, tokenizer/runtime details, vector-cache layout, corpus table/storage layout,
  backend names, worker rows, retrieval keys, raw statements, model input prefixes, and private paths do not appear in
  public search/eval/report APIs or artifacts.
- Preserved capability: default audit and ordinary eval remain symbolic and do not prepare models, build vector
  corpora, query vector indexes, or show vector progress.
- Discarded behavior: opaque manual vector validation runs that consume minutes or large RSS without progress, cache
  reuse facts, or a partial artifact.

Design It Twice:

- *Run full mathlib validation as one opaque command.* Rejected because failure or interruption leaves no useful
  validation evidence and hides runtime/RSS/cache cost.
- *Print ad hoc progress from embedding/vector-index/search internals.* Rejected because it leaks ownership details and
  teaches workflow code about backend phases.
- *Report progress and cost at workflow boundaries through stable DTOs.* Chosen because CLI/eval own operator-facing
  behavior, while embedding, vector-index, and search only expose stable counters and statuses through crate-root
  surfaces.

Hidden vector validation now reports progress for these stable phases: model preparation, declaration loading,
eligibility filtering, document construction, embedding/vector-cache lookup, corpus build/open/reuse, vector query,
scoring variants, artifact writing, and leak checks where the workflow performs them. Phase names are workflow/search
facts, not backend implementation names.

Vector artifacts carry a `validation_bounds` block and a `validation_cost` block. Bounds include maximum declarations,
queries, runtime, and RSS observation threshold. Cost includes phase runtimes, RSS or `unavailable`, cache sizes, vector
corpus size, query count, eligible corpus size, top-k, saturation, corpus reuse status, cold-build timing,
warm-open/query timing, and artifact path. A warm validation run is expected to reuse the model cache, text-vector
cache, and vector corpus when provenance matches; the artifact records whether the vector corpus was built or reused.

When a hidden validation is skipped or exceeds a configured budget, eval writes a partial vector artifact with a stable
status and reason. Partial artifacts intentionally contain stable bounds, cost, query/corpus counts, and vector summary
facts, but no pair rows and no raw declaration/model/backend details.

35X Red Flag Review:

- *Shallow module:* CLI/eval now own real workflow observability and budget enforcement instead of forwarding an opaque
  long-running command.
- *Pass-through wrapper:* progress/cost fields summarize stable validation facts, not backend logs.
- *Temporal decomposition:* long-run lifecycle, candidate generation, embedding, and vector persistence remain separated
  by hidden knowledge rather than by execution order alone.
- *Information leakage:* progress events and artifacts use phase names, counts, statuses, and cache sizes; they do not
  expose model files, vector storage layout, backend names, worker rows, raw text, prefixes, or private paths.
- *Special-general mixture:* bounds and progress are hidden validation behavior, not ordinary audit behavior.
- *Conjoined methods:* eval handles budgets/artifacts, search reports vector candidate phases, embedding handles model
  runtime, and vector-index handles corpus reuse.
- *Hard-to-describe public API:* the operator-facing surface is bounded validation plus phase/cost facts.
- *Implementation-detail comments:* interface comments describe caller-visible progress and budget facts, not temporary
  backend mechanics.
