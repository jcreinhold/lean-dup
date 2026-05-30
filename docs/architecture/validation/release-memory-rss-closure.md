# Release Memory And RSS Closure

Date: 2026-05-21

Measured revision: `36c3215`

## Design Note

Eval orchestration owns quality-suite execution, child-suite aggregation, label-resolution traces, and release
denominators. Full-audit workflow owns probe-enabled operator audits. Index owns cache reuse and declaration hydration.
Report owns bounded projection and parseable JSON. Diagnostics owns platform memory observation through stable RSS
facts.

The smallest public memory interface is a named workload plus stable facts: status, cache state, runtime, peak RSS or
RSS-unavailable status, output size, parse time, candidate count, visible groups, and quality denominators. Cache
layout, worker rows, index storage, retrieval keys, report assembly, and platform-specific `getrusage` mechanics do not
leak into the release gate.

The preserved user-facing capability is a symbolic read-only audit and eval path with bounded ordinary JSON. The
Python-era behavior intentionally discarded is treating missing RSS, large unbounded report payloads, or fixture-only
measurements as production memory evidence.

## Design It Twice

Three memory-closure designs were considered:

1. **Accept missing RSS as a local timing-wrapper limitation.** Rejected. Prompt 60 already showed that this leaves the
   gate unable to distinguish "not measured" from "safe."
2. **Lower workload scope until the target passes.** Rejected. A smaller suite would not prove KanProofs/mathlib release
   behavior.
3. **Use stable in-process RSS facts, measure named workloads, and only optimize a measured dominant source.** Chosen.
   This is deeper because release memory interpretation lives in this artifact while eval, search, index, worker, and
   report keep their hidden implementation details.

No code optimization was made in this session. The measurements identify an eval-specific memory blocker, but not a
small safe one-session fix. A follow-up prompt was added for focused eval materialization reduction.

## Prompt 66 Design Note

Eval stage metrics own label denominators, label-resolution traces, and suite aggregation. Search observation owns the
boundary between retrieval/ranking and eval. Report projection owns bounded JSON. Diagnostics owns stable RSS facts.

The smallest public eval-memory interface is unchanged: eval consumes stable pair identity, stage survival, feature
family labels, rank, visibility, retrieval counters, label traces, timings, and peak RSS. Search now exposes a compact
stage-observation path for ordinary eval; detailed feature vectors and scorer component maps remain available only for
explicit search-dataset and scorer-ablation artifacts. Retrieval keys, scorer weights, cache layout, worker rows, source
snippets, and platform RSS mechanics remain private to their owning layers.

The preserved user-facing capability is the same symbolic eval and audit output with the same raw denominators and
bounded report contract. The discarded Python-era behavior is retaining forensic per-pair detail in the ordinary eval
path when release metrics only need stable stage facts.

## Prompt 66 Design It Twice

Three eval-memory designs were considered:

1. **Split manual eval into smaller release workloads.** Rejected. It would lower scope and make the aggregate
   `production-gate` memory claim meaningless.
2. **Raise the RSS target because full audits already pass.** Rejected. Prompt 63 showed an eval-only data-shape
   problem, not an invalid full-audit target.
3. **Make ordinary eval consume compact search-stage facts and reserve detailed observations for explicit forensic
   artifacts.** Chosen. This is deeper because search hides detailed feature/scorer internals behind a smaller eval
   surface, while eval still owns truth denominators and report still owns projection.

## Observation Mechanism

Prompt 60's `/usr/bin/time -l` wrapper ran the full-audit commands but returned a nonzero status after command success
because this environment denied `sysctl kern.clockrate`. Peak RSS from that wrapper was therefore unavailable.

The CLI and eval code already expose a stable in-process RSS observation through `lean_dup_diagnostics::perf` using
`getrusage(RUSAGE_SELF)`. Prompt 63 used that mechanism for both eval artifacts and full-audit perf artifacts. On macOS
the returned `ru_maxrss` value is bytes.

The local operator-supplied KanProofs and mathlib paths are redacted in this document. The generated target artifacts
may contain the local command used to run the workload; they are measurement artifacts, not release-contract examples.

## Commands And Artifacts

Release binary:

```sh
cargo build --release -p lean-dup-cli
```

