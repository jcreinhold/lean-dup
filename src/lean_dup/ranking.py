"""Evidence aggregation and review ranking for duplicate groups."""

from __future__ import annotations

from itertools import combinations
from typing import Mapping

from lean_dup.features import broad_conclusion, pair_features, pair_signal_score
from lean_dup.models import Declaration, DuplicateGroup, DuplicateKind, DuplicateMember, ReviewPriority
from lean_dup.probes import ProbeResult, declaration_probe_key, heuristic_probe_pair

PRIORITY_ORDER = {
    ReviewPriority.HIGH: 0,
    ReviewPriority.MEDIUM: 1,
    ReviewPriority.LOW: 2,
    ReviewPriority.NOISE: 3,
}


def rank_group(
    group: DuplicateGroup,
    declarations: tuple[Declaration, ...],
    *,
    probe_results: Mapping[frozenset[str], ProbeResult] | None = None,
) -> DuplicateGroup:
    """Attach triage evidence and recommended cleanup action to a group."""

    signals = set(group.evidence)
    blockers: set[str] = set()
    generated_count = sum(1 for member in group.members if _member_is_generated(member))
    workspace = [member for member in group.members if member.origin == "workspace"]
    external_members = [member for member in group.members if member.origin != "workspace"]
    has_mathlib = any(member.origin == "mathlib" for member in group.members)
    has_backport = any(_member_is_backport(member) for member in workspace)
    summaries: list[str] = []
    confirmed_specialization = False
    confirmed_same = False

    if generated_count:
        blockers.add(f"generated-declarations={generated_count}")
    if all(broad_conclusion(declaration) for declaration in declarations):
        blockers.add("broad-conclusion-only")
    if any(member.kind == "instance" or member.display_name.startswith("inst") for member in group.members):
        blockers.add("typeclass-instance-noise")

    for first, second in combinations(declarations, 2):
        features = pair_features(first, second)
        score, evidence = pair_signal_score(first, second)
        signals.update(evidence)
        probe = _probe_result(first, second, probe_results)
        signals.update(probe.signals)
        blockers.update(probe.blockers)
        if probe.summary:
            summaries.append(probe.summary)
        confirmed_same = confirmed_same or probe.same_statement or probe.same_reducible_def
        confirmed_specialization = confirmed_specialization or (
            probe.specializes or probe.specializes_left_to_right or probe.specializes_right_to_left
        )
        if features.same_namespace_family:
            signals.add("same-namespace-family")
        if score < 0.25 and group.kind is DuplicateKind.NEAR_STATEMENT:
            blockers.add("weak-feature-overlap")
        if not features.shared_heads and features.same_heads:
            blockers.add("low-signal-heads-only")

    if has_mathlib and group.kind is DuplicateKind.EXACT_STATEMENT:
        action = "already-in-mathlib"
        priority = ReviewPriority.HIGH
        signals.add("same-imported-mathlib-declaration")
    elif group.kind is DuplicateKind.EXACT_STATEMENT:
        action = "local-alias"
        priority = ReviewPriority.HIGH
    elif group.kind in {DuplicateKind.PERMUTED_STATEMENT, DuplicateKind.CONNECTIVE_EQUIVALENT}:
        action = "local-alias"
        priority = ReviewPriority.HIGH if "probe:same-up-to-reordering" in signals else ReviewPriority.MEDIUM
    elif group.kind is DuplicateKind.SOURCE_CLONE:
        action = "probable-source-clone"
        priority = ReviewPriority.LOW
    elif confirmed_specialization:
        action = "specialization-of"
        priority = ReviewPriority.MEDIUM
    elif group.kind is DuplicateKind.SUBSUMPTION_CANDIDATE:
        action = "merge-generalization"
        priority = ReviewPriority.MEDIUM
    else:
        action = "review"
        priority = ReviewPriority.LOW

    if has_backport and has_mathlib:
        signals.add("backport-now-in-mathlib")
        priority = ReviewPriority.HIGH
    if has_mathlib and (confirmed_same or group.kind is DuplicateKind.EXACT_STATEMENT):
        action = "already-in-mathlib"
        priority = ReviewPriority.HIGH
    if generated_count and group.kind is DuplicateKind.SOURCE_CLONE:
        priority = ReviewPriority.NOISE
    elif blockers == {"broad-conclusion-only"} and group.kind is DuplicateKind.SUBSUMPTION_CANDIDATE:
        priority = ReviewPriority.LOW
    elif generated_count and priority is ReviewPriority.HIGH:
        priority = ReviewPriority.MEDIUM
    elif not workspace:
        priority = ReviewPriority.NOISE

    return DuplicateGroup(
        id=group.id,
        kind=group.kind,
        confidence=_adjust_confidence(group.confidence, priority, blockers),
        reason=group.reason,
        evidence=group.evidence,
        members=group.members,
        signals=tuple(sorted(signals)),
        blockers=tuple(sorted(blockers)),
        recommended_action=action,
        review_priority=priority,
        recommended_target=_recommended_target(external_members),
        probe_summary=_probe_summary(summaries),
    )


