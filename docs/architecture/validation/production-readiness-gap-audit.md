# Production Readiness Gap Audit

Date: 2026-05-20 23:33 EDT

Repository revision inspected: `1d9433d`

## Design Note

Production-readiness docs own the release gate vocabulary, required evidence, and no-go
criteria. Eval suites own label truth, suite lifecycle, and raw denominators. Search owns
symbolic retrieval, ranking, semantic-probe planning, review-profile policy, and default
actionability. Report owns bounded projection, schema `lean-dup.report.v3`, and explanation
facts. CLI diagnostics own operator commands, process output, extension dispatch, and release
identity. Packaging owns reproducible install and CI evidence.

The smallest public interface to a release gate is a checked command or artifact path with a
status, schema/version facts where the artifact has them, raw denominators, and a named owner for
each blocker. Gate logic must not learn retrieval keys, SQLite rows, worker transport records,
report rendering internals, cache layout, semantic/vector internals, or private manual-corpus
paths. The preserved user-facing capability is the current symbolic, read-only audit/eval/report
workflow. The intentionally discarded Python-era behavior is treating ad hoc scripts, enormous
unbounded dumps, skipped workloads, and fixture-only evidence as release proof.

## Design It Twice

Three audit boundaries were considered.

1. Treat green unit tests and fixture evals as enough. This is shallow because it collapses
   regression quality, manual-corpus coverage, probe yield, report contract, cache lifecycle, and
   release diagnostics into one vague "tests pass" signal.
2. Write a prose checklist and defer missing evidence to the final release prompt. This preserves
   uncertainty and creates temporal decomposition: each later prompt must rediscover which gate
   needed which artifact.
3. Build a gate-by-gate artifact map for `G1` through `G8`, including commands, raw denominators,
   blockers, owners, and follow-up prompts.

The gate-by-gate artifact map is the chosen design. It keeps release logic in one evidence-oriented
document while search, eval, report, CLI, cache, and packaging keep their own mechanisms. It also
keeps semantic/vector facts out of release gating unless Prompt 45 later writes an explicit
allowance.

## Evidence Policy

Prompt 45 has not produced
`docs/architecture/evaluation/semantic-theorem-profile-validation-decision.md`; the file is
missing in this checkout. Semantic/vector facts are therefore ignored for Prompt 46 calibration and
for this readiness audit. Prompt 35K, Prompt 35Q, Prompt 35Y, and vector-slice parity evidence are
historical or hidden-experiment evidence only.

No additional prompts were added by this investigation. Every blocker found here fits an existing
follow-up prompt in the 53-61 production-readiness sequence.

## Commands Run

| Command | Status | Artifact or output | Notes |
| --- | --- | --- | --- |
| `cargo run -p lean-dup-cli -- eval --suite default --format json --output target/eval/default.json` | passed | `target/eval/default.json` | Fixture eval completed with `status = ok`. |
| `cargo run -p lean-dup-cli -- eval --suite hard-negatives --format json --output target/eval/hard-negatives.json` | passed | `target/eval/hard-negatives.json` | Hard-negative eval completed with `status = ok`. |
| `cargo run -p lean-dup-cli -- eval --suite production-gate --format json --output target/eval/production-gate.json` | incomplete | `target/eval/production-gate.json` | Manual suites skipped; aggregate `status = incomplete`. |
| `cargo run -p lean-dup-cli -- --help` | passed | stdout | Lists built-ins and `--list`; no static vector command. |
| `cargo run -p lean-dup-cli -- --version` | failed | stderr | Clap rejects `--version`: `unexpected argument '--version' found`. |
| `cargo run -p lean-dup-cli -- doctor --workspace tests/fixtures/tiny --module Tiny --format json > target/cache/doctor-production.json` | passed | `target/cache/doctor-production.json` | Fixture-scale cache/worker diagnostic only. |
| `cargo run -p lean-dup-cli -- audit --workspace tests/fixtures/tiny --module Tiny --no-semantic-probes --format json > target/report-contract/fixture-audit.json` | passed | `target/report-contract/fixture-audit.json` | Fixture-scale report-contract artifact only. |
| `cargo test -p lean-dup-cli --test boundaries` | passed | command output | 7 boundary tests passed. |
| `cargo fmt --check` | passed | command output | Formatting check passed. |

## Artifact Summary

| Artifact | Schema/version facts | Size | Parse status |
| --- | --- | ---: | --- |
| `target/eval/default.json` | no `schema_version`; `scorer_version = lean-dup.symbolic-scorer.v1` | 4,212 bytes | parsed with `jq` |
| `target/eval/hard-negatives.json` | no `schema_version`; `scorer_version = lean-dup.symbolic-scorer.v1` | 4,197 bytes | parsed with `jq` |
| `target/eval/production-gate.json` | no `schema_version`; `scorer_version = lean-dup.symbolic-scorer.v1` | 14,549 bytes | parsed with `jq` |
| `target/cache/doctor-production.json` | no report schema; includes Lean version and cache fingerprint | 77,815 bytes | parsed with `jq` |
| `target/report-contract/fixture-audit.json` | `report_schema_version = lean-dup.report.v3` | 55,667 bytes | parsed with `jq` |

