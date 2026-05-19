# Embedding Architecture

This document records the architecture boundary for embedding experiments. Prompt 35A
added the `lean-dup-embedding` crate skeleton and workspace boundary. Prompt 35B adds
explicit model acquisition and cache validation. Tokenizer loading, Candle inference,
vector cache, search integration, and eval artifact mode remain later work.

For the current pipeline, see [end-to-end-architecture.md](../end-to-end-architecture.md).
For crate boundaries, see [crate-factoring.md](../crate-factoring.md).

## Design Note

Hidden knowledge: the embedding subsystem owns model acquisition policy, text embedding
input policy, model/cache/runtime summaries, local CPU runtime facts, and the rules for
keeping embedding artifacts reproducible. It is also the place where model download,
tokenizer compatibility, tensor layout, pooling, normalization, batching, and vector cache
decisions live or will live.

Smallest public interface: the `lean-dup-embedding` crate root accepts explicit model
preparation requests and, after Prompt 35C, declaration-summary strings. It returns stable
model, cache, acquisition-policy, input-policy, vector-dimension, runtime-counter, and
typed-error facts.

Decisions that must not leak upward or sideways: Hugging Face cache layout,
tokenizer/model filenames, Candle tensor shapes, pooling and normalization details,
vector cache format, download mechanics, runtime batching, and any model-specific
fallbacks. Search, eval, report, and CLI callers should not learn those details.

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
- model/cache status, including prepared, missing, unusable, or skipped states;
- acquisition policy used for explicit preparation;
- required model-file roles and their present/missing/downloaded/unavailable state;
- input-policy version for declaration-summary text;
- batch embedding result with vector dimension and per-input outcome;
- runtime counters for load, tokenization, inference, cache hits/misses, and batch count;
- typed errors that callers can report without knowing model-file or tensor details.

Private decisions:

- Hugging Face cache directory layout and file resolution;
- tokenizer/config/model filenames and validation rules;
- Candle tensor construction, attention masks, dtype, and device selection;
- pooling, L2 normalization, and batch sizing;
- vector cache key ingredients and on-disk format;
- download retry, cache-only, and download-if-missing mechanics;
- model-specific compatibility shims.

Prompt 35B required roles for the first model family are stable at the public boundary:
`config`, `tokenizer`, `tokenizer-config`, `special-tokens`, `pooling-config`, and
`weights`. Their private filenames currently correspond to the default
`sentence-transformers/all-MiniLM-L6-v2` layout, preferring `model.safetensors` for
weights. Callers must not depend on those filenames or snapshot paths.

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

## Red Flag Review

- Shallow module: mitigated at the design level. The future public surface is a text
  embedding capability with stable summaries; runtime complexity stays inside the
  embedding crate.
- Pass-through wrapper: mitigated. The crate is not a facade over Hugging Face or
  Candle APIs; it hides model acquisition, local inference, pooling, normalization, and
  cache policy behind lean-dup-specific facts.
- Temporal decomposition: partially mitigated. Callers run one prepare capability instead
  of "check cache, download files, validate files" themselves. Prompt 35C will fold
  tokenize/infer/pool/normalize/cache into the same crate boundary.
- Information leakage: mitigated by contract. Tokenizer files, tensor layout, model cache
  layout, and vector cache format are private decisions.
- Special-general mixture: mitigated. `lean-dup-embedding` lives in the `lean-dup`
  workspace because `lean-dup` is the only current caller; extraction waits for a real
  second product caller.
- Conjoined methods: residual risk deferred. Search summary construction, embedding
  runtime, and eval artifact comparison must remain separate in prompts 35B-35D.
- Hard-to-describe public API: mitigated. The public story is local text embedding for
  declaration summaries plus stable model/cache/runtime facts.
- Implementation details contaminating interface comments: mitigated. This document names
  caller obligations and hidden decisions, not specific tokenizer fields, tensor shapes,
  or cache filenames.
