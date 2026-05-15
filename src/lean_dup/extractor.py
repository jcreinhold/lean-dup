"""Lean-backed declaration extraction."""

from __future__ import annotations

import json
import subprocess
import tempfile
from dataclasses import dataclass
from hashlib import sha256
from pathlib import Path
from typing import Any

from lean_dup.models import Declaration, SourcePoint, SourceSpan
from lean_dup.text import normalize_source, stable_hash
from lean_dup.workspace import Workspace, module_to_file

EXTRACTOR_VERSION = "extractor.v1"


@dataclass(frozen=True)
class ExtractedIndex:
    """One loaded declaration index."""

    declarations: tuple[Declaration, ...]
    cache_hit: bool


def load_or_build_index(workspace: Workspace) -> ExtractedIndex:
    """Load a cached declaration index or ask Lean to build it."""

    cache_dir = workspace.root / ".lean-dup" / "cache"
    cache_dir.mkdir(parents=True, exist_ok=True)
    cache_key = _cache_key(workspace)
    cache_id = sha256(json.dumps(cache_key, sort_keys=True).encode("utf-8")).hexdigest()
    cache_path = cache_dir / f"{cache_id}.jsonl"
    if cache_path.exists():
        return ExtractedIndex(declarations=_read_index(cache_path), cache_hit=True)
    raw_path = _run_extractor(workspace)
    declarations = _read_index(raw_path)
    cache_path.write_text(raw_path.read_text(encoding="utf-8"), encoding="utf-8")
    return ExtractedIndex(declarations=declarations, cache_hit=False)


def extractor_path() -> Path:
    """Return the bundled Lean extractor source path."""

    return Path(__file__).resolve().parents[2] / "lean-runtime" / "Extractor.lean"


def _run_extractor(workspace: Workspace) -> Path:
    build_targets = ["lake", "build", *workspace.modules]
    build = subprocess.run(
        build_targets,
        cwd=workspace.root,
        check=False,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if build.returncode != 0:
        details = "\n".join(part for part in (build.stdout.strip(), build.stderr.strip()) if part)
        msg = details or "`lake build` failed"
        raise RuntimeError(msg)
    with tempfile.NamedTemporaryFile("w", encoding="utf-8", suffix=".json", delete=False) as manifest:
        json.dump(list(workspace.modules), manifest)
        manifest_path = Path(manifest.name)
    output = tempfile.NamedTemporaryFile("w", encoding="utf-8", suffix=".jsonl", delete=False)
    output_path = Path(output.name)
    output.close()
    command = [
        "lake",
        "env",
        "lean",
        "--run",
        str(extractor_path()),
        str(workspace.root),
        str(manifest_path),
        str(output_path),
    ]
    try:
        completed = subprocess.run(
            command,
            cwd=workspace.root,
            check=False,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
    finally:
        manifest_path.unlink(missing_ok=True)
    if completed.returncode != 0:
        output_path.unlink(missing_ok=True)
        details = "\n".join(part for part in (completed.stdout.strip(), completed.stderr.strip()) if part)
        msg = details or "Lean extractor failed"
        raise RuntimeError(msg)
    return output_path


def _read_index(path: Path) -> tuple[Declaration, ...]:
    declarations: list[Declaration] = []
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        row = json.loads(line)
        declarations.append(_declaration_from_row(row))
    return tuple(declarations)


def _declaration_from_row(row: dict[str, Any]) -> Declaration:
    workspace = Path(str(row["workspace"])).expanduser().resolve()
    file_path = Path(str(row["file"])).expanduser().resolve()
    span = _span_from_json(row["span"])
    source_fingerprint = _source_fingerprint(file_path, span, str(row["name"]))
    normalized_type = str(row["normalized_type"])
    conclusion = str(row["conclusion_normalized_type"])
    return Declaration(
        workspace=workspace,
        module=str(row["module"]),
        name=str(row["name"]),
        short_name=str(row["short_name"]),
        kind=str(row["kind"]),
        file=file_path,
        span=span,
        type_text=str(row["type_text"]),
        normalized_type=normalized_type,
        type_fingerprint=stable_hash(normalized_type),
        conclusion_fingerprint=stable_hash(conclusion),
        constants=tuple(row.get("constants", [])),
        type_heads=tuple(row.get("type_heads", [])),
        binder_count=int(row.get("binder_count", 0)),
        source_fingerprint=source_fingerprint,
    )


def _span_from_json(value: dict[str, Any]) -> SourceSpan:
    start = value["start"]
    end = value["end"]
    return SourceSpan(
        start=SourcePoint(line=int(start["line"]), column=int(start["column"])),
        end=SourcePoint(line=int(end["line"]), column=int(end["column"])),
    )


def _source_fingerprint(file_path: Path, span: SourceSpan, declaration_name: str) -> str | None:
    try:
        lines = file_path.read_text(encoding="utf-8").splitlines()
    except OSError:
        return None
    if span.start.line < 1 or span.end.line < span.start.line:
        return None
    snippet = "\n".join(lines[span.start.line - 1 : span.end.line])
    short_name = declaration_name.rsplit(".", 1)[-1]
    skeleton = normalize_source(snippet).replace(short_name, "_decl", 1)
    if not skeleton:
        return None
    return stable_hash(skeleton)


def _cache_key(workspace: Workspace) -> dict[str, Any]:
    return {
        "schema": EXTRACTOR_VERSION,
        "extractor": _file_hash(extractor_path()),
        "lean_toolchain": _file_text(workspace.root / "lean-toolchain"),
        "lake_manifest": _file_hash(workspace.root / "lake-manifest.json"),
        "modules": [
            {
                "name": module,
                "source": _file_hash(module_to_file(workspace.root, module)),
            }
            for module in workspace.modules
        ],
    }


def _file_hash(path: Path) -> str | None:
    if not path.exists():
        return None
    return sha256(path.read_bytes()).hexdigest()


def _file_text(path: Path) -> str | None:
    if not path.exists():
        return None
    return path.read_text(encoding="utf-8").strip()
