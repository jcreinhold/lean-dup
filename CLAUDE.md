# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`lean-dup` is a read-only duplication auditor for Lean 4 Lake workspaces. It indexes declarations from the *elaborated*
Lean environment (via `.olean` files) and reports likely duplicate or subsumed statements. The default audit path is
local and deterministic: no network, no embeddings, no proof-term analysis. The CLI never edits audited Lean source — it
only reads source, builds Lean artifacts through Lake, and writes indexes/reports/diagnostics under the cache root or
`target/`.

It is a Rust workspace (`edition = "2024"`, Rust 1.91+) plus a small Lean worker package under `lean/`. The pinned Lean
toolchain is `leanprover/lean4:v4.30.0` (other 4.x untested).

## Commands

```sh
# Build
cargo build --release -p lean-dup-cli
(cd lean && lake build LeanDup)   # builds the LeanDup shared-facet capability dylib + LeanDup.olean (needed by the audit pipeline)

# Lint / format (CI runs with -D warnings; treat all clippy warnings as errors)
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings

# Test
cargo test --workspace --locked
cargo test -p lean-dup-cli --test boundaries --locked        # architecture/boundary tests
cargo test -p lean-dup-search audit::                        # single module's tests
cargo test -p lean-dup-cli --test cli -- <name>              # single CLI integration test by name

# Run the auditor locally (swap target/release/lean-dup for this during dev)
cargo run -p lean-dup-cli -- audit --workspace /path/to/lake/workspace --compare-mathlib --progress
cargo run -p lean-dup-cli -- doctor --workspace tests/fixtures/tiny --module Tiny --format json
cargo run -p lean-dup-cli -- eval --suite default --format json
```

Tests shell out to `lake`: `crates/eval` and the CLI integration suite build/audit the Lean fixtures under
`tests/fixtures/{tiny,external,source-backed}`. Those fixtures must be `lake build`-ed (and `lean/` built) before the
Rust tests can find the `.olean` artifacts — see `.github/workflows/ci.yml` for the exact pre-build sequence CI uses.

User-facing commands: `doctor`, `index`, `index-mathlib`, `audit`, `show`, `diff`, `eval`. Hidden developer commands
(production engineering only): `embedding prepare`, `perf`, `cache-cleanup`.

## The one architectural rule

Read `docs/architecture/overview.md` and `docs/architecture/end-to-end-architecture.md` before non-trivial changes. The
single rule above everything else:

- **Lean** computes semantic facts that require the elaborated environment (declaration identity, fingerprints,
  role-aware feature keys, bounded probe results).
- **Rust** owns everything else: workspace discovery, Lake invocation, worker lifecycle, persistence, retrieval,
  ranking, reporting, evaluation.

Rust talks to Lean only through the narrow, versioned worker protocol (`docs/architecture/worker-protocol.md`), carried
by the `lean-rs-worker-parent` pool loading the `LeanDup` shared-facet capability through `lean-dup-worker-child` (no
subprocess). Rust must **not** inspect Lean `Expr`s, recompute semantic fingerprints from pretty-printed type text, or
rely on pretty-printed statement strings as anything but display. The request schema and JSON payloads are transport,
not architecture.

## Layout and boundaries

The pipeline is `CLI → Workspace → Worker → Index (SQLite) → Retrieval → Verification → Ranking → Source facts → Report
contract → Render`. Each crate is a deep module hiding one decision that changes; callers ask for capabilities, not
lower-level steps.

| Crate | Owns | Must not leak / depend on |
| --- | --- | --- |
| `cli` | command surface (`cli.rs`), routing + I/O (`commands.rs`), rendering, hidden `perf`/`release` | Lake layout, worker transport, SQLite schema, audit phase order, ranking policy |
| `project` | `workspace.rs` (Lake discovery, module roots, source enumeration), `mathlib.rs` (project-pinned mathlib contract) | — |
| `worker` | Rust side of the worker protocol; pool capability lifecycle (`engine/`: `WorkerEngine`/`PoolEngine`/`LeanDupCapabilityRuntime`), substrate facts, error mapping | Lean `Expr` constructors, pretty text, private worker batching, capability symbol names, child ABI |
| `index` | `index.rs` (SQLite local/external/mathlib), `cache.rs` + `cache_lifecycle.rs`, `external_provenance.rs` (source-backed vs static) | SQLite table names, cache-key JSON, latest-pointer details must stay inside this crate |
| `search` | the full audit workflow (`audit::run_audit`): `retrieval.rs`, `semantic_verification.rs`, `ranking.rs`, `source_refs.rs`, `replacement_hints.rs`, `baseline.rs` | SQLite layout, worker command names |
| `report` | stable explanation facts → text/JSON/`show`/`diff` (`report-contract.md`) | terminal layout leaking into audit/ranking |
| `eval` | suite definitions, label provenance, hard-negative gate, raw metrics | corpus paths in default CI |
| `diagnostics` | progress (`indicatif`), profiling, structured errors | — |
| `embedding`, `vector-index`, `vector-search` | optional/experimental external extensions (`fastembed`, `lancedb`); **not** on the core audit path | — |

The Lean worker lives in `lean/LeanDup/` (`Extract`, `Features`, `Probe`, `Protocol`, `Index`, `Capability`).
`Capability` exposes five capability command exports loaded by the pool: `version` (json command) and the streaming
`extract`, `features`, `index`, `probe`. `doctor`-style health is composed Rust-side from `version` plus
workspace/cache checks; worker substrate facts come from the pool handshake. See
`docs/architecture/validation/worker-migration-validation.md`.

Red flags (from `overview.md#design-rules`): table-name leakage outside `index`; worker-command names in
retrieval/ranking/reporting; Rust recomputing Lean facts from pretty text; modules named after audit *phases* that share
hidden state (temporal decomposition); corpus-specific cleanup policy entering general ranking/retrieval; a static index
label like `mathlib` implying proof-grade evidence without provenance.

## Conventions that bite

- **Schema versions are contracts.** CI asserts exact strings: `report_schema_version == "lean-dup.report.v3"`,
  `worker.protocol_version == "lean-dup.worker.v1"`. Bumping a schema means updating its doc and the CI assertions
  together. Audit JSON is additive.
- **stdout stays parseable.** Progress and `--profile` output go to **stderr only** so `--format json` stdout is never
  corrupted. Keep it that way.
- **Default queue is high-precision, not a dump.** Feature-only/noisy groups are hidden unless the user passes
  `--private`, `--low-priority`, or `--diagnostics`. Don't make broad candidate dumps the default.
- **Cache invalidation tracks semantic inputs only** (Lean source, Lake files, toolchain, worker/protocol/ index
  semantic versions, include policy, selected roots, relevant deps) — never unrelated non-Lean files or broad repo
  dirtiness. Default cache root `~/.cache/lean-dup`, override `LEAN_DUP_CACHE_DIR`.
- **Lints are strict.** The workspace denies `unsafe-code` and `rust-2024-compatibility`; clippy runs
  `pedantic`/`nursery` plus restriction lints (`unwrap_used`, `expect_used`, `panic`, `indexing_slicing`,
  `arithmetic_side_effects`, `todo`, `unimplemented` are all `warn` → errors in CI). Prefer `.get()`, checked
  arithmetic, and `Result` over panicking constructs.
- **Production gates are open.** `--compare-mathlib` runs but gates `G1 regression_quality` /
  `G2 precision_control` are not met (recall not yet demonstrated). See `docs/architecture/production-readiness.md`.
</content>
