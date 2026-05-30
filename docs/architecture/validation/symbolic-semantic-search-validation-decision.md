# Symbolic Semantic Search Validation Decision

Date: 2026-05-21 Revision: `99d1027` Decision: release-blocked for symbolic semantic search readiness.

This artifact validates the repaired symbolic semantic-search path after Prompts 67-75. It is a decision record, not a
release record. Vector and semantic-profile evidence remain experimental and are not used for release calibration unless
Prompt 45 explicitly approves them.

## Design Note

Candidate sources, Lean semantic lanes, fanout policy, probe planning, scorer calibration, kind policy, family actions,
replacement hints, eval denominators, and release validation each own hidden knowledge that should not leak into the
final decision. The decision consumes only stable source counts, label denominators, stage facts, queue/action facts,
bounded report facts, probe status counts, runtime/RSS status, and blocker states.

The smallest public interface for the decision is:

- search exposes candidate-source ids, source families, fanout/top-k summaries, scorer/review policy ids, visible
  queues, family actions, and leak-safe probe facts;
- eval exposes label resolution, stage denominators, hard-negative survival, precision/recall, and artifact status;
- report exposes `lean-dup.report.v3` bounded ordinary JSON, queue counts, actions, hints, and detail/show boundaries;
- CLI exposes command surfaces and operator-visible status.

Private paths, retrieval keys, posting internals, worker rows, raw Lean expressions, proof scripts, scorer internals,
cache layout, backend storage terms, and vector facts must not move upward into release evidence. The preserved
user-facing capability is a conservative symbolic cleanup audit that can be widened with private/low-priority/
diagnostic visibility, with bounded JSON and stable review actions. The Python-era behavior intentionally discarded is
opaque global heuristics and ad hoc report-side interpretation of retrieval details.

## Design It Twice

Three validation boundaries were considered:

- Trust the prompt-by-prompt checks from Prompts 67-75. This is too scattered; it makes the release reader reconstruct
  readiness from unrelated artifacts.
- Rerun only the default and hard-negative fixtures. This catches regressions in small suites but cannot validate
  KanProofs-scale retrieval, probes, report size, or manual labels.
- Run one symbolic matrix covering real labels, candidate sources, probes, scoring, visibility, family actions, hints,
  performance, report contracts, and boundary tests.

The full symbolic matrix is the chosen design. It is deeper because each crate keeps owning its mechanisms while the
decision consumes one coherent evidence artifact.

## Matrix Summary

All commands below used the release binary and operator-supplied KanProofs/mathlib prerequisites when required. Commands
in this document redact local absolute paths as `<kan-proofs-workspace>` and `<project-pinned-mathlib>`.

| Workload | Status | Artifact | Key result |
| --- | --- | --- | --- |
| `eval --suite default` | pass | `target/symbolic-semantic-validation/eval/default.json` | recall@5 `16/16`, visible precision `8/8`, visible hard negatives `0/3` |
| `eval --suite hard-negatives` | pass | `target/symbolic-semantic-validation/eval/hard-negatives.json` | recall@5 `1/1`, visible hard negatives `0/5` |
| `eval --suite manual-internal` | blocked | `target/symbolic-semantic-validation/eval/manual-internal.json` | prerequisites present, but only `1/6` positives resolve and visible recall is `0/6` |
| `eval --suite manual-mathlib` | blocked | `target/symbolic-semantic-validation/eval/manual-mathlib.json` | prerequisites present, but `0/11` positives resolve as one workspace plus one mathlib declaration |
| `eval --suite production-gate` | blocked | `target/symbolic-semantic-validation/eval/production-gate.json` | manual children blocked; visible hard negatives `0/15`; RSS still above release target |
| `audit --private` on KanProofs | pass as audit run | `target/symbolic-semantic-validation/audit/kanproofs-private.json` | `19` visible groups, bounded report, probes enabled |
| ordinary KanProofs audit | pass as audit run | `target/symbolic-semantic-validation/audit/kanproofs-full.json` | `10` visible groups, probes cached/enabled, bounded JSON |
| focused Prompt 68-75 tests | pass | Cargo test output | search/report/CLI/boundary tests pass |
| Lean build | pass | terminal output | `lake build` completed successfully |
| clippy | pass | terminal output | `cargo clippy --all-targets -- -D warnings` passed |

