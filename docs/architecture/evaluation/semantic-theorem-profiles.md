# Semantic Theorem Profiles

Semantic search should find declarations that prove or define substantially the
same mathematical fact or reusable abstraction, even when theorem names,
variable names, statement order, proof route, and local helper names differ. The
input text for embeddings therefore must not be a thin `name + statement` blob.
It must be a deliberately constructed semantic profile with bounded, auditable
content.

This document is a design spec for the next semantic-search input boundary. It
does not change Prompt 35Y's decision: vector facts remain hidden and Prompt 36
must ignore them until repaired validation explicitly allows calibration.

## Design Note

The semantic theorem profile boundary owns declaration-document construction for
semantic search: statement normalization, lane construction, proof-neighborhood
summaries, content caps, content hashes, availability counters, and extraction
cost facts.

The smallest public interface is: build semantic retrieval documents for a set
of declarations under a named policy/version and return lane documents, hashes,
availability/cap counters, and cost facts. Callers should not learn the
extraction algorithm, proof-dependency classifier, batching strategy, Lean
worker mechanics, or text-vector formatting.

The decisions that must not leak upward or sideways are model input prefixes,
tokenizer/runtime behavior, vector database layout, raw worker rows, raw proof
bodies, source snippets, private filesystem paths, extraction batching, cache
layout, and detailed proof-dependency classification heuristics.

The preserved user-facing capability is the ordinary symbolic audit/eval path:
read-only, embedding-free, vector-index-free, and governed by the existing
visibility policy.

The intentionally discarded behavior is treating current `name-and-statement`
documents, retired rerank-only evidence, or saturated vector fixtures as
sufficient semantic-search input design.

## Design It Twice

Three designs were considered.

First, patch the current formatter by adding more lines to
`SearchEmbeddingDocumentPolicy`: statement sections, docstrings, proof
dependencies, and body summaries. This is rejected because it keeps the
volatile decisions in a shallow text formatter. It would make search, eval,
embedding, and validation tests learn too much about how semantic evidence is
assembled.

Second, introduce a semantic-document builder that emits one combined document
per declaration. This hides extraction better, but it blends distinct evidence:
statement meaning, proof neighborhood, names, docstrings, and implementation
summary all become one vector. Eval would struggle to distinguish "same theorem"
from "similar proof route."

Third, introduce a semantic-document builder that emits two lane documents per
actionable declaration. This is the chosen design. It is deeper because callers
ask for semantic retrieval documents and receive stable lane facts, while the
builder hides normalization, proof-summary construction, caps, caching,
batching, and extraction cost. Search can stage lane evidence, and eval can
measure each lane separately.

## Target Boundary

The semantic profile boundary should be owned inside `lean-dup-search`, behind
a private semantic-document module and exposed only through the
`lean-dup-search` crate-root facade. Search owns semantic document policy
because it owns duplicate/reuse meaning, declaration actionability, top-k
policy, vector evidence, and scorer variants. If the implementation later
requires an internal submodule split, the crate-root surface should remain the
same.

The boundary should expose stable facts, not implementation details:

- semantic profile policy id/version;
- lane id/version;
- declaration id/name/module/kind as weak context facts;
- in-memory lane text for hidden embedding calls;
- lane content hashes;
- statement/proof availability counters;
- cap and skip counts by stable reason;
- extraction/cache cost counters.

The boundary should not expose:

- raw proof bodies;
- final model input with role prefixes;
- tokenizer, tensor, ONNX/FastEmbed, or model-file details;
- vector database or vector-cache layout;
- worker protocol rows;
- source snippets or absolute private paths;
- proof-dependency classifier internals.

Embedding continues to own model profiles, role wrapping, model acquisition,
runtime, normalization, and text-vector cache identity. Vector-index continues
to own persisted corpora and nearest-neighbor mechanics. Eval owns labels,
artifact truth, denominators, cost interpretation, and go/no-go decisions.

## Lane Documents

Each actionable theorem-like declaration should produce two semantic lane
documents when facts are available.

### Statement-Meaning Lane

This is the primary lane for likely duplicate, subsuming, or equivalent theorem
retrieval.

It should include:

- normalized pretty formal statement/signature;
- binder and hypothesis sections;
- target/conclusion section;
- proposition shape and major logical connectives;
- important constants appearing in the statement;
- typeclass, structure, coercion, and universe/context signals when they affect
  meaning;
- docstring as secondary context when present;
- declaration/module/name as weak context, not the headline.

It should not collapse dependent types into lossy prose. The formal statement
must remain present in a recognizable form. Structured extracts are added to
make semantic similarity easier for the embedding model; they are not a
replacement for the formal statement.

Example shape:

```text
lane: statement-meaning
kind: theorem
context:
  module: Mathlib.Data.Set.Image
  declaration: Set.mem_image_of_mem
statement:
  theorem ... : y ∈ f '' s
binders:
  α β : Type*
  f : α → β
  s : Set α
  x : α
hypotheses:
  hx : x ∈ s
conclusion:
  f x ∈ f '' s
statement constants:
  Set.image, Membership.mem
docstring:
  <present only when available>
```

The exact section names are policy-owned and may change behind the boundary.
Artifacts record policy ids, hashes, and counters, not raw lane text.

### Proof-Neighborhood Lane

This is a secondary lane for recall and supporting evidence. It should help find
nearby facts with different surface formulations, but it is not enough by
itself to make a result visible.

It should include a bounded summary of direct proof dependencies grouped by
mathematical role:

