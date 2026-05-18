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
| `lean-dup-diagnostics` | Shared error projection, progress/profile events, runtime perf collection. | Project, index, search, eval, report, CLI. |
| `lean-dup-project` | Lake workspace discovery, selected module roots, mathlib source/execution roots, toolchain facts. | Index, search, eval, CLI. |
| `lean-dup-index` | SQLite indexes, cache keys, provenance metadata, latest pointers, cache diagnostics and cleanup. | Search, eval, CLI. |
| `lean-dup-search` | Audit workflow, candidate generation, semantic evidence planning, ranking, source facts, replacement hints. | Eval, report, CLI. |
| `lean-dup-report` | Stable JSON DTOs, report explanations, text rendering, show/diff projection, eval table wording. | CLI. |
| `lean-dup-eval` | Labels, suites, stage metrics, quality gates, hidden perf workload artifacts. | CLI. |
| `lean-dup-cli` | Clap parsing, command dispatch, stdout/stderr routing, output file writes, binary compatibility. | None; this is the top layer. |

The package and directory names deliberately omit `-rs`. The binary remains `lean-dup` until a separate user-facing
rename is accepted.

## Current Tradeoffs

The second pass preserves behavior while removing the main misleading boundaries:

- `clap` is confined to `lean-dup-cli`; domain crates expose plain enums and request/result APIs.
- `lean-dup-search::audit::run_audit` owns the audit phase ordering, so CLI no longer sequences retrieval, probes,
  ranking, source facts, and replacement hints.
- `lean-dup-diagnostics` owns shared plumbing; `lean-dup-report` owns report projection and wording.
- Old file-shaped modules such as `search::retrieval`, `search::ranking`, `index::index`, and `eval::eval` are private
  implementation modules with curated root exports.

Some report and search DTOs remain public because JSON output, eval scoring, and report rendering use the same data
model. Further tightening should proceed only when a concrete caller can be simplified without losing report stability.

## Red Flag Review

- **Shallow module:** improved. Diagnostics, report projection, and audit workflow now hide real decisions rather than
  exposing old source-file modules.
- **Pass-through wrapper:** mitigated. Crates own files and behavior, not only re-export old modules.
- **Temporal decomposition:** mitigated. Audit sequencing is inside `lean-dup-search::audit`, not hand-wired by CLI.
- **Information leakage:** mitigated. SQLite, worker transport, CLI parsing, report wording, and diagnostic plumbing are
  separated and enforced by boundary tests.
- **Special-general mixture:** mitigated. Evaluation and CLI sit above production search/indexing.
- **Conjoined methods:** mitigated for audit. `show` and `diff` reuse the search audit output instead of duplicating
  phase sequencing.
- **Hard-to-describe public API:** improved. Public module surfaces are now curated root exports; remaining broad DTOs
  are tied to stable reports or eval.
- **Implementation details contaminating interface comments:** mitigated in crate-level docs; some moved item comments
  predate the split and should be reviewed opportunistically.
