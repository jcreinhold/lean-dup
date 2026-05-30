# Semantic Search Validation Decision

**Decision: keep semantic vector search hidden and off-default for further study. Prompt 36 must ignore vector facts.**

Prompt 35Y is the authoritative semantic/vector gate for Prompt 36. Prompt 35Q remains historical insufficient evidence:
it correctly refused calibration, but it was written before the clean-break sequence repaired command-level fixtures,
model/input-format selection, semantic document policy, vector scorer variants, and bounded validation progress.

The repaired validation shows useful plumbing and one controlled vector-only recall gain, but it still fails the
allow-calibration gate. The non-saturated command-level fixture is deterministic and warm-cache reproducible, yet the
vector scorer variants introduce visible hard-negative leakage. The default and hard-negative suites are still saturated
when run through the fixture profile. Manual mathlib-scale validation remains operationally unsafe because pre-vector
mathlib declaration enumeration can run for a long time before vector bounds take effect.

## Design Note

This document owns the validation decision, workload interpretation, and the evidence boundary for threshold
calibration. It does not own semantic document construction, model runtime, vector persistence, candidate generation,
report projection, or CLI parsing.

The smallest public interface is documentary: one remove/keep-hidden/allow-calibration decision, plus raw denominators,
scorer-variant outcomes, top-k saturation status, runtime/RSS/cache cost, warm-cache reproducibility, artifact privacy,
and manual-workload blockers.

The decisions that must not leak upward or sideways are model prefixes, tokenizer behavior, backend distance semantics,
vector database layout, cache filenames, raw declaration text, source snippets, worker rows, retrieval keys, private
filesystem paths, and proof-body content.

The preserved user-facing capability is the default symbolic duplicate audit and ordinary eval path: read-only,
embedding-free, vector-index-free, and unchanged by this validation.

The Python-era behavior intentionally discarded is accepting a working embedding/vector database, a saturated fixture,
or a retired rerank-only experiment as enough evidence for threshold calibration.

## Design It Twice

Three validation designs were considered.

First, treat successful model/vector-index execution as enough to keep semantic vector search. This is rejected because
a database can build, reopen, and return nearest neighbors without improving duplicate-search quality.

Second, decide from command-level fixture evidence only. This is better than the earlier unit-only evidence, but still
too narrow: a deterministic fixture can prove artifact and scorer plumbing while saying little about mathlib-scale cost
or real model behavior.

Third, decide from repaired command-level fixtures, bounded manual/scale workloads when available, scorer variants,
hard-negative survival, top-k saturation, reproducibility, and cost. This is the chosen design. It is deeper because
eval owns the decision and artifacts; search owns candidate generation, vector evidence, and scorer variants; embedding
owns profile/runtime/wrapping details; vector-index owns persistence and nearest-neighbor mechanics. No layer has to
reverse-engineer another layer's private state to interpret the result.

## Commands and Artifacts

Artifacts were written under `target/search-quality/semantic-validation-decision/`.

| Workload | Artifact | Status |
| --- | --- | --- |
| ordinary default eval | `default-ordinary.json` | parseable, vector-free |
| cache-only missing BGE-small model | `default-missing-model-vector-search.json` | skipped |
| vector fixture, `name-and-statement`, cold | `vector-fixture-cold-vector-search.json` | ok |
| vector fixture, `name-and-statement`, warm | `vector-fixture-warm-vector-search.json` | ok, corpus reused |
| vector fixture, `statement` | `vector-fixture-statement-vector-search.json` | ok |
| vector fixture, `definition-aware` | `vector-fixture-definition-aware-vector-search.json` | ok |
| default suite with fixture profile | `default-fixture-vector-search.json` | ok, saturated |
| hard-negative suite with fixture profile | `hard-negatives-fixture-vector-search.json` | ok, saturated |
| production gate without manual workspace args | `production-gate-fixture-vector-search.json` | incomplete aggregate |
| bounded manual-internal | `manual-internal-fixture-vector-search.json` | budget-exceeded |
| bounded manual-mathlib | `manual-mathlib-interrupted-vector-search.json` plus stderr log | interrupted operational blocker |

The ordinary command `eval --suite default --format json` remained parseable and contained no `vector_search` or
`vector_candidates` fields.

The cache-only missing-model run produced a deterministic skipped artifact with reason `vector-model-not-prepared`. It
reported 13/42 query declarations eligible, 6/8 corpus declarations eligible, `top_k = 32`, `eligible_corpus_size = 6`,
and `top_k_saturated = true`.

