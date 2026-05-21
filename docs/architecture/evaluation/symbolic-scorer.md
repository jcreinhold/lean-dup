# Symbolic Scorer And Ablations

Ranking policy is explicit but not user-configurable. The default scorer records current symbolic behavior as a
versioned Rust policy; ablation variants measure feature-family dependence before any default weight change is
accepted.

The scorer is crate-private Rust. Search exports only versioned facts; eval requests fixed variants through the
search observation API and writes artifacts. A user-facing TOML/JSON config was rejected: it would expose an unstable
model before the feature set and match classes are calibrated, and it would push search policy onto users.

## Contract

Scorer version: `lean-dup.symbolic-scorer.v2`. Default variant: `all-features` (the ordinary calibrated symbolic
scorer).

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

## Evidence commands

```sh
cargo run -p lean-dup-cli -- eval --suite default --format json --write-scorer-ablations
cargo run -p lean-dup-cli -- eval --suite production-gate --format json \
  --write-scorer-ablations --output target/eval/production-gate.json
```

The `all-features` row of each artifact matches the normal eval metrics; other rows are
diagnostic, naming which feature families currently carry positives, hard negatives, and
visible findings. The ablations are not release gates.

`symbolic-only` remains a hidden vector-validation comparison label. It is not the ordinary symbolic eval baseline and
is intentionally omitted from the symbolic ablation artifact set.
