# Production Readiness

This document is the release contract for `lean-dup`. It converts "not production-ready" into named gates, required
evidence, and no-go criteria. A gate is open until concrete command output, report evidence, or a checked artifact
proves it.

For the current as-built system, see
[06-end-to-end-architecture.md](/Users/jcreinhold/Code/lean-dup/docs/architecture/06-end-to-end-architecture.md).

## Design Note

This document owns the hidden knowledge needed to judge release readiness: the production gate taxonomy, acceptance
evidence, supported workflow contract, no-go criteria, and the distinction between validated capabilities and old
implementation shape.

Its smallest public interface is this architecture document. Later prompts may update gate status and artifact links,
but they should not create parallel release checklists with different definitions of readiness.

These decisions must not leak upward or sideways:

- evaluation label layout, fixture paths, and KanProofs private-path policy;
- cache directory layout, cache-key ingredients, SQLite table names, row ids, and cleanup mechanics;
- semantic-probe chunking, heartbeat recovery, worker transport framing, JSONL parsing, and Lean traversal details;
- prompt sequencing mechanics or temporary migration steps.

The validated user-facing capability to preserve is a read-only local duplicate auditor: it indexes local workspaces,
builds and reuses cached mathlib or external indexes, compares against source-backed evidence where available, runs
bounded semantic verification for actionable findings, emits text and JSON reports, supports `show`, and supports saved
baseline review.

Python-era behavior intentionally discarded:

- treating Python parity as the architecture goal;
- treating Python cache paths, index layout, or scoring heuristics as compatibility contracts;
- preserving string-driven or source-parsing semantic policy in Rust;
- requiring users to run project workflows through Python-era shell conventions.

Production preserves proven capabilities, not Python implementation shape.

## Design It Twice

**Rejected: prompt-by-prompt release checklist.** A checklist organized by prompts 21 through 40 would be easy to write,
but it would be temporally decomposed. It would make release status depend on execution order rather than evidence, and
it would not hide the release policy behind a stable boundary.

**Chosen: gate-centered production contract.** The release boundary is a small set of named gates with commands and
artifacts. This is deeper because future prompts, users, and release reviewers can ask whether a capability is proven
without learning the internal sequence of cache work, probe work, report work, CI work, or Python deletion work.

## Production Definition

`lean-dup` is production-ready when it is a local, deterministic, read-only duplicate auditor with proven correctness,
high-precision default output, stable cache behavior, explainable reports, reproducible releases, and no dependency on
Python-era implementation paths.

Production readiness requires all of the following:

- **Correctness and regression quality:** fixture and KanProofs regression suites prove known true positives, known hard
  negatives, raw denominators, and runtime.
- **Precision and false-positive control:** default visible queues do not show weak feature-only mathlib overlaps or
  known bogus structural collisions as actionable findings.
- **Semantic probe availability and usefulness:** semantic verification is recoverable and produces proof-grade evidence
  when source-backed comparison declarations are importable.
- **External comparison semantics:** source-backed and static indexes are distinguished in reports and JSON, and static
  evidence cannot masquerade as proof-grade semantic evidence.
- **Cache validity and lifecycle:** shared caches reuse across projects pinned to the same relevant sources and
  invalidate on Lean source, Lake, toolchain, worker, protocol, or semantic-version changes.
- **Full-audit runtime and memory:** warm full audits with and without mathlib comparison run in an acceptable,
  documented production range with measured peak memory.
- **Report UX and JSON stability:** empty visible queues explain why they are empty, hidden evidence is summarized, and
  JSON has a documented stable contract.
- **CI, packaging, versioning, and release docs:** local and CI checks build Rust and Lean code, exercise fixture
  workflows, expose release-grade version/doctor diagnostics, and document supported commands.
- **Python-era implementation deprecation:** Python modules, tests, and packaging are removed or quarantined after
  validated Rust/Lean capability preservation.

## Gate Status

Current status is evidence-based, not release-ready. Prompt 27 proved the Rust/Lean production surface can run the
aggregate eval command, but it also exposed quality failures: `target/eval/prompt27-production-gate.json` reports
aggregate recall@10 `15/32`, aggregate hard-negative hits `3/16`, KanProofs/mathlib recall@10 `0/11`, and
KanProofs/mathlib hard-negative hits `3/4`.

Prompts 28 through 37 are now the search-quality prerequisite for release hardening. They are governed by
[search-quality.md](/Users/jcreinhold/Code/lean-dup/docs/architecture/search-quality.md), which defines the match
taxonomy, stage objectives, and quality evidence needed before `G1` and `G2` can close.

