# Report Contract

This document defines `G7 report_contract` in [production-readiness.md](production-readiness.md). It covers ordinary
audit JSON/text, source-located lint JSON/text, ordinary eval JSON/text, and targeted `show` detail for the symbolic
auditor.

## Design Note

Report projection owns schema stability, truncation, path redaction, bounded group summaries, compact diagnostics, and
renderer-specific formatting. Search owns review policy, visible queue membership, group ids, evidence facts, and queue
counts. Eval owns labels, denominators, and quality metrics. CLI owns stdout/stderr routing and command-line
configuration.

The smallest public report interface is:

- schema id `lean-dup.report.v3`;
- grouped command/config metadata;
- redacted workspace/cache path references;
- bounded family-level `visible_groups` plus total/emitted/limit/truncated counts;
- compact `review` diagnostics without full `review.groups`;
- stable eval denominators and timing facts;
- targeted `show` detail for one selected family or pair.

Rendering structure, truncation mechanics, raw worker/probe records, private paths, cache layout, storage tables, and
forensic group assembly do not leak upward into CLI callers or sideways into eval/search. The preserved user-facing
capability is parseable, bounded symbolic audit/eval output with enough evidence to review the queue. The Python-era
behavior intentionally discarded is dumping every candidate/group row into ordinary JSON and expecting downstream tools
to filter it.

## Design It Twice

Three designs were considered:

- Keep full ordinary JSON and rely on `jq` or custom tools to filter it. Rejected because report consumers would still
  pay the cost of forensic material and would need hidden search-policy knowledge.
- Split each report into specialized schemas for summary, queue, diagnostics, provenance, and detail. Rejected because
  it multiplies public contracts while preserving most caller coordination costs.
- Keep one bounded ordinary schema plus explicit targeted detail. Chosen because report owns projection policy once,
  ordinary output stays small and stable, and full evidence is still available when a user asks for one group.

The chosen boundary is deeper: search and eval continue to own their facts, report owns only projection, and callers do
not learn scorer weights, visibility reconstruction, cache layout, or worker transport details.

## Ordinary Audit JSON

Audit JSON is summary-first. The stable surface is `report_schema_version = "lean-dup.report.v3"`, grouped command
metadata, compact review diagnostics, bounded family-level `visible_groups`, and the `explanations` object. A one-pair
finding is represented as a one-pair family. When several pairs share one coherent cleanup action and target, search may
surface them as one family with bounded pair summaries. Full forensic pair detail belongs in targeted `show` output, not
in ordinary audit JSON.

Representative shape:

```jsonc
{
  "command": "audit",
  "report_schema_version": "lean-dup.report.v3",
  "status": "ok",
  "workspace": {
    "requested_workspace": {
      "kind": "workspace-root",
      "fingerprint": "sha256:07049e02f8629df73d07d007"
    },
    "lake_root": {
      "kind": "workspace-root",
      "fingerprint": "sha256:07049e02f8629df73d07d007"
    },
    "selected_roots": ["Tiny"],
    "source_count": 3,
    "declarations_skipped_by_budget": 0
  },
  "cache": {
    "root": {
      "kind": "cache-root",
      "fingerprint": "sha256:3466c039192046f7fbbbbe95"
    },
    "fingerprint": "rust-cli-cache.v1:110a3e453c253620570cb59c6e2693d8aea12480e7c24619898beab4d0c3abc8"
  },
  "review": {
    "group_count": 22,
    "suppressed_count": 68,
    "diagnostics": {
      "candidate_pairs": 44,
      "emitted_groups": 22,
      "suppressed_groups": 68
    },
    "candidate_pairs": 44,
    "emitted_groups": 22
  },
  "visible_group_count": 5,
  "visible_groups_emitted": 5,
  "visible_group_limit": 500,
  "visible_groups_truncated": false,
  "visible_groups": [],
  "retrieval": {
    "candidate_count": 260,
    "hydrated_external_count": 0,
    "pruned_feature_fanouts": 0,
    "heap_truncations": 0
  },
  "explanations": {
    "visible_queue": {
      "visible": 5,
      "emitted": 5,
      "limit": 500,
      "truncated": false,
      "total": 22,
      "summary": "5 groups match the active audit visibility options; 17 groups are hidden.",
      "reason": "Some ranked groups are hidden by the active audit visibility options or blockers."
    }
  }
}
```

`review.groups` is intentionally absent. Ordinary JSON must not expose raw worker rows, Lean proof obligations, private
paths, cache layout, storage vocabulary, backend names, or unbounded group arrays. Source spans and caller locations use
redacted path references:

```json
{ "kind": "workspace-root", "fingerprint": "sha256:3b3ee301c874bc75e6203513" }
```

## Ordinary Eval JSON

Ordinary eval JSON uses the same report schema id and keeps raw denominators rather than rendered conclusions:

- `suite`;
- `scorer_version`;
- `review_policy_version`;
- recall denominators;
- shown-queue precision;
- hard-negative hits;
- visible group counts;
- stage metrics;
- timings and peak memory when available.

Eval output does not duplicate audit group detail. It may report artifact paths chosen by the operator, but release
examples and golden artifacts use relative paths so private local paths are not written into checked evidence.

## Text Contract

Default text audit output includes, in order:

1. report schema version;
2. redacted workspace/cache identity;
3. comparison provenance status;
4. semantic-probe planned / cache / worker / unavailable counts;
5. candidate and review-group counts;
6. visible-group count and emitted/limit facts;
7. visible-queue explanation;
8. exclusive hidden-group counts;
9. queue counts and suppressed groups;
10. the first visible groups, if any.

When `visible_group_count = 0`, the text report explains why without requiring `jq`.

## `lint` Contract