def actionable(group: DuplicateGroup, *, include_generated: bool, show_noise: bool, min_priority: ReviewPriority) -> bool:
    """Return whether a group should appear in default human-facing output."""

    if group.review_priority is ReviewPriority.NOISE and not show_noise:
        return False
    if not include_generated and any(blocker.startswith("generated-declarations=") for blocker in group.blockers):
        return False
    return PRIORITY_ORDER[group.review_priority] <= PRIORITY_ORDER[min_priority]


def _adjust_confidence(confidence: float, priority: ReviewPriority, blockers: set[str]) -> float:
    if priority is ReviewPriority.NOISE:
        return min(confidence, 0.25)
    if blockers and priority is ReviewPriority.LOW:
        return min(confidence, 0.55)
    if blockers and priority is ReviewPriority.MEDIUM:
        return min(confidence, 0.82)
    return confidence


def _member_is_generated(member: DuplicateMember) -> bool:
    short_name = member.name.rsplit(".", 1)[-1]
    return short_name in {
        "rec",
        "recOn",
        "casesOn",
        "noConfusion",
        "noConfusionType",
        "ctorElim",
        "elim",
    } or member.name.startswith("_aux_") or "._aux_" in member.name or short_name.startswith("term_")


def _member_is_backport(member: DuplicateMember) -> bool:
    return ".Mathlib4Backports." in member.module or member.module.endswith("Mathlib4Backports")


def _probe_result(
    first: Declaration,
    second: Declaration,
    probe_results: Mapping[frozenset[str], ProbeResult] | None,
) -> ProbeResult:
    key = frozenset({declaration_probe_key(first), declaration_probe_key(second)})
    if probe_results is not None and key in probe_results:
        result = probe_results[key]
        if not result.unavailable:
            return result
        fallback = heuristic_probe_pair(first, second)
        return ProbeResult(
            same_statement=fallback.same_statement,
            same_up_to_reordering=fallback.same_up_to_reordering,
            connective_equivalent=fallback.connective_equivalent,
            specializes=fallback.specializes,
            specializes_left_to_right=fallback.specializes_left_to_right,
            specializes_right_to_left=fallback.specializes_right_to_left,
            mutual_implication_shape=fallback.mutual_implication_shape,
            same_reducible_def=fallback.same_reducible_def,
            unavailable=True,
            source="lean+heuristic",
            message=result.message,
        )
    return heuristic_probe_pair(first, second)


def _recommended_target(external_members: list[DuplicateMember]) -> str | None:
    if not external_members:
        return None
    preferred = sorted(
        external_members,
        key=lambda member: (0 if member.origin == "mathlib" else 1, member.name),
    )[0]
    return preferred.name


def _probe_summary(summaries: list[str]) -> str | None:
    if not summaries:
        return None
    return "; ".join(dict.fromkeys(summaries[:4]))
