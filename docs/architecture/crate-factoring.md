# Crate Factoring

Eleven Rust crates, each owning one kind of hidden knowledge. The split is functional: a crate exists to localize a
class of change (Lean protocol mechanics, Lake project resolution, persisted storage, search and review policy,
embedding model acquisition/runtime policy, vector-corpus persistence, detachable vector experiments, report projection,
quality measurement, terminal I/O), not to mirror one old source file.

For the pipeline the crates implement, see [end-to-end-architecture.md](end-to-end-architecture.md).

## Crates

| Crate | Owns | May not depend on |
| --- | --- | --- |
| `lean-dup-worker` | Lean worker protocol, subprocess transport, worker version/build policy, timeouts. | any other `lean-dup` crate |
| `lean-dup-diagnostics` | Progress/profile events, runtime perf collection, generic file/JSON helpers. | any other `lean-dup` crate |
| `lean-dup-project` | Lake workspace discovery, module roots, mathlib source/execution roots, toolchain facts. | index, search, eval, cli |
| `lean-dup-index` | SQLite indexes, cache keys, provenance metadata, latest pointers, cache diagnostics, cleanup. | search, eval, cli |
| `lean-dup-vector-index` | Persisted declaration-vector corpora, vector database backend policy, corpus provenance, nearest-declaration lookup. | search, eval, report, cli |
| `lean-dup-search` | Symbolic audit workflow, symbolic candidate generation, semantic evidence planning, ranking, source facts, replacement hints. | embedding, vector-index, vector-search, eval, report, cli |
| `lean-dup-embedding` | Embedding model acquisition, local text embedding runtime, vector-cache policy, stable embedding facts. | search, eval, report, cli |
| `lean-dup-vector-search` | Hidden semantic/vector experiment workflow, vector validation artifacts, vector scorer variants, progress/cost accounting. | lower crates may not depend on it |
| `lean-dup-report` | Stable JSON DTOs, explanations, text rendering, report-owned cache/show/diff/eval projections, wording. | cli |
| `lean-dup-eval` | Labels, suites, stage metrics, quality gates, hidden perf workload artifacts. | cli |
| `lean-dup-cli` | clap parsing, command dispatch, stdout/stderr routing, output file writes, binary compatibility. | top layer; depends on the others |

Package and directory names omit `-rs`. The binary is `lean-dup` until a user-facing rename is accepted.

## Public API per crate

Each crate root is the supported public facade. Submodules and internals stay private.

- **`lean-dup-worker`**: `WorkerClient`, request/result DTOs, version/build policy. Subprocess transport, JSONL framing,
  protocol envelopes, request ids, and timeouts are private.
- **`lean-dup-diagnostics`**: progress/profile events, runtime measurement helpers. No semantic dependencies.
- **`lean-dup-project`**: `WorkspaceRequest`, `ResolvedWorkspace`, `SourceFile`, `resolve`, `ProjectMathlib`, mathlib
  resolution entry points. Lake path rules and `.olean` discovery sit on `ResolvedWorkspace`.
- **`lean-dup-index`**: `IndexStore`, build/open/hydrate DTOs, `SemanticFeatureFanout`, provenance summaries, cache
  diagnostics, safe cleanup reports. SQLite schema, posting layout, and latest-pointer layout are private; feature keys
  are opaque Lean-owned strings. The semantic index itself — feature rows and opaque-key postings — is served from the
  shared `lean-semantic-search-store` corpus beside each cache entry; this crate keeps only the display/probe SQLite.
  See [shared-search-adoption.md](shared-search-adoption.md).
- **`lean-dup-vector-index`**: `VectorCorpusBuildRequest`, `VectorCorpusOpenRequest`, opaque `VectorCorpus`,
  nearest-declaration query DTOs, corpus summaries, provenance facts, and stable vector-index errors. LanceDB/Arrow
  rows, vector database layout, index parameters, score conversion, cache paths, and backend fallback rules are private.
- **`lean-dup-search`**: `AuditVisibilityOptions`, `ProbePolicy`, `AuditRequest`, `AuditOutput`, `ShowOutput`,
  `DiffOutput`, `run_audit`, `run_show`, `run_diff`, `observe_search`. Retrieval keys, ranking constants, probe
  obligations, source-scan policy, and replacement-hint internals stay private. Search does not expose vector candidate
  DTOs, embedding-document DTOs, vector scorer variants, or vector artifact fields.
- **`lean-dup-embedding`**: `EmbeddingModelSpec`, `EmbeddingAcquisitionPolicy`, `EmbeddingPrepareRequest`,
  `EmbeddingPrepareResult`, stable input-format ids, `prepare_embedding_model`, and batch text embedding request/result
  DTOs. Model profiles, FastEmbed backend selection, Hugging Face cache layout, model filenames, tokenizer/runtime
  internals, normalization, vector-cache format, and download mechanics stay private. Normal audit/eval paths do not
  depend on this crate.
- **`lean-dup-vector-search`**: `VectorValidationRequest`, `VectorValidationOutcome`, and `run_vector_validation`. This
  is the only public entry point for hidden semantic/vector experiments. It may depend on `lean-dup-search`,
  `lean-dup-eval`, `lean-dup-embedding`, and `lean-dup-vector-index`; no lower crate may depend on it.
- **`lean-dup-report`**: report DTOs, projection functions, explanation facts, `render_text`.
- **`lean-dup-eval`**: `EvalSuite`, `EvalRequest`, `EvalOutput`, stage metrics, quality denominators. Text rendering
  belongs to report; runtime/memory measurement belongs to diagnostics.
- **`lean-dup-cli`**: clap argument types, command dispatch, stdout/stderr/file I/O, final error aggregation.

## Removed flags

Misleading audit flags that parsed without reliably changing behavior were removed instead of deprecated: `--threshold`,
`--include-imports`, `--import-root`, `--min-priority`, `--replacement-hints`.

## Why ten, not seven, not "core + cli"

A crate per old module would produce shallow pass-through crates around `retrieval`, `ranking`, `semantic_verification`,
`cache`, and `render`, forcing unstable internal records into public APIs. A single `core` plus a CLI crate is easy to
move around but leaves the same complected internal architecture.

The current split moves audit ordering into `lean-dup-search`, separates diagnostic plumbing from user-facing report
contracts, and lets `lean-dup-report` own stable projection and wording. Embedding model acquisition and CPU inference
sit in their own crate because those decisions change with the local ML runtime, not with retrieval, labels, report
wording, or terminal I/O. The vector index is separate from both `lean-dup-index` and `lean-dup-search` because vector
database persistence, ANN tuning, corpus provenance, and backend replacement change for different reasons than SQLite
feature storage or search candidate policy.

## Vector Deletion Contract

Vector search is a detachable experiment slice, not part of core symbolic audit/eval/report. Removing
`crates/vector-search`, `crates/embedding`, and `crates/vector-index` should not require edits to `crates/search`,
`crates/eval`, or `crates/report`. The core crates must not depend on those crates, re-export vector DTOs, include
vector fields in ordinary report JSON, or mention backend/model runtime vocabulary in public interfaces. Hidden vector
command wiring belongs in `lean-dup-vector-search`, not in the ordinary `lean-dup` CLI.
