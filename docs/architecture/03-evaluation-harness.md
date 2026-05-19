# Evaluation Harness

The harness scores observed candidate pairs against gold labels and emits raw counts. It is how a retrieval,
ranking, or probe change is shown to be an improvement rather than a vibe shift.

For the production-gate suite, see [evaluation/production-gates.md](evaluation/production-gates.md).
For the full pipeline, see [06-end-to-end-architecture.md](06-end-to-end-architecture.md).

## Metrics

All percentage-like metrics are raw counts (`5/7`, not `71%`) so the denominator stays visible.

| Metric | What it counts |
| --- | --- |
| `recall@k` | gold positives the candidate set surfaces at each `k` |
| `shown_queue_precision` | gold positives among shown pairs / all shown pairs |
| `hard_negative_leakage` | hard negatives that reach the shown queue / all hard negatives |
| `candidate_count` | observed candidate pair count |
| `timings` | index load, retrieval, probe, total (ms) |
| `peak_memory_bytes` | peak RSS when the platform exposes it |

## Suites

| Suite | Speed | Default CI? | What it covers |
| --- | --- | --- | --- |
| `default` | fast | yes | small fixture suite; all gold positives within recall@10, zero hard-negative leakage required |
| `hard-negatives` | fast | yes | fixture precision gate (same-conclusion, broad-key, known static-fingerprint collisions) |
| `kanproofs-internal` | slow | no | KanProofs internal duplicate labels; requires compiled artifacts |
| `kanproofs-mathlib` | slow | no | KanProofs/mathlib labels, including known bogus mathlib collisions |
| `production-gate` | slow | no | aggregates the four above; `status = incomplete` if a manual suite is unavailable |

## Public interface

- `lean-dup eval --suite <name> --format table|json`
- `score_run(labels, observed, k_values) -> EvaluationMetrics`

Suites load corpus labels, run the index and retrieval through the normal cache layer, record timings and memory, and
emit metrics. The scorer itself knows nothing about fixture paths, KanProofs paths, label-file layout, retrieval
weights, probe policy, queue thresholds, or report formatting; only normalized labels and observed pairs.

A corpus-specific scorer was rejected: it would have mixed corpus knowledge with metric
definitions, so adding a new corpus would mean editing scoring. A general scorer plus suite
definitions keeps the metric interface small and reusable.

## Reading the numbers

Command-level success means the suite ran. Release readiness depends on the raw denominators
satisfying `G1 regression_quality` and `G2 precision_control` in
[04-production-readiness.md](04-production-readiness.md). The `production-gate` aggregate may
report `status = incomplete` on machines without the KanProofs workspace; that is a recorded
fact, not a pass.
