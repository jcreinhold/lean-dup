"""Duplicate classification over Lean declarations."""

from __future__ import annotations

import time
from collections import Counter, defaultdict
from itertools import combinations
from pathlib import Path

from lean_dup.extractor import load_or_build_index
from lean_dup.models import (
    AuditOptions,
    AuditReport,
    Declaration,
    DuplicateGroup,
    DuplicateKind,
    DuplicateMember,
)
from lean_dup.workspace import resolve_workspace

MAX_BUCKET_SIZE = 400
MAX_NEAR_CANDIDATES_PER_DECLARATION = 350


def run_audit(
    *,
    workspace: Path,
    module_root: str | None = None,
    options: AuditOptions | None = None,
) -> AuditReport:
    """Audit one Lake workspace for duplicated Lean declarations."""

    resolved_options = options or AuditOptions(workspace=workspace, module_root=module_root)
    if module_root is not None and options is not None and options.module_root != module_root:
        resolved_options = AuditOptions(
            workspace=options.workspace,
            module_root=module_root,
            include_private=options.include_private,
            include_imports=options.include_imports,
            import_roots=options.import_roots,
            threshold=options.threshold,
            profile=options.profile,
        )
    resolved = resolve_workspace(
        resolved_options.workspace,
        resolved_options.module_root,
        include_imports=resolved_options.include_imports,
        import_roots=resolved_options.import_roots,
    )
    extracted = load_or_build_index(resolved, resolved_options)
    declarations = _filter_declarations(extracted.declarations, resolved_options)
    classified = _classify(declarations, threshold=resolved_options.threshold)
    warnings = tuple(classified.warnings)
    if resolved_options.profile:
        warnings += tuple(f"profile.{key}={value:.3f}s" for key, value in extracted.timings.items())
        warnings += tuple(f"profile.{key}={value:.3f}s" for key, value in classified.timings.items())
    return AuditReport(
        workspace=resolved.root,
        module_root=resolved_options.module_root,
        declaration_count=len(declarations),
        cache_hit=extracted.cache_hit,
        warnings=warnings,
        groups=tuple(classified.groups),
    )


class ClassifiedGroups:
    """Internal classifier result."""

    def __init__(
        self,
        *,
        groups: list[DuplicateGroup],
        warnings: list[str],
        timings: dict[str, float],
    ) -> None:
        self.groups = groups
        self.warnings = warnings
        self.timings = timings


def _filter_declarations(
    declarations: tuple[Declaration, ...],
    options: AuditOptions,
) -> tuple[Declaration, ...]:
    if options.include_private:
        return declarations
    return tuple(declaration for declaration in declarations if declaration.visibility != "private")


def _classify(declarations: tuple[Declaration, ...], *, threshold: float) -> ClassifiedGroups:
    started = time.perf_counter()
    warnings: list[str] = []
    groups: list[DuplicateGroup] = []
    used_exact_pairs: set[frozenset[str]] = set()
    group_count: Counter[DuplicateKind] = Counter()

    for kind, key_name, confidence, reason in (
        (
            DuplicateKind.EXACT_STATEMENT,
            "type_fingerprint",
            1.0,
            "same elaborated statement fingerprint after binder-name erasure",
        ),
        (
            DuplicateKind.PERMUTED_STATEMENT,
            "permutation_fingerprint",
            0.94,
            "same statement after safe binder/connective canonicalization",
        ),
        (
            DuplicateKind.CONNECTIVE_EQUIVALENT,
            "connective_fingerprint",
            0.9,
            "same statement after safe commutative connective normalization",
        ),
        (
            DuplicateKind.SUBSUMPTION_CANDIDATE,
            "conclusion_fingerprint",
            0.78,
            "same conclusion shape with compatible statement features",
        ),
    ):
        keyed_groups = _fingerprint_groups(
            declarations=tuple(_statement_declarations(declarations)),
            key_name=key_name,
            kind=kind,
            confidence=confidence,
            reason=reason,
            warnings=warnings,
            group_count=group_count,
        )
        groups.extend(keyed_groups)
        if kind is DuplicateKind.EXACT_STATEMENT:
            used_exact_pairs.update(_pairs_for_group(group) for group in keyed_groups)

    source_groups = _fingerprint_groups(
        declarations=tuple(d for d in declarations if d.origin == "workspace" and d.source_fingerprint),
        key_name="source_fingerprint",
        kind=DuplicateKind.SOURCE_CLONE,
        confidence=0.9,
        reason="same local declaration source skeleton after comment and whitespace normalization",
        warnings=warnings,
        group_count=group_count,
    )
    groups.extend(source_groups)

    near_groups = _near_statement_groups(
        declarations=tuple(_statement_declarations(declarations)),
        threshold=threshold,
        exact_pairs=used_exact_pairs,
        warnings=warnings,
        group_count=group_count,
    )
    groups.extend(near_groups)
    groups.sort(key=lambda group: (-group.confidence, str(group.kind), group.id))
    return ClassifiedGroups(
        groups=groups,
        warnings=warnings,
        timings={"classify": time.perf_counter() - started},
    )