## Prompt 67 Baseline Comparison

| Baseline | Prompt 67 | Current matrix | Result |
| --- | ---: | ---: | --- |
| Default recall@5 | `16/16` | `16/16` | preserved |
| Default visible precision | `8/8` | `8/8` | preserved |
| Default visible hard negatives | `0/3` | `0/3` | preserved |
| Hard-negative recall@5 | `1/1` | `1/1` | preserved |
| Hard-negative visible precision | `1/8` | `1/8` | preserved |
| Hard-negative visible hard negatives | `0/5` | `0/5` | preserved |
| KanProofs retrieval candidates | `558,109` | `562,437` | comparable, slightly higher after semantic lanes |
| KanProofs review candidate pairs | `13,581` | `13,581` probe-considered | preserved as probe consideration set |
| KanProofs visible groups | `5` | `19` with `--private`, `10` default | improved action surface |
| Pruned fanouts | `39,387` | `39,387` | accounted by policy id `lean-dup.fanout-policy.v1` |
| Heap truncations | `6,522` | `6,522` symbolic, plus source-specific semantic saturation counts | accounted |
| Semantic probes planned | `500` | `230` | more selective |
| Semantic probes verified | `37` | `42` | improved |
| Semantic probes rejected | `294` | `104` | improved |
| Semantic probes unavailable | `169` | `84` | improved but still visible as unavailable status |
| Manual internal current positive | generated/ranked but hidden | still hidden; suite blocked by unresolved labels | blocked |
| Manual mathlib positives | blocked | still blocked | blocked |

## Eval Evidence

### Default Suite

Command:

```sh
env LEAN_DUP_CACHE_DIR=target/symbolic-semantic-validation/cache-default \
  target/release/lean-dup eval --suite default --format json \
  --output target/symbolic-semantic-validation/eval/default.json
```

Facts:

- schema `lean-dup.report.v3`, scorer `lean-dup.symbolic-scorer.v2`, review policy `lean-dup.symbolic-review-policy.v2`;
- recall@1 `7/16`, recall@5 `16/16`, recall@10 `16/16`;
- candidate generation recall `16/16`, ranked recall `16/16`, visible recall `8/16`;
- source recall: symbolic-only `0/16`, semantic-lane-only `0/16`, merged `16/16`;
- visible precision `8/8`; visible hard negatives `0/3`;
- generated source counts: symbolic `169`, Lean semantic `169`;
- candidate loss from fanout/top-k: `0/16` positives and `0/3` hard negatives;
- runtime `4.69s`; max RSS `655,572,992` bytes; report size `5,642` bytes.

### Hard-Negative Suite

Command:

```sh
env LEAN_DUP_CACHE_DIR=target/symbolic-semantic-validation/cache-hard \
  target/release/lean-dup eval --suite hard-negatives --format json \
  --output target/symbolic-semantic-validation/eval/hard-negatives.json
```

Facts:

- recall@5 `1/1`;
- visible precision `1/8`;
- visible hard negatives `0/5`;
- candidate generation recall `1/1`, ranked recall `1/1`, visible recall `1/1`;
- source recall: symbolic-only `0/1`, semantic-lane-only `0/1`, merged `1/1`;
- generated hard negatives `2/5` survive ranking, but `0/5` survive visibility;
- candidate loss from fanout/top-k: `0/1` positives and `0/5` hard negatives;
- runtime `4.48s`; max RSS `655,704,064` bytes; report size `5,633` bytes.

### Manual Internal

Command:

```sh
env LEAN_DUP_CACHE_DIR=target/symbolic-semantic-validation/cache-manual \
  target/release/lean-dup eval --suite manual-internal \
  --workspace <kan-proofs-workspace> --manual-module KanProofs \
  --format json --output target/symbolic-semantic-validation/eval/manual-internal.json
```

Facts:

- status `blocked`, not skipped;
- prerequisites present: workspace, labels, and compiled oleans;
- label resolution `1/6` positives and `1/3` hard negatives;
- one resolved positive is generated and ranked at rank `4`, but hidden by review policy;
- recall@5 `1/6`, visible recall `0/6`;
- visible precision `0/4`, visible hard negatives `0/3`;
- source recall: symbolic-only `0/6`, semantic-lane-only `0/6`, merged `1/6`;
- candidate count `562,437`;
- fanout/top-k positive loss `0/6`; one hard negative is fanout-pruned;
- runtime `54.47s`; max RSS `7,542,210,560` bytes; report size `20,115` bytes.

