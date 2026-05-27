# Release-candidate validation

Prompt 60 reran the 0.1.0 release-candidate matrix after the production-readiness repairs from Prompts 52 through 59.
This artifact is the evidence map for Prompt 61. It is not a release approval.

## Design note

Release validation owns the interpretation of the matrix and the final G1-G8 gate map. Eval suites expose raw
denominators and status; full audits expose runtime, RSS, output size, parse time, cache state, and visible counts;
report and doctor expose stable schema/version facts; packaging diagnostics expose release identity. The smallest public
surface for the final decision is this artifact: command outcomes, checked artifact paths, gate statuses, and blockers.
Private workspace paths, cache layout, worker rows, report assembly details, and CI implementation details do not become
release claims.

The preserved user-facing capability is the symbolic Lean duplicate audit with bounded ordinary reports, source-backed
external comparison, semantic probe status, release diagnostics, and cache-aware operation. The Python-era behavior
discarded here is release judgment from scattered logs, fixture-only evidence, or unbounded forensic report payloads.

## Design it twice

Three validation designs were considered:

1. Trust the Prompt 52-59 session checks. This was rejected because it would force Prompt 61 to reconstruct release
   state from scattered artifacts and old command output.
2. Rerun only fast CI and fixture gates. This was rejected because the production question is about real KanProofs and
   mathlib behavior, not only unit coverage.
3. Run one release matrix that rechecks quality, performance, diagnostics, report contract, real workloads, and
   boundary tests from clean artifacts. This was chosen. It is deeper because each owning layer exposes stable facts,
   while the release decision consumes one coherent evidence artifact.

## Matrix setup

- Git revision reported by `lean-dup --version`: `7e43ccbbb0a4`.
- Release binary: `target/release/lean-dup`, built with `cargo build --release -p lean-dup-cli`.
- Cache roots were kept under `target/release-candidate/cache/` for sandboxed repeatability.
- KanProofs workspace: local operator-supplied workspace with module selector `KanProofs`.
- Mathlib workspace: project-pinned mathlib source tree.
- Semantic/vector facts: not used. Prompt 45 did not allow semantic/vector evidence into release calibration.

The first eval attempt without `LEAN_DUP_CACHE_DIR` tried to write the default user cache outside the sandbox and was
rerun with an explicit target-local cache root. This is a validation-environment setup issue, not release evidence.

`/usr/bin/time -l` successfully ran the full audit commands but returned a nonzero process status after command success
because this environment denied `sysctl kern.clockrate`. Runtime lines were recorded; peak RSS from that wrapper is
unavailable for the full audit commands. Eval artifacts still recorded peak memory.

## Command results

| Check | Command or artifact | Result |
| --- | --- | --- |
| Formatting | `cargo fmt --check` | Passed. |
| Workspace tests | `cargo test` | Passed. `/usr/bin/time -l cargo test` recorded `47.37 real` and max RSS `655998976` bytes. |
| Clippy | `cargo clippy --all-targets -- -D warnings` | Passed. |
| Lean build | `(cd lean && lake build)` | Passed: `Build completed successfully (14 jobs)`. |
| Core-without-vector tests | `LEAN_DUP_CACHE_DIR=target/release-candidate/cache/core-tests cargo test --workspace --exclude lean-dup-vector-search --exclude lean-dup-embedding --exclude lean-dup-vector-index` | Passed after using target-local cache. |
| Boundary tests | `cargo test -p lean-dup-cli --test boundaries` | Passed. |
| Version | `target/release/lean-dup --version` | Passed; artifact `target/release-candidate/diagnostics/version.txt`. |
| Doctor | `target/release/lean-dup doctor --workspace tests/fixtures/tiny --module Tiny --format json` | Passed; artifact `target/release-candidate/diagnostics/doctor.json`. |
| Report contract | Fixture ordinary audit plus `jq` schema/truncation check | Passed; artifact `target/release-candidate/report-contract/ordinary-audit.json`. |
| Leak check | `rg` over diagnostics/report-contract artifacts for private paths, worker rows, proof obligations, backend/cache vocabulary | Passed with no matches. |

