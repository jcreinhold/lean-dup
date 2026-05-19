# Embedding Validation

Prompt 35E decides whether the hidden embedding rerank experiment should be removed,
kept hidden, or allowed into Prompt 36 threshold calibration. The decision here is:
**keep hidden/off-default for further study**. The implementation works and stays behind
the intended boundaries, but the prepared model sharply regresses fixture recall over the
observed candidate pool.

The prompt text referred to `docs/architecture/04-production-readiness.md`. The current
repo has the same release-gate document at `docs/architecture/production-readiness.md`,
which is the path linked by `docs/architecture/README.md`.

## Design Note

Hidden knowledge: eval owns validation lifecycle, label joining, artifact collection,
quality comparison, and the go/no-go decision. Search owns declaration-summary input
construction. The embedding crate owns model acquisition, CPU inference, vector caching,
and runtime/cache counters. Report only projects optional artifact status/path fields.

Smallest public interface: the existing hidden eval request writes embedding-rerank
artifacts. No new public API is added for validation.

Decisions that must not leak upward or sideways: tokenizer internals, tensor layout,
model filenames, Hugging Face snapshot layout, vector-cache filenames, raw Lean/source
text, SQLite details, retrieval keys, and worker rows.

Preserved capability: default symbolic duplicate auditing remains authoritative.
Normal audit and ordinary eval do not download, load, or require embedding models.

Discarded Python-era behavior: successful model inference or anecdotal semantic-search
inspection is not enough. Embeddings must be judged by labeled metrics, runtime/RSS/cache
cost, reproducibility, and hard-negative leakage.

## Design It Twice

Rejected: treat successful model inference as enough to keep or promote embeddings. That
would validate the runtime, not the search-quality value. It would also hide recall loss
behind the fact that vectors were produced.

Chosen: decide from labeled search-quality metrics plus runtime/RSS/cache evidence. This
is deeper because the validation boundary exposes one stable decision and keeps model
mechanics, search inputs, labels, and report projection in their owning crates.

## Evidence

Model preparation used the explicit hidden command:

```sh
cargo run -p lean-dup-cli -- embedding prepare \
  --policy download-if-missing \
  --format json \
  --cache-root target/search-quality/embedding-validation/hf-cache
```

Result: `status = ok`, `cache_status = prepared`, model
`sentence-transformers/all-MiniLM-L6-v2`, downloaded bytes `91,335,887`, elapsed
`3,433 ms`.

Cache sizes after validation:

| Cache | Size |
| --- | ---: |
| Hugging Face model cache | 97 MiB |
| Vector cache | 1.1 MiB |

The leak check over `target/search-quality/embedding-validation/*embedding-rerank.json`
found no forbidden tokenizer, tensor, model-file, snapshot, SQLite/posting, raw path,
statement-text, worker-row, `FeatureMatch`, or `IndexQuery` strings.

| Workload | Status | Symbolic baseline | Embedding rerank | Runtime / RSS | Result |
| --- | --- | --- | --- | --- | --- |
| fake backend unit test | passed | n/a | deterministic normalized vectors | n/a | runtime math works without a real model |
| fake rerank builder | passed | n/a | uses symbolic shown budget and typed labels | n/a | artifact builder contract works |
| default, cache-only empty cache | skipped | recall@1/5/10 `7/16`, `16/16`, `16/16`; precision `14/34`; hard negatives `0/3`; candidates `299` | none | n/a | missing model is non-fatal and deterministic |
| default, prepared run 1 | ok | recall@1/5/10 `7/16`, `16/16`, `16/16`; precision `14/34`; hard negatives `0/3`; candidates `299`; scorer `lean-dup.symbolic-scorer.v1` | recall@1/5/10 `1/16`, `2/16`, `3/16`; precision `11/17`; hard negatives `0/3`; candidates `299` | `996 ms`, RSS `310,722,560` bytes | recall regression |
| default, prepared warm repeat | ok | same symbolic denominators; eval timing changed with cache warmth | same quality metrics and pair ordering as run 1 | `0 ms` embedding runtime from vector-cache hits, RSS `101,269,504` bytes | reproducible quality; runtime differs as expected |
| hard-negatives, prepared | ok | recall@1/5/10 `0/1`, `1/1`, `1/1`; precision `1/34`; hard negatives `0/5`; candidates `299` | recall@1/5/10 `0/1`, `0/1`, `0/1`; precision `0/17`; hard negatives `0/5`; candidates `299` | `1,017 ms`, RSS `314,949,632` bytes | recall regression; no hard-negative leakage |
| production-gate with KanProofs workspace | incomplete | aggregate over completed fast children: recall@1/5/10 `7/17`, `17/17`, `17/17`; precision `15/68`; hard negatives `0/8`; candidates `598` | child artifacts only; aggregate omitted embedding metrics because manual children skipped | default child `998 ms`; hard-negative child cache-hit runtime `0 ms` | manual children not counted |

