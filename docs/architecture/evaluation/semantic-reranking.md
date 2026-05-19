# Semantic Reranking And Obligation Yield

Semantic reranking turns selected Lean probe obligations into stable search-quality facts. The boundary owns obligation
selection, yield accounting, cache-hit accounting, and the projection from worker probe results to search evidence.
Eval, report, and CLI see versioned obligation facts; they do not see worker rows, probe chunks, cache keys, Lean
expression payloads, or retrieval/ranking internals.

## Design Note

Hidden knowledge: which candidate pairs deserve Lean obligations, how obligations map to worker probes, how probe
results are cached and recovered, and how unavailable reasons are normalized.

Smallest public interface: search-owned DTOs for semantic-reranking version, per-pair obligation facts, and aggregate
yield counters by obligation kind. The normal user interface remains `audit`, `show`, `diff`, and `eval`; no probe
transport or heartbeat flag is added.

Decisions that must not leak: worker JSONL framing, probe chunking, cache-key ingredients, Lean expression details,
SQLite layout, retrieval key strings, and ranking structs.

Preserved capability: read-only duplicate auditing with bounded proof-grade semantic evidence, stable report JSON/text,
and eval artifacts that explain where proof-grade evidence helped or failed.

Discarded Python-era behavior: treating manual probe inspection or anecdotal timing as evidence. Semantic work is now
counted by obligation kind, status, cache use, and worker cost.

## Design It Twice

Rejected: let eval and report inspect `ProbeDiagnostics`, worker rows, or ranking groups directly. That would make
quality artifacts easy to produce in the short term, but it would also force every scoring, probe, and worker refactor
to become a JSON-contract migration.

Chosen: keep semantic reranking private to `lean-dup-search` and export only stable facts. Search owns obligation
planning and converts probe results into `SearchSemanticObligationFact` and `SearchSemanticObligationYield`. Eval
joins those facts with labels; report projects them into stable JSON/text. This is deeper because callers learn only
obligation kind, status, reason, version, and counters.

## Contract

Semantic reranking version: `lean-dup.semantic-reranking.v1`.

Stable obligation kinds:

- `exact-theorem`
- `permuted-theorem`
- `replacement`
- `reducible-definition`
- `specialization`
- `local-duplicate`

Stable statuses:

- `planned`
- `verified`
- `rejected`
- `unavailable`
- `cached`

Stable unavailable reasons:

- `missing-declaration`
- `unsupported`
- `opaque-or-unreducible`
- `timeout`
- `internal-error`
- `unknown`

The yield contract records `planned`, `verified`, `rejected`, `unavailable`, `cached`, and `worker_pairs` for each
obligation kind. These are denominators for search-quality work, not release gates by themselves.

## Artifacts

Search datasets include semantic-reranking version, per-pair semantic evidence state, per-pair obligation facts, and
aggregate obligation yield. Scorer ablation artifacts include the same semantic-reranking version and yield summary so
the `semantic-evidence-only-rerank` row is interpretable.

Forbidden artifact payloads remain forbidden: raw Lean expressions, raw source text, worker JSONL rows, probe cache
keys, SQLite rows, posting names, and absolute private paths.

## Current Evidence

Fixture evidence is produced by:

```sh
cargo run -p lean-dup-cli -- eval --suite default --format json \
  --write-search-dataset --write-scorer-ablations
```

Production-gate evidence is produced by:

```sh
cargo run -p lean-dup-cli -- eval --suite production-gate --format json \
  --write-scorer-ablations --output target/eval/prompt34-production-gate.json
```

Manual KanProofs suites may be skipped when compiled oleans are unavailable. A skipped manual child is a prerequisite
blocker, not a quality pass. The fixture suite still records the semantic-reranking version and zero-yield retrieval
observations; audit-backed source-backed fixture tests exercise verified proof-grade evidence.

Prompt 34 fixture evidence:

- `eval --suite default --format json --write-search-dataset --write-scorer-ablations` completed with
  `semantic_reranking.version = lean-dup.semantic-reranking.v1` in stage metrics, dataset artifacts, and ablation
  artifacts.
- Retrieval-only eval observations currently report zero obligation yield; this is expected until an audit-backed eval
  observation path is added.
- The source-backed fixture audit test verifies proof-grade evidence and records nonzero planned and verified probe
  counts.
- `production-gate` completed as `incomplete` in this local run because both manual child suites were skipped with
  reason `manual suite workspace is unavailable`.

## Red Flag Review

- Shallow module: mitigated. The public DTOs are small while search hides obligation planning, cache use, recovery, and
  worker calls.
- Pass-through wrapper: mitigated. The boundary normalizes statuses and yield counters rather than forwarding probe
  rows.
- Temporal decomposition: mitigated. Callers do not run "plan, probe, then rerank"; `lean-dup-search` owns the order.
- Information leakage: mitigated. Worker transport, cache keys, raw expressions, retrieval keys, and SQLite details do
  not enter artifacts.
- Special-general mixture: mitigated. Fixture/manual suite policy stays in eval; semantic obligation policy stays in
  search.
- Conjoined methods: mitigated. Scoring consumes semantic facts; it does not run probes or parse worker diagnostics.
- Hard-to-describe public API: mitigated. The public story is version, obligation kind, status, reason, and counters.
- Implementation details contaminating comments: mitigated. Interface comments describe caller-visible facts, not probe
  chunking or cache-key construction.
