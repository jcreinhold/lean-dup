# Full Audit Performance And Memory Bounds

Date: 2026-05-21

Measured revision: `a41ada9`

## Design Note

Workspace discovery owns Lake/project shape, selected roots, and source enumeration. Index owns
cache validity, persisted symbolic facts, local/mathlib reuse, and invalidation. Worker owns Lean
subprocess/protocol, import, extraction, semantic feature generation, and probe transport. Search
owns retrieval, probe planning, probe cache use, and review policy. Report owns bounded projection
and JSON/text size. CLI/progress owns operator-visible execution, stderr progress/profile events,
and output routing.

The smallest public performance interface is a named command plus stable cost facts: cache state,
phase timings, RSS status, JSON size, `jq '.status'` parse time, candidate count, visible group
count, emitted group count, probe cache hits, worker probe attempts, and exact blockers. Cache
layout, SQLite tables, worker transport, retrieval keys, report materialization internals, and
platform-specific RSS mechanics must not leak upward or sideways.

The preserved user-facing capability is read-only symbolic auditing with semantic probes enabled.
The intentionally discarded Python-era behavior is using unbounded dumps, skipped workloads, or
anecdotal timing as release performance evidence.

## Design It Twice

Three performance boundaries were considered.

1. **Set release targets from intuition.** Rejected. It would repeat the earlier oversized-report
   mistake by deciding from expectations rather than workload artifacts.
2. **Optimize every slow-looking component first.** Rejected. Without before/after evidence, this
   risks complexity in worker, index, search, or report internals without proving release value.
3. **Measure named cold/warm audits, then optimize only measured bottlenecks.** Chosen. Each crate
   keeps its hidden mechanism while the release artifact exposes stable cost facts.

No optimization was applied in this session. The measurements show bounded report output and strong
warm-cache reuse. The remaining pressure point is RSS, which is high but measurable and below the
release target proposed here.

## Environment

Machine:

- macOS 26.4.1 (`25E253`) on `Mac16,8`;
- Darwin `25.4.0`;
- 12 hardware threads;
- 25,769,803,776 bytes physical memory.

Toolchain:

- Rust `1.95.0 (59807616e 2026-04-14)`;
- KanProofs Lake environment: Lean `4.30.0-rc2`, Lake `5.0.0-src+3dc1a08`;
- shell `LEAN_NUM_THREADS=2` for KanProofs workloads;
- binary profile: `target/release/lean-dup`;
- progress/profile enabled on audit commands;
- RSS measured with `/usr/bin/time -l`;
- JSON parseability measured with `/usr/bin/time -l jq '.status'`.

Local prerequisites:

- `/Users/jcreinhold/Code/kan-proofs` exists;
- compiled KanProofs and mathlib oleans exist under the project Lake graph;
- `/Users/jcreinhold/Code/kan-proofs/.lake/packages/mathlib` exists.

## Commands

Fixture cold/warm:

```sh
env LEAN_DUP_CACHE_DIR=target/perf/cache/fixture \
  target/release/lean-dup --progress --profile audit \
  --workspace tests/fixtures/tiny --module Tiny --format json
```

Source-backed fixture cold/warm:

```sh
env LEAN_DUP_CACHE_DIR=target/perf/cache/source-backed \
  target/release/lean-dup --progress --profile audit \
  --workspace tests/fixtures/source-backed --module Tiny --format json
```

KanProofs internal cold/warm:

```sh
env LEAN_DUP_CACHE_DIR=target/perf/cache/kanproofs-internal LEAN_NUM_THREADS=2 \
  target/release/lean-dup --progress --profile audit \
  --workspace /Users/jcreinhold/Code/kan-proofs --module KanProofs --format json
```

KanProofs with mathlib comparison cold/warm:

```sh
env LEAN_DUP_CACHE_DIR=target/perf/cache/kanproofs-mathlib LEAN_NUM_THREADS=2 \
  target/release/lean-dup --progress --profile audit \
  --workspace /Users/jcreinhold/Code/kan-proofs --module KanProofs \
  --compare-mathlib --format json
```

Artifacts:

- `target/audit-runs/perf-fixture-cold.json`
- `target/audit-runs/perf-fixture-warm.json`
- `target/audit-runs/perf-source-backed-cold.json`
- `target/audit-runs/perf-source-backed-warm.json`
- `target/audit-runs/perf-kanproofs-internal-cold.json`
- `target/audit-runs/perf-kanproofs-internal-warm.json`
- `target/audit-runs/perf-kanproofs-mathlib-cold.json`
- `target/audit-runs/perf-kanproofs-mathlib-warm.json`
- timing/progress stderr under `target/perf/perf-*.stderr`
- `jq '.status'` timing stderr under `target/perf/*-jq.stderr`

## Results

All measured audits completed with `status = ok` and `report_schema_version =
lean-dup.report.v3`.

| Workload | Cache | Runtime | Peak RSS | JSON size | `jq '.status'` | Candidates | External hydrated | Visible/emitted | Probe worker/cache |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| fixture | cold | 1.49 s | 655,769,600 B | 21,584 B | 0.00 s | 260 | 0 | 5/5 | 18/0 |
| fixture | warm | 0.44 s | 655,671,296 B | 21,584 B | 0.00 s | 260 | 0 | 5/5 | 0/18 |
| source-backed fixture | cold | 0.80 s | 655,507,456 B | 3,578 B | 0.00 s | 0 | 0 | 0/0 | 0/0 |
| source-backed fixture | warm | 0.44 s | 655,704,064 B | 3,578 B | 0.00 s | 0 | 0 | 0/0 | 0/0 |
| KanProofs internal | cold | 126.41 s | 5,627,068,416 B | 38,859 B | 0.00 s | 556,455 | 0 | 4/4 | 500/0 |
| KanProofs internal | warm | 9.04 s | 5,647,597,568 B | 38,858 B | 0.00 s | 556,455 | 0 | 4/4 | 0/500 |
| KanProofs + mathlib | cold | 418.26 s | 5,894,995,968 B | 10,499 B | 0.00 s | 574,673 | 13,630 | 0/0 | 212/0 |
| KanProofs + mathlib | warm | 15.10 s | 5,578,211,328 B | 10,498 B | 0.00 s | 574,673 | 13,630 | 0/0 | 0/212 |

