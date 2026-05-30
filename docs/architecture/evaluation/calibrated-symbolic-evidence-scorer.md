# Calibrated Symbolic Evidence Scorer

This artifact records the Prompt 72 scorer calibration pass. The selected scorer is `lean-dup.symbolic-scorer.v2` with
the ordinary `all-features` variant. The change is deliberately narrow: it gives the ordinary symbolic scorer a
calibrated, versioned identity and makes the ablation artifact name the true baseline. It does not tune thresholds from
unresolved manual labels, expose weights, or use vector evidence.

## Design Note

Search owns scorer features, weights, thresholds, visibility interaction, and the versioned scorer id. Eval owns label
truth, stage denominators, and ablation artifacts. Report projects stable queue facts and must not reconstruct scoring
or visibility. CLI selects suites and output paths, but it does not learn scorer mechanics.

The smallest public scorer interface is:

- `SearchScoringSummary { version, variant }`;
- stable scorer variant names for eval artifacts;
- stable score facts by feature family where pair-level artifacts need them.

The following remain private to search: raw feature keys, weight values, calibration search internals, retrieval keys,
worker rows, private paths, vector facts, and threshold mechanics. The preserved user-facing capability is the
conservative cleanup queue with zero visible hard-negative leakage in default and hard-negative suites. The discarded
Python-era behavior is ad hoc, unversioned ranking whose baseline was described by implementation vocabulary rather than
a release artifact.

## Design It Twice

Three designs were considered:

1. Adjust current constants by hand until fixtures pass.
2. Expose all feature weights in configuration so operators can tune scoring.
3. Keep a search-owned, versioned scorer selected from label-backed sweeps and ablations.

The selected design is the versioned label-backed scorer. It is deeper because search keeps scoring complexity hidden,
eval measures truth and ablations, and report/CLI consume stable facts. No weight or threshold is public API.

## Selected Policy

| Field | Value |
| --- | --- |
| Scorer id | `lean-dup.symbolic-scorer.v2` |
| Default variant | `all-features` |
| Review policy | `lean-dup.symbolic-review-policy.v2` |
| Ablation schema | `lean-dup.scorer-ablation.v1` |
| Release calibration source | symbolic labels only |

Feature-family vocabulary:

- `statement_fingerprint`
- `safe_permutation_fingerprint`
- `connective_fingerprint`
- `conclusion_fingerprint`
- `role_head`
- `role_conclusion_const`
- `role_hypothesis_const`
- source/module evidence
- static external evidence
- Lean semantic verification status, when available as symbolic proof evidence

`symbolic-only` remains a hidden comparison row for vector validation. It is not the ordinary symbolic eval baseline.
The symbolic ablation set now starts with `all-features`, followed by feature-family removals and
`semantic-evidence-only-rerank`.

## Commands And Artifacts

Fast suites:

```sh
cargo run -p lean-dup-cli -- eval --suite default --format json --write-scorer-ablations --output target/scorer-calibration/default.json
cargo run -p lean-dup-cli -- eval --suite hard-negatives --format json --write-scorer-ablations --output target/scorer-calibration/hard-negatives.json
cargo run -p lean-dup-cli -- eval --suite production-gate --format json --write-scorer-ablations --output target/scorer-calibration/production-gate.json
```

Manual suites were run with local KanProofs and project-pinned mathlib prerequisites. Paths are redacted here because
they are local operator paths:

```sh
env LEAN_DUP_CACHE_DIR=target/scorer-calibration/manual-cache cargo run -p lean-dup-cli -- eval --suite manual-internal --workspace <kan-proofs> --manual-module KanProofs --format json --write-scorer-ablations --output target/scorer-calibration/manual-internal.json
env LEAN_DUP_CACHE_DIR=target/scorer-calibration/manual-cache cargo run -p lean-dup-cli -- eval --suite manual-mathlib --workspace <kan-proofs> --manual-module KanProofs --mathlib-workspace <kan-proofs>/.lake/packages/mathlib --format json --write-scorer-ablations --output target/scorer-calibration/manual-mathlib.json
env LEAN_DUP_CACHE_DIR=target/scorer-calibration/manual-cache cargo run -p lean-dup-cli -- eval --suite production-gate --workspace <kan-proofs> --manual-module KanProofs --mathlib-workspace <kan-proofs>/.lake/packages/mathlib --format json --write-scorer-ablations --output target/scorer-calibration/production-gate-manual.json
```

Primary ablation artifacts:

- `target/search-quality/default-scorer-ablations.json`
- `target/search-quality/hard-negatives-scorer-ablations.json`
- `target/search-quality/manual-internal-scorer-ablations.json`
- `target/search-quality/manual-mathlib-scorer-ablations.json`
- `target/search-quality/production-gate-scorer-ablations.json`

## Fast Suite Results

