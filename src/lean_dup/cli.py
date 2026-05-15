"""Command line interface for `lean-dup`."""

from __future__ import annotations

import argparse
import json
import subprocess
from pathlib import Path
from typing import Any

from lean_dup.audit import run_audit
from lean_dup.external_index import MATHLIB_DEFAULT_WORKSPACE, build_external_index, build_mathlib_index
from lean_dup.extractor import extractor_path
from lean_dup.models import AuditOptions, AuditReport, ReviewPriority
from lean_dup.ranking import actionable
from lean_dup.workspace import resolve_workspace


def main(argv: list[str] | None = None) -> int:
    """Run the `lean-dup` CLI."""

    parser = argparse.ArgumentParser(prog="lean-dup")
    subparsers = parser.add_subparsers(dest="command", required=True)

    doctor = subparsers.add_parser("doctor", help="check workspace and extractor health")
    doctor.add_argument("--workspace", required=True, type=Path)
    doctor.add_argument("--module", dest="module_root")

    index = subparsers.add_parser("index", help="build or reuse an external comparison index")
    index.add_argument("--workspace", required=True, type=Path)
    index.add_argument("--module", dest="module_root", required=True)
    index.add_argument("--label", required=True)
    index.add_argument("--force", action="store_true")
    index.add_argument("--profile", action="store_true")
    index.add_argument("--progress", action="store_true")

    index_mathlib = subparsers.add_parser("index-mathlib", help="build or reuse the mathlib comparison index")
    index_mathlib.add_argument("--workspace", type=Path, default=MATHLIB_DEFAULT_WORKSPACE)
    index_mathlib.add_argument("--force", action="store_true")
    index_mathlib.add_argument("--profile", action="store_true")
    index_mathlib.add_argument("--progress", action="store_true")

    audit = subparsers.add_parser("audit", help="audit a Lake workspace")
    audit.add_argument("--workspace", required=True, type=Path)
    audit.add_argument("--module", dest="module_root")
    audit.add_argument("--format", choices=("text", "json"), default="text")
    audit.add_argument("--public-only", action="store_true")
    audit.add_argument("--include-private", dest="include_private", action="store_true", default=True)
    audit.add_argument("--no-include-private", dest="include_private", action="store_false")
    audit.add_argument("--include-imports", action="store_true")
    audit.add_argument("--import-root", action="append", default=[])
    audit.add_argument("--compare-index", action="append", default=[])
    audit.add_argument("--compare-mathlib", action="store_true")
    audit.add_argument("--mathlib-workspace", type=Path)
    audit.add_argument("--threshold", type=float, default=0.78)
    audit.add_argument("--profile", action="store_true")
    audit.add_argument("--progress", action="store_true")
    audit.add_argument("--include-generated", action="store_true")
    audit.add_argument("--show-noise", action="store_true")
    audit.add_argument("--min-priority", choices=tuple(ReviewPriority), default=ReviewPriority.LOW)

    show = subparsers.add_parser("show", help="show one group from the latest audit")
    show.add_argument("--workspace", required=True, type=Path)
    show.add_argument("--group", required=True)

    args = parser.parse_args(argv)
    try:
        if args.command == "doctor":
            return _doctor(workspace=args.workspace, module_root=args.module_root)
        if args.command == "index":
            metadata = build_external_index(
                workspace=args.workspace,
                module_root=args.module_root,
                label=args.label,
                force=args.force,
                profile=args.profile,
                progress=args.progress,
            )
            print(_render_index_metadata(metadata))
            return 0
        if args.command == "index-mathlib":
            metadata = build_mathlib_index(
                workspace=args.workspace,
                force=args.force,
                profile=args.profile,
                progress=args.progress,
            )
            print(_render_index_metadata(metadata))
            return 0
        if args.command == "audit":
            include_private = args.include_private and not args.public_only
            audit_options = AuditOptions(
                workspace=args.workspace,
                module_root=args.module_root,
                include_private=include_private,
                include_imports=args.include_imports,
                import_roots=tuple(args.import_root),
                compare_indexes=tuple(args.compare_index),
                compare_mathlib=args.compare_mathlib,
                mathlib_workspace=args.mathlib_workspace,
                threshold=args.threshold,
                profile=args.profile,
                progress=args.progress,
                include_generated=args.include_generated,
                show_noise=args.show_noise,
                min_priority=ReviewPriority(args.min_priority),
            )
            report = run_audit(
                workspace=args.workspace,
                options=audit_options,
            )
            _write_latest_report(report)
            if args.format == "json":
                print(json.dumps(report.to_jsonable(), indent=2, sort_keys=True))
            else:
                print(_render_report(report, options=audit_options))
            return 0
        if args.command == "show":
            print(_render_group(workspace=args.workspace, group_id=args.group))
            return 0
    except RuntimeError as error:
        print(f"error: {error}")
        return 1
    return 2