## Gate Map

### G1 `regression_quality`

Status: incomplete.

Owner: `lean-dup-eval`, `lean-dup-search`, production-gate docs.

Classification: correctness and coverage.

Evidence:

- `target/eval/default.json`: `status = ok`.
- `target/eval/hard-negatives.json`: `status = ok`.
- `target/eval/production-gate.json`: `status = incomplete`.

Raw denominators:

| Suite | Recall@1 | Recall@5 | Recall@10 | Candidate-generation recall | Visible positives | Candidate count | Timing |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `default` | 7/16 | 16/16 | 16/16 | 16/16 | 14/16 | 299 | 3,342 ms |
| `hard-negatives` | 0/1 | 1/1 | 1/1 | 1/1 | 1/1 | 299 | 798 ms |
| `production-gate` | 7/17 | 17/17 | 17/17 | 17/17 | 15/17 | 598 | 1,230 ms |

Blocker:

Manual-corpus coverage is missing. `production-gate` skipped `manual-internal` and
`manual-mathlib` with reason `manual suite workspace is unavailable`. Fixture recall is good at
`k = 5` and `k = 10`, but release quality requires manual-corpus denominators.

Expected fixing prompt: Prompt 53.

### G2 `precision_control`

Status: incomplete.

Owner: `lean-dup-search`, `lean-dup-eval`, `lean-dup-report`.

Classification: precision.

Evidence:

- Fixture hard-negative visible leakage is zero.
- Default and aggregate shown-queue precision remain low enough that calibrated production policy
  still needs evidence.

Raw denominators:

| Suite | Shown queue precision | Hard-negative visible hits | Hard negatives generated | Visible groups |
| --- | ---: | ---: | ---: | ---: |
| `default` | 14/34 | 0/3 | 3/3 | 25/39 |
| `hard-negatives` | 1/34 | 0/5 | 2/5 | 25/39 |
| `production-gate` | 15/68 | 0/8 | 5/8 | 50/78 |

Blocker:

The current fixture artifacts show zero hard-negative leakage, but precision is not calibrated
against manual or realistic workloads. A release claim would overfit fast fixtures.

Expected fixing prompt: Prompt 54.

### G3 `semantic_probe_yield`

Status: blocked.

Owner: `lean-dup-search`, `lean-dup-worker`, `lean-dup-eval`.

Classification: correctness and observability.

Evidence:

- Fast eval artifacts record semantic reranking version `lean-dup.semantic-reranking.v1`.
- All required eval runs here had `planned = 0`, `worker = 0`, `unavailable = 0`.
- No real-workload probe artifact exists under `target/audit-runs/`.

Raw denominators:

| Suite | Planned probes | Worker probes | Unavailable probes |
| --- | ---: | ---: | ---: |
| `default` | 0 | 0 | 0 |
| `hard-negatives` | 0 | 0 | 0 |
| `production-gate` | 0 | 0 | 0 |

Blocker:

No artifact demonstrates source-backed semantic probe yield, rejection reasons, timeout behavior,
or fallback policy on a real workload. Fixture eval cannot close this gate because it did not plan
probes.

Expected fixing prompt: Prompt 55.

### G4 `external_comparison_provenance`

Status: blocked.

Owner: `lean-dup-index`, `lean-dup-search`, `lean-dup-report`.

Classification: correctness and documentation.

Evidence:

- `docs/architecture/external-comparison-provenance.md` defines typed evidence modes:
  `proof-grade`, `source-backed-not-importable`, and `static`.
- No JSON/profile fixture artifact was found under the production evidence paths.
- The fixture audit artifact has an `explanations.comparison_provenance` key, but it is not a
  source-backed/static comparison lifecycle validation.

Raw denominators:

- No completed source-backed-vs-static validation denominator is available in this audit.

Blocker:

The release gate still lacks command-level evidence distinguishing source-backed mathlib evidence
from static external evidence, including stale/missing provenance behavior.

Expected fixing prompt: Prompt 57.

### G5 `cache_validity_lifecycle`

Status: incomplete.

Owner: `lean-dup-index`, `lean-dup-project`, `lean-dup-cli`.

Classification: correctness and observability.

Evidence:

- `target/cache/doctor-production.json`: `status = ok`.
- `lean_version = Lean 4.30.0-rc2`.
- `requested_workspace = /Users/jcreinhold/Code/lean-dup/tests/fixtures/tiny`.
- `source_count = 3`, `selected_roots = ["Tiny"]`, `missing_oleans = 0`.
- `cache_root = /Users/jcreinhold/.cache/lean-dup`.
- `cache_fingerprint = rust-cli-cache.v1:110a3e453c253620570cb59c6e2693d8aea12480e7c24619898beab4d0c3abc8`.

