# Architecture docs

Reference material for contributors. For a Lean-developer-facing walkthrough, see
[../getting-started.md](../getting-started.md).

## Where to start

- [end-to-end-architecture.md](end-to-end-architecture.md) — the as-built pipeline. Start here.
- [overview.md](overview.md) — the layering rule (Lean/Rust boundary), the design rules other docs rely on.
- [production-readiness.md](production-readiness.md) — release gates, evidence, no-go criteria.

## By topic

**Boundaries and contracts**

- [worker-protocol.md](worker-protocol.md) — Lean worker protocol (six commands, JSONL transport, v1 schema).
- [rust-cli-foundation.md](rust-cli-foundation.md) — CLI engine module map.
- [crate-factoring.md](crate-factoring.md) — the eight Rust crates and their boundaries.
- [report-contract.md](report-contract.md) — stable explanation facts every report must carry.

**Indexing and comparison**

- [cache-validity-lifecycle.md](cache-validity-lifecycle.md) — when an index is stale, doctor and cleanup.
- [external-comparison-provenance.md](external-comparison-provenance.md) — `proof-grade` vs `source-backed-not-importable` vs `static` evidence modes.

**Search quality**

- [search-quality.md](search-quality.md) — match-class taxonomy, four-stage pipeline, quality contract.
- [evaluation/](evaluation/) — labels, metrics, gates, scorer, dataset artifacts.

**Historical**

- [python-deprecation-map.md](python-deprecation-map.md) — what the retired Python implementation was superseded by.
