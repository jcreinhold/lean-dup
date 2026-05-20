# Embedding Architecture

`lean-dup-embedding` is the only crate that may download, load, or run a text-embedding
model. Other crates see stable model, profile, cache, runtime, and typed-error facts; they
never see tokenizer fields, tensor layout, model filenames, Hugging Face cache layout,
FastEmbed enums, ONNX/ORT mechanics, normalization rules, query/document prefixes, vector
cache format, or download mechanics.

The default audit path (`audit`, `doctor`, `show`, `diff`, ordinary `eval`) does not call
the embedding crate. The hidden `embedding prepare` command and hidden embedding
experiments are the only callers.

For the pipeline that consumes these facts, see
[end-to-end-architecture.md](../end-to-end-architecture.md). For crate boundaries, see
[crate-factoring.md](../crate-factoring.md).

## Crate contract

Crate: `lean-dup-embedding` at `crates/embedding`.

Public capability:

- embed batches of declaration-document strings locally on CPU;
- return deterministic vectors and stable runtime facts;
- report model and cache readiness without forcing the default audit path to download or
  load anything.

### Public surface

| Surface                          | Role                                                                            |
| -------------------------------- | ------------------------------------------------------------------------------- |
| `EmbeddingAcquisitionPolicy`     | `cache-only` or `download-if-missing`                                            |
| `EmbeddingPrepareRequest`        | model spec, acquisition policy, optional cache-root override                     |
| `prepare_embedding_model`        | validates the local cache; downloads only under `download-if-missing`            |
| `EmbeddingPrepareResult`         | model/cache status, elapsed time, file-role status, byte counts, stable reasons |
| `TextEmbeddingBatchRequest`      | model, input role, input-policy facts, document texts, optional cache roots     |
| `embed_text_batch`               | validates that the model is already prepared; never downloads                   |
| `TextEmbeddingBatchResult`       | model/cache summaries, vector dimension, runtime counters, normalized vectors   |

Public facts include model identity (id, resolved revision), profile identity (backend
family, vector dimension, supported input roles), model/cache status (prepared, missing,
unusable, skipped), required file-role status, input-policy id and version, and runtime
counters for model load, inference, cache hits/misses, and batch count.

### Private decisions

Hugging Face/FastEmbed cache directory layout and file resolution; tokenizer/config/model
filenames; ONNX/ORT/FastEmbed initialization; L2 normalization and batch sizing; vector
cache key ingredients and on-disk format; download retry and cache-only/download-if-missing
mechanics; model-specific compatibility shims.

## Model profiles

The profile registry is the extension point. A profile owns supported model id and
revision policy, backend family, vector dimension, max token length, supported input roles,
query/document wrapping, normalization expectation, acquisition strategy, and runtime
support status.

Current default: `bge-small-en-v1.5` (FastEmbed, `BAAI/bge-small-en-v1.5`, backend family
`fastembed`, dimension 384, roles `document` and `query`). The embedding crate applies
FastEmbed/BGE-specific wrapping internally; callers do not pass `query:` or `passage:`
strings as policy.

`sentence-transformers/all-MiniLM-L6-v2` is no longer a supported profile. Its only
remaining trace is the historical record in [embedding-validation.md](embedding-validation.md).

Required file roles are profile-derived. The BGE profile reports stable roles
`runtime-model`, `config`, `tokenizer`, `tokenizer-config`, and `special-tokens`;
FastEmbed-specific filenames remain private.

Runtime policy: CPU-only through FastEmbed; supported model ids resolve through private
profiles before acquisition or embedding; vectors are L2-normalized before they cross the
crate boundary; vector cache keys combine model fingerprint, embedding input-policy
version, input role, and a hash of the model-wrapped document string; cache filenames,
token ids, raw runtime errors, ONNX details, and model-file paths stay private.

## Acquisition policy

| Policy                 | Effect                                                                    |
| ---------------------- | ------------------------------------------------------------------------- |
| `cache-only`           | validates already-prepared files; never calls a download API              |
| `download-if-missing`  | may fetch missing required files; only when an operator opts in           |

The embedding crate resolves the Hugging Face cache root in the order: explicit request
override, `HF_HUB_CACHE`, `HF_HOME/hub`, `hf-hub` default. Public reports may show the
resolved cache root for operator diagnostics; they do not expose repository folder names,
snapshot hashes, blob paths, or individual model filenames.

## Boundary with search, eval, report, and CLI

`lean-dup-search` constructs declaration documents from search-owned facts. It must not
download models, read model environment variables, know tokenizer metadata or model
prefixes, or write embedding artifacts. The default vector-search policy is
`name-and-statement`. Other stable policies are `statement`, `definition-aware`, and
`docstring-augmented`. The default excludes retrieval feature families, ranking facts,
semantic obligations, SQLite details, proof bodies, and worker protocol fields.

