# lean-dup

A read-only duplication auditor for Lean 4 Lake workspaces. It indexes declarations from the
elaborated Lean environment and reports likely duplicate or subsumed statements. Local and
deterministic: no network services, no embeddings, no proof-term analysis.

## Build

```sh
cargo build --release -p lean-dup-cli
cd lean && lake build
```

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

## Caches and replacement hints

Project and mathlib indexes are cached under the resolved `lean-dup` cache root; shared project-pinned mathlib
indexes default to `~/.cache/lean-dup` (`LEAN_DUP_CACHE_DIR` overrides).

For confirmed external matches, text and JSON reports include read-only replacement hints: the target declaration,
the specific import line, direct-import status, and bounded local source references to replace or preserve behind a
transitional alias.

## Architecture

Start with the [end-to-end architecture](docs/architecture/06-end-to-end-architecture.md) for the
as-built pipeline. Then:

- [Architecture charter](docs/architecture/00-overview.md)
- [Worker protocol](docs/architecture/01-worker-protocol.md)
- [Crate factoring](docs/architecture/07-crate-factoring.md)
- [Production readiness](docs/architecture/04-production-readiness.md)
- [Search quality](docs/architecture/search-quality.md)

The Python implementation has been retired; the production command surface is the Rust
`lean-dup` binary. The [deprecation map](docs/architecture/python-deprecation-map.md) records what
each Python module was superseded by.
