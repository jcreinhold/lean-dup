# Semantic Search Clean Break

Prompt 35Q is not the final semantic/vector-search gate. It is historical evidence that
the hidden vector path can prepare a model, build and reuse a corpus on small inputs, and
write privacy-safe artifacts. It is also evidence that the design still cannot justify
threshold calibration: command-level quality workloads were saturated, realistic
non-saturated fixtures were not command-level artifacts, manual validation was opaque and
too expensive, and scorer-variant artifacts produced suspicious visible-stage counts.

Prompt 36 must ignore embedding and vector facts unless Prompt 35Y records a new
clean-break allow-calibration decision.

## Design Note

This document owns the clean-break semantic-search repair boundary. It does not own model
runtime, vector persistence, candidate generation implementation, report projection, CLI
behavior, or validation code.

The smallest interface it exposes is documentary: the remaining quality risks, the
ownership boundary for each volatile decision, the compatibility concepts that must not be
preserved, and the rule that Prompt 35Y is the next authoritative semantic/vector gate.

The decisions that must not leak upward or sideways are tokenizer/runtime details, model
input prefixes, final model input text, vector database backend or layout, ANN parameters,
database table or row vocabulary, vector-cache filenames, raw declaration text, worker
rows, retrieval keys, absolute private paths, and artifact construction internals.

The preserved user-facing capability is the ordinary symbolic duplicate audit and eval
path: read-only, deterministic, embedding-free, vector-index-free, and governed by the
existing report visibility policy.

The discarded behavior is compatibility-driven semantic search: rerank-only leftovers,
retired input strings, compatibility aliases, saturated fixture promotion, and quality
decisions based on command success rather than raw denominators, hard-negative survival,
cost, and reproducibility.

## Design It Twice

Three designs were considered.

First, keep the 35Q hidden vector path and only add more metrics. This is rejected.
Metrics would describe the current oddities more precisely, but the design would still
embed weak declaration text, rely on saturated command-level workloads, and leave scorer
variant behavior suspect.

Second, preserve retired rerank-only concepts as compatibility baselines while improving
vector search. This is rejected. Rerank-only over the symbolic pool is useful as
historical negative evidence, but preserving its code vocabulary or input policy would
force current search, eval, report, embedding, and vector-index code to share knowledge of
an experiment that is no longer part of the architecture.

Third, remove compatibility shells and write a clean semantic-search repair architecture
that names every volatile decision and assigns it to one owning crate. This is the chosen
design. It is deeper because each future change has one owner: search owns candidate
policy and vector evidence, embedding owns runtime and model wrapping, vector-index owns
persistence and nearest-neighbor mechanics, and eval owns truth, cost, and decisions. No
caller has to learn retired experiment details to interpret current semantic-search facts.

## Remaining Quality Risks

The clean-break repair sequence targets these named risks: fixed/saturated top-k,
weak embedded text, ineffective informal policy, vector score/scorer inconsistency,
tiny fixture overclaiming, command-level validation gaps, and long-run opacity.

Rerank-only leftovers or compatibility language can confuse the architecture even when the
old code path is removed. The current architecture should describe only hidden
candidate-generation over persisted vector corpora and the stable facts it produces.
Historical documents may remain as negative evidence, but code and current architecture
documents must not preserve retired rerank-only paths, retired input strings, or
compatibility aliases.

The fixed private top-k policy and saturated command-level corpora cannot prove vector
retrieval quality. A run with `top_k >= eligible_corpus_size` can verify plumbing and
artifact shape, but it does not test whether nearest-neighbor search selects useful
declarations from a large comparison corpus. Prompt 35Y may count only non-saturated
command-level quality workloads as retrieval evidence.

The semantic document policy is too weak when it embeds only declaration name plus formal
statement. For definition-like declarations, that can omit the implementation content
that may distinguish duplicates. For theorem-like declarations, proof bodies should not
enter the default semantic document, but statements and useful docstrings should be
available when the worker/index can supply them. Prompt 35S must make the content policy
truthful rather than preserving names that imply unavailable data.

The `informal-or-formal` policy is misleading if no informal text is extracted. A policy
that silently behaves like formal-only forever is a false abstraction. It should either be
removed or backed by real worker/index facts with availability counters.

Vector similarity has been treated inconsistently across candidate generation, ranking,
and visibility. Vector-generated recall asks whether the nearest-neighbor stage produced
a labeled pair. Ranked recall and visible precision ask whether search used evidence well
enough to rank and show useful pairs. Those are different questions. Search must own the
conversion from nearest-neighbor facts to stable pair evidence, and eval must measure
candidate generation separately from ranking and visibility.

