# Semantic Reranking and Obligation Yield

Semantic reranking turns selected Lean probe obligations into stable search-quality facts.
The boundary owns obligation selection, yield accounting, cache-hit accounting, and the
projection from worker probe results to search evidence. Eval, report, and CLI see
versioned obligation facts; they do not see worker rows, probe chunks, cache keys, Lean
expression payloads, or retrieval/ranking internals.

Semantic reranking is private to `lean-dup-search`. The alternative—letting eval and
report inspect `ProbeDiagnostics`, worker rows, or ranking groups directly—would force
every scoring, probe, and worker refactor to become a JSON-contract migration.

## Contract

Schema version: `lean-dup.semantic-reranking.v1`.

| Stable obligation kinds  | Stable statuses | Stable unavailable reasons   |
| ------------------------ | --------------- | ---------------------------- |
| `exact-theorem`          | `planned`       | `missing-declaration`        |
| `permuted-theorem`       | `verified`      | `unsupported`                |
| `replacement`            | `rejected`      | `opaque-or-unreducible`      |
| `reducible-definition`   | `unavailable`   | `timeout`                    |
| `specialization`         | `cached`        | `internal-error`             |
| `local-duplicate`        |                 | `unknown`                    |

The yield contract records `planned`, `verified`, `rejected`, `unavailable`, `cached`,
and `worker_pairs` per obligation kind. These are denominators for search-quality work,
not release gates by themselves.

## Hidden knowledge boundary

Search decides which candidate pairs deserve Lean obligations, how obligations map to
worker probes, how probe results are cached and recovered, and how unavailable reasons
are normalized. Eval joins facts with labels; report projects them. Worker JSONL framing,
probe chunking, cache-key ingredients, Lean expression details, SQLite layout, retrieval
key strings, and ranking structs do not cross the boundary.

The default user interface remains `audit`, `show`, `diff`, and `eval`. No probe transport
or heartbeat flag is added.

## Artifacts

Search datasets include the semantic-reranking version, per-pair semantic evidence state,
per-pair obligation facts, and aggregate obligation yield. Scorer ablation artifacts
include the same version and yield so the `semantic-evidence-only-rerank` row is
interpretable.

Forbidden artifact payloads: raw Lean expressions, raw source text, worker JSONL rows,
probe cache keys, SQLite rows, posting names, absolute private paths.

## Current evidence

Fixture evidence:

```sh
cargo run -p lean-dup-cli -- eval --suite default --format json \
  --write-search-dataset --write-scorer-ablations
```

Production-gate evidence:

```sh
cargo run -p lean-dup-cli -- eval --suite production-gate --format json \
  --write-scorer-ablations --output target/eval/production-gate.json
```

Manual KanProofs suites may skip when compiled oleans are unavailable; a skipped manual
child is a prerequisite blocker, not a quality pass. The fixture suite still records
the semantic-reranking version and zero-yield retrieval observations;
audit-backed source-backed fixture tests exercise verified proof-grade evidence.

Observed today on this workspace:

- `eval --suite default … --write-search-dataset --write-scorer-ablations` completes with
  `semantic_reranking.version = lean-dup.semantic-reranking.v1` in stage metrics, dataset
  artifacts, and ablation artifacts.
- Retrieval-only eval observations report zero obligation yield. This is expected until
  an audit-backed eval observation path is added.
- The source-backed fixture audit test verifies proof-grade evidence and records nonzero
  planned and verified probe counts.
- `production-gate` completes as `incomplete` locally because both manual child suites
  skip with reason `manual suite workspace is unavailable`.

## Red-flag checklist

- *Shallow module:* public DTOs are small; search hides obligation planning, cache use,
  recovery, and worker calls.
- *Pass-through wrapper:* the boundary normalizes statuses and yield counters; it does
  not forward probe rows.
- *Temporal decomposition:* callers do not run "plan, probe, then rerank"; search owns
  the order.
- *Information leakage:* worker transport, cache keys, raw expressions, retrieval keys,
  and SQLite details do not enter artifacts.
- *Special-general mixture:* fixture/manual suite policy stays in eval; semantic
  obligation policy stays in search.
- *Conjoined methods:* scoring consumes semantic facts; it does not run probes or parse
  worker diagnostics.
- *Hard-to-describe public API:* version, obligation kind, status, reason, and counters.
- *Implementation details in interface comments:* interface comments describe
  caller-visible facts, not probe chunking or cache-key construction.
