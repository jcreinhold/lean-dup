# Vector Search Validation

Prompt 35K validates the hidden vector candidate-generation path added in Prompt
35J. This is a validation and decision document, not a default-behavior change.

Decision: keep vector search hidden and off by default for further study. Do not
allow Prompt 36 to use vector candidate facts in threshold calibration yet.

The completed fixture runs show that the system can prepare BGE-small, embed
declaration documents, build and reuse a persisted vector corpus, and write
privacy-safe artifacts. They do not show a recall gain over symbolic retrieval:
merged, ranked, and visible recall are unchanged on every completed labeled
suite. The KanProofs manual suites skipped on missing compiled oleans, so this
run provides no mathlib-scale retrieval evidence.

External references were checked before interpreting the result. The
[FastEmbed supported-models table](https://qdrant.github.io/fastembed/examples/Supported_Models/)
lists `BAAI/bge-small-en-v1.5` as a supported 384-dimensional text model and
lists larger alternatives for later comparison. The
[LanceDB vector-index documentation](https://docs.lancedb.com/indexing/vector-index)
describes persistent vector indexes including IVF and HNSW-backed variants.
Those facts justify the current hidden local stack, but they are not quality
evidence by themselves.

## Design Note

Hidden knowledge:

- `lean-dup-embedding` owns model profiles, model acquisition, query/document
  wrapping, CPU inference, normalization, runtime counters, and text-vector
  caching.
- `lean-dup-vector-index` owns vector corpus storage, provenance, reuse,
  nearest-neighbor lookup, backend choice, and backend diagnostics.
- `lean-dup-search` owns hybrid candidate-generation policy, including the
  per-query vector top-k and merge rules.
- `lean-dup-eval` owns labels, workload lifecycle, artifacts, denominators,
  and the go/no-go decision.

Smallest public interface: stable model/profile facts, vector-corpus facts,
search stage facts, and eval artifacts. The validation doc consumes only those
facts.

Decisions that must not leak upward or sideways: FastEmbed internals, tokenizer
rules, model prefixes, LanceDB table layout, index parameters, vector-cache
filenames, raw declaration text, source snippets, SQLite rows, and worker
transport details.

Preserved capability: default symbolic duplicate auditing and ordinary eval
remain embedding-free and vector-index-free.

Discarded Python-era behavior: a working embedding call, an anecdotal nearest
neighbor, or a rerank-only artifact is not enough to promote vector search.
Promotion requires labeled stage metrics, hard-negative survival, runtime/RSS
cost, cache reuse, and reproducibility.

## Design It Twice

Rejected: treat successful vector-index build/query as enough to keep or
promote vector search. That would validate storage mechanics, not search
quality. A database can work perfectly while candidate generation still fails
to recover any new labeled positives.

Chosen: decide from labeled stage metrics, runtime/RSS/cache cost,
reproducibility, and hard-negative leakage. This design is deeper because each
owning crate reports stable facts and eval makes the decision without learning
model runtime or database mechanics. It also avoids temporal decomposition:
validation is not "prepare, embed, index, query" completion; it is evidence
that the whole hidden search workflow improves candidate generation.

## Workloads

All outputs were written under `target/search-quality/vector-validation/` and
`target/eval/vector-validation/`. The model was explicitly prepared with:

```sh
cargo run -p lean-dup-cli -- embedding prepare \
  --policy download-if-missing \
  --format json \
  --cache-root target/search-quality/vector-validation/hf-cache
```

Prepared model facts:

| Fact | Value |
| --- | --- |
| model | `BAAI/bge-small-en-v1.5` |
| profile | `bge-small-en-v1.5` |
| backend family | `fastembed` |
| dimension | 384 |
| prepared bytes | 133,806,060 |
| isolated model cache size | 128 MiB |
| prepare wall time | 11.20 s |
| prepare peak RSS | 412,516,352 bytes |

Cache-only missing-model behavior was validated with an intentionally empty
cache. The hidden eval wrote
`target/search-quality/vector-validation/default-skipped-vector-search.json`
with `status = "skipped"` and `reason = "vector-model-not-prepared"`. Symbolic
metrics remained parseable and unchanged.

## Completed Evidence

The vector-search columns report the hidden vector-enabled run. "Vector stage"
is recall at the vector-generated stage only; "merged/ranked/visible" are the
actual search pipeline stages after vector and symbolic candidates are merged.

| Workload | Policy | Status | Symbolic generated recall | Vector stage recall | Merged recall | Ranked recall | Visible recall | Visible precision | Hard negatives visible | Candidates |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| default | `formal-statement` | ok | 16/16 | 5/16 | 16/16 | 16/16 | 14/16 | 14/34 | 0/3 | 596 |
| default | `name-and-formal-statement` cold | ok | 16/16 | 5/16 | 16/16 | 16/16 | 14/16 | 14/34 | 0/3 | 596 |
| default | `name-and-formal-statement` warm | ok | 16/16 | 5/16 | 16/16 | 16/16 | 14/16 | 14/34 | 0/3 | 596 |
| default | `informal-or-formal` | ok | 16/16 | 5/16 | 16/16 | 16/16 | 14/16 | 14/34 | 0/3 | 596 |
| hard-negatives | `name-and-formal-statement` | ok | 1/1 | 1/1 | 1/1 | 1/1 | 1/1 | 1/34 | 0/5 | 596 |
| production-gate completed fast children | `name-and-formal-statement` | incomplete | 17/17 | 6/17 | 17/17 | 17/17 | 15/17 | 15/68 | 0/8 | 1192 |

Input-policy comparison is separated from model/backend comparison. All three
policies used the same BGE-small/FastEmbed model and the same LanceDB-backed
vector-corpus facade. On the default fixture, policy choice did not change
merged, ranked, visible, or hard-negative outcomes. `informal-or-formal` fell
back to formal statements because the current fixture documents do not carry
docstrings or informal text.

Hard-negative leakage did not increase at the visible stage on any completed
suite. Vector generation also did not generate labeled hard negatives in the
completed runs: `0/3` on default, `0/5` on hard-negatives, `0/8` on the
production-gate completed aggregate.

## Runtime And Cache Evidence

The first release-mode validation command included a one-time release build of
the LanceDB/FastEmbed stack, so its wall time is not a runtime measurement for
search. Search runtime claims below use the JSON runtime counters plus warm
`/usr/bin/time` runs after release build completion.

| Workload | Corpus status | Embedding ms | Corpus build ms | Query ms | Eval total ms | Time wall | Peak RSS from eval | Cache notes |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| default/name cold | built | 314 | 13 | 132 | 1480 | 5.46 s | 521,748,480 bytes | builds corpus |
| default/name warm | reused | 0 | 0 | 92 | 1137 | 4.72 s | 448,921,600 bytes | reuses corpus and text vectors |
| hard-negatives/name | built | 247 | 8 | 74 | 1448 | 4.96 s | 558,366,720 bytes | separate suite corpus |
| production-gate | child corpora reused/built as needed | child counters in artifacts | child counters in artifacts | child counters in artifacts | 2412 | 7.08 s | 558,170,112 bytes | manual children skipped |

Cache sizes after validation:

| Cache | Size |
| --- | ---: |
| Hugging Face model cache | 128 MiB |
| text-vector cache | 2.0 MiB |
| vector corpus cache | 180 KiB |

Warm-cache reproducibility was checked by comparing normalized pair rows for
`default-name-run1` and `default-name-run2`. Pair ordering, vector ranks, vector
scores, labels, and visibility facts were identical. The warm run reported
`corpus_status = "reused"`, `corpus_build_ms = 0`, and `embedding_ms = 0`.

## Manual Suites

The KanProofs workspace and mathlib checkout were present, but both manual
children skipped:

```text
index error: missing compiled oleans for index (5 missing; sample:
KanProofs.IUT.Foundation.Mutation.PositiveExistentialSolution,
KanProofs.ModelTheory.SetTheory.ZFC.Arithmetic.Int.Relation,
KanProofs.ModelTheory.SetTheory.ZFC.Arithmetic.Nat.Domain,
KanProofs.ModelTheory.SetTheory.ZFC.Grothendieck.Algebra.Ring,
KanProofs.ModelTheory.SetTheory.ZFC.Grothendieck.FiniteFold)
```

Skipped manual suites are not counted as passes. Because the manual suites did
not run, this validation cannot claim mathlib-scale vector retrieval quality.

## Boundary And Leak Evidence

Boundary tests passed for:

```sh
cargo test -p lean-dup-cli --test boundaries
```

The current allowed dependency/import shape is exact:

- embedding runtime dependencies remain inside `lean-dup-embedding`;
- vector database dependencies remain inside `lean-dup-vector-index`;
- `lean-dup-search` imports only crate-root embedding/vector-index APIs for the
  hidden vector policy;
- `lean-dup-eval` owns artifacts and labels;
- `lean-dup-report` projects status/path/stage facts only.

Artifact leak checks over `target/search-quality/vector-validation/*-vector-search.json`
found no backend names, raw Lean/source text, SQLite/posting vocabulary, worker
rows, absolute private paths, model input prefixes, vector database layout
vocabulary, tokenizer terms, tensor terms, or cache-root paths.

## Decision

Keep vector search hidden and off by default for further study.

Do not allow Prompt 36 to include vector candidate facts in threshold
calibration. The allow condition was not met: completed vector runs did not
improve merged, ranked, or visible recall on any labeled suite. They only showed
that vectors can generate some of the same positives symbolic retrieval already
finds.

Do not remove the experiment. The hidden path is bounded, privacy-safe in the
artifacts checked here, cache-only failure is deterministic, warm-cache reuse is
working, and visible hard-negative leakage did not increase on completed
fixtures. The next useful study is a true larger comparison corpus with compiled
KanProofs/mathlib oleans available, plus a fixture where symbolic retrieval is
known to miss a labeled positive that vector search should recover.

## Red Flag Review

- Shallow module: mitigated. Validation consumes stable facts from embedding,
  vector-index, search, and eval rather than new backend-specific surfaces.
- Pass-through wrapper: mitigated. The doc records a decision from denominators;
  it is not a restatement of command success.
- Temporal decomposition: mitigated. Build/query completion was explicitly not
  treated as promotion evidence.
- Information leakage: mitigated. Artifact leak checks passed, and backend names
  remain architecture evidence rather than search/report API.
- Special-general mixture: mitigated. Fixture/manual suite policy stays in eval;
  vector corpus mechanics stay in vector-index; model mechanics stay in
  embedding.
- Conjoined methods: mitigated. Search owns candidate policy while eval owns the
  decision, so eval does not reconstruct search.
- Hard-to-describe public API: acceptable. The public vocabulary is model
  profile, declaration document, vector corpus, and search stage facts.
- Implementation details contaminating interface comments: no new public
  interface comments were added in this validation doc.
