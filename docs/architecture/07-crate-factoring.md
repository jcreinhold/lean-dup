# Crate Factoring

This document records the post-split Rust crate boundaries for `lean-dup`. The split is functional: each crate owns one
kind of hidden knowledge rather than one execution step or one old source file.

For the full pipeline, see
[06-end-to-end-architecture.md](/Users/jcreinhold/Code/lean-dup/docs/architecture/06-end-to-end-architecture.md).

## Design Note

The crate graph owns the coarse information-hiding contract for the Rust implementation. Its hidden knowledge is the
reason each component changes: Lean protocol mechanics, Lake project resolution, persisted corpus storage, search and
review policy, report projection, quality measurement, and terminal I/O.

The smallest public interface is eight package names and their dependency direction. Callers should not learn SQLite
tables, cache key layout, retrieval key encodings, worker JSONL framing, prompt/eval corpus policy, or CLI stdout/stderr
rules unless they are in the crate that owns that concern.

The preserved capability is the same local, read-only duplicate audit pipeline exposed through the `lean-dup` binary.
The discarded Python-era behavior is a single implementation surface where workspace discovery, indexing, search,
reporting, and evaluation all changed together.

## Design It Twice

Rejected: a crate per old module. That would make shallow pass-through crates around `retrieval`, `ranking`,
`semantic_verification`, `cache`, and `render`, forcing unstable internal records into public APIs.

Rejected: one `core` crate plus one CLI crate. That keeps the code easy to move but leaves the same complected internal
architecture.

Chosen first: seven functional crates. This made the old single-crate concerns visible, but it left diagnostics
misnamed as report output and left audit phase ordering in the CLI.

Chosen now: eight functional crates with a diagnostics/report split. This is deeper because diagnostic plumbing and
user-facing report contracts change for different reasons. It also lets `lean-dup-search` own the audit workflow while
`lean-dup-report` owns stable projection and wording.

## Crates

| Crate | Hidden knowledge | Must not depend on |
| --- | --- | --- |
| `lean-dup-worker` | Lean worker protocol, subprocess transport, worker version/build policy, timeouts. | Any other `lean-dup` crate. |
| `lean-dup-diagnostics` | Progress/profile events, runtime perf collection, and generic file/JSON/write helpers. It does not know product-layer errors. | Any other `lean-dup` crate. |
| `lean-dup-project` | Lake workspace discovery, selected module roots, mathlib source/execution roots, toolchain facts. | Index, search, eval, CLI. |
| `lean-dup-index` | SQLite indexes, cache keys, provenance metadata, latest pointers, cache diagnostics and cleanup. | Search, eval, CLI. |
| `lean-dup-search` | Audit workflow, candidate generation, semantic evidence planning, ranking, source facts, replacement hints. | Eval, report, CLI. |
| `lean-dup-report` | Stable JSON DTOs, report explanations, text rendering, report-owned cache/show/diff/eval projections, and output wording. | CLI. |
| `lean-dup-eval` | Labels, suites, stage metrics, quality gates, hidden perf workload artifacts. | CLI. |
| `lean-dup-cli` | Clap parsing, command dispatch, stdout/stderr routing, output file writes, binary compatibility. | None; this is the top layer. |

The package and directory names deliberately omit `-rs`. The binary remains `lean-dup` until a separate user-facing
rename is accepted.

## Current Boundary Contract

The final boundary pass preserves behavior while making crate roots the supported public facades:

- `clap` is confined to `lean-dup-cli`; domain crates expose plain enums and request/result APIs.
- `lean-dup-worker` exposes typed worker capabilities and stable row/progress/diagnostic DTOs. Subprocess transport,
  JSONL framing, protocol envelopes, request ids, timeout constants, and row dispatch stay private.
- `lean-dup-project` exposes workspace and project-mathlib concepts from the root. Callers do not import file-shaped
  `workspace` or `mathlib` modules, and Lake path rules such as module source paths and `.olean` discovery live on
  `ResolvedWorkspace`.
- `lean-dup-search` owns audit, show, diff, and eval-observation workflows from its root API. CLI and eval do not import
  `search::audit` or `search::observation`; retrieval keys, ranking structs, probe planning, source-reference scanning,
  replacement-hint mechanics, and baseline storage remain private.
