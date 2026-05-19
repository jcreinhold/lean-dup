# Embedding Vector Search

Prompt 35F corrects the embedding direction before implementation. Prompts 35D and 35E
validated `sentence-transformers/all-MiniLM-L6-v2` as a hidden reranker over the already
observed symbolic candidate pool. That was a useful negative probe, but it did not test
the capability Lean-Dup needs for mathlib-scale search: per-declaration vector candidate
generation over a persisted comparison corpus that is built once, reopened, and reused
across audits.

Prompt 35I adds the vector-index crate boundary described here. It does not change the
default symbolic audit path, tune ranking thresholds, or make embeddings part of default
eval.

## Design Note

Hidden knowledge:

- `lean-dup-embedding` owns model profiles, model acquisition, model-specific input
  wrapping, CPU inference, vector normalization, runtime counters, and per-text vector
  cache policy.
- `lean-dup-vector-index` owns persisted declaration-vector corpora, vector database
  or ANN backend choice, corpus identity, index/cache invalidation, nearest-neighbor
  lookup, and vector-corpus diagnostics.
- `lean-dup-search` owns declaration document construction, vector candidate-generation
  policy, and merging vector candidates with symbolic candidates before scoring.
- `lean-dup-eval` owns labels, experiment lifecycle, vector-stage metrics, artifacts, and
  go/no-go decisions.
- `lean-dup-report` projects stable status and artifact facts without depending on model
  runtime or vector-index internals.
- `lean-dup-cli` owns hidden flags, explicit model/corpus preparation commands, and
  stdout/stderr/file behavior.

Smallest public interfaces:

- model-profile facade: prepare a supported model, embed a batch, and report stable
  model/input/runtime facts;
- vector-corpus facade: build, open, reuse, and query a persisted declaration-vector
  corpus while reporting stable corpus/build/query facts;
- search workflow facts: vector-generated, symbolic-generated, merged-generated, ranked,
  and visible stage counters.

Decisions that must not leak upward or sideways: tokenizer mechanics, pooling,
normalization, model-file layout, backend names, persistence format, ANN parameters,
database/index cache layout, raw declaration-document text, vector-cache filenames, and
vector database query syntax. Search, eval, report, and CLI callers should not learn
whether the current vector-index backend is LanceDB, sqlite-vec, Qdrant, HNSW, or a
future replacement.

Preserved capability: default symbolic duplicate auditing remains read-only,
deterministic, embedding-free, and authoritative unless a later validation prompt proves
that vector candidate generation should enter calibration.

Discarded Python-era behavior: ad hoc semantic-search experiments and anecdotal
"looks related" inspection are not accepted as evidence. The MiniLM rerank-only path is
also discarded as promotion evidence: it produced vectors and artifacts, but it did not
measure candidate-generation recall over a reusable corpus.

## Design It Twice

Rejected: keep the current rerank-only experiment and tune the model or input text. That
would improve an experiment that already asks the wrong question. Reranking only the
symbolic candidate pool cannot discover pairs missing from symbolic retrieval, so it
cannot measure vector candidate-generation recall.

Rejected: put vector corpus storage and nearest-neighbor lookup inside
`lean-dup-search`. Search already owns audit workflow, symbolic retrieval, semantic
evidence, ranking, and visibility policy. Adding persistence format, vector database
selection, ANN parameters, and corpus invalidation would mix search policy with storage
mechanics and make every backend change a search change.

Rejected: rebuild an in-memory vector graph on every search run. In-memory HNSW can be a
useful algorithm inside a backend, but rebuilding vectors or ANN structures per audit is
a mathlib-scale performance bug. Persistent corpus identity and reuse are part of the
core abstraction, not an optimization to add later.

Rejected: require a local vector database service such as Qdrant for the first CLI
backend. Qdrant is strong for service deployments and may become a future backend behind
the vector-index facade, but requiring a service or Docker path in the first local CLI
experiment would make the default developer workflow heavier than the capability needs.

