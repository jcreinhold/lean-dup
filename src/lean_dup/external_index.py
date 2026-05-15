"""Persistent external declaration indexes."""

from __future__ import annotations

import gzip
import json
import os
import pickle
import subprocess
import time
from dataclasses import dataclass
from hashlib import sha256
from pathlib import Path
from typing import Any

from lean_dup.extractor import EXTRACTOR_VERSION, extractor_path, run_extractor
from lean_dup.models import Declaration, ExternalIndexMetadata, SourcePoint, SourceSpan
from lean_dup.text import stable_hash
from lean_dup.workspace import ModuleEntry, Workspace, module_to_file, resolve_workspace

MATHLIB_DEFAULT_WORKSPACE = Path("/Users/jcreinhold/Code/mathlib4")
DERIVED_DECLARATION_CACHE_VERSION = "external-declarations.v1"


@dataclass(frozen=True)
class ExternalIndex:
    """One loaded external declaration index."""

    metadata: ExternalIndexMetadata
    declarations: tuple[Declaration, ...]
    warnings: tuple[str, ...] = ()
    timings: dict[str, float] | None = None


def cache_root() -> Path:
    """Return the external index cache root."""

    configured = os.environ.get("LEAN_DUP_CACHE_DIR")
    if configured:
        return Path(configured).expanduser().resolve()
    return (Path.home() / ".cache" / "lean-dup").resolve()


def build_external_index(
    *,
    workspace: Path,
    module_root: str,
    label: str,
    force: bool = False,
    profile: bool = False,
    progress: bool = False,
    build: bool = True,
    require_oleans: bool = False,
) -> ExternalIndexMetadata:
    """Build or reuse a persistent external declaration index."""

    started = time.perf_counter()
    show_progress = progress or profile
    if show_progress:
        print(f"lean-dup: resolving workspace {workspace}", flush=True)
    project = resolve_workspace(workspace, module_root)
    if show_progress:
        print(f"lean-dup: discovered {len(project.workspace_modules)} module(s) under {module_root}", flush=True)
    if require_oleans:
        _require_oleans(project, progress=show_progress)
    key = _external_cache_key(project=project, label=label, module_root=module_root, progress=show_progress)
    cache_id = sha256(json.dumps(key, sort_keys=True).encode("utf-8")).hexdigest()
    index_dir = _label_dir(label)
    index_dir.mkdir(parents=True, exist_ok=True)
    index_path = index_dir / f"{cache_id}.jsonl.gz"
    metadata_path = index_dir / f"{cache_id}.metadata.json"
    if index_path.exists() and metadata_path.exists() and not force:
        metadata = _read_metadata(metadata_path, cache_hit=True)
        if profile:
            print(f"profile.external_index_load={time.perf_counter() - started:.3f}s")
        return metadata

    extracted = run_extractor(_with_origin(project, label), build=build, progress=show_progress)
    try:
        line_count = _write_gzip_index(source=extracted, target=index_path, progress=show_progress)
    finally:
        extracted.unlink(missing_ok=True)
    metadata = ExternalIndexMetadata(
        label=label,
        path=index_path,
        workspace=project.root,
        module_root=module_root,
        declaration_count=line_count,
        cache_hit=False,
    )
    metadata_path.write_text(
        json.dumps(metadata.to_jsonable(), indent=2, sort_keys=True),
        encoding="utf-8",
    )
    _write_label_pointer(label=label, metadata_path=metadata_path)
    if profile:
        print(f"profile.external_index_build={time.perf_counter() - started:.3f}s")
    return metadata


def build_mathlib_index(
    *,
    workspace: Path | None = None,
    force: bool = False,
    profile: bool = False,
    progress: bool = False,
) -> ExternalIndexMetadata:
    """Build or reuse the configured mathlib declaration index."""

    return build_external_index(
        workspace=workspace or MATHLIB_DEFAULT_WORKSPACE,
        module_root="Mathlib",
        label="mathlib",
        force=force,
        profile=profile,
        progress=progress,
        build=False,
        require_oleans=True,
    )


