from __future__ import annotations

import json
import sqlite3
from pathlib import Path

from lean_dup.audit import run_audit
from lean_dup.cli import _render_report
from lean_dup.external_index import build_external_index
from lean_dup.models import (
    AuditOptions,
    Declaration,
    DuplicateKind,
    ReviewPriority,
    SourcePoint,
    SourceSpan,
)
from lean_dup.semantic_probes import ProbePair, _ProbeCache


FIXTURE = Path(__file__).parent / "fixtures" / "tiny"
EXTERNAL = Path(__file__).parent / "fixtures" / "external"


def test_tiny_workspace_reports_expected_groups() -> None:
    report = run_audit(workspace=FIXTURE, module_root="Tiny")
    kinds = {group.kind for group in report.groups}
    assert DuplicateKind.EXACT_STATEMENT in kinds
    assert DuplicateKind.PERMUTED_STATEMENT in kinds
    assert DuplicateKind.CONNECTIVE_EQUIVALENT in kinds
    assert DuplicateKind.SOURCE_CLONE in kinds
    assert all(group.recommended_action for group in report.groups)
    assert all(group.review_priority in set(ReviewPriority) for group in report.groups)
    assert report.declaration_count >= 10
    assert not any(
        group.kind is DuplicateKind.PERMUTED_STATEMENT
        and {"dependent_left", "dependent_right"}
        <= {member.display_name for member in group.members}
        for group in report.groups
    )


def test_full_workspace_audit_without_module() -> None:
    report = run_audit(workspace=FIXTURE)
    assert report.declaration_count >= 10
    assert report.groups


def test_private_theorems_included_by_default_and_excluded_by_public_only() -> None:
    default_report = run_audit(workspace=FIXTURE, module_root="Tiny")
    assert any(
        member.visibility == "private"
        for group in default_report.groups
        for member in group.members
    )
    public_report = run_audit(
        workspace=FIXTURE,
        options=AuditOptions(workspace=FIXTURE, module_root="Tiny", include_private=False),
    )
    assert not any(
        member.visibility == "private" for group in public_report.groups for member in group.members
    )


def test_direct_imports_are_optional() -> None:
    default_report = run_audit(workspace=FIXTURE, module_root="Tiny")
    assert not any(
        member.origin == "direct-import"
        for group in default_report.groups
        for member in group.members
    )
    import_report = run_audit(
        workspace=FIXTURE,
        options=AuditOptions(workspace=FIXTURE, module_root="Tiny", include_imports=True),
    )
    assert any(
        member.origin == "direct-import"
        for group in import_report.groups
        for member in group.members
    )


def test_cache_is_reused_on_second_run() -> None:
    run_audit(workspace=FIXTURE, module_root="Tiny")
    report = run_audit(workspace=FIXTURE, module_root="Tiny")
    assert report.cache_hit


def test_external_index_reports_workspace_matches(monkeypatch, tmp_path) -> None:
    monkeypatch.setenv("LEAN_DUP_CACHE_DIR", str(tmp_path))
    first = build_external_index(workspace=EXTERNAL, module_root="External", label="fixture")
    second = build_external_index(workspace=EXTERNAL, module_root="External", label="fixture")
    assert not first.cache_hit
    assert second.cache_hit
    assert first.path.name == "index.sqlite"
    assert first.path.parent.name
    assert first.path.exists()
    assert not (first.path.parent / "declarations.jsonl.gz").exists()
    assert not (first.path.parent / "buckets.sqlite").exists()
    with sqlite3.connect(first.path) as connection:
        metadata = dict(connection.execute("SELECT key, value FROM metadata").fetchall())
    assert metadata["schema_version"] == "external-index.sqlite.v1"
    latest = json.loads(
        (tmp_path / "indexes" / "fixture" / "latest.json").read_text(encoding="utf-8")
    )
    assert Path(latest["index_dir"]) == first.path.parent

    report = run_audit(
        workspace=FIXTURE,
        options=AuditOptions(
            workspace=FIXTURE,
            module_root="Tiny",
            compare_indexes=("fixture",),
        ),
    )

    assert report.external_indexes
    assert report.external_indexes[0].label == "fixture"
    assert any(
        member.origin == "external:fixture" for group in report.groups for member in group.members
    )
    assert any(
        group.kind is DuplicateKind.EXACT_STATEMENT
        and any(member.display_name == "same_as_tiny" for member in group.members)
        and group.recommended_action == "local-alias"
        and group.review_priority is ReviewPriority.HIGH
        and group.recommended_target == "External.same_as_tiny"
        and "probe:same-statement" in group.signals
        for group in report.groups
    )
    assert not any(
        group.kind in {DuplicateKind.PERMUTED_STATEMENT, DuplicateKind.CONNECTIVE_EQUIVALENT}
        and any(
            member.display_name == "same_as_tiny" and member.origin != "workspace"
            for member in group.members
        )
        for group in report.groups
    )
    assert not any(
        group.kind is DuplicateKind.SUBSUMPTION_CANDIDATE
        and any(member.display_name == "impossible_tiny" for member in group.members)
        and any(member.origin != "workspace" for member in group.members)
        for group in report.groups
    )
    assert not any(
        group.members and all(member.origin != "workspace" for member in group.members)
        for group in report.groups
    )

    for reference in (str(first.path.parent), str(first.path)):
        path_report = run_audit(
            workspace=FIXTURE,
            options=AuditOptions(
                workspace=FIXTURE,
                module_root="Tiny",
                compare_indexes=(reference,),
            ),
        )
        assert path_report.external_indexes[0].path == first.path


