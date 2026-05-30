# Symbolic Semantic Search Gap Audit

Date: 2026-05-21

This audit maps the current symbolic search engine against the next semantic-search repair sequence. It is an
investigation artifact only. It does not change retrieval, ranking, probe planning, labels, reports, or release gates.

## Design Note

Candidate generation owns semantic-key planning, posting fanout policy, per-anchor top-k selection, candidate-source
merge, and retrieval diagnostics. Lean semantic candidate lanes should own stable Lean-extracted semantic facts that can
create candidates before ranking without exposing raw expressions or proof scripts. Probe planning owns source-backed
obligation selection, bounded worker use, cache reuse, and unavailable/rejected status facts. Scoring owns feature
weights, thresholds, ablations, and visibility policy. Eval owns labels, stage denominators, manual-suite blockers, and
artifact truth. Report owns bounded projection, queue summaries, family/action wording, and replacement hints.

The smallest release-facing interface is a stage map: candidate-source facts, fanout/saturation counters, probe-yield
denominators, scorer/review policy ids, visible-family counts, and bounded replacement-hint facts. Callers should not
learn retrieval keys, posting limits, scorer weights, Lean worker rows, raw expressions, proof scripts, cache layout,
private filesystem paths, or vector facts.

The preserved user-facing capability is a conservative symbolic cleanup audit that reports a small high-confidence queue
with zero visible hard-negative leakage on the checked fixture suites. The Python-era behavior intentionally discarded
is treating a large syntactic similarity list as a semantic duplicate search result and expecting users to separate real
cleanup from related-theorem noise by hand.

## Design It Twice

Three audit designs were considered.

1. Treat the current KanProofs private audit and fast evals as enough evidence. This was rejected because green fixture
   suites prove the cleanup queue is precise, not that semantic discovery is broad enough.
2. Write a prose issue list and defer denominators to repair prompts. This was rejected because it would make later
   agents rediscover where each concern is lost.
3. Build a stage-by-stage repair map from code, artifacts, manual-label state, KanProofs private audit output, and
   search-stage metrics. This was chosen. It is deeper because the audit artifact owns release-facing evidence while
   search, eval, report, CLI, index, project, and worker keep owning their private mechanisms.

## Evidence Commands

Repository revision: `a09ba65`. Prompt repository revision: `23aa11a`.

Commands run:

```sh
git status --short
cargo run -p lean-dup-cli -- eval --suite default --format json --output target/symbolic-search/default.json
cargo run -p lean-dup-cli -- eval --suite hard-negatives --format json --output target/symbolic-search/hard-negatives.json
env LEAN_DUP_CACHE_DIR=target/symbolic-search/cache cargo run -p lean-dup-cli -- audit \
  --workspace <kan-proofs-workspace> --module KanProofs --private --format json \
  > target/symbolic-search/kanproofs-private.json
```

The working tree was clean before the artifact edit. The KanProofs workspace was locally available. The private
workspace path is redacted here; the artifact stores only normalized workspace-root file references and content
fingerprints.

Artifacts:

| Artifact | Status | Size |
| --- | --- | ---: |
| `target/symbolic-search/default.json` | ok | 4,378 bytes |
| `target/symbolic-search/hard-negatives.json` | ok | 4,373 bytes |
| `target/symbolic-search/kanproofs-private.json` | ok | 52,012 bytes |

Fast-suite denominators:

| Suite | Recall@1 | Recall@5 | Recall@10 | Visible precision | Visible positives | Visible groups | Visible hard negatives | Candidates |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `default` | 7/16 | 16/16 | 16/16 | 8/8 | 8/16 | 7/39 | 0/3 | 299 |
| `hard-negatives` | 0/1 | 1/1 | 1/1 | 1/8 | 1/1 | 7/39 | 0/5 | 299 |

Fast-suite stage facts:

