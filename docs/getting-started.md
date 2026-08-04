# Getting started with lean-dup

A walkthrough for Lean 4 developers and mathlib4 contributors who want to find duplicate or near-duplicate declarations
in their own Lake workspace.

## What it does, what it does not

`lean-dup` indexes declarations from your elaborated Lean environment and reports likely duplicates or subsumed
statements: exact theorem matches, safe binder reorderings, equivalent connective shapes, reducible-definition equality,
and replacement candidates against a comparison corpus (mathlib or another cached index).

The normal audit path is read-only. It does not edit your Lean source, call network services, run broad proof search, or
use embeddings.

## Requirements

| Component | Required |
| --- | --- |
| Lean toolchain | `leanprover/lean4:v4.33.0-rc2` (the pinned version; other 4.x versions are untested) |
| Rust toolchain | 1.91+ (the workspace uses `edition = "2024"`) |
| Target project | a Lake workspace whose `lake build` succeeds |
| `.olean` files | the modules you want audited must be compiled |
| Disk | the mathlib index alone is multi-gigabyte; default cache root is `~/.cache/lean-dup` |

If the target project does not build cleanly with Lake first, `lean-dup` will not produce useful output: the tool reads
the elaborated environment, not source text.

## Install

```sh
cargo install lean-dup
```

`cargo install lean-dup` builds the auditor as pure Rust — no Lean toolchain is needed on the build path, because the
parent binary does not link `libleanshared`. The Lean worker that reads your project's `.olean` files is built on your
machine, once per toolchain you audit:

```sh
# Run inside your Lake project (uses its lean-toolchain), or pass --toolchain <id>.
lean-dup install-worker
```

`install-worker` needs the matching elan toolchain (`elan toolchain install <id>`) and a Rust toolchain; it builds the
worker into `<data_local>/lean-dup/workers/<toolchain-id>/` and runs a smoke test. Audits resolve the worker from the
audited project's `lean-toolchain` pin, so a project on a different toolchain just needs its own `install-worker` run.

To work from a checkout instead:

```sh
cargo build --release -p lean-dup
target/release/lean-dup install-worker --source-dir .
```

## Quick start: audit the bundled tiny fixture

The repository ships a small Lake project at `tests/fixtures/tiny/` whose declarations include deliberate duplicates,
permutations, and hard negatives. Running the audit against it takes a second or two:

```sh
target/release/lean-dup audit \
  --workspace tests/fixtures/tiny --module Tiny --no-semantic-probes
```

You will see a summary block followed by ranked groups. Real output, abridged:

```
command: audit
report schema: lean-dup.report.v3
status: ok
selected roots: Tiny
source files: 3
candidates: 260
comparison provenance: no comparison indexes
semantic probes: planned=0 cached=0 worker=0 unavailable=0
review groups: 22
visible groups: 16
visible queue: 16 groups match the active audit visibility options; 6 groups are hidden.
hidden groups: total=6 visibility/noise=6 generated=0 unverified-proof-grade=0 unavailable-probe=0 other=0
probe summary: semantic probes disabled
...
exact-statement-1f4b80d2bb8e: high replace-local-uses exact-statement -> Tiny.same_left
  hint: import=direct callers=1 target_module=Tiny.Basic
  evidence mode: static
...
```

Read the same audit as JSON when you want to script over it. The `explanations` block is the stable, documented part of
the schema:

```sh
target/release/lean-dup audit \
  --workspace tests/fixtures/tiny --module Tiny --no-semantic-probes --format json \
  | jq '.explanations'
```

```json
{
  "visible_queue": {
    "visible": 16,
    "total": 22,
    "summary": "16/22 ranked groups visible",
    "reason": "16 groups match the active audit visibility options; 6 groups are hidden."
  },
  "hidden_groups": {
    "total": 6,
    "visibility_or_noise": 6,
    "generated": 0,
    "unverified_proof_grade": 0,
    "unavailable_probe": 0,
    "other_blockers": 0
  },
  "semantic_probes": {
    "enabled": false,
    "summary": "semantic probes disabled",
    "planned_pairs": 0,
    "verified_results": 0,
    "unavailable_results": 0,
    "cached_hits": 0,
    "worker_pairs": 0,
    "unavailable_by_reason": {}
  },
  "comparison_provenance": {
    "summary": "no comparison indexes",
    "entries": []
  }
}
```