This suite cannot count as release evidence until current manual positives resolve or are replaced by documented current
declarations.

### Manual Mathlib

Command:

```sh
env LEAN_DUP_CACHE_DIR=target/symbolic-semantic-validation/cache-manual \
  target/release/lean-dup eval --suite manual-mathlib \
  --workspace <kan-proofs-workspace> --manual-module KanProofs \
  --mathlib-workspace <project-pinned-mathlib> \
  --format json --output target/symbolic-semantic-validation/eval/manual-mathlib.json
```

Facts:

- status `blocked`, not skipped;
- prerequisites present: workspace, labels, compiled oleans, source-backed mathlib, and project-pinned mathlib index;
- label resolution `0/11` positives and `3/4` hard negatives;
- most positive blockers are labels that resolve to mathlib/mathlib pairs rather than one workspace declaration plus one
  source-backed mathlib declaration;
- recall@5 `0/11`;
- visible precision `0/4`, visible hard negatives `0/4`;
- source recall: symbolic-only `0/11`, semantic-lane-only `0/11`, merged `0/11`;
- candidate count `588,906`;
- fanout/top-k positive loss `0/11`;
- runtime `554.71s`; max RSS `7,940,112,384` bytes; report size `29,786` bytes.

This suite cannot count as release evidence until its positives are rebuilt around current workspace/mathlib pairs.

### Aggregate Production Gate

Command:

```sh
env LEAN_DUP_CACHE_DIR=target/symbolic-semantic-validation/cache-manual \
  target/release/lean-dup eval --suite production-gate \
  --workspace <kan-proofs-workspace> --manual-module KanProofs \
  --mathlib-workspace <project-pinned-mathlib> \
  --format json --output target/symbolic-semantic-validation/eval/production-gate.json
```

Facts:

- status `blocked`;
- recall@1 `7/34`, recall@5 `18/34`, recall@10 `18/34`;
- visible precision `9/24`, visible hard negatives `0/15`;
- source recall: symbolic-only `0/34`, semantic-lane-only `0/34`, merged `18/34`;
- candidate generation/ranked recall `18/34`, visible recall `9/34`;
- candidate count `1,151,941`;
- generated source counts: symbolic `886,806`, Lean semantic `312,239`;
- candidate loss from fanout/top-k: `0/34` positives; one hard negative fanout-pruned;
- runtime `267.21s`; max RSS `8,717,860,864` bytes; report size `74,888` bytes.

The aggregate gate remains blocked because manual suites are blocked and memory exceeds the previously documented
release target.

## Audit Evidence

### KanProofs Private Audit

Command:

```sh
env LEAN_DUP_CACHE_DIR=target/symbolic-semantic-validation/cache-audit \
  target/release/lean-dup audit --workspace <kan-proofs-workspace> \
  --module KanProofs --private --format json \
  > target/symbolic-semantic-validation/audit/kanproofs-private.json
```

Facts:

- status `ok`, report schema `lean-dup.report.v3`;
- retrieval candidates `562,437`;
- fanout policy `lean-dup.fanout-policy.v1`;
- pruned fanouts `39,387`; heap truncations `6,522`;
- source-specific top-k saturation: binder-role `6,398`, statement-meaning `1,156`, symbolic retrieval `6,522`;
- visible groups `19/19` emitted, limit `500`, not truncated;
- queue counts: cleanup `10`, with private `19`, with low priority `31`, diagnostics `7,379`;
- actions: `inline-private-helper` `1`, `local-alias` `16`, `replace-local-uses` `2`;
- replacement hint caller-impact states: bounded callers `14`, no callers `2`, truncated callers `2`, wrapper-only `1`;
- runtime `73.48s`; max RSS `6,090,489,856` bytes; JSON size `271,343` bytes.

Probe facts:

