# Semantic Probe Yield And Fallback Policy

Date: 2026-05-21

## Design Note

Search owns Lean probe planning, probe-status policy, cache use, and the fallback rule that rejected or unavailable
probes block default actionability. Worker owns subprocess transport, Lean command protocol, generated proof
obligations, heartbeat behavior, and raw probe rows. Eval owns denominators and release-gate interpretation. Report owns
bounded projection and user-facing explanations.

The smallest public interface is the stable probe summary: enabled, policy id, budget, planned pairs, worker-attempted
pairs, cached hits, verified results, rejected results, unavailable results, timeout/unavailable breakdowns, skipped
counts, and obligation-yield counters. Callers do not see worker transport, Lean obligations, retry mechanics, raw rows,
cache keys, or proof terms.

Preserved capability: ordinary audit can run with semantic probes enabled and can surface proof-grade findings when Lean
verifies them. Discarded behavior: probe failures are not silently downgraded into ordinary visible findings, and
`--no-semantic-probes` is not treated as the normal release path.

## Design It Twice

Three probe boundaries were considered.

1. **Require every planned probe to succeed.** Rejected. It would make large source-backed audits brittle and would turn
   one opaque/reducible-definition limitation into a whole-command failure.
2. **Let unavailable probes degrade silently into static visible findings.** Rejected. That hides exactly the
   distinction operators need: source-backed proof-grade evidence was attempted but did not produce evidence.
3. **Make search own probe status policy and expose measured facts.** Chosen. Search hides Lean execution details while
   report/eval expose yield, blockers, and fallback outcomes.

The chosen boundary is deeper because ranking and visibility know only stable semantic evidence states, report projects
those states, and worker mechanics remain below the search facade.

## Public Probe Facts

Audit JSON now records `rejected_results` beside existing probe counters. The production probe surface is:

- `planned_pairs`: probe obligations selected by search;
- `worker_pairs`: uncached pairs attempted through the Lean worker;
- `cached_hits`: planned pairs answered by the probe cache;
- `verified_results`: proof-grade semantic evidence;
- `rejected_results`: Lean ran the planned obligation and did not verify it;
- `unavailable_results`: probe could not produce an answer;
- `unavailable_timeout`: unavailable probes classified as timeout;
- `skipped_by_policy`, `skipped_by_budget`, `cheap_summary_rejects`: pairs not probed by search policy;
- `obligation_yield`: the same counters by obligation kind.

`worker_pairs` is the attempted-probe denominator. Timeout distribution is the `unavailable_timeout` count plus
`unavailable_by_reason["timeout"]` when present.

## Command Evidence

Commands were run with semantic probes enabled unless the command name says otherwise.

### Tiny Cold Fixture

Command:

```sh
rm -rf target/probe-cache/tiny
env LEAN_DUP_CACHE_DIR=target/probe-cache/tiny /usr/bin/time -l \
  cargo run -p lean-dup-cli -- audit --workspace tests/fixtures/tiny --module Tiny --format json \
  > target/audit-runs/probe-tiny-cold.json \
  2> target/audit-runs/probe-tiny-cold.stderr
```

Result: `status = ok`.

Raw probe facts:

| Fact | Count |
| --- | ---: |
| planned | 18 |
| attempted / worker | 18 |
| cached | 0 |
| verified | 17 |
| rejected | 1 |
| unavailable | 0 |
| timed out | 0 |
| skipped by policy | 22 |
| skipped by budget | 4 |
| visible groups | 5 |
| groups hidden by unavailable probes | 0 |

Cost: 3.41 seconds wall time; maximum RSS 655,769,600 bytes.

### Source-Backed External Fixture

Commands:

```sh
(cd tests/fixtures/source-backed && lake build)
env LEAN_DUP_CACHE_DIR=target/probe-cache/source-backed \
  cargo run -p lean-dup-cli -- index --workspace tests/fixtures/source-backed --module External --label linked
env LEAN_DUP_CACHE_DIR=target/probe-cache/source-backed /usr/bin/time -l \
  cargo run -p lean-dup-cli -- audit --workspace tests/fixtures/source-backed --module Tiny \
    --compare-index linked --format json --review-profile api-design \
  > target/audit-runs/probe-source-backed.json \
  2> target/audit-runs/probe-source-backed.stderr
```

