# Embedding Rerank Experiment

This document records the hidden embedding rerank experiment added after the CPU
embedding runtime. The experiment measures whether local sentence embeddings help order
already-observed candidates. It does not change candidate generation, symbolic ranking,
semantic probes, report JSON semantics, or the default audit path.

For the embedding runtime boundary, see [embedding-architecture.md](embedding-architecture.md).
For crate boundaries, see [../crate-factoring.md](../crate-factoring.md).

## Design Note

Hidden knowledge: eval owns the embedding experiment lifecycle, label joining, skipped
status, artifact schema, and comparison against symbolic metrics. Search owns the
declaration-document policy. Embedding owns model acquisition, CPU inference,
model-specific role wrapping, vector cache, and runtime counters. Report owns projection
of optional status and artifact paths.

Smallest public interface: search exposes stable declaration-document facts on
`SearchObservation`; eval accepts an optional hidden embedding-rerank request and writes
one artifact; report copies optional status/path fields. No caller receives tokenizer
state, tensor shapes, model filenames, vector-cache paths, or Hugging Face layout.

Decisions that must not leak upward or sideways: tokenizer/runtime details, model-specific
input prefixes, embedding batching, vector-cache key format, model-file names, cache
layout, source snippets, raw Lean expressions, retrieval keys, SQLite storage, and worker
rows.

Preserved capability: default read-only symbolic duplicate auditing remains authoritative.
Normal `audit`, `doctor`, `show`, `diff`, and ordinary `eval` do not download, load, or
require embedding models.

Discarded Python-era behavior: unrecorded semantic-search experiments and anecdotal model
inspection. Embedding evidence now requires deterministic artifacts with labels, baseline
metrics, embedding metrics, runtime/cache facts, and hard-negative impact.

## Design It Twice

Rejected: search owns model acquisition, artifact writing, and rerank scoring. That would
complect search policy with model-runtime mechanics and label evaluation. Search would need
to know download policy, model readiness, vector-cache behavior, suite labels, and artifact
schema, which are independent decisions.

Chosen: eval owns experiment lifecycle and artifacts, search owns declaration-document
policy, and embedding owns runtime/cache mechanics. This is deeper because each crate
hides one volatile decision: search can change document policy without touching model
loading, the embedding crate can change role wrapping or runtime internals without
touching labels, and eval can change artifact comparison without touching ranking.

## Artifact Contract

Hidden command:

```sh
cargo run -p lean-dup-cli -- eval --suite default --format json --write-embedding-rerank
```

The command writes `target/search-quality/<suite>-embedding-rerank.json`.

Schema version: `lean-dup.embedding-rerank.v1`.

Top-level fields:

| Field | Contents |
| --- | --- |
| `schema_version` | artifact schema version |
| `suite` | eval suite name |
| `status` | `ok`, `skipped`, or `failed` |
| `reason` | stable skip/failure reason when present |
| `model` | model id/revision/fingerprint facts |
| `cache` | prepared/unprepared cache status |
| `acquisition_policy` | `cache-only` or `download-if-missing` |
| `input_policy_id` | search-owned declaration-document policy |
| `input_policy_version` | search-owned declaration-document contract |
| `runtime` | embedding runtime counters |
| `symbolic_baseline` | current metrics and scorer version |
| `embedding_rerank` | rerank metrics over the same observed pool |
| `pairs` | deterministic per-pair comparison rows |

The visible budget for embedding is the current symbolic shown-queue count. Embedding
rerank does not invent a production threshold. It sorts observed candidate pairs by cosine
similarity and marks the top budget as visible for metric comparison.

## Rerank-Only Limitation

This experiment measures reranking signal over candidates already observed by the symbolic
pipeline. It does not measure vector candidate-generation recall over all mathlib. A
positive missing from the observed candidate pool is still missing for this experiment.

## Privacy Rules

Artifacts may contain declaration names, label metadata, stable model/cache summaries,
document policy ids, privacy-safe content hashes, runtime counters, ranks, visibility
facts, and cosine similarities.

Artifacts must not contain tokenizer internals, tensor shapes, model filenames, cache file
paths, vector-cache filenames, final model input text, raw formal statements, source
snippets, raw Lean expressions, worker rows, SQLite table names, posting vocabulary,
retrieval keys, or absolute private paths.

## Evidence Commands

```sh
cargo run -p lean-dup-cli -- eval --suite default --format json --write-embedding-rerank
test -f target/search-quality/default-embedding-rerank.json
rg -n 'tokenizer|tensor|model\\.safetensors|snapshot|sqlite|posting|/Users/|statement_text|worker JSONL' \
  target/search-quality/default-embedding-rerank.json
```

Any match in the leak check must be intentional stable vocabulary, not a private runtime or
storage detail.

## Red Flag Review

- Shallow module: mitigated. The experiment adds one hidden eval capability and reuses
  search/embedding/report root facts instead of exposing runtime internals.
- Pass-through wrapper: mitigated. Eval performs label joining, skipped-state handling,
  metric comparison, and artifact writing; it is not a thin embedding call.
- Temporal decomposition: mitigated. Search summary construction, embedding runtime, and
  eval artifact comparison are split by owned knowledge, not by execution order alone.
- Information leakage: mitigated. Model/runtime/cache details stay inside
  `lean-dup-embedding`; search/report never import embedding internals.
- Special-general mixture: mitigated. The experiment is hidden and lean-dup-specific until
  validation proves a production need.
- Conjoined methods: mitigated. Embedding rerank does not alter symbolic ranking or
  candidate generation.
- Hard-to-describe public API: mitigated. Public additions are optional eval request/output
  facts and search-owned declaration-document facts.
- Implementation details contaminating interface comments: mitigated. Interface comments
  name caller-visible behavior and hidden decisions, not tokenizer files, tensor layout, or
  model-prefix strings.
