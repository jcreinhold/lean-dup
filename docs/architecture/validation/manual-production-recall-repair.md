# Manual Production Recall Repair

Prompt 62 investigated the Prompt 60 release blocker where the manual KanProofs production suites ran with prerequisites
present but found zero labeled positives. This artifact records the repair and the remaining release blocker.

## Design Note

Manual labels own the human adjudication target: which current declarations should count as positives or hard negatives
for release evidence. Declaration identity resolution owns the mapping from label strings to current indexed
declarations. Search owns candidate generation, ranking, review policy, and semantic-probe fallback. Eval owns truth,
stage denominators, and the decision that a manual suite is usable release evidence. Report owns bounded projection of
those facts.

The smallest public interface added by this repair is a stable label-resolution report: endpoint status, bounded
candidate summaries, canonical pair when unambiguous, stage facts, loss layer, and blockers. The interface does not
expose index storage, worker rows, cache layout, raw proof obligations, scorer internals, or private local paths.

The preserved user-facing capability is production-gate evaluation from real KanProofs/manual labels. The intentionally
discarded Python-era behavior is treating stale label text as if it were a valid release denominator and then reporting
zero recall without first proving that the labels still name current declarations.

## Design It Twice

Three repair designs were considered:

1. Lower or tune review thresholds until manual positives appear. This was rejected because Prompt 60 showed the
   positives were already missing at candidate generation; threshold tuning would hide the real failure.
2. Rewrite manual labels to match current search output. This was rejected because it would move truth to the scorer and
   risk weakening the manual adjudication corpus.
3. Trace each labeled pair through declaration resolution, eligibility, candidate generation, ranking, and visibility,
   then repair the owning layer that loses it. This was chosen. It keeps eval responsible for truth and denominators,
   search responsible for retrieval/review behavior, and report as a projection layer.

## Implementation Summary

The repair added eval-owned label tracing for manual suites:

- label endpoints resolve against current workspace declarations and, for manual mathlib, source-backed external
  declarations;
- exact qualified-name matches and unique display-name matches can canonicalize labels to current declaration names;
- ambiguous, missing, or wrong-corpus endpoints block manual suites before they are counted as release evidence;
- each typed label records whether it reached eligibility, candidate generation, ranking, and visibility;
- aggregate `production-gate` status becomes `blocked` when a manual child is blocked;
- default and hard-negative fixture suites do not emit manual label-resolution traces.

The index crate exposes one storage-neutral helper for declaration lookup by display name. It returns hydrated
declaration facts through the crate root and keeps SQLite/cache/storage details private.

## Manual-Internal Trace

Artifact: `target/release-repair/manual-internal.json`

Summary:

- suite status: `blocked`;
- positive labels resolved: `0/6`;
- hard-negative labels resolved: `1/3`;
- candidate-generation recall: `0/6`;
- recall@1/5/10: `0/6`, `0/6`, `0/6`;
- visible hard-negative hits: `0/3`.

| Label | Resolution | Generated | Ranked | Visible | Owning loss layer |
| --- | --- | ---: | ---: | ---: | --- |
| `mem` / `mem_innerFibrations` | left ambiguous, right missing | no | no | no | label resolution |
| `ext` / `ext_of_mem_iff` | left ambiguous, right missing | no | no | no | label resolution |
| `integer_omega_map` / `omega_map` | left missing, right display-unique | no | no | no | label resolution |
| `instZFSetModelsDelta0Theory` / `zfSet_models_delta0Theory` | left display-unique, right missing | no | no | no | label resolution |
| `naturality_component` / `naturality_component_source_order` | left display-unique, right missing | no | no | no | label resolution |
| `SSet.Subcomplex.Pairing.toAnodynePresentation` / `SSet.Subcomplex.PairingCore.toAnodynePresentation` | both missing | no | no | no | label resolution |

The owned-layer repair here is not a search-policy change. None of the manual-internal positive labels reached a stable
current pair, so candidate-generation and ranking behavior cannot yet be judged from this suite.

## Manual-Mathlib Trace

Artifact: `target/release-repair/manual-mathlib.json`

Summary:

- suite status: `blocked`;
- positive labels resolved: `0/11`;
- hard-negative labels resolved: `3/4`;
- candidate-generation recall: `0/11`;
- recall@1/5/10: `0/11`, `0/11`, `0/11`;
- visible hard-negative hits: `0/4`.

