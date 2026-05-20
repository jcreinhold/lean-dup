# Embedding Validation

**Decision: the embedding rerank experiment stays hidden and off by default.** The prepared
model regresses recall@10 from 16/16 to 3/16 on the default fixture and from 1/1 to 0/1 on
the hard-negative fixture. The experiment must not feed threshold calibration.

The experiment is retained, not removed: the hidden path, cache-only skip behavior, runtime,
artifact writing, leak checks, and crate boundaries all work. The signal from the current
local sentence-transformer is weaker than the symbolic scorer over the observed candidate
pool. The historical model record below is for an earlier MiniLM/Candle implementation;
the current default profile is BGE-small through FastEmbed (see
[embedding-architecture.md](embedding-architecture.md)).

## Evidence

Model preparation:

```sh
cargo run -p lean-dup-cli -- embedding prepare \
  --policy download-if-missing \
  --format json \
  --cache-root target/search-quality/embedding-validation/hf-cache
```

Result: `status = ok`, `cache_status = prepared`, model `sentence-transformers/all-MiniLM-L6-v2`,
downloaded `91.3 MB` in `3.43 s`. After validation: 97 MiB Hugging Face cache, 1.1 MiB vector
cache. The leak check over `target/search-quality/embedding-validation/*embedding-rerank.json`
found no forbidden tokenizer, tensor, model-file, snapshot, SQLite/posting, raw path,
statement-text, worker-row, `FeatureMatch`, or `IndexQuery` strings.

### Quality, per workload

Denominators are positive pairs (for recall) and observed candidate pairs (for precision).
Symbolic baseline is `lean-dup.symbolic-scorer.v1`; embedding rerank uses the same observed
pool.

| Workload                       | Status     | Symbolic R@1 | R@10  | Sym P  | Emb R@1 | R@10 | Emb P  | Hard neg leaked |
| ------------------------------ | ---------- | -----------: | ----: | -----: | ------: | ---: | -----: | --------------: |
| default, prepared run 1        | ok         |         7/16 | 16/16 |  14/34 |    1/16 | 3/16 |  11/17 |             0/3 |
| default, prepared warm repeat  | ok         |         7/16 | 16/16 |  14/34 |    1/16 | 3/16 |  11/17 |             0/3 |
| hard-negatives, prepared       | ok         |          0/1 |   1/1 |   1/34 |     0/1 |  0/1 |   0/17 |             0/5 |
| production-gate, fast children | incomplete |         7/17 | 17/17 |  15/68 |       — |    — |      — |             0/8 |
| default, cache-only empty      | skipped    |         7/16 | 16/16 |  14/34 |       — |    — |      — |               — |

Run 2 reproduced run 1's ranks and labels exactly with `embedding_ms = 0` from cache hits.
The production-gate row aggregates only completed fast children; the embedding column is
omitted because the manual children skipped (see below). Fake-backend and fake-builder unit
tests pass and confirm the deterministic-vector contract independent of any real model.

### Runtime and cost

Per prepared run: ≤ 1.02 s wall, RSS 310-315 MB cold and 101 MB warm. All workloads stay
under the 2 min wall / 2 GiB RSS / 500 MiB on-disk cache ceiling.

### Manual suite blocker

The production-gate manual children require KanProofs `.olean` files that are not built:

```text
index error: missing compiled oleans for index (1 missing; sample:
KanProofs.AlgebraicGeometry.EllipticCurve.DivisionPolynomial.Multiplication)
```

Skipped manual children do not count as passes; the production-gate row above cannot speak
to promotion.

### Artifacts

```
target/search-quality/embedding-validation/default-skipped-embedding-rerank.json
target/search-quality/embedding-validation/default-prepared-run1-embedding-rerank.json
target/search-quality/embedding-validation/default-prepared-run2-embedding-rerank.json
target/search-quality/embedding-validation/hard-negatives-prepared-embedding-rerank.json
target/search-quality/embedding-validation/production-gate-embedding-rerank.json
target/eval/embedding-validation-production-gate.json
```

## Boundary audit

`cargo test -p lean-dup-cli --test boundaries` passes. The enforced shape:

- `hf-hub`, `tokenizers`, Candle crates, and `safetensors` are dependencies of
  `lean-dup-embedding` only;
- `lean-dup-embedding` depends only on `lean-dup-diagnostics` among product crates;
- `lean-dup-cli` and `lean-dup-eval` may import crate-root APIs for hidden plumbing;
- `lean-dup-search` and `lean-dup-report` do not depend on `lean-dup-embedding`;
- no embedding implementation modules are public cross-crate APIs.

[crate-factoring.md](../crate-factoring.md) and
[embedding-architecture.md](embedding-architecture.md) describe the full nine-crate
architecture.

## What "keep hidden" requires

- The hidden path stays cache-only safe: a missing model is non-fatal and produces a
  `status = skipped` artifact, not a crash.
- The artifact leak rules in [embedding-rerank-experiment.md](embedding-rerank-experiment.md)
  remain enforced.
- Threshold calibration must not use embedding facts until a labeled run on real mathlib
  oleans shows recall parity or gain.

## Hidden knowledge boundary

Eval owns validation lifecycle, label joining, artifact collection, quality comparison, and
the go/no-go decision. Search owns declaration-document construction. The embedding crate
owns model acquisition, CPU inference, vector caching, and runtime counters. Report only
projects optional artifact status/path fields. Tokenizer internals, tensor layout, model
filenames, snapshot layout, vector-cache filenames, raw source text, SQLite details,
retrieval keys, and worker rows do not cross those boundaries.

## Red-flag checklist

- *Shallow module:* the validation produces one decision from stable artifacts; it adds no
  public API.
- *Pass-through wrapper:* eval joins labels, compares metrics, records blockers, and writes
  decision evidence.
- *Temporal decomposition:* validation follows owned responsibilities, not a public
  prepare-embed-score-report workflow.
- *Information leakage:* artifact leak checks pass; boundary tests keep dependencies narrow.
- *Special-general mixture:* the result remains a hidden lean-dup experiment.
- *Conjoined methods:* embedding validation does not alter symbolic ranking, candidate
  generation, semantic probes, or report semantics.
- *Hard-to-describe public API:* none was added.
- *Implementation details in interface comments:* the decision doc records stable evidence;
  tokenizer/model/cache internals remain private.
