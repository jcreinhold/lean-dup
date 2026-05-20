# Vector Search Validation Redo

**Decision: keep vector search hidden and off-default for further study. Prompt 36 must ignore vector facts.**

Prompt 35Q reran validation after the 35L-35P repairs. The repaired code now records eligibility, top-k saturation,
vector-only and symbolic-only denominators, expanded-label facts, scorer variants, and warm-cache reuse. That makes the
negative result more trustworthy: the completed command-level quality workloads are still saturated, vector search did
not produce vector-only positives, `symbolic-plus-vector` introduced visible hard-negative leakage, and the
production-gate/manual run exceeded the current CPU/RSS budget before producing an artifact.

Prompt 35K remains historical inconclusive/negative evidence. This document is the authoritative vector-search gate for
Prompt 36.

## Design Note

This document owns the validation decision, workload interpretation, and the evidence boundary for threshold
calibration. It does not own model runtime, vector storage, candidate generation, report projection, or CLI behavior.

The smallest public interface is documentary: raw stage denominators, scorer-variant outcomes, top-k saturation,
runtime/RSS/cache cost, corpus reuse, artifact leak status, and the final remove/keep-hidden/allow-calibration decision.
Future prompts should read the decision, not re-interpret private artifacts.

The decisions that must not leak upward or sideways are FastEmbed runtime mechanics, model input prefixes, vector index
backend layout, database table or row names, ANN parameters, cache filenames, raw formal statements, source snippets,
worker rows, retrieval keys, and absolute private paths.

The preserved user-facing capability is the default symbolic duplicate audit and ordinary eval path: read-only,
embedding-free, vector-index-free, and unchanged by this validation.

The discarded behavior is accepting command success, saturated top-k runs, or a working vector database as search-quality
evidence. The validation decision uses labeled stage metrics, hard-negative leakage, non-saturation, warm-cache
reproducibility, runtime/RSS/cache cost, and artifact privacy.

## Design It Twice

Two validation designs were considered.

First, treat successful vector-index build/query and any nonzero vector candidate count as enough to keep vector search.
This is rejected. It repeats the 35K mistake: a working database and nonzero nearest-neighbor output can still be a
saturated smoke test with no vector-only recall gain and no ranking benefit.

Second, decide from repaired labeled stage metrics, vector-only positives, hard-negative leakage, top-k saturation,
runtime, cache cost, corpus reuse, and warm-cache reproducibility. This is the chosen design. It is deeper because eval
owns the decision and artifacts, search owns candidate generation and scorer facts, embedding owns model/profile/runtime
mechanics, and vector-index owns persistence and nearest-neighbor mechanics. No layer has to learn the private decisions
of another layer to interpret the result.

## Commands and Artifacts

Model preparation used the explicit hidden acquisition path:

```sh
target/release/lean-dup embedding prepare \
  --policy download-if-missing \
  --format json \
  --cache-root target/search-quality/vector-validation-redo/hf-cache
```

Prepared model facts: `BAAI/bge-small-en-v1.5`, profile `bge-small-en-v1.5`, dimension 384, `prepared`, 133806060
bytes, 4692 ms, 405520384-byte maximum resident set size from `/usr/bin/time -l`.

Completed artifacts:

| Workload | Artifact |
| --- | --- |
| cache-only missing model | `target/search-quality/vector-validation-redo/default-missing-model-vector-search.json` |
| default, `name-and-formal-statement`, cold | `target/search-quality/vector-validation-redo/default-name-run1-vector-search.json` |
| default, `name-and-formal-statement`, warm | `target/search-quality/vector-validation-redo/default-name-run2-vector-search.json` |
| default, `formal-statement` | `target/search-quality/vector-validation-redo/default-formal-statement-vector-search.json` |
| default, `informal-or-formal` | `target/search-quality/vector-validation-redo/default-informal-or-formal-vector-search.json` |
| hard-negatives, `name-and-formal-statement` | `target/search-quality/vector-validation-redo/hard-negatives-name-vector-search.json` |

