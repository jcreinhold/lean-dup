# Symbolic Scorer And Ablations

Prompt 33 makes ranking policy explicit without making it user-configurable. The default scorer records the current
symbolic behavior as a versioned Rust policy; ablations measure feature-family dependence before any default weight
change is accepted.

## Design Note

The scorer boundary owns ranking weights, cheap blocker interpretation, score thresholds, feature-family enablement,
scorer versioning, and ablation variants.

Its smallest public interface is a scorer summary in search/eval/report DTOs and hidden eval ablation artifacts. Normal
audit callers choose review profiles and probe controls; they do not choose scorer weights, config files, retrieval
keys, SQLite rows, Lean expressions, or worker transport details.

These decisions must not leak upward or sideways:

- weight values, threshold constants, and feature-family ablation masks;
- conversion from retrieval contributions and pair features into component scores;
- semantic-evidence rerank policy before Prompt 34 makes probe yield richer;
- artifact layout under `target/search-quality/`;
- private corpus paths and production-gate suite orchestration.

The preserved capability is read-only duplicate auditing and evaluation with the current default ranking behavior.
Prompt 33 adds observability and an experiment harness; it does not tune default ranking.

Python-era behavior intentionally discarded: ranking by scattered constants, explaining quality from terminal examples,
and changing defaults without labeled before/after evidence.

## Design It Twice

**Rejected: user-facing scorer configuration.** TOML or JSON weights would look professional, but it would expose an
unstable model before the feature set and match classes are calibrated. It would also make ordinary users responsible
for search policy that the system should own.

**Chosen: crate-private Rust scorer plus hidden ablations.** Search owns the symbolic model and exports only versioned
facts. Eval requests fixed variants through the search observation API and writes artifacts. This is deeper because
scoring internals can change while reports and CLI users keep one stable explanation: which scorer version and variant
produced the metrics.

## Scorer Contract

The scorer version is `lean-dup.symbolic-scorer.v1`. The default variant is `all-features` and preserves current
ranking behavior.

Supported variants:

- `all-features`;
- `no-role-features`;
- `no-connective-conclusion-features`;
- `no-source-module-features`;
- `no-static-evidence-features`;
- `semantic-evidence-only-rerank`.

The scorer consumes stable pair-feature facts: feature-family names, declaration kinds, evidence mode, structural
fingerprint family matches, role-overlap counts, module relation, semantic evidence state, and cheap blockers. It must
not consume SQLite rows, posting records, raw Lean expressions, source text, worker rows, or CLI flags.

Default weights are encoded as crate-private Rust constants. A later prompt may replace or tune those constants only
with before/after evidence over fixture and production-gate suites.

## Ablation Contract

The hidden command is:

```sh
cargo run -p lean-dup-cli -- eval --suite default --format json --write-scorer-ablations
```

It writes:

```text
target/search-quality/<suite>-scorer-ablations.json
```

The artifact schema version is `lean-dup.scorer-ablation.v1`. Each variant reports:

- status;
- recall;
- visible-queue precision;
- hard-negative hits and survival;
- candidate count and stage metrics;
- timing and peak RSS fields.

Production-gate ablation artifacts include aggregate variant metrics and child-suite variant metrics for completed
children. Skipped children remain skipped and are not counted as quality passes.

## Current Evidence

Prompt 33 does not accept a default weight change. The evidence commands are:

```sh
cargo run -p lean-dup-cli -- eval --suite default --format json --write-scorer-ablations
cargo run -p lean-dup-cli -- eval --suite production-gate --format json \
  --write-scorer-ablations --output target/eval/prompt33-production-gate.json
```

The default `all-features` row matches normal eval metrics. Other rows explain which feature families currently carry
positives, hard negatives, and visible findings; they are diagnostic, not release gates.

Fast fixture evidence from Prompt 33:

| Variant | Recall@10 | Visible precision | Hard-negative hits | Candidate count |
| --- | ---: | ---: | ---: | ---: |
| `all-features` | 16/16 | 14/34 | 0/3 | 299 |
| `no-role-features` | 16/16 | 15/82 | 2/3 | 299 |
| `no-connective-conclusion-features` | 16/16 | 13/32 | 0/3 | 299 |
| `no-source-module-features` | 16/16 | 15/82 | 2/3 | 299 |
| `no-static-evidence-features` | 16/16 | 15/82 | 2/3 | 299 |
| `semantic-evidence-only-rerank` | 0/16 | 0/0 | 0/3 | 0 |

Production-gate evidence from Prompt 33 completed the fast child suites but skipped the KanProofs manual suites because
the local KanProofs workspace was missing compiled oleans:

| Variant | Recall@10 | Visible precision | Hard-negative hits | Candidate count |
| --- | ---: | ---: | ---: | ---: |
| `all-features` | 17/17 | 15/68 | 0/8 | 598 |
| `no-role-features` | 17/17 | 16/164 | 3/8 | 598 |
| `no-connective-conclusion-features` | 17/17 | 14/64 | 0/8 | 598 |
| `no-source-module-features` | 17/17 | 16/164 | 3/8 | 598 |
| `no-static-evidence-features` | 17/17 | 16/164 | 3/8 | 598 |
| `semantic-evidence-only-rerank` | 0/17 | 0/0 | 0/8 | 0 |

These numbers are evidence for the ablation harness and default behavior preservation, not evidence that the search
quality gates are closed.

## Red Flag Review

- **Shallow module:** mitigated. The scorer centralizes weights, thresholds, versioning, and ablation masks rather than
  leaving constants scattered through ranking and replacement hints.
- **Pass-through wrapper:** avoided. Eval artifacts do not merely echo metrics; they run fixed scorer variants and
  expose per-variant denominators.
- **Temporal decomposition:** mitigated. Scoring policy is organized by feature families and evidence facts, not by the
  order in which retrieval, ranking, and reporting happen.
- **Information leakage:** mitigated. Public DTOs expose version and variant only; artifacts use stable feature-family
  names, not raw retrieval keys, SQLite/posting vocabulary, Lean expressions, source text, or worker rows.
- **Special-general mixture:** contained. KanProofs production-gate data uses the same ablation schema as fixtures.
- **Conjoined methods:** mitigated. Search owns scoring; eval owns label joins and artifact writing; report owns
  projection and wording.
- **Hard-to-describe public API:** mitigated. The normal user API is unchanged, with one hidden eval flag for raw
  ablation evidence.
- **Implementation details contaminating interface comments:** mitigated. Interface comments describe scorer facts and
  variant semantics, not storage layout or temporary migration mechanics.