## Eval evidence

All eval commands wrote `lean-dup.report.v3` artifacts under `target/release-candidate/eval/`.

| Suite | Status | Recall@1 | Recall@5 | Recall@10 | Candidate-generation recall | Visible precision | Hard-negative visible hits | Visible groups | Candidate count | Peak memory | Total time |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| default | ok | 7/16 | 16/16 | 16/16 | 16/16 | 8/8 | 0/3 | 7/39 | 299 | 12,009,472 B | 1,457 ms |
| hard-negatives | ok | 0/1 | 1/1 | 1/1 | 1/1 | 1/8 | 0/5 | 7/39 | 299 | 11,894,784 B | 1,456 ms |
| manual-internal | ok | 0/6 | 0/6 | 0/6 | 0/6 | 0/4 | 0/3 | 8/7,820 | 556,455 | 6,605,783,040 B | 47,840 ms |
| manual-mathlib | ok | 0/11 | 0/11 | 0/11 | 0/11 | 0/4 | 0/4 | 8/7,830 | 574,673 | 6,695,862,272 B | 563,029 ms |
| production-gate | ok | 7/34 | 17/34 | 17/34 | 17/34 | 9/24 | 0/15 | 30/15,728 | 1,131,726 | 7,326,777,344 B | 83,105 ms |

Manual prerequisites were present for both manual suites:

- `manual-internal`: workspace resolved, labels parsed as 6 positives and 3 hard negatives, compiled oleans present.
- `manual-mathlib`: workspace resolved, labels parsed as 11 positives and 4 hard negatives, mathlib source and oleans
  present, source-backed comparison index available.

These were not skipped manual suites. They ran and produced no positive recall. That is a release blocker.

## Full-audit evidence

Artifacts were written under `target/release-candidate/audit/`.

| Workload | Status | Runtime | RSS | JSON size | `jq '.status'` parse time | Cache state | Candidate count | Visible groups | Probe summary |
| --- | --- | ---: | --- | ---: | ---: | --- | ---: | ---: | --- |
| KanProofs internal cold | ok | 110.02 s | unavailable from `time -l` | 39,356 B | 0.00 s | cold cache root | 556,455 | 4 emitted / 4 total | 500 planned, 37 verified, 293 rejected, 170 unavailable |
| KanProofs internal warm | ok | 9.21 s | unavailable from `time -l` | 39,355 B | 0.00 s | reused semantic probe/index cache | 556,455 | 4 emitted / 4 total | 500 cached, 0 worker pairs, 37 verified, 170 unavailable |
| KanProofs mathlib comparison | ok | 502.13 s | unavailable from `time -l` | 10,677 B | 0.00 s | cold mathlib cache root | 574,673 | 0 emitted / 0 total | 212 planned, 0 verified, 176 rejected, 148 unavailable |

The internal warm-cache audit demonstrates cache reuse: worker pairs dropped from 500 to 0 and runtime dropped from
110.02 seconds to 9.21 seconds. The mathlib audit completed and used source-backed proof-grade provenance with 312,711
external declarations and 13,630 hydrated external candidates.

The report-size and parseability target passed. The RSS gate remains incomplete for full audits because peak RSS was not
available from the timing wrapper in this environment. The eval production-gate artifact recorded
7,326,777,344 bytes, above the 6.5 GiB target recorded in the performance work, so memory remains a release blocker even
without full-audit RSS.

## Diagnostics and report contract

`lean-dup --version` produced:

```text
lean-dup 0.1.0
package: lean-dup-cli
git revision: 7e43ccbbb0a4
build profile: release
report schema: lean-dup.report.v3
index schema: lean-dup.index.v2
cache key: rust-cli-cache.v1
worker: run `lean-dup doctor --workspace <workspace> --format json` for Lean worker facts
```

