# Manual Production Label Corpus Rebuild

## Design Note

Manual adjudication owns the truth about which current declarations should count as production positives or hard
negatives. Declaration identity resolution owns whether those labels name current workspace or source-backed mathlib
declarations. Eval owns label expansion, blockers, and denominators. Search owns candidate generation, ranking, and
review policy. Report owns projection only.

The smallest release-gate interface is the typed label corpus plus eval's stage facts: endpoint resolution, eligibility,
generated/ranked/visible status, denominators, and blockers. Label migration choices, local source inspection, search
feature weights, review rendering, and hidden experiment facts must not leak into search, report, or CLI contracts.

This preserves the user-facing capability of running production-gate eval against current KanProofs/mathlib declarations
with honest blockers. It intentionally discards the Python-era behavior of carrying stale short names as if they were
valid release evidence.

## Design It Twice

Three rebuild designs were considered:

1. Keep the current labels and mark the manual suites permanently blocked. This is honest but shallow: it preserves
   blockers without separating stale identity from real search misses.
2. Search current output for high-scoring pairs and relabel those as positives. This was rejected because it makes
   labels a reflection of the current search algorithm instead of independent truth.
3. Rebuild from current declaration identities and documented mathematical intent, then rerun unchanged denominators.
   This was chosen. It keeps eval responsible for truth, search responsible for retrieval, and report responsible for
   projection. It also makes unresolved labels explicit blockers rather than silent denominator changes.

The chosen boundary is deeper because the active label file only changes where current declaration identity is known.
Pairs whose mathematical intent cannot be recovered remain blocked and cannot become release evidence.

## Corpus Rebuild Policy

Rules used in this rebuild:

- fully qualify every endpoint that resolved to one current declaration;
- retain a positive only as release evidence when both endpoints resolve and preserve the original adjudication intent;
- for manual-mathlib positives, require one workspace endpoint and one source-backed mathlib endpoint;
- preserve typed fields: polarity, match class, expected stage, adjudication source, confidence, semantic verification
  requirement, and static-evidence setting;
- do not change search thresholds, review policy, report projection, or semantic embedding behavior;
- keep unresolved labels as blockers when no current declaration pair can be established safely.

## Per-Label Mapping

### Manual Internal

| Label intent | Old endpoints | Rebuilt endpoints | Result |
| --- | --- | --- | --- |
| Inner-fibration membership duplicate | `mem` / `mem_innerFibrations` | unchanged | Blocked. `mem` is ambiguous and `mem_innerFibrations` has no current workspace endpoint. |
| ZFC extensionality duplicate | `ext` / `ext_of_mem_iff` | unchanged | Blocked. `ext` is ambiguous and `ext_of_mem_iff` has no current workspace endpoint. |
| Grothendieck omega duplicate | `omega_map` / `integer_omega_map` | `FirstOrder.SetTheory.ZFC.Grothendieck.NaturalNumbers.omega_map` / `integer_omega_map` | Blocked. The current omega theorem resolves, but `integer_omega_map` has no current endpoint. |
| ZFSet delta-zero model replacement | `zfSet_models_delta0Theory` / `instZFSetModelsDelta0Theory` | `FirstOrder.SetTheory.ZFC.ZFCModel.models_delta0Theory` / `FirstOrder.SetTheory.ZFC.instZFSetModelsDelta0Theory` | Current positive. Both endpoints resolve; the concrete instance preserves the original model-theory intent. |
| IUT naturality binder-order duplicate | `naturality_component` / `naturality_component_source_order` | `IUT.Foundation.MorphismOfMutations.naturality_component` / `naturality_component_source_order` | Blocked. The current theorem resolves, but the source-order variant has no current endpoint. |
| Pairing presentation specialization | `SSet.Subcomplex.Pairing.toAnodynePresentation` / `SSet.Subcomplex.PairingCore.toAnodynePresentation` | unchanged | Blocked. Neither endpoint resolves in the current workspace corpus. |
| Recursion/naturality hard negative | `omegaRecursiveGraph_functional` / `naturality_component` | `_private.KanProofs.ModelTheory.SetTheory.ZFC.Arithmetic.Nat.Recursion.0.FirstOrder.SetTheory.ZFC.omegaRecursiveGraph_functional` / `IUT.Foundation.MorphismOfMutations.naturality_component` | Preserved as an ineligible hard-negative blocker; it does not become visible release evidence. |
| Diophantine conjecture hard negative | `VojtaConjectureRankOne` / `RiemannHypothesis` | unchanged | Still blocked in the internal-only suite because the right endpoint is not a workspace declaration. |
| Weierstrass valuation hard negative | `WeierstrassCurve.valuation_c₄_aux` / `WeierstrassCurve.valuation_Δ_aux` | unchanged | Blocked. The first endpoint resolves; the second no longer does. |

### Manual Mathlib