The repaired realistic fixture evidence is currently unit-level:
`cargo test -p lean-dup-eval realistic_vector_validation_fixture_is_non_saturated_and_has_required_label_classes`. It
passed and records `top_k = 32`, eligible corpus size `72`, one vector-only positive, one symbolic-only positive, and
one vector-only lexical hard negative. That is useful regression coverage, but it is not a command-level corpus artifact
and cannot support Prompt 36 calibration by itself.

The production-gate/manual command was stopped after it became a cost and observability finding: 1022.28 s real time,
1516.51 s user time, 806.55 s sys time, 14351794176-byte maximum resident set size, and no completed artifact. Directory
prerequisites existed (`/Users/jcreinhold/Code/kan-proofs`, compiled library directory, and mathlib package directory),
but this run does not count as a completed manual pass.

## Cache-only Behavior

With an intentionally empty model cache, vector search wrote a deterministic skipped artifact:

| Field | Value |
| --- | --- |
| status | `skipped` |
| reason | `vector-model-not-prepared` |
| model | `BAAI/bge-small-en-v1.5` |
| profile | `bge-small-en-v1.5` |
| acquisition | `cache-only` |
| query eligibility | 13/42 eligible; skipped 22 `low-signal`, 2 `private`, 5 `unsupported-kind` |
| corpus eligibility | 6/8 eligible; skipped 2 `low-signal` |
| top-k | 32 |
| eligible corpus size | 6 |
| top-k saturated | true |

The ordinary `eval --suite default --format json` output remained parseable without vector flags.

## Completed Workload Metrics

All command-level quality workloads below are saturated (`top_k = 32`, eligible corpus size `6`). They are smoke and
regression evidence, not valid vector-retrieval quality evidence.

| Workload | Corpus | Sat | Sym gen recall | Vector top-k recall | Vector top-k precision | Vector-only positives | Vector-only HN | Symbolic-only positives | Merged recall | Visible precision | Visible HN |
| --- | ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| default/name cold | 6 | yes | 16/16 | 5/16 | 5/78 | 0/16 | 0/3 | 11/16 | 16/16 | 14/34 | 0/3 |
| default/name warm | 6 | yes | 16/16 | 5/16 | 5/78 | 0/16 | 0/3 | 11/16 | 16/16 | 14/34 | 0/3 |
| default/formal | 6 | yes | 16/16 | 5/16 | 5/78 | 0/16 | 0/3 | 11/16 | 16/16 | 14/34 | 0/3 |
| default/informal | 6 | yes | 16/16 | 5/16 | 5/78 | 0/16 | 0/3 | 11/16 | 16/16 | 14/34 | 0/3 |
| hard-negatives/name | 6 | yes | 1/1 | 1/1 | 1/78 | 0/1 | 0/5 | 0/1 | 1/1 | 1/34 | 0/5 |

Input-policy comparison is inconclusive. `formal-statement`, `name-and-formal-statement`, and `informal-or-formal`
produced the same stage denominators on the fixture corpus. The fixture has no usable informal text, and top-k is
saturated.

## Scorer Variants

The scorer-variant artifacts show why vector facts cannot enter calibration yet.

| Workload | Variant | Ranked recall / recall@10 | Visible precision | Visible hard negatives |
| --- | --- | ---: | ---: | ---: |
| default/name | `symbolic-only` | 16/16 | 14/34 | 0/3 |
| default/name | `vector-evidence-only` | 5/16 | 5/39 | 0/3 |
| default/name | `symbolic-plus-vector` | 16/16 | 15/107 | 2/3 |
| hard-negatives/name | `symbolic-only` | 1/1 | 1/34 | 0/5 |
| hard-negatives/name | `vector-evidence-only` | 1/1 | 1/39 | 0/5 |
| hard-negatives/name | `symbolic-plus-vector` | 1/1 | 1/107 | 1/5 |

`symbolic-plus-vector` does not satisfy the acceptance criteria: visible hard-negative leakage regresses on completed
workloads. The default workload also shows an invalid visible-group count in the variant artifact (`157/39`), which is a
separate artifact/scorer-variant bug to fix before any future validation claim.

## Runtime, RSS, and Cache Cost