Raw denominators:

- Doctor reported `cache_entries = 0` for the fixture diagnostic context.

Blocker:

This confirms the doctor command can emit fixture diagnostics, but it does not validate cache reuse,
dirty-project behavior, stale cache invalidation, schema/protocol version checks, or lifecycle
diagnostics over populated caches. The artifact also contains absolute local paths, which may be
acceptable for local diagnostics but is not yet a release-artifact privacy decision.

Expected fixing prompt: Prompt 57.

### G6 `full_audit_performance`

Status: blocked.

Owner: `lean-dup-cli`, `lean-dup-search`, `lean-dup-report`, `lean-dup-diagnostics`.

Classification: performance and observability.

Evidence:

- No `target/perf/` artifacts were found.
- No `target/audit-runs/` full internal, full mathlib, or no-probes audit artifacts were found.
- Eval fixture peak RSS was small: default 11,599,872 bytes; hard-negatives 11,583,488 bytes;
  production-gate 12,451,840 bytes.

Raw denominators:

- No full-audit runtime, warm-cache runtime, full-audit RSS, output size, or interruption data is
  available.

Blocker:

Fixture eval cost cannot close a full-audit performance gate. The previous oversized-report issue
makes this a release blocker until real full audits prove runtime, RSS, output size, parseability,
and cache reuse.

Expected fixing prompt: Prompt 56.

### G7 `report_contract`

Status: incomplete.

Owner: `lean-dup-report`, `lean-dup-cli`.

Classification: observability and contract stability.

Evidence:

- `target/report-contract/fixture-audit.json` has `report_schema_version = lean-dup.report.v3`.
- `visible_group_count = 15`, `visible_groups_emitted = 15`, `visible_group_limit = 500`,
  `visible_groups_truncated = false`.
- `review` is compact: `group_count = 22`, `suppressed_count = 68`,
  `diagnostics.candidate_pairs = 44`, `diagnostics.emitted_groups = 22`,
  `diagnostics.suppressed_groups = 68`.
- `explanations` has keys `comparison_provenance`, `hidden_groups`, `semantic_probes`,
  `visible_queue`.

Raw denominators:

| Artifact | Visible count | Emitted | Limit | Truncated | Review group count | Suppressed |
| --- | ---: | ---: | ---: | --- | ---: | ---: |
| `target/report-contract/fixture-audit.json` | 15 | 15 | 500 | false | 22 | 68 |

Blocker:

The fixture artifact supports the v3 report contract shape, but golden artifacts, text output,
`show` detail stability, leak checks, and README-compatible examples are not yet locked. This is
fixture evidence only.

Expected fixing prompt: Prompt 58.

### G8 `release_hardening`

Status: blocked.

Owner: `lean-dup-cli`, CI/package docs, release docs.

Classification: packaging and diagnostics.

Evidence:

- `cargo run -p lean-dup-cli -- --help` passed.
- `cargo run -p lean-dup-cli -- --version` failed:
  `error: unexpected argument '--version' found`.
- `cargo test -p lean-dup-cli --test boundaries` passed: 7 tests passed.
- `cargo fmt --check` passed.

Blocker:

Release version output is missing. Release diagnostics do not yet identify binary version, Git
revision, worker/protocol/index/report schema versions, and supported command state from a single
release-grade command set. CI/package/install docs were not validated in this investigation.

Expected fixing prompt: Prompt 59.

## Overall Decision

Current 0.1.0 readiness status: no-go.

The symbolic fast fixtures are useful evidence, but the release gate is not closed. The aggregate
production-gate suite is incomplete because manual suites are unavailable; semantic-probe yield is
unmeasured on real workloads; full-audit performance artifacts are absent; external provenance and
cache lifecycle evidence is fixture-only or missing; report v3 has only a fixture artifact; and
`lean-dup --version` is not implemented.

## Red Flag Review

- Shallow module: not introduced by this artifact. The artifact maps gates to owning modules rather
  than defining a new release module with pass-through responsibilities.
- Pass-through wrapper: not present.
- Temporal decomposition: mitigated by grouping evidence by gate ownership instead of by command
  execution order.
- Information leakage: remaining risk in local diagnostic artifacts that contain absolute paths.
  Prompt 57 and Prompt 59 must decide which release artifacts may expose paths.
- Special-general mixture: avoided here by keeping semantic/vector experiment evidence separate
  from symbolic release gates.
- Conjoined methods: not applicable to this documentation-only audit.
- Hard-to-describe public API: release status remains hard to describe until G1-G8 have stable
  artifact shapes; this is exactly the blocker sequence 53-61 addresses.
- Implementation details contaminating interface comments: no public interface comments changed.