A search-owned declaration document carries: declaration name, module name, declaration
kind, normalized statement/signature text, optional docstring text, optional definition
body summary, stable document-policy id and version, content availability counters, and a
privacy-safe content hash. Search keeps these out of normal JSON. Hidden eval may ask
search for plain document text; artifacts record policy ids, availability counters, and
content hashes rather than raw statements, body text, docstrings, or final model-formatted
input.

`lean-dup-embedding` owns role wrapping. Its public request names `document` or `query`;
the profile decides whether the role requires a prefix or instruction. Search, eval,
report, and CLI must not contain strings such as BGE query/document prefixes.

`lean-dup-eval` owns labels, suite selection, hidden experiment lifecycle, and artifact
writing. It chooses acquisition policy for hidden experiments. Ordinary eval remains
symbolic.

`lean-dup-report` may project optional embedding experiment status or artifact paths after
later prompts add them. It must not recompute embedding scores or inspect model internals.

`lean-dup-cli` owns user-visible and hidden flags. Normal `audit`, `show`, `diff`,
`doctor`, and ordinary `eval` must not download or require embedding models. The hidden
`embedding prepare` command is the only explicit model-acquisition surface today.

## Design alternatives considered

- *Embedding runtime inside `lean-dup-search`.* Rejected: would mix search policy with
  model download, tokenizer loading, tensor inference, pooling, and vector cache layout.
- *Embedding crate in a separate Markdown-tooling workspace.* Rejected: that workspace has
  no Lean-duplicate-audit caller. Extraction waits for a second real product caller.
- *Require a preexisting `LEAN_DUP_EMBEDDING_MODEL` directory as the only input.* Rejected:
  pushes offline-deployment inconvenience onto every experiment without a reliable
  preparation path.
- *Hand-owned tokenizer/model/pooling/tensor code.* Rejected: makes the first probe's
  BERT/MiniLM assumptions look like a general embedding subsystem. The FastEmbed
  profile-resolved runtime keeps model-specific mechanics private while callers see only
  `embed_text_batch`.
- *Open-ended `EmbeddingModelSpec { id, revision }` exposed to callers.* Rejected: a false
  abstraction if every id reaches the same MiniLM/BERT runtime assumptions, and exposing
  tokenizer/pooling/backend/prefix/file choices would make model support a cross-repo edit.
  The private model-profile registry resolves supported ids before acquisition or runtime;
  unsupported ids fail early with `unsupported-model-profile`.
- *Search emits final model-input strings,* or *embedding constructs all text.* Rejected:
  the first leaks BGE/FastEmbed prefix policy into search; the second pushes Lean
  declaration semantics, retrieval keys, and ranking facts into the model crate. Search
  provides structured documents; embedding applies model-profile-specific wrapping.

## CPU evidence

Unit tests cover vector-cache keys, cache hits/misses, missing-prepared-model handling,
BGE profile resolution, unsupported-model rejection, and boundary rules. The previous
MiniLM/Candle rerank artifact is kept only as historical negative evidence in
[embedding-validation.md](embedding-validation.md). The current BGE-small/FastEmbed
profile is what later vector-search work measures.

## 35T Model and Format Selection

The current model-selection decision lives in
[embedding-model-selection.md](embedding-model-selection.md). Prompt 35U should keep
`bge-small-en-v1.5` as the default hidden baseline, add only the same-family
`bge-base-en-v1.5` profile, and add the role-format variants `symmetric-document` and
`asymmetric-query-document`.

That decision is deliberately narrow. It tests two concrete risks without exposing model
runtime details outside this crate: BGE-small may be underpowered for semantic declaration
documents, and the current query/document wrapping may be wrong for declaration-to-
declaration retrieval. Search, eval, report, and CLI should see only stable profile ids,
role-format ids, dimensions, model/cache/runtime facts, and validation denominators.
FastEmbed enum names, prefix strings, ONNX/ORT details, tokenizer files, model paths, and
feature flags remain private implementation details of `lean-dup-embedding`.

## Red-flag checklist

- *Shallow module:* the public surface is a text-embedding capability with stable
  summaries; runtime complexity stays inside the crate.
- *Pass-through wrapper:* the crate is not a facade over Hugging Face or FastEmbed APIs;
  it hides acquisition, inference, normalization, wrapping, and cache policy behind
  lean-dup-specific facts.
- *Temporal decomposition:* callers run one prepare capability and one batch-embedding
  capability; they do not sequence tokenizer loading, model loading, inference,
  normalization, or cache writes.
- *Information leakage:* tokenizer files, runtime internals, cache layout, model prefixes,
  raw document text in artifacts, and vector cache format are private.
- *Special-general mixture:* the crate lives in the `lean-dup` workspace because lean-dup
  is the only caller; extraction waits for a second product caller.
- *Conjoined methods:* search document construction and eval artifact comparison live
  outside the embedding runtime.
- *Hard-to-describe public API:* local text embedding for declaration summaries plus stable
  model/cache/runtime facts.
- *Implementation details in interface comments:* this document names caller obligations
  and hidden decisions, not tokenizer fields, tensor shapes, or cache filenames.