## Completed Workload Metrics

The only completed non-saturated command-level workload is `vector-fixture`: `top_k = 32`, eligible corpus size `72`,
query count `72`, and `top_k_saturated = false`. It uses the deterministic fixture profile, so it validates the full
hidden command/artifact/scorer path rather than real embedding quality.

| Workload | Policy | Sat | Sym gen recall | Vector top-k recall | Vector top-k precision | Vector-only positives | Vector-only HN | Symbolic-only positives | Merged recall | Ranked recall | Visible precision | Visible HN |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| vector fixture cold | name-and-statement | no | 1/2 | 1/2 | 1/1271 | 1/2 | 1/1 | 1/2 | 2/2 | 1/2 | 1/1 | 0/1 |
| vector fixture warm | name-and-statement | no | 1/2 | 1/2 | 1/1271 | 1/2 | 1/1 | 1/2 | 2/2 | 1/2 | 1/1 | 0/1 |
| vector fixture | statement | no | 1/2 | 2/2 | 2/1270 | 1/2 | 1/1 | 0/2 | 2/2 | 1/2 | 1/1 | 0/1 |
| vector fixture | definition-aware | no | 1/2 | 1/2 | 1/1271 | 1/2 | 1/1 | 1/2 | 2/2 | 1/2 | 1/1 | 0/1 |
| default fixture-profile | name-and-statement | yes | 16/16 | 5/16 | 5/78 | 0/16 | 0/3 | 11/16 | 16/16 | 16/16 | 14/34 | 0/3 |
| hard-negatives fixture-profile | name-and-statement | yes | 1/1 | 1/1 | 1/78 | 0/1 | 0/5 | 0/1 | 1/1 | 1/1 | 1/34 | 0/5 |

Input-policy comparison is limited. `statement` improved vector top-k recall on the deterministic fixture, but no
completed real-model workload validates that improvement. `definition-aware` did not add body-summary content in this
fixture (`with_definition_body_summary = 0`), and no docstring-augmented run was meaningful because the fixture reports
`with_docstring = 0`.

Expanded cluster label truthfulness is preserved in the fixture artifact. The vector-only positive and symbolic-only
positive carry `expanded-positive` facts; the lexical hard negative carries `expanded-hard-negative` and typed
hard-negative facts rather than appearing unlabeled.

## Scorer Variants

The scorer variants are the blocking evidence for calibration. Vector candidate generation finds useful controlled
pairs, but vector-visible variants surface hard negatives.

| Workload | Variant | Candidate count | Visible groups | Ranked recall | Visible precision | Visible hard negatives |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| vector fixture cold | symbolic-only | 2 | 2/2 | 1/2 | 1/1 | 0/1 |
| vector fixture cold | vector-evidence-only | 1271 | 54/70 | 1/2 | 1/91 | 1/1 |
| vector fixture cold | symbolic-plus-vector | 1273 | 55/70 | 2/2 | 2/109 | 1/1 |
| vector fixture statement | vector-evidence-only | 1270 | recorded | 2/2 | 2/90 | 1/1 |
| vector fixture statement | symbolic-plus-vector | 1272 | recorded | 2/2 | 2/105 | 1/1 |
| default fixture-profile | symbolic-plus-vector | 363 | 33/40 | 16/16 | 15/91 | 2/3 |
| hard-negatives fixture-profile | symbolic-plus-vector | 363 | 33/40 | 1/1 | 1/91 | 1/5 |

This violates the allow-calibration rule: visible hard-negative leakage regresses on completed workloads. The regression
is not hidden by thresholds; it is recorded directly in the hidden variant artifacts.

## Runtime, RSS, Cache, and Reuse

| Workload | Corpus status | Cold build | Warm open/query | Vector query | Total vector validation | Peak RSS | Text-vector cache | Vector corpus |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| vector fixture cold | built | 25 ms | 0 ms | 360 ms | 330 ms | 84672512 B | 5535 B | 18367 B |
| vector fixture warm | reused | 0 ms | 430 ms | 418 ms | 328 ms | 79790080 B | 5535 B | 18367 B |
| default fixture-profile | built | 20 ms | 0 ms | 86 ms | 915 ms | 85524480 B | 1519 B | 6332 B |
| hard-negatives fixture-profile | built | 21 ms | 0 ms | 88 ms | 1536 ms | 85983232 B | 1519 B | 6330 B |
| manual-internal bounded | skipped before vector work | 0 ms | 0 ms | 0 ms | 51392 ms | 6689406976 B | 0 B | 0 B |

