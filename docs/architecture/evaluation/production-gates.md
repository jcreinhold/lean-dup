# Production Gate Evaluation

Quality is measured before release work continues. This boundary defines the suites, their gate status, and the raw
counts they emit.

For the pipeline that produces the observations, see
[../06-end-to-end-architecture.md](/Users/jcreinhold/Code/lean-dup/docs/architecture/06-end-to-end-architecture.md).
For the typed adjudication schema, see [search-labels.md](search-labels.md). For stage-level denominators, see
[search-stage-metrics.md](search-stage-metrics.md).

## Suites

| Suite | Speed | Default CI? | Purpose |
| --- | --- | --- | --- |
| `default` | fast | yes | small fixture quality gate; all positives at recall@10; zero hard-negative leakage |
| `hard-negatives` | fast | yes | fixture precision gate (same-conclusion, broad-key, known static-fingerprint collisions) |
| `kanproofs-internal` | slow | no | targeted KanProofs internal labels |
| `kanproofs-mathlib` | slow | no | targeted KanProofs/mathlib labels including known bogus collisions like `Height.WeilHeight` vs unrelated mathlib structures |
| `production-gate` | slow | no | aggregates the four above; `status = incomplete` when a manual suite is unavailable |

The `production-gate` suite may be `incomplete` on machines without `/Users/jcreinhold/Code/kan-proofs`. That status
is a recorded fact, not a pass.

## Metrics

All percentage-like metrics are raw counts so the denominator stays visible.

| Metric | What it counts |
| --- | --- |
| `recall` | found positives at each requested `k` / total positives |
| `shown_queue_precision` | shown true positives / all shown pairs |
| `hard_negative_hits` | hard negatives reaching the shown queue / all hard negatives |
| `visible_groups` | groups visible under the suite observation policy / total observed groups |
| `probe_unavailable` | unavailable semantic probes / planned probes |
| `candidate_count` | observed candidate pair count |
| `timings` | index load, retrieval, probe, total (ms) |
| `peak_memory_bytes` | peak RSS when the platform exposes it |

`status = ok` means the suite ran and its command-level gate logic did not abort. It is not a release-quality pass.
Release readiness depends on the raw denominators satisfying `G1 regression_quality` and `G2 precision_control`.

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

## Current evidence

Recall and precision are raw counts; runtime is in ms. The aggregate command completes; the quality numbers do not.

| Suite | Status | Recall@10 | Hard-neg hits | Shown precision | Candidates | Time (ms) |
| --- | :---: | ---: | ---: | ---: | ---: | ---: |
| `default` | ok | 14/14 | 0/4 | 13/34 | 299 | 6 351 |
| `hard-negatives` | ok | 1/1 | 0/5 | 1/34 | 299 | 423 |
| `kanproofs-internal` | ok | 0/6 | 0/3 | 0/7 507 | 466 710 | 57 197 |
| `kanproofs-mathlib` | ok | 0/11 | 3/4 | 0/13 593 | 482 205 | 723 796 |
| `production-gate` | ok | 15/32 | 3/16 | 14/21 168 | 949 513 | 787 767 |

Reading the table:

- Fast fixture gates pass; they remain useful CI-style checks.
- The aggregate command completes; that was necessary for the Python deprecation and for future validation.
- `G1 regression_quality` remains open: KanProofs internal and KanProofs/mathlib recall are poor.
- `G2 precision_control` remains open: KanProofs/mathlib shows 3/4 hard-negative leakage.
- The aggregate `status = ok` is not a release approval. Use the raw denominators.

## Why a general scorer plus suite orchestration

Adding KanProofs checks directly to the scorer would mix a special corpus with general metric policy. The scorer would
learn private paths, slow-suite rules, and audit execution details: every future corpus would become a scorer change.

The general scorer knows only unordered pairs, ranks, shown membership, and raw denominators. Suite definitions own
corpus loading, fixture/KanProofs execution, manual skip policy, and gate enforcement. Callers ask for a suite result
instead of coordinating label files, cache roots, retrieval output, and skip rules themselves.

Artifacts:

- `target/eval/prompt27-default.json`
- `target/eval/prompt27-hard-negatives.json`
- `target/eval/prompt27-production-gate.json`
