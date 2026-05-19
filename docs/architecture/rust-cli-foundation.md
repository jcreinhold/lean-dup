# Rust CLI Engine

The `lean-dup` binary parses commands, routes orchestration, and writes output. Workspace
discovery, worker management, indexing, retrieval, ranking, reporting, evaluation, performance,
and cache lifecycle live behind capability-oriented modules.

For the full pipeline this CLI drives, see [end-to-end-architecture.md](end-to-end-architecture.md).
For the layering rule, see [overview.md](overview.md).

## Commands

| Command | What it does |
| --- | --- |
| `doctor` | Check workspace, Lake, worker, and cache health. JSON mode reports cache lifecycle (root, labels, latest-pointer status, schema/provenance, declaration counts, disk usage). |
| `index` | Build or reuse a source-backed workspace index for selected modules. |
| `index-mathlib` | Build or reuse the audited project's pinned mathlib index, using the shared content-addressed cache. |
| `audit` | Build or reuse indexes, retrieve candidates, optionally run bounded semantic verification, rank groups under the selected review profile, attach source/replacement context, render text or JSON. |
| `eval` | Run named quality suites with raw denominators for recall, shown-queue precision, hard-negative leakage, visible groups, probe availability, runtime, and memory. |
| `show` | Explain one resolvable group: evidence mode, semantic/probe state, blockers, visibility reason, replacement/import/caller impact. |
| `diff` | Compare saved baselines. |
| *hidden* `perf` | Run named performance workloads, write JSON artifacts. |
| *hidden* `cache-cleanup` | Dry-run by default; deletes unprotected stale entries when explicitly executed. |

Public flags name user intent: workspace, modules, comparison sources, review profile, semantic-probe enablement,
output format, diagnostics. Cache layout, SQLite tables, worker transport, and probe chunking do not surface as
normal audit options.

## Module boundaries

The CLI owns command vocabulary and orchestration. Each internal module owns the decisions likely to change inside its
boundary:

- **workspace / mathlib**: Lakefile parsing, workspace fallback, mathlib package layout.
- **worker runtime**: build policy, subprocess framing, JSONL parsing, timeout policy.
- **index store + cache lifecycle**: cache-key serialization, latest-pointer shape, SQLite table layout, cleanup
  safety.
- **retrieval / ranking**: key shapes, thresholds, blockers, profile filters.
- **semantic verification**: probe chunking, budgets, recovery.
- **report contract**: explanation precedence, JSON shaping, terminal layout.
- **eval / perf**: suite definitions, workload definitions, artifact policy.

None of these details leak into command parsing or output routing.

## Output policy

Text and JSON reports render from typed report facts. Progress and profile output go to stderr. JSON stdout remains a
single parseable value even when progress/profile flags are enabled.

The CLI is a thin command shell over deep internal boundaries. The two rejected alternatives —
inlining workspace/worker/cache/retrieval/ranking/rendering into `main.rs`, and a Rust binary
that delegates to retired Python — both create two production surfaces against one set of
contracts and let parsing or output routing drift with every production change.
