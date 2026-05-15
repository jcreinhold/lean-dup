from __future__ import annotations

import json
import sqlite3
from pathlib import Path

from lean_dup.audit import run_audit
from lean_dup.external_index import build_external_index
from lean_dup.models import AuditOptions, DuplicateKind


FIXTURE = Path(__file__).parent / "fixtures" / "tiny"
EXTERNAL = Path(__file__).parent / "fixtures" / "external"


def test_tiny_workspace_reports_expected_groups() -> None:
    report = run_audit(workspace=FIXTURE, module_root="Tiny")
    kinds = {group.kind for group in report.groups}
    assert DuplicateKind.EXACT_STATEMENT in kinds
    assert DuplicateKind.PERMUTED_STATEMENT in kinds
    assert DuplicateKind.CONNECTIVE_EQUIVALENT in kinds
    assert DuplicateKind.SOURCE_CLONE in kinds
    assert report.declaration_count >= 10
    assert not any(
        group.kind is DuplicateKind.PERMUTED_STATEMENT
        and {"dependent_left", "dependent_right"} <= {member.display_name for member in group.members}
        for group in report.groups
    )


def test_full_workspace_audit_without_module() -> None:
    report = run_audit(workspace=FIXTURE)
    assert report.declaration_count >= 10
    assert report.groups


def test_private_theorems_included_by_default_and_excluded_by_public_only() -> None:
    default_report = run_audit(workspace=FIXTURE, module_root="Tiny")
    assert any(member.visibility == "private" for group in default_report.groups for member in group.members)
    public_report = run_audit(
        workspace=FIXTURE,
        options=AuditOptions(workspace=FIXTURE, module_root="Tiny", include_private=False),
    )
    assert not any(member.visibility == "private" for group in public_report.groups for member in group.members)


def test_direct_imports_are_optional() -> None:
    default_report = run_audit(workspace=FIXTURE, module_root="Tiny")
    assert not any(member.origin == "direct-import" for group in default_report.groups for member in group.members)
    import_report = run_audit(
        workspace=FIXTURE,
        options=AuditOptions(workspace=FIXTURE, module_root="Tiny", include_imports=True),
    )
    assert any(member.origin == "direct-import" for group in import_report.groups for member in group.members)


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
    latest = json.loads((tmp_path / "indexes" / "fixture" / "latest.json").read_text(encoding="utf-8"))
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
    assert any(member.origin == "external:fixture" for group in report.groups for member in group.members)
    assert any(
        group.kind is DuplicateKind.EXACT_STATEMENT
        and any(member.display_name == "same_as_tiny" for member in group.members)
        for group in report.groups
    )
    assert not any(
        group.kind in {DuplicateKind.PERMUTED_STATEMENT, DuplicateKind.CONNECTIVE_EQUIVALENT}
        and any(member.display_name == "same_as_tiny" and member.origin != "workspace" for member in group.members)
        for group in report.groups
    )
    assert not any(
        group.kind is DuplicateKind.SUBSUMPTION_CANDIDATE
        and any(member.display_name == "impossible_tiny" for member in group.members)
        and any(member.origin != "workspace" for member in group.members)
        for group in report.groups
    )
    assert not any(
        group.members
        and all(member.origin != "workspace" for member in group.members)
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