def test_mathlib_labeled_external_match_gets_action(monkeypatch, tmp_path) -> None:
    monkeypatch.setenv("LEAN_DUP_CACHE_DIR", str(tmp_path))
    build_external_index(workspace=EXTERNAL, module_root="External", label="mathlib")
    report = run_audit(
        workspace=FIXTURE,
        options=AuditOptions(
            workspace=FIXTURE,
            module_root="Tiny",
            compare_indexes=("mathlib",),
        ),
    )
    assert any(
        group.kind is DuplicateKind.EXACT_STATEMENT
        and group.recommended_action == "already-in-mathlib"
        and group.review_priority is ReviewPriority.HIGH
        and group.recommended_target == "External.same_as_tiny"
        and group.replacement_hint is not None
        and group.replacement_hint.target_decl == "External.same_as_tiny"
        and group.replacement_hint.target_module == "External.Basic"
        and group.replacement_hint.import_line == "import External.Basic"
        and group.replacement_hint.import_status == "missing"
        and group.replacement_hint.action in {"replace-with-import", "replace-local-uses"}
        and "same-imported-mathlib-declaration" in group.signals
        and any(member.origin == "mathlib" for member in group.members)
        for group in report.groups
    )


def test_source_clones_ignore_external_indexes(monkeypatch, tmp_path) -> None:
    monkeypatch.setenv("LEAN_DUP_CACHE_DIR", str(tmp_path))
    build_external_index(workspace=EXTERNAL, module_root="External", label="fixture")
    report = run_audit(
        workspace=FIXTURE,
        options=AuditOptions(
            workspace=FIXTURE,
            module_root="Tiny",
            compare_indexes=("fixture",),
        ),
    )
    assert not any(
        group.kind is DuplicateKind.SOURCE_CLONE
        and any(member.origin != "workspace" for member in group.members)
        for group in report.groups
    )


def test_replacement_hints_can_be_disabled(monkeypatch, tmp_path) -> None:
    monkeypatch.setenv("LEAN_DUP_CACHE_DIR", str(tmp_path))
    build_external_index(workspace=EXTERNAL, module_root="External", label="mathlib")
    report = run_audit(
        workspace=FIXTURE,
        options=AuditOptions(
            workspace=FIXTURE,
            module_root="Tiny",
            compare_indexes=("mathlib",),
            replacement_hints=False,
        ),
    )
    assert not any(group.replacement_hint is not None for group in report.groups)


def test_text_report_includes_replacement_hint(monkeypatch, tmp_path) -> None:
    monkeypatch.setenv("LEAN_DUP_CACHE_DIR", str(tmp_path))
    build_external_index(workspace=EXTERNAL, module_root="External", label="mathlib")
    options = AuditOptions(workspace=FIXTURE, module_root="Tiny", compare_indexes=("mathlib",))
    report = run_audit(workspace=FIXTURE, options=options)
    text = _render_report(report, options=options)
    assert "replacement:" in text
    assert "import: import External.Basic (missing)" in text


def test_semantic_probe_cache_reuses_and_invalidates(monkeypatch, tmp_path) -> None:
    first = _declaration("Example.left", "aaa")
    second = _declaration("Example.right", "bbb")
    cache = _ProbeCache(tmp_path)
    pair = ProbePair(first=first, second=second)

    assert cache.get(pair) is None
    result_report = run_audit(
        workspace=FIXTURE,
        options=AuditOptions(workspace=FIXTURE, module_root="Tiny", semantic_probes=False),
    )
    cached_result = next(
        group for group in result_report.groups if group.kind is DuplicateKind.EXACT_STATEMENT
    )
    assert cached_result.probe_summary is not None

    from lean_dup.probes import ProbeResult

    cache.put(pair, ProbeResult(same_statement=True, source="lean"))
    assert cache.get(pair) is not None
    changed = ProbePair(first=_declaration("Example.left", "changed"), second=second)
    assert cache.get(changed) is None

    monkeypatch.setattr(
        "lean_dup.semantic_probes.PROBE_SCHEMA_VERSION", "semantic-probes.test-change"
    )
    assert cache.get(pair) is None


def test_include_imports_gets_lean_confirmed_probe() -> None:
    report = run_audit(
        workspace=FIXTURE,
        options=AuditOptions(workspace=FIXTURE, module_root="Tiny", include_imports=True),
    )
    assert any(
        group.kind is DuplicateKind.EXACT_STATEMENT
        and any(member.name == "Other.imported_dup" for member in group.members)
        and "probe:same-statement" in group.signals
        and group.probe_summary is not None
        and "lean:" in group.probe_summary
        and "lean-probe-unavailable" not in group.blockers
        for group in report.groups
    )


def _declaration(name: str, fingerprint: str) -> Declaration:
    short_name = name.rsplit(".", 1)[-1]
    return Declaration(
        workspace=Path("/tmp/workspace"),
        module="Example",
        name=name,
        display_name=short_name,
        short_name=short_name,
        kind="theorem",
        visibility="public",
        origin="workspace",
        modifiers=(),
        file=Path("/tmp/workspace/Example.lean"),
        span=SourceSpan(start=SourcePoint(line=1, column=1), end=SourcePoint(line=1, column=1)),
        type_text="True",
        normalized_type="True",
        type_fingerprint=fingerprint,
        permutation_fingerprint=fingerprint,
        connective_fingerprint=fingerprint,
        conclusion_fingerprint=fingerprint,
        constants=(),
        type_heads=("True",),
        binder_count=0,
        source_fingerprint=None,
    )
