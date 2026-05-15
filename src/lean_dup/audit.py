"""Duplicate classification over Lean declarations."""

from __future__ import annotations

from collections import defaultdict
from itertools import combinations
from pathlib import Path

from lean_dup.extractor import load_or_build_index
from lean_dup.models import (
    AuditReport,
    Declaration,
    DuplicateGroup,
    DuplicateKind,
    DuplicateMember,
)
from lean_dup.workspace import resolve_workspace


def run_audit(*, workspace: Path, module_root: str | None = None) -> AuditReport:
    """Audit one Lake workspace for duplicated Lean declarations."""

    resolved = resolve_workspace(workspace, module_root)
    index = load_or_build_index(resolved)
    groups = _classify(index.declarations)
    return AuditReport(
        workspace=resolved.root,
        module_root=module_root,
        declaration_count=len(index.declarations),
        cache_hit=index.cache_hit,
        groups=tuple(groups),
    )


def _classify(declarations: tuple[Declaration, ...]) -> list[DuplicateGroup]:
    groups: list[DuplicateGroup] = []
    groups.extend(_exact_statement_groups(declarations))
    groups.extend(_source_clone_groups(declarations))
    groups.extend(_subsumption_groups(declarations))
    groups.extend(_near_statement_groups(declarations))
    return groups


def _exact_statement_groups(declarations: tuple[Declaration, ...]) -> list[DuplicateGroup]:
    buckets: dict[str, list[Declaration]] = defaultdict(list)
    for declaration in _statement_declarations(declarations):
        buckets[declaration.type_fingerprint].append(declaration)
    groups = []
    count = 1
    for members in buckets.values():
        if len(members) < 2:
            continue
        groups.append(
            _group(
                group_id=f"exact-statement-{count}",
                kind=DuplicateKind.EXACT_STATEMENT,
                confidence=1.0,
                reason="same elaborated statement fingerprint after binder-name erasure",
                declarations=members,
            )
        )
        count += 1
    return groups


def _source_clone_groups(declarations: tuple[Declaration, ...]) -> list[DuplicateGroup]:
    buckets: dict[str, list[Declaration]] = defaultdict(list)
    for declaration in declarations:
        if declaration.source_fingerprint:
            buckets[declaration.source_fingerprint].append(declaration)
    groups = []
    count = 1
    for members in buckets.values():
        if len(members) < 2:
            continue
        groups.append(
            _group(
                group_id=f"source-clone-{count}",
                kind=DuplicateKind.SOURCE_CLONE,
                confidence=0.9,
                reason="same declaration source skeleton after comment and whitespace normalization",
                declarations=members,
            )
        )
        count += 1
    return groups


def _subsumption_groups(declarations: tuple[Declaration, ...]) -> list[DuplicateGroup]:
    buckets: dict[str, list[Declaration]] = defaultdict(list)
    for declaration in _statement_declarations(declarations):
        buckets[declaration.conclusion_fingerprint].append(declaration)
    groups = []
    count = 1
    for members in buckets.values():
        binder_counts = {member.binder_count for member in members}
        if len(members) < 2 or len(binder_counts) < 2:
            continue
        groups.append(
            _group(
                group_id=f"subsumption-candidate-{count}",
                kind=DuplicateKind.SUBSUMPTION_CANDIDATE,
                confidence=0.72,
                reason="same conclusion shape with different binder counts",
                declarations=members,
            )
        )
        count += 1
    return groups


def _near_statement_groups(declarations: tuple[Declaration, ...]) -> list[DuplicateGroup]:
    groups: list[DuplicateGroup] = []
    declarations = tuple(_statement_declarations(declarations))
    exact_pairs = {
        frozenset({a.name, b.name})
        for a, b in combinations(declarations, 2)
        if a.type_fingerprint == b.type_fingerprint
    }
    count = 1
    for first, second in combinations(declarations, 2):
        if frozenset({first.name, second.name}) in exact_pairs:
            continue
        score = _near_score(first, second)
        if score < 0.78:
            continue
        groups.append(
            _group(
                group_id=f"near-statement-{count}",
                kind=DuplicateKind.NEAR_STATEMENT,
                confidence=round(score, 2),
                reason="high overlap in statement constants, head symbols, and names",
                declarations=[first, second],
            )
        )
        count += 1
    return groups


def _statement_declarations(declarations: tuple[Declaration, ...]) -> tuple[Declaration, ...]:
    return tuple(declaration for declaration in declarations if declaration.kind in {"theorem", "axiom"})


def _near_score(first: Declaration, second: Declaration) -> float:
    constants = _jaccard(set(first.constants), set(second.constants))
    heads = 1.0 if set(first.type_heads) == set(second.type_heads) and first.type_heads else 0.0
    names = _jaccard(_name_tokens(first.short_name), _name_tokens(second.short_name))
    conclusion = 1.0 if first.conclusion_fingerprint == second.conclusion_fingerprint else 0.0
    return constants * 0.5 + heads * 0.2 + names * 0.1 + conclusion * 0.2


def _jaccard(first: set[str], second: set[str]) -> float:
    if not first and not second:
        return 0.0
    return len(first & second) / len(first | second)


def _name_tokens(name: str) -> set[str]:
    return {part for part in name.replace("'", "").split("_") if part}


def _group(
    *,
    group_id: str,
    kind: DuplicateKind,
    confidence: float,
    reason: str,
    declarations: list[Declaration],
) -> DuplicateGroup:
    members = tuple(
        DuplicateMember(
            name=declaration.name,
            module=declaration.module,
            file=declaration.file,
            line=declaration.span.start.line,
            kind=declaration.kind,
            type_text=declaration.type_text,
        )
        for declaration in sorted(declarations, key=lambda item: (str(item.file), item.span.start.line, item.name))
    )
    return DuplicateGroup(
        id=group_id,
        kind=kind,
        confidence=confidence,
        reason=reason,
        members=members,
    )