| Gate | Status | Production claim | Required evidence artifact |
| --- | --- | --- | --- |
| `G1 regression_quality` | Open | KanProofs and fixture quality are proven with raw denominators and stage-level search metrics. | `docs/architecture/search-quality.md`, `docs/architecture/evaluation/production-gates.md`, and `target/eval/production-gate.json`. |
| `G2 precision_control` | Open | Hard negatives and known bogus mathlib matches do not leak into the default visible queue. | `docs/architecture/search-quality.md`, `docs/architecture/evaluation/production-gates.md` hard-negative section, and fixture/KanProofs eval JSON. |
| `G3 semantic_probe_yield` | Partial | Probes are recoverable and typed, but useful proof-grade yield remains insufficiently validated. | `docs/architecture/performance/prompt-23-semantic-probe-yield.md` plus real-workload probe evidence. |
| `G4 external_comparison_provenance` | Implemented, awaiting validation | Source-backed and static external indexes have explicit, user-visible semantics. | `docs/architecture/05-external-comparison-provenance.md` plus JSON/profile fixture outputs. |
| `G5 cache_validity_lifecycle` | Implemented, awaiting validation | Shared caches reuse safely and invalidate only on relevant source, toolchain, worker, protocol, or semantic changes. | `docs/architecture/cache-validity-lifecycle.md` plus `target/cache/doctor-production.json`. |
| `G6 full_audit_performance` | Partial | Warm full audits meet documented runtime and memory targets. | `docs/architecture/performance/prompt-25-full-audit-throughput.md` plus raw JSON/profile outputs under `target/perf/`. |
| `G7 report_contract` | Implemented, awaiting validation | Empty queues, hidden groups, unavailable probes, provenance, and JSON schema are explained. | `docs/architecture/report-contract.md` plus text/JSON golden outputs under `target/report-contract/`. |
| `G8 release_hardening` | Open | CI, packaging, version output, install docs, and reproducibility are release-grade. | `docs/architecture/release-hardening.md`, CI config, and `target/release-diagnostics/`. |
| `G9 python_deprecation` | Complete | Validated Python-era capabilities are preserved and superseded Python paths are removed. | `docs/architecture/python-deprecation-map.md` plus Prompt 27 eval artifacts. |

## Acceptance Evidence

No gate may be marked complete by prose alone. Each gate needs at least one command transcript or machine-readable
artifact plus the architecture document named above.

Required local checks for every release-candidate gate update:

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cd /Users/jcreinhold/Code/lean-dup/lean && lake build
```

Required quality and production-gate commands:

```sh
cargo run -p lean-dup-rs -- eval --suite default --format json \
  --output target/eval/default.json

cargo run -p lean-dup-rs -- eval --suite production-gate --format json \
  --output target/eval/production-gate.json
```

The `production-gate` suite may remain manual and private-path dependent. It must not become default CI without an
explicit runtime and privacy decision. Command completion is not enough for release readiness; the raw recall and
hard-negative denominators must pass.

Required full-audit commands:

```sh
env LEAN_NUM_THREADS=2 target/release/lean-dup-rs --progress --profile \
  audit --workspace /Users/jcreinhold/Code/kan-proofs --format json \
  > target/audit-runs/kanproofs-full-internal.json

env LEAN_NUM_THREADS=2 target/release/lean-dup-rs --progress --profile \
  audit --workspace /Users/jcreinhold/Code/kan-proofs --compare-mathlib --format json \
  > target/audit-runs/kanproofs-full-mathlib.json

env LEAN_NUM_THREADS=2 target/release/lean-dup-rs --progress --profile \
  audit --workspace /Users/jcreinhold/Code/kan-proofs --compare-mathlib --no-semantic-probes --format json \
  > target/audit-runs/kanproofs-full-mathlib-no-probes.json
```

Required targeted audit command:

```sh
env LEAN_NUM_THREADS=2 target/release/lean-dup-rs --progress --profile \
  audit --workspace /Users/jcreinhold/Code/kan-proofs \
  --module KanProofs.Mathlib4Backports --compare-mathlib --format json \
  > target/audit-runs/kanproofs-mathlib4backports.json
```

Required cache and release diagnostics once implemented:

```sh
target/release/lean-dup-rs doctor --format json \
  > target/release-diagnostics/doctor.json

target/release/lean-dup-rs --version \
  > target/release-diagnostics/version.txt
