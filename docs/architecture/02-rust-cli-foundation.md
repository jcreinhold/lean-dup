# Rust CLI Engine

This document records the current Rust CLI boundary. It supersedes the prompt-08 foundation note: `lean-dup-rs` is now
the operational Rust/Lean engine, not a skeleton around future work. For the full as-built pipeline, see
[06-end-to-end-architecture.md](/Users/jcreinhold/Code/lean-dup/docs/architecture/06-end-to-end-architecture.md).

## Design Note

The CLI engine owns command parsing, command orchestration, progress/profile plumbing, top-level output routing, and the
small user-facing command vocabulary. It coordinates workspace resolution, worker use, indexing, retrieval, semantic
verification, ranking, report construction, evaluation, and diagnostics through internal modules.

Its smallest public interface is the `lean-dup-rs` binary plus the crate-level `run` entrypoint. The normal public
commands are `doctor`, `index`, `index-mathlib`, `audit`, `eval`, `show`, and `diff`. Hidden developer commands are
`perf` and `cache-cleanup`.

These decisions must not leak upward or sideways:

- Lakefile parsing, workspace fallback rules, or mathlib package layout;
- worker build policy, subprocess framing, JSONL parsing, or timeout policy;
- cache-key serialization, latest-pointer shape, SQLite table layout, or cleanup safety rules;
- retrieval key shapes, ranking thresholds, probe chunking, and report explanation precedence;
- stdout/stderr routing details beyond the guarantee that JSON stdout remains machine-clean.

The preserved user-facing capability is a local read-only duplicate auditor with real indexing, cached mathlib/external
comparison, semantic evidence, ranked review groups, replacement hints, baseline diffs, evaluation suites, performance
workloads, and cache diagnostics.

Python-era behavior intentionally discarded:

- Python as the production command surface;
- Rust wrappers that forward to Python implementation paths;
- loosely typed command state;
- source/text-driven Lean semantic parsing in orchestration code;
- ad hoc stderr writes from internal modules.

## Design It Twice

**Rejected: one large command script.** Putting workspace discovery, worker calls, cache decisions, retrieval, ranking,
and rendering in `main.rs` would make the CLI easy to follow in the small and brittle in the large. Every production
change would risk changing command parsing or output routing.

**Rejected: Rust as a compatibility shell.** A Rust binary that preserves Python entry points or delegates behavior to
retired Python modules would keep two production surfaces and make release status depend on obsolete cache and protocol
assumptions.

**Chosen: command shell over deep internal boundaries.** The CLI owns command vocabulary and orchestration only.
Workspace, mathlib, worker, index, retrieval, verification, ranking, source facts, report contract, eval, perf, and
cache lifecycle each own their hidden decisions. This is deeper because users see one binary while volatile details
stay behind capability-oriented module interfaces.

## Public Behavior

`doctor` checks workspace, Lake, Lean worker, and cache health. In JSON mode it reports cache lifecycle diagnostics,
including cache root, labels, latest-pointer status, schema/provenance state, declaration counts when readable, and disk
usage.

`index` builds or reuses a source-backed workspace index for selected modules.

`index-mathlib` builds or reuses the audited project's pinned mathlib index. It runs from the local project Lake
environment and uses the shared content-addressed cache by default.

`audit` builds or reuses the needed indexes, retrieves candidates, optionally runs bounded semantic verification, ranks
groups under the selected review profile, attaches source/replacement context where useful, and renders text or JSON.

`eval` runs named quality suites with raw denominators for recall, shown-queue precision, hard-negative leakage,
visible groups, probe availability, runtime, and memory. The fast suites are suitable for routine checks; KanProofs
suites are explicit manual workloads.

`show` explains one resolvable group with evidence mode, semantic/probe state, blockers, visibility reason, and
replacement/import/caller impact.

`diff` compares saved baselines.

Hidden `perf` runs named performance workloads and writes JSON artifacts. Hidden `cache-cleanup` is dry-run by default
and deletes only unprotected stale entries when explicitly executed.

## Output Policy

Text and JSON reports are rendered from typed report facts. Progress and profile output go to stderr. JSON stdout is a
single parseable value even when progress/profile flags are enabled.

The CLI does not expose SQLite details, worker transport details, probe chunking, or cache internals as normal audit
options. Public flags describe user intent: workspace, modules, comparison sources, review profile, semantic-probe
enablement, output format, and diagnostics.

## Red Flag Review

- **Shallow module:** mitigated. The CLI coordinates workflows but leaves hidden decisions to workspace, worker, index,
  retrieval, semantic verification, ranking, source, report, eval, perf, and cache modules.
- **Pass-through wrapper:** avoided. Rust no longer forwards to Python; it owns the production command surface.
- **Temporal decomposition:** mitigated. Modules are organized by hidden knowledge, not merely by audit phase.
- **Information leakage:** mitigated. Cache, SQLite, worker, probe, and retrieval internals stay out of normal CLI
  flags and reports.
- **Special-general mixture:** contained. KanProofs policy is in eval/perf/manual artifacts, not normal command parsing.
- **Conjoined methods:** mitigated. Commands exchange typed domain values rather than shared mutable phase state.
- **Hard-to-describe public API:** mitigated. Users run one binary with a small set of task-oriented commands.
- **Implementation details contaminating interface comments:** mitigated. This document describes caller-visible
  behavior and boundaries, not table layouts, worker framing, or temporary migration scaffolding.