Manual suite blocker:

```text
index error: missing compiled oleans for index (1 missing; sample:
KanProofs.AlgebraicGeometry.EllipticCurve.DivisionPolynomial.Multiplication)
```

Skipped manual children are not counted as quality passes.

Artifact locations:

- `target/search-quality/embedding-validation/default-skipped-embedding-rerank.json`
- `target/search-quality/embedding-validation/default-prepared-run1-embedding-rerank.json`
- `target/search-quality/embedding-validation/default-prepared-run2-embedding-rerank.json`
- `target/search-quality/embedding-validation/hard-negatives-prepared-embedding-rerank.json`
- `target/search-quality/embedding-validation/production-gate-embedding-rerank.json`
- `target/eval/embedding-validation-production-gate.json`

## Boundary Audit

`cargo test -p lean-dup-cli --test boundaries` passed. The enforced state is:

- `hf-hub`, `tokenizers`, Candle crates, and `safetensors` are dependencies only of
  `lean-dup-embedding`;
- `lean-dup-embedding` depends only on `lean-dup-diagnostics` among product crates;
- `lean-dup-cli` may import embedding crate-root APIs for hidden prepare/eval plumbing;
- `lean-dup-eval` may import embedding crate-root APIs for the hidden rerank experiment;
- search and report do not depend on `lean-dup-embedding`;
- no embedding implementation modules are public cross-crate APIs.

`crate-factoring.md` and `embedding-architecture.md` already describe the nine-crate
hidden/off-default architecture and match the observed boundary tests.

## Decision

Keep the embedding rerank experiment hidden/off-default for further study. Do not let
Prompt 36 use embedding facts for threshold calibration.

Reasons:

- The prepared model is reproducible over the default fixture repeat, and runtime/cache
  costs are within the threshold (`< 2 min`, `< 2 GiB`, `< 500 MiB`).
- The experiment does not leak hard negatives in the completed suites.
- However, it regresses recall from `16/16` at recall@10 to `3/16` on the default suite,
  and from `1/1` to `0/1` on the hard-negative suite's positive denominator.
- The production-gate manual children were skipped because a required KanProofs `.olean`
  is missing, so they cannot provide evidence for promotion.

The experiment should not be removed because the hidden path, cache-only skip behavior,
runtime, artifact writing, privacy checks, and boundaries all work. It also should not be
used for calibration yet because the current local sentence-transformer signal is weaker
than the symbolic scorer over the observed candidate pool.

## Red Flag Review

- Shallow module: mitigated. Validation produces one decision from stable artifacts; it
  does not add a new API.
- Pass-through wrapper: mitigated. Eval joins labels, compares metrics, records blockers,
  and writes decision evidence.
- Temporal decomposition: mitigated. The validation follows owned responsibilities rather
  than exposing prepare, embed, score, and report steps as a public workflow.
- Information leakage: mitigated. Leak checks over artifacts passed, and boundary tests
  keep runtime dependencies and embedding imports narrow.
- Special-general mixture: mitigated. The result remains a hidden lean-dup experiment.
- Conjoined methods: mitigated. Embedding validation does not alter symbolic ranking,
  candidate generation, semantic probes, or report semantics.
- Hard-to-describe public API: mitigated. No public API was added.
- Implementation details contaminating interface comments: mitigated. The decision doc
  records stable evidence and keeps tokenizer/model/cache internals private.