`doctor --format json` on the fixture workspace reported `status: ok`, `report_schema_version:
lean-dup.report.v3`, `index_schema_version: lean-dup.index.v2`, cache key `rust-cli-cache.v1`, worker protocol
`lean-dup.worker.v1`, worker version `0.1.0`, and Lean `4.30.0`.

The ordinary report-contract fixture satisfied:

- schema `lean-dup.report.v3`;
- bounded `visible_groups`;
- `visible_groups_emitted <= visible_group_limit`;
- no duplicated full `review.groups`;
- no leak-check matches for private paths, worker rows, proof obligations, storage/backend vocabulary, or cache layout.

## Gate map

| Gate | Status | Evidence |
| --- | --- | --- |
| G1 regression_quality | fail | Fixture suites pass, but real manual suites are zero-recall: `manual-internal` 0/6 and `manual-mathlib` 0/11 at candidate generation, ranking, and visible stages. Aggregate production-gate recall@10 is 17/34. |
| G2 precision_control | fail | Default visible precision is 8/8 and hard negatives do not leak visibly, but real manual visible precision is 0/4 for both manual suites and aggregate visible precision is 9/24. The queue is bounded but not yet release-actionable on real labels. |
| G3 semantic_probe_yield | incomplete | Full internal audit has useful exact-theorem yield, but mathlib comparison has 212 planned probes, 0 verified, 176 rejected, and 148 unavailable. Ordinary audits complete with probes enabled, but source-backed mathlib probe yield is not release-grade. |
| G4 external_comparison_provenance | pass | Mathlib comparison reports source-backed proof-grade provenance, 312,711 external declarations, and no private-path/backend leaks in release diagnostics. |
| G5 cache_validity_lifecycle | pass with caveat | Doctor/version facts are present, target-local cache roots work, and warm internal audit reuses cached semantic results. The sandbox rerun also shows release validation should pass an explicit cache root when default user cache is unavailable. |
| G6 full_audit_performance | fail | JSON size and parseability pass, and warm-cache reuse is strong. RSS is unavailable for full audits in this environment, and eval production-gate peak memory is 7,326,777,344 bytes, above the 6.5 GiB target. |
| G7 report_contract | pass | `lean-dup.report.v3` ordinary JSON is bounded, parseable, no `review.groups` duplication, and leak checks pass. |
| G8 release_hardening | pass | `--version`, `doctor --format json`, fmt, tests, clippy, Lean build, report-contract checks, and boundary/vector isolation checks pass. |

## Blockers and follow-up

This is a no-go release-candidate validation. The blockers are not skipped prerequisites; they are completed workloads
with failing quality or missing memory evidence.

1. Manual production-quality recall is zero on the real KanProofs labels. The next repair needs to diagnose whether the
   labels are stale, the manual fixtures identify declarations that no longer exist, the candidate-generation features
   fail to connect the intended pairs, or review policy/ranking suppresses valid generated pairs. Labels and hard
   negatives must not be weakened to make the gate pass.
2. Release memory evidence is not closed. Full-audit peak RSS was unavailable from the local timing wrapper, and the
   aggregate eval artifact exceeds the documented RSS target. The next repair must either reduce measured memory or
   revise the target from better evidence, not hide memory behind quality metrics.

Prompt 61 should not approve 0.1.0 from this matrix. Follow-up prompt files were added to the prompt sequence to repair
these release-candidate blockers before rerunning Prompt 60 and then Prompt 61.

## Red flag review

- Shallow module: no new module was added; the artifact consumes stable facts from existing owning layers.
- Pass-through wrapper: no wrapper was introduced.
- Temporal decomposition: release interpretation is centralized here instead of spread across the prompt session order.
- Information leakage: the artifact references stable artifact paths and aggregate facts, not worker rows, cache layout,
  or backend/storage internals.
- Special-general mixture: fixture evidence and real-workload evidence are separated.
- Conjoined methods: quality, performance, diagnostics, and report contract are separate gates.
- Hard-to-describe public API: the release surface is one gate map plus command/artifact facts.
- Implementation details contaminating interface comments: no public API comments were changed.