def load_external_indexes(
    *,
    references: tuple[str, ...],
    compare_mathlib: bool,
    mathlib_workspace: Path | None,
    profile: bool = False,
) -> tuple[tuple[Declaration, ...], tuple[ExternalIndexMetadata, ...], tuple[str, ...]]:
    """Load external comparison declarations and their metadata."""

    all_references = list(references)
    if compare_mathlib:
        metadata = build_mathlib_index(workspace=mathlib_workspace, profile=profile)
        all_references.append(str(metadata.path))

    declarations: list[Declaration] = []
    metadata_entries: list[ExternalIndexMetadata] = []
    warnings: list[str] = []
    for reference in all_references:
        index = load_external_index(reference, profile=profile)
        declarations.extend(index.declarations)
        metadata_entries.append(index.metadata)
        warnings.extend(index.warnings)
    return tuple(declarations), tuple(metadata_entries), tuple(warnings)


def load_external_index(reference: str, *, profile: bool = False) -> ExternalIndex:
    """Load one external index by label or path."""

    started = time.perf_counter()
    index_path, metadata_path = _resolve_index_reference(reference)
    metadata = _read_metadata(metadata_path, cache_hit=True)
    declarations = tuple(_external_declarations(index_path, metadata.label, profile=profile))
    if profile:
        print(
            f"profile.external_declaration_load.{metadata.label}={time.perf_counter() - started:.3f}s",
            flush=True,
        )
    return ExternalIndex(metadata=metadata, declarations=declarations)


def _external_declarations(path: Path, label: str, *, profile: bool = False) -> tuple[Declaration, ...]:
    cache_path = _derived_declaration_cache_path(path=path, label=label)
    cache_key = _derived_declaration_cache_key(path=path, label=label)
    cached = _read_derived_declaration_cache(cache_path=cache_path, cache_key=cache_key)
    if cached is not None:
        if profile:
            print(f"lean-dup: external declaration cache hit: {label}", flush=True)
        return cached
    if profile:
        print(f"lean-dup: building external declaration cache: {label}", flush=True)
    declarations: list[Declaration] = []
    with gzip.open(path, "rt", encoding="utf-8") as handle:
        for line in handle:
            if not line.strip():
                continue
            row = json.loads(line)
            declarations.append(_external_declaration_from_row(row, label))
    result = tuple(declarations)
    _write_derived_declaration_cache(cache_path=cache_path, cache_key=cache_key, declarations=result)
    return result


def _external_declaration_from_row(row: dict[str, Any], label: str) -> Declaration:
    origin = "mathlib" if label == "mathlib" else f"external:{label}"
    normalized_type = str(row["normalized_type"])
    permutation_type = str(row["permutation_normalized_type"])
    connective_type = str(row["connective_normalized_type"])
    conclusion = str(row["conclusion_normalized_type"])
    return Declaration(
        workspace=Path(str(row["workspace"])).expanduser().resolve(),
        module=str(row["module"]),
        name=str(row["name"]),
        display_name=str(row.get("display_name", row["short_name"])),
        short_name=str(row["short_name"]),
        kind=str(row["kind"]),
        visibility=str(row.get("visibility", "public")),
        origin=origin,
        modifiers=tuple(row.get("modifiers", [])),
        file=Path(str(row["file"])).expanduser().resolve(),
        span=_span_from_json(row["span"]),
        type_text=str(row["type_text"]),
        normalized_type=normalized_type,
        type_fingerprint=stable_hash(normalized_type),
        permutation_fingerprint=stable_hash(permutation_type),
        connective_fingerprint=stable_hash(connective_type),
        conclusion_fingerprint=stable_hash(conclusion),
        constants=tuple(row.get("constants", [])),
        type_heads=tuple(row.get("type_heads", [])),
        binder_count=int(row.get("binder_count", 0)),
        source_fingerprint=None,
    )


