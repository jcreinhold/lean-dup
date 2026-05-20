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