| Suite | Symbolic generated positives | Ranked positives | Visible positives | Generated hard negatives | Ranked hard negatives | Visible hard negatives | Semantic probes |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `default` | 16/16 | 16/16 | 8/16 | 3/3 | 3/3 | 0/3 | 0 planned |
| `hard-negatives` | 1/1 | 1/1 | 1/1 | 2/5 | 2/5 | 0/5 | 0 planned |

KanProofs private audit denominators:

| Fact | Count |
| --- | ---: |
| Retrieval candidates before review shaping | 558,109 |
| Review candidate pairs | 13,581 |
| Review groups emitted before visibility filtering | 7,378 |
| Suppressed groups | 296 |
| Visible groups with `--private` | 5 |
| Pruned feature fanouts | 39,387 |
| Per-anchor heap truncations | 6,522 |
| Semantic candidates considered | 13,581 |
| Semantic probes planned | 500 |
| Skipped by probe policy | 6,217 |
| Skipped by probe budget/per-declaration cap | 6,864 |
| Verified probe results | 37 |
| Rejected probe results | 294 |
| Unavailable probe results | 169 |

Probe-yield details from the private audit:

| Obligation | Planned | Verified | Rejected | Unavailable |
| --- | ---: | ---: | ---: | ---: |
| exact theorem | 30 | 23 | 7 | 0 |
| reducible definition | 470 | 14 | 287 | 169 |

Visible KanProofs findings:

| Action | Pair |
| --- | --- |
| `inline-private-helper` | `IUT.Foundation.Species.ParameterizedUniqueObjectPredicate.objectInModel_object` / private `objectInModel_object_aux` |
| `local-alias` | `FirstOrder.SetTheory.ZFC.IsRepresentedCommRing.toIsRepresentedCommSemiring` / `to_isRepresentedCommSemiring` |
| `replace-local-uses` | `FirstOrder.SetTheory.ZFC.IsRepresentedRing.toIsRepresentedSemiring` / `to_is_represented_semiring` |
| `replace-local-uses` | `IUT.Foundation.Species.ParameterizedUniqueObjectPredicate.object_parameter` / `parameter_of_object` |
| `local-alias` | `FirstOrder.SetTheory.ZFC.IsRepresentedRing.toIsRepresentedAddCommGroup` / `to_isRepresentedAddCommGroup` |

Manual-label state from existing artifacts:

- Prompt 62 artifact: `docs/architecture/validation/manual-production-recall-repair.md`.
- Prompt 65 artifact: `docs/architecture/validation/manual-production-label-corpus-rebuild.md`.
- Manual internal remains blocked overall. One rebuilt positive resolves, is generated, and is ranked, but is not
  visible.
- Manual mathlib positives remain blocked because the old positive corpus mostly names mathlib/mathlib or stale
  endpoints rather than one current workspace declaration plus one source-backed mathlib declaration.

Prompt 45 did not produce `docs/architecture/evaluation/semantic-theorem-profile-validation-decision.md` in this
checkout. The existing `semantic-search-validation-decision.md` keeps semantic vector search hidden and off-default.
Prompt 46 therefore continues to ignore semantic/vector facts for release calibration.

## Repair Map

### Candidate-Source Boundary And Merge Policy

Files and APIs:

- `crates/search/src/retrieval.rs`
- `crates/search/src/observation.rs`
- crate-root exports `SearchObservationRequest`, `SearchStageObservation`, `SearchRetrievalObservation`,
  `observe_search`, and `observe_search_stages`
- Prompt 68: `68-candidate-source-boundary.md`

Current behavior:

Retrieval directly creates `CandidateSet` values from statement, safe-permutation, connective, conclusion, and role
features. `SearchStageObservation` is already a compact eval surface, but the internal retrieval layer is still one
syntactic source with policy labels such as `local_duplicate_audit` and `source_backed_external_comparison`. Merge
policy is implicit because there is only symbolic generation in the core path.

Evidence:

