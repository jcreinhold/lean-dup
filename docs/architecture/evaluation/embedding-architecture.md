# Embedding Architecture

This document records the architecture boundary for future embedding experiments. Prompt 35A
adds the `lean-dup-embedding` crate skeleton and workspace boundary, but it does not add model
download, tokenizer loading, Candle inference, vector cache, search integration, eval artifact
mode, or CLI behavior.

For the current pipeline, see [end-to-end-architecture.md](../end-to-end-architecture.md).
For crate boundaries, see [crate-factoring.md](../crate-factoring.md).

## Design Note

Hidden knowledge: the future embedding subsystem owns model acquisition policy, text
embedding input policy, model/cache/runtime summaries, local CPU runtime facts, and the
rules for keeping embedding artifacts reproducible. It is also the place where model
download, tokenizer compatibility, tensor layout, pooling, normalization, batching, and
vector cache decisions will eventually live.

Smallest public interface: a future `lean-dup-embedding` crate root that accepts
declaration-summary strings and returns local text embedding results plus stable model,
cache, input-policy, vector-dimension, runtime-counter, and typed-error facts.

Decisions that must not leak upward or sideways: Hugging Face cache layout,
tokenizer/model filenames, Candle tensor shapes, pooling and normalization details,
vector cache format, download mechanics, runtime batching, and any model-specific
fallbacks. Search, eval, report, and CLI callers should not learn those details.

Preserved capability: the default `lean-dup` auditor remains read-only, local,
deterministic, symbolic, and independent of embedding models. Existing audit, show,
diff, eval, JSON, cache, ranking, and semantic-probe behavior remains authoritative.

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

Chosen: add a future `lean-dup-embedding` crate inside the `lean-dup` workspace. The
crate will be consumed through crate-root APIs only. It will own model acquisition,
local CPU embedding, and vector cache policy, while `lean-dup-search` supplies stable
declaration-summary inputs and `lean-dup-eval` owns labels, experiment lifecycle, and
artifact comparison. This design is deeper because callers learn a small text-embedding
capability instead of Hugging Face, tokenizer, Candle, or cache internals.

## Future Crate Contract

The future crate is `lean-dup-embedding` at `crates/embedding`.

Public capability:

- embed batches of declaration-summary strings locally on CPU;
- return deterministic vectors and stable runtime facts for evaluation artifacts;
- report model/cache readiness without forcing normal audit to download or load models.

Public facts:

- model identity, including model id and resolved revision when available;
- model/cache status, including prepared, missing, unusable, or skipped states;
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
Explicit model preparation and hidden experiment commands are the only places where
model acquisition may occur.

## Prompt 35A Scope

Prompt 35A creates the architecture document, the empty `lean-dup-embedding` crate
boundary, and boundary tests that keep ML runtime dependencies out of the rest of the
workspace. The crate may expose skeleton request/result/error DTOs, but embedding model
acquisition and inference remain unsupported until Prompts 35B and 35C.

## Red Flag Review

- Shallow module: mitigated at the design level. The future public surface is a text
  embedding capability with stable summaries; runtime complexity stays inside the
  embedding crate.
- Pass-through wrapper: mitigated. The future crate is not a facade over Hugging Face or
  Candle APIs; it hides model acquisition, local inference, pooling, normalization, and
  cache policy behind lean-dup-specific facts.
- Temporal decomposition: mitigated. Callers should not run "download, validate,
  tokenize, infer, pool, normalize, cache" themselves; the embedding crate owns that
  sequence.
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
