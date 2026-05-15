"""Lean-backed declaration extraction."""

from __future__ import annotations

import json
import re
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass
from hashlib import sha256
from pathlib import Path
from typing import Any

from lean_dup.models import AuditOptions, Declaration, SourcePoint, SourceSpan
from lean_dup.text import normalize_source, stable_hash
from lean_dup.workspace import Workspace, module_to_file

EXTRACTOR_VERSION = "extractor.v2"
PRIVATE_DECL_RE = re.compile(r"^\s*private\s+(?P<kind>theorem|lemma)\s+(?P<name>[A-Za-z_][\w.']*)\b")


@dataclass(frozen=True)
class ExtractedIndex:
    """One loaded declaration index."""

    declarations: tuple[Declaration, ...]
    cache_hit: bool
    timings: dict[str, float]


def load_or_build_index(workspace: Workspace, options: AuditOptions) -> ExtractedIndex:
    """Load a cached declaration index or ask Lean to build it."""

    started = time.perf_counter()
    cache_dir = workspace.root / ".lean-dup" / "cache"
    cache_dir.mkdir(parents=True, exist_ok=True)
    cache_key = _cache_key(workspace, options)
    cache_id = sha256(json.dumps(cache_key, sort_keys=True).encode("utf-8")).hexdigest()
    cache_path = cache_dir / f"{cache_id}.jsonl"
    if cache_path.exists():
        if options.progress:
            _log(f"lean-dup: loading cached workspace index {cache_path}")
        declarations = _augment_with_private_source_declarations(
            workspace,
            _read_index(cache_path, progress=options.progress),
            progress=options.progress,
        )
        return ExtractedIndex(
            declarations=declarations,
            cache_hit=True,
            timings={"load_index": time.perf_counter() - started},
        )
    if options.progress:
        _log("lean-dup: workspace index cache miss")
    raw_path = run_extractor(workspace, build=True, progress=options.progress)
    declarations = _augment_with_private_source_declarations(
        workspace,
        _read_index(raw_path, progress=options.progress),
        progress=options.progress,
    )
    cache_path.write_text(raw_path.read_text(encoding="utf-8"), encoding="utf-8")
    raw_path.unlink(missing_ok=True)
    return ExtractedIndex(
        declarations=declarations,
        cache_hit=False,
        timings={"build_index": time.perf_counter() - started},
    )


def extractor_path() -> Path:
    """Return the bundled Lean extractor source path."""

    return Path(__file__).resolve().parent / "lean_runtime" / "Extractor.lean"