- Fast eval reports only `symbolic_generated` and `merged_generated`; they are identical.
- KanProofs private audit reports 558,109 retrieval candidates and 13,581 review candidate pairs after shaping.
- The current public stage surface is safer than raw retrieval rows, but it cannot distinguish future Lean semantic
  candidate lanes from existing symbolic retrieval without adding source-owned candidate facts.

Classification: architecture and observability, with recall implications.

Expected repair: Prompt 68. No additional prompt is needed.

Release calibration boundary: vector and semantic-profile facts remain forbidden unless Prompt 45 explicitly allows
them.

### Lean Semantic Candidate Lanes Before Ranking And Probing

Files and APIs:

- `crates/worker/src/lib.rs` exports extracted declaration, fingerprint, and role facts
- `crates/index/src/lib.rs` exposes hydrated declaration and semantic feature facades
- `crates/search/src/retrieval.rs` consumes only current fingerprint/role feature facts for generation
- Prompt 69: `69-lean-semantic-candidate-lanes.md`

Current behavior:

Lean contributes normalized fingerprints and role features, but there is no separate semantic candidate lane that can
generate pairs from richer Lean-owned facts before ranking. Semantic worker probes happen after retrieval has already
selected candidate pairs. This means Lean is mostly a verifier of syntactically generated candidates, not a discovery
engine.

Evidence:

- Fast eval semantic verification counters are all zero.
- KanProofs private audit probes only 500 of 13,581 shaped pairs and none of the 558,109 raw retrieval candidates.
- The visible queue is precise but tiny: 5 visible groups out of 7,378 emitted review groups on private KanProofs.

Classification: recall and architecture.

Expected repair: Prompt 69. No additional prompt is needed.

Release calibration boundary: semantic lane facts may become symbolic/Lean facts only if they are extracted and owned by
the core symbolic path. They must not import vector facts.

### Fanout, Top-K Caps, Saturation, And Loss Accounting

Files and APIs:

- `crates/search/src/retrieval.rs`
- constants `TOP_K_PER_WORKSPACE_DECLARATION = 80`, `ROLE_POSTING_LIMIT = 512`, `BROAD_HEAD_POSTING_LIMIT = 64`
- `RetrievalDiagnostics`, `PrunedFeatureFanout`, `HeapTruncation`
- Prompt 70: `70-high-recall-fanout-and-topk-policy.md`

Current behavior:

Retrieval prunes broad postings and selects the top 80 candidates per workspace declaration. It records aggregate pruned
fanout and heap-truncation counts, but eval does not yet tie those losses back to labeled positives, hard negatives,
match class, source, or candidate-source saturation.

Evidence:

- KanProofs private audit recorded 39,387 pruned feature fanouts.
- It recorded 6,522 per-anchor heap truncations.
- Fast eval proves fixture positives survive the current caps, but the fixtures are too small to validate KanProofs
  fanout loss.
- Manual labels are mostly blocked or stale, so they currently cannot prove whether cap losses hide real production
  positives.

Classification: recall, performance, and observability.

Expected repair: Prompt 70. No additional prompt is needed.

Release calibration boundary: fanout policy must use symbolic/Lean candidate facts only; vector nearest-neighbor recall
is not accepted release evidence.

### Semantic Probe Planning Before Final Ranking

Files and APIs:

- `crates/search/src/audit.rs`
- `crates/search/src/semantic_verification.rs`
- `candidate_sets_for_review`, `verify_candidate_probes`, `ProbeSettings`, `ProbeDiagnostics`
- Prompt 71: `71-semantic-probe-planning-before-ranking.md`

Current behavior:

Audit first narrows retrieval to strong static evidence, performs cheap ranking, then plans probes from the shaped
candidate sets and cheap review groups. Default actionability probes are bounded by a total budget and a per-declaration
cap of 2. Diagnostics can keep broader candidates, but ordinary probes are still downstream of syntactic generation and
early ranking.

Evidence:

