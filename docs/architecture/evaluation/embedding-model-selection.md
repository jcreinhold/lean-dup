# Embedding Model Selection

Prompt 35T chooses the next local embedding-model and input-format experiments. It does
not claim that a model improves search quality. It records which profiles and role
formats Prompt 35U may implement so validation can measure them without turning
`lean-dup-embedding` into an open-ended model launcher.

Sources checked on 2026-05-20:

- [fastembed crate docs](https://docs.rs/crate/fastembed/latest), version 5.13.4.
- [fastembed feature flags](https://docs.rs/crate/fastembed/latest/features), version
  5.13.4.
- [FastEmbed supported models](https://qdrant.github.io/fastembed/examples/Supported_Models/).
- [BGE-small model card](https://huggingface.co/BAAI/bge-small-en-v1.5).
- [BGE-base model card](https://huggingface.co/BAAI/bge-base-en-v1.5).

Local constraints checked on 2026-05-20:

- Rust toolchain: `rustc 1.95.0`, host `aarch64-apple-darwin`.
- Machine: Apple M4 Pro, 12 hardware threads, 24 GiB RAM.
- Current embedding dependency: `fastembed = 5.13.4` with default features disabled and
  only `hf-hub-rustls-tls` plus `ort-download-binaries-rustls-tls` enabled.
- No GPU, service, remote API, Candle feature, image-model feature, Qwen3 feature, or
  Nomic v2 MoE feature is part of the current runtime path.

## Design Note

This document owns the experiment-selection decision: which local CPU embedding profiles
and role-format variants are worth implementing next. It does not own model runtime,
tokenization, cache layout, vector-corpus persistence, candidate generation, ranking, or
quality validation.

The smallest interface it exposes is a decision for Prompt 35U: supported profile ids to
add, role-format variant ids to add, rejected candidates with reasons, and cost facts that
future validation must measure.

The decisions that must not leak upward or sideways are FastEmbed enum names, ONNX/ORT
mechanics, tokenizer files, pooling, model filenames, model-specific prefix strings,
download paths, text-vector cache filenames, and backend feature flags. Those stay inside
`lean-dup-embedding`; search, eval, report, and CLI see profile ids, model ids,
dimensions, role-format ids, and stable runtime/cache facts.

The preserved user-facing capability is the ordinary symbolic audit and ordinary eval
path. They remain embedding-free, vector-index-free, and governed by the symbolic scorer.

The discarded behavior is choosing models because they are familiar, because they appear
in a public enum, or because an old rerank-only experiment mentioned them. Retired
MiniLM/rerank paths are not comparison baselines for this decision.

## Design It Twice

Three designs were considered.

First, keep only `BAAI/bge-small-en-v1.5` and tune corpus/search policy. This is
reasonable but too narrow for the current failure analysis: poor vector results could be
caused by role formatting or model capacity, and keeping one model would leave those
failure modes conflated.

Second, add several model profiles and let validation sort them out. This is rejected.
It would turn the private profile registry into a junk drawer, multiply cache/build cost
for mathlib-scale validation, and make Prompt 35U spend most of its session wiring models
instead of preserving a clean embedding boundary.

Third, choose one additional feasible local candidate plus explicit role-format
ablations from current FastEmbed/runtime evidence before changing code. This is chosen.
It is deeper because it keeps model mechanics inside one crate, gives search/eval only
stable profile and role-format facts, and tests two concrete risks with the smallest
credible experiment matrix: model capacity and query/document formatting.

## Runtime Evidence

FastEmbed 5.13.4 supports synchronous local text embedding through ONNX/ORT for the BGE
v1.5 models. Its crate docs list `BAAI/bge-small-en-v1.5` as the default text embedding
model and show `query:` / `passage:`-style examples. The feature table shows Qwen3 and
Nomic v2 MoE behind Candle feature flags, while the current crate uses only Rustls Hugging
Face access and ORT binary download features.

The FastEmbed supported-model table reports:

| Model | Dim | Approx model size | License |
| --- | ---: | ---: | --- |
| `BAAI/bge-small-en-v1.5` | 384 | 0.067 GiB | MIT |
| `BAAI/bge-base-en-v1.5` | 768 | 0.210 GiB | MIT |
| `jinaai/jina-embeddings-v2-base-code` | 768 | 0.640 GiB | Apache-2.0 |
| `nomic-ai/nomic-embed-text-v1.5` | 768 | 0.520 GiB | Apache-2.0 |
| `BAAI/bge-large-en-v1.5` | 1024 | 1.200 GiB | MIT |

BGE v1.5 model cards say the v1.5 models have a more reasonable similarity distribution
than earlier BGE releases. They recommend adding an instruction to short queries for
short-query-to-long-passage retrieval, adding no instruction to passages, and choosing the
instruction setting by downstream task performance. They also state that no-instruction
embedding has only slight retrieval degradation for BGE v1.5. That matters here because
Lean duplicate search compares declarations to declarations, not natural-language user
queries to long prose passages.

## Role-Format Variants

Prompt 35U should implement role-format variants as profile-private wrapping policy, not
as model prefixes in search.

| Variant id | Query declaration input | Corpus declaration input | Why test it |
| --- | --- | --- | --- |
| `symmetric-document` | document-style wrapping | document-style wrapping | Declaration-to-declaration search is closer to symmetric similarity than user-query retrieval. BGE guidance allows no query instruction, and passages need no instruction. |
| `asymmetric-query-document` | query-style wrapping | document-style wrapping | FastEmbed examples recommend query/passage-style prefixes, and the workspace declaration can be treated as a query against the comparison corpus. |

Both variants must record role-format id and version in hidden artifacts. Neither variant
may expose final model input text or prefix strings outside `lean-dup-embedding`.

Do not add a symmetric query-style variant in Prompt 35U. It adds another axis without a
clear model-card rationale and would make the first ablation matrix harder to interpret.

## Candidate Comparison

### Current Baseline: BGE Small

Model id: `BAAI/bge-small-en-v1.5`.

Profile id: keep `bge-small-en-v1.5`.

Vector dimension: 384.

Approximate model size: 0.067 GiB in FastEmbed's supported-model table.

Runtime feature requirements: current FastEmbed ONNX/ORT path; no new features.

CPU/RSS risk: lowest candidate risk. It is the current baseline and should stay the first
model used for fast fixtures and smoke validation.

Input roles: supports both query and document roles through profile-private wrapping.

Relevance to Lean declaration text: reasonable local baseline for English-like statement
and docstring text, with enough capacity to test plumbing and non-saturated candidate
generation. It may be underpowered for definition-aware documents that include symbolic
body summaries.

Expected cache impact: raw vectors cost 384 `f32` values per declaration, about 1.5 KiB
before vector-database overhead. A 200k-declaration corpus would store about 293 MiB of
raw vector values. Model cache cost is small relative to the corpus.

### Selected Additional Candidate: BGE Base

Model id: `BAAI/bge-base-en-v1.5`.

Profile id proposal: `bge-base-en-v1.5`.

Vector dimension: 768.

Approximate model size: 0.210 GiB in FastEmbed's supported-model table.

Runtime feature requirements: same FastEmbed ONNX/ORT path as BGE-small; no GPU, service,
remote API, Candle feature, or new dependency.

CPU/RSS risk: moderate. The model file is roughly 3.1x BGE-small's listed size and the
vectors are 2x wider. On the current 24 GiB local machine this is acceptable for hidden
validation, but Prompt 35Y must report cold-build time, warm-query time, RSS, embedding
cache size, and vector-corpus size before any quality claim.

Input roles: same BGE v1.5 role guidance as BGE-small; test both selected role-format
variants.

Relevance to Lean declaration text: it isolates model capacity while keeping the same
family, license, runtime path, context limit, and role-format guidance. If BGE-small fails
because declaration documents are too technical or compressed, BGE-base is the narrowest
capacity ablation that does not add a new model family.

Expected cache impact: raw vectors cost 768 `f32` values per declaration, about 3.0 KiB
before vector-database overhead. A 200k-declaration corpus would store about 586 MiB of
raw vector values. The model cache remains below the rejected heavyweight candidates.

## Rejected Candidates

`sentence-transformers/all-MiniLM-L6-v2`: rejected. It was part of the retired
rerank-only history and must not be preserved as a compatibility baseline.

`BAAI/bge-large-en-v1.5`: rejected for Prompt 35U. It is supported and same-family, but
FastEmbed lists it at about 1.200 GiB with 1024-dimensional vectors. That is too much
cost for the first clean-break ablation while validation still needs non-saturated
command-level workloads and progress/cost accounting.

`BAAI/bge-m3`: rejected for Prompt 35U. It is attractive for long and multilingual input,
but it mixes dense, sparse, and multi-vector retrieval concerns. The next prompt should
not add a multifunction model before the dense-vector baseline has clean artifacts.

`Qwen/Qwen3-Embedding-0.6B`: rejected for Prompt 35U. FastEmbed exposes Qwen3 through a
feature-gated Candle backend. That would add a backend family, memory-budget questions,
and input-format questions in the same session as the role-format ablation.

`Qwen/Qwen3-Embedding-4B`, `Qwen/Qwen3-Embedding-8B`, and `Qwen/Qwen3-VL-Embedding-2B`:
rejected. They are too large or multimodal for local CPU mathlib validation and require
the same feature-gated Candle path.

`nomic-ai/nomic-embed-text-v1.5`: rejected for Prompt 35U. It is supported by FastEmbed,
but the supported-model table lists about 0.520 GiB, and the model family introduces its
own search-query/search-document formatting convention. That is too many variables for
the next implementation step.

`nomic-ai/nomic-embed-text-v2-moe`: rejected. FastEmbed requires the `nomic-v2-moe`
feature and Candle backend. It is out of scope for the current ONNX/ORT local CPU path.

`jinaai/jina-embeddings-v2-base-code`: rejected for Prompt 35U. It is relevant to
definition body summaries, but FastEmbed lists about 0.640 GiB. It also changes both
domain bias and model family at once; test BGE capacity and role formatting first.

`jinaai/jina-embeddings-v2-base-en`, `mixedbread-ai/mxbai-embed-large-v1`,
`snowflake/snowflake-arctic-embed-m`, `snowflake/snowflake-arctic-embed-m-long`,
`snowflake/snowflake-arctic-embed-l`, `thenlper/gte-large`, `intfloat/multilingual-e5-*`,
and `google/embeddinggemma-300m`: rejected for Prompt 35U because they either add a new
family without a narrower diagnosis, have larger model/vector cost, target multilingual or
general RAG needs not yet shown to matter, or require extra validation surface.

`snowflake/snowflake-arctic-embed-xs` and `snowflake/snowflake-arctic-embed-s`: rejected
for Prompt 35U despite feasible size. They are plausible future alternatives, but they do
not test the immediate capacity question as cleanly as BGE-base and would add a new family
before the BGE role-format ambiguity is resolved.

Cross-encoder rerankers: rejected for this prompt. They are not vector-corpus models and
would require a separate top-k reranking architecture. This prompt selects local
embedding models for candidate generation, not a cross-encoder reranker.

Remote APIs, Qdrant-hosted model services, GPU-only paths, image models, sparse models,
and late-interaction/ColBERT models: rejected. They either violate the local CLI
constraint, add service/GPU requirements, or change the retrieval architecture rather than
testing dense declaration-vector candidate generation.

## Prompt 35U Decision

Prompt 35U should implement:

1. Keep `bge-small-en-v1.5` as the default hidden baseline.
2. Add one additional supported profile: `bge-base-en-v1.5`, model id
   `BAAI/bge-base-en-v1.5`, dimension 768, backend family label `fastembed`.
3. Add role-format ablation support inside `lean-dup-embedding`:
   `symmetric-document` and `asymmetric-query-document`.
4. Record profile id, role-format id, model id, dimension, document policy id,
   eligibility policy id, top-k, saturation status, runtime counters, RSS, model cache
   size, text-vector cache size, and vector-corpus size in hidden artifacts.
5. Keep FastEmbed enum names, prefix strings, ONNX/ORT details, tokenizer files, and
   model-file paths private to `lean-dup-embedding`.

Prompt 35U should not implement Qwen3, Nomic, Jina, Snowflake, sparse, late-interaction,
image, remote, GPU, or cross-encoder profiles. It should not make embeddings or vector
search default.

## Red Flag Review

- *Shallow module:* this document chooses a bounded experiment matrix; it does not expose
  every runtime-supported model as a public profile.
- *Pass-through wrapper:* selected profiles are lean-dup capability facts, not public
  FastEmbed enum forwarding.
- *Temporal decomposition:* the decision is organized by hidden knowledge and experiment
  axes, not by download, load, embed, query, validate.
- *Information leakage:* model runtime, tokenizer, prefix, file, and backend feature
  details stay below `lean-dup-embedding`.
- *Special-general mixture:* search-quality experiment policy stays in search/eval;
  model mechanics stay in embedding.
- *Conjoined methods:* role-format ablation and model-profile selection are recorded as
  separate axes so validation can attribute outcomes.
- *Hard-to-describe public API:* Prompt 35U needs two profile ids and two role-format ids,
  not a general model loader.
- *Implementation details in interface comments:* this is a decision artifact. Future
  comments should describe profile capability facts, not FastEmbed enum names or model
  file layout.
