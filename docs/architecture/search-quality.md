# Search Quality

`lean-dup` does not search for one kind of duplicate. Different relationships need different evidence and different
review actions. This charter defines the match classes, the four-stage pipeline that produces them, and the quality
contract release hardening waits on.

## Match classes

| Class | Meaning | Default evidence standard |
| --- | --- | --- |
| **exact theorem duplicate** | two theorem-like declarations have the same proposition after binder-preserving normalization | proof-grade if source-backed; strong static evidence may stay diagnostic for static indexes |
| **binder/permutation duplicate** | same statement with safe binder reordering, premise permutation, or equivalent connective shape | proof-grade for source-backed comparisons |
| **reducible-definition duplicate** | two reducible definitions compute to the same value or expose equivalent reducible bodies | proof-grade; opaque or unreducible bodies must be classified before visibility |
| **replacement candidate** | a local declaration can likely be replaced by an imported or mathlib one without losing callers | proof-grade source-backed plus source/import impact, or explicitly static when the index is static |
| **specialization/generalization** | one declaration is a useful specialization or generalization of another, not a literal duplicate | verified semantic relation, or high-confidence diagnostic under non-default profiles |
| **local cleanup duplicate** | two local declarations duplicate or alias each other, or represent mergeable cleanup | strong local static, source-clone, or semantic evidence |
| **static structural similarity** | declarations share indexed structure but no source-backed proof-grade check is available | static only; must not be reported as proof-grade |
| **non-actionable related theorem** | mathematically related but no replacement, duplicate, or cleanup action | hidden by default; diagnostic/API-design profiles only |
| **hard negative** | a known non-match that must not become visible as actionable under the default policy | tracked at every stage; counted as leakage if shown |

The class is part of the quality target. A system that finds many related theorems but cannot tell replacement
candidates apart from hard negatives is not production-ready, however high its aggregate candidate count.

## The four stages

Each stage has one objective and one failure mode. A bug at any stage looks different from a bug at any other.

### Candidate generation: recall

Place plausible pairs into the candidate set from indexed facts without requiring proof-grade evidence. Measured by
candidate-generation recall, candidate volume, origin breakdown, and hard-negative entry by feature family.

Generation may be noisy; measure the noise by feature family, origin, and hard-negative entry. Do not hydrate broad
mathlib matches unboundedly, and do not encode final visibility decisions as retrieval shortcuts.

### First-stage ranking: cheap precision

Order and prune generated candidates using typed pair features, provenance, source facts, and cheap blockers before
expensive Lean probes. Measured by top-k recall, hard-negative survival, visible-queue precision before semantic
reranking, and runtime.

Ranking consumes feature facts, not SQLite rows, raw Lean expressions, or transport records. Future calibrated scoring
should make weights and ablations explicit while keeping the user interface small.

### Semantic verification: proof-grade reranking

Turn selected source-backed candidate pairs into typed semantic evidence: verified, rejected, or unavailable with a
stable reason. Not a broad search engine over every weak feature overlap.

Proof-grade evidence requires source-backed declarations importable in the current Lean worker environment. Static
external indexes remain useful but cannot silently produce proof-grade claims. Probe failures must be recoverable and
counted by reason, obligation kind, module, and origin.

### Report visibility: calibrated actionability

Decide what appears in the default review queue from match class, evidence mode, semantic evidence, profile, source
impact, and blockers. The default queue prefers high precision; broad findings remain available through explicit
profiles or noise controls; empty queues must explain what was hidden and why.

## Why typed stages, not one duplicate score

A single duplicate score is easy to describe and too shallow to be useful. It hides the difference between exact theorem
equality, reducible definitions, replacement candidates, weak structural similarity, and hard negatives. It also
encourages ranking, semantic probes, and report visibility to optimize one mixed bucket, which is exactly how weak
mathlib matches end up looking actionable.

Splitting candidate generation, ranking, semantic verification, and visibility into stages with separate objectives
hides implementation details while making failures diagnosable.

## Guardrails

- Lean owns semantic facts that require the elaborated environment; Rust owns retrieval, scoring, evaluation, reporting,
  persistence, workflow.
- Source-backed and static evidence stay explicit. A label such as `mathlib` never implies proof-grade by itself.
- Semantic probes stay bounded and recoverable. Raising heartbeats, broadening budgets, or parallelizing weak probes is
  not a substitute for better planning and evidence yield.
- Retrieval/ranking changes report stage-level recall, visible precision, hard-negative leakage, candidate volume, and
  runtime before changing default behavior.
- Embeddings are not accepted default architecture. Any embedding experiment must be hidden, off-by-default, run local
  inference, acquire models only through explicit preparation, and be measured against the symbolic baseline.
- Default report visibility must not collapse all match classes into one duplicate bucket. A visible finding names what
  kind of match it is and what evidence makes it actionable.
