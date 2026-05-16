"""Duplicate classification over Lean declarations."""

from __future__ import annotations

import sys
import time
from collections import Counter, defaultdict
from collections.abc import Mapping, Sequence
from itertools import combinations
from pathlib import Path

from lean_dup.candidates import external_near_candidates, local_near_candidates
from lean_dup.external_index import ExternalIndex, load_external_indexes
from lean_dup.extractor import load_or_build_index
from lean_dup.features import namespace_family, pair_signal_score, useful_name_tokens
from lean_dup.matching import MAX_BUCKET_SIZE, jaccard
from lean_dup.models import (
    AuditOptions,
    AuditReport,
    Declaration,
    DuplicateGroup,
    DuplicateKind,
    DuplicateMember,
)
from lean_dup.ranking import rank_group
from lean_dup.semantic_probes import probe_candidate_groups
from lean_dup.workspace import Workspace, resolve_workspace

SUBSUMPTION_MIN_CONSTANTS = 0.50
SUBSUMPTION_MIN_NAME_TOKENS = 0.30


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
            compare_indexes=options.compare_indexes,
            compare_mathlib=options.compare_mathlib,
            mathlib_workspace=options.mathlib_workspace,
            threshold=options.threshold,
            profile=options.profile,
            progress=options.progress,
            include_generated=options.include_generated,
            show_noise=options.show_noise,
            min_priority=options.min_priority,
            semantic_probes=options.semantic_probes,
        )
    if resolved_options.progress:
        _log(f"lean-dup: resolving audit workspace {resolved_options.workspace}")
    resolved = resolve_workspace(
        resolved_options.workspace,
        resolved_options.module_root,
        include_imports=resolved_options.include_imports,
        import_roots=resolved_options.import_roots,
    )
    if resolved_options.progress:
        _log(
            "lean-dup: auditing "
            f"{len(resolved.workspace_modules)} workspace module(s), "
            f"{len(resolved.extraction_modules)} extraction module(s)"
        )
    extracted = load_or_build_index(resolved, resolved_options)
    if resolved_options.progress:
        cache = "hit" if extracted.cache_hit else "miss"
        _log(
            f"lean-dup: workspace index {cache}; loaded {len(extracted.declarations)} declaration row(s)"
        )
    external_indexes, external_metadata, external_warnings = load_external_indexes(
        references=resolved_options.compare_indexes,
        compare_mathlib=resolved_options.compare_mathlib,
        mathlib_workspace=resolved_options.mathlib_workspace,
        profile=resolved_options.profile,
    )
    declarations = _filter_declarations(extracted.declarations, resolved_options)
    classified = _classify(
        workspace=resolved,
        declarations=declarations,
        external_indexes=external_indexes,
        semantic_probes=resolved_options.semantic_probes,
        threshold=resolved_options.threshold,
        progress=resolved_options.progress,
    )
    warnings = (*external_warnings, *classified.warnings)
    if resolved_options.profile:
        warnings += tuple(f"profile.{key}={value:.3f}s" for key, value in extracted.timings.items())
        warnings += tuple(
            f"profile.{key}={value:.3f}s" for key, value in classified.timings.items()
        )
    return AuditReport(
        workspace=resolved.root,
        module_root=resolved_options.module_root,
        declaration_count=len(declarations)
        + sum(metadata.declaration_count for metadata in external_metadata),
        cache_hit=extracted.cache_hit,
        external_indexes=external_metadata,
        warnings=warnings,
        groups=tuple(classified.groups),
    )


class ClassifiedGroups:
    """Internal classifier result."""

    def __init__(
        self,
        *,
        groups: Sequence[DuplicateGroup],
        warnings: Sequence[str],
        timings: Mapping[str, float],
    ) -> None:
        self.groups = groups
        self.warnings = warnings
        self.timings = timings


def _filter_declarations(
    declarations: Sequence[Declaration],
    options: AuditOptions,
) -> tuple[Declaration, ...]:
    if options.include_private:
        return tuple(declarations)
    return tuple(declaration for declaration in declarations if declaration.visibility != "private")