Inspect one group with `show`:

```sh
target/release/lean-dup show \
  --workspace tests/fixtures/tiny --module Tiny \
  --group exact-statement-1f4b80d2bb8e
```

```
group: exact-statement-1f4b80d2bb8e
priority: high
action: replace-local-uses
relation: exact-statement
target: Tiny.same_left
target module: Tiny.Basic
evidence mode: static
members:
  workspace theorem Tiny.same_left   tests/fixtures/tiny/Tiny/Basic.lean:3
  workspace theorem _private.Tiny.Basic.0.Tiny.private_dup_left
                                     tests/fixtures/tiny/Tiny/Basic.lean:85
evidence:
  evidence=conclusion-fingerprint           score=126.688
  evidence=connective-fingerprint           score=193.014
  evidence=safe-permutation-fingerprint     score=252.402
  evidence=statement-fingerprint            score=296.944
signals: conclusion-fingerprint, connective-fingerprint,
         safe-permutation-fingerprint, statement-fingerprint
blockers: none
replacement: Tiny.same_left
import status: direct
callers: 1
  caller tests/fixtures/tiny/Tiny/Basic.lean:7:55
         theorem use_same_left (p q : Prop) : p → q → p := same_left p q
explanation:
  visibility: visible
  why visible or hidden: included by the active audit visibility options and output filters
  evidence mode: static
  static/proof evidence: static indexed evidence; Lean did not verify this group
  semantic evidence: no additional semantic probe evidence is attached
  blockers: none
  replacement/import/callers: target Tiny.same_left; import=direct; callers=1
```

That is the loop: `audit` gives you a list, `show <id>` gives you the evidence and the action.

## Run against your own project

Replace the fixture workspace with your project root:

```sh
target/release/lean-dup audit --workspace /path/to/your/lake/project --progress
```

- `--module Root.Module` scopes the audit to one module subtree. Use this to keep runs short while you are still
  evaluating the tool.
- `--public-only` excludes private declarations from the report.
- `--private` includes otherwise-actionable private helper findings.
- `--low-priority` includes lower-priority structural/API-design findings.
- `--diagnostics` shows broad diagnostic findings, including noise/debug groups.
- `--progress` writes phase events to stderr; stdout stays parseable as JSON when `--format json` is set.

First run on your project is cold: `lean-dup` builds an index of your workspace declarations. Subsequent runs reuse that
index from the cache (default `~/.cache/lean-dup`).

## Reading the output

### Match class

Each group describes one kind of relationship between declarations. The full taxonomy lives in the
[search-quality charter](architecture/search-quality.md); the ones you will see most often:

- *exact theorem duplicate*: same proposition after binder-preserving normalization.
- *binder/permutation duplicate*: same statement under safe binder reordering or premise permutation.
- *reducible-definition duplicate*: reducible definitions that compute to the same value.
- *replacement candidate*: your declaration can likely be replaced by an imported or mathlib one.
- *hard negative*: a tracked non-match; the tool should never surface this as actionable.

### Evidence mode

Each group declares how strong the evidence behind it is. See
[external comparison provenance](architecture/external-comparison-provenance.md) for the policy.

- `proof-grade`: Lean verified the relationship, or the comparison index is source-backed and importable in the current
  Lean environment.
- `source-backed-not-importable`: the index has source provenance, but its execution root differs from your audit; no
  Lean probe was possible.
- `static`: the group rests on indexed/static fingerprint evidence. Useful as a suggestion; not a proof.

The fixture quick-start above shows `evidence mode: static` because the run was local and used `--no-semantic-probes`.

### Visible vs hidden

