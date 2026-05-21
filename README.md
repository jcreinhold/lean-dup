# lean-dup

A read-only duplication auditor for Lean 4 Lake workspaces. It indexes declarations from the
elaborated Lean environment and reports likely duplicate or subsumed statements. Local and
deterministic on the normal audit path: no network services, no embeddings, no proof-term
analysis. Hidden developer commands may explicitly prepare local experiment assets.

## Requirements

Lean toolchain `leanprover/lean4:v4.30.0-rc2` (other 4.x versions are untested); Rust 1.85+
(`edition = "2024"`); a Lake workspace whose `lake build` already succeeds, with `.olean` files
present for the modules to be audited.

## Build

```sh
cargo build --release -p lean-dup-cli
cd lean && lake build
```

## Install

From a checkout:

```sh
cargo install --path crates/cli
lean-dup --version
```

The installed `lean-dup` binary is the symbolic auditor. Optional tools such as
`lean-dup-vector` are external extensions and are not required for the core audit workflow.

For a walkthrough with sample output, the full audit/show loop, and what each field means, see
[docs/getting-started.md](docs/getting-started.md).

## Audit a workspace

```sh
target/release/lean-dup audit --workspace /path/to/lake/workspace --compare-mathlib --progress
```

`audit` walks the inferred Lake workspace roots. Private theorem-like declarations are included
by default when Lean exposes them through compiled modules; `--module Root.Module` scopes the
audit to one root and its descendants.

| Flag                    | Effect                                                                        |
| ----------------------- | ----------------------------------------------------------------------------- |
| `--public-only`         | exclude private declarations                                                  |
| `--compare-mathlib`     | compare against the project's pinned mathlib index                            |
| `--compare-index LABEL` | compare against a named cached external index                                 |
| `--profile`             | include extraction and classification timings                                 |
| `--format json`         | machine-readable output (stdout stays parseable with `--progress`/`--profile`) |

For local development, swap `target/release/lean-dup` for `cargo run -p lean-dup-cli --`. Other
commands: `doctor` (workspace, worker, Lake, cache health), `show --group <id>` (one ranked
group), `diff` (saved baselines), `eval` (quality suites).

Before a longer run, ask the binary what it is and whether the workspace is auditable:

```sh
lean-dup --version
lean-dup doctor --workspace /path/to/lake/workspace --format json
```

## Caches and replacement hints

Project and mathlib indexes are cached under the resolved `lean-dup` cache root; shared
project-pinned mathlib indexes default to `~/.cache/lean-dup` (`LEAN_DUP_CACHE_DIR` overrides).

For confirmed external matches, text and JSON reports include read-only replacement hints: the
target declaration, the specific import line, direct-import status, and bounded local source
references to replace or preserve behind a transitional alias.

## Current status

Intra-workspace audits are usable today. `--compare-mathlib` runs but the release-quality gates
`G1 regression_quality` and `G2 precision_control` are open: recall against real mathlib corpora
has not been demonstrated yet. See
[docs/architecture/production-readiness.md](docs/architecture/production-readiness.md) for
the gate table. The CLI is read-only with respect to your Lean source, so trying it costs only
time.

## Architecture

Start with the [end-to-end architecture](docs/architecture/end-to-end-architecture.md) for the
as-built pipeline. Then:

- [Architecture charter](docs/architecture/overview.md)
- [Worker protocol](docs/architecture/worker-protocol.md)
- [Crate factoring](docs/architecture/crate-factoring.md)
- [Production readiness](docs/architecture/production-readiness.md)
- [Search quality](docs/architecture/search-quality.md)

## License

Apache-2.0 OR MIT, at your option. See `LICENSE-APACHE` and `LICENSE-MIT`.
