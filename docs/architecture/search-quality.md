# Search Quality Charter

This document defines the retrieval, ranking, semantic-verification, and visibility problem for `lean-dup`. It is the
quality contract for prompts 28 through 37: later prompts may change labels, metrics, features, scoring, probes, or
thresholds, but they must preserve this task taxonomy and stage separation unless they update this charter with new
evidence.

## Design Note

This document owns the hidden knowledge needed to reason about search quality: match-class taxonomy, stage objectives,
quality failure interpretation, proof-grade versus static evidence boundaries, and the no-go rules for default report
visibility.

Its smallest public interface is one architecture document. Evaluation, retrieval, ranking, semantic verification, and
reporting code should expose named metrics and typed evidence that fit this taxonomy; users and future prompts should
not need to learn retrieval keys, SQLite tables, Lean expression traversal, probe chunking, or scoring constants to
understand what the system is trying to prove.

These design decisions must not leak upward or sideways:

- indexed key families, posting table layout, hydration strategy, and candidate-pruning mechanics;
- ranking constants, threshold experiments, feature-vector layout, and ablation plumbing;
- semantic-probe chunking, heartbeat recovery, JSONL transport, cache-key construction, and Lean reduction strategy;
- eval label storage format, private KanProofs paths, and fixture construction details;
- optional local embedding inputs, model caches, and experiment wiring before Prompt 35 measures them.

The validated user-facing capability to preserve is read-only local duplicate auditing: build or reuse cached indexes,
compare local declarations with local/imported/mathlib/external evidence, run bounded source-backed semantic
verification when available, and report actionable review groups with stable text and JSON explanations.

Python-era behavior intentionally discarded:

- treating Python parity, Python cache layout, or Python ranking heuristics as production architecture;
- relying on anecdotal inspection rather than labeled denominators;
- collapsing all possible findings into one generic duplicate score;
- exposing broad noisy candidate dumps as the normal review experience.

## Design It Twice

**Rejected: one generic duplicate score.** A single score is easy to describe but too shallow. It hides the difference
between exact theorem equality, reducible definitions, replacement candidates, weak structural similarity, and hard
negatives. It also encourages ranking, semantic probes, and report visibility to optimize one mixed bucket, which is
exactly how weak mathlib matches can look actionable.

**Chosen: task taxonomy plus stage objectives.** The search boundary is deeper when each stage has one job and each
match class has a clear interpretation. Candidate generation optimizes recall; first-stage ranking optimizes cheap
precision; semantic verification provides proof-grade reranking for eligible source-backed pairs; report visibility
calibrates actionability. This hides implementation details while making failures diagnosable: a positive can be lost
at generation, ranking, verification, or visibility, and those are different bugs.

## Match Classes

`lean-dup` does not search for one undifferentiated kind of duplicate. It searches for typed relationships with
different evidence requirements and different review actions.

| Match class | Meaning | Default evidence standard |
| --- | --- | --- |
| exact theorem duplicate | Two theorem-like declarations have the same proposition after normalization that preserves binder meaning. | Proof-grade semantic evidence when source-backed; strong static evidence may remain diagnostic for static indexes. |
| binder/permutation duplicate | The same theorem statement appears with safe binder reordering, premise permutation, or equivalent connective shape. | Proof-grade semantic evidence for source-backed comparisons. |
| reducible-definition duplicate | Two reducible definitions compute to the same value or expose equivalent reducible bodies. | Proof-grade semantic evidence; opaque or unreducible bodies should be classified before visibility. |
| replacement candidate | A local declaration can likely be replaced by an imported or mathlib declaration without losing callers. | Proof-grade source-backed evidence plus source/import impact, or explicitly static evidence when the comparison index is static. |
| specialization/generalization | One declaration is a useful specialization or generalization of another rather than a literal duplicate. | Verified semantic relation or high-confidence diagnostic evidence under non-default profiles. |
| local cleanup duplicate | Two local declarations duplicate each other, alias each other, or represent mergeable local cleanup. | Strong local static evidence, source-clone evidence, or semantic evidence when available. |
| static structural similarity | Declarations share indexed structure, constants, roles, or fingerprints, but no source-backed proof-grade check is available. | Static evidence only; it must not be reported as proof-grade. |
| non-actionable related theorem | Declarations are mathematically related but do not imply a replacement, duplicate, or cleanup action. | Hidden by default; useful for diagnostics or broad/API-design profiles. |
| hard negative | A known non-match that must not become visible as actionable under the default policy. | Must be tracked through every stage and counted as leakage if shown. |

The class is part of the quality target. A system that finds many related theorems but cannot distinguish replacement
candidates from hard negatives is not production-ready, even if its aggregate candidate count is high.

## Stage Objectives

