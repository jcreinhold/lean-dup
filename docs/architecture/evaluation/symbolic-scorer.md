# Symbolic Scorer And Ablations

Ranking policy is explicit but not user-configurable. The default scorer records current symbolic behavior as a
versioned Rust policy; ablation variants measure feature-family dependence before any default weight change is
accepted.

The scorer is crate-private Rust. Search exports only versioned facts; eval requests fixed variants through the
search observation API and writes artifacts. A user-facing TOML/JSON config was rejected: it would expose an unstable
model before the feature set and match classes are calibrated, and it would push search policy onto users.

## Contract

Scorer version: `lean-dup.symbolic-scorer.v1`. Default variant: `all-features` (preserves current ranking).

Supported variants:

- `all-features`
- `no-role-features`
- `no-connective-conclusion-features`
- `no-source-module-features`
- `no-static-evidence-features`
- `semantic-evidence-only-rerank`

Inputs are stable pair-feature facts: feature-family names, declaration kinds, evidence mode, structural-fingerprint
family matches, role-overlap counts, module relation, semantic-evidence state, cheap blockers. **Not** SQLite rows,
posting records, raw Lean expressions, source text, worker rows, or CLI flags.

Default weights are crate-private Rust constants. Changes require before/after evidence over fixture and
production-gate suites.

## Ablations

```sh
cargo run -p lean-dup-cli -- eval --suite default --format json --write-scorer-ablations
# writes target/search-quality/<suite>-scorer-ablations.json
```

Artifact schema: `lean-dup.scorer-ablation.v1`. Each variant reports status, recall, visible-queue precision,
hard-negative hits and survival, candidate count, stage metrics, timing, peak RSS.

Production-gate ablation artifacts include aggregate variant metrics and child-suite variant metrics for completed
children. Skipped children stay skipped and are not counted as quality passes.

## Evidence

Default fixture suite:

| Variant | Recall@10 | Visible precision | Hard-neg hits | Candidates |
| --- | ---: | ---: | ---: | ---: |
| `all-features` | 16/16 | 14/34 | 0/3 | 299 |
| `no-role-features` | 16/16 | 15/82 | 2/3 | 299 |
| `no-connective-conclusion-features` | 16/16 | 13/32 | 0/3 | 299 |
| `no-source-module-features` | 16/16 | 15/82 | 2/3 | 299 |
| `no-static-evidence-features` | 16/16 | 15/82 | 2/3 | 299 |
| `semantic-evidence-only-rerank` | 0/16 | 0/0 | 0/3 | 0 |

Production-gate. Completed the fast children; skipped the KanProofs manual suites because the local
KanProofs workspace was missing compiled oleans:

| Variant | Recall@10 | Visible precision | Hard-neg hits | Candidates |
| --- | ---: | ---: | ---: | ---: |
| `all-features` | 17/17 | 15/68 | 0/8 | 598 |
| `no-role-features` | 17/17 | 16/164 | 3/8 | 598 |
| `no-connective-conclusion-features` | 17/17 | 14/64 | 0/8 | 598 |
| `no-source-module-features` | 17/17 | 16/164 | 3/8 | 598 |
| `no-static-evidence-features` | 17/17 | 16/164 | 3/8 | 598 |
| `semantic-evidence-only-rerank` | 0/17 | 0/0 | 0/8 | 0 |

The `all-features` row matches normal eval metrics. Other rows are diagnostic: they show which feature families
currently carry positives, hard negatives, and visible findings. They are not release gates.

The numbers above are evidence that the ablation harness works and that default behavior is preserved. They are not
evidence that the search-quality gates are closed.

Evidence commands:

```sh
cargo run -p lean-dup-cli -- eval --suite default --format json --write-scorer-ablations
cargo run -p lean-dup-cli -- eval --suite production-gate --format json \
  --write-scorer-ablations --output target/eval/prompt33-production-gate.json
```