Chosen: keep `lean-dup-embedding` as the model-profile facade, add a
`lean-dup-vector-index` corpus facade, and let `lean-dup-search` own the hybrid candidate
policy. This is deeper because each crate hides a volatile decision: model/runtime
compatibility in embedding, persisted vector storage in vector-index, and search policy
in search. The caller-facing surface stays small and stable even if model families,
database backends, or candidate merge policy change.

## POSD Diagnosis Of The Current Design

`EmbeddingModelSpec { id, revision }` is currently a false abstraction. It appears to
accept arbitrary model identities, but the implementation still assumes a private
BERT/MiniLM runtime shape. Prompt 35G must replace that with private model profiles. A
supported model should require one model-profile edit, and only a new architecture family
should require one backend adapter edit inside `lean-dup-embedding`.

The current declaration-summary input policy leaks internal feature labels into a
sentence embedding model. That may be useful for an artifact, but it is not an
in-distribution document/query contract. Prompt 35H must define search-owned declaration
documents and let the embedding crate apply model-specific query/document prefixes.

The rerank-only experiment from Prompts 35D and 35E was a negative probe, not vector
search validation. It measured whether MiniLM cosine similarity could reorder already
observed symbolic candidates. It did not measure whether vectors can recover candidates
that symbolic retrieval missed.

Rebuilding vectors or ANN structures per run is not an implementation detail. For
mathlib-scale search, corpus persistence, provenance, reuse, and invalidation are part of
the public capability. The facade should expose stable corpus facts, not backend storage
mechanics.

Adding models must not require edits across search, eval, report, and CLI. Search should
talk about declaration documents and candidate facts; eval should talk about labels and
metrics; report should talk about status and artifact paths. Model-specific behavior
belongs in the embedding crate's profile registry and backend adapters.

## Target Architecture

`lean-dup-embedding` exposes model-profile operations. Profiles own model id, backend
family, dimension, input role behavior, max length, normalization expectation, and cache
requirements. Runtime code currently uses FastEmbed; a later backend is allowed only
behind the same profile facade. Callers see only stable preparation, embedding, and
runtime facts.

`lean-dup-vector-index` exposes corpus operations. It builds or opens a persisted
declaration-vector corpus identified by source corpus provenance, embedding model
fingerprint, input policy version, vector dimension, and vector-index schema version. It
queries nearest declaration candidates and reports build/query counters. It hides backend
names, index parameters, persistence layout, and query syntax.

`lean-dup-search` creates declaration documents and controls hidden vector
candidate-generation policy. It asks embedding for vectors and vector-index for nearest
neighbors through crate-root APIs only. It then merges vector and symbolic candidates
before scoring, while preserving deterministic ranking and default symbolic behavior
unless hidden vector experiment flags are used.

`lean-dup-eval` measures the stages separately: vector-generated, symbolic-generated,
merged-generated, ranked, and visible. It owns labels, artifacts, and the decision about
whether vector facts may enter later threshold calibration.

`lean-dup-report` projects only stable status, paths, and stage facts. It must not
recompute vector search, inspect model profiles, or mention backend-specific storage.

`lean-dup-cli` provides explicit hidden preparation and experiment commands. Normal
`audit`, `doctor`, `show`, `diff`, and ordinary `eval` do not prepare models, build
vector corpora, open vector databases, or require embedding dependencies at runtime.

## Prompt 35I Vector-Corpus Contract

`lean-dup-vector-index` exposes only declaration-corpus operations:

- build or reuse a persisted corpus from declaration identities, metadata, content
  hashes, and normalized vectors;
- open a previously built corpus only when schema and provenance match;
- query nearest declarations and return stable declaration facts with scores where higher
  means closer;
- report corpus status as built, reused, missing, stale, or unusable.

The public provenance includes the source corpus fingerprint, embedding model profile and
fingerprint, declaration-document policy id/version, vector dimension, and normalization
contract. The schema version is owned by the vector-index crate. If any provenance fact
changes, the crate reports stale state instead of letting search inspect database files.

The implementation uses LanceDB privately for the first persistent local backend. The
crate also enables vendored protobuf support for Lance dependencies, because the local
toolchain did not have `protoc` on `PATH` during the first build attempt. This keeps the
backend self-contained inside the vector-index crate rather than adding a machine-level
operator prerequisite.