| Workload | Corpus status | Embedding ms | Build ms | Query ms | Eval total ms | Wall time | Max RSS |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| prepare BGE-small | prepared | — | — | — | 4692 | 4.70 s | 405520384 B |
| default missing model | skipped | 0 | 0 | 0 | — | 4.47 s | 655245312 B |
| default/name cold | built | 161 | 7 | 24 | 1224 | 1.24 s | 655294464 B |
| default/name warm | reused | 0 | 0 | 22 | 1029 | 1.04 s | 655556608 B |
| default/formal | built | 157 | 3 | 20 | 2268 | 2.28 s | 655687680 B |
| default/informal | built | 151 | 3 | 20 | 1189 | 1.20 s | 655228928 B |
| hard-negatives/name | built | 158 | 3 | 14 | 1194 | 1.21 s | 655622144 B |
| production-gate/manual | blocked | — | — | — | — | 1022.28 s interrupted | 14351794176 B |

Cache sizes after completed validation: 129 MiB model cache, 51 MiB text-vector cache, 4.4 MiB vector corpus cache.

The warm default run reused the vector corpus (`corpus_status = reused`, build 0 ms) and text-vector cache (embedding 0
ms). Normalized pair rows for default/name run 1 and run 2 were byte-identical after sorting through `jq -S`, so warm
cache preserved pair ordering and metrics for the saturated fixture.

## Manual Suite Status

Manual KanProofs directory prerequisites existed, but the production-gate run did not complete. It was interrupted after
1022.28 s and a 14351794176-byte maximum resident set size with no vector-search artifact. This is not a pass and not a
mathlib-scale quality result. It is evidence that the hidden vector validation path needs progress reporting and a more
bounded manual workload before it can be used as a release gate.

## Boundary and Leak Evidence

Artifact leak checks over `target/search-quality/vector-validation-redo/*-vector-search.json` found no raw formal
statement text, source snippets, absolute private paths, model input prefixes, backend names, storage vocabulary,
SQLite/posting vocabulary, worker rows, retrieval keys, tokenizer terms, tensor terms, or vector-cache paths. A broader
check that included the word `static` matched the legitimate label field `static_evidence_acceptable`; that is label
metadata, not static-index or storage leakage.

Boundary verification is covered by `cargo test -p lean-dup-cli --test boundaries`: embedding runtime dependencies stay
inside `lean-dup-embedding`; vector database dependencies stay inside `lean-dup-vector-index`; search imports only
crate-root embedding/vector-index APIs for the hidden vector policy; report does not depend on embedding or vector-index
internals.

## Decision

Vector search remains hidden/off-default for further study. Prompt 36 must ignore vector facts.

The allow-calibration criteria were not met:

- No completed command-level non-saturated quality workload exists. The realistic non-saturated fixture is currently a
  unit test, not an eval artifact.
- Completed command-level workloads show `top_k_saturated = true`, so they cannot support retrieval-quality claims.
- Completed workloads show no vector-only positive gain (`0/16` on default, `0/1` on hard-negatives).
- `symbolic-plus-vector` increases visible hard-negative leakage (`2/3` on default and `1/5` on hard-negatives).
- Warm-cache reproducibility passed for the saturated default fixture, but that is insufficient without non-saturated
  quality evidence.
- Production-gate/manual validation exceeded current runtime/RSS acceptability and produced no artifact.
- A scorer-variant artifact bug produced impossible visible-group counts, so variant artifacts need repair before use in
  calibration.

Do not remove the vector experiment yet: the repaired unit fixture protects the intended contract, cache-only behavior
is deterministic, artifacts are privacy-safe, and warm reuse works on the fixture path. The next useful work is to make
the realistic non-saturated fixture a command-level eval workload, fix scorer-variant visible-group accounting, add
progress reporting for long validation, and rerun only then.

## Red Flag Review

- *Shallow module:* the validation decision uses raw denominators, cost, reuse, and leak evidence rather than command
  success.
- *Pass-through wrapper:* this document interprets stable facts from search/eval artifacts; it does not forward backend
  status as a quality decision.
- *Temporal decomposition:* build/query completion is not promotion evidence; ownership boundaries define what each
  result means.