def _doctor(*, workspace: Path, module_root: str | None) -> int:
    resolved = resolve_workspace(workspace, module_root)
    lean_version = _run(["lake", "env", "lean", "--version"], cwd=resolved.root)
    if lean_version.returncode != 0:
        raise RuntimeError(lean_version.stderr.strip() or "could not run `lake env lean --version`")
    if not extractor_path().exists():
        raise RuntimeError(f"missing extractor: {extractor_path()}")
    print(f"workspace: {resolved.root}")
    print(f"lean: {lean_version.stdout.strip()}")
    print(f"modules: {len(resolved.modules)}")
    print(f"extractor: {extractor_path()}")
    return 0


def _run(command: list[str], *, cwd: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        command,
        cwd=cwd,
        check=False,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )


def _render_report(report: AuditReport, *, options: AuditOptions) -> str:
    shown_groups = [
        group
        for group in report.groups
        if actionable(
            group,
            include_generated=options.include_generated,
            show_noise=options.show_noise,
            min_priority=options.min_priority,
        )
    ]
    lines = [
        f"workspace: {report.workspace}",
        f"module root: {report.module_root or '(inferred)'}",
        f"declarations: {report.declaration_count}",
        f"cache: {'hit' if report.cache_hit else 'miss'}",
        f"groups: {len(shown_groups)} shown / {len(report.groups)} total",
    ]
    for external in report.external_indexes:
        lines.append(
            "external index: "
            f"{external.label} declarations={external.declaration_count} "
            f"cache={'hit' if external.cache_hit else 'miss'}"
        )
    for warning in report.warnings:
        lines.append(f"warning: {warning}")
    for group in shown_groups:
        lines.append("")
        lines.append(
            f"{group.id} [{group.kind}] "
            f"priority={group.review_priority} action={group.recommended_action} "
            f"confidence={group.confidence:.2f}"
        )
        lines.append(f"  {group.reason}")
        for signal in group.signals[:8]:
            lines.append(f"  signal: {signal}")
        for blocker in group.blockers:
            lines.append(f"  blocker: {blocker}")
        for evidence in group.evidence:
            lines.append(f"  evidence: {evidence}")
        for member in group.members:
            visibility = "" if member.visibility == "public" else f" {member.visibility}"
            origin = "" if member.origin == "workspace" else f" {member.origin}"
            lines.append(f"  - {member.display_name} ({member.file}:{member.line}){visibility}{origin}")
    return "\n".join(lines)


def _render_group(*, workspace: Path, group_id: str) -> str:
    report = _read_latest_report(workspace)
    for group in report["groups"]:
        if group["id"] != group_id:
            continue
        lines = [f"{group['id']} [{group['kind']}]", f"reason: {group['reason']}"]
        lines.append(f"priority: {group.get('review_priority', 'medium')}")
        lines.append(f"recommended action: {group.get('recommended_action', 'review')}")
        for signal in group.get("signals", []):
            lines.append(f"signal: {signal}")
        for blocker in group.get("blockers", []):
            lines.append(f"blocker: {blocker}")
        for member in group["members"]:
            lines.append("")
            lines.append(f"{member['name']}")
            lines.append(f"  file: {member['file']}:{member['line']}")
            lines.append(f"  kind: {member['kind']}")
            lines.append(f"  visibility: {member.get('visibility', 'public')}")
            lines.append(f"  origin: {member.get('origin', 'workspace')}")
            lines.append(f"  type: {member['type_text']}")
        return "\n".join(lines)
    raise RuntimeError(f"group not found in latest report: {group_id}")


def _render_index_metadata(metadata: Any) -> str:
    return "\n".join(
        [
            f"label: {metadata.label}",
            f"workspace: {metadata.workspace}",
            f"module root: {metadata.module_root}",
            f"declarations: {metadata.declaration_count}",
            f"cache: {'hit' if metadata.cache_hit else 'miss'}",
            f"path: {metadata.path}",
        ]
    )


def _write_latest_report(report: AuditReport) -> None:
    path = _latest_report_path(report.workspace)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(report.to_jsonable(), indent=2, sort_keys=True), encoding="utf-8")


def _read_latest_report(workspace: Path) -> dict[str, Any]:
    path = _latest_report_path(workspace.expanduser().resolve())
    if not path.exists():
        raise RuntimeError("no latest report; run `lean-dup audit` first")
    return json.loads(path.read_text(encoding="utf-8"))


def _latest_report_path(workspace: Path) -> Path:
    return workspace / ".lean-dup" / "cache" / "latest-report.json"
