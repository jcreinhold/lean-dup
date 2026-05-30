# Semantic Probe Planning

Date: 2026-05-21

This document records the Prompt 71 probe-planning repair. Semantic probes still run after candidate generation, but
planning now consumes candidate-source facts and records stable status denominators by source and match class before
final ranking consumes proof evidence.

## Design Note

Candidate-source facts own stable source ids, source families, declaration-pair ids, and leak-safe feature-family
labels. Semantic verification owns probe obligation planning, quotas, cache lookup, worker execution, and fallback
status. Ranking owns visibility decisions from stable evidence facts. Eval owns denominators. Report owns projection.

The smallest public interface is a bounded probe summary: planned, cached, worker-attempted, verified, rejected,
unavailable, skipped-by-policy, skipped-by-budget, and timeout counts by source id and match class. Callers do not see
Lean worker rows, proof obligations, raw expressions, retrieval keys, scorer internals, cache layout, private paths, or
vector facts.

The preserved user-facing capability is the conservative audit queue with semantic probes enabled by default.
Rejected/unavailable probes remain blockers for default actionability unless another proof-grade or calibrated symbolic
basis justifies visibility. The Python-era behavior discarded is spending a fixed probe budget after cheap ranking with
little evidence about which source or match class consumed it.

## Design It Twice

Three designs were considered.

1. Keep probes after cheap ranking and accept symbolic-recall misses. Rejected: this preserves the current blind spot,
   where the budget can be spent without source-level explanation.
2. Probe every generated candidate until the budget runs out. Rejected: Prompt 67 showed 13,581 shaped KanProofs
   candidates, so this would turn probing into broad proof search and make ordinary audits brittle.
3. Plan probes from candidate-source facts with bounded quotas by source, match class, declaration kind, and label risk.
   Chosen: search hides proof-cost policy while eval and report consume stable evidence facts.

## Active Planning Policy

The policy id remains `semantic-probe-policy.v2`. The planner now records a `ProbePlanningFacts` row internally for each
considered pair. The stable public projection is:

- `status_by_source`: keyed by candidate-source id such as `symbolic-retrieval` or a future Lean semantic lane id;
- `status_by_match_class`: keyed by stable obligation or review class such as `exact-theorem`, `reducible-definition`,
  `local-duplicate`, or `unranked`;
- each entry records `planned`, `cached`, `worker`, `verified`, `rejected`, `unavailable`, `skipped_by_policy`,
  `skipped_by_budget`, and `timeout`.

The actionable policy keeps ordinary exact/permutation/replacement probes first. Reducible-definition probes are capped
to 30% of the configured budget, local-duplicate fallback probes to 10%, and broad hard-negative-risk diagnostics to
10%. Broad probe policy keeps the old uncapped behavior for diagnostic runs.

## Prompt 67 Baseline

Prompt 67 recorded this KanProofs private-audit baseline:

| Fact | Count |
| --- | ---: |
| shaped candidates considered for probes | 13,581 |
| planned probes | 500 |
| skipped by policy | 6,217 |
| skipped by budget or per-declaration cap | 6,864 |
| exact-theorem planned / verified / rejected | 30 / 23 / 7 |
| reducible-definition planned / verified / rejected / unavailable | 470 / 14 / 287 / 169 |

The baseline shows that the bounded budget was dominated by low-yield reducible-definition probes. The repaired planner
does not claim a new KanProofs release result until the full validation matrix reruns, but it records enough
source/match-class facts to prove whether future budgets are spent on exact theorem evidence, reducible-definition
evidence, semantic-lane candidates, or low-yield diagnostic classes.

## Focused Evidence

`crates/search/src/semantic_verification.rs` now has focused planning tests that prove:

- skipped unranked pairs are counted as `skipped_by_policy` under `status_by_match_class["unranked"]`;
- per-declaration budget loss is counted under the exact match class that lost the pair;
- reducible-definition obligations are capped independently from exact theorem obligations;
- cached, verified, rejected, unavailable, and timeout statuses are attributed to stable source and match-class keys.

The cap fixture plans 3 exact theorem probes and 3 reducible-definition probes from a budget of 10, then records 7
reducible-definition candidates as skipped by budget. This is deliberately not release evidence; it proves the
denominator path before rerunning KanProofs.

## Command Evidence

Tiny fixture command:

```sh
env LEAN_DUP_CACHE_DIR=target/probe-planning/cache \
  cargo run -p lean-dup-cli -- audit --workspace tests/fixtures/tiny --module Tiny --format json \
  > target/probe-planning/tiny.json
```

Result: `status = ok`.

| Probe fact | Count |
| --- | ---: |
| planned pairs | 18 |
| cached hits | 18 |
| worker pairs | 0 |
| verified | 17 |
| rejected | 1 |
| unavailable | 0 |
| skipped by policy | 22 unranked |

KanProofs private audit command:

```sh
env LEAN_DUP_CACHE_DIR=target/probe-planning/kanproofs-cache /usr/bin/time -l \
  cargo run -p lean-dup-cli -- audit --workspace /Users/jcreinhold/Code/kan-proofs --module KanProofs \
    --private --format json \
  > target/probe-planning/kanproofs-private.json \
  2> target/probe-planning/kanproofs-private.stderr
```

Result: `status = ok`; probes were enabled. The run reused 180 cached worker results from the first Prompt 71 KanProofs
run and completed in 29.56 seconds with maximum RSS 6,112,149,504 bytes.

| Match class | Planned | Cached | Worker | Verified | Rejected | Unavailable | Skipped by policy | Skipped by budget |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| exact-theorem | 30 | 30 | 0 | 23 | 7 | 0 | 0 | 0 |
| reducible-definition | 150 | 150 | 0 | 5 | 92 | 53 | 0 | 6,037 |
| local-duplicate | 50 | 0 | 0 | 0 | 0 | 50 | 0 | 1,097 |
| unranked | 0 | 0 | 0 | 0 | 0 | 0 | 6,217 | 0 |

Source status shows 230 planned facts from `lean-semantic.statement-meaning.v1`, with 5,824 skipped by budget, and 1,816
policy skips plus 1,310 budget skips from `symbolic-retrieval`. This verifies that the report now distinguishes source
spending and match-class spending; it does not by itself close release quality or memory gates.

## Report Surface

Audit JSON now includes probe status maps under `semantic_verification.status_by_source` and
`semantic_verification.status_by_match_class`. These are queue-cost facts, not worker traces. They avoid raw obligation
terms, worker rows, cache keys, source snippets, private paths, and backend vocabulary.

Report text remains compact. Operators who need detailed probe-cost accounting can use JSON, while ordinary audit text
continues to show the summary line and visible queue.

## Red Flag Review

- Shallow module: the planner now owns quotas and source/match status accounting, not just pass-through counters.
- Pass-through wrapper: report receives stable status maps, not worker rows or raw proof obligations.
- Temporal decomposition: the split follows ownership: candidate-source facts, probe planning, worker execution,
  ranking, eval denominators, report projection.
- Information leakage: raw expressions, proof scripts, worker rows, retrieval keys, source snippets, private paths,
  cache layout, backend vocabulary, and vector facts stay out of artifacts.
- Special-general mixture: the policy is concrete for current symbolic sources and leaves semantic-lane source ids as
  data rather than as new public APIs.
- Conjoined methods: candidate generation, probing, scoring, and reporting remain separately owned.
- Hard-to-describe public API: a probe status map is one stable count table keyed by source id or match class.
- Implementation-detail comments: public comments describe stable status facts, not worker transport or generated Lean
  terms.
