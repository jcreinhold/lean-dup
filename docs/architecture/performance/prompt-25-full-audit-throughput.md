# Prompt 25 Full-Audit Throughput

For the current end-to-end architecture around audit throughput, see
[../06-end-to-end-architecture.md](/Users/jcreinhold/Code/lean-dup/docs/architecture/06-end-to-end-architecture.md).

## Current Status Note

This remains a historical performance report. Prompt 27 later moved large index-worker timeout policy into the
worker/index boundary, so the old KanProofs/mathlib eval timeout described below is superseded. The current Prompt 27
production-gate artifact completes, but it still exposes quality failures: aggregate recall@10 `15/32`, aggregate
hard-negative hits `3/16`, KanProofs/mathlib recall@10 `0/11`, and KanProofs/mathlib hard-negative hits `3/4`.

## Design Note

The internal audit-throughput profiling layer owns workload cache state, timing labels, memory snapshots,
retrieval/source/probe/render counters, and before/after artifact paths. Its smallest public interface is the existing
`--profile` output, the hidden `lean-dup perf` workloads, and this architecture report.

Audit, retrieval, ranking, semantic verification, indexing, and rendering callers must not learn SQLite layout,
retrieval key shape, source scanning policy, probe chunking, JSONL transport details, or cache internals. The preserved
capability is read-only local duplicate auditing with cached indexes, mathlib comparison, semantic evidence, JSON/text
reports, and production-gate evaluation. The Python-era behavior intentionally discarded is anecdotal timing and manual
inspection as performance evidence.

## Design It Twice

Rejected: ad hoc shell timing plus speculative fixes. That approach cannot reproduce results, makes workload setup part
of the operator's memory, and encourages tuning whatever is visible in the last terminal line.

Chosen: checked-in perf workloads plus private instrumentation. Future sessions ask for named workload results and
cost classes, not a sequence of cache cleanup, audit commands, `grep`, and timing notes. This is deeper because the
measurement boundary owns artifact naming and cost classification while the normal audit interface stays unchanged.

## Workloads And Artifacts

All commands used `target/release/lean-dup` against `/Users/jcreinhold/Code/kan-proofs`. The shared cache root was
the default `~/.cache/lean-dup`. Before artifacts are under `target/perf/prompt25/before/`; after artifacts are under
`target/perf/prompt25/after-final/`. The hidden perf cost-class artifacts are under
`target/perf/prompt25/after-final/perf/`.

```sh
target/release/lean-dup --progress --profile audit --workspace /Users/jcreinhold/Code/kan-proofs --module KanProofs --format json
target/release/lean-dup --progress --profile audit --workspace /Users/jcreinhold/Code/kan-proofs --module KanProofs --compare-mathlib --no-semantic-probes --format json
target/release/lean-dup --progress --profile audit --workspace /Users/jcreinhold/Code/kan-proofs --module KanProofs --compare-mathlib --format json
target/release/lean-dup --progress --profile audit --workspace /Users/jcreinhold/Code/kan-proofs --module KanProofs.Mathlib4Backports --compare-mathlib --format json
```

The first baseline attempt found that KanProofs needed a fresh build for the current local workspace; `lake build` in
`/Users/jcreinhold/Code/kan-proofs` completed before the recorded baselines. The before
`kanproofs-full-mathlib-no-probes` workload rebuilt a stale mathlib index, so its 526 s wall time is a cache-state
artifact, not an audit-only baseline. The proof-grade mathlib run immediately after it used the warmed index.

## Before And After