The default review queue prefers high precision. Groups that are noisy, unverified, private-only, or lower priority are
hidden by default and counted in the `hidden_groups` block of the JSON. Use `--private`, `--low-priority`, or
`--diagnostics` to widen the queue by the dimension you want. An empty visible queue is always explained by
`visible_queue.reason`.

### Replacement hint

When the right side of a group is importable, the report attaches:

- `target`: the declaration you would call instead.
- `import status`: `direct` if your module already imports the target's module; otherwise the module to add.
- `callers`: how many local references would need to change, with file:line for each.

You can act on a hint by hand, or save the JSON for later tooling. `lean-dup` itself never edits your Lean source.

## Current limitations

The release-readiness gates `G1 regression_quality` and `G2 precision_control` documented in
[architecture/production-readiness.md](architecture/production-readiness.md) are both open. Today:

- *Intra-workspace audits* (no `--compare-mathlib`) are the most useful invocation. The fixture suite passes its quality
  bar and the same path scales to real Lake projects.
- *`--compare-mathlib` is implemented but unvalidated.* The aggregate quality and the manual-corpus mathlib gate suites
  do not currently pass; recall against real mathlib corpora has not been demonstrated. Do not expect strong recall yet.
- *Semantic probes* are bounded and recoverable, but their proof-grade yield on full mathlib runs is still partial.
  `--no-semantic-probes` is supported and is the fastest way to start.
- *No `--version` flag yet*; release diagnostics are still being shaped.

The CLI is read-only with respect to your source either way, so trying it costs you only time.

## For mathlib4 contributors

Two practical paths today.

**Find duplicates inside a branch before you push.** Point `--workspace` at your local mathlib checkout and use
`--module Mathlib.Some.Subnamespace` to keep the run scoped. You get intra-workspace candidates and a queue you can work
through with `show`. This is the workflow the tool serves best right now.

**Compare your branch against mathlib's pinned dependency in another project.** `--compare-mathlib` builds (or reuses) a
project-pinned mathlib index from `.lake/packages/mathlib`. Recall is currently low (see *Current limitations* above),
so treat results as a starting point, not a guarantee.

The first run that touches mathlib is multi-minute: the worker imports a large environment and the index is several
hundred MB. The shared cache under `~/.cache/lean-dup` makes subsequent runs fast.

## Troubleshooting

- **`target/release/lean-dup: No such file`**: `cargo build --release -p lean-dup` did not finish, or you are running
  from a directory other than the repo root.
- **"worker not installed" / "run lean-dup install-worker"**: no worker is built for the audited project's toolchain.
  Run `lean-dup install-worker` (it prints the exact `--toolchain <id>` if the pin differs from the current
  directory's).
- **Worker fails to start, or schema mismatch in stderr**: the installed worker is stale (e.g. after a toolchain bump).
  Rebuild it with `lean-dup install-worker --force`.
- **"missing olean" or import failures**: the modules you asked for are not compiled. Run `lake build` in the audited
  workspace first.
- **First mathlib run hangs for many minutes**: expected. The worker is importing mathlib and building the index.
  Subsequent runs reuse the cache.
- **`doctor` shows many `status=unchecked` cache entries with large `bytes`**: these are indexes for other workspaces
  sharing the cache root. The hidden `cache-cleanup` command is dry-run by default; pass `--execute` to remove
  unprotected stale entries.
- **An audit reports `visible groups: 0`**: the `visible_queue.reason` field always names why. Common causes: no ranked
  groups (no candidates passed the filters), all groups hidden by the audit visibility options, or all proof-grade
  candidates remained unverified.

For deeper diagnosis run `doctor --format json` and inspect the cache + provenance state directly.

## Reporting issues and contributing

For bugs and feature requests, open an issue on the project's GitHub repository.

For contributors, start with the [architecture charter](architecture/overview.md) and the
[end-to-end architecture](architecture/end-to-end-architecture.md).

License: Apache-2.0 OR MIT. See `LICENSE-APACHE` and `LICENSE-MIT`.