Result: `status = ok`.

Raw probe facts:

| Fact | Count |
| --- | ---: |
| planned | 1 |
| attempted / worker | 1 |
| cached | 0 |
| verified | 1 |
| rejected | 0 |
| unavailable | 0 |
| timed out | 0 |
| skipped by policy | 0 |
| skipped by budget | 0 |
| visible groups | 1 |
| groups hidden by unavailable probes | 0 |

The visible group has `evidence_mode = proof-grade`, signal `probe:verified:exact-theorem`, and no probe blockers.

Cost: 1.52 seconds wall time; maximum RSS 656,211,968 bytes.

### KanProofs Full Audit

Command:

```sh
env LEAN_DUP_CACHE_DIR=target/probe-cache/kanproofs /usr/bin/time -l \
  cargo run -p lean-dup-cli -- audit --workspace ~/Code/kan-proofs --module KanProofs --format json \
  > target/audit-runs/probe-kanproofs.json \
  2> target/audit-runs/probe-kanproofs.stderr
```

Result: `status = ok`. The command completed with probes enabled; ordinary use does not currently require
`--no-semantic-probes`.

Raw probe facts:

| Fact | Count |
| --- | ---: |
| considered | 13,581 |
| planned | 500 |
| attempted / worker | 500 |
| cached | 0 |
| verified | 37 |
| rejected | 293 |
| unavailable | 170 |
| timed out | 0 |
| skipped by policy | 6,217 |
| skipped by budget | 6,864 |
| visible groups | 4 |
| groups hidden by unavailable probes | 170 |
| output size | 38,851 bytes |

Unavailable distribution:

| Reason | Count |
| --- | ---: |
| opaque-or-unreducible | 170 |

Obligation yield:

| Obligation | Planned | Verified | Rejected | Unavailable | Worker |
| --- | ---: | ---: | ---: | ---: | ---: |
| exact-theorem | 30 | 23 | 7 | 0 | 30 |
| reducible-definition | 470 | 14 | 286 | 170 | 470 |

Cost: 122.75 seconds wall time; maximum RSS 5,628,461,056 bytes.

The four visible groups all had `probe:verified:exact-theorem` and no `lean-probe-rejected` or
`lean-probe-unavailable` blocker. A JSON check found zero visible groups carrying either blocker.

## Fallback Policy

Default actionable output blocks groups with:

- `lean-probe-rejected`;
- `lean-probe-unavailable`;
- `unverified-proof-grade-evidence`.

Those groups remain counted in hidden diagnostics and may be inspected through diagnostic/noise workflows. They do not
become default cleanup findings unless a separate proof-grade or calibrated symbolic basis produces a visible group
without those blockers.

## Release Assessment

`G3 semantic_probe_yield` is improved but not fully closed.

Closed:

- probe-enabled audits complete on the controlled fixture, source-backed fixture, and KanProofs workload;
- JSON and text report stable planned, attempted, verified, rejected, unavailable, timeout, skipped, and cache counters;
- rejected and unavailable groups do not enter the default visible queue;
- source-backed fixture proves one proof-grade exact-theorem finding.

Remaining blockers for Prompt 56 and later release prompts:

- KanProofs probe-enabled full audit cost is still high: 122.75 seconds and about 5.63 GB RSS in a dev build;
- reducible-definition probe yield is low on KanProofs: 14 verified, 286 rejected, 170 unavailable;
- this artifact does not validate mathlib-scale probe yield or warm-cache probe reuse.

## Red Flag Review

- Shallow module: avoided. Search exposes stable probe facts, not worker mechanics.
- Pass-through wrapper: avoided. Report summarizes probe facts instead of forwarding worker rows.
- Temporal decomposition: avoided. The boundary is organized around probe status and fallback policy, not command steps.
- Information leakage: avoided for the new surface. Worker rows, proof obligations, subprocess details, cache keys, and
  raw probe transport stay private.
- Special-general mixture: avoided. Fixture and KanProofs evidence use the same probe/status policy.
- Conjoined methods: avoided. Ranking visibility can be understood from stable semantic evidence and blockers.
- Hard-to-describe public API: avoided. Planned, attempted, verified, rejected, unavailable, skipped, and timeout counts
  are the public facts.
- Implementation details contaminating interface comments: avoided. Public comments describe stable status facts.