- rewrite/simp dependencies;
- induction or recursion principles;
- algebraic, order, topology, category, set, relation, or logic lemmas;
- definitions unfolded or used as key concepts;
- coercion/typeclass/instance machinery when semantically relevant;
- module or namespace category summaries for dependencies.

It should not include full raw proof bodies, full tactic scripts, proof terms,
or unbounded dependency lists. Large dependency sets are capped per role, with
stable cap reasons and counts.

Example shape:

```text
lane: proof-neighborhood
kind: theorem
proof dependencies:
  rewrite:
    Set.mem_image, exists_prop
  set-theory:
    Set.image_eq_range
  logic:
    Exists.intro
dependency modules:
  Mathlib.Data.Set.Basic
  Mathlib.Logic.Basic
caps:
  simp dependencies capped: 24 of 93 retained
```

Proof-neighborhood lane evidence must be measured separately from
statement-meaning lane evidence. A proof-neighborhood hit should be treated as a
candidate-generation or ranking signal, not as proof that two statements are
duplicates.

## Kind-Aware Profiles

The document contract is kind-aware.

For theorem-like declarations, including theorems, lemmas, and axioms, the
statement-meaning lane is primary. The proof-neighborhood lane is secondary and
bounded.

For definitions and abbreviations, the statement lane should include signature
and type-level context. A body-behavior summary may be included when available
and bounded. The system should avoid embedding large raw bodies by default.

For structures, classes, and instances, semantic profiles should be included
only when they are actionable retrieval targets under an explicit policy.
Otherwise they should be skipped with stable eligibility reasons. Instance
documents must avoid becoming typeclass-noise magnets.

## Extraction and Caching

Extraction should use a hybrid model.

Cheap stable facts should be indexed normally:

- declaration identity;
- kind/module/visibility;
- normalized statement/signature;
- docstring when available;
- cheap statement constants and structural fingerprints;
- existing actionability and low-signal facts.

Rich semantic facts should be hidden semantic-search assets:

- proof dependency roles;
- grouped proof-neighborhood summaries;
- bounded definition body behavior summaries;
- extraction cap/skip metadata.

Rich facts should be cached per declaration by content hash and semantic profile
policy version. Per-declaration caching localizes invalidation and allows large
corpora to resume or reuse work across runs. Worker batching and cache storage
layout are implementation details hidden by the profile builder.

Mathlib-scale extraction must report:

- total declarations considered;
- eligible declarations;
- lane documents produced;
- cache hits and misses;
- skipped/capped reasons;
- extraction runtime;
- peak RSS or RSS-unavailable status;
- partial/budget-exceeded status.

Extraction caps must apply early enough to prevent a hidden semantic validation
run from enumerating or hydrating all of mathlib before discovering that the
semantic-search budget has been exceeded.

## Ranking and Validation

Candidate generation may use both lanes. Ranking and artifacts must keep the
lanes staged.

Statement-meaning lane evidence can support likely duplicate, subsumption, or
equivalent-theorem ranking when other search facts agree.

Proof-neighborhood lane evidence is secondary. It may promote a candidate for
further scoring, but it must not make a pair visible by itself unless later
statement evidence or proof-check evidence supports the match.

Eval must report lane-specific denominators:

- statement-lane top-k recall and precision;
- proof-neighborhood top-k recall and precision;
- vector-only positives by lane;
- vector-only hard negatives by lane;
- merged recall;
- ranked recall by scorer variant;
- visible precision and visible hard-negative survival;
- top-k saturation for each quality workload.

Prompt 36 may use semantic/vector facts only after a future validation document
shows non-saturated command-level gains, no visible hard-negative regression,
warm-cache reproducibility, acceptable CPU/RSS/cache cost, and clean leak and
boundary checks.

## Red Flag Review

- *Shallow module:* the proposed builder hides statement normalization,
  proof-neighborhood extraction, capping, caching, lane construction, and cost
  accounting behind one narrow request/result boundary.
- *Pass-through wrapper:* the boundary does not forward worker or embedding
  APIs. It turns declaration facts into semantic lane documents and stable
  counters.
- *Temporal decomposition:* the design is organized by hidden knowledge:
  document policy, extraction cost, model wrapping, vector persistence, and eval
  truth remain separately owned.
- *Information leakage:* raw proof bodies, worker rows, model prefixes, vector
  storage details, cache layout, and private paths are explicitly excluded from
  search/eval/report public artifacts.
- *Special-general mixture:* Lean duplicate/reuse meaning stays in search;
  model mechanics stay in embedding; corpus mechanics stay in vector-index;
  validation truth stays in eval.
- *Conjoined methods:* statement meaning and proof neighborhood are separate
  lanes so eval does not have to untangle blended evidence after the fact.
- *Hard-to-describe public API:* the intended surface is small: policy id,
  lane id, lane text for hidden embedding calls, content hashes, counters, and
  cost facts.
- *Implementation details contaminating interface comments:* future public
  comments should describe caller obligations and privacy guarantees, not
  worker batching, proof classifier heuristics, or cache layout.

## Local Implementation Decisions

The design intentionally keeps these choices inside the future semantic profile
builder rather than exposing them to search callers, eval artifacts, embedding,
or report:

- exact statement-section names and rendering;
- proof dependency role classifier;
- per-role proof dependency caps;
- definition body summary extraction;
- cache file layout;
- worker batching strategy.

Those choices must be made locally behind the semantic profile boundary. The
public contract is the policy/lane id, content hashes, counters, cost facts, and
privacy guarantees. Mathlib-scale cost measurements must validate the chosen
implementation before vector facts can influence threshold calibration.