Hidden scorer variants have suspicious visible group/count artifacts. Until scorer
variant artifacts are internally consistent, validation can record the bug but must not
use the variant output for calibration.

Tiny fixtures were overused as evidence. Unit fixtures are useful regression checks, but
Prompt 35Y needs command-level artifacts with non-saturated top-k, vector-only positives,
symbolic-only positives, lexical/name hard negatives, eligibility skip reasons, and leak
checks. Fixture-only evidence must not be described as mathlib-scale evidence.

Long manual validation lacks operator-grade observability. A hidden validation run that
consumes substantial runtime and RSS without progress or a partial status artifact is not
a usable quality gate. Large workloads need bounded execution, phase progress, cache
reuse facts, cold/warm timing separation, RSS/cache-size accounting, and explicit blocker
or interruption status.

## Clean Ownership Boundary

`lean-dup-search` owns semantic document policy, declaration eligibility, vector top-k
policy, vector evidence feature construction, symbolic/vector merge policy, hidden scorer
variant execution, and stage facts. Search may depend on `lean-dup-embedding` and
`lean-dup-vector-index` only through crate-root APIs for hidden vector experiments. It
must not know model prefixes, tokenizer details, backend types, database layout, table
names, ANN parameters, or cache paths.

`lean-dup-embedding` owns model profiles, model acquisition, input role wrapping, backend
runtime, vector normalization, runtime counters, and text-vector cache identity. It
exposes stable profile/runtime facts, not FastEmbed, ONNX, tokenizer, model-file, pooling,
or prefix details.

`lean-dup-vector-index` owns persisted declaration-vector corpora, nearest-neighbor
mechanics, backend details, corpus provenance, reuse, invalidation, and backend
diagnostics. Search receives declaration-corpus facts and nearest declarations with
stable scores where higher means closer; it does not receive backend rows, handles, table
names, or query plans.

`lean-dup-eval` owns labels, expanded label truth, artifact row truth, denominators,
workload lifecycle, cost accounting, leak checks, and go/no-go decisions. Eval measures
search behavior. It must not call embedding or vector-index to reconstruct candidates or
rerank pairs.

`lean-dup-report` projects stable status and artifact facts only. It must not depend on
model runtime, vector-index internals, raw semantic documents, vector database vocabulary,
or hidden artifact construction logic.

`lean-dup-cli` owns hidden flags and operator-visible progress, stdout, stderr, output
paths, and exit behavior. Ordinary audit and ordinary eval remain symbolic and do not
prepare models, build vector corpora, or query vector indexes.

## Prompt 35Y Gate

Prompt 35Y replaces Prompt 35Q as the next authoritative semantic/vector decision. It may
choose only one of three outcomes:

- remove the semantic vector experiment;
- keep it hidden and off-default for further study;
- allow Prompt 36 to use vector facts in threshold calibration.

Prompt 35Y may allow calibration only if all of these are true:

- non-saturated command-level workloads show vector-only recall gain;
- visible hard-negative leakage does not regress on any completed workload;
- scorer variant artifacts have internally consistent counts;
- warm-cache runs preserve metrics and pair ordering within documented deterministic tie
  rules;
- cold-build and warm-reuse CPU/RSS/cache costs are within documented thresholds;
- manual or mathlib-scale workloads either complete or record exact blockers without being
  counted as passes;
- artifacts pass leak checks and boundary tests prove exact dependency/import allowances.

Until then, Prompt 36 must ignore vector facts and treat the symbolic scorer as
authoritative.

## Red Flag Review

- *Shallow module:* this document defines ownership of volatile semantic-search decisions
  rather than forwarding the 35Q result as a quality gate.
- *Pass-through wrapper:* the target interfaces are stable semantic/vector facts, not
  wrappers around model runtime or vector database APIs.
- *Temporal decomposition:* the repair is organized by hidden knowledge and ownership, not
  by the execution order prepare, embed, build, query, rank, validate.
- *Information leakage:* model runtime, vector backend, storage layout, raw text, worker
  rows, retrieval keys, paths, and final model input text have explicit owners and must not
  appear in public search/eval/report APIs.
- *Special-general mixture:* search-specific policy stays in search, model mechanics stay
  in embedding, vector persistence stays in vector-index, and suite-specific truth stays
  in eval.
- *Conjoined methods:* eval measures search stage facts and labels; it does not recreate
  candidate generation or ranking from lower-level internals.
- *Hard-to-describe public API:* the intended decision surface is small: policy ids, raw
  denominators, top-k saturation, scorer variant id, cost, reproducibility, leak status,
  and a single go/no-go decision.
- *Implementation details in interface comments:* this pass adds an architecture document
  only. Future interface comments should describe stable caller obligations, not backend
  layout, model files, or temporary migration details.