- `lean-dup-index` exposes a semantic corpus facade. Search asks for feature fanout and matched handles through
  `SemanticFeatureFanout` and `SemanticFeatureMatches`, while SQLite schema, posting-list storage, query shape, and
  cache pointer layout stay private.
- `lean-dup-diagnostics` owns shared progress, profile, perf, runtime-memory, and generic file/JSON/write helpers only.
  Worker, project, index, search, eval, report, and CLI each own domain errors.
- `lean-dup-report` exposes report DTOs, projection functions, explanations, and text rendering from the root. Callers
  do not import `report::render` or `report::reports`.
- `lean-dup-eval` exposes suite/request/output and metric contracts only. Runtime memory measurement lives in
  diagnostics, and text rendering belongs to report.
- Misleading audit flags that parsed without reliably changing behavior were removed instead of deprecated:
  `--threshold`, `--include-imports`, `--import-root`, `--min-priority`, and `--replacement-hints`.

## Approved Public APIs

- `lean-dup-worker`: `WorkerClient`, worker request/result DTOs, stream row/progress/diagnostic DTOs, version/build
  policy, and worker errors. These DTOs are semantic worker facts after transport framing has been hidden, not raw JSONL
  protocol objects.
- `lean-dup-project`: `WorkspaceRequest`, `ResolvedWorkspace`, `SourceFile`, `resolve`, `ProjectMathlib`,
  `resolve_project_mathlib`, `resolve_workspace_mathlib`, and project errors.
- `lean-dup-index`: `IndexStore`, build/open/hydrate request and result types, semantic feature match queries,
  provenance summaries, cache diagnostics, and safe cleanup reports. Semantic feature keys are opaque Lean-owned keys;
  SQLite schema, posting-list layout, SQL queries, and latest-pointer layout stay private.
- `lean-dup-search`: `ReviewProfile`, `ProbePolicy`, `AuditRequest`, `AuditOutput`, audit DTOs, `ShowOutput`,
  `DiffOutput`, `run_audit`, `run_show`, `run_diff`, `SearchObservationRequest`, `SearchObservation`, and
  `observe_search`. Retrieval keys, ranking constants, probe obligations, source scan policy, replacement-hint
  internals, and baseline storage helpers stay private.
- `lean-dup-report`: report DTOs, projection functions, explanation facts, and `render_text`.
- `lean-dup-eval`: `EvalSuite`, `EvalRequest`, `EvalOutput`, suite execution, stage metrics, and quality denominators.
  Text rendering belongs to `lean-dup-report`; runtime memory measurement belongs to `lean-dup-diagnostics`.
- `lean-dup-cli`: clap argument types, command dispatch, stdout/stderr/file I/O, and final app error aggregation.

## Red Flag Review

- **Shallow module:** mitigated. Crate roots now expose curated capabilities rather than old file-shaped modules or
  wildcard facades.
- **Pass-through wrapper:** mitigated. The remaining facades translate to stable workflow, project, corpus, worker, and
  report concepts; they do not re-export implementation modules wholesale.
- **Temporal decomposition:** mitigated. Audit sequencing is inside `lean-dup-search`, not hand-wired by CLI, and
  project/source path ordering is owned by `lean-dup-project`.
- **Information leakage:** mitigated. SQLite, posting-list storage, retrieval keys, ranking structs, probe diagnostics,
  worker transport, CLI parsing, report wording, diagnostic plumbing, and Lake path layout are separated and enforced by
  boundary tests.
- **Overexposure:** mitigated. Public submodules for report, project, search audit, and search observation are private;
  eval no longer exports runtime-memory helpers.
- **Special-general mixture:** mitigated. Evaluation and CLI sit above production search/indexing, and diagnostics owns
  general runtime/perf plumbing instead of eval owning memory measurement.
- **Conjoined methods:** mitigated for audit. `show` and `diff` reuse the search audit output instead of duplicating
  phase sequencing.
- **Hard-to-describe public API:** improved. Public module surfaces are now curated workflow and DTO exports, and
  implementation-shaped records stay private or semantic.
- **Implementation details contaminating interface comments:** mitigated. Interface comments describe caller-facing
  contracts: semantic worker facts, project paths, corpus feature queries, report DTOs, and evaluation metrics rather
  than subprocess frames, SQL rows, or private module layout.