- KanProofs private audit considered 13,581 shaped candidates.
- It planned 500 probes, skipped 6,217 by policy, and skipped 6,864 by budget/per-declaration cap.
- Probe yield was mixed: 37 verified, 294 rejected, 169 unavailable.
- Reducible-definition probes dominated the plan: 470 planned, 14 verified, 287 rejected, 169 unavailable.

Classification: recall, precision, performance, and observability.

Expected repair: Prompt 71. No additional prompt is needed.

Release calibration boundary: Lean probe facts are part of the symbolic audit path, but semantic/vector profile facts
remain forbidden unless Prompt 45 allows them.

### Calibrated Scorer And Visibility Thresholds

Files and APIs:

- `crates/search/src/scorer.rs`
- `crates/search/src/review_policy.rs`
- `SearchScoringVariant`, `SearchScoringSummary`, `SearchReviewPolicySummary`
- `docs/architecture/evaluation/symbolic-threshold-calibration.md`
- Prompt 72: `72-calibrated-symbolic-evidence-scorer.md`

Current behavior:

Scoring uses hand-set weights and thresholds in `DEFAULT_SCORER_CONFIG`. Visibility uses review-policy blocker rules and
theorem-like statement/permutation checks. The current defaults produce high fixture precision, but they encode policy
through constants and blocker predicates rather than a label-backed calibrated scorer.

Evidence:

- Default suite: visible precision 8/8, visible positives 8/16, visible hard negatives 0/3.
- Hard-negative suite: visible precision 1/8, visible hard negatives 0/5.
- Previous calibration artifact records manual-internal recall as 0/6 before the label-rebuild work and states that
  semantic/vector facts were ignored.
- The rebuilt manual-internal corpus has one current positive that is generated and ranked but hidden by visibility.

Classification: precision, actionability, and architecture.

Expected repair: Prompt 72. No additional prompt is needed.

Release calibration boundary: Prompt 46 still ignores semantic/vector facts because the Prompt 45 decision artifact is
absent and the existing semantic-search decision rejects calibration.

### Kind And Low-Signal Policy

Files and APIs:

- `crates/search/src/review_policy.rs`
- `crates/search/src/ranking.rs`
- `crates/search/src/retrieval.rs`
- Prompt 73: `73-kind-and-low-signal-policy-repair.md`

Current behavior:

Visibility blockers are blunt: generated declarations, non-public declarations, low-signal declarations, broad-head-only
matches, typeclass-instance noise, and static non-theorem pairs are hidden by default. Recent UI work lets `--private`
show actionable private helper findings, but kind and low-signal classification still has coarse rules.

Evidence:

- `--private` KanProofs audit surfaced exactly one private-helper cleanup group and four public groups.
- Fast eval still has zero visible hard-negative leakage.
- KanProofs retrieval volume is high while visible output is tiny, suggesting the broad/noise filters are carrying a
  large actionability burden.
- Manual internal has a current ZFC positive that is generated and ranked but not visible, so at least one real label is
  lost at review/visibility rather than candidate generation.

Classification: precision and recall.

Expected repair: Prompt 73, with Prompt 72 providing scorer evidence first. No additional prompt is needed.

Release calibration boundary: kind policy must not use vector facts or proof-neighborhood-only facts as default
actionability evidence.

### Duplicate-Family Clustering And Action Selection

Files and APIs:

- `crates/search/src/ranking.rs`
- `crates/search/src/audit.rs`
- `crates/report/src/reports.rs`
- `crates/report/src/report_contract.rs`
- Prompt 74: `74-duplicate-family-clustering-and-actions.md`

Current behavior:

Review groups are still pair-shaped. `ranking.rs` can suppress weaker relations for the same pair, and reports have
bounded visible groups, but ordinary audit does not yet promote stable family-level cleanup actions where several
declarations form one duplicate family.

Evidence:

- KanProofs visible findings include alias-style pairs such as `toIsRepresentedSemiring` / `to_is_represented_semiring`
  and `toIsRepresentedAddCommGroup` / `to_isRepresentedAddCommGroup`.
- The report emits 5 visible pair groups even though two findings are part of the same ZFC representation naming pattern
  and likely need family-level review.
