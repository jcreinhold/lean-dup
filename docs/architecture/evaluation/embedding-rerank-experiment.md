# Embedding Rerank Experiment

Hidden experiment: measure whether local sentence embeddings help order candidates the
symbolic pipeline already observed. The experiment does not change candidate generation,
symbolic ranking, semantic probes, report JSON semantics, or the default audit path.

For the embedding runtime boundary, see
[embedding-architecture.md](embedding-architecture.md). For crate boundaries, see
[../crate-factoring.md](../crate-factoring.md).

## Boundary

- *Eval* owns experiment lifecycle, label joining, skipped-state handling, artifact
  schema, and comparison against symbolic metrics.
- *Search* owns the declaration-document policy.
- *Embedding* owns model acquisition, CPU inference, model-specific role wrapping, the
  vector cache, and runtime counters.
- *Report* projects optional status and artifact paths only.

Public surface: search exposes stable declaration-document facts on `SearchObservation`;
eval accepts an optional hidden embedding-rerank request and writes one artifact; report
copies optional status and path fields. No caller receives tokenizer state, tensor
shapes, model filenames, vector-cache paths, or Hugging Face cache layout.

The rejected alternative was for search to own model acquisition, artifact writing, and
rerank scoring. That would have mixed search policy with model-runtime mechanics, label
evaluation, suite knowledge, and artifact schema—four independent decisions.

## Default behavior unchanged

Normal `audit`, `doctor`, `show`, `diff`, and ordinary `eval` do not download, load, or
require embedding models. The artifact ban list (tokenizer/runtime details,
model-specific input prefixes, embedding batching, vector-cache key format, model-file
names, cache layout, source snippets, raw Lean expressions, retrieval keys, SQLite
storage, worker rows) stays enforced.

## Artifact contract

```sh
cargo run -p lean-dup-cli -- eval --suite default --format json --write-embedding-rerank
```

Writes `target/search-quality/<suite>-embedding-rerank.json`. Schema version:
`lean-dup.embedding-rerank.v1`.

| Field                 | Contents                                                       |
| --------------------- | -------------------------------------------------------------- |
| `schema_version`      | artifact schema version                                        |
| `suite`               | eval suite name                                                |
| `status`              | `ok`, `skipped`, or `failed`                                   |
| `reason`              | stable skip or failure reason when present                     |
| `model`               | model id, revision, fingerprint                                |
| `cache`               | prepared or unprepared cache status                            |
| `acquisition_policy`  | `cache-only` or `download-if-missing`                          |
| `input_policy_id`     | search-owned declaration-document policy                       |
| `input_policy_version`| search-owned declaration-document contract                     |
| `runtime`             | embedding runtime counters                                     |
| `symbolic_baseline`   | current metrics and scorer version                             |
| `embedding_rerank`    | rerank metrics over the same observed pool                     |
| `pairs`               | deterministic per-pair comparison rows                         |

The visible budget for embedding rerank is the current symbolic shown-queue count. The
experiment does not invent a production threshold: it sorts observed pairs by cosine
similarity and marks the top budget as visible for metric comparison.

**Limitation:** this is reranking over an already-observed pool. A positive missing from
that pool is still missing after rerank. The experiment cannot speak to vector
candidate-generation recall.

## Allowed and forbidden artifact contents

| Allowed                                                                                   | Forbidden                                                                                                                                                                       |
| ----------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| declaration names; label metadata; model/cache summaries; document-policy id; runtime counters; ranks; visibility facts; cosine similarities; privacy-safe content hashes | tokenizer internals; tensor shapes; model filenames; cache file paths; vector-cache filenames; final model-input text; raw formal statements; source snippets; raw Lean expressions; worker rows; SQLite tables; posting vocabulary; retrieval keys; absolute private paths |

## Evidence commands

```sh
cargo run -p lean-dup-cli -- eval --suite default --format json --write-embedding-rerank
test -f target/search-quality/default-embedding-rerank.json
```

Re-run the artifact leak grep against the same artifact (the `/Users/` literal in the
pattern is a deliberate filter for absolute private paths):

```sh
rg -n 'tokenizer|tensor|model\.safetensors|snapshot|sqlite|posting|/Users/|statement_text|worker JSONL' \
  target/search-quality/default-embedding-rerank.json
```

A match in the leak check must be intentional stable vocabulary, not a private runtime
or storage detail.

## Red-flag checklist

- *Shallow module:* one hidden eval capability that reuses search, embedding, and report
  root facts.
- *Pass-through wrapper:* eval performs label joining, skipped-state handling, metric
  comparison, and artifact writing.
- *Temporal decomposition:* split by owned knowledge, not by execution order alone.
- *Information leakage:* model, runtime, and cache details stay inside
  `lean-dup-embedding`.
- *Special-general mixture:* hidden and lean-dup-specific until validation proves a
  production need.
- *Conjoined methods:* embedding rerank does not alter symbolic ranking or candidate
  generation.
- *Hard-to-describe public API:* optional eval request/output facts and search-owned
  declaration-document facts.
- *Implementation details in interface comments:* none; interface comments name
  caller-visible behavior.