Fixture corpora may use exact backend search when they are too small for an ANN index to
be useful. Production-sized corpora should create the backend vector index during corpus
build. This distinction is private: callers receive the same corpus summary and nearest
declaration facts either way.

## Backend Tradeoff

FastEmbed is a plausible first embedding runtime path because current Rust docs list
`BAAI/bge-small-en-v1.5` as the default text embedding model, list Qwen3 embedding
models behind an explicit feature, and hide tokenizer/runtime details behind a batch
embedding API. The backend still belongs behind `lean-dup-embedding` profiles because
FastEmbed's model set, ONNX/ORT mechanics, and prefix conventions are runtime details,
not search or eval facts. Source: <https://docs.rs/crate/fastembed/latest>.

LanceDB is the first persistent local vector database candidate because it supports an
embedded local filesystem path, Rust, persistent storage, metadata filtering, and vector
indexes. This matches Lean-Dup's local CLI workflow better than requiring a service for
the first backend. The risk is dependency weight and index tuning, so Prompt 35I must
validate build profile and local persistence before treating it as accepted. Sources:
<https://docs.rs/lancedb/latest/lancedb/index.html>,
<https://docs.lancedb.com/quickstart>, and
<https://docs.lancedb.com/indexing/vector-index>.

sqlite-vec is a fallback candidate because it is small, SQLite-shaped, and easy to fit
beside the existing cache story. Its current pre-v1 status and extension shape make it a
fallback rather than the first architecture assumption. Source:
<https://github.com/asg017/sqlite-vec>.

Qdrant remains a future backend candidate behind the vector-index facade. It is a strong
vector database for service/server deployments and extended filtering, but the first
Lean-Dup CLI path should not require a local service. If later workloads need service
scale, multi-client access, or Qdrant-specific filtering, the vector-index facade should
absorb that change without search/eval/report API churn. Source:
<https://qdrant.tech/documentation/quick-start/>.

HNSW remains relevant as an ANN algorithm, but an in-memory-only HNSW graph is rejected
as the default architecture because the corpus must be persistent and reusable. A backend
may use HNSW internally, but `hnsw_rs` or any HNSW vocabulary must not leak outside
`lean-dup-vector-index`. Source:
<https://docs.rs/hnsw_rs/latest/hnsw_rs/hnsw/index.html>.

## Invalidation And Provenance

A vector corpus is reusable only when its provenance matches the requested workload. The
identity must include at least:

- source corpus identity and declaration handles;
- embedding model profile and model fingerprint;
- declaration-document input policy version;
- vector dimension and normalization contract;
- vector-index schema version and backend-compatible build facts.

If any of these facts change, the vector-index crate should report the corpus as missing
or stale and let the hidden preparation workflow rebuild it. Search should not inspect
backend-specific manifests or decide whether a persisted index can be reused.

## Red Flag Review

- Shallow module: mitigated. The proposed vector-index crate hides persistence, ANN, and
  invalidation mechanics behind a corpus facade rather than exposing backend operations.
- Pass-through wrapper: mitigated by design. `lean-dup-vector-index` must provide corpus
  build/open/query facts, not re-export LanceDB, sqlite-vec, Qdrant, or HNSW APIs.
- Temporal decomposition: mitigated. The boundary is organized by hidden knowledge
  instead of build, embed, index, query, and report steps.
- Information leakage: mitigated. Model profiles, backend names, ANN parameters, vector
  cache layout, and raw document text each have one owning crate.
- Special-general mixture: mitigated. Lean-Dup-specific declaration documents stay in
  search; general vector persistence stays in vector-index; model runtime stays in
  embedding.
- Conjoined methods: mitigated. Preparing models, building corpora, querying candidates,
  and evaluating metrics are not exposed as one public workflow object.
- Hard-to-describe public API: mitigated. The intended surfaces are model-profile,
  vector-corpus, and stage-fact facades with stable facts.
- Implementation details contaminating interface comments: mitigated. Backend names are
  recorded as architecture evidence only and must not become search/eval/report interface
  comments.
