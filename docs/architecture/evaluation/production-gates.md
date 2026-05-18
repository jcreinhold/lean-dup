# Production Gate Evaluation

This document records the evaluation boundary added by prompt 21 and updated through prompt 27. The goal is not to make
retrieval look good; it is to make audit quality measurable before release work continues. For the current end-to-end
pipeline, see
[../06-end-to-end-architecture.md](/Users/jcreinhold/Code/lean-dup/docs/architecture/06-end-to-end-architecture.md).
For the Prompt 29 typed adjudication schema, see
[search-labels.md](/Users/jcreinhold/Code/lean-dup/docs/architecture/evaluation/search-labels.md).
For Prompt 30 stage-level quality denominators, see
[search-stage-metrics.md](/Users/jcreinhold/Code/lean-dup/docs/architecture/evaluation/search-stage-metrics.md).

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

**Chosen: general scorer plus production-gate suite orchestration.** The scorer only knows unordered pairs, ranks,
shown membership, and raw denominators. Suite definitions own corpus loading, fixture/KanProofs execution, manual skip
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

Command completion is not a production-quality pass. `status = ok` means the suite ran and its current command-level
gate logic did not abort. Release readiness still depends on the raw denominators satisfying `G1 regression_quality`
and `G2 precision_control`.

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

Prompt 27 refreshed the eval artifacts:

- `target/eval/prompt27-default.json`
- `target/eval/prompt27-hard-negatives.json`
- `target/eval/prompt27-production-gate.json`

The old KanProofs/mathlib eval timeout is fixed: the aggregate now completes on this machine. The current quality
results are still not production-ready.

| Suite | Status | Recall@10 | Hard-negative hits | Shown precision | Candidates | Total time |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| `default` | `ok` | `14/14` | `0/4` | `13/34` | `299` | `6351 ms` |
| `hard-negatives` | `ok` | `1/1` | `0/5` | `1/34` | `299` | `423 ms` |
| `kanproofs-internal` | `ok` | `0/6` | `0/3` | `0/7507` | `466710` | `57197 ms` |
| `kanproofs-mathlib` | `ok` | `0/11` | `3/4` | `0/13593` | `482205` | `723796 ms` |
| `production-gate` | `ok` | `15/32` | `3/16` | `14/21168` | `949513` | `787767 ms` |

Interpretation:

- Fast fixture gates pass and remain useful CI-style checks.
- The aggregate command now completes, which was necessary for Python deprecation and future validation work.
- `G1 regression_quality` remains open because KanProofs internal and KanProofs/mathlib recall are poor.
- `G2 precision_control` remains open because the KanProofs/mathlib suite shows `3/4` hard-negative leakage.
- The aggregate `status = ok` must not be used as a release approval until the raw denominators pass.

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