def _span_from_json(payload: dict[str, Any]) -> SourceSpan:
    start = payload["start"]
    end = payload["end"]
    return SourceSpan(
        start=SourcePoint(line=int(start["line"]), column=int(start["column"])),
        end=SourcePoint(line=int(end["line"]), column=int(end["column"])),
    )


def _derived_declaration_cache_path(*, path: Path, label: str) -> Path:
    safe_label = "".join(ch if ch.isalnum() or ch in {"-", "_", "."} else "_" for ch in label)
    return path.parent / f"{path.name}.{safe_label}.decls.pickle.gz"


def _derived_declaration_cache_key(*, path: Path, label: str) -> dict[str, Any]:
    stat = path.stat()
    return {
        "version": DERIVED_DECLARATION_CACHE_VERSION,
        "label": label,
        "index": str(path),
        "index_mtime_ns": stat.st_mtime_ns,
        "index_size": stat.st_size,
    }


def _read_derived_declaration_cache(
    *,
    cache_path: Path,
    cache_key: dict[str, Any],
) -> tuple[Declaration, ...] | None:
    if not cache_path.exists():
        return None
    try:
        with gzip.open(cache_path, "rb") as handle:
            payload = pickle.load(handle)  # noqa: S301 - local cache generated by this tool.
    except (OSError, EOFError, pickle.PickleError, ValueError, TypeError):
        return None
    if not isinstance(payload, dict) or payload.get("cache_key") != cache_key:
        return None
    declarations = payload.get("declarations")
    if not isinstance(declarations, tuple):
        return None
    return declarations


def _write_derived_declaration_cache(
    *,
    cache_path: Path,
    cache_key: dict[str, Any],
    declarations: tuple[Declaration, ...],
) -> None:
    cache_path.parent.mkdir(parents=True, exist_ok=True)
    with gzip.open(cache_path, "wb") as handle:
        pickle.dump(
            {"cache_key": cache_key, "declarations": declarations},
            handle,
            protocol=pickle.HIGHEST_PROTOCOL,
        )


def _with_origin(project: Workspace, label: str) -> Workspace:
    origin = "mathlib" if label == "mathlib" else f"external:{label}"
    return Workspace(
        root=project.root,
        workspace_modules=project.workspace_modules,
        extraction_modules=tuple(ModuleEntry(name=module, origin=origin) for module in project.workspace_modules),
    )


def _external_cache_key(*, project: Workspace, label: str, module_root: str, progress: bool = False) -> dict[str, Any]:
    module_stamps = []
    total = len(project.workspace_modules)
    for index, module in enumerate(project.workspace_modules, start=1):
        if progress and (index == 1 or index == total or index % 250 == 0):
            print(f"lean-dup: cache-key source scan {index}/{total}: {module}", flush=True)
        module_stamps.append(
            {
                "name": module,
                "source": _file_stamp(module_to_file(project.root, module)),
            }
        )
    return {
        "schema": EXTRACTOR_VERSION,
        "label": label,
        "root": str(project.root),
        "module_root": module_root,
        "extractor": _file_hash(extractor_path()),
        "lean_toolchain": _file_text(project.root / "lean-toolchain"),
        "lake_manifest": _file_hash(project.root / "lake-manifest.json"),
        "git_head": _git_head(project.root),
        "git_dirty": _git_dirty(project.root),
        "modules": module_stamps,
    }


def _require_oleans(project: Workspace, *, progress: bool = False) -> None:
    missing = []
    total = len(project.workspace_modules)
    for index, module in enumerate(project.workspace_modules, start=1):
        if progress and (index == 1 or index == total or index % 250 == 0):
            print(f"lean-dup: checking oleans {index}/{total}: {module}", flush=True)
        if not _olean_exists(project.root, module):
            missing.append(module)
    if missing:
        sample = ", ".join(missing[:5])
        raise RuntimeError(
            "missing compiled oleans for external index "
            f"({len(missing)} missing; sample: {sample}); run `lake exe cache get` or `lake build Mathlib` in "
            f"{project.root}"
        )


