# Report Contract

What every audit report—text, JSON, and `show`—must explain. Renderers consume typed explanation facts and never
invent the policy themselves.

For the pipeline that builds these facts, see [end-to-end-architecture.md](end-to-end-architecture.md).
This document defines `G7 report_contract` in [production-readiness.md](production-readiness.md).

## Schema

Audit JSON is summary-first. The stable surface is `report_schema_version`, grouped command metadata, compact review
diagnostics, bounded `visible_groups`, and the `explanations` object. Full forensic group detail belongs in targeted
`show` output, not in ordinary audit JSON.

```jsonc
{
  "report_schema_version": "lean-dup.report.v3",
  "workspace": {
    "requested_workspace": "tests/fixtures/tiny",
    "lake_root": "tests/fixtures/tiny",
    "selected_roots": ["Tiny"],
    "source_count": 1
  },
  "cache": {
    "root": "target/lean-dup/cache",
    "fingerprint": "..."
  },
  "options": {
    "include_private": true,
    "compare_indexes": [],
    "compare_mathlib": false,
    "include_generated": false,
    "show_noise": false,
    "review_profile": "mathlib"
  },
  "review": {
    "group_count": 12,
    "suppressed_count": 4,
    "candidate_pairs": 88,
    "emitted_groups": 12,
    "diagnostics": {
      "candidate_pairs": 88,
      "emitted_groups": 12,
      "suppressed_groups": 4
    }
  },
  "visible_groups": [],
  "visible_group_count": 12,
  "visible_groups_emitted": 0,
  "visible_group_limit": 500,
  "visible_groups_truncated": false,
  "explanations": {
    "visible_queue": {
      "count": 0,                       // visible groups
      "total_ranked": 12,               // all ranked, before visibility filtering
      "summary": "no visible groups",
      "reason": "all-unverified-proof-grade"
      // reason ∈ {none-ranked, all-noise-or-profile, all-unverified-proof-grade,
      //          probes-unavailable, mixed-blockers}
    },
    "hidden_groups": {
      // exclusive counts; each group classified once using this precedence:
      // generated-decl > unverified-proof-grade > unavailable-probe >
      // profile-noise > other-blockers
      "generated_decl": 3,
      "unverified_proof_grade": 6,
      "unavailable_probe": 2,
      "profile_noise": 1,
      "other_blockers": 0
    },
    "semantic_probes": {
      "ran": true,
      "planned": 177,
      "verified": 0,
      "unavailable": 70,
      "cache_hits": 0,
      "worker_pairs": 177,
      "unavailable_reasons": { "missing-decl": 61, "opaque-or-unreducible": 9 },
      "summary": "177 planned, 0 verified, 70 unavailable"
    },
    "comparison_provenance": {
      "summary": "mathlib: proof-grade (312 611 decls)",
      "entries": [
        { "label": "mathlib", "origin": "mathlib", "evidence_mode": "proof-grade",
          "declaration_count": 312611, "reason": "source-backed, importable" }
      ]
    }
  }
}
```

Progress and profile output remain stderr-only. JSON stdout must parse as one JSON value even with `--progress` and
`--profile`.

## Text contract

Default text audit output must include, in order:

1. report schema version;
2. comparison provenance status;
3. semantic-probe planned / cache / worker / unavailable counts;
4. visible-group count;
5. visible-queue reason;
6. exclusive hidden-group counts;
7. probe summary;
8. the first visible groups, if any.

When `visible groups: 0`, the text report must explain why without requiring `jq`.

## `show` contract

`show` explains one ranked group. It must include:

- evidence mode: `static`, `source-backed-not-importable`, or `proof-grade`;
- semantic evidence status or the reason no semantic evidence is attached;
- blockers, or `none`;
- replacement target, import status, caller count, and replacement blockers or notes when available;
- whether the group is visible or hidden under the active filter, and why.

`show` does not expose worker records, SQLite rows, retrieval keys, or source-scan implementation details.

## Why a private contract

A renderer-local approach—conditionals inside the text renderer—spreads hidden-count policy across text, JSON, and
`show`. JSON consumers would still have to reconstruct why the visible queue is empty.

Audit and `show` instead build typed explanation facts before rendering. Text and JSON format those facts without
knowing hidden-count precedence, probe diagnostics, provenance policy, SQLite layout, or worker transport. The
contract changes in one place; renderers stay narrow.

## How to regenerate the evidence

```sh
cargo run -p lean-dup-cli -- audit --workspace tests/fixtures/tiny --module Tiny \
  --no-semantic-probes --format json > target/report-contract/fixture-audit.json

cargo run -p lean-dup-cli -- audit --workspace tests/fixtures/tiny --module Tiny \
  --no-semantic-probes > target/report-contract/fixture-audit.txt

target/release/lean-dup --progress --profile audit \
  --workspace <workspace> --module <Root.Module> \
  --compare-mathlib --format json > target/report-contract/full-mathlib.json
```

Verification:

```sh
cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check
(cd lean && lake build)
```