- enabled with actionable policy;
- candidates considered `13,581`;
- planned `230`, cached `0` on the cold private run, worker pairs `180`;
- verified `42`, rejected `104`, unavailable `84`;
- skipped by policy `6,346`, skipped by budget `7,005`;
- exact-theorem yield `23/30` verified, `7/30` rejected;
- reducible-definition yield `19/150` verified, `97/150` rejected, `34/150` unavailable;
- local-duplicate obligations `0/50` verified, `50/50` unavailable;
- no rejected or unavailable probe state is used by itself as a default actionable finding.

### Ordinary Full Audit With Probes Enabled

Command:

```sh
env LEAN_DUP_CACHE_DIR=target/symbolic-semantic-validation/cache-audit \
  target/release/lean-dup audit --workspace <kan-proofs-workspace> \
  --format json > target/symbolic-semantic-validation/audit/kanproofs-full.json
```

Facts:

- status `ok`, report schema `lean-dup.report.v3`;
- retrieval candidates `562,437`;
- visible groups `10/10` emitted, limit `500`, not truncated;
- queue counts match the private audit: cleanup `10`, with private `19`, with low priority `31`, diagnostics `7,379`;
- actions: `local-alias` `8`, `replace-local-uses` `2`;
- replacement hint caller-impact states: bounded callers `8`, no callers `2`;
- probes used cached evidence: planned `230`, cached `180`, worker pairs `0`, verified `42`, rejected `104`, unavailable
  `84`;
- runtime `11.50s`; max RSS `6,111,920,128` bytes; JSON size `136,596` bytes;
- `jq '.status'` parse time was below the timer resolution (`0.00s` reported).

## Focused Fixtures And Boundary Checks

The following checks passed:

```sh
cargo fmt --check
cargo test -p lean-dup-search
cargo test -p lean-dup-report
cargo test -p lean-dup-cli --test cli
cargo test -p lean-dup-cli --test boundaries
cargo test
cargo clippy --all-targets -- -D warnings
(cd lean && lake build)
```

The focused search tests cover the Prompt 68-75 repair areas: candidate-source facts, Lean semantic lanes, fanout
policy, probe planning, calibrated scoring, kind/low-signal policy, family actions, and replacement hints. Boundary
tests continue to keep the core symbolic crates vector-free.

## Release Decision

Symbolic search is not release-grade yet.

The system has materially improved since Prompt 67:

- fast default and hard-negative denominators are preserved;
- hard-negative visible leakage remains zero;
- candidate-source and semantic-lane counts are now visible to eval;
- fanout/top-k saturation is accounted under a named policy;
- probe planning is more selective and has better yield;
- ordinary reports are bounded and parseable;
- private-helper cleanup is now surfaced through actionable family semantics and replacement hints.

However, the release-grade decision is blocked by evidence that cannot be counted as success:

- manual-internal has current prerequisites but unresolved labels; only `1/6` positives resolve, and the one resolved
  positive remains hidden;
- manual-mathlib has current prerequisites but `0/11` positives satisfy the one-workspace/one-mathlib rule;
- aggregate production-gate remains blocked and above the release memory target;
- manual evidence is therefore incomplete and cannot validate production recall.

No new prompt was added by this decision. The remaining blockers are already owned by existing repair tracks:

- the manual label blocker is the manual corpus/adjudication repair path;
- the eval memory blocker is the eval-memory repair path.

Prompt 60 should not be rerun for a final release candidate until those blockers are closed or explicitly accepted as
no-go evidence. Prompt 61 must not make a final 0.1.0 release decision from this blocked matrix.

## Red Flag Review

- Shallow module: no new module was added in this decision; the artifact consumes existing deep surfaces.
- Pass-through wrapper: no new pass-through API was introduced.
- Temporal decomposition: the matrix is organized by evidence gates, not by implementation chronology.
- Information leakage: this document redacts local paths and avoids retrieval keys, worker rows, raw expressions, proof
  scripts, scorer internals, cache layout, backend vocabulary, and vector facts.
- Special-general mixture: fixture, manual, full-audit, and diagnostic evidence are separated so fixture success is not
  generalized into production success.
- Conjoined methods: quality, probe, performance, report, action, and hint facts are recorded separately.
- Hard-to-describe public API: the public surfaces remain stable source facts, eval denominators, report schema, CLI
  commands, and bounded review actions.
- Implementation details contaminating interface comments: no public interface comments were changed in this
  validation-only session.
