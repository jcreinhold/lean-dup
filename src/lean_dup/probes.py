"""Bounded structural probes for duplicate candidates."""

from __future__ import annotations

from dataclasses import dataclass

from lean_dup.features import pair_features
from lean_dup.matching import jaccard
from lean_dup.models import Declaration


def declaration_probe_key(declaration: Declaration) -> str:
    """Return a stable identity key for probe/ranking maps."""

    return "\0".join(
        (
            declaration.origin,
            declaration.name,
            str(declaration.file),
            str(declaration.span.start.line),
        )
    )


@dataclass(frozen=True)
class ProbeResult:
    """Cheap structural probe results for a declaration pair."""

    same_statement: bool = False
    same_up_to_reordering: bool = False
    connective_equivalent: bool = False
    specializes: bool = False
    specializes_left_to_right: bool = False
    specializes_right_to_left: bool = False
    mutual_implication_shape: bool = False
    same_reducible_def: bool = False
    unavailable: bool = False
    source: str = "heuristic"
    message: str | None = None

    @property
    def signals(self) -> tuple[str, ...]:
        signals: list[str] = []
        if self.same_statement:
            signals.append("probe:same-statement")
        if self.same_up_to_reordering:
            signals.append("probe:same-up-to-reordering")
        if self.connective_equivalent:
            signals.append("probe:connective-equivalent")
        if self.specializes or self.specializes_left_to_right or self.specializes_right_to_left:
            signals.append("probe:specializes")
        if self.specializes_left_to_right:
            signals.append("probe:specializes-left-to-right")
        if self.specializes_right_to_left:
            signals.append("probe:specializes-right-to-left")
        if self.mutual_implication_shape:
            signals.append("probe:mutual-implication-shape")
        if self.same_reducible_def:
            signals.append("probe:same-reducible-def")
        return tuple(signals)

    @property
    def blockers(self) -> tuple[str, ...]:
        """Return probe-derived blockers."""

        if self.unavailable:
            return ("lean-probe-unavailable",)
        return ()

    @property
    def confirmed(self) -> bool:
        """Return whether this probe confirmed a strong semantic relation."""

        return any(
            (
                self.same_statement,
                self.same_up_to_reordering,
                self.connective_equivalent,
                self.specializes,
                self.specializes_left_to_right,
                self.specializes_right_to_left,
                self.same_reducible_def,
            )
        )

    @property
    def summary(self) -> str | None:
        """Return a compact human-facing probe summary."""

        parts = list(self.signals)
        if self.message:
            parts.append(self.message)
        if not parts:
            return None
        return f"{self.source}: " + ", ".join(parts)


NO_PROBE = ProbeResult(source="none", message="semantic probes disabled")


def heuristic_probe_pair(first: Declaration, second: Declaration) -> ProbeResult:
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
        and (
            features.same_namespace_family or features.names >= 0.25 or bool(features.shared_heads)
        )
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
        specializes_left_to_right=specializes and first.binder_count >= second.binder_count,
        specializes_right_to_left=specializes and second.binder_count >= first.binder_count,
        mutual_implication_shape=mutual_shape,
        same_reducible_def=same_def,
    )


def probe_pair(first: Declaration, second: Declaration) -> ProbeResult:
    """Backward-compatible heuristic probe."""

    return heuristic_probe_pair(first, second)
