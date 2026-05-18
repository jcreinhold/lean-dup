# Crate Factoring

This document records the post-split Rust crate boundaries for `lean-dup`. The split is functional: each crate owns one
kind of hidden knowledge rather than one execution step or one old source file.

For the full pipeline, see
[06-end-to-end-architecture.md](/Users/jcreinhold/Code/lean-dup/docs/architecture/06-end-to-end-architecture.md).

## Design Note

The crate graph owns the coarse information-hiding contract for the Rust implementation. Its hidden knowledge is the
reason each component changes: Lean protocol mechanics, Lake project resolution, persisted corpus storage, search and
review policy, report projection, quality measurement, and terminal I/O.

The smallest public interface is seven package names and their dependency direction. Callers should not learn SQLite
tables, cache key layout, retrieval key encodings, worker JSONL framing, prompt/eval corpus policy, or CLI stdout/stderr
rules unless they are in the crate that owns that concern.

The preserved capability is the same local, read-only duplicate audit pipeline exposed through the `lean-dup-rs` binary.
The discarded Python-era behavior is a single implementation surface where workspace discovery, indexing, search,
reporting, and evaluation all changed together.

## Design It Twice

Rejected: a crate per old module. That would make shallow pass-through crates around `retrieval`, `ranking`,
`semantic_verification`, `cache`, and `render`, forcing unstable internal records into public APIs.

Rejected: one `core` crate plus one CLI crate. That keeps the code easy to move but leaves the same complected internal
architecture.

Chosen: seven functional crates. This is deeper because each crate hides a volatile design family and the dependency
graph prevents lower-level concerns from learning about CLI, evaluation, or report presentation.

## Crates

| Crate | Hidden knowledge | Must not depend on |
| --- | --- | --- |
| `lean-dup-worker` | Lean worker protocol, subprocess transport, worker version/build policy, timeouts. | Any other `lean-dup` crate. |
| `lean-dup-project` | Lake workspace discovery, selected module roots, mathlib source/execution roots, toolchain facts. | Index, search, eval, CLI. |
| `lean-dup-index` | SQLite indexes, cache keys, provenance metadata, latest pointers, cache diagnostics and cleanup. | Search, eval, CLI. |
| `lean-dup-search` | Candidate generation, semantic evidence planning, ranking, source facts, replacement hints, report explanations. | Eval, CLI. |
| `lean-dup-report` | Shared errors, progress/profile events, and runtime performance diagnostics. | Project, index, search, eval, CLI. |
| `lean-dup-eval` | Labels, suites, stage metrics, quality gates, hidden perf workload artifacts. | CLI. |
| `lean-dup-cli` | Clap parsing, command dispatch, stdout/stderr routing, output file writes, binary compatibility. | None; this is the top layer. |

The package and directory names deliberately omit `-rs`. The binary remains `lean-dup-rs` until a separate user-facing
rename is accepted.

## Current Tradeoffs

The split preserves behavior first. Some APIs are still broader than ideal because cross-crate extraction made former
`pub(crate)` records public. Those records are now visible design debt, not an intended final surface. The next
interface-tightening pass should replace broad records with request/result APIs where the caller only needs a capability
such as "audit", "build index", "score eval suite", or "render report".

`lean-dup-report` currently owns shared errors and diagnostic collectors rather than only final report DTOs. This keeps
the graph acyclic while preserving cross-crate profile/perf collection. If report projection grows independently from
diagnostics, split the internal modules inside `lean-dup-report` before adding another crate.

## Red Flag Review

- **Shallow module:** residual risk in public records widened for compilation. The crate boundaries themselves hide
  meaningful concerns; later API tightening should reduce record exposure.
- **Pass-through wrapper:** mitigated. Crates own files and behavior, not only re-export old modules.
- **Temporal decomposition:** mitigated. Boundaries are by concern, not by audit execution order.
- **Information leakage:** partially mitigated. SQLite, worker transport, and CLI output policy are separated; broad
  public structs remain to be tightened.
- **Special-general mixture:** mitigated. Evaluation and CLI sit above production search/indexing.
- **Conjoined methods:** residual risk in `lean-dup-cli::commands`, which still orchestrates audit phases. A later
  search API pass should pull that sequence down.
- **Hard-to-describe public API:** residual risk from the first extraction. The crate graph is describable; individual
  public types need trimming.
- **Implementation details contaminating interface comments:** mitigated in crate-level docs; some moved item comments
  predate the split and should be reviewed opportunistically.
