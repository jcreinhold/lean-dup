# lean-dup

`lean-dup` is a read-only duplication auditor for Lean 4 Lake workspaces.

It indexes declarations from the elaborated Lean environment, then reports likely duplicate or subsumed statements. It
is intentionally local and deterministic: no network services, no embeddings, and no proof-term analysis in v1.

## Usage

Build the Rust binary and Lean worker first:

```sh
cargo build -p lean-dup-rs
cd lean && lake build
```

For local development, run the Rust CLI through Cargo:

```sh
cargo run -p lean-dup-rs -- doctor --workspace /path/to/lake/workspace
cargo run -p lean-dup-rs -- audit --workspace /path/to/lake/workspace --format text
cargo run -p lean-dup-rs -- audit --workspace /path/to/lake/workspace --format json
cargo run -p lean-dup-rs -- audit --workspace /path/to/lake/workspace --compare-mathlib --progress
cargo run -p lean-dup-rs -- show --workspace /path/to/lake/workspace --group exact-statement-1
```

For production-style local runs, use the release binary:

```sh
cargo build --release -p lean-dup-rs
target/release/lean-dup-rs audit --workspace /path/to/lake/workspace --compare-mathlib --progress
```

## Architecture Docs

Start with
[docs/architecture/06-end-to-end-architecture.md](/Users/jcreinhold/Code/lean-dup/docs/architecture/06-end-to-end-architecture.md)
for the as-built pipeline and design rationale. Production gates are tracked in
[docs/architecture/04-production-readiness.md](/Users/jcreinhold/Code/lean-dup/docs/architecture/04-production-readiness.md),
and the Lean/Rust worker protocol is specified in
[docs/architecture/01-worker-protocol.md](/Users/jcreinhold/Code/lean-dup/docs/architecture/01-worker-protocol.md).

By default, `audit` scans the inferred local Lake workspace roots and includes private theorem-like declarations when
Lean exposes them through compiled modules. Use `--module Root.Module` to restrict the audit to one root module and its
descendants.

Useful audit flags:

- `--public-only`: exclude private declarations from the report.
- `--include-imports`: compare workspace declarations with direct imports.
- `--import-root Mathlib.Some.Module`: compare with an additional named module.
- `--compare-mathlib`: compare workspace declarations with the cached mathlib index.
- `--threshold 0.82`: adjust the near-duplicate threshold.
- `--profile`: include extraction and classification timings in the report.
- `--no-replacement-hints`: skip import/replacement hint generation.

Project and mathlib indexes are cached under the resolved `lean-dup` cache root; shared project-pinned mathlib indexes
default to `~/.cache/lean-dup`. For confirmed external matches, text and JSON reports include read-only replacement
hints: the target declaration, the specific import line, direct-import status, and bounded local source references to
replace or preserve behind a transitional alias.

The Python implementation has been retired. Historical Python modules and tests were used as regression evidence during
the Rust/Lean rewrite; the production command surface is the Rust `lean-dup-rs` binary.
