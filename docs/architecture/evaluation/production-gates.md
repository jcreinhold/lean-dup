# Production Gate Evaluation

This document records the evaluation boundary added by prompt 21. The goal is not to make retrieval look good; it is
to make audit quality measurable before more feature work.

## Design Note

The production-gate evaluation layer owns suite taxonomy, label provenance, manual/private-path policy, audit
observation policy, gate enforcement, runtime counters, memory counters, visible-queue denominators, and probe
availability denominators.

Its smallest public interface is:

- `lean-dup-rs eval --suite default|hard-negatives|kanproofs-internal|kanproofs-mathlib|production-gate`;
- JSON/table metrics with raw `found/total` counts.

These decisions must not leak upward or sideways:

- label file layout and fixture naming conventions;
- the private KanProofs path policy;
- cache and SQLite layout;
- retrieval weights, ranking thresholds, and profile filters;
- semantic-probe chunking, worker transport, JSONL framing, or Lean traversal details.

The preserved user-facing capability is read-only duplicate-audit quality measurement. Users can run a named suite and
see recall, shown-queue precision, hard-negative leakage, visible-group counts, probe availability, runtime, and peak
memory without learning how indexes, retrieval, ranking, or probes are implemented.

Python-era behavior intentionally discarded:

- anecdotal manual inspection as the regression signal;
- Python cache layout or path shape as a production contract;
- tuning only against positives without hard negatives;
- treating private KanProofs runs as default CI.

## Design It Twice

**Rejected: add KanProofs checks directly to the scorer.** That would mix a special corpus with general metric policy.
The scorer would learn private paths, slow-suite rules, and audit execution details, making every future corpus a scorer
change.

**Chosen: general scorer plus production-gate suite orchestration.** The scorer only knows unordered pairs, ranks, shown
membership, and raw denominators. Suite definitions own corpus loading, fixture/KanProofs execution, manual skip
policy, and gate enforcement. This is deeper because callers ask for a suite result rather than coordinating label
files, cache roots, retrieval output, audit output, and skip rules themselves.

## Suites

`default` is the fast fixture gate. It remains suitable for tests and default CI. It requires all positives at
recall@10 and zero hard-negative leakage.

`hard-negatives` is a fast fixture-focused precision gate. It carries at least one true positive and hard negatives for
same-conclusion, broad-key, known static-fingerprint, and obvious non-duplicate cases.

`kanproofs-internal` is manual and slow. It records targeted KanProofs internal labels and is not default CI.

`kanproofs-mathlib` is manual and slow. It records targeted KanProofs/mathlib labels, including known bogus mathlib
collisions such as `Height.WeilHeight` versus unrelated mathlib structures.

`production-gate` aggregates the fast gates and manual KanProofs gates. If a manual workspace is unavailable, the
aggregate status is `incomplete`, not `ok`.

## Metrics

All percentage-like metrics are raw counts:

- `recall`: found positives at each requested `k` over total positives;
- `shown_queue_precision`: shown true positives over all shown pairs;
- `hard_negative_hits`: hard negatives that reached the shown queue over all hard negatives;
- `visible_groups`: groups visible under the suite observation policy over total observed groups;
- `probe_unavailable`: unavailable semantic probes over planned probes;
- `candidate_count`: observed candidate pair count;
- `timings`: index load, retrieval, probe, and total milliseconds;
- `peak_memory_bytes`: peak RSS when the platform exposes it.

## Commands

Fast gate:

```sh
cargo run -p lean-dup-rs -- eval --suite default --format json \
  --output target/eval/default.json
```

Hard-negative gate:

```sh
cargo run -p lean-dup-rs -- eval --suite hard-negatives --format json \
  --output target/eval/hard-negatives.json
```

Production aggregate:

```sh
cargo run -p lean-dup-rs -- eval --suite production-gate --format json \
  --output target/eval/production-gate.json
```

The production aggregate may be `incomplete` on machines without `/Users/jcreinhold/Code/kan-proofs`. That status is a
recorded fact, not a pass.

## Current Evidence

The first runs after adding this boundary produced:

- `default`: recall@10 `14/14`, hard-negative hits `0/4`, candidate count `299`;
- `hard-negatives`: recall@10 `1/1`, hard-negative hits `0/5`, candidate count `299`;
- `production-gate` on this machine: status `failed`, aggregate recall@10 `15/21`, hard-negative hits `0/12`,
  candidate count `456839`, peak memory `4285382656` bytes.

The aggregate failed because `kanproofs-mathlib` hit `worker timed out after 60s`. `default`, `hard-negatives`, and
`kanproofs-internal` completed successfully. The raw artifact is `target/eval/production-gate.json`.

These are not final production results for `G1` or `G2`; they prove that the gate machinery can represent positives,
hard negatives, manual slow-suite failures, raw denominators, runtime, and memory.

## Red Flag Review

- **Shallow module:** avoided. Suite orchestration hides label loading, index setup, manual skip policy, and aggregate
  metrics behind one named command.
- **Pass-through wrapper:** avoided. `production-gate` adds aggregation, gate status, and manual-suite policy rather
  than forwarding to one child suite.
- **Temporal decomposition:** avoided. The boundary is organized around quality evidence, not the order in which
  retrieval, ranking, and probes run.
- **Information leakage:** avoided. Callers see named suites and metrics, not label layout, SQLite tables, cache paths,
  worker framing, or probe chunks.
- **Special-general mixture:** contained. KanProofs remains a named manual suite; the scorer and metric definitions are
  corpus-independent.
- **Conjoined methods:** no remaining red flag. Scoring accepts a complete observation and does not share retrieval or
  audit state with suite execution.
- **Hard-to-describe public API:** no remaining red flag. The public API is a named suite and raw counts.
- **Implementation details contaminating interface comments:** avoided. Interface comments describe labels,
  observations, and metrics, not storage layout or worker internals.