def _olean_exists(root: Path, module: str) -> bool:
    relative = Path(*module.split(".")).with_suffix(".olean")
    return any((base / relative).exists() for base in (root / ".lake").glob("build/lib*/lean"))


def _write_gzip_index(*, source: Path, target: Path, progress: bool = False) -> int:
    count = 0
    with source.open("rt", encoding="utf-8") as source_handle:
        with gzip.open(target, "wt", encoding="utf-8") as target_handle:
            for line in source_handle:
                if line.strip():
                    count += 1
                    if progress and count % 5000 == 0:
                        print(f"lean-dup: compressed {count} declaration row(s)", flush=True)
                target_handle.write(line)
    if progress:
        print(f"lean-dup: compressed {count} declaration row(s)", flush=True)
    return count


def _label_dir(label: str) -> Path:
    safe_label = "".join(ch if ch.isalnum() or ch in {"-", "_", "."} else "_" for ch in label)
    return cache_root() / "indexes" / safe_label


def _write_label_pointer(*, label: str, metadata_path: Path) -> None:
    pointer = _label_dir(label) / "latest.json"
    pointer.write_text(json.dumps({"metadata": str(metadata_path)}, indent=2), encoding="utf-8")


def _resolve_index_reference(reference: str) -> tuple[Path, Path]:
    candidate = Path(reference).expanduser()
    if candidate.exists():
        index_path = candidate.resolve()
        metadata_path = index_path.with_suffix("").with_suffix(".metadata.json")
        if not metadata_path.exists():
            raise RuntimeError(f"missing external index metadata: {metadata_path}")
        return index_path, metadata_path

    pointer = _label_dir(reference) / "latest.json"
    if not pointer.exists():
        raise RuntimeError(f"external index not found by label or path: {reference}")
    payload = json.loads(pointer.read_text(encoding="utf-8"))
    metadata_path = Path(str(payload["metadata"])).expanduser().resolve()
    if not metadata_path.exists():
        raise RuntimeError(f"stale external index pointer for {reference}: {metadata_path}")
    metadata = _read_metadata(metadata_path, cache_hit=True)
    if not metadata.path.exists():
        raise RuntimeError(f"missing external index file for {reference}: {metadata.path}")
    return metadata.path, metadata_path


def _read_metadata(path: Path, *, cache_hit: bool) -> ExternalIndexMetadata:
    payload = json.loads(path.read_text(encoding="utf-8"))
    return ExternalIndexMetadata(
        label=str(payload["label"]),
        path=Path(str(payload["path"])).expanduser().resolve(),
        workspace=Path(str(payload["workspace"])).expanduser().resolve(),
        module_root=str(payload["module_root"]),
        declaration_count=int(payload["declaration_count"]),
        cache_hit=cache_hit,
    )


def _file_hash(path: Path) -> str | None:
    if not path.exists():
        return None
    return sha256(path.read_bytes()).hexdigest()


def _file_text(path: Path) -> str | None:
    if not path.exists():
        return None
    return path.read_text(encoding="utf-8").strip()


def _file_stamp(path: Path) -> dict[str, int | str | None]:
    if not path.exists():
        return {"mtime_ns": None, "size": None}
    stat = path.stat()
    return {"mtime_ns": stat.st_mtime_ns, "size": stat.st_size}


def _git_head(root: Path) -> str | None:
    return _git(root, "rev-parse", "HEAD")


def _git_dirty(root: Path) -> bool | None:
    status = _git(root, "status", "--porcelain")
    if status is None:
        return None
    return bool(status.strip())


def _git(root: Path, *args: str) -> str | None:
    completed = subprocess.run(
        ["git", *args],
        cwd=root,
        check=False,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
    )
    if completed.returncode != 0:
        return None
    return completed.stdout.strip()
