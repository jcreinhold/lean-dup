"""Bounded structural probes for duplicate candidates."""

from __future__ import annotations

from dataclasses import dataclass

from lean_dup.features import pair_features
from lean_dup.matching import jaccard
from lean_dup.models import Declaration


@dataclass(frozen=True)
class ProbeResult:
    """Cheap structural probe results for a declaration pair."""

    same_statement: bool = False
    same_up_to_reordering: bool = False
    connective_equivalent: bool = False
    specializes: bool = False
    mutual_implication_shape: bool = False
    same_reducible_def: bool = False

    @property
    def signals(self) -> tuple[str, ...]:
        signals: list[str] = []
        if self.same_statement:
            signals.append("probe:same-statement")
        if self.same_up_to_reordering:
            signals.append("probe:same-up-to-reordering")
        if self.connective_equivalent:
            signals.append("probe:connective-equivalent")
        if self.specializes:
            signals.append("probe:specializes")
        if self.mutual_implication_shape:
            signals.append("probe:mutual-implication-shape")
        if self.same_reducible_def:
            signals.append("probe:same-reducible-def")
        return tuple(signals)


def probe_pair(first: Declaration, second: Declaration) -> ProbeResult:
    """Run bounded structural checks without proof search."""

    features = pair_features(first, second)
    constants = jaccard(set(first.constants), set(second.constants))
    binder_delta = abs(first.binder_count - second.binder_count)
    same_def = (
        first.kind == "def"
        and second.kind == "def"
        and first.normalized_type == second.normalized_type
        and features.same_source_skeleton
    )
    specializes = (
        features.same_conclusion
        and constants >= 0.45
        and binder_delta <= 3
        and (features.same_namespace_family or features.names >= 0.25 or bool(features.shared_heads))
    )
    mutual_shape = (
        features.same_conclusion
        and constants >= 0.25
        and bool(features.shared_heads)
        and binder_delta <= 2
    )
    return ProbeResult(
        same_statement=features.exact_statement,
        same_up_to_reordering=features.same_permutation and not features.exact_statement,
        connective_equivalent=features.same_connective and not features.same_permutation,
        specializes=specializes,
        mutual_implication_shape=mutual_shape,
        same_reducible_def=same_def,
    )
