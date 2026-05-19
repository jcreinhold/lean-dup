# Embedding Architecture

This document records the architecture boundary for embedding experiments. Prompt 35A
added the `lean-dup-embedding` crate skeleton and workspace boundary. Prompt 35B added
explicit model acquisition and cache validation. Prompt 35C tried a narrow
MiniLM/Candle CPU runtime for the rerank-only probe. Prompt 35G replaced that mistaken
extension point with private model profiles and a FastEmbed-backed BGE-small baseline.
Because this code is pre-release, the legacy MiniLM/Candle implementation was removed
instead of preserved for compatibility; historical results remain in
[embedding-validation.md](embedding-validation.md).

For the current pipeline, see [end-to-end-architecture.md](../end-to-end-architecture.md).
For crate boundaries, see [crate-factoring.md](../crate-factoring.md).

## Design Note

Hidden knowledge: the embedding subsystem owns model profiles, model acquisition policy,
text embedding input policy, model/cache/runtime summaries, local CPU runtime facts, and
the rules for keeping embedding artifacts reproducible. It is also the place where model
download, tokenizer compatibility, runtime selection, normalization, batching,
query/document wrapping, backend selection, and vector cache decisions live.

Smallest public interface: the `lean-dup-embedding` crate root accepts explicit model
preparation requests and declaration-document text with a stable input role. It returns stable model, profile,
cache, acquisition-policy, input-role, vector-dimension, runtime-counter, and typed-error
facts.

Decisions that must not leak upward or sideways: Hugging Face/FastEmbed cache layout,
tokenizer/model filenames, FastEmbed enums, ONNX/ORT mechanics, normalization details,
query/document prefix strings, vector cache format, download mechanics, runtime batching,
and any model-specific fallbacks. Search, eval, report, and CLI callers should not learn
those details.

Preserved capability: the default `lean-dup` auditor remains read-only, local,
deterministic, symbolic, and independent of embedding models. Existing audit, doctor,
show, diff, eval, JSON, cache, ranking, and semantic-probe behavior remains authoritative.
Only the hidden `embedding prepare` developer command may acquire embedding model files.

Discarded Python-era behavior: ad hoc semantic-search experiments, manual model setup
notes, and anecdotal "looks related" inspection are not evidence. Embedding work must
produce explicit artifacts with model identity, input policy, quality metrics, runtime,
memory, cache state, and hard-negative impact.

## Design It Twice

Rejected: put embedding runtime code inside `lean-dup-search`. Search owns declaration
summary construction, candidate observation, symbolic scoring, semantic evidence, and
visibility policy. If it also owns model download, tokenizer loading, tensor inference,
pooling, and vector cache layout, then search becomes a mixed search-and-ML-runtime
module. That would violate the crate-factoring rule that volatile mechanism decisions
stay behind the crate that owns them.

Rejected: put the embedding crate in `/Users/jcreinhold/Code/mdwright`. `mdwright` is a
Markdown linter/formatter workspace. A Lean duplicate-audit embedding runtime would
couple unrelated products and make release, dependency, cache, and performance policy
harder to reason about. If the embedding runtime later gets a second real product
caller, it can be extracted to a standalone workspace with evidence.

Rejected: require a preexisting `LEAN_DUP_EMBEDDING_MODEL` directory as the only model
input. That design copies the inconvenience of offline deployment into the experiment
without giving users a reliable sentence-transformers-like preparation path. The right
boundary is explicit model acquisition and validation owned by the embedding model
manager, not an environment variable that every caller must understand.

Chosen: keep `lean-dup-embedding` inside the `lean-dup` workspace. The crate is consumed
through crate-root APIs only. It owns model acquisition, local CPU embedding, and vector
cache policy, while `lean-dup-search` supplies stable
declaration documents and `lean-dup-eval` owns labels, experiment lifecycle, and
artifact comparison. This design is deeper because callers learn a small text-embedding
capability instead of Hugging Face, FastEmbed, ONNX, tokenizer, or cache internals.

For the CPU runtime boundary, two designs were considered. Hand-owning tokenizer, model,
pooling, and tensor code looked controllable, but it made the first probe's BERT/MiniLM
assumptions look like a general embedding subsystem. The replacement design is a
profile-resolved FastEmbed runtime boundary inside `lean-dup-embedding`: callers still
see only `embed_text_batch`, while the crate keeps model-specific runtime mechanics,
normalization, wrapping, batching, and vector-cache layout private. This is deeper
because the public interface does not grow with each runtime mechanism.

For the model-profile boundary, three designs were considered. Keeping
`EmbeddingModelSpec { id, revision }` open-ended looks flexible, but it is a false
abstraction if every id reaches the same MiniLM/BERT runtime assumptions. Exposing
tokenizer, pooling, backend, prefix, and file choices to eval/search/CLI would make model
support a cross-repo edit. The chosen design is a private model-profile registry inside
`lean-dup-embedding`. Supported model ids resolve to profiles before acquisition or
runtime; unsupported ids fail early with the stable reason `unsupported-model-profile`.
Adding a model normally touches one profile definition. Adding a new architecture family
may also touch one backend adapter file, still inside `lean-dup-embedding`.

