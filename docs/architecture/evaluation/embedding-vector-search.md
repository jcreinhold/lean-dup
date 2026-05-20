# Embedding-backed vector search

Hidden architecture: a persisted declaration-vector corpus built once and reused across
audits, queried as a candidate-generation stage that runs alongside symbolic retrieval.
The default audit path stays read-only, deterministic, embedding-free, and authoritative.
Vector candidate facts may enter threshold calibration only after a labeled validation
shows recall gain (see [vector-search-validation.md](vector-search-validation.md)).

For the prior rerank-only experiment over the symbolic candidate pool, see
[embedding-rerank-experiment.md](embedding-rerank-experiment.md). That experiment was a
useful negative probe; it does not test candidate-generation recall.

## Crate boundaries

- *`lean-dup-embedding`* owns model profiles, model acquisition, model-specific input
  wrapping, CPU inference, vector normalization, runtime counters, and the per-text
  vector cache.
- *`lean-dup-vector-index`* owns persisted declaration-vector corpora, backend choice,
  corpus identity, index/cache invalidation, nearest-neighbor lookup, and corpus
  diagnostics.
- *`lean-dup-search`* owns declaration-document construction, vector candidate-generation
  policy, and merging vector candidates with symbolic candidates before scoring.
- *`lean-dup-eval`* owns labels, experiment lifecycle, vector-stage metrics, artifacts,
  and go/no-go decisions.
- *`lean-dup-report`* projects stable status and artifact facts without depending on
  model runtime or vector-index internals.
- *`lean-dup-cli`* owns hidden flags, explicit preparation commands, and I/O behavior.

Public surfaces are three small facades: model-profile (prepare a supported model, embed
a batch, report stable model/input/runtime facts); vector-corpus (build, open, reuse,
query a persisted declaration-vector corpus while reporting stable corpus/build/query
facts); search-workflow stage counters (vector-generated, symbolic-generated,
merged-generated, ranked, visible).

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
declaration-document policy id and version, vector dimension, normalization contract.
The schema version is owned by the vector-index crate. If any provenance fact changes,
the crate reports `stale` rather than letting search inspect database files.

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

Vector candidates merge with symbolic candidates before scoring. Existing symbolic pairs
gain vector facts when the nearest-neighbor query also finds them. Vector-only pairs are
generated and ranked so eval can measure recall, but they are not shown by default; vector
score is not a production visibility threshold.

### 35M design note: eligibility and top-k

`lean-dup-search` owns vector corpus and query eligibility. The hidden knowledge is Lean
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
- *Search owns named corpus/query eligibility policies.* Chosen: search already owns
  candidate policy, generated/private facts, actionability, and merge semantics. This
  keeps the public surface small and prevents temporal decomposition across search,
  embedding, vector-index, and eval.

The default hidden policy is `actionable-public-statement`. It excludes declarations with
stable reasons: `generated`, `private`, `synthetic`, `low-signal`, `missing-statement`,
`not-actionable`, and `unsupported-kind`. A named `broad` policy may include normally
excluded declarations for experiments, but the artifact must record that policy choice.
Saturated runs (`top_k >= eligible_corpus_size`) are smoke evidence only; they cannot
support vector-search quality claims.

Search controls declaration-document selection, per-query top-k, symbolic/vector merge
ordering, and the rule that vector-only candidates are ranked for measurement but not
shown by vector score alone. Eval passes a search-owned vector request; search returns
stable observation facts: vector summary, the five stage counters above, optional vector
score, optional vector rank. Model profile internals, FastEmbed runtime mechanics, final
model input strings, LanceDB storage, ANN parameters, vector database paths, table names,
and vector-cache filenames stay out of search, eval, and report artifacts.

Stage counters in hidden artifacts:

| Counter             | Meaning                                                     |
| ------------------- | ----------------------------------------------------------- |
| `vector_generated`  | pair produced by nearest-neighbor vector search             |
| `symbolic_generated`| pair produced by symbolic retrieval                         |
| `merged_generated`  | pair exists after the hidden search merge stage             |
| `ranked`            | pair survived into ranked observation facts                 |
| `visible`           | pair entered the shown queue                                |

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
- *Join only direct typed labels.* Rejected: legacy expanded clusters still define scoring denominators.
- *Eval-owned truth-preserving artifact builder.* Chosen: search supplies stage facts; eval supplies label truth and
  writes one stable row per unordered pair.

The artifact records label facts as `positive`, `hard-negative`, `expanded-positive`, `expanded-hard-negative`,
`skipped`, or `unlabeled`. A row can carry multiple label facts, for example a skipped hard-negative fact caused by a
positive-label conflict. Scoring denominators still come from the normalized positive and hard-negative label sets.

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

| Backend     | Status              | Rationale                                                                                  |
| ----------- | ------------------- | ------------------------------------------------------------------------------------------ |
| FastEmbed   | first runtime       | provides BGE-small as default text model and hides ONNX/tokenizer details behind a batch API |
| LanceDB     | first persistence   | embedded local filesystem path, Rust-native, persistent storage, metadata filtering, vector indexes |
| sqlite-vec  | fallback persistence | small, SQLite-shaped, fits beside existing cache; pre-v1 makes it a fallback not first choice |
| Qdrant      | future persistence  | strong for service deployments and extended filtering; first CLI path should not require a service |
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
