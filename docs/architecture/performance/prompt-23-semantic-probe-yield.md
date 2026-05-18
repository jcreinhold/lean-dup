# Prompt 23 Semantic Probe Availability And Evidence Yield

For the current end-to-end architecture around semantic verification, see
[../06-end-to-end-architecture.md](/Users/jcreinhold/Code/lean-dup/docs/architecture/06-end-to-end-architecture.md).

## Design Note

This pass makes semantic probing own its own evidence availability policy instead of letting audit, ranking, or worker
transport details leak into each other. The hidden knowledge is the declaration universe used for probes, source-backed
module import planning, cheap declaration summaries, unavailable-reason classification, probe cache versioning, and
diagnostic aggregation.

The smallest public interface is still the existing audit flow: ranking consumes `SemanticEvidence` plus JSON-safe
diagnostics. Callers do not receive Lean worker rows, JSONL framing, SQLite cache keys, chunk sizes, source-root mapping
rules, or private/generated filter decisions.

The validated capability preserved is read-only duplicate auditing with cached indexes, source-backed mathlib/external
comparison, proof-grade semantic evidence where Lean can verify it, and hidden noisy findings by default. The
Python-era behavior intentionally discarded is treating broad static overlap or Python cache/layout parity as production
evidence; this pass preserves capabilities, not the old implementation shape.

## Design It Twice

Rejected design: raise heartbeats and add broader retries. That would keep the same shallow boundary: weak candidates
would still be sent to Lean, private declarations would still depend on ambient worker defaults, and heartbeat failures
would remain a normal control path.

Rejected design: report worker rows upward and let ranking decide how to interpret them. That exposes Lean reduction
status strings, missing-declaration messages, and transport details outside the verifier, making report behavior depend
on implementation accidents.

Chosen design: a semantic-evidence planner. The verifier owns import filters, source-backed module selection, cheap
summary classification, cache policy, and typed unavailable diagnostics. It is deeper because audit asks for evidence,
not for modules, private filters, worker requests, and failure-message parsing.

## Root Cause

The full KanProofs proof-grade run before this pass planned 177 semantic probe pairs and produced 70 unavailable
results, including 61 missing declarations and 0 verified results. The failures were not primarily heartbeat failures:
many missing declarations involved `_private.*` names.

The indexer includes private declarations by default, but `probe_batch` did not pass the extraction filters used to
build the index. Lean therefore defaulted `include_private` and `include_generated` to `false` for probe lookup, so
declarations present in the Rust index were absent from the Lean probe environment.

## Changes

`ProbeBatch` now carries `include_private` and `include_generated`. `SemanticVerificationInput` passes the audit
extraction policy into worker probes, and individual planned probes request those filters when either side of the pair
requires private or generated declarations.

Probe module planning now imports only modules referenced by the planned chunk, plus source-backed comparison modules
resolved through provenance. It no longer imports every workspace module for each probe chunk.

The verifier derives private `DeclarationProbeSummary` values from hydrated declarations. These summaries classify
importability and supported declaration kinds before Lean work, select exact theorem, permuted theorem, replacement,
specialization, and reducible-definition obligations, and record unsupported or non-importable cases as typed
unavailable evidence. Ranking still consumes only `SemanticEvidence`.

The probe cache and policy versions were bumped so old missing-declaration cache entries cannot mask the filter fix.
Diagnostics now aggregate unavailable results by stable reason, obligation kind, module, and origin.

## Measurements

Commands:

```bash
cargo run -p lean-dup-cli -- audit --workspace /Users/jcreinhold/Code/kan-proofs \
  --module KanProofs.Mathlib4Backports --compare-mathlib --progress --profile --format json \
  > target/audit-runs/kanproofs-mathlib4backports-prompt23.json

cargo run -p lean-dup-cli -- audit --workspace /Users/jcreinhold/Code/kan-proofs \
  --compare-mathlib --progress --profile --format json \
  > target/audit-runs/kanproofs-full-mathlib-prompt23-rerun.json
```

Before artifact: `target/audit-runs/kanproofs-full-mathlib-proof-grade-probes-warm.json`.

After artifact: `target/audit-runs/kanproofs-full-mathlib-prompt23-rerun.json`.

| Workload | Planned | Worker pairs | Cache hits | Unavailable | Missing | Unsupported | Opaque/unreducible | Internal | Verified | Visible |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Full KanProofs before | 177 | 177 | 0 | 70 | 61 | 0 | 0 | 9 | 0 | 0 |
| Full KanProofs after | 180 | 0 | 180 | 96 | 0 | 84 | 12 | 0 | 0 | 0 |
| `KanProofs.Mathlib4Backports` after | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |

The main availability regression gate is fixed: missing declarations dropped from 61 to 0. The remaining yield blocker
is not availability but evidence quality: the full workload still has no verified proof-grade findings, and the
remaining unavailable results are now typed as unsupported or opaque/unreducible. The default visible queue remains
empty, which is expected for this pass because weak mathlib/static feature matches are still hidden unless backed by
proof-grade source-backed evidence.

The rerun spent most measured profile time in retrieval (`profile.retrieval=38958ms` from stderr), while semantic probes
were served from the new cache version. Prompt 25 remains the right place for broader full-audit throughput work.

## POSD Intervention Mapping

Define special cases out of existence: probe availability now uses the same declaration universe as indexing, so
private/generated declaration mismatches are not a recoverable runtime surprise.

Pull complexity downward: private filters, module selection, source-backed importability, cache keys, and diagnostic
classification live inside semantic verification and worker request construction.

Optimize the abstraction boundary first: audit/ranking did not gain Lean expression details, worker framing fields, or
SQLite layout. The changed interface is internal and evidence-shaped.

Avoid speculative tuning: no heartbeat increase, broad retry policy, FFI work, or parallel Lean workers were added.

## Residual Risks

The prompt improves availability but not proof yield. Full KanProofs still has 0 verified probe results, and the
remaining unsupported/opaque categories show that the next useful work is richer obligation planning and Lean-side
support for the specific theorem/definition shapes that matter.

The module/origin diagnostic maps count both sides of a pair, so totals can exceed unavailable result count. They are
for locating concentrations of unavailable evidence, not for computing a denominator.

## Red Flag Review

- Shallow module: addressed. The verifier owns filter alignment, summaries, cache versions, and diagnostics.
- Pass-through wrapper: addressed. `ProbeBatch` still transports data, but policy is owned by semantic verification.
- Temporal decomposition: addressed. The boundary is evidence-centered rather than split into “collect rows, then
  interpret somewhere else.”
- Information leakage: addressed. Ranking/reporting consume `SemanticEvidence`, not Lean worker rows.
- Special-general mixture: residual risk. Unsupported and opaque cases are typed, but richer proof obligations remain
  future work.
- Conjoined methods: addressed. Planning, execution, and diagnostics are separate private functions under one boundary.
- Hard-to-describe public API: addressed. No new normal user-facing CLI surface was added.
- Implementation details contaminating interface comments: addressed. Comments describe caller obligations and evidence
  contracts, not SQLite or JSONL internals.
