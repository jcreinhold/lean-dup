# Candidate Source Boundary

Date: 2026-05-21

This document records the Prompt 68 boundary refactor. The change keeps symbolic retrieval behavior stable while making
candidate generation observable through search-owned source facts rather than retrieval internals.

## Design Note

Candidate sources own source-specific generation policy, feature planning, posting fanout, per-anchor top-k selection,
and saturation facts. Search owns merge policy and the stable source-fact vocabulary. Eval owns denominators and label
joins. Report owns aggregate projection and wording.

The smallest public interface is a candidate-source fact attached to an observed declaration pair: source id, source
family, stable pair id, declaration ids, candidate origin, generation rank when known, top-k status, saturation, and
leak-safe feature-family labels. Callers do not receive retrieval keys, posting-table shape, fanout caps, raw feature
keys, worker rows, probe obligations, raw expressions, private paths, or vector facts.

The preserved user-facing capability is the existing symbolic cleanup audit and eval behavior. The Python-era behavior
discarded is making downstream stages infer candidate-source truth from syntactic retrieval rows or from one mixed
duplicate score.

## Design It Twice

Three designs were considered.

1. Keep `retrieval.rs` as the only candidate source and add semantic lanes later. This was rejected because eval would
   still observe only `symbolic_generated` and `merged_generated`, so the next lane would require another public-shape
   migration.
2. Add a trait per candidate source and expose source-specific DTOs. This was rejected because there is still only one
   in-tree source in the core path; a trait would be speculative and source-specific DTOs would leak volatile source
   decisions upward.
3. Make search own a concrete candidate-source workflow that emits stable source facts. This was chosen. It hides
   retrieval mechanics, gives eval source-stage denominators, and leaves room for Prompt 69 to add a second source
   without public API churn.

## Current Boundary

Prompt 67 recorded these baseline facts:

| Fact | Value |
| --- | ---: |
| Fast eval source stages | `symbolic_generated == merged_generated` |
| KanProofs retrieval candidates | 558,109 |
| KanProofs review candidate pairs | 13,581 |
| KanProofs visible groups with `--private` | 5 |

Prompt 69 added `lean-semantic` candidate-source facts behind the same boundary. `symbolic` and `merged` therefore no
longer have to be identical: a pair can be selected by symbolic retrieval, by a Lean semantic lane, or by both.

## Source-Fact Contract

Each generated observed pair carries one or more candidate-source facts. Current source ids include:

- `symbolic-retrieval` with source family `symbolic`;
- `lean-semantic.statement-meaning.v1` with source family `lean-semantic`;
- `lean-semantic.binder-role-shape.v1` with source family `lean-semantic`;
- stable unordered declaration-pair id;
- left and right declaration ids;
- candidate origin;
- generation rank if selected by the bounded top-k stage;
- top-k status: selected or generated but not selected;
- source saturation for the anchor;
- leak-safe feature-family labels.

Search owns this vocabulary. Eval may count by `source_family` and `source_id`, but it must not reconstruct retrieval
keys, posting layout, scorer internals, or index storage facts.

## Verification Baseline

The boundary must preserve default and hard-negative visible behavior unless a focused test records an intentional
delta. After Prompt 69, eval reports source-family/source-id counts and `candidate_source_recall` so semantic-lane-only,
symbolic-only, and merged recall are separately measurable.

## Red Flag Review

- Shallow module: the boundary adds source-fact normalization and top-k/saturation facts; it is not a pass-through
  wrapper over retrieval rows.
- Pass-through wrapper: eval consumes stable facts rather than forwarding retrieval contribution keys.
- Temporal decomposition: the split follows hidden knowledge ownership, not the chronological retrieval/ranking order.
- Information leakage: feature keys, posting shape, fanout caps, worker rows, raw expressions, paths, and vector facts
  stay private.
- Special-general mixture: the interface is general over candidate sources, while supported source ids remain concrete
  and versioned.
- Conjoined methods: source generation, eval denominators, and report projection remain separately owned.
- Hard-to-describe public API: a candidate-source fact is one stable source explanation for one generated pair.
- Implementation-detail comments: public comments describe source facts and invariants, not retrieval algorithms.