The pipeline has four search-quality stages. Each stage has a distinct objective and a distinct failure mode.

### Candidate Generation

Candidate generation means high recall. Its job is to place plausible pairs into the candidate set using indexed facts
without requiring proof-grade evidence. It should be evaluated by candidate-generation recall, candidate volume, origin
breakdown, and hard-negative entry by feature family.

Candidate generation is allowed to be noisy, but the noise must be measured. It must not hydrate broad mathlib matches
unboundedly, and it must not encode final visibility decisions as retrieval shortcuts.

### First-Stage Ranking

First-stage ranking means cheap precision. Its job is to order and prune generated candidates using typed pair
features, provenance, source facts, and cheap blockers before expensive Lean probes. It should be evaluated by top-k
recall, hard-negative survival, visible-queue precision before semantic reranking, and runtime.

Ranking should consume feature facts, not SQLite rows, raw Lean expressions, or transport records. Future calibrated
scoring should make weights and ablations explicit while keeping the normal user interface small.

### Semantic Verification

Semantic verification means proof-grade reranking. Its job is to turn selected source-backed candidate pairs into typed
semantic evidence: verified, rejected, or unavailable with a stable reason. It is not a broad search engine over every
weak feature overlap.

Proof-grade evidence requires source-backed declarations importable in the current Lean worker environment. Static
external indexes remain useful, but they cannot silently produce proof-grade claims. Probe failures must be recoverable
and counted by reason, obligation kind, module, and origin.

### Report Visibility

Report visibility means calibrated actionability. Its job is to decide what appears in the default review queue, given
match class, evidence mode, semantic evidence, profile, source impact, and blockers.

The default queue should prefer high precision. Broad or exploratory findings may remain available through explicit
profiles or noise controls, but empty queues must explain what was hidden and why.

## Current Failure Evidence

Prompt 27 proved that the Rust/Lean aggregate evaluation command can run, but it did not prove search quality. The
current production-gate artifact reports:

- KanProofs/mathlib recall@10: `0/11`;
- KanProofs/mathlib hard-negative leakage: `3/4`;
- aggregate recall@10: `15/32`;
- aggregate hard-negative hits: `3/16`.

The aggregate `eval status = ok` means command completion and current gate execution, not release-quality search. A
professional search-quality gate must explain where positives are lost and where hard negatives survive: candidate
generation, ranking, semantic verification, or report visibility.

## Architecture Guardrails

- Lean owns semantic facts that require the elaborated environment; Rust owns retrieval, scoring, evaluation,
  reporting, persistence, and workflow.
- Source-backed and static comparison evidence must remain explicit. A label such as `mathlib` never implies
  proof-grade evidence by itself.
- Semantic probes must stay bounded and recoverable. Raising heartbeats, broadening probe budgets, or parallelizing
  weak probes is not a substitute for better planning and evidence yield.
- Retrieval/ranking changes must report stage-level recall, visible precision, hard-negative leakage, candidate volume,
  and runtime before changing default behavior.
- Local embeddings are not accepted architecture before Prompt 35. Any embedding experiment must be hidden,
  off-by-default, local-only, and measured against the symbolic baseline.
- Default report visibility must not collapse all match classes into one duplicate bucket. A visible finding should
  name what kind of match it is and what evidence makes it actionable.

## Prompt Sequence Role

Prompts 28 through 37 are the search-quality prerequisite for release hardening. They define labels, stage metrics,
feature artifacts, candidate generation, calibrated symbolic scoring, semantic reranking, optional embedding
experiments, threshold calibration, and final search-quality validation.

Release hardening may proceed only after search-quality validation either closes the quality blockers or records a
measured no-go.

## Red Flag Review

- **Shallow module:** avoided. The charter hides match taxonomy and stage objectives behind one document instead of
  scattering them across retrieval, ranking, eval, and report prompts.
- **Pass-through wrapper:** avoided. The document is not a restatement of the prompt sequence; it defines the quality
  contract later prompts must satisfy.
- **Temporal decomposition:** avoided. The charter is organized by match class and search stage, not by implementation
  order.
- **Information leakage:** avoided. It names stable evidence concepts without exposing retrieval keys, SQLite tables,
  Lean expression details, worker framing, or probe chunks.
- **Special-general mixture:** contained. KanProofs provides current failure evidence, but the taxonomy and stage
  objectives are corpus-independent.
- **Conjoined methods:** no remaining red flag. Candidate generation, ranking, semantic verification, and report
  visibility have distinct objectives and failure modes.
- **Hard-to-describe public API:** no remaining red flag. The public interface is one architecture document plus later
  named metrics and artifacts.
- **Implementation details contaminating interface comments:** avoided. The document describes production-quality
  evidence and actionability, not storage layout, transport fields, or temporary prompt mechanics.
