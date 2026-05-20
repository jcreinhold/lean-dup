# Vector Search Validation

**Decision: hidden vector candidate generation stays off by default.** Completed labeled
suites show no recall gain over symbolic retrieval at any pipeline stage; vector candidate
facts must not feed threshold calibration. The experiment is retained because the hidden
path is bounded, privacy-safe, deterministic on cache-only failure, reproducible across
warm runs, and produced no extra hard-negative leakage.

The completed runs show that the system can prepare BGE-small, embed declaration documents,
build and reuse a persisted vector corpus, and write privacy-safe artifacts. They do not
show a recall lift. The KanProofs manual suites skipped on missing `.olean` files, so this
run carries no mathlib-scale retrieval evidence. The next useful study needs compiled
KanProofs or mathlib oleans plus a fixture where symbolic retrieval is known to miss a
labeled positive that vector search should recover.

External reference: the
[FastEmbed supported-models table](https://qdrant.github.io/fastembed/examples/Supported_Models/)
lists `BAAI/bge-small-en-v1.5` as a 384-dimensional text model with larger alternatives
available; the [LanceDB vector-index documentation](https://docs.lancedb.com/indexing/vector-index)
describes the IVF and HNSW backends used under the hidden stack. Neither is quality
evidence on its own.

## Setup

```sh
cargo run -p lean-dup-cli -- embedding prepare \
  --policy download-if-missing \
  --format json \
  --cache-root target/search-quality/vector-validation/hf-cache
```

Prepared model facts: `BAAI/bge-small-en-v1.5`, profile `bge-small-en-v1.5`, backend
family `fastembed`, dimension 384. Prepare downloaded 133.8 MB into a 128 MiB isolated
cache; wall time 11.20 s, peak RSS 412 MB. Cache-only with an empty cache writes
`default-skipped-vector-search.json` with `status = "skipped"` and `reason =
"vector-model-not-prepared"`; symbolic metrics remain unchanged.

Outputs are written under `target/search-quality/vector-validation/` and
`target/eval/vector-validation/`.

## Quality

"Vector stage" is recall at the vector-generated stage only; merged/ranked/visible are the
actual pipeline stages after vector and symbolic candidates merge. Hard negatives reported
are the count that became visible.

| Workload                              | Policy                          | Status     | Sym gen | Vec stage | Merged | Ranked | Visible R | Visible P | HN visible |
| ------------------------------------- | ------------------------------- | ---------- | ------: | --------: | -----: | -----: | --------: | --------: | ---------: |
| default                               | `name-and-formal-statement` cold | ok        |   16/16 |      5/16 |  16/16 |  16/16 |     14/16 |     14/34 |        0/3 |
| default                               | `name-and-formal-statement` warm | ok        |   16/16 |      5/16 |  16/16 |  16/16 |     14/16 |     14/34 |        0/3 |
| default                               | `formal-statement`               | ok        |   16/16 |      5/16 |  16/16 |  16/16 |     14/16 |     14/34 |        0/3 |
| default                               | `informal-or-formal`             | ok        |   16/16 |      5/16 |  16/16 |  16/16 |     14/16 |     14/34 |        0/3 |
| hard-negatives                        | `name-and-formal-statement`      | ok        |     1/1 |       1/1 |    1/1 |    1/1 |       1/1 |      1/34 |        0/5 |
| production-gate, fast children only   | `name-and-formal-statement`      | incomplete |   17/17 |      6/17 |  17/17 |  17/17 |     15/17 |     15/68 |        0/8 |

All three input policies share the BGE-small/FastEmbed model and the LanceDB-backed
corpus. On this fixture, policy choice changes nothing downstream of vector generation;
`informal-or-formal` falls back to formal statements because the fixture documents carry
no docstrings.

Vector generation produced 0/3, 0/5, and 0/8 labeled hard negatives. Visible hard-negative
leakage did not increase on any completed suite.

## Runtime and cache

Times below come from JSON runtime counters and from warm `/usr/bin/time` after release
build completion; the first release-mode run includes a one-time stack rebuild that is not
a search-runtime measurement.

| Workload              | Corpus  | Embedding | Corpus build | Query  | Eval total | Wall    | Peak RSS |
| --------------------- | ------- | --------: | -----------: | -----: | ---------: | ------: | -------: |
| default/name cold     | built   |    314 ms |        13 ms | 132 ms |    1480 ms |  5.46 s |   522 MB |
| default/name warm     | reused  |      0 ms |         0 ms |  92 ms |    1137 ms |  4.72 s |   449 MB |
| hard-negatives/name   | built   |    247 ms |         8 ms |  74 ms |    1448 ms |  4.96 s |   558 MB |
| production-gate       | mixed   |         — |            — |      — |    2412 ms |  7.08 s |   558 MB |

The warm run reported `corpus_status = "reused"`, `corpus_build_ms = 0`, and
`embedding_ms = 0`. Comparing normalized pair rows for `default-name-run1` and
`default-name-run2` showed identical pair ordering, vector ranks, vector scores, labels,
and visibility facts.

After validation: 128 MiB Hugging Face cache, 2.0 MiB text-vector cache, 180 KiB vector
corpus cache.

## Manual suite blocker

```text
index error: missing compiled oleans for index (5 missing; sample:
KanProofs.IUT.Foundation.Mutation.PositiveExistentialSolution,
KanProofs.ModelTheory.SetTheory.ZFC.Arithmetic.Int.Relation,
KanProofs.ModelTheory.SetTheory.ZFC.Arithmetic.Nat.Domain,
KanProofs.ModelTheory.SetTheory.ZFC.Grothendieck.Algebra.Ring,
KanProofs.ModelTheory.SetTheory.ZFC.Grothendieck.FiniteFold)
```

Skipped manual children do not count as passes. This validation cannot speak to
mathlib-scale vector retrieval quality.

## Boundary and leak evidence

`cargo test -p lean-dup-cli --test boundaries` passes. The allowed shape:

- embedding runtime dependencies remain inside `lean-dup-embedding`;
- vector database dependencies remain inside `lean-dup-vector-index`;
- `lean-dup-search` imports only crate-root embedding and vector-index APIs for the hidden
  vector policy;
- `lean-dup-eval` owns artifacts and labels;
- `lean-dup-report` projects status/path/stage facts only.

Leak checks over `target/search-quality/vector-validation/*-vector-search.json` found no
backend names, raw Lean or source text, SQLite/posting vocabulary, worker rows, absolute
private paths, model input prefixes, vector-database layout vocabulary, tokenizer or
tensor terms, or cache-root paths.

## Hidden knowledge boundary

`lean-dup-embedding` owns model profiles, model acquisition, query/document wrapping,
CPU inference, normalization, runtime counters, and text-vector caching.
`lean-dup-vector-index` owns vector corpus storage, provenance, reuse, nearest-neighbor
lookup, backend choice, and backend diagnostics. `lean-dup-search` owns hybrid
candidate-generation policy, including per-query vector top-k and merge rules.
`lean-dup-eval` owns labels, workload lifecycle, artifacts, denominators, and the go/no-go
decision. FastEmbed internals, tokenizer rules, model prefixes, LanceDB table layout,
index parameters, vector-cache filenames, raw declaration text, source snippets, SQLite
rows, and worker transport details do not cross those boundaries.

Default symbolic duplicate auditing and ordinary eval remain embedding-free and
vector-index-free.

## Red-flag checklist

- *Shallow module:* validation consumes stable facts from embedding, vector-index, search,
  and eval; no new backend-specific surface.
- *Pass-through wrapper:* the doc records a decision from denominators, not from command
  success.
- *Temporal decomposition:* build/query completion is not promotion evidence.
- *Information leakage:* artifact leak checks pass; backend names stay in architecture
  evidence rather than search or report API.
- *Special-general mixture:* fixture/manual policy stays in eval; vector mechanics stay in
  vector-index; model mechanics stay in embedding.
- *Conjoined methods:* search owns candidate policy; eval owns the decision.
- *Hard-to-describe public API:* model profile, declaration document, vector corpus, and
  search stage facts.
- *Implementation details in interface comments:* this validation doc added none.
