"""Command line interface for `lean-dup`."""

from __future__ import annotations

import argparse
import json
import subprocess
from pathlib import Path
from typing import Any

from lean_dup.audit import run_audit
from lean_dup.extractor import extractor_path
from lean_dup.models import AuditOptions, AuditReport
from lean_dup.workspace import resolve_workspace


def main(argv: list[str] | None = None) -> int:
    """Run the `lean-dup` CLI."""

    parser = argparse.ArgumentParser(prog="lean-dup")
    subparsers = parser.add_subparsers(dest="command", required=True)

    doctor = subparsers.add_parser("doctor", help="check workspace and extractor health")
    doctor.add_argument("--workspace", required=True, type=Path)
    doctor.add_argument("--module", dest="module_root")

    audit = subparsers.add_parser("audit", help="audit a Lake workspace")
    audit.add_argument("--workspace", required=True, type=Path)
    audit.add_argument("--module", dest="module_root")
    audit.add_argument("--format", choices=("text", "json"), default="text")
    audit.add_argument("--public-only", action="store_true")
    audit.add_argument("--include-private", dest="include_private", action="store_true", default=True)
    audit.add_argument("--no-include-private", dest="include_private", action="store_false")
    audit.add_argument("--include-imports", action="store_true")
    audit.add_argument("--import-root", action="append", default=[])
    audit.add_argument("--threshold", type=float, default=0.78)
    audit.add_argument("--profile", action="store_true")

    show = subparsers.add_parser("show", help="show one group from the latest audit")
    show.add_argument("--workspace", required=True, type=Path)
    show.add_argument("--group", required=True)

    args = parser.parse_args(argv)
    try:
        if args.command == "doctor":
            return _doctor(workspace=args.workspace, module_root=args.module_root)
        if args.command == "audit":
            include_private = args.include_private and not args.public_only
            report = run_audit(
                workspace=args.workspace,
                options=AuditOptions(
                    workspace=args.workspace,
                    module_root=args.module_root,
                    include_private=include_private,
                    include_imports=args.include_imports,
                    import_roots=tuple(args.import_root),
                    threshold=args.threshold,
                    profile=args.profile,
                ),
            )
            _write_latest_report(report)
            if args.format == "json":
                print(json.dumps(report.to_jsonable(), indent=2, sort_keys=True))
            else:
                print(_render_report(report))
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


def _render_report(report: AuditReport) -> str:
    lines = [
        f"workspace: {report.workspace}",
        f"module root: {report.module_root or '(inferred)'}",
        f"declarations: {report.declaration_count}",
        f"cache: {'hit' if report.cache_hit else 'miss'}",
        f"groups: {len(report.groups)}",
    ]
    for warning in report.warnings:
        lines.append(f"warning: {warning}")
    for group in report.groups:
        lines.append("")
        lines.append(f"{group.id} [{group.kind}] confidence={group.confidence:.2f}")
        lines.append(f"  {group.reason}")
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