| Workload | Before wall | After wall | Before RSS | After RSS | Before JSON | After JSON | Candidates | Groups before | Groups after | Visible before | Visible after | Probe worker before | Probe worker after |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Full KanProofs, no mathlib | 503.52 s | 10.16 s | 13.06 GB | 4.57 GB | 2.6 GB | 60 MB | 462631 | 343367 | 6449 | 337553 | 6300 | 500 | 0 |
| Full KanProofs, mathlib, no probes | 526.21 s | 36.71 s | 5.83 GB | 4.44 GB | 41 MB | 41 MB | 477809 | 3406 | 3406 | 0 | 0 | 0 | 0 |
| Full KanProofs, mathlib, proof-grade probes | 53.51 s | 46.25 s | 4.44 GB | 4.37 GB | 41 MB | 41 MB | 477809 | 3406 | 3406 | 0 | 0 | 180 | 0 |
| Targeted `KanProofs.Mathlib4Backports` | 8.93 s | 11.96 s | 2.03 GB | 0.66 GB | 124 KB | 124 KB | 1360 | 0 | 0 | 0 | 0 | 0 | 0 |

The completed code win is the first row: default full-audit output no longer constructs and renders a giant hidden
feature-only queue, and caller-reference scans are now scoped to groups that can actually receive replacement hints.
The mathlib no-probes row is kept for artifact completeness but should not be read as a pure code speedup because the
before run refreshed a stale shared mathlib index. The targeted run became slower in wall time but used much less peak
RSS; no accepted optimization targeted that path.

## Cost Classification

Direct `--profile` timings from the after runs:

| Workload | Workspace | Index reuse/build | Retrieval | Worker version/build |
| --- | ---: | ---: | ---: | ---: |
| Full no mathlib | 19 ms | 1028 ms | 4593 ms | 1020 ms |
| Full mathlib, no probes | 8 ms | 973 ms | 33745 ms | 1592 ms |
| Full mathlib, proof-grade | 9 ms | 1029 ms | 41458 ms | 1663 ms |
| Targeted mathlib | 1 ms | 3129 ms | 6457 ms | 3777 ms |

Hidden perf after-run cost-class totals:

| Workload | Worker startup | Transport | SQLite/index | Retrieval/ranking | Reporting |
| --- | ---: | ---: | ---: | ---: | ---: |
| Full no mathlib | 8494 ms | 1 ms | 2073 ms | 15689 ms | 392 ms |
| Full mathlib, no probes | 54980 ms | 12 ms | 172497 ms | 178561 ms | 49 ms |
| Full mathlib, proof-grade | 4137 ms | 0 ms | 51707 ms | 57599 ms | 41 ms |
| Targeted mathlib | 1738 ms | 0 ms | 113 ms | 131 ms | 0 ms |

The perf wrapper records events from inside the audit process and is useful for class attribution, but the direct audit
artifacts are the wall-time evidence above. The no-probes perf wrapper run was noisier than the direct run; it is not
used as the before/after wall-time claim.

Important event-level after measurements:

- Full no-mathlib direct retrieval stayed broad at 462631 candidates, but default review shaping reduced report groups
  from 343367 to 6449.
- Intermediate instrumentation before the replacement-reference refinement measured
  `source_refs.collect.references = 56105 ms`. The final perf artifact reduced that to 3138 ms by scanning references
  only for visible groups that can receive replacement hints.
- Full no-mathlib report rendering fell with output size: 2.6 GB before to 60 MB after.
- Full mathlib proof-grade probes were all cache hits after the previous prompt's probe-cache work:
  180 planned, 180 cached, 0 worker pairs, 96 unavailable.

## Accepted Interventions

Default review shaping now applies the Mathlib review profile to both mathlib and non-mathlib audits unless
`--show-noise` is requested. This removes feature-only/noise groups from the default visible queue before source facts,
replacement hints, and JSON rendering are built. Broad/API-design review remains available through explicit profiles.

Source-reference collection is now scoped. `source_refs` owns the reference-scan scope, and `replacement_hints` owns the
decision about which visible groups need caller references. Audit asks for facts, ranks once without caller scans, then
recollects caller references only for declarations whose visible replacement hints can use them. Imports and source
fingerprints remain available for every hydrated declaration.