For the declaration-document boundary, three designs were considered. Letting search emit
final model input strings would leak BGE/FastEmbed prefix policy into search. Letting
embedding construct all text would make the model crate understand Lean declaration
semantics, retrieval keys, and ranking facts. The chosen design is structured declaration
documents owned by search, plus embedding-owned role wrapping. Search provides names,
formal statements, optional informal text, stable policy ids, and content hashes;
embedding applies model-profile-specific query/document formatting privately.

## Crate Contract

The crate is `lean-dup-embedding` at `crates/embedding`.

Public capability:

- embed batches of declaration-document strings locally on CPU;
- return deterministic vectors and stable runtime facts for evaluation artifacts;
- report model/cache readiness without forcing normal audit to download or load models.

Model acquisition public interface:

- `EmbeddingAcquisitionPolicy` names whether an explicit preparation request is
  `cache-only` or `download-if-missing`;
- `EmbeddingPrepareRequest` combines the model spec, acquisition policy, and optional
  cache root override for tests/developer isolation;
- `prepare_embedding_model` validates the local cache and downloads missing required files
  only under `download-if-missing`;
- `EmbeddingPrepareResult` reports stable model/cache status, elapsed time, required
  file-role status, byte counts where known, and stable reasons.

Public facts:

- model identity, including model id and resolved revision when available;
- profile identity, backend-family label, vector dimension, and stable input roles;
- model/cache status, including prepared, missing, unusable, or skipped states;
- acquisition policy used for explicit preparation;
- required model-file roles and their present/missing/downloaded/unavailable state;
- input-policy id and version for declaration-document text;
- batch embedding result with vector dimension and per-input outcome;
- runtime counters for model load, inference, cache hits/misses, and batch count;
- typed errors that callers can report without knowing model-file or runtime details.

Runtime public interface:

- `TextEmbeddingBatchRequest` names the model, the input role, the input-policy facts,
  declaration-document texts, and optional model/vector cache roots for tests or hidden
  experiments;
- `embed_text_batch` validates that the model is already prepared and never downloads;
- `TextEmbeddingBatchResult` returns model/cache summaries, vector dimension, runtime
  counters, and normalized vectors in input order.

Private decisions:

- Hugging Face/FastEmbed cache directory layout and file resolution;
- tokenizer/config/model filenames and validation rules;
- ONNX/ORT/FastEmbed initialization details;
- L2 normalization and batch sizing;
- vector cache key ingredients and on-disk format;
- download retry, cache-only, and download-if-missing mechanics;
- model-specific compatibility shims.

## Model Profiles

Prompt 35G makes the profile registry the extension point. A profile owns:

- supported model id and optional revision policy;
- backend family;
- vector dimension;
- max token length;
- supported input roles;
- query/document wrapping behavior;
- normalization expectation;
- acquisition/readiness strategy;
- runtime support status.

The current default experiment profile is `bge-small-en-v1.5`, backed by FastEmbed and
model id `BAAI/bge-small-en-v1.5`. It exposes the stable backend-family label
`fastembed`, dimension `384`, and input roles `document` and `query`. The embedding crate
applies FastEmbed/BGE-specific wrapping internally; callers do not pass `query:` or
`passage:` strings as policy.

The previous `sentence-transformers/all-MiniLM-L6-v2` implementation is removed from the
codebase. It remains only as historical evidence in Prompt 35E's validation document. It
is not a supported profile, not the default model, and not an extension point.

FastEmbed types, model enums, ONNX/ORT details, and cache layout are private to
`lean-dup-embedding`. The crate may use FastEmbed to acquire or run a supported profile,
but search, eval, report, and CLI see only stable profile/model/cache/runtime facts.

Current runtime policy:

- model loading is CPU-only through FastEmbed;
- supported model ids resolve through private profiles before acquisition or embedding;
- vectors are L2-normalized before they cross the crate boundary;
- vector cache keys combine model fingerprint, embedding input-policy version, input role,
  and a hash of the model-wrapped declaration-document string;
- cache filenames, token ids, raw runtime errors, ONNX details, and model-file paths stay
  private.

Prompt 35G makes required roles profile-derived. The FastEmbed-backed BGE profile reports
stable roles such as `runtime-model`, `config`, `tokenizer`, `tokenizer-config`, and
`special-tokens`, while FastEmbed-specific filenames remain private.
Callers must not depend on private filenames, snapshot paths, or FastEmbed cache layout.

## Acquisition Policy

`cache-only` validates already-prepared files and never calls a download API. It is the
policy for automated tests and future offline experiment checks.

`download-if-missing` may fetch missing required files from Hugging Face, but only when
an operator explicitly runs hidden preparation or a later hidden embedding experiment
chooses that policy. Normal `audit`, `doctor`, `show`, `diff`, and ordinary `eval` do
not call the embedding crate, so they cannot acquire models as a side effect.