Eval artifacts:

- `target/release-memory/eval/default.json`
- `target/release-memory/eval/hard-negatives.json`
- `target/release-memory/eval/manual-internal.json`
- `target/release-memory/eval/manual-internal-after.json`
- `target/release-memory/eval/manual-mathlib.json`
- `target/release-memory/eval/manual-mathlib-after.json`
- `target/release-memory/eval/production-gate.json`
- `target/release-memory/eval/production-gate-after.json`

Full-audit perf artifacts:

- `target/release-memory/perf/manual-full-no-mathlib-cold.json`
- `target/release-memory/perf/manual-full-no-mathlib-warm.json`
- `target/release-memory/perf/manual-full-mathlib-cold.json`
- `target/release-memory/perf/manual-full-mathlib-warm.json`

Representative command shapes:

```sh
env LEAN_DUP_CACHE_DIR=target/release-memory/cache/eval \
  target/release/lean-dup eval --suite manual-internal \
  --workspace <kan-proofs> --manual-module KanProofs \
  --format json --output target/release-memory/eval/manual-internal.json

target/release/lean-dup perf --workload manual-full-mathlib \
  --manual-workspace <kan-proofs> --manual-module KanProofs \
  --mathlib-workspace <project-mathlib> \
  --cache-root target/release-memory/cache/perf-mathlib \
  --output target/release-memory/perf/manual-full-mathlib-warm.json
```

## Eval Memory Results

Release target: peak RSS <= 6.5 GiB for release-gate workloads.

| Suite | Status | Runtime | Peak RSS | Candidate count | Visible groups | Precision | Hard-negative hits | Output size | Parse time |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| default | ok | 3.54 s | 9,076,736 B | 299 | 7/39 | 8/8 | 0/3 | 4,381 B | 0.00 s |
| hard-negatives | ok | 1.38 s | 8,994,816 B | 299 | 7/39 | 1/8 | 0/5 | 4,378 B | 0.00 s |
| manual-internal | blocked | 43.36 s | 6,769,213,440 B | 558,109 | 8/7,840 | 0/4 | 0/3 | 18,773 B | 0.00 s |
| manual-mathlib | blocked | 549.06 s | 6,466,568,192 B | 576,325 | 8/7,850 | 0/4 | 0/4 | 28,511 B | 0.00 s |
| production-gate | blocked | 85.44 s | 7,093,010,432 B | 1,135,032 | 30/15,768 | 9/24 | 0/15 | 67,723 B | 0.00 s |

The fast fixture suites are safely below the target. `manual-mathlib` is just below the target on this run, but
`manual-internal` and aggregate `production-gate` exceed it. The aggregate command is a release blocker even though its
children may free memory between runs: peak RSS is the process high-water mark, and the release-gate process still
exceeds the documented bound.

Quality denominators were not weakened. Manual suites remain blocked because the label corpus still has unresolved or
invalid positives; memory closure does not count those suites as release passes.

## Eval Memory Reduction Results

Prompt 66 changed ordinary eval to use compact search-stage observations. Detailed per-pair feature vectors and scorer
component maps are still produced for `--write-search-dataset` and `--write-scorer-ablations`, but ordinary release eval
no longer retains that forensic detail while constructing metrics.

| Suite | Status | Runtime before | Runtime after | RSS before | RSS after | Candidate count | Visible groups | Precision | Hard-negative hits | Candidate-generation recall | Output after | Parse time |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| manual-internal | blocked | 43.36 s | 10.57 s | 6,769,213,440 B | 5,546,688,512 B | 558,109 | 8/7,840 | 0/4 | 0/3 | 1/6 | 18,777 B | 0.00 s |
| manual-mathlib | blocked | 549.06 s | 37.26 s | 6,466,568,192 B | 5,875,662,848 B | 576,325 | 8/7,850 | 0/4 | 0/4 | 0/11 | 28,514 B | 0.00 s |
| production-gate | blocked | 85.44 s | 86.57 s | 7,093,010,432 B | 6,548,488,192 B | 1,135,032 | 30/15,768 | 9/24 | 0/15 | 18/34 | 67,727 B | 0.00 s |

