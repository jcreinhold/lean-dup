# Crate Factoring

Eight Rust crates, each owning one kind of hidden knowledge. The split is functional: a crate exists to localize a
class of change (Lean protocol mechanics, Lake project resolution, persisted storage, search and review policy,
report projection, quality measurement, terminal I/O), not to mirror one old source file.

For the pipeline the crates implement, see
[06-end-to-end-architecture.md](/Users/jcreinhold/Code/lean-dup/docs/architecture/06-end-to-end-architecture.md).

## Crates

| Crate | Owns | May not depend on |
| --- | --- | --- |
| `lean-dup-worker` | Lean worker protocol, subprocess transport, worker version/build policy, timeouts. | any other `lean-dup` crate |
| `lean-dup-diagnostics` | Progress/profile events, runtime perf collection, generic file/JSON helpers. | any other `lean-dup` crate |
| `lean-dup-project` | Lake workspace discovery, module roots, mathlib source/execution roots, toolchain facts. | index, search, eval, cli |
| `lean-dup-index` | SQLite indexes, cache keys, provenance metadata, latest pointers, cache diagnostics, cleanup. | search, eval, cli |
| `lean-dup-search` | Audit workflow, candidate generation, semantic evidence planning, ranking, source facts, replacement hints. | eval, report, cli |
| `lean-dup-report` | Stable JSON DTOs, explanations, text rendering, report-owned cache/show/diff/eval projections, wording. | cli |
| `lean-dup-eval` | Labels, suites, stage metrics, quality gates, hidden perf workload artifacts. | cli |
| `lean-dup-cli` | clap parsing, command dispatch, stdout/stderr routing, output file writes, binary compatibility. | top layer; depends on the others |

Package and directory names omit `-rs`. The binary is `lean-dup` until a user-facing rename is accepted.

## Public API per crate

Each crate root is the supported public facade. Submodules and internals stay private.

- **`lean-dup-worker`**: `WorkerClient`, request/result DTOs, stream row/progress/diagnostic DTOs, version/build
  policy, worker errors. DTOs are semantic worker facts; subprocess transport, JSONL framing, protocol envelopes,
  request ids, and timeouts are private.
- **`lean-dup-project`**: `WorkspaceRequest`, `ResolvedWorkspace`, `SourceFile`, `resolve`, `ProjectMathlib`,
  `resolve_project_mathlib`, `resolve_workspace_mathlib`, project errors. Lake path rules and `.olean` discovery live
  on `ResolvedWorkspace`.
- **`lean-dup-index`**: `IndexStore`, build/open/hydrate DTOs, `SemanticFeatureFanout`, `SemanticFeatureMatches`,
  provenance summaries, cache diagnostics, safe cleanup reports. Feature keys are opaque Lean-owned strings; SQLite
  schema, posting layout, queries, and latest-pointer layout are private.
- **`lean-dup-search`**: `ReviewProfile`, `ProbePolicy`, `AuditRequest`, `AuditOutput`, audit DTOs, `ShowOutput`,
  `DiffOutput`, `run_audit`, `run_show`, `run_diff`, `SearchObservationRequest`, `SearchObservation`,
  `observe_search`. Retrieval keys, ranking constants, probe obligations, source-scan policy, replacement-hint
  internals, baseline storage stay private.
- **`lean-dup-report`**: report DTOs, projection functions, explanation facts, `render_text`.
- **`lean-dup-eval`**: `EvalSuite`, `EvalRequest`, `EvalOutput`, suite execution, stage metrics, quality
  denominators. Text rendering belongs to report; runtime memory measurement belongs to diagnostics.
- **`lean-dup-cli`**: clap argument types, command dispatch, stdout/stderr/file I/O, final app error aggregation.

## Removed flags

Misleading audit flags that parsed without reliably changing behavior were removed instead of deprecated:
`--threshold`, `--include-imports`, `--import-root`, `--min-priority`, `--replacement-hints`.

## Why eight, not seven, not "core + cli"

A crate per old module would produce shallow pass-through crates around `retrieval`, `ranking`, `semantic_verification`,
`cache`, `render`, forcing unstable internal records into public APIs.

A single `core` plus a CLI crate is easy to move around but leaves the same complected internal architecture.

The previous seven-crate split misnamed diagnostics as report output and left audit phase ordering in the CLI. The
current split moves audit ordering into `lean-dup-search`, separates diagnostic plumbing from user-facing report
contracts, and lets `lean-dup-report` own stable projection and wording.