- *Information leakage:* artifacts passed leak checks for raw text, runtime prefixes, backend/storage vocabulary, paths,
  and worker details.
- *Special-general mixture:* model/runtime mechanics remain in embedding, persistence remains in vector-index, search
  owns candidate/scorer facts, and eval owns validation decisions.
- *Conjoined methods:* eval measures search behavior and does not reconstruct candidate generation from embedding or
  vector-index internals.
- *Hard-to-describe public API:* the decision surface is policy ids, raw denominators, saturation, scorer variants,
  runtime/RSS/cache cost, and a single go/no-go decision.
- *Implementation details in interface comments:* this pass adds a validation document only; no public interface comments
  were changed.

## 35X Addendum: Bounded Manual Validation Contract

Prompt 35X repairs the operational gap found above: the interrupted production-gate/manual run produced no vector-search
artifact after a long runtime and high RSS. That run remains non-evidence for mathlib-scale quality. Future manual
validation must complete under explicit bounds or write a partial status artifact that explains why it did not.

Design Note:

- Hidden knowledge: eval owns workload lifecycle, bounds, partial status artifacts, runtime/RSS/cache accounting, and
  the final validation interpretation. CLI owns operator-visible progress. Search, embedding, and vector-index continue
  to expose only stable counters and statuses.
- Smallest public interface: hidden validation bounds and artifact fields for phase runtimes, RSS availability, cache
  sizes, vector corpus size, query count, eligible corpus size, top-k, saturation status, corpus reuse status,
  cold-build time, warm-open/query time, and artifact path.
- Non-leaking decisions: model runtime details, tokenizer files, vector storage layout, table names, backend names,
  worker rows, raw source text, model input prefixes, and private filesystem paths are not validation facts.
- Preserved capability: ordinary eval and audit remain symbolic and embedding-free unless the hidden vector experiment
  is explicitly requested.
- Discarded behavior: treating an opaque interrupted manual run as an acceptable validation attempt.

Design It Twice:

- *Wait for full mathlib validation without progress.* Rejected because interruption gives no stable result and hides
  cost.
- *Add prints inside model/vector internals.* Rejected because it spreads operator workflow knowledge into the wrong
  crates.
- *Make eval/CLI own progress, bounds, and partial artifacts.* Chosen because it keeps backend mechanics hidden while
  making validation cost and cache reuse visible.

Required large-workload behavior for Prompt 35Y:

- Report progress for model preparation, declaration loading, eligibility filtering, document construction,
  embedding/vector-cache lookup, corpus build/open/reuse, vector query, scoring variants, artifact writing, and leak
  checks.
- Enforce or record hidden bounds for maximum declarations, maximum queries, maximum runtime, and RSS observation
  threshold.
- Write partial artifacts for skipped, interrupted, timed-out, or budget-exceeded runs.
- Separate cold-build timing from warm-open/query timing.
- Record model cache size, text-vector cache size, vector corpus size, corpus reuse status, top-k, eligible corpus size,
  query count, and saturation status.
- Do not count skipped, interrupted, timed-out, budget-exceeded, or saturated runs as mathlib-scale retrieval-quality
  passes.

35X Red Flag Review:

- *Shallow module:* validation now has workflow-level observability and budget semantics, not just a command invocation.
- *Pass-through wrapper:* artifacts summarize stable cost/reuse facts rather than copying backend logs.
- *Temporal decomposition:* progress phases are workflow facts and do not make eval own embedding or vector-index
  internals.
- *Information leakage:* cost/progress artifacts exclude raw text, private paths, backend/storage vocabulary, model
  prefixes, tokenizer details, and worker rows.
- *Special-general mixture:* large-workload bounds remain hidden/manual validation controls and do not affect default
  audit/eval.
- *Conjoined methods:* eval owns validation lifecycle and decision evidence; search/embedding/vector-index own their
  respective hidden mechanics.
- *Hard-to-describe public API:* the validation surface is explicit bounds, phase timings, cache/corpus sizes, reuse
  status, and a stable completion status.
- *Implementation-detail comments:* comments must describe operator-visible progress/budget behavior, not storage or
  runtime layouts.
