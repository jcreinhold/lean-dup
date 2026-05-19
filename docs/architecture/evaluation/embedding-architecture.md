# Embedding Architecture

This document records the architecture boundary for embedding experiments. Prompt 35A
added the `lean-dup-embedding` crate skeleton and workspace boundary. Prompt 35B added
explicit model acquisition and cache validation. Prompt 35C added the MiniLM/Candle CPU
runtime used for the rerank-only negative probe. Prompt 35G replaces that narrow runtime
assumption with private model profiles and a FastEmbed-backed BGE-small baseline.

For the current pipeline, see [end-to-end-architecture.md](../end-to-end-architecture.md).
For crate boundaries, see [crate-factoring.md](../crate-factoring.md).

## Design Note

Hidden knowledge: the embedding subsystem owns model profiles, model acquisition policy,
text embedding input policy, model/cache/runtime summaries, local CPU runtime facts, and
the rules for keeping embedding artifacts reproducible. It is also the place where model
download, tokenizer compatibility, tensor layout, pooling, normalization, batching,
query/document wrapping, backend selection, and vector cache decisions live.

Smallest public interface: the `lean-dup-embedding` crate root accepts explicit model
preparation requests and declaration-summary strings. It returns stable model, profile,
cache, acquisition-policy, input-role, vector-dimension, runtime-counter, and typed-error
facts.

Decisions that must not leak upward or sideways: Hugging Face/FastEmbed cache layout,
tokenizer/model filenames, Candle tensor shapes, FastEmbed enums, ONNX/ORT mechanics,
pooling and normalization details, query/document prefix strings, vector cache format,
download mechanics, runtime batching, and any model-specific fallbacks. Search, eval,
report, and CLI callers should not learn those details.

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
declaration-summary inputs and `lean-dup-eval` owns labels, experiment lifecycle, and
artifact comparison. This design is deeper because callers learn a small text-embedding
capability instead of Hugging Face, tokenizer, Candle, or cache internals.

For the CPU runtime boundary, two designs were considered. A high-level wrapper around a
generic embedding library would be easy to call, but it would hide tokenizer, pooling,
normalization, and cache decisions before `lean-dup` can measure whether they are right
for Lean declaration summaries. The chosen design is a dedicated runtime boundary inside
`lean-dup-embedding`: callers still see only `embed_text_batch`, while the crate keeps
tokenizer loading, BERT/MiniLM execution, attention-mask mean pooling, L2 normalization,
batching, and vector-cache layout private. This is deeper because the public interface
does not grow with each runtime mechanism.

For the model-profile boundary, three designs were considered. Keeping
`EmbeddingModelSpec { id, revision }` open-ended looks flexible, but it is a false
abstraction if every id reaches the same MiniLM/BERT runtime assumptions. Exposing
tokenizer, pooling, backend, prefix, and file choices to eval/search/CLI would make model
support a cross-repo edit. The chosen design is a private model-profile registry inside
`lean-dup-embedding`. Supported model ids resolve to profiles before acquisition or
runtime; unsupported ids fail early with the stable reason `unsupported-model-profile`.
Adding a model normally touches one profile definition. Adding a new architecture family
may also touch one backend adapter file, still inside `lean-dup-embedding`.

## Crate Contract

The crate is `lean-dup-embedding` at `crates/embedding`.

Public capability:

- embed batches of declaration-summary strings locally on CPU;
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
- input-policy version for declaration-summary text;
- batch embedding result with vector dimension and per-input outcome;
- runtime counters for load, tokenization, inference, cache hits/misses, and batch count;
- typed errors that callers can report without knowing model-file or tensor details.

Runtime public interface:

- `TextEmbeddingBatchRequest` names the model, the input-policy facts, declaration-summary
  texts, and optional model/vector cache roots for tests or hidden experiments;
- `embed_text_batch` validates that the model is already prepared and never downloads;
- `TextEmbeddingBatchResult` returns model/cache summaries, vector dimension, runtime
  counters, and normalized vectors in input order.

