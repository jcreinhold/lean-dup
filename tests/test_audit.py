from __future__ import annotations

from pathlib import Path

from lean_dup.audit import run_audit
from lean_dup.models import AuditOptions, DuplicateKind


FIXTURE = Path(__file__).parent / "fixtures" / "tiny"


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
