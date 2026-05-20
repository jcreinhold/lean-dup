# Architecture Charter

This is the doctrine document for `lean-dup`: the layer boundaries, the design rules other
documents rely on, and the non-goals. For the as-built pipeline, see
[end-to-end-architecture.md](end-to-end-architecture.md). For release gates, see
[production-readiness.md](production-readiness.md).

## The Lean/Rust boundary

One rule sits above everything else:

- **Lean** computes semantic facts that require the elaborated Lean environment.
- **Rust** owns everything else: persistence, workflow, retrieval, ranking, reporting,
  evaluation, release.

Rust asks Lean semantic questions through a narrow, versioned
[worker protocol](worker-protocol.md). JSON and JSONL are transport encodings, not
architecture. Rust must not inspect Lean expressions, recompute semantic fingerprints from
pretty-printed types, or let SQLite layout leak into audit, ranking, or reporting code.

### What each side computes

| Lean                                                                                        | Rust                                                                |
| ------------------------------------------------------------------------------------------- | ------------------------------------------------------------------- |
| declaration identity, kind, visibility, modifiers, source spans                             | workspace discovery, Lake invocation, worker lifecycle              |
| pretty-printed statement text (display only)                                                | cache keys, cache validation, index labels, index paths             |
| exact, safe-binder-permutation, connective, and conclusion fingerprints                     | SQLite indexes (local, mathlib, external)                           |
| role-aware feature keys for constants, heads, binders, conclusions                          | weighted retrieval, broad-key suppression, candidate caps           |
| binder count, low-signal markers                                                            | source-reference scans, name-token features                         |
| bounded probe results: same-statement, safe reordering, structural specialization, guarded reducible-definition equality | ranking, blockers, priorities, recommended actions, replacement hints |
| (none)                                                                                      | text, JSON, `show`, profile, and baseline diff reports              |

## Why this shape

The rejected alternative had Lean emit names and pretty-printed types and let Rust recompute
fingerprints, statement features, and probe-like checks from strings. It looks convenient because
Rust owns the CLI and index, but it leaks Lean semantics into the scale layer and turns display
text into a false abstraction.

The chosen design hides Lean expression traversal entirely behind the worker. Rust stores opaque
ids and keys, never parses Lean syntax, and never calls into a Lean FFI on the default path. A
measured FFI spike remains optional; it is not the production starting point.

## The five layers

| Layer                                        | Abstraction                                                                                          |
| -------------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| Lean worker package                          | semantic capabilities: `extract`, `features`, `index`, `probe`, `doctor`, `version`                  |
| Versioned worker protocol                    | schema-versioned requests, responses, progress events, structured errors                             |
| Rust CLI engine                              | workspace discovery, module roots, Lake invocation, worker validation, command coordination          |
| SQLite indexes + cache lifecycle             | persisted truth for local/mathlib/external; reuse, validation, doctor, protected cleanup             |
| Retrieval, verification, ranking, reporting | candidates, proof-grade evidence, ranked groups, stable explanations, text/JSON/`show`/diff          |

Each layer presents a different abstraction; each hides decisions likely to change.

## Information-hiding boundaries

Each boundary owns one decision that changes.

| Boundary                  | Hides                                                                                              |
| ------------------------- | -------------------------------------------------------------------------------------------------- |
| Lean semantics            | expression traversal, binder dependency, universe-sensitive canonicalization, generated-declaration detection, reducibility guards |
| Worker protocol           | transport encoding, process framing, schema compatibility, stderr policy, progress delivery, error mapping |
| Index persistence         | SQLite schema, cache-key storage, posting tables, declaration hydration, invalidation              |
| External provenance       | source-root mapping, execution-root policy, importability, static fallback                         |
| Retrieval                 | rare-key weighting, broad-key suppression, top-k heap maintenance, origin-aware pairing            |
| Semantic verification     | probe obligations, budgets, module planning, private/generated filters, cache keys                 |
| Ranking                   | confidence adjustment, blockers, suppression, review profiles, recommended actions                 |
| Reporting                 | terminal layout, JSON shaping, `show` expansion, baseline diff presentation                        |
| Evaluation and performance | suite/workload definitions, manual private-path policy, artifact names, cost-class extraction      |

A caller asks each boundary for the result it needs. It does not assemble the lower-level steps
itself.

## Design rules

- deep modules over shallow wrappers;
- information hiding over shared conventions;
- different layers, different abstractions;
- somewhat general interfaces, not encodings of one caller's vocabulary;
- errors and special cases defined out of public interfaces where possible;
- performance designed around measured workloads, with critical-path simplification hidden.

Red flags to watch for:

- table-name leakage outside the index module;
- worker-command leakage into retrieval, ranking, or reporting;
- Rust recomputation of Lean semantic facts from pretty text;
- temporal decomposition where modules are named after audit phases but share hidden knowledge;
- special-general mixture where corpus-specific cleanup policy enters general ranking or
  retrieval;
- interface comments that describe SQLite layout, Lean traversal algorithms, or migration
  scaffolding.

## Validated capabilities

These capabilities are preserved across the rewrite; new work must not regress them.

- `doctor`, `index`, `index-mathlib`, `audit`, `eval`, `show`, `diff`;
- private/public filtering and direct or named import comparison;
- project-pinned mathlib and other external indexes;
- source-backed versus static provenance in reports;
- semantic probes for high-value candidates;
- ranked actionable findings and review priorities;
- text and JSON reports with stable explanation facts;
- progress and profile output that never corrupts JSON;
- read-only replacement/import hints with target declaration, import status, and bounded caller
  references;
- baseline diff, production-gate evaluation, hidden perf workloads, cache diagnostics.

## Non-goals

`lean-dup` is not a theorem prover or a semantic search service. The default auditor does not
perform broad proof search, use embeddings, call network services, or rewrite Lean files. The
default route is a versioned worker API over subprocess transport. FFI is not used unless a
future measurement justifies its safety and maintenance cost.

## Architectural commitments

**Mathlib indexing.** Indexes are built once from a project's pinned dependency, resolved by
label, reused across compatible projects, invalidated by schema/toolchain/source changes, and
queried without hydrating all mathlib declarations during ordinary audits.

**Report quality.** Default text output is a high-precision cleanup queue. Empty queues explain
themselves. `show` explains the evidence and blockers for one group. Replacement hints include
import and caller impact. JSON retains typed evidence for deeper review without depending on
terminal formatting.

**Read-only by default.** The auditor never edits Lean source. Version output records binary,
worker, protocol, index schema, report schema, and Git revision; that record makes any audit
result reproducible.

## References

- [End-to-end architecture](end-to-end-architecture.md)
- [Production readiness](production-readiness.md)
- [Loogle.lean](https://github.com/nomeata/loogle/blob/master/Loogle.lean)
