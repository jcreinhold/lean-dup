# lean-dup

A read-only duplication auditor for Lean 4 Lake workspaces. It indexes declarations from the elaborated Lean environment
and reports likely duplicate or subsumed statements. Local and deterministic on the normal audit path: no network
services, no embeddings, no proof-term analysis. Hidden developer commands may explicitly prepare local experiment
assets.

## Requirements

Lean toolchain `leanprover/lean4:v4.34.0-rc1` (other 4.x versions are untested); Rust 1.91+ (`edition = "2024"`); a Lake
workspace whose `lake build` already succeeds, with `.olean` files present for the modules to be audited.

## Install

```sh
cargo install lean-dup
```

`cargo install lean-dup` ships the auditor as **pure Rust** — the parent binary does not link `libleanshared`, so no
Lean toolchain is needed on the build path. The Lean worker that reads your project's `.olean` files is built on your
machine, once per toolchain you audit:

```sh
# Run inside your Lake project (uses its lean-toolchain), or pass --toolchain <id>.
lean-dup install-worker
lean-dup --version
```

`install-worker` builds the toolchain-specific worker — the native `lean-dup-worker` Lean executable — into
`<data_local>/lean-dup/workers/<toolchain-id>/` with that toolchain's own `lake`, runs a smoke test that spawns it and
answers a `version` command over the JSONL transport, and records a provenance sidecar. Audits resolve the worker from
the audited project's `lean-toolchain` pin; if one is not installed, `lean-dup` prints the exact `install-worker
--toolchain <id>` command to run. Run `lean-dup doctor` to confirm the worker is reachable. Building a worker requires
only the matching elan toolchain (`elan toolchain install <id>`) — no Rust toolchain.

Optional tools such as `lean-dup-vector` are external extensions and are not required for the core audit workflow.

### From a checkout

```sh
cargo build --release -p lean-dup
target/release/lean-dup install-worker --source-dir .
```

`--source-dir .` builds the worker from the checkout's `lean/` project instead of the packaged Lean source.

For a walkthrough with sample output, the full audit/show loop, and what each field means, see
[docs/getting-started.md](docs/getting-started.md).

## Audit a workspace

```sh
target/release/lean-dup audit --workspace /path/to/lake/workspace --compare-mathlib --progress
```

`audit` walks the inferred Lake workspace roots. Private theorem-like declarations are included by default when Lean
exposes them through compiled modules; `--module Root.Module` scopes the audit to one root and its descendants.

| Flag | Effect |
| --- | --- |
| `--public-only` | exclude private declarations |
| `--private` | show otherwise-actionable private helper findings |
| `--low-priority` | include lower-priority structural/API-design findings |
| `--diagnostics` | show broad diagnostic findings, including noise/debug groups |
| `--compare-mathlib` | compare against the project's pinned mathlib index |
| `--compare-index LABEL` | compare against a named cached external index |
| `--profile` | include extraction and classification timings |
| `--max-heartbeats N` | per-declaration Lean elaboration heartbeat budget (worker default 200000; `0` = unlimited) |
| `--format json` | machine-readable output (stdout stays parseable with `--progress`/`--profile`) |

A declaration whose elaboration exceeds the heartbeat budget is **skipped, not fatal** — the audit completes over the
rest of the corpus instead of aborting on one pathological declaration. The skip count is surfaced
(`workspace.declarations_skipped_by_budget` in `--format json`, plus a stderr progress line); raise `--max-heartbeats`
(or set `0` for unlimited) to trade runtime for including those declarations. Because the budget changes which
declarations are indexed, it participates in the cache key.

For local development, swap `target/release/lean-dup` for `cargo run -p lean-dup --`. Other commands: `doctor`
(workspace, worker, Lake, cache health), `show --group <id>` (one ranked group), `diff` (saved baselines), `eval`
(quality suites).

## Lint focused declarations

```sh
lean-dup lint --workspace /path/to/lake/workspace --module MyLib --changed-since origin/main
```

`lint` turns only Lean-verified exact-statement, safe-premise-permutation, and connective-equivalence matches into
source-located warnings. The full workspace remains the comparison corpus, but `--changed-since REV`, repeatable
`--file PATH`, and repeatable `--declaration NAME` selectors restrict which declarations can be reported. Add
`--compare-mathlib` only when the pinned mathlib index is part of the intended audit.

Warnings are advisory and exit `0`; decide which declaration owns the API after checking types, generality, callers, and
imports. Missing declarations, exhausted budgets, unavailable semantic probes, and other incomplete measurements exit
`2`. Operational or CLI failures exit `1`. Use `--format json` for deterministic structured output. Static fingerprint
collisions never become lint warnings. Opaque, unsupported, or deliberately size-bounded definition comparisons remain
silent; they are outside the lint contract. Missing declarations, timeouts, and internal probe failures still make the
measurement incomplete.

Before a longer run, ask the binary what it is and whether the workspace is auditable:

```sh
lean-dup --version
lean-dup doctor --workspace /path/to/lake/workspace --format json
```

## Caches and replacement hints

Project and mathlib indexes are cached under the resolved `lean-dup` cache root; shared project-pinned mathlib indexes
default to `~/.cache/lean-dup` (`LEAN_DUP_CACHE_DIR` overrides).

For confirmed external matches, text and JSON reports include read-only replacement hints: the target declaration, the
specific import line, direct-import status, and bounded local source references to replace or preserve behind a
transitional alias.

## Current status

Intra-workspace audits and source-located advisory linting are usable today. `--compare-mathlib` runs but the
release-quality gates `G1 regression_quality` and `G2 precision_control` are open: recall against real mathlib corpora
has not been demonstrated yet. See
[docs/architecture/production-readiness.md](docs/architecture/production-readiness.md) for the gate table. The CLI is
read-only with respect to your Lean source, so trying it costs only time.

## Architecture

Start with the [end-to-end architecture](docs/architecture/end-to-end-architecture.md) for the as-built pipeline. Then:

- [Architecture charter](docs/architecture/overview.md)
- [Worker protocol](docs/architecture/worker-protocol.md)
- [Crate factoring](docs/architecture/crate-factoring.md)
- [Production readiness](docs/architecture/production-readiness.md)
- [Search quality](docs/architecture/search-quality.md)

## License

Apache-2.0 OR MIT, at your option. See `LICENSE-APACHE` and `LICENSE-MIT`.
