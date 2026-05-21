# Symbolic Precision And Threshold Calibration

Date: 2026-05-21

## Design Note

Search owns the calibrated symbolic review policy: which retrieved pairs are actionable, which
remain diagnostics, and which blockers prevent default visibility. Eval owns label truth,
threshold-sweep denominators, manual-suite status, and before/after measurement. Report owns the
bounded projection of search decisions; it records the scorer and review-policy versions but does
not recompute visibility. CLI owns review-profile selection and artifact paths.

The smallest public interface is:

- `scorer_version = lean-dup.symbolic-scorer.v1`;
- `review_policy_version = lean-dup.symbolic-review-policy.v2`;
- raw eval denominators for generated, ranked, and visible stages;
- bounded audit queue counts and hidden-reason summaries.

Scorer weights, feature keys, exact blocker predicates, fixture shortcuts, private manual-corpus
paths, report rendering choices, and retrieval storage details must not leak upward or sideways.
The preserved user-facing capability is the ordinary symbolic audit/eval workflow with bounded,
explainable output. The discarded Python-era behavior is treating every static structural hit as
default actionable evidence and relying on large unfiltered reports as a review queue.

Prompt 45 has not produced
`docs/architecture/evaluation/semantic-theorem-profile-validation-decision.md` in this checkout.
Semantic/vector facts are ignored for this calibration.

## Design It Twice

Three designs were considered.

1. **Raise global thresholds.** Rejected. A single score threshold would hide symptoms without
   expressing why private helpers, low-signal broad-shape matches, static definition pairs, and
   theorem statement duplicates deserve different treatment.
2. **Remove noisy feature families from reports.** Rejected. Those features are still useful for
   candidate generation and diagnostics; deleting them would weaken recall evidence and obscure
   generated-stage denominators.
3. **Make search own a versioned review policy.** Chosen. The policy keeps public theorem
   statement/permutation evidence visible by default, blocks private/generated/low-signal/static
   non-theorem pairs from default actionability, and leaves broad diagnostics available through
   explicit non-default profiles.

The chosen boundary is deeper because eval measures truth rather than implementing visibility,
report projects search-owned decisions rather than reconstructing them, and callers only learn a
stable policy id plus denominators.

## Calibrated Policy

`lean-dup.symbolic-review-policy.v2` keeps default symbolic visibility narrow:

- visible by default: public theorem-like pairs with statement or safe-permutation evidence;
- hidden by default: generated declarations, non-public declarations, low-signal declarations,
  broad-head-only matches, typeclass-instance noise, and static non-theorem pairs without
  proof-grade/source-clone evidence;
- diagnostic: connective/conclusion/role-shape and static definition similarities remain generated
  and ranked evidence but do not enter the default cleanup queue by themselves.

Search records the policy id in eval/search-dataset/audit facts. Report records it without
recomputing the policy.

## Threshold Sweep Evidence

Commands:

```sh
cargo run -p lean-dup-cli -- eval --suite default --format json --write-scorer-ablations \
  --write-search-dataset --output target/eval/default-after-54.json
cargo run -p lean-dup-cli -- eval --suite hard-negatives --format json --write-scorer-ablations \
  --output target/eval/hard-negatives-after-54.json
cargo run -p lean-dup-cli -- eval --suite production-gate --format json --write-scorer-ablations \
  --output target/eval/production-gate-after-54.json
cargo run -p lean-dup-cli -- eval --suite manual-internal \
  --workspace /Users/jcreinhold/Code/kan-proofs --manual-module KanProofs --format json \
  --output target/eval/manual-internal-after-54.json
```

Before artifacts:

- `target/eval/default-before-54.json`
- `target/eval/hard-negatives-before-54.json`
- `target/eval/production-gate-before-54.json`

After artifacts:

- `target/eval/default-after-54.json`
- `target/eval/hard-negatives-after-54.json`
- `target/eval/production-gate-after-54.json`
- `target/eval/manual-internal-after-54.json`
- `target/search-quality/default-scorer-ablations.json`
- `target/search-quality/hard-negatives-scorer-ablations.json`
- `target/search-quality/production-gate-scorer-ablations.json`
- `target/search-quality/default-dataset.json`

Fast-suite before/after:

| Suite | Policy | Recall@10 | Visible precision | Visible positives | Visible groups | Hard-negative visible |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| `default` | old symbolic shown filter | 16/16 | 14/34 | 14/16 | 25/39 | 0/3 |
| `default` | `lean-dup.symbolic-review-policy.v2` | 16/16 | 8/8 | 8/16 | 7/39 | 0/3 |
| `hard-negatives` | old symbolic shown filter | 1/1 | 1/34 | 1/1 | 25/39 | 0/5 |
| `hard-negatives` | `lean-dup.symbolic-review-policy.v2` | 1/1 | 1/8 | 1/1 | 7/39 | 0/5 |
| `production-gate` | old symbolic shown filter | 17/17 | 15/68 | 15/17 | 50/78 | 0/8 |
| `production-gate` | `lean-dup.symbolic-review-policy.v2` | 17/17 | 9/16 | 9/17 | 14/78 | 0/8 |

Stage-level hard-negative survival after calibration:

| Suite | Generated | Ranked | Visible |
| --- | ---: | ---: | ---: |
| `default` | 3/3 | 3/3 | 0/3 |
| `hard-negatives` | 2/5 | 2/5 | 0/5 |
| `production-gate` | 5/8 | 5/8 | 0/8 |

The production-gate artifact remains `status = incomplete` because manual suites without operator
paths are skipped; skipped manual suites are not counted as passes.

Manual KanProofs internal evidence:

| Suite | Status | Recall@10 | Visible precision | Visible positives | Visible groups | Hard-negative visible | Runtime | Peak RSS |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `manual-internal` | ok | 0/6 | 0/4 | 0/6 | 8/7819 | 0/3 | 28.363 s | 6,599,884,800 bytes |

This is not release-quality evidence. It shows the calibrated policy greatly narrows the queue, but
the labeled manual positives are still missed. Prompt 55 and later production-readiness prompts must
treat this as a blocker, not as a pass.

## Red Flag Review

- Shallow module: avoided. The new review-policy module hides blocker and visibility decisions
  behind one stable policy id.
- Pass-through wrapper: avoided. Eval/report do not forward a new wrapper; they record policy facts
  alongside existing denominators.
- Temporal decomposition: avoided. Visibility policy is organized around actionability facts, not
  execution order.
- Information leakage: avoided. No retrieval keys, feature weights, private paths, or report
  rendering internals are part of the policy interface.
- Special-general mixture: acceptable. Fixture evidence uses the same search policy as ordinary
  eval; fixture-specific labels stay in eval.
- Conjoined methods: avoided. Eval scoring remains understandable without reading report projection.
- Hard-to-describe public API: avoided. The public fact is a policy version plus raw denominators.
- Implementation details contaminating interface comments: avoided. Public comments describe stable
  review-policy facts, not blocker implementation.

## Decision

Use `lean-dup.symbolic-review-policy.v2` as the default symbolic visibility policy for continued
0.1.0 readiness work. It improves fast-suite default precision and preserves recall@10 and
zero hard-negative leakage. It does not close production readiness because manual internal evidence
still misses labeled positives.