Cache sizes after warm runs:

| Cache root | Size |
| --- | ---: |
| `target/perf/cache/fixture` | 196 KiB |
| `target/perf/cache/source-backed` | 64 KiB |
| `target/perf/cache/kanproofs-internal` | 93 MiB |
| `target/perf/cache/kanproofs-mathlib` | 3.2 GiB |

Phase timings from profile stderr:

| Workload | Cache | Local index | Mathlib index | Retrieval |
| --- | --- | ---: | ---: | ---: |
| fixture | cold | 887 ms | n/a | 1 ms |
| fixture | warm | 440 ms | n/a | 0 ms |
| source-backed fixture | cold | 804 ms | n/a | 0 ms |
| source-backed fixture | warm | 438 ms | n/a | 0 ms |
| KanProofs internal | cold | 19,495 ms | n/a | 9,155 ms |
| KanProofs internal | warm | 1,199 ms | n/a | 5,763 ms |
| KanProofs + mathlib | cold | 21,119 ms | 333,872 ms | 15,063 ms |
| KanProofs + mathlib | warm | 1,185 ms | reused | 10,686 ms |

Probe facts:

| Workload | Cache | Planned | Worker pairs | Cached hits | Verified | Rejected | Unavailable |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| fixture | cold | 18 | 18 | 0 | 17 | 1 | 0 |
| fixture | warm | 18 | 0 | 18 | 17 | 1 | 0 |
| KanProofs internal | cold | 500 | 500 | 0 | 37 | 293 | 170 |
| KanProofs internal | warm | 500 | 0 | 500 | 37 | 293 | 170 |
| KanProofs + mathlib | cold | 212 | 212 | 0 | 0 | 176 | 148 |
| KanProofs + mathlib | warm | 212 | 0 | 212 | 0 | 176 | 148 |

## Cache Reuse Assessment

Warm-cache reuse is effective for both indexes and semantic probes:

- fixture warm run used 18 cached probe hits and 0 worker probe pairs;
- KanProofs internal warm run used 500 cached probe hits and 0 worker probe pairs;
- KanProofs mathlib warm run used 212 cached probe hits and 0 worker probe pairs;
- warm KanProofs + mathlib omitted `profile.worker.index`, which indicates the 312,711-declaration
  mathlib index was reused rather than rebuilt;
- warm JSON metrics matched cold metrics for candidate counts, visible/emitted counts, and probe
  outcome counts.

## Release Targets

These targets are release-candidate targets for the measured machine class and must be rechecked in
Prompt 60:

| Target | Bound | Evidence |
| --- | ---: | --- |
| Warm KanProofs internal audit | <= 30 s | measured 9.04 s |
| Warm KanProofs + mathlib audit | <= 45 s | measured 15.10 s |
| Cold KanProofs internal audit | <= 3 min | measured 126.41 s |
| Cold KanProofs + mathlib audit | <= 10 min | measured 418.26 s |
| Peak RSS | <= 6.5 GiB | measured max 5.89 GB RSS |
| Ordinary audit JSON size | <= 25 MiB | measured max 38,859 B |
| `jq '.status'` parse time | <= 2 s | measured 0.00 s for all outputs |
| Mathlib cache size | <= 4 GiB | measured 3.2 GiB |

The RSS target is the tightest bound. Warm runs still hold about 5.6 GiB RSS, so Prompt 60 should
treat any regression above the 6.5 GiB bound as a release blocker. This session did not identify a
single safe, focused optimization with before/after evidence; memory reduction remains future
performance work, not a patch made from intuition.

## G6 Assessment

`G6 full_audit_performance` is provisionally improved from blocked to measured.

Closed for this machine:

- full KanProofs internal audit completed with probes enabled;
- full KanProofs + mathlib comparison completed with probes enabled;
- cold and warm cache states were measured separately;
- warm runs reused probe caches and indexes;
- JSON output remained bounded and parseable;
- progress/profile stderr exposed long phases instead of an opaque run.

Still open for release:

- targets are based on one local machine and one repository revision;
- RSS remains high enough that release-candidate validation must enforce the bound;
- no interruption/resume behavior was measured in this prompt;
- the source-backed fixture used here is a small second workload, not an independent large
  production corpus.

## Red Flag Review

- Shallow module: avoided. The artifact records stable cost facts instead of exposing internals.
- Pass-through wrapper: avoided. The measurement interface is command-level evidence, not a thin
  wrapper around worker/index calls.
- Temporal decomposition: avoided. Results are organized by workload/cache state and stable cost
  facts rather than by implementation order.
- Information leakage: acceptable. Local artifact paths appear in commands because this is a local
  validation artifact, but report JSON remains bounded and does not expose worker rows, SQLite
  tables, retrieval keys, or proof obligations.
- Special-general mixture: avoided. Fixture, KanProofs, and mathlib workloads use the same audit
  command surface; fixture facts are not counted as mathlib-scale evidence.
- Conjoined methods: avoided. Search/report behavior can be evaluated from emitted denominators
  without reading worker/index internals.
- Hard-to-describe public API: avoided. The release-facing facts are runtime, RSS, report size,
  parse time, cache state, and counters.
- Implementation details contaminating interface comments: avoided. No public API comments were
  changed in this prompt.