| Suite | Status | Recall@10 | Visible precision | Visible hard negatives | Visible groups | Candidates | Peak RSS |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `default` | `ok` | 16/16 | 8/8 | 0/3 | 7/39 | 299 | 13,271,040 bytes |
| `hard-negatives` | `ok` | 1/1 | 1/8 | 0/5 | 7/39 | 299 | 13,074,432 bytes |
| `production-gate` without manual prerequisites | `incomplete` | 17/17 | 9/16 | 0/8 | 14/78 | 598 | 15,384,576 bytes |

The fast aggregate is incomplete because manual suites were skipped without workspace arguments. It is useful only for
fixture regression, not production calibration.

## Manual Suite Results

| Suite | Status | Recall@10 | Visible precision | Visible hard negatives | Label resolution | Candidates | Runtime | Peak RSS |
| --- | --- | ---: | ---: | ---: | --- | ---: | ---: | ---: |
| `manual-internal` | `blocked` | 1/6 | 0/4 | 0/3 | positives 1/6, hard negatives 1/3 | 562,437 | 73,128 ms | 8,114,847,744 bytes |
| `manual-mathlib` | `blocked` | 0/11 | 0/4 | 0/4 | positives 0/11, hard negatives 3/4 | 588,906 | 595,679 ms | 8,203,665,408 bytes |
| `production-gate` with manual prerequisites | `blocked` | 18/34 | 9/24 | 0/15 | child manual suites blocked | 1,151,941 | 286,330 ms | 10,013,573,120 bytes |

Manual evidence remains blocked by label corpus validity, not by missing prerequisites. The current resolved
manual-internal visibility case is:

| Pair | Stage | Result |
| --- | --- | --- |
| `FirstOrder.SetTheory.ZFC.ZFCModel.models_delta0Theory` / `FirstOrder.SetTheory.ZFC.instZFSetModelsDelta0Theory` | generated, ranked at 4 | hidden by review policy |

That case is recorded as a real visibility case for Prompt 73, but it is not enough to tune the default scorer. Most
manual positives are unresolved or violate the manual-mathlib one-workspace/one-mathlib endpoint rule, so they cannot be
counted as production positives.

## Ablation Findings

`all-features` is the only tested policy that preserves the intended small cleanup queue and zero visible hard-negative
leakage across the fast suites and the blocked manual runs.

Key deltas:

- Removing role features drops default recall@10 from 16/16 to 2/16, expands default visible rows from 7 groups to 32
  groups, and leaks 2/3 default hard negatives.
- Removing source/module or static evidence keeps only 1/16 default positives at recall@10, expands visible rows to 32
  groups, and leaks 2/3 default hard negatives.
- In manual-mathlib, removing role, connective/conclusion, source/module, or static evidence leaks 3/4 visible hard
  negatives and inflates visible rows to thousands of groups.
- `semantic-evidence-only-rerank` has zero candidates in these runs because the Prompt 45 allow-calibration decision is
  absent and semantic/vector facts remain forbidden for release calibration.

No larger default budget or lower visibility threshold is selected from this evidence. Doing so would either rely on
blocked manual labels or make broad/noisy queues visible.

## Before/After Decision

Before Prompt 72, the symbolic scorer version was `lean-dup.symbolic-scorer.v1`, and ordinary eval artifacts used the
`symbolic-only` label even though the documented baseline was `all-features`. That was a calibration-interface smell:
the baseline name was inherited from vector comparison rather than ordinary symbolic search.

After Prompt 72:

- ordinary eval uses `lean-dup.symbolic-scorer.v2`;
- the normal row is `all-features`;
- the ablation set names removed feature families directly;
- `symbolic-only` is retained only for hidden vector-validation comparisons;
- score weights remain private to search.

No score constants were changed. The selected policy is the existing all-feature scorer, now correctly versioned and
backed by the ablation evidence above.

## Blockers

- Manual calibration is blocked until manual labels are rebuilt or explicitly replaced with current, resolving labels.
- Eval memory remains above the current release target for manual and aggregate production-gate runs.
- The resolved manual-internal pair hidden by review policy should be handled by Prompt 73's kind/low-signal policy
  repair or a later visibility-policy repair, not by a broad scorer threshold drop.
- Prompt 45 still does not authorize semantic/vector facts for release calibration.

## Red Flag Review

- Shallow module: avoided. Search owns scoring; eval receives stable summaries and denominators.
- Pass-through wrapper: avoided. The scorer boundary carries a versioned policy id and curated variants, not raw
  internals.
- Temporal decomposition: avoided. Candidate generation, scoring, eval, and report remain separated by purpose.
- Information leakage: avoided. Artifacts expose feature families and counts, not raw feature keys, weights, retrieval
  keys, worker rows, or private paths.
- Special-general mixture: contained. `symbolic-only` remains only as hidden vector comparison vocabulary; ordinary
  symbolic eval uses `all-features`.
- Conjoined methods: no new conjoined API was introduced.
- Hard-to-describe public API: acceptable. The public surface is `version`, `variant`, and stable feature-family
  summaries.
- Implementation details contaminating interface comments: avoided. Comments describe scorer facts and artifact use, not
  weight values or calibration internals.