The warm vector fixture reused the corpus (`corpus_status = reused`, build 0 ms). Normalized quality artifacts for cold
and warm fixture runs were byte-identical after removing timings and memory counters, so metrics and pair ordering are
reproducible under the documented tie rules.

The manual-internal run hit `vector-validation-budget-exceeded:max-declarations:15654>32` after 51392 ms and peak RSS
6689406976 B. This is a useful partial status artifact, not quality evidence.

The manual-mathlib run exposed a remaining operational blocker: local prerequisites existed and progress was visible,
but pre-vector declaration enumeration over mathlib was still running at about 10% after more than a minute, before the
vector bounds could stop the run. It was interrupted and copied only as blocker evidence. This means the 35X bounds are
not sufficient for mathlib-scale validation until they apply before full mathlib declaration enumeration or the manual
workflow gains an earlier corpus cap.

## Boundary and Leak Evidence

Artifact leak checks over `target/search-quality/semantic-validation-decision/*-vector-search.json` found no raw source
snippets, absolute private paths, model input prefixes, backend names, storage vocabulary, SQLite/posting vocabulary,
worker rows, retrieval keys, tokenizer terms, or nearest-neighbor implementation terms. The checks intentionally ignore
operator stderr logs because progress output necessarily includes local workspace paths.

Boundary verification remains the responsibility of `cargo test -p lean-dup-cli --test boundaries`: embedding runtime
dependencies stay inside `lean-dup-embedding`, vector database dependencies stay inside `lean-dup-vector-index`, search
uses crate-root APIs for hidden vector experiments, and report exposes stable artifact facts only.

## Decision

Semantic vector search remains hidden and off-default. Prompt 36 must ignore vector facts.

The allow-calibration criteria were not met:

- Non-saturated command-level fixture evidence exists and shows vector-only recall gain, but it uses the deterministic
  fixture profile rather than a real local embedding model.
- Visible hard-negative leakage regresses in completed vector scorer variants: `1/1` on the non-saturated vector fixture
  for `vector-evidence-only` and `symbolic-plus-vector`, `2/3` on saturated default for `symbolic-plus-vector`, and
  `1/5` on saturated hard-negatives for `symbolic-plus-vector`.
- Default and hard-negative command-level suites are still saturated (`top_k = 32`, eligible corpus size `6`) and cannot
  support retrieval-quality claims.
- Warm-cache reproducibility passed on the non-saturated fixture, but that is insufficient while visible hard-negative
  leakage regresses.
- Manual/scale workloads did not complete as quality evidence. Manual-internal produced a budget-exceeded partial
  artifact; manual-mathlib still has a pre-vector enumeration bound gap.
- Artifacts are privacy-safe and internally usable, but they record unacceptable scorer behavior rather than an
  allow-calibration result.

Do not remove the experiment yet. The repaired command-level fixture proves useful validation machinery: top-k is
non-saturated, vector-only and symbolic-only denominators are explicit, expanded labels are truthful, skipped/cache-only
behavior is deterministic, and warm corpus reuse works. The next useful work is to prevent vector scorer variants from
making broad vector top-k membership visible without enough precision evidence, and to move mathlib bounds before the
full declaration-enumeration cost.

## Red Flag Review

- *Shallow module:* the decision uses raw denominators, scorer variants, cost, reuse, and leak evidence rather than
  command success.
- *Pass-through wrapper:* this document interprets stable artifacts and rejects backend success as a quality decision.
- *Temporal decomposition:* build, query, ranking, visibility, and validation cost are separate facts, not sequential
  excuses for promotion.
- *Information leakage:* artifacts passed leak checks; private model, backend, cache, worker, path, and source-text
  details are not decision facts.
- *Special-general mixture:* search owns semantic/vector search facts, embedding owns runtime/profile details,
  vector-index owns persistence, eval owns artifacts and the go/no-go decision.
- *Conjoined methods:* eval does not reconstruct candidate generation or rerank from raw vector scores.
- *Hard-to-describe public API:* the decision surface is small: policy ids, raw denominators, saturation, scorer
  variants, runtime/RSS/cache cost, reproducibility, leak status, and a single Prompt 36 decision.
- *Implementation details contaminating interface comments:* this prompt adds a validation document only; no public API
  comments were changed.