def _fingerprint_groups(
    *,
    declarations: tuple[Declaration, ...],
    key_name: str,
    kind: DuplicateKind,
    confidence: float,
    reason: str,
    warnings: list[str],
    group_count: Counter[DuplicateKind],
) -> list[DuplicateGroup]:
    buckets: dict[str, list[Declaration]] = defaultdict(list)
    for declaration in declarations:
        key = getattr(declaration, key_name)
        if key:
            buckets[key].append(declaration)
    groups: list[DuplicateGroup] = []
    for key, members in buckets.items():
        if len(members) < 2:
            continue
        if len(members) > MAX_BUCKET_SIZE:
            warnings.append(
                f"pruned {kind} bucket {key}: {len(members)} declarations exceeds {MAX_BUCKET_SIZE}"
            )
            continue
        group_count[kind] += 1
        groups.append(
            _group(
                group_id=f"{kind}-{group_count[kind]}",
                kind=kind,
                confidence=confidence,
                reason=reason,
                evidence=(f"{key_name}={key}",),
                declarations=members,
            )
        )
    return groups


def _near_statement_groups(
    *,
    declarations: tuple[Declaration, ...],
    threshold: float,
    exact_pairs: set[frozenset[str]],
    warnings: list[str],
    group_count: Counter[DuplicateKind],
) -> list[DuplicateGroup]:
    candidates = _near_candidates(declarations, warnings=warnings)
    scored: list[tuple[float, Declaration, Declaration, tuple[str, ...]]] = []
    for first, second in candidates:
        pair_key = frozenset({first.name, second.name})
        if pair_key in exact_pairs:
            continue
        score, evidence = _near_score(first, second)
        if score >= threshold:
            scored.append((score, first, second, evidence))
    return _cluster_scored_pairs(scored, group_count=group_count)


def _near_candidates(
    declarations: tuple[Declaration, ...],
    *,
    warnings: list[str],
) -> set[tuple[Declaration, Declaration]]:
    constants_by_decl = {declaration.name: set(declaration.constants) for declaration in declarations}
    frequency = Counter(constant for constants in constants_by_decl.values() for constant in constants)
    index: dict[str, list[Declaration]] = defaultdict(list)
    for declaration in declarations:
        for constant in constants_by_decl[declaration.name]:
            if frequency[constant] <= 1 or frequency[constant] > MAX_BUCKET_SIZE:
                continue
            index[f"const:{constant}"].append(declaration)
        for head in declaration.type_heads:
            index[f"head:{head}"].append(declaration)
        for token in _name_tokens(declaration.short_name):
            index[f"name:{token}"].append(declaration)
        index[f"conclusion:{declaration.conclusion_fingerprint}"].append(declaration)

    candidates: set[tuple[Declaration, Declaration]] = set()
    seen_for_decl: dict[str, int] = defaultdict(int)
    for key, members in index.items():
        unique = tuple(dict.fromkeys(members))
        if len(unique) > MAX_BUCKET_SIZE:
            warnings.append(f"pruned near bucket {key}: {len(unique)} declarations exceeds {MAX_BUCKET_SIZE}")
            continue
        for first, second in combinations(sorted(unique, key=lambda item: item.name), 2):
            if seen_for_decl[first.name] > MAX_NEAR_CANDIDATES_PER_DECLARATION:
                continue
            if seen_for_decl[second.name] > MAX_NEAR_CANDIDATES_PER_DECLARATION:
                continue
            candidates.add((first, second))
            seen_for_decl[first.name] += 1
            seen_for_decl[second.name] += 1
    return candidates