```

The expected evidence artifacts are:

- `docs/architecture/evaluation/production-gates.md` for quality gates and hard negatives;
- `docs/architecture/performance/` reports for runtime, memory, retrieval, probe, and rendering costs;
- `docs/architecture/validation/` reports for real-workload inspection and production/no-go decisions;
- `docs/architecture/report-contract.md` for stable JSON and report behavior;
- `docs/architecture/release-hardening.md` for CI, packaging, versioning, and reproducibility;
- JSON outputs under `target/eval/`, `target/audit-runs/`, `target/perf/`, `target/report-contract/`, and
  `target/release-diagnostics/`.

## Prompt Map

| Prompt | Gates advanced | Current result |
| --- | --- | --- |
| 21 | `G1`, `G2` | Production-gate eval suites, labels, hard negatives, raw denominators, and quality docs exist. |
| 22 | `G4`, `G7` | Source-backed/static provenance contract exists in index metadata, reports, and JSON. |
| 23 | `G3`, `G2` | Missing/private probe mismatch was fixed; proof-grade yield still needs validation. |
| 24 | `G5`, `G8` | Source-relevant cache fingerprints, doctor diagnostics, and safe cleanup lifecycle exist. |
| 25 | `G6`, `G3`, `G5` | Warm full-audit throughput improved; full mathlib comparison remains expensive. |
| 26 | `G7`, `G2` | Empty-queue explanations, `show` explanations, and stable JSON contract exist. |
| 27 | `G9`, `G1` | Python implementation removed; production-gate command completes but quality gates fail. |
| 28 | `G1`, `G2`, `G3`, `G7` | Search-quality charter and match taxonomy. |
| 29 | `G1`, `G2` | Pending: task-specific label schema and adjudication corpus. |
| 30 | `G1`, `G2` | Pending: stage metrics and retrieval observability. |
| 31 | `G1`, `G2`, `G6` | Pending: pair feature store and search dataset artifacts. |
| 32 | `G1`, `G2`, `G6` | Pending: candidate generation as an explicit high-recall stage. |
| 33 | `G1`, `G2`, `G6` | Pending: calibrated symbolic scorer and ablation harness. |
| 34 | `G2`, `G3` | Pending: semantic reranking and obligation-yield improvements. |
| 35 | `G1`, `G2`, `G6` | Pending: hidden local embedding experiment gate. |
| 36 | `G1`, `G2`, `G7` | Pending: threshold calibration and review policy. |
| 37 | `G1`, `G2`, `G3`, `G6`, `G7` | Pending: search-quality validation and go/no-go. |
| 38 | `G8`, `G5`, `G7` | Pending: CI, packaging, version output, release docs, and reproducibility diagnostics. |
| 39 | `G1`, `G2`, `G3`, `G4`, `G5`, `G6`, `G7` | Pending: real-workload validation on KanProofs and a second project or fixture. |
| 40 | All gates | Pending: final production/no-go document. |

## No-Go Criteria

The release is a no-go if any of these remain true:

- default reports show known hard negatives or weak mathlib noise as actionable findings;
- full KanProofs audits fail, require `--no-semantic-probes` for ordinary use, or lack clear unavailable-probe
  diagnostics;
- source-backed and static external evidence are indistinguishable in JSON or text output;
- cache reuse depends on unrelated project dirtiness, absolute paths, or Python-era layout;
- release artifacts cannot identify binary, Git revision, Lean version, worker version, protocol version, index schema,
  and report schema;
- README examples rely on Python entry points or private local paths for common workflows.

## Red Flag Review

- **Shallow module:** avoided. The document defines release gates, evidence, and no-go policy instead of restating the
  prompt sequence.
- **Pass-through wrapper:** avoided. The charter is not a wrapper around prompts 21 through 40; it gives the prompts a
  shared release contract.
- **Temporal decomposition:** avoided. Gates are organized by production capability, not by implementation order.
- **Information leakage:** avoided. The document names evidence requirements without exposing eval label layout, SQLite
  tables, probe chunking, worker framing, or cache internals as release interfaces.
- **Special-general mixture:** contained. KanProofs is a required real workload, but the production gates are general
  release claims about quality, evidence, cache behavior, reporting, and packaging.
- **Conjoined methods:** no remaining red flag. Each gate has a separable claim and artifact; later prompts can close
  one gate without editing unrelated gate semantics.
- **Hard-to-describe public API:** no remaining red flag. The public interface is one architecture document with named
  gates and required artifacts.
- **Implementation details contaminating interface comments:** avoided. The document describes caller-visible production
  guarantees and evidence paths, not Lean traversal algorithms, SQLite layout, or temporary migration machinery.
