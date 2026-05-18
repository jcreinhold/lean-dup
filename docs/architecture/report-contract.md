# Report Contract

This document defines the production report contract for `lean-dup`. It closes `G7 report_contract` when the contract
is backed by fixture and real-workload artifacts.

For the current end-to-end pipeline that builds these explanation facts, see
[06-end-to-end-architecture.md](/Users/jcreinhold/Code/lean-dup/docs/architecture/06-end-to-end-architecture.md).

## Design Note

The report contract owns the hidden knowledge for user-facing explanations: visible-queue summaries, hidden-group
reason precedence, semantic-probe summaries, comparison provenance summaries, `show` explanations, and JSON schema
versioning.

Its smallest public interface is additive report fields plus text explanations:

- `report_schema_version`;
- `explanations.visible_queue`;
- `explanations.hidden_groups`;
- `explanations.semantic_probes`;
- `explanations.comparison_provenance`;
- group-level explanations in `show`.

These decisions must not leak upward or sideways:

- SQLite table names, row ids, cache paths, or cache-key ingredients;
- worker JSONL framing, probe chunking, heartbeat recovery, or Lean expression traversal;
- retrieval key shapes, ranking heap policy, source scanning policy, or replacement-hint internals.

The preserved user-facing capability is read-only local duplicate auditing with cached indexes, mathlib or external
comparison, semantic evidence, text and JSON reports, `show`, and baseline review.

Python-era behavior intentionally discarded: report meaning is not inferred from Python cache layout, Python string
heuristics, or manual `jq` inspection. The Rust report emits typed explanation facts directly.

## Design It Twice

**Rejected: renderer-local explanation strings.** Adding conditionals directly to the text renderer would be quick, but
it would spread hidden-count policy across text, JSON, and `show`. JSON consumers would still need to reconstruct why
the visible queue is empty.

**Chosen: private report-contract facts.** Audit and `show` build typed explanation facts before rendering. Text and
JSON output format those facts without knowing hidden-count precedence, probe diagnostics, provenance policy, SQLite
layout, or worker transport. This is deeper because callers get stable report meaning through one narrow contract.

## Stable JSON Contract

Audit JSON is additive. Existing top-level fields remain available, but production consumers should prefer
`report_schema_version` and `explanations` for user-facing meaning.

`report_schema_version` is currently:

```text
lean-dup.report.v1
```

The `explanations` object contains:

- `visible_queue`: visible count, total ranked groups, a compact summary, and a direct reason. If the visible queue is
  empty, this reason must say whether no groups were ranked, all groups were filtered as noise/profile results, all
  groups lacked verified proof-grade evidence, probes were unavailable, or mixed blockers remain.
- `hidden_groups`: exclusive hidden counts. A hidden group is counted once using this precedence: generated declaration,
  unverified proof-grade evidence, unavailable semantic probe, profile/noise filtering, then other blockers.
- `semantic_probes`: whether probes ran, planned pairs, verified results, unavailable results, cache hits, worker pairs,
  stable unavailable reason counts, and a short summary.
- `comparison_provenance`: one compact summary plus entries for label, origin, evidence mode, declaration count, and
  reason. It intentionally omits index paths and source roots from the explanation layer; legacy provenance fields
  still carry detailed diagnostics.

Progress and profile output remain stderr-only. JSON stdout must parse as one JSON value even when `--progress` and
`--profile` are enabled.

## Text UX Contract

Default text audit output must include:

- report schema version;
- comparison provenance status;
- semantic-probe planned/cache/worker/unavailable counts;
- visible-group count;
- visible-queue reason;
- exclusive hidden-group counts;
- probe summary;
- the first visible groups, if any.

When `visible groups: 0`, the text report must explain why without requiring `jq`.

## `show` Contract

`show` explains one ranked group. It must include:

- static, source-backed-not-importable, or proof-grade evidence mode;
- semantic evidence status or the reason no semantic evidence is attached;
- blockers or `none`;
- replacement target, import status, caller count, and replacement blockers or notes when available;
- whether the group is visible or hidden under the active filter and why.

`show` does not expose worker records, SQLite rows, retrieval keys, or source-scan implementation details.

## Evidence Artifacts

Prompt 26 evidence should live under `target/report-contract/`:

```sh
cargo run -p lean-dup-cli -- audit --workspace tests/fixtures/tiny --module Tiny \
  --no-semantic-probes --format json \
  > target/report-contract/fixture-audit.json

cargo run -p lean-dup-cli -- audit --workspace tests/fixtures/tiny --module Tiny \
  --no-semantic-probes \
  > target/report-contract/fixture-audit.txt

target/release/lean-dup --progress --profile audit \
  --workspace /Users/jcreinhold/Code/kan-proofs --module KanProofs \
  --compare-mathlib --format json \
  > target/report-contract/kanproofs-full-mathlib.json
```

Required verification:

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cd /Users/jcreinhold/Code/lean-dup/lean && lake build
```

## Red Flag Review

- **Shallow module:** mitigated. The report contract computes explanation facts; renderers do not duplicate the policy.
- **Pass-through wrapper:** mitigated. The new boundary transforms ranking/probe/provenance diagnostics into stable
  report meaning rather than forwarding raw fields.
- **Temporal decomposition:** mitigated. The boundary is organized around report meaning, not around the order audit
  phases execute.
- **Information leakage:** mitigated. SQLite, worker transport, retrieval keys, source scanning, and cache layout stay
  out of the contract.
- **Special-general mixture:** mitigated. KanProofs is an evidence workload, but the report contract is general across
  fixture, local, mathlib, and external-index audits.
- **Conjoined methods:** mitigated. Audit explanation and group explanation are separate facts with separate callers.
- **Hard-to-describe public API:** mitigated. The public report shape is a small additive `explanations` object and one
  schema version.
- **Implementation details contaminating interface comments:** mitigated. Comments and this document describe caller
  guarantees, not storage layout, worker framing, or temporary migration machinery.
