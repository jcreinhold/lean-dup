# Lean-Dup Architecture Overview

This document is the first design reference for the `lean-dup` rewrite. Later prompts should read it before choosing
module boundaries, schemas, caches, ranking policy, or release behavior.

The Lean/Rust boundary is the central rule:

- Lean owns semantic facts and probes.
- Rust owns scale, persistence, workflow, retrieval, ranking, and reporting.

Rust asks Lean semantic questions through a narrow, versioned Lean worker protocol. Lean types and Rust domain structs
are the semantic model. JSON and JSONL are subprocess transport encodings chosen by the worker runtime; they are not the
architecture. Rust must not inspect Lean expressions, recompute semantic fingerprints from pretty-printed types, or let
SQLite storage details leak into audit, ranking, or reporting code.

## References

- [Lean-Dup prompt sequence](/Users/jcreinhold/Code/prompts/lean-dup/README.md)
- [KanProofs mathlib duplicate audit](/Users/jcreinhold/Code/prompts/kanproofs-mathlib-duplicate-audit.md)
- [KanProofs internal duplicate audit](/Users/jcreinhold/Code/prompts/kanproofs-internal-duplicate-audit.md)
- [Loogle.lean](https://github.com/nomeata/loogle/blob/master/Loogle.lean)

## Problem And Thesis

The current Python tool proved useful capabilities: extract Lean declarations, compare local declarations against local
and external indexes such as mathlib, and turn confirmed duplication into cleanup hints. It is a source of lessons and
regression evidence, but it is not a compatibility target or the right production boundary. Python owns too much
semantic policy, cache behavior is scattered through audit code, report policy is mixed with candidate generation, and
large-audit performance depends on ad hoc bucket limits and hydration choices.

The production tool remains read-only, local, and deterministic. It does not edit audited workspaces, call network
services, use embeddings, or run broad proof search. Its job is to produce a high-precision cleanup queue with enough
evidence for a human or later cleanup prompt to act safely.

Lean owns:

- elaborated declaration facts from the Lean environment;
- Lean expression traversal and canonicalization;
- exact, permutation, connective, and conclusion fingerprints;
- role-aware statement features, including generated/private visibility facts where Lean can supply them;
- bounded semantic probes such as same-statement, safe binder reordering, structural specialization, and guarded
    reducible-definition equality.

Rust owns:

- workspace discovery, Lake orchestration, progress, profiling, and CLI workflow;
- worker process lifecycle, transport handling, and protocol validation;
- local and external SQLite indexes;
- cache-key construction and cache validation;
- weighted top-k retrieval without broad mathlib hydration;
- candidate ranking, review priorities, replacement hints, report rendering, baselines, and release engineering.

## Design Note

This overview owns the hidden knowledge that later prompts must not rediscover: the architectural doctrine, layer
responsibilities, design red flags, and migration order. Its smallest public interface is this charter. A later prompt
may refine a schema or module interface, but it must not silently reverse this boundary.

These decisions must not leak upward or sideways:

- Lean `Expr` structure and traversal rules;
- worker command names and transport framing outside the Rust worker runtime and protocol types;
- SQLite table names, row IDs, transaction order, bucket tables, or index insertion phases;
- bucket-cap policy, heap policy, ranking thresholds, and report formatting details.

Validated capabilities to preserve:

- `doctor`, `index`, `index-mathlib`, `audit`, and `show` workflows for full workspaces;
- private/public filtering and optional direct or named import comparison;
- mathlib and other external-index comparison through cached indexes;
- semantic probes for high-value candidates;
- ranked actionable findings and review priorities;
- text and JSON reports;
- progress and profile output that never corrupts JSON;
- read-only replacement/import hints with target declaration, import status, and bounded caller references;
- baseline diff and review workflows.

Python-era implementation behavior to discard:

- Python-side semantic policy over Lean statements;
- heuristic scoring leakage across candidate generation, ranking, and reporting;
- source parsing as a fallback for facts Lean should own;
- storage-aware audit code;
- report policy mixed with candidate generation;
- JSON/string-driven semantics;
- global pair materialization or broad bucket hydration as the primary large-audit safety mechanism;
- pass-through facade modules that forward another layer's interface without hiding a decision.

## Design It Twice

Two plausible designs were considered for the main boundary.

**Rejected: Rust-first semantic mirror.** Lean would emit declaration names and pretty-printed types, while Rust would
recompute fingerprints, statement features, and probe-like checks from strings. This appears convenient because Rust
already owns the CLI and index, but it leaks Lean semantics into the scale layer. It turns pretty text into a false
abstraction, makes binder dependency and definitional equality policy available to the wrong layer, and recreates the
current Python mistake in a faster language.

**Chosen: Lean semantic worker plus Rust audit engine.** Lean imports modules, extracts semantic rows, computes opaque
fingerprints and role-aware features, and answers bounded probe requests. Rust stores and combines those facts through
typed domain structs and index handles. This design is deeper because Rust has a smaller semantic interface, Lean
expression traversal is hidden, cache/index choices stay out of Lean, and ordinary audits do not depend on a Lean/Rust
FFI boundary. FFI remains an optional measured spike after batching and caching, not the production starting point.

## Public Architecture

The public architecture has five layers, each with a different abstraction.

1. **Lean worker package.** The worker exposes `extract`, `features`, `probe`, `doctor`, and `version`. These are
    semantic capabilities, not storage phases. The worker may report structured progress and diagnostics, but callers
    must not depend on Lean internal names, expression constructors, or traversal algorithms.

1. **Versioned worker protocol.** The protocol carries schema-versioned requests, responses, progress events, and
    structured errors. Declaration rows, feature rows, and probe results are caller-facing facts. Lean types and Rust
    domain structs are the semantic model; JSON and JSONL are transport encodings private to the worker runtime. Cache
    layout, SQLite tables, and report formatting are not protocol facts. Prompt 02 owns the executable schema.

1. **Rust CLI engine.** Rust discovers workspaces, resolves module roots, invokes Lake, locates and validates the
    worker, batches worker requests, tracks progress/profile events, and coordinates audit workflows. Command-line
    parsing must not leak into workspace discovery, Lake orchestration, indexing, ranking, or rendering modules.

1. **SQLite indexes.** The index layer is the persisted source of truth for local and external indexes. It exposes
    operations such as "build or reuse this index", "query postings for these semantic keys", and "hydrate these
    declaration handles". Callers do not know table names, row IDs as semantic identity, insertion order, or
    transaction sequencing.

1. **Retrieval, ranking, and reporting.** Retrieval uses exact/permutation/connective/conclusion fingerprints and
    role-weighted postings to return bounded candidate sets. Ranking consumes candidates, Lean probe results,
    source-reference facts, and review profiles to produce signals, blockers, priorities, actions, and replacement
    hints. Renderers consume a stable audit model to produce text, JSON, `show`, and baseline diff output.

## Information-Hiding Boundaries

The rewrite is organized around decisions that are likely to change.

- **Lean expression semantics.** Lean hides expression traversal, binder dependency, universe-sensitive
    canonicalization, generated declaration detection, and reducibility guards.
- **Worker protocol.** The protocol hides transport encoding, process framing, schema compatibility, stderr policy,
    progress delivery, structured error mapping, and worker-version validation from audit logic.
- **Index persistence.** The index layer hides SQLite schema, cache-key storage, posting tables, declaration hydration,
    and cache invalidation mechanics.
- **Retrieval strategy.** Retrieval hides rare-key weighting, broad-key suppression, top-k heap maintenance, pruning
    diagnostics, and origin-aware pairing.
- **Ranking policy.** Ranking hides confidence adjustment, blockers, suppression of weaker groups, review profiles, and
    recommended action selection.
- **Report rendering.** Rendering hides terminal layout, JSON shaping, `show` detail expansion, and baseline diff
    presentation.

Each boundary should have a capability-oriented interface. A caller should ask for the result it needs, not assemble the
lower-level steps itself.

## POSD Doctrine And Red Flags

This project follows these POSD rules as operational constraints:

- deep modules over shallow wrappers;
- information hiding over shared conventions;
- different layers with different abstractions;
- somewhat general interfaces that serve current workflows without encoding one caller's vocabulary;
- errors and special cases defined out of public interfaces where possible;
- performance designed around measured workloads and hidden critical-path simplification.

Later prompts must enforce these red flags:

- no table-name leakage outside the index module;
- no worker-command leakage into retrieval, ranking, or reporting;
- no Rust recomputation of Lean semantic facts from pretty text;
- no Python-era pass-through facade modules;
- no temporal decomposition where modules are named after audit phases but share the same hidden knowledge;
- no special-general mixture where KanProofs-specific cleanup policy enters general ranking or retrieval;
- no interface comments that describe SQLite layout, Lean traversal algorithms, or temporary migration details.

If a red flag is temporarily unavoidable, the implementing prompt must name it, explain why it is temporary, and name
the later prompt that removes it.

## Non-Goals

`lean-dup` is not a theorem prover or semantic search service. The default auditor does not perform broad proof search,
use embeddings, call network services, or depend on remote APIs. It does not rewrite Lean files. It does not use
Lean/Rust FFI as the primary route. The production path is a versioned worker API over subprocess transport unless
prompt 19 proves, with measurements, that an FFI migration is worth the safety and maintenance cost.

## Migration Map

The prompt sequence is the migration plan. The current Python implementation supplies lessons and regression examples
until switchover; it is not a parity target or the target architecture.

| Prompt | Responsibility                                                                                                                                    |
| ------ | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| 02     | Specify the versioned worker protocol/schema, cache keys, and worker error model.                                                                 |
| 03     | Create the Lean package foundation and worker command skeleton.                                                                                   |
| 04     | Move declaration extraction into Lean.                                                                                                            |
| 05     | Implement Lean canonical fingerprints.                                                                                                            |
| 06     | Emit Lean role-aware features and low-signal markers.                                                                                             |
| 07     | Implement batched Lean semantic probes.                                                                                                           |
| 08     | Create the Rust CLI and workspace foundation.                                                                                                     |
| 09     | Connect Rust to the Lean worker runtime.                                                                                                          |
| 10     | Build canonical local and external SQLite indexes.                                                                                                |
| 11     | Implement weighted top-k retrieval over index postings.                                                                                           |
| 12     | Add evaluation harnesses, gold positives, hard negatives, and metrics.                                                                            |
| 13     | Implement ranking, review actions, and replacement hints.                                                                                         |
| 14     | Implement reporting, `show`, review profiles, baselines, and diff mode.                                                                           |
| 15     | Profile realistic workloads and optimize measured bottlenecks.                                                                                    |
| 16     | Prove capability parity through regression validation, switch docs/install to the production binary, and remove superseded Python only when safe. |
| 17     | Validate full KanProofs and mathlib audits and update cleanup findings.                                                                           |
| 18     | Harden CI, packaging, versioning, reproducibility, and release docs.                                                                              |
| 19     | Optional FFI spike only after measured subprocess overhead dominates.                                                                             |

## Success Criteria

Mathlib indexing succeeds when a mathlib index can be built once, resolved by label, reused across audits, invalidated
by schema/toolchain/source changes, and queried without hydrating all mathlib declarations during ordinary local audits.

KanProofs auditing succeeds when full internal audits and targeted or full mathlib-comparison audits complete with
profile/progress data, regression validation preserves known inspected cleanup findings, and broad-head or generated
noise stays out of the default queue.

Report quality succeeds when default text output is a high-precision cleanup queue, `show` explains the evidence and
blockers for one group, replacement hints include import and caller impact, and JSON retains enough typed evidence for
deeper review without depending on terminal formatting.

Release readiness succeeds when CI covers the Rust engine, Lean worker, schema compatibility, fixture audits, and
default report behavior; version output records the binary, worker, protocol, index schema, and Git revision; docs
explain common workflows and the architecture boundaries; and the auditor remains read-only by default.

## Red Flag Review

- **Shallow module:** avoided by making this document a boundary charter rather than a file-by-file wrapper list.
- **Pass-through wrapper:** avoided by requiring capability-oriented Lean, worker, index, retrieval, ranking, and
    rendering interfaces.
- **Temporal decomposition:** avoided by organizing around hidden decisions, not audit execution order.
- **Information leakage:** explicitly guarded at Lean semantics, worker protocol, SQLite, retrieval, ranking, and
    rendering boundaries.
- **Special-general mixture:** avoided by keeping Lean semantic facts and core ranking general while KanProofs-specific
    expectations live in fixtures, reports, or review profiles.
- **Conjoined methods:** avoided by requiring typed outputs between subsystems rather than shared mutable phase state.
- **Hard-to-describe public API:** kept small at this level; prompt 02 owns detailed protocol names and schema fields.
- **Implementation details contaminating interface comments:** avoided by stating what callers may rely on, not how
    tables, caches, Lean traversals, or temporary migration scaffolding work.