Private decisions:

- Hugging Face cache directory layout and file resolution;
- tokenizer/config/model filenames and validation rules;
- Candle tensor construction, attention masks, dtype, and device selection;
- pooling, L2 normalization, and batch sizing;
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

The previous `sentence-transformers/all-MiniLM-L6-v2` path is retained only as
`legacy-minilm-rerank-baseline`. It exists to reproduce Prompt 35E's negative rerank-only
evidence. It is not the default model and not the production extension point.

FastEmbed types, model enums, ONNX/ORT details, and cache layout are private to
`lean-dup-embedding`. The crate may use FastEmbed to acquire or run a supported profile,
but search, eval, report, and CLI see only stable profile/model/cache/runtime facts.

Prompt 35C runtime policy:

- model loading is CPU-only through `tokenizers`, Candle, and `safetensors`;
- only BERT-family sentence-transformer configs are accepted in this pass;
- pooling follows the model's sentence-transformers pooling config and currently accepts
  attention-mask mean pooling;
- vectors are L2-normalized before they cross the crate boundary;
- vector cache keys combine model fingerprint, embedding input-policy version, and a hash
  of the declaration-summary string;
- cache filenames, tensor shapes, token ids, raw tokenizer errors, and model-file paths
  stay private.

Prompt 35G makes required roles profile-derived. The FastEmbed-backed BGE profile reports
stable roles such as `runtime-model`, `config`, `tokenizer`, `tokenizer-config`, and
`special-tokens`, while FastEmbed-specific filenames remain private. The legacy MiniLM
profile reports `config`, `tokenizer`, `tokenizer-config`, `special-tokens`,
`pooling-config`, and `weights` only for historical negative-baseline reproducibility.
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

`lean-dup-search` may construct stable declaration-summary input strings from search-owned
facts, but it must not download models, read model environment variables, know tokenizer
metadata, or write embedding artifacts. Search should receive embedding scores or
embedding experiment facts through crate-root DTOs only when a hidden experiment asks for
them.

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
CLI preparation. It does not add tokenizer loading, Candle inference, vector caching,
search integration, eval artifact writing, or default audit behavior.

Prompt 35C replaces the unsupported batch-embedding path with a cache-only CPU runtime.
It does not add search integration, eval artifact writing, user-facing ranking changes,
or default audit behavior.

## CPU Evidence

Implementation evidence is intentionally modest in Prompt 35C and 35G. Unit tests cover
fake deterministic vectors, pooling, vector-cache keys, cache hits/misses, missing
prepared model handling, model-profile resolution, unsupported-model rejection, and
boundary rules. A legacy MiniLM real-model smoke test remains available but ignored by
default; prepare that historical model explicitly with:

```sh
cargo run -p lean-dup-cli -- embedding prepare --policy download-if-missing --model-id sentence-transformers/all-MiniLM-L6-v2
cargo test -p lean-dup-embedding -- --ignored prepared_legacy_minilm_model_produces_normalized_vectors_or_clean_skip
```

The smoke test checks the legacy model's 384-dimensional normalized output when local
files are prepared, and skips cleanly when they are not. The current default experiment
profile is BGE-small through FastEmbed; Prompt 35F and later vector-search prompts decide
whether that model helps candidate generation.

## Red Flag Review

- Shallow module: mitigated at the design level. The future public surface is a text
  embedding capability with stable summaries; runtime complexity stays inside the
  embedding crate.
- Pass-through wrapper: mitigated. The crate is not a facade over Hugging Face or
  Candle APIs; it hides model acquisition, local inference, pooling, normalization, and
  cache policy behind lean-dup-specific facts.
- Temporal decomposition: mitigated. Callers run one prepare capability for acquisition
  and one batch-embedding capability for runtime; they do not sequence tokenizer loading,
  model loading, inference, pooling, normalization, or cache writes themselves.
- Information leakage: mitigated by contract. Tokenizer files, tensor layout, model cache
  layout, and vector cache format are private decisions.
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
