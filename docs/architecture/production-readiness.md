# Production readiness

The release contract: named gates, required evidence, no-go criteria. Gates close only on
command output or a checked artifact, not prose.

For the pipeline these gates measure, see [end-to-end-architecture.md](end-to-end-architecture.md).
For the search-quality contract that governs `G1`–`G3`, see [search-quality.md](search-quality.md).

## Definition

`lean-dup` is production-ready when it is a local, deterministic, read-only duplicate auditor
with proven correctness, high-precision default output, stable cache behavior, explainable
reports, and reproducible releases. Concretely, all of:

- correctness and regression quality from labeled denominators, not anecdote;
- precision control: default queues do not show weak feature-only or known-bogus mathlib matches
  as actionable;
- recoverable semantic verification that produces proof-grade evidence when source-backed
  declarations are importable;
- explicit source-backed vs static external evidence in reports and JSON;
- shared cache reuse across projects pinned to the same relevant sources; invalidation on Lean
  source, Lake, toolchain, worker, protocol, or semantic-version changes;
- warm full-audit runtime and memory inside a documented production range;
- empty visible queues that explain themselves; hidden evidence summarized; JSON with a
  documented stable contract;
- CI, packaging, version output, doctor diagnostics, and supported-commands docs at release
  grade.

## Gates

| Gate                                | Evidence artifact                                                                                                                            |
| ----------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| `G1 regression_quality`             | [search-quality.md](search-quality.md), [evaluation/production-gates.md](evaluation/production-gates.md), `target/eval/production-gate.json` |
| `G2 precision_control`              | search-quality.md, production-gates.md hard-negative section, fixture and manual eval JSON                                                   |
| `G3 semantic_probe_yield`           | real-workload probe evidence under `target/audit-runs/`                                                                                      |
| `G4 external_comparison_provenance` | [external-comparison-provenance.md](external-comparison-provenance.md) + JSON/profile fixtures                                         |
| `G5 cache_validity_lifecycle`       | [cache-validity-lifecycle.md](cache-validity-lifecycle.md) + `target/cache/doctor-production.json`                                           |
| `G6 full_audit_performance`         | `target/perf/` outputs                                                                                                                       |
| `G7 report_contract`                | [report-contract.md](report-contract.md) + `target/report-contract/` golden outputs                                                          |
| `G8 release_hardening`              | release-hardening.md, CI config, `target/release-diagnostics/`                                                                               |

Production claim per gate:

- **G1**: fixture and manual-corpus quality proven with raw denominators and stage-level search
  metrics.
- **G2**: Hard negatives and known bogus mathlib matches do not leak into the default visible
  queue.
- **G3**: Probes are recoverable, typed, and produce useful proof-grade yield on real workloads.
- **G4**: Source-backed and static external indexes have explicit, user-visible semantics.
- **G5**: Shared caches reuse safely and invalidate only on relevant source, toolchain, worker,
  protocol, or semantic changes.
- **G6**: Warm full audits meet documented runtime and memory targets.
- **G7**: Empty queues, hidden groups, unavailable probes, provenance, and JSON schema are
  explained.
- **G8**: CI, packaging, version output, install docs, and reproducibility are release-grade.

## Required commands

Every release-candidate update runs:

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
(cd lean && lake build)
```

Quality and production gates:

```sh
cargo run -p lean-dup-cli -- eval --suite default --format json \
  --output target/eval/default.json

cargo run -p lean-dup-cli -- eval --suite production-gate --format json \
  --output target/eval/production-gate.json
```

The `production-gate` suite stays manual and may depend on private paths. It does not
become default CI without an explicit runtime and privacy decision. Command completion is
not enough: raw recall and hard-negative denominators must pass.

Full audits (operator supplies the project workspace path as `<workspace>`):

```sh
env LEAN_NUM_THREADS=2 target/release/lean-dup --progress --profile \
  audit --workspace <workspace> --format json \
  > target/audit-runs/full-internal.json

env LEAN_NUM_THREADS=2 target/release/lean-dup --progress --profile \
  audit --workspace <workspace> --compare-mathlib --format json \
  > target/audit-runs/full-mathlib.json

env LEAN_NUM_THREADS=2 target/release/lean-dup --progress --profile \
  audit --workspace <workspace> --compare-mathlib --no-semantic-probes --format json \
  > target/audit-runs/full-mathlib-no-probes.json
```

Targeted audit:

```sh
env LEAN_NUM_THREADS=2 target/release/lean-dup --progress --profile \
  audit --workspace <workspace> \
  --module <Root.Module> --compare-mathlib --format json \
  > target/audit-runs/targeted.json
```

Release diagnostics:

```sh
target/release/lean-dup doctor --format json \
  > target/release-diagnostics/doctor.json

target/release/lean-dup --version \
  > target/release-diagnostics/version.txt
```

Evidence locations: `docs/architecture/evaluation/` for quality gates,
`docs/architecture/validation/` for real-workload inspection, `report-contract.md` and
`release-hardening.md` for their gates; JSON under `target/eval/`, `target/audit-runs/`,
`target/perf/`, `target/report-contract/`, `target/release-diagnostics/`.

## No-go criteria

The release is a no-go if any of these remain true:

- default reports show known hard negatives or weak mathlib noise as actionable;
- full manual-corpus audits fail, require `--no-semantic-probes` for ordinary use, or lack
  clear unavailable-probe diagnostics;
- source-backed and static external evidence are indistinguishable in JSON or text output;
- cache reuse depends on unrelated project dirtiness or absolute paths;
- release artifacts cannot identify binary, Git revision, Lean version, worker version, protocol
  version, index schema, and report schema;
- README examples rely on private local paths for common workflows.