| Label | Resolution | Generated | Ranked | Visible | Owning loss layer |
| --- | --- | ---: | ---: | ---: | --- |
| `quasicategory_iff_from_innerFibration` / `quasicategory_of_innerFibration_quasicategory` | both resolve to mathlib | no | no | no | label resolution |
| `innerFibration_iff` / `quasicategory_iff_from_innerFibration` | both resolve to mathlib | no | no | no | label resolution |
| `innerFibration_iff` / `quasicategory_of_innerFibration_quasicategory` | both resolve to mathlib | no | no | no | label resolution |
| `innerAnodyneExtensions_eq_llp_rlp` / `innerAnodyneExtensions_le` | both resolve to mathlib | no | no | no | label resolution |
| `innerAnodyneExtensions_eq_llp_rlp` / `innerAnodyneExtensions_eq_retracts_transfiniteCompositions` | both resolve to mathlib | no | no | no | label resolution |
| `innerAnodyneExtensions_eq_retracts_transfiniteCompositions` / `innerAnodyneExtensions_le` | both resolve to mathlib | no | no | no | label resolution |
| `whiskerLeft_isoOfNatIso_ι_hom` / `whiskerLeft_isoOfNatIso_ι_hom_assoc` | both resolve to mathlib | no | no | no | label resolution |
| `isoOfNatIso_ι_hom_whiskerRight` / `whiskerLeft_isoOfNatIso_ι_hom` | both resolve to mathlib | no | no | no | label resolution |
| `isoOfNatIso_ι_hom_whiskerRight` / `whiskerLeft_isoOfNatIso_ι_hom_assoc` | both resolve to mathlib | no | no | no | label resolution |
| `SSet.Subcomplex.Pairing.anodyneExtensions` / `SSet.Subcomplex.Pairing.toAnodynePresentation` | left exact, right missing | no | no | no | label resolution |
| `IsLocalRing.isUnit_of_residue_isUnit` / `IsLocalRing.residue_ne_zero_iff_isUnit` | left missing, right exact | no | no | no | label resolution |

For manual mathlib, a positive label must identify one workspace declaration and one mathlib declaration. The old labels
mostly identify mathlib/mathlib pairs, which are invalid for this production gate. That is stale-corpus evidence, not a
symbolic retrieval result.

## Production-Gate Rerun

Artifact: `target/release-repair/production-gate.json`

The aggregate production gate is now `blocked` rather than `ok` with zero manual recall. The default and hard-negative
fixture children still pass and do not emit manual label-resolution traces. The manual children are blocked with exact
label-resolution blockers and raw denominators.

## Remaining Blocker

Manual production recall did not improve because no manual positive currently resolves to a valid production pair. The
correct repair is to rebuild the manual label corpus against the current workspace and source-backed mathlib corpus,
preserving mathematical intent only when a trace proves the mapping. This cannot be done mechanically in this session
without weakening labels or inventing intent.

A follow-up prompt was added for that work: `65-manual-production-label-corpus-rebuild.md`.

## Red Flag Review

- Shallow module: the new behavior is not a pass-through report field; eval now owns label identity truth before
  scoring denominators.
- Pass-through wrapper: report only projects eval's stable label-resolution DTOs.
- Temporal decomposition: label resolution happens before candidate generation, so stale labels are defined out of the
  release denominator instead of being discovered after a failed score.
- Information leakage: artifacts contain bounded declaration summaries and stage facts, not private paths, worker rows,
  cache layout, proof obligations, or storage internals.
- Special-general mixture: manual-only tracing is separated from ordinary fixture reports.
- Conjoined methods: identity resolution, eligibility, generation, ranking, and visibility are separate trace stages.
- Hard-to-describe public API: the public facts are endpoint status, canonical pair, loss layer, and counters.
- Implementation details contaminating interface comments: new interface comments describe label/search facts, not
  temporary migration logic or scorer internals.

## Release Status

This prompt does not close the Prompt 60 no-go blocker. It changes the blocker from unexplained zero recall to an
explicit stale/invalid manual label corpus. Prompt 60 must be rerun only after the manual production labels are rebuilt
and the memory/RSS blocker is closed.