def run_extractor(workspace: Workspace, *, build: bool, progress: bool = False) -> Path:
    """Run the bundled Lean extractor for a resolved project."""

    if build:
        build_targets = ["lake", "build", *workspace.workspace_modules]
        if progress:
            _log(f"lean-dup: building {len(workspace.workspace_modules)} module(s) in {workspace.root}")
        completed_build = subprocess.run(
            build_targets,
            cwd=workspace.root,
            check=False,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        if completed_build.returncode != 0:
            details = "\n".join(
                part for part in (completed_build.stdout.strip(), completed_build.stderr.strip()) if part
            )
            msg = details or "`lake build` failed"
            raise RuntimeError(msg)
    with tempfile.NamedTemporaryFile("w", encoding="utf-8", suffix=".json", delete=False) as manifest:
        json.dump(
            [
                {
                    "name": module.name,
                    "origin": module.origin,
                }
                for module in workspace.extraction_modules
            ],
            manifest,
        )
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
    if progress:
        _log(f"lean-dup: starting Lean extraction for {len(workspace.extraction_modules)} module(s)")
    try:
        if progress:
            completed = subprocess.run(
                command,
                cwd=workspace.root,
                check=False,
                text=True,
                stdout=sys.stderr,
                stderr=sys.stderr,
            )
        else:
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
        stdout = completed.stdout or ""
        stderr = completed.stderr or ""
        details = "\n".join(part for part in (stdout.strip(), stderr.strip()) if part)
        msg = details or "Lean extractor failed"
        raise RuntimeError(msg)
    return output_path


def _read_index(path: Path, *, progress: bool = False) -> tuple[Declaration, ...]:
    declarations: list[Declaration] = []
    for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
        if not line.strip():
            continue
        if progress and line_number % 5000 == 0:
            _log(f"lean-dup: read {line_number} workspace declaration row(s)")
        row = json.loads(line)
        declarations.append(_declaration_from_row(row))
    if progress:
        _log(f"lean-dup: read {len(declarations)} workspace declaration row(s)")
    return tuple(declarations)


def _augment_with_private_source_declarations(
    workspace: Workspace,
    declarations: tuple[Declaration, ...],
    *,
    progress: bool = False,
) -> tuple[Declaration, ...]:
    existing = {(declaration.module, declaration.display_name) for declaration in declarations}
    private_declarations: list[Declaration] = []
    total = len(workspace.workspace_modules)
    for index, module in enumerate(workspace.workspace_modules, start=1):
        if progress and (index == 1 or index == total or index % 250 == 0):
            _log(f"lean-dup: scanning private source declarations {index}/{total}: {module}")
        file_path = module_to_file(workspace.root, module)
        private_declarations.extend(
            declaration
            for declaration in _private_source_declarations(
                workspace_root=workspace.root,
                module=module,
                file_path=file_path,
            )
            if (declaration.module, declaration.display_name) not in existing
        )
    return tuple([*declarations, *private_declarations])


def _declaration_from_row(row: dict[str, Any]) -> Declaration:
    workspace = Path(str(row["workspace"])).expanduser().resolve()
    file_path = Path(str(row["file"])).expanduser().resolve()
    span = _span_from_json(row["span"])
    source_fingerprint = _source_fingerprint(file_path, span, str(row["name"]))
    normalized_type = str(row["normalized_type"])
    permutation_type = str(row["permutation_normalized_type"])
    connective_type = str(row["connective_normalized_type"])
    conclusion = str(row["conclusion_normalized_type"])
    return Declaration(
        workspace=workspace,
        module=str(row["module"]),
        name=str(row["name"]),
        display_name=str(row.get("display_name", row["short_name"])),
        short_name=str(row["short_name"]),
        kind=str(row["kind"]),
        visibility=str(row.get("visibility", "public")),
        origin=str(row.get("origin", "workspace")),
        modifiers=tuple(row.get("modifiers", [])),
        file=file_path,
        span=span,
        type_text=str(row["type_text"]),
        normalized_type=normalized_type,
        type_fingerprint=stable_hash(normalized_type),
        permutation_fingerprint=stable_hash(permutation_type),
        connective_fingerprint=stable_hash(connective_type),
        conclusion_fingerprint=stable_hash(conclusion),
        constants=tuple(row.get("constants", [])),
        type_heads=tuple(row.get("type_heads", [])),
        binder_count=int(row.get("binder_count", 0)),
        source_fingerprint=source_fingerprint,
    )


def _private_source_declarations(*, workspace_root: Path, module: str, file_path: Path) -> list[Declaration]:
    if not file_path.exists():
        return []
    lines = file_path.read_text(encoding="utf-8").splitlines()
    declarations: list[Declaration] = []
    for index, line in enumerate(lines):
        match = PRIVATE_DECL_RE.match(line)
        if match is None:
            continue
        snippet_lines = _declaration_header(lines, index)
        snippet = "\n".join(snippet_lines)
        display_name = match.group("name")
        kind = "theorem" if match.group("kind") in {"theorem", "lemma"} else match.group("kind")
        normalized = _source_statement_fingerprint(snippet, display_name)
        fingerprint = stable_hash(normalized)
        span = SourceSpan(
            start=SourcePoint(line=index + 1, column=0),
            end=SourcePoint(line=index + len(snippet_lines), column=len(snippet_lines[-1])),
        )
        declarations.append(
            Declaration(
                workspace=workspace_root,
                module=module,
                name=f"{module}.{display_name}",
                display_name=display_name,
                short_name=display_name,
                kind=kind,
                visibility="private",
                origin="workspace",
                modifiers=("private",),
                file=file_path,
                span=span,
                type_text=normalize_source(snippet),
                normalized_type=normalized,
                type_fingerprint=fingerprint,
                permutation_fingerprint=fingerprint,
                connective_fingerprint=fingerprint,
                conclusion_fingerprint=stable_hash(_source_conclusion(snippet)),
                constants=tuple(sorted(set(re.findall(r"\b[A-Z][A-Za-z0-9_.']*", snippet)))),
                type_heads=tuple(sorted(set(re.findall(r"[→∧∨=↔]", snippet)))),
                binder_count=snippet.count("(") + snippet.count("{") + snippet.count("["),
                source_fingerprint=stable_hash(normalize_source(snippet).replace(display_name, "_decl", 1)),
            )
        )
    return declarations


def _declaration_header(lines: list[str], start: int) -> list[str]:
    collected: list[str] = []
    for line in lines[start : min(len(lines), start + 12)]:
        collected.append(line)
        if ":=" in line or " where" in line:
            break
    return collected


def _source_statement_fingerprint(snippet: str, display_name: str) -> str:
    text = normalize_source(snippet)
    text = text.replace(display_name, "_decl", 1)
    text = re.sub(r"\b[A-Za-z_][A-Za-z0-9_']*\b(?=\s*:)", "_", text)
    text = re.sub(r"\b[A-Za-z_][A-Za-z0-9_']*\b", _normalize_source_identifier, text)
    return text


def _normalize_source_identifier(match: re.Match[str]) -> str:
    word = match.group(0)
    if word in {"private", "theorem", "lemma", "by", "Prop", "Type", "Sort"}:
        return word
    if word[:1].isupper():
        return word
    return "v"


def _source_conclusion(snippet: str) -> str:
    text = normalize_source(snippet)
    before_body = text.split(":=", 1)[0].split(" where", 1)[0]
    if ":" not in before_body:
        return before_body
    return before_body.rsplit(":", 1)[-1].strip()


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


def _cache_key(workspace: Workspace, options: AuditOptions) -> dict[str, Any]:
    return {
        "schema": EXTRACTOR_VERSION,
        "extractor": _file_hash(extractor_path()),
        "lean_toolchain": _file_text(workspace.root / "lean-toolchain"),
        "lake_manifest": _file_hash(workspace.root / "lake-manifest.json"),
        "include_imports": options.include_imports,
        "include_private": options.include_private,
        "import_roots": list(options.import_roots),
        "modules": [
            {
                "name": module.name,
                "origin": module.origin,
                "source": _file_hash(module_to_file(workspace.root, module.name)),
            }
            for module in workspace.extraction_modules
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


def _log(message: str) -> None:
    print(message, file=sys.stderr, flush=True)
