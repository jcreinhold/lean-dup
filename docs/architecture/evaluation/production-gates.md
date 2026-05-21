# Production Gate Evaluation

Quality is measured before release work continues. This boundary defines the suites, their gate
status, and the raw counts they emit.

For the pipeline that produces the observations, see [../end-to-end-architecture.md](../end-to-end-architecture.md).
For the typed adjudication schema, see [search-labels.md](search-labels.md). For stage-level
denominators, see [search-stage-metrics.md](search-stage-metrics.md).

## Suites

| Suite                | Speed | Default CI? | Purpose                                                                                |
| -------------------- | ----- | ----------- | -------------------------------------------------------------------------------------- |
| `default`            | fast  | yes         | small fixture quality gate; all positives at recall@10; zero hard-negative leakage     |
| `hard-negatives`     | fast  | yes         | fixture precision gate (same-conclusion, broad-key, known static-fingerprint collisions) |
| `manual-internal`    | slow  | no          | targeted manual-corpus internal labels                                                 |
| `manual-mathlib`     | slow  | no          | targeted manual-corpus/mathlib labels including known bogus collisions                 |
| `production-gate`    | slow  | no          | aggregates the four above; `status = incomplete` when a manual suite is unavailable    |

The `production-gate` suite may be `incomplete` on machines without a manual-corpus workspace
(`--workspace <path> --manual-module <Root>`). That status is a recorded fact, not a pass.

## Manual Suite Boundary

Design note:

- Hidden knowledge: eval owns manual suite resolution, label parsing, prerequisite checks, raw denominators, and the
  decision to report skipped manual suites as incomplete evidence. Project owns Lake workspace and mathlib source
  discovery; index owns compiled-olean and cache mechanics; search owns observations and review policy.
- Smallest public interface: CLI/eval callers provide a suite id plus optional `--workspace`, `--manual-module`, and
  `--mathlib-workspace`; the JSON report returns either metrics or a structured `manual_prerequisites` blocker report.
- Non-leaking decisions: private corpus paths are operator-supplied only; worker rows, cache directories, retrieval
  keys, and index internals do not become label or scorer inputs.
- Preserved capability: default and hard-negative suites remain fast, checked-in, and free of private paths.
- Discarded behavior: a skipped manual suite is no longer a vague note such as "workspace unavailable" and is never
  counted as a release-quality pass.

Design it twice:

- *Documentation-only manual suites.* Rejected because release evidence would still depend on prose instructions rather
  than command output.
- *Baked-in private paths.* Rejected because one machine's KanProofs/mathlib layout would leak into the eval contract.
- *Operator-supplied workloads with prerequisite reports.* Chosen because private environment knowledge stays outside
  search while skipped and completed manual runs both produce stable, actionable facts.

Manual child runs emit `manual_prerequisites` when they are skipped or completed. The object records the required
workspace argument, selected module root, typed-label parse status, compiled-olean status, mathlib source/olean status
for `manual-mathlib`, blockers, and the next command to run. A completed run additionally emits the ordinary raw
metrics: recall denominators, shown queue precision, hard-negative hits, visible groups, probe unavailable counts,
stage metrics, timings, peak RSS status, and the eval report schema/path when the CLI wrote an output artifact.

## Metrics

All percentage-like metrics are raw counts so the denominator stays visible.

| Metric                  | What it counts                                                          |
| ----------------------- | ----------------------------------------------------------------------- |
| `recall`                | found positives at each requested `k` / total positives                 |
| `shown_queue_precision` | shown true positives / all shown pairs                                  |
| `hard_negative_hits`    | hard negatives reaching the shown queue / all hard negatives            |
| `visible_groups`        | groups visible under the suite observation policy / total observed groups |
| `probe_unavailable`     | unavailable semantic probes / planned probes                            |
| `candidate_count`       | observed candidate pair count                                           |
| `timings`               | index load, retrieval, probe, total (ms)                                |
| `peak_memory_bytes`     | peak RSS when the platform exposes it                                   |

`status = ok` means the suite ran and its command-level gate logic did not abort. It is not a
release-quality pass; release readiness depends on the raw denominators satisfying
`G1 regression_quality` and `G2 precision_control`.

## Commands

```sh
# Fast gates
cargo run -p lean-dup-cli -- eval --suite default --format json \
  --output target/eval/default.json
cargo run -p lean-dup-cli -- eval --suite hard-negatives --format json \
  --output target/eval/hard-negatives.json

# Aggregate
cargo run -p lean-dup-cli -- eval --suite production-gate --format json \
  --output target/eval/production-gate.json
```

## Why a general scorer plus suite orchestration

Adding manual-corpus checks directly to the scorer would mix a special corpus with general
metric policy. The scorer would learn private paths, slow-suite rules, and audit execution
details; every future corpus would become a scorer change.

The general scorer knows only unordered pairs, ranks, shown membership, and raw denominators.
Suite definitions own corpus loading, fixture and manual-suite execution, manual skip policy,
and gate enforcement. Callers ask for a suite result instead of coordinating label files,
cache roots, retrieval output, and skip rules themselves.
