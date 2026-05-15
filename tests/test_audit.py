from __future__ import annotations

from pathlib import Path

from lean_dup.audit import run_audit
from lean_dup.models import DuplicateKind


FIXTURE = Path(__file__).parent / "fixtures" / "tiny"


def test_tiny_workspace_reports_expected_groups() -> None:
    report = run_audit(workspace=FIXTURE, module_root="Tiny")
    kinds = {group.kind for group in report.groups}
    assert DuplicateKind.EXACT_STATEMENT in kinds
    assert DuplicateKind.SOURCE_CLONE in kinds
    assert report.declaration_count >= 4


def test_cache_is_reused_on_second_run() -> None:
    run_audit(workspace=FIXTURE, module_root="Tiny")
    report = run_audit(workspace=FIXTURE, module_root="Tiny")
    assert report.cache_hit