`lint` is a high-precision projection of the audit workflow, not a second duplicate engine. The full selected workspace
index remains its local comparison corpus, while optional source ranges and declaration names restrict candidate anchors
before retrieval. Only proof-grade `exact-statement`, `permuted-statement`, and `connective-equivalent` groups become
findings; static evidence never does.

Text findings use `file:line:column` diagnostics and include the focused declaration, duplicate declaration, relation,
semantic evidence, recommended action, and a targeted `show` command. JSON uses the ordinary report schema id and adds
`status`, selected roots, focus metadata, a deterministic `findings` array, and `incomplete_reasons`. Source paths are
workspace-relative when possible.

Findings have severity `warning` and exit `0`. `status = incomplete` exits `2` when requested declarations are missing,
workspace or probe budgets skip evidence, semantic results are unavailable, or the report queue is truncated. CLI,
workspace, Git, and worker failures exit `1`. This separation lets commit-time automation remain advisory without
silently accepting an unmeasured workload.

An opaque, unsupported, or definition-size-guarded pair is outside the lint's semantic domain and remains silent; it
does not make the measurement incomplete. A missing declaration, timeout, or internal probe failure does.

## `show` Contract

`show` explains one review family. It accepts a family id, ranked pair-group id, or pair id. It includes:

- redacted workspace/cache identity;
- family id, pair count, action, relation, and target;
- members with redacted source references;
- bounded evidence facts and signals;
- pair evidence summaries for a selected family;
- evidence mode: `static`, `source-backed-not-importable`, or `proof-grade`;
- typed semantic evidence status or the reason no semantic evidence is attached;
- blockers, or `none`;
- replacement target, import status, caller-impact state, caller count, truncation status, and replacement notes;
- whether the group is visible or hidden under the active filter, and why.

`show` may include full evidence for that selected family. It still does not expose worker records, raw proof
obligations, SQLite rows, retrieval keys, absolute private paths, cache layout, or source-scan implementation details.

## Golden Artifacts

Prompt 58 generated representative artifacts under `target/report-contract/`. They are not committed as release
artifacts, but the commands and summaries define the contract to regenerate.

| Artifact | Purpose | Key facts |
| --- | --- | --- |
| `ordinary-audit.json` | bounded ordinary audit JSON | schema `lean-dup.report.v3`, status `ok`, `visible_group_count = 5`, `visible_groups_emitted = 5`, limit `500`, not truncated |
| `ordinary-audit.txt` | text audit contract | redacted workspace/cache labels and visible-queue explanation |
| `ordinary-eval.json` | ordinary eval denominators | schema `lean-dup.report.v3`, status `ok`, suite `default`, queue precision `8/8`, hard-negative hits `0/3` |
| `empty-audit.json` | empty queue explanation | status `ok`, `visible_group_count = 0`, reason states no candidate groups were ranked |
| `truncated-audit.json` | bounded large queue | status `ok`, `visible_group_count = 417255`, `visible_groups_emitted = 500`, `visible_groups_truncated = true` |
| `show-detail.txt` | targeted detail path | one selected group with proof-grade evidence and redacted source references |

The representative parse checks completed quickly:

- `jq '.status' target/report-contract/ordinary-audit.json`: `0.00s`;
- `jq '.status' target/report-contract/truncated-audit.json`: `0.02s`.

Leak check command:

```sh
rg -n '/Users|target/report-contract/cache|index\.sqlite|latest\.json|\bpostings\b|pruned_postings|worker row|worker_row|FeatureMatch|IndexQuery|proof_obligation|raw_obligation|backend|tokenizer|lancedb|lance|sqlite|cache layout|source snippet' \
  target/report-contract/*.json target/report-contract/*.txt
```

Expected result: no matches.

## Regeneration Commands

```sh
rm -rf target/report-contract
mkdir -p target/report-contract/cache

env LEAN_DUP_CACHE_DIR=target/report-contract/cache \
  cargo run -q -p lean-dup-cli -- audit \
  --workspace tests/fixtures/tiny --module Tiny \
  --no-semantic-probes --format json \
  > target/report-contract/ordinary-audit.json

env LEAN_DUP_CACHE_DIR=target/report-contract/cache \
  cargo run -q -p lean-dup-cli -- audit \
  --workspace tests/fixtures/tiny --module Tiny \
  --no-semantic-probes \
  > target/report-contract/ordinary-audit.txt

cargo run -q -p lean-dup-cli -- eval \
  --suite default --format json \
  --output target/report-contract/ordinary-eval.json \
  > target/report-contract/ordinary-eval.stdout

gid=$(jq -r '.visible_groups[0].id' target/report-contract/ordinary-audit.json)
env LEAN_DUP_CACHE_DIR=target/report-contract/cache \
  cargo run -q -p lean-dup-cli -- show \
  --workspace tests/fixtures/tiny --module Tiny --group "$gid" \
  > target/report-contract/show-detail.txt

env LEAN_DUP_CACHE_DIR=target/report-contract/cache \
  cargo run -q -p lean-dup-cli -- audit \
  --workspace tests/fixtures/source-backed --module Tiny \
  --no-semantic-probes --format json \
  > target/report-contract/empty-audit.json

env LEAN_DUP_CACHE_DIR=target/report-contract/cache LEAN_NUM_THREADS=2 \
  cargo run -q -p lean-dup-cli -- audit \
  --workspace <proofs-workspace> --module Proofs \
  --no-semantic-probes --diagnostics --format json \
  > target/report-contract/truncated-audit.json
```

Verification:

```sh
cargo fmt --check
cargo test -p lean-dup-report
cargo test -p lean-dup-cli --test cli
cargo test -p lean-dup-cli --test boundaries
cargo test
cargo clippy --all-targets -- -D warnings
(cd lean && lake build)
```
