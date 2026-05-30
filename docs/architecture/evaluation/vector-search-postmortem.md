# Vector Search Postmortem

Prompt 35K is historical inconclusive and negative evidence, not a final judgment about vector search. It showed that
the current hidden vector path can prepare a model, build and reuse a persisted corpus, query nearest declarations, and
write privacy-safe artifacts. It did not provide a valid vector-search quality decision because the validation design
could not measure the capability we care about: vector-only candidate-generation recall over a realistic declaration
corpus.

Prompt 36 must ignore vector facts unless Prompt 35Q records a repaired allow-calibration decision.

## Design Note

This document owns the diagnosis of the flawed 35K validation and the repair boundary for the next prompts. It does not
own model runtime, vector persistence, candidate generation, labels, report rendering, or CLI behavior.

The smallest public interface it exposes is documentary: the repaired boundary assignment and the rule that vector facts
remain hidden and off-default until a repaired validation allows them. Future code should expose stable facts only:
eligibility counts, top-k policy and saturation status, vector-only and symbolic-only stage denominators,
label-expansion facts, and hidden scorer variant results.

The decisions that must not leak upward or sideways are tokenizer and model runtime details, input prefixes, vector
database backend names, persistence layout, ANN parameters, table or row names, cache paths, raw declaration text,
worker rows, SQLite posting vocabulary, and private corpus paths.

The preserved user-facing capability is the default symbolic duplicate audit: read-only, deterministic, embedding-free,
vector-index-free, and governed by the existing report visibility policy.

The discarded behavior is the Python-era style of ad hoc semantic search: manual text mixtures, one-off embedding
experiments, tiny corpora treated as quality evidence, and promotion decisions made without raw denominators,
hard-negative tracking, reproducibility, and scale evidence.

## Design It Twice

Three designs were considered.

First, treat Prompt 35K as the final vector-search decision and continue to threshold calibration. This is rejected. The
validation completed, but completion is not quality evidence. The fixture was saturated, symbolic retrieval already
found the positives, and vector score did not participate in ranking.

Second, patch individual metrics in place. This is also rejected. Adding a few fields to the artifact would leave the
same hidden assumptions in search and eval: corpus eligibility, query eligibility, top_k, label expansion, and vector
score policy would still be scattered across the system.

Third, write a postmortem architecture that names the invalid assumptions, then repair eligibility, artifacts, scoring,
and validation in separate prompts. This is the chosen design. It is deeper because each future prompt owns one hidden
decision and one artifact surface. Search owns candidate policy; embedding owns model/runtime decisions; vector-index
owns persistence and nearest-neighbor mechanics; eval owns denominators and decisions. The quality decision is no longer
smeared across search, eval, embedding, and vector-index code.

## POSD Diagnosis

Prompt 35K was not a valid vector-search test because `top_k >= eligible_corpus_size` saturated the corpus. A saturated
top-k run cannot show that nearest-neighbor search is selecting useful declarations from a large comparison corpus; it
mostly shows that the corpus was small enough to return nearly everything.

The fixture could not demonstrate vector-only recall. Symbolic retrieval already found all fixture positives, so there
was no labeled positive that vector search could recover after symbolic generation missed it.

Vector score was metadata, not search evidence. The artifact recorded similarity facts, but the scorer did not consume
vector similarity as a ranking feature. That means the validation did not test whether vector evidence can improve rank
or visibility decisions after generation.

Corpus eligibility was too loose. Generated, private, low-signal, synthetic, and non-actionable declarations could enter
vector corpora without a named policy that explains why they belong. That makes both runtime and quality difficult to
interpret.

Artifact label truthfulness was incomplete. Expanded cluster positives and hard negatives could appear unlabeled in pair
rows because label expansion was not attached to each row. This makes a reader distrust the denominator even when the
aggregate metric is computed correctly.

Corpus eligibility, query eligibility, and top-k policy were implicit. A reader could not tell which declarations were
eligible, why others were skipped, whether top-k was saturated, or whether vector-only positives and hard negatives were
being measured at the right stage. That made the result easy to overclaim.

## Repaired Boundaries

`lean-dup-search` owns candidate policy, vector corpus eligibility, query eligibility, top-k policy, vector score
feature construction, and merging symbolic and vector candidates. It may consume only crate-root embedding and
vector-index capabilities for the hidden vector policy. It must not know model prefixes, tokenizer rules, database
tables, backend paths, ANN parameters, or vector-cache layout.

`lean-dup-embedding` owns model profiles, model acquisition, input role wrapping, CPU runtime, normalization, runtime
counters, and the per-text vector cache. Adding or changing a model must stay inside this crate unless a new model
family requires one backend adapter.

`lean-dup-vector-index` owns persisted vector corpus storage, nearest-neighbor mechanics, backend details, corpus
provenance, reuse, invalidation, and backend-level diagnostics. Search receives nearest declaration facts, not backend
rows or database handles.

`lean-dup-eval` owns denominators, label expansion, artifact truthfulness, workload lifecycle, validation decisions, and
go/no-go evidence. Eval measures search behavior; it must not reconstruct candidate generation from embedding or
vector-index internals.

`lean-dup-report` may project stable status and artifact facts. It must not depend on model runtime, vector-index
internals, raw declaration documents, or backend vocabulary.

`lean-dup-cli` owns hidden flags and operator-visible file, stdout, and stderr behavior. Hidden flags may request vector
experiments, but ordinary audit and ordinary eval remain symbolic by default.

## Repair Sequence

Prompt 35M adds explicit search-owned vector corpus and query eligibility policy, including skip counts and top-k
saturation reporting.

Prompt 35N fixes vector artifacts and label truthfulness before further quality claims. Pair rows must be deduplicated
by unordered declaration pair and must carry expanded cluster positive and hard-negative facts.

Prompt 35O makes vector similarity a hidden search-owned pair feature consumed by scorer variants. It measures
candidate-generation recall separately from ranking and visible-stage effects.

Prompt 35P adds realistic validation corpora: non-saturated top-k, vector-only positives, lexical hard negatives, and
optional KanProofs/mathlib workloads when compiled oleans exist.

Prompt 35Q reruns validation and records the only vector decision Prompt 36 may read: remove vector search, keep it
hidden/off-default, or allow vector facts into threshold calibration.

## Red Flag Review

- *Shallow module:* the repair defines ownership boundaries around hidden knowledge, not a checklist of commands.
- *Pass-through wrapper:* the future vector-index and embedding surfaces must expose declaration-corpus and embedding
  capabilities, not backend APIs.
- *Temporal decomposition:* the repair is organized by volatile decisions rather than the order build, embed, query,
  rank, evaluate.
- *Information leakage:* model runtime, vector backend, cache layout, raw text, table names, and worker rows have
  explicit owners and must not appear in search/eval/report public APIs.
- *Special-general mixture:* vector mechanics stay in vector-index, model mechanics stay in embedding, lean-dup
  candidate policy stays in search, and suite-specific decisions stay in eval.
- *Conjoined methods:* search candidate generation and eval validation are kept separate so eval measures search instead
  of recreating it.
- *Hard-to-describe public API:* the intended public facts are stage counts, eligibility summaries, top-k policy, label
  expansion, and hidden scorer variants.
- *Implementation details in interface comments:* backend and runtime names may appear in architecture evidence, but
  must not become interface comments or artifact schema vocabulary outside their owning crates.