def _classify(
    workspace: Workspace,
    declarations: tuple[Declaration, ...],
    *,
    external_indexes: tuple[ExternalIndex, ...],
    semantic_probes: bool,
    threshold: float,
    progress: bool = False,
) -> ClassifiedGroups:
    started = time.perf_counter()
    warnings: list[str] = []
    groups: list[DuplicateGroup] = []
    used_exact_pairs: set[frozenset[str]] = set()
    group_count: Counter[DuplicateKind] = Counter()
    declaration_by_key = {
        _declaration_key(declaration): declaration for declaration in declarations
    }
    workspace_declarations = tuple(
        declaration for declaration in declarations if _is_workspace(declaration)
    )
    local_statements = tuple(_statement_declarations(declarations))
    workspace_statements = tuple(_statement_declarations(workspace_declarations))
    if progress:
        _log(
            "lean-dup: classifying "
            f"{len(workspace_statements)} workspace statement(s) "
            f"against {len(external_indexes)} external index(es)"
        )

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
        if progress:
            _log(f"lean-dup: fingerprint pass {kind}")
        keyed_groups = _fingerprint_groups(
            declarations=local_statements,
            key_name=key_name,
            kind=kind,
            confidence=confidence,
            reason=reason,
            warnings=warnings,
            group_count=group_count,
            declaration_by_name=declaration_by_key,
        )
        groups.extend(keyed_groups)
        external_groups = _external_fingerprint_groups(
            workspace_declarations=workspace_statements,
            external_indexes=external_indexes,
            key_name=key_name,
            kind=kind,
            confidence=confidence,
            reason=reason,
            warnings=warnings,
            group_count=group_count,
            declaration_by_name=declaration_by_key,
        )
        groups.extend(external_groups)
        if kind is DuplicateKind.EXACT_STATEMENT:
            used_exact_pairs.update(_pairs_for_group(group) for group in keyed_groups)
            used_exact_pairs.update(_pairs_for_group(group) for group in external_groups)

    source_groups = _fingerprint_groups(
        declarations=tuple(d for d in workspace_declarations if d.source_fingerprint),
        key_name="source_fingerprint",
        kind=DuplicateKind.SOURCE_CLONE,
        confidence=0.9,
        reason="same local declaration source skeleton after comment and whitespace normalization",
        warnings=warnings,
        group_count=group_count,
        declaration_by_name=declaration_by_key,
    )
    groups.extend(_groups_with_workspace_member(source_groups))
    if progress:
        _log("lean-dup: near-statement pass")

    near_groups = _near_statement_groups(
        declarations=local_statements,
        threshold=threshold,
        exact_pairs=used_exact_pairs,
        warnings=warnings,
        group_count=group_count,
        progress=progress,
        declaration_by_name=declaration_by_key,
    )
    groups.extend(near_groups)
    near_external_groups = _near_statement_groups_against_external(
        workspace_declarations=workspace_statements,
        external_indexes=external_indexes,
        threshold=threshold,
        exact_pairs=used_exact_pairs,
        warnings=warnings,
        group_count=group_count,
        progress=progress,
        declaration_by_name=declaration_by_key,
    )
    groups.extend(near_external_groups)
    if progress:
        _log(f"lean-dup: suppressing redundant groups from {len(groups)} candidate group(s)")
    groups = _suppress_redundant_groups(groups)
    probe_results = probe_candidate_groups(
        workspace=workspace,
        groups=groups,
        declarations_by_name=declaration_by_key,
        enabled=semantic_probes,
        progress=progress,
    )
    groups = [
        rank_group(
            group,
            tuple(
                declaration_by_key[_member_key(member)]
                for member in group.members
                if _member_key(member) in declaration_by_key
            ),
            probe_results=probe_results,
        )
        for group in groups
    ]
    groups.sort(key=lambda group: (-group.confidence, _kind_priority(group.kind), group.id))
    if progress:
        _log(f"lean-dup: classification complete; {len(groups)} group(s)")
    return ClassifiedGroups(
        groups=groups,
        warnings=_dedupe_warnings(warnings),
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
    declaration_by_name: dict[str, Declaration],
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
        if not any(_is_workspace(declaration) for declaration in members):
            continue
        if len(members) > MAX_BUCKET_SIZE:
            warnings.append(
                f"pruned {kind} bucket {key}: {len(members)} declarations exceeds {MAX_BUCKET_SIZE}"
            )
            continue
        group_count[kind] += 1
        group_reason = reason
        if kind is DuplicateKind.EXACT_STATEMENT and any(
            not _is_workspace(declaration) for declaration in members
        ):
            group_reason = (
                "workspace declaration matches an external elaborated statement fingerprint"
            )
        groups.append(
            _group(
                group_id=f"{kind}-{group_count[kind]}",
                kind=kind,
                confidence=confidence,
                reason=group_reason,
                evidence=(f"{key_name}={key}",),
                declarations=members,
                declaration_by_name=declaration_by_name,
            )
        )
    return groups


def _external_fingerprint_groups(
    *,
    workspace_declarations: tuple[Declaration, ...],
    external_indexes: tuple[ExternalIndex, ...],
    key_name: str,
    kind: DuplicateKind,
    confidence: float,
    reason: str,
    warnings: list[str],
    group_count: Counter[DuplicateKind],
    declaration_by_name: dict[str, Declaration],
) -> list[DuplicateGroup]:
    workspace_buckets: dict[str, list[Declaration]] = defaultdict(list)
    for declaration in workspace_declarations:
        key = getattr(declaration, key_name)
        if key:
            workspace_buckets[key].append(declaration)
    if not workspace_buckets:
        return []

    groups: list[DuplicateGroup] = []
    for external_index in external_indexes:
        external_buckets = external_index.fingerprint_matches(
            key_name=key_name,
            keys=workspace_buckets.keys(),
        )
        for key, external_members in external_buckets.items():
            members = [*workspace_buckets[key], *external_members]
            if kind is DuplicateKind.SUBSUMPTION_CANDIDATE:
                members = _strict_subsumption_members(workspace_buckets[key], external_members)
                if len(members) < 2:
                    continue
            if len(members) > MAX_BUCKET_SIZE:
                warnings.append(
                    f"pruned {kind} external bucket {key}: {len(members)} declarations exceeds {MAX_BUCKET_SIZE}"
                )
                continue
            group_count[kind] += 1
            group_reason = reason
            if kind is DuplicateKind.EXACT_STATEMENT:
                group_reason = (
                    "workspace declaration matches an external elaborated statement fingerprint"
                )
            groups.append(
                _group(
                    group_id=f"{kind}-{group_count[kind]}",
                    kind=kind,
                    confidence=confidence,
                    reason=group_reason,
                    evidence=(f"{key_name}={key}",),
                    declarations=members,
                    declaration_by_name=declaration_by_name,
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
    progress: bool,
    declaration_by_name: dict[str, Declaration],
) -> list[DuplicateGroup]:
    candidates = local_near_candidates(declarations, warnings=warnings, progress=progress)
    if progress:
        _log(f"lean-dup: scoring {len(candidates)} local near candidate pair(s)")
    scored: list[tuple[float, Declaration, Declaration, tuple[str, ...]]] = []
    for first, second in candidates:
        pair_key = frozenset({first.name, second.name})
        if pair_key in exact_pairs:
            continue
        score, evidence = pair_signal_score(first, second)
        if score >= threshold:
            scored.append((score, first, second, evidence))
    return _cluster_scored_pairs(
        scored,
        group_count=group_count,
        declaration_by_name=declaration_by_name,
    )


def _near_statement_groups_against_external(
    *,
    workspace_declarations: tuple[Declaration, ...],
    external_indexes: tuple[ExternalIndex, ...],
    threshold: float,
    exact_pairs: set[frozenset[str]],
    warnings: list[str],
    group_count: Counter[DuplicateKind],
    progress: bool,
    declaration_by_name: dict[str, Declaration],
) -> list[DuplicateGroup]:
    candidates = external_near_candidates(
        workspace_declarations=workspace_declarations,
        external_indexes=external_indexes,
        warnings=warnings,
        progress=progress,
    )
    if progress:
        _log(f"lean-dup: scoring {len(candidates)} external near candidate pair(s)")
    scored: list[tuple[float, Declaration, Declaration, tuple[str, ...]]] = []
    for first, second in candidates:
        pair_key = frozenset({first.name, second.name})
        if pair_key in exact_pairs:
            continue
        score, evidence = pair_signal_score(first, second)
        if score >= threshold:
            scored.append((score, first, second, evidence))
    return _cluster_scored_pairs(
        scored,
        group_count=group_count,
        declaration_by_name=declaration_by_name,
    )


def _cluster_scored_pairs(
    scored: list[tuple[float, Declaration, Declaration, tuple[str, ...]]],
    *,
    group_count: Counter[DuplicateKind],
    declaration_by_name: dict[str, Declaration],
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
        if not any(_is_workspace(declaration) for declaration in members):
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
                declaration_by_name=declaration_by_name,
            )
        )
    return groups


def _strict_subsumption_members(
    workspace_members: list[Declaration],
    external_members: tuple[Declaration, ...],
) -> list[Declaration]:
    members: list[Declaration] = []
    for workspace in workspace_members:
        matched = [
            external
            for external in external_members
            if _is_meaningful_subsumption_candidate(workspace, external)
        ]
        if matched:
            members.append(workspace)
            members.extend(matched)
    return list(dict.fromkeys(members))


def _is_meaningful_subsumption_candidate(workspace: Declaration, external: Declaration) -> bool:
    constants = jaccard(set(workspace.constants), set(external.constants))
    if constants >= SUBSUMPTION_MIN_CONSTANTS:
        return True
    heads = bool(set(workspace.type_heads) & set(external.type_heads))
    names = jaccard(useful_name_tokens(workspace), useful_name_tokens(external))
    if heads and names >= SUBSUMPTION_MIN_NAME_TOKENS:
        return True
    namespace_tail_matches = _namespace_tail(workspace.name) == _namespace_tail(external.name)
    return namespace_tail_matches and abs(workspace.binder_count - external.binder_count) <= 1


def _suppress_redundant_groups(groups: list[DuplicateGroup]) -> list[DuplicateGroup]:
    covered: list[frozenset[str]] = []
    kept: list[DuplicateGroup] = []
    for group in sorted(
        groups,
        key=lambda item: (
            _kind_priority(item.kind),
            _external_rank(item),
            -item.confidence,
            item.id,
        ),
    ):
        workspace_names = frozenset(
            member.name for member in group.members if member.origin == "workspace"
        )
        if (
            group.kind is not DuplicateKind.SOURCE_CLONE
            and workspace_names
            and any(workspace_names <= existing for existing in covered)
        ):
            continue
        kept.append(group)
        if workspace_names:
            covered.append(workspace_names)
    return kept


def _external_rank(group: DuplicateGroup) -> int:
    return 0 if any(member.origin != "workspace" for member in group.members) else 1


def _kind_priority(kind: DuplicateKind) -> int:
    return {
        DuplicateKind.EXACT_STATEMENT: 0,
        DuplicateKind.PERMUTED_STATEMENT: 1,
        DuplicateKind.CONNECTIVE_EQUIVALENT: 2,
        DuplicateKind.NEAR_STATEMENT: 3,
        DuplicateKind.SUBSUMPTION_CANDIDATE: 4,
        DuplicateKind.SOURCE_CLONE: 5,
    }[kind]


def _dedupe_warnings(warnings: list[str]) -> tuple[str, ...]:
    return tuple(dict.fromkeys(warnings))


def _statement_declarations(declarations: tuple[Declaration, ...]) -> tuple[Declaration, ...]:
    return tuple(
        declaration for declaration in declarations if declaration.kind in {"theorem", "axiom"}
    )


def _groups_with_workspace_member(groups: list[DuplicateGroup]) -> list[DuplicateGroup]:
    return [
        group for group in groups if any(member.origin == "workspace" for member in group.members)
    ]


def _is_workspace(declaration: Declaration) -> bool:
    return declaration.origin == "workspace"


def _declaration_key(declaration: Declaration) -> str:
    return "\0".join(
        (
            declaration.origin,
            declaration.name,
            str(declaration.file),
            str(declaration.span.start.line),
        )
    )


def _member_key(member: DuplicateMember) -> str:
    return "\0".join((member.origin, member.name, str(member.file), str(member.line)))


def _namespace_tail(name: str) -> str:
    return namespace_family(name, depth=2)


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
    declaration_by_name: dict[str, Declaration],
) -> DuplicateGroup:
    for declaration in declarations:
        declaration_by_name[_declaration_key(declaration)] = declaration
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
        for declaration in sorted(
            declarations, key=lambda item: (str(item.file), item.span.start.line, item.name)
        )
    )
    return DuplicateGroup(
        id=group_id,
        kind=kind,
        confidence=confidence,
        reason=reason,
        evidence=evidence,
        members=members,
    )


def _log(message: str) -> None:
    print(message, file=sys.stderr, flush=True)