The embedding crate hides how it resolves the Hugging Face cache root. The current
precedence is explicit request cache root, `HF_HUB_CACHE`, `HF_HOME/hub`, then the
`hf-hub` default. Public reports may show the resolved cache root for operator
diagnostics, but they do not expose repository folder names, snapshot hashes, blob paths,
or individual model filenames.

## Boundary With Search, Eval, Report, And CLI

`lean-dup-search` constructs structured declaration documents from search-owned facts, but
it must not download models, read model environment variables, know tokenizer metadata,
know model prefixes, or write embedding artifacts. The default vector-search policy is
`name-and-formal-statement`: declaration name plus normalized formal statement. Other
stable policies are `formal-statement`, `informal-or-formal`, and `legacy-rerank-v1`.
The default policy deliberately excludes retrieval feature families, ranking facts,
semantic obligations, SQLite details, and worker protocol fields.

Search-owned declaration documents contain:

- declaration name;
- module name;
- declaration kind;
- normalized formal statement text;
- optional informal/docstring text when a future worker/index surface provides it;
- stable document policy id and version;
- a privacy-safe content hash for artifacts.

Search keeps these documents out of normal JSON. Hidden eval may ask search for plain
document text, but artifacts record policy ids and content hashes rather than raw formal
statements or final model-formatted input.

`lean-dup-embedding` owns role wrapping. Its public request names `document` or `query`;
the profile code decides whether that role requires a prefix, instruction, or no wrapping.
Search, eval, report, and CLI must not contain strings such as BGE query/document prefixes.

`lean-dup-eval` owns labels, suite selection, hidden experiment lifecycle, and artifact
writing. It chooses acquisition policy for hidden experiments: cache-only or explicitly
download-if-missing. Ordinary eval remains symbolic and does not download or load models.

`lean-dup-report` may project optional embedding experiment status or artifact paths after
future prompts add them. It must not recompute embedding scores or inspect model/runtime
details.

`lean-dup-cli` owns user-visible and hidden command flags. Normal `audit`, `show`,
`diff`, `doctor`, and ordinary `eval` must not download or require embedding models.
The hidden `embedding prepare` command is the first explicit model-acquisition surface.
Hidden experiment commands added later may use the same explicit policy object.

## Prompt 35A And 35B Scope

Prompt 35A created the architecture document, the initial `lean-dup-embedding` crate
boundary, and boundary tests that keep ML runtime dependencies out of the rest of the
workspace. The crate may expose skeleton request/result/error DTOs, but embedding model
acquisition and inference remain unsupported during Prompt 35A.

Prompt 35B adds explicit model acquisition and validation through `hf-hub`, plus hidden
CLI preparation. It does not add runtime inference, vector caching, search integration,
eval artifact writing, or default audit behavior.

Prompt 35C replaces the unsupported batch-embedding path with a cache-only CPU runtime.
It does not add search integration, eval artifact writing, user-facing ranking changes,
or default audit behavior.

## CPU Evidence

Implementation evidence is intentionally modest in Prompt 35G. Unit tests cover
vector-cache keys, cache hits/misses, missing prepared model handling, BGE profile
resolution, unsupported-model rejection, and boundary rules. Prompt 35E's MiniLM
rerank-only artifact remains historical negative evidence; no runnable legacy MiniLM
runtime is kept in the pre-release codebase. The current default experiment profile is
BGE-small through FastEmbed; Prompt 35F and later vector-search prompts decide whether
that model helps candidate generation.

## Red Flag Review

- Shallow module: mitigated at the design level. The future public surface is a text
  embedding capability with stable summaries; runtime complexity stays inside the
  embedding crate.
- Pass-through wrapper: mitigated. The crate is not a facade over Hugging Face or
  FastEmbed APIs; it hides model acquisition, local inference, normalization, wrapping,
  and cache policy behind lean-dup-specific facts.
- Temporal decomposition: mitigated. Callers run one prepare capability for acquisition
  and one batch-embedding capability for runtime; they do not sequence tokenizer loading,
  model loading, inference, normalization, or cache writes themselves.
- Information leakage: mitigated by contract. Tokenizer files, runtime internals, model
  cache layout, model prefixes, raw document text in artifacts, and vector cache format
  are private decisions.
- Special-general mixture: mitigated. `lean-dup-embedding` lives in the `lean-dup`
  workspace because `lean-dup` is the only current caller; extraction waits for a real
  second product caller.
- Conjoined methods: residual risk deferred. Search summary construction and eval
  artifact comparison remain outside the embedding runtime and are still enforced by
  later prompt boundaries.
- Hard-to-describe public API: mitigated. The public story is local text embedding for
  declaration summaries plus stable model/cache/runtime facts.
- Implementation details contaminating interface comments: mitigated. This document names
  caller obligations and hidden decisions, not specific tokenizer fields, tensor shapes,
  or cache filenames.