The aggregate `production-gate` high-water RSS is now below the 6.5 GiB release target when interpreted as GiB
(`6,979,321,856` bytes). The suite still reports `blocked`; the remaining release blockers are label/quality blockers,
not memory blockers.

Quality denominators are unchanged for the fields relevant to this prompt: candidate counts, visible groups, shown queue
precision, hard-negative visible hits, and candidate-generation recall match the before measurements. Runtime improved
for the two manual child suites because ordinary eval avoids building detailed feature/scorer artifacts. The aggregate
runtime is effectively unchanged because it still runs all child suites in one process and reuses warm caches.

## Full-Audit RSS Results

| Workload | Cache | Runtime | Peak RSS | JSON size | Candidates | Review groups | Visible groups | Probe cache/worker |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| KanProofs internal | cold | 102.10 s | 5,692,719,104 B | 39,163 B | 558,109 | 7,378 | 4 | 0 cached / 500 worker |
| KanProofs internal | warm | 8.56 s | 5,679,267,840 B | 39,162 B | 558,109 | 7,378 | 4 | 500 cached / 0 worker |
| KanProofs + mathlib | cold | 561.84 s | 5,497,683,968 B | 10,677 B | 576,325 | 3,750 | 0 | 0 cached / 212 worker |
| KanProofs + mathlib | warm | 30.49 s | 5,619,433,472 B | 10,676 B | 576,325 | 3,750 | 0 | 212 cached / 0 worker |

The full-audit RSS gap from Prompt 60 is closed: in-process RSS is available, and both internal and mathlib full audits
fit under the 6.5 GiB target. Warm-cache reuse is visible in probe worker counts dropping to zero and runtime dropping
from 102.10 s to 8.56 s for the internal audit and from 561.84 s to 30.49 s for mathlib comparison.

## Dominant Memory Source

The dominant Prompt 63 memory source was eval materialization, not report projection or ordinary full-audit output.

Evidence:

- full audits and manual evals have similar candidate counts, but full audits stay below 5.70 GiB while
  `manual-internal` eval reaches 6.77 GiB;
- report JSON sizes are tiny: the largest measured eval artifact is 67,723 bytes, and all `jq` status parses complete in
  0.00 s;
- ordinary audit JSON remains bounded and parseable;
- aggregate `production-gate` combines the two manual eval paths in one process and reaches 7.09 GiB high-water RSS.

The measured allocation shape was the ordinary eval path retaining detailed search observations: per-pair feature
vectors, module facts, role-overlap facts, cheap blockers, and scorer component maps were constructed for every ranked
candidate even when eval only needed stable stage denominators. Search now exposes a compact stage-observation DTO for
ordinary eval, and detailed observations remain confined to explicit forensic artifact modes.

## Release Gate Status

`G6 full_audit_performance` is closed for memory/RSS evidence and still blocked by unrelated manual-label quality
evidence:

- **Closed:** full-audit RSS is measurable without `/usr/bin/time -l`; full internal and mathlib audits meet the current
  6.5 GiB target; release eval workloads now meet the same target; report size and parseability remain within target.
- **Still no-go for release overall:** manual suites remain blocked by unresolved/stale labels and current-label recall
  evidence. Prompt 61 must not approve 0.1.0 until Prompt 60 is rerun after the manual-label and memory repairs.

## Red Flag Review

- Shallow module: the compact search-stage observation adds a narrower eval surface that hides detailed feature/scorer
  internals; it is not a pass-through over detailed observations.
- Pass-through wrapper: avoided; no wrapper or forwarding API was introduced.
- Temporal decomposition: avoided; the evidence is organized by workload and cache state, not command execution order.
- Information leakage: acceptable for checked release facts. This document redacts private operator paths and records no
  worker rows, cache layout, retrieval keys, storage tables, backend details, or source snippets.
- Special-general mixture: avoided; fixture evals, manual evals, aggregate eval, and full audits are separated.
- Conjoined methods: avoided; compact ordinary eval and detailed forensic artifact generation remain separate modes.
- Hard-to-describe public API: the memory surface is workload, RSS status, runtime, output size, parseability, and
  denominators; the new search surface is stable pair stage facts.
- Implementation details contaminating interface comments: public comments describe caller-visible stage facts, not
  allocation strategy or scorer internals.