def _cluster_scored_pairs(
    scored: list[tuple[float, Declaration, Declaration, tuple[str, ...]]],
    *,
    group_count: Counter[DuplicateKind],
) -> list[DuplicateGroup]:
    if not scored:
        return []
    parent: dict[str, str] = {}
    declarations: dict[str, Declaration] = {}
    pair_evidence: dict[frozenset[str], tuple[float, tuple[str, ...]]] = {}

    def find(name: str) -> str:
        parent.setdefault(name, name)
        if parent[name] != name:
            parent[name] = find(parent[name])
        return parent[name]

    def union(left: str, right: str) -> None:
        left_root = find(left)
        right_root = find(right)
        if left_root != right_root:
            parent[right_root] = left_root

    for score, first, second, evidence in scored:
        declarations[first.name] = first
        declarations[second.name] = second
        union(first.name, second.name)
        pair_evidence[frozenset({first.name, second.name})] = (score, evidence)

    by_root: dict[str, list[Declaration]] = defaultdict(list)
    for declaration in declarations.values():
        by_root[find(declaration.name)].append(declaration)

    groups: list[DuplicateGroup] = []
    for members in by_root.values():
        if len(members) < 2:
            continue
        scores: list[float] = []
        evidence_parts: list[str] = []
        for first, second in combinations(members, 2):
            score_evidence = pair_evidence.get(frozenset({first.name, second.name}))
            if score_evidence is None:
                continue
            score, evidence = score_evidence
            scores.append(score)
            evidence_parts.extend(evidence)
        group_count[DuplicateKind.NEAR_STATEMENT] += 1
        groups.append(
            _group(
                group_id=f"{DuplicateKind.NEAR_STATEMENT}-{group_count[DuplicateKind.NEAR_STATEMENT]}",
                kind=DuplicateKind.NEAR_STATEMENT,
                confidence=round(max(scores or [0.0]), 2),
                reason="high overlap in statement constants, head symbols, names, or conclusions",
                evidence=tuple(sorted(set(evidence_parts)))[:8],
                declarations=members,
            )
        )
    return groups


def _statement_declarations(declarations: tuple[Declaration, ...]) -> tuple[Declaration, ...]:
    return tuple(declaration for declaration in declarations if declaration.kind in {"theorem", "axiom"})


def _near_score(first: Declaration, second: Declaration) -> tuple[float, tuple[str, ...]]:
    evidence: list[str] = []
    constants = _jaccard(set(first.constants), set(second.constants))
    if constants > 0:
        evidence.append(f"constants={constants:.2f}")
    heads = 1.0 if set(first.type_heads) == set(second.type_heads) and first.type_heads else 0.0
    if heads:
        evidence.append("same-heads")
    names = _jaccard(_name_tokens(first.short_name), _name_tokens(second.short_name))
    if names > 0:
        evidence.append(f"name-tokens={names:.2f}")
    conclusion = 1.0 if first.conclusion_fingerprint == second.conclusion_fingerprint else 0.0
    if conclusion:
        evidence.append("same-conclusion")
    permutation = 1.0 if first.permutation_fingerprint == second.permutation_fingerprint else 0.0
    if permutation:
        evidence.append("same-permutation-fingerprint")
    connective = 1.0 if first.connective_fingerprint == second.connective_fingerprint else 0.0
    if connective:
        evidence.append("same-connective-fingerprint")
    namespace = 1.0 if first.name.rsplit(".", 1)[0] == second.name.rsplit(".", 1)[0] else 0.0
    score = (
        constants * 0.36
        + heads * 0.12
        + names * 0.08
        + conclusion * 0.16
        + permutation * 0.16
        + connective * 0.08
        + namespace * 0.04
    )
    return score, tuple(evidence)


def _jaccard(first: set[str], second: set[str]) -> float:
    if not first and not second:
        return 0.0
    return len(first & second) / len(first | second)


def _name_tokens(name: str) -> set[str]:
    return {part for part in name.replace("'", "").split("_") if part}


def _pairs_for_group(group: DuplicateGroup) -> frozenset[str]:
    return frozenset(member.name for member in group.members)


def _group(
    *,
    group_id: str,
    kind: DuplicateKind,
    confidence: float,
    reason: str,
    evidence: tuple[str, ...],
    declarations: list[Declaration],
) -> DuplicateGroup:
    members = tuple(
        DuplicateMember(
            name=declaration.name,
            display_name=declaration.display_name,
            module=declaration.module,
            file=declaration.file,
            line=declaration.span.start.line,
            kind=declaration.kind,
            visibility=declaration.visibility,
            origin=declaration.origin,
            type_text=declaration.type_text,
        )
        for declaration in sorted(declarations, key=lambda item: (str(item.file), item.span.start.line, item.name))
    )
    return DuplicateGroup(
        id=group_id,
        kind=kind,
        confidence=confidence,
        reason=reason,
        evidence=evidence,
        members=members,
    )