| Label intent | Old endpoints | Rebuilt endpoints | Result |
| --- | --- | --- | --- |
| Quasicategory/inner-fibration replacements | short `quasicategory_*` and `innerFibration_iff` names | `SSet.quasicategory_iff_from_innerFibration`, `SSet.quasicategory_of_innerFibration_quasicategory`, `SSet.innerFibration_iff` | Blocked. All resolved endpoints are mathlib endpoints; there is no current workspace counterpart. |
| Inner-anodyne replacement family | short `innerAnodyneExtensions_*` names | `SSet.innerAnodyneExtensions_eq_llp_rlp`, `SSet.innerAnodyneExtensions_le`, `SSet.innerAnodyneExtensions_eq_retracts_transfiniteCompositions` | Blocked. All resolved endpoints are mathlib endpoints; there is no current workspace counterpart. |
| Monoidal whiskering replacement family | short `whiskerLeft_*` / `isoOfNatIso_*` names | `CategoryTheory.MonoidalCategory.Limits.HasColimit.whiskerLeft_isoOfNatIso_ι_hom`, `CategoryTheory.MonoidalCategory.Limits.HasColimit.whiskerLeft_isoOfNatIso_ι_hom_assoc`, `CategoryTheory.MonoidalCategory.Limits.HasColimit.isoOfNatIso_ι_hom_whiskerRight` | Blocked. All resolved endpoints are mathlib endpoints; there is no current workspace counterpart. |
| Pairing presentation replacement | `SSet.Subcomplex.Pairing.toAnodynePresentation` / `SSet.Subcomplex.Pairing.anodyneExtensions` | unchanged | Blocked. `anodyneExtensions` resolves in mathlib; `toAnodynePresentation` does not. |
| Local-ring residue replacement | `IsLocalRing.isUnit_of_residue_isUnit` / `IsLocalRing.residue_ne_zero_iff_isUnit` | unchanged | Blocked. The residue theorem resolves in mathlib; the old helper no longer resolves. |
| Saturation multiplicativity hard negative | `IsSaturated.toIsStableUnderComposition` / `MorphismProperty.IsMultiplicative.toIsStableUnderComposition` | unchanged | Blocked. Neither endpoint resolves. |
| Height/stream hard negative | `Height.WeilHeight` / `Stream'.Seq1` | unchanged | Preserved. The pair resolves across workspace/mathlib but is ineligible because the workspace endpoint is an unsupported kind. |
| Height/Lawson hard negative | `Height.WeilHeight` / `Topology.WithLawson` | unchanged | Preserved. The pair resolves across workspace/mathlib but is ineligible because the workspace endpoint is an unsupported kind. |
| Diophantine conjecture hard negative | `VojtaConjectureRankOne` / `RiemannHypothesis` | unchanged | Preserved. The pair resolves across workspace/mathlib, ranks at 2, and remains hidden by review policy. |

## Rerun Results

Commands were run with `LEAN_DUP_CACHE_DIR=target/release-repair/manual-label-cache`.

| Suite | Artifact | Status | Positive resolution | Hard-negative resolution | Recall at 5 | Visible positives | Visible hard negatives |
| --- | --- | --- | --- | --- | --- | --- | --- |
| manual-internal | `target/release-repair/manual-label-internal.json` | blocked | 1/6 | 1/3 | 1/6 | 0/6 | 0/3 |
| manual-mathlib | `target/release-repair/manual-label-mathlib.json` | blocked | 0/11 | 3/4 | 0/11 | 0/11 | 0/4 |
| production-gate | `target/release-repair/manual-label-production-gate.json` | blocked | manual children blocked | 15 total hard negatives | 18/34 | 9/34 | 0/15 |

The one current internal positive is generated and ranked:

- pair: `FirstOrder.SetTheory.ZFC.ZFCModel.models_delta0Theory` /
  `FirstOrder.SetTheory.ZFC.instZFSetModelsDelta0Theory`;
- generated: yes;
- ranked: yes;
- rank: 4;
- visible: no;
- lost layer: visibility.

This is a current, resolved manual positive that Prompt 64 may examine as a representation/review-policy miss. The other
manual positives remain identity blockers, not search-quality evidence.

## Blockers

- The old manual-mathlib positive corpus mostly names mathlib declarations that no longer have current workspace
  backport endpoints. These cannot be counted as production recall.
- Several internal positives refer to deleted local wrappers or ambiguous short names. They remain blocked until a human
  adjudicates current replacement declarations.
- The current internal ZFC positive is a real generated/ranked search hit, but review policy keeps it out of the default
  visible queue. This is not a label rebuild issue.
- Manual suite runtime and RSS remain separate release blockers; this rebuild recorded the behavior but did not optimize
  it.

## Red Flag Review

- Shallow module: no new module was added; label identity remains eval-owned.
- Pass-through wrapper: no pass-through API was introduced.
- Temporal decomposition: the rebuild artifact separates current identity repair from later retrieval/visibility repair.
- Information leakage: artifacts record declaration names and stable denominators only; local implementation material is
  not part of the public report contract.
- Special-general mixture: manual-suite rules remain specific to eval labels; search and report stay general.
- Conjoined methods: no new combined label/search/report operation was added.
- Hard-to-describe public API: unchanged; callers still consume typed labels and eval artifacts.
- Implementation details contaminating interface comments: no public API comments were added in this doc-only/fixture
  edit.

## Decision

The manual production label corpus is partially rebuilt, but the production gate remains blocked. Prompt 64 should only
investigate the current resolved internal ZFC positive. The stale or mathlib-only positives require new manual
adjudication before they can become release evidence.