The hidden perf harness now includes `kanproofs-full-mathlib-no-probes` and extracts review-group counts, visible-group
counts, semantic planned/cached/worker/unavailable counts, and `profile.*` timings from audit output.

## Rejected Interventions

No parallelism was added. The measured no-mathlib bottleneck was avoidable work in hidden queue construction, source
reference scanning, and giant report rendering. Adding workers would have made that waste concurrent instead of
removing it.

No SQLite query tuning was accepted. The noisy perf wrapper showed SQLite/index time can matter for full mathlib
comparison, but the direct runs and event logs also show retrieval still dominates there. Prompt 26/production JSON
work should first decide which hidden details are contractually required before query-shaping hidden rows.

No Lean heartbeat or probe-worker tuning was accepted. In the after proof-grade run, all 180 planned probe pairs were
cache hits and no Lean probe worker pairs ran.

## POSD Mapping

Remove work: the default queue no longer materializes hundreds of thousands of feature-only groups that cannot produce
actionable default output. Caller-reference scans are not performed for groups that cannot receive replacement hints.

Pull complexity down: source scan policy moved into `source_refs`, and hint reference eligibility moved into
`replacement_hints`. Audit orchestrates the phases but does not know caller-search tokens, import parsing, or hint
display policy.

Preserve the abstraction boundary: no public CLI flags expose retrieval keys, SQLite tables, caller-scan policy, or JSON
rendering shortcuts. The visible controls remain review profile, noise visibility, semantic-probe controls, and output
format.

Optimize before micro-tuning: the accepted changes remove avoidable source/render work. Lower-level tuning of SQLite,
ranking allocation, or parallelism remains deferred until the production report contract says those hidden groups must
still be materialized.

## Residual Risks

The no-mathlib default output is intentionally less exhaustive: it now follows the same default actionability filter as
mathlib review unless `--show-noise` or a broader profile is requested. This is a production-readiness choice, not a
retrieval recall claim.

Full mathlib comparison still spends tens of seconds in retrieval/index reads even with warm caches. The next throughput
work should measure whether hidden/noise groups must be hydrated at all under the final JSON contract before changing
SQLite query shape.

The targeted mathlib workload regressed in wall time from 8.93 s to 11.96 s in this measurement set while peak RSS
dropped from 2.03 GB to 0.66 GB. No code path specific to targeted mathlib was tuned in this prompt.

The production-gate eval artifact was refreshed at `target/eval/production-gate.json`, but at the time of this report
the aggregate gate still failed:

```text
status = failed
default = ok
hard-negatives = ok
kanproofs-internal = ok
kanproofs-mathlib = failed because the eval worker used the old short indexing timeout
```

This was not a new failure introduced by the throughput changes. Prompt 27 fixed that timeout policy, but Prompt 29
must still validate the real mathlib quality gate before release because current recall and hard-negative denominators
remain unacceptable.

## Red Flag Review

- Shallow module: mitigated. New behavior sits behind source-reference scope, replacement-hint eligibility, and hidden
  perf metrics rather than command scripts.
- Pass-through wrapper: mitigated. The perf workload now extracts audit metrics and profile timings; it is not just a
  shell alias.
- Temporal decomposition: mitigated. The source scan is phased, but the policy is owned by the source/hint modules
  rather than exposed as "run this first, then that" to callers.
- Information leakage: mitigated. SQLite layout, retrieval keys, and caller-token matching stay private.
- Special-general mixture: residual risk. Default Mathlib-profile shaping now affects no-mathlib audits; this is
  intentional for default actionability, and Prompt 26 must make the report contract explicit.
- Conjoined methods: mitigated. Replacement-hint caller needs are separated from source scanning and audit orchestration.
- Hard-to-describe public API: mitigated. No new normal user-facing API was added.
- Implementation details contaminating interface comments: mitigated. New comments describe caller obligations and
  output facts, not table layouts or scanning internals.