- `review.groups` full detail is not duplicated in ordinary JSON, so family work should preserve the bounded report
  contract rather than re-expanding pairs.

Classification: actionability and report UX.

Expected repair: Prompt 74. No additional prompt is needed.

Release calibration boundary: family clustering must consume search-owned pair/family facts, not vector evidence.

### Source-Use And Replacement-Hint Quality

Files and APIs:

- `crates/search/src/replacement_hints.rs`
- `crates/search/src/source_refs.rs`
- `crates/search/src/audit.rs`
- report DTOs for replacement hints and source references
- Prompt 75: `75-source-use-and-replacement-hint-quality.md`

Current behavior:

Replacement hints expose target declaration, target module, import status, caller count, bounded callers, notes, and
blockers. Private-helper wrapper cleanup is recognized when a private member is only referenced inside the public
wrapper span. The source-use evidence remains read-only and bounded, but it is still based on source-reference scanning
and action-specific heuristics.

Evidence:

- KanProofs private audit produced one `inline-private-helper` hint with one bounded caller reference inside the public
  wrapper.
- It produced `local-alias` hints with zero callers and `replace-local-uses` hints with five bounded callers.
- The report uses workspace-root file fingerprints and bounded text snippets rather than absolute private paths.

Classification: actionability and observability.

Expected repair: Prompt 75. No additional prompt is needed.

Release calibration boundary: replacement hints should not depend on vector facts or expose worker/cache internals.

## Additional Prompt Decision

No new prompt was added. Prompts 68-75 cover the blockers found in this audit:

- candidate-source boundary: 68;
- semantic candidate lanes: 69;
- fanout/top-k loss accounting: 70;
- probe planning: 71;
- calibrated scorer: 72;
- kind/low-signal policy: 73;
- duplicate-family actions: 74;
- replacement hints: 75.

Prompt 76 remains the validation decision after those repairs. Prompt 60/61 still own release-candidate validation and
0.1.0 go/no-go.

## Red Flag Review

- Shallow module: current search is not a pure pass-through, but candidate generation, fanout policy, and probe planning
  still expose too little stable stage information for eval to locate real production misses. Prompts 68-71 address
  this.
- Pass-through wrapper: no new wrapper was added. Existing report/eval surfaces project stable facts rather than raw
  retrieval structs.
- Temporal decomposition: current audit order is retrieval, candidate shaping, cheap ranking, probing, final ranking,
  source hints. The risk is that probe planning follows execution order rather than semantic evidence ownership. Prompt
  71 addresses this.
- Information leakage: release artifacts avoid private paths, worker rows, raw expressions, and retrieval keys. Some
  search DTOs still carry feature-family diagnostics, which are stable vocabulary rather than raw keys.
- Special-general mixture: vector/semantic-profile experiment facts remain outside release calibration. Core symbolic
  repair prompts are scoped to Lean/search/eval/report facts.
- Conjoined methods: ranking currently combines relation classification, blocker application, action selection, and
  target selection in one path. Prompts 72-75 split the policy concerns by scorer, kind policy, family actions, and
  replacement hints.
- Hard-to-describe public API: the intended repair interface is stage facts plus policy ids. The current internal
  retrieval/probe APIs are harder to describe because caps and candidate shaping are implicit.
- Implementation details contaminating interface comments: the public crate-root comments are mostly ownership-focused.
  Later prompts should keep constants, posting mechanics, worker transport, and source-scan heuristics out of interface
  comments.

## Verification Notes

- This file maps all eight repair themes.
- Required fast eval artifacts were created under `target/symbolic-search/`.
- The KanProofs private audit artifact was created because local prerequisites were available.
- Prompt 62 and Prompt 65 artifacts exist and were inspected.
- No manual suite skip was counted as a pass.
- No vector fact is used as release evidence; Prompt 46 remains constrained by the missing Prompt 45 decision artifact
  and the existing keep-hidden semantic-search decision.
