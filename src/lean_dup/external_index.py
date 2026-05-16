"""Persistent external declaration indexes."""

from __future__ import annotations

import json
import os
import sqlite3
import subprocess
import sys
import time
from collections import Counter
from dataclasses import dataclass
from hashlib import sha256
from pathlib import Path
from typing import Any, Iterable

from lean_dup.extractor import EXTRACTOR_VERSION, extractor_path, run_extractor
from lean_dup.matching import MAX_BUCKET_SIZE, near_index_keys
from lean_dup.models import Declaration, ExternalIndexMetadata, SourcePoint, SourceSpan
from lean_dup.text import stable_hash
from lean_dup.workspace import ModuleEntry, Workspace, module_to_file, resolve_workspace

MATHLIB_DEFAULT_WORKSPACE = Path("/Users/jcreinhold/Code/mathlib4")
INDEX_SCHEMA_VERSION = "external-index.sqlite.v1"
FINGERPRINT_COLUMNS = {
    "type_fingerprint",
    "permutation_fingerprint",
    "connective_fingerprint",
    "conclusion_fingerprint",
}


@dataclass(frozen=True)
class ExternalIndex:
    """One queryable external declaration index."""

    metadata: ExternalIndexMetadata
    index_dir: Path
    index_path: Path
    warnings: tuple[str, ...] = ()

    def fingerprint_matches(
        self, *, key_name: str, keys: Iterable[str]
    ) -> dict[str, tuple[Declaration, ...]]:
        """Return external declarations bucketed by requested fingerprint key."""

        if key_name not in FINGERPRINT_COLUMNS:
            raise RuntimeError(f"unknown external fingerprint key: {key_name}")
        key_values = tuple(sorted(set(key for key in keys if key)))
        if not key_values:
            return {}
        result: dict[str, tuple[Declaration, ...]] = {}
        with sqlite3.connect(self.index_path) as connection:
            for chunk in _chunks(key_values, 250):
                placeholders = ",".join("?" for _ in chunk)
                rows = connection.execute(
                    "SELECT key, COUNT(*) FROM fingerprint_bucket "
                    f"WHERE kind = ? AND key IN ({placeholders}) GROUP BY key",
                    (key_name, *chunk),
                ).fetchall()
                allowed = [str(row[0]) for row in rows if int(row[1]) <= MAX_BUCKET_SIZE]
                if not allowed:
                    continue
                allowed_placeholders = ",".join("?" for _ in allowed)
                matches = connection.execute(
                    "SELECT b.key, d.* FROM fingerprint_bucket b "
                    "JOIN declarations d ON d.id = b.decl_id "
                    f"WHERE b.kind = ? AND b.key IN ({allowed_placeholders}) "
                    "ORDER BY b.key, d.name",
                    (key_name, *allowed),
                ).fetchall()
                for row in matches:
                    key = str(row[0])
                    result.setdefault(key, tuple())
                    result[key] = (*result[key], _declaration_from_sqlite_row(row[1:]))
        return result

    def near_matches(
        self, *, keys: Iterable[str]
    ) -> tuple[dict[str, tuple[Declaration, ...]], tuple[str, ...]]:
        """Return external declarations bucketed by requested near keys."""

        key_values = tuple(sorted(set(key for key in keys if key)))
        if not key_values:
            return {}, ()
        result: dict[str, tuple[Declaration, ...]] = {}
        warnings: list[str] = []
        seen_warnings: set[str] = set()
        with sqlite3.connect(self.index_path) as connection:
            for chunk in _chunks(key_values, 250):
                placeholders = ",".join("?" for _ in chunk)
                rows = connection.execute(
                    f"SELECT key, COUNT(*) FROM near_bucket WHERE key IN ({placeholders}) GROUP BY key",
                    chunk,
                ).fetchall()
                allowed = []
                for key, count in rows:
                    if int(count) > MAX_BUCKET_SIZE:
                        warning = f"pruned external near bucket {key}: {count} declarations exceeds {MAX_BUCKET_SIZE}"
                        if warning not in seen_warnings:
                            seen_warnings.add(warning)
                            warnings.append(warning)
                    else:
                        allowed.append(str(key))
                if not allowed:
                    continue
                allowed_placeholders = ",".join("?" for _ in allowed)
                refs = connection.execute(
                    "SELECT key, decl_id FROM near_bucket "
                    f"WHERE key IN ({allowed_placeholders}) ORDER BY key, decl_id",
                    allowed,
                ).fetchall()
                declarations = _declarations_by_id(
                    connection=connection,
                    decl_ids=tuple(sorted({int(row[1]) for row in refs})),
                )
                for key_value, decl_id in refs:
                    key = str(key_value)
                    declaration = declarations[int(decl_id)]
                    result.setdefault(key, tuple())
                    result[key] = (*result[key], declaration)
        return result, tuple(warnings)


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
    show_progress = progress
    if show_progress:
        _log(f"lean-dup: resolving workspace {workspace}")
    project = resolve_workspace(workspace, module_root)
    if show_progress:
        _log(f"lean-dup: discovered {len(project.workspace_modules)} module(s) under {module_root}")
    if require_oleans:
        _require_oleans(project, progress=show_progress)
    cache_key = _external_cache_key(
        project=project, label=label, module_root=module_root, progress=show_progress
    )
    cache_id = sha256(json.dumps(cache_key, sort_keys=True).encode("utf-8")).hexdigest()
    index_dir = _label_dir(label) / cache_id
    index_path = index_dir / "index.sqlite"
    if (
        index_path.exists()
        and not force
        and _sqlite_cache_is_current(index_path=index_path, cache_key=cache_key)
    ):
        metadata = _metadata_from_sqlite(index_path=index_path, cache_hit=True)
        _write_label_pointer(label=label, index_dir=index_dir)
        if profile:
            _log(f"profile.external_index_load={time.perf_counter() - started:.3f}s")
        return metadata

    index_dir.mkdir(parents=True, exist_ok=True)
    extracted = run_extractor(_with_origin(project, label), build=build, progress=show_progress)
    try:
        declaration_count = _build_sqlite_index(
            extracted=extracted,
            index_path=index_path,
            cache_key=cache_key,
            label=label,
            workspace=project.root,
            module_root=module_root,
            progress=show_progress,
        )
    finally:
        extracted.unlink(missing_ok=True)
    metadata = ExternalIndexMetadata(
        label=label,
        path=index_path,
        workspace=project.root,
        module_root=module_root,
        declaration_count=declaration_count,
        cache_hit=False,
    )
    _write_label_pointer(label=label, index_dir=index_dir)
    if profile:
        _log(f"profile.external_index_build={time.perf_counter() - started:.3f}s")
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
) -> tuple[tuple[ExternalIndex, ...], tuple[ExternalIndexMetadata, ...], tuple[str, ...]]:
    """Load external comparison indexes and their metadata."""

    all_references = list(references)
    if compare_mathlib:
        metadata = build_mathlib_index(workspace=mathlib_workspace, profile=profile)
        all_references.append(str(metadata.path.parent))

    indexes: list[ExternalIndex] = []
    metadata_entries: list[ExternalIndexMetadata] = []
    warnings: list[str] = []
    for reference in all_references:
        index = load_external_index(reference, profile=profile)
        indexes.append(index)
        metadata_entries.append(index.metadata)
        warnings.extend(index.warnings)
    return tuple(indexes), tuple(metadata_entries), tuple(warnings)


def load_external_index(reference: str, *, profile: bool = False) -> ExternalIndex:
    """Load one external index by label, index directory, or SQLite path."""

    started = time.perf_counter()
    index_dir, index_path = _resolve_index_reference(reference)
    metadata = _metadata_from_sqlite(index_path=index_path, cache_hit=True)
    if profile:
        _log(f"profile.external_index_open.{metadata.label}={time.perf_counter() - started:.3f}s")
    return ExternalIndex(metadata=metadata, index_dir=index_dir, index_path=index_path)


def _build_sqlite_index(
    *,
    extracted: Path,
    index_path: Path,
    cache_key: dict[str, Any],
    label: str,
    workspace: Path,
    module_root: str,
    progress: bool,
) -> int:
    temp_path = index_path.with_suffix(".tmp.sqlite")
    temp_path.unlink(missing_ok=True)
    declaration_count = 0
    constants_frequency: Counter[str] = Counter()
    with sqlite3.connect(temp_path) as connection:
        _initialize_schema(connection)
        with extracted.open("rt", encoding="utf-8") as handle:
            for line in handle:
                if not line.strip():
                    continue
                declaration_count += 1
                if progress and declaration_count % 5000 == 0:
                    _log(f"lean-dup: indexed {declaration_count} declaration row(s)")
                row = json.loads(line)
                declaration = _external_declaration_from_row(row, label)
                constants_frequency.update(set(declaration.constants))
                connection.execute(
                    """
                    INSERT INTO declarations VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                    """,
                    _sqlite_declaration_values(declaration_count, declaration),
                )
                for kind in FINGERPRINT_COLUMNS:
                    key = getattr(declaration, kind)
                    if key:
                        connection.execute(
                            "INSERT INTO fingerprint_bucket VALUES (?, ?, ?)",
                            (kind, key, declaration_count),
                        )
        for row in connection.execute("SELECT * FROM declarations ORDER BY id"):
            declaration = _declaration_from_sqlite_row(row)
            for key in near_index_keys(declaration, constants_frequency=constants_frequency):
                connection.execute("INSERT INTO near_bucket VALUES (?, ?)", (key, int(row[0])))
        _write_sqlite_metadata(
            connection=connection,
            label=label,
            workspace=workspace,
            module_root=module_root,
            index_path=index_path,
            declaration_count=declaration_count,
            cache_key=cache_key,
        )
        connection.execute("CREATE INDEX fingerprint_bucket_key ON fingerprint_bucket(kind, key)")
        connection.execute("CREATE INDEX near_bucket_key ON near_bucket(key)")
        connection.execute("CREATE INDEX declarations_name ON declarations(name)")
        connection.commit()
    temp_path.replace(index_path)
    if progress:
        _log(f"lean-dup: indexed {declaration_count} declaration row(s)")
    return declaration_count


def _initialize_schema(connection: sqlite3.Connection) -> None:
    connection.executescript(
        """
        PRAGMA journal_mode = OFF;
        PRAGMA synchronous = OFF;
        CREATE TABLE metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);
        CREATE TABLE declarations (
          id INTEGER PRIMARY KEY,
          workspace TEXT NOT NULL,
          name TEXT NOT NULL,
          display_name TEXT NOT NULL,
          short_name TEXT NOT NULL,
          module TEXT NOT NULL,
          origin TEXT NOT NULL,
          file TEXT NOT NULL,
          line INTEGER NOT NULL,
          column INTEGER NOT NULL,
          kind TEXT NOT NULL,
          visibility TEXT NOT NULL,
          type_text TEXT NOT NULL,
          normalized_type TEXT NOT NULL,
          type_fingerprint TEXT NOT NULL,
          permutation_fingerprint TEXT NOT NULL,
          connective_fingerprint TEXT NOT NULL,
          conclusion_fingerprint TEXT NOT NULL,
          constants_json TEXT NOT NULL,
          heads_json TEXT NOT NULL,
          binder_count INTEGER NOT NULL
        );
        CREATE TABLE fingerprint_bucket (kind TEXT NOT NULL, key TEXT NOT NULL, decl_id INTEGER NOT NULL);
        CREATE TABLE near_bucket (key TEXT NOT NULL, decl_id INTEGER NOT NULL);
        """
    )


def _write_sqlite_metadata(
    *,
    connection: sqlite3.Connection,
    label: str,
    workspace: Path,
    module_root: str,
    index_path: Path,
    declaration_count: int,
    cache_key: dict[str, Any],
) -> None:
    values = {
        "schema_version": INDEX_SCHEMA_VERSION,
        "extractor_schema": EXTRACTOR_VERSION,
        "label": label,
        "path": str(index_path),
        "workspace": str(workspace),
        "module_root": module_root,
        "declaration_count": str(declaration_count),
        "cache_key": json.dumps(cache_key, sort_keys=True),
    }
    connection.executemany("INSERT INTO metadata VALUES (?, ?)", values.items())


def _sqlite_cache_is_current(*, index_path: Path, cache_key: dict[str, Any]) -> bool:
    try:
        with sqlite3.connect(index_path) as connection:
            metadata = dict(connection.execute("SELECT key, value FROM metadata").fetchall())
    except sqlite3.Error:
        return False
    return (
        metadata.get("schema_version") == INDEX_SCHEMA_VERSION
        and metadata.get("extractor_schema") == EXTRACTOR_VERSION
        and metadata.get("cache_key") == json.dumps(cache_key, sort_keys=True)
    )


def _metadata_from_sqlite(*, index_path: Path, cache_hit: bool) -> ExternalIndexMetadata:
    try:
        with sqlite3.connect(index_path) as connection:
            metadata = dict(connection.execute("SELECT key, value FROM metadata").fetchall())
    except sqlite3.Error as error:
        raise RuntimeError(f"could not read external index metadata: {index_path}") from error
    if metadata.get("schema_version") != INDEX_SCHEMA_VERSION:
        raise RuntimeError(f"unsupported external index schema in: {index_path}")
    return ExternalIndexMetadata(
        label=str(metadata["label"]),
        path=index_path,
        workspace=Path(str(metadata["workspace"])).expanduser().resolve(),
        module_root=str(metadata["module_root"]),
        declaration_count=int(metadata["declaration_count"]),
        cache_hit=cache_hit,
    )


def _sqlite_declaration_values(decl_id: int, declaration: Declaration) -> tuple[Any, ...]:
    return (
        decl_id,
        str(declaration.workspace),
        declaration.name,
        declaration.display_name,
        declaration.short_name,
        declaration.module,
        declaration.origin,
        str(declaration.file),
        declaration.span.start.line,
        declaration.span.start.column,
        declaration.kind,
        declaration.visibility,
        declaration.type_text,
        declaration.normalized_type,
        declaration.type_fingerprint,
        declaration.permutation_fingerprint,
        declaration.connective_fingerprint,
        declaration.conclusion_fingerprint,
        json.dumps(declaration.constants),
        json.dumps(declaration.type_heads),
        declaration.binder_count,
    )


def _declaration_from_sqlite_row(row: sqlite3.Row | tuple[Any, ...]) -> Declaration:
    (
        _decl_id,
        workspace,
        name,
        display_name,
        short_name,
        module,
        origin,
        file,
        line,
        column,
        kind,
        visibility,
        type_text,
        normalized_type,
        type_fingerprint,
        permutation_fingerprint,
        connective_fingerprint,
        conclusion_fingerprint,
        constants_json,
        heads_json,
        binder_count,
    ) = row
    return Declaration(
        workspace=Path(str(workspace)).expanduser().resolve(),
        module=str(module),
        name=str(name),
        display_name=str(display_name),
        short_name=str(short_name),
        kind=str(kind),
        visibility=str(visibility),
        origin=str(origin),
        modifiers=("private",) if visibility == "private" else (),
        file=Path(str(file)).expanduser().resolve(),
        span=SourceSpan(
            start=SourcePoint(line=int(line), column=int(column)),
            end=SourcePoint(line=int(line), column=int(column)),
        ),
        type_text=str(type_text),
        normalized_type=str(normalized_type),
        type_fingerprint=str(type_fingerprint),
        permutation_fingerprint=str(permutation_fingerprint),
        connective_fingerprint=str(connective_fingerprint),
        conclusion_fingerprint=str(conclusion_fingerprint),
        constants=tuple(json.loads(str(constants_json))),
        type_heads=tuple(json.loads(str(heads_json))),
        binder_count=int(binder_count),
        source_fingerprint=None,
    )


def _declarations_by_id(
    *,
    connection: sqlite3.Connection,
    decl_ids: tuple[int, ...],
) -> dict[int, Declaration]:
    if not decl_ids:
        return {}
    result: dict[int, Declaration] = {}
    for chunk in _chunks(tuple(str(decl_id) for decl_id in decl_ids), 500):
        placeholders = ",".join("?" for _ in chunk)
        rows = connection.execute(
            f"SELECT * FROM declarations WHERE id IN ({placeholders})",
            chunk,
        ).fetchall()
        for row in rows:
            result[int(row[0])] = _declaration_from_sqlite_row(row)
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


def _with_origin(project: Workspace, label: str) -> Workspace:
    origin = "mathlib" if label == "mathlib" else f"external:{label}"
    return Workspace(
        root=project.root,
        workspace_modules=project.workspace_modules,
        extraction_modules=tuple(
            ModuleEntry(name=module, origin=origin) for module in project.workspace_modules
        ),
    )


def _external_cache_key(
    *, project: Workspace, label: str, module_root: str, progress: bool = False
) -> dict[str, Any]:
    module_stamps = []
    total = len(project.workspace_modules)
    for index, module in enumerate(project.workspace_modules, start=1):
        if progress and (index == 1 or index == total or index % 250 == 0):
            _log(f"lean-dup: cache-key source scan {index}/{total}: {module}")
        module_stamps.append(
            {
                "name": module,
                "source": _file_stamp(module_to_file(project.root, module)),
            }
        )
    return {
        "schema": EXTRACTOR_VERSION,
        "index_schema": INDEX_SCHEMA_VERSION,
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
            _log(f"lean-dup: checking oleans {index}/{total}: {module}")
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


def _label_dir(label: str) -> Path:
    safe_label = "".join(ch if ch.isalnum() or ch in {"-", "_", "."} else "_" for ch in label)
    return cache_root() / "indexes" / safe_label


def _write_label_pointer(*, label: str, index_dir: Path) -> None:
    pointer = _label_dir(label) / "latest.json"
    pointer.parent.mkdir(parents=True, exist_ok=True)
    pointer.write_text(json.dumps({"index_dir": str(index_dir)}, indent=2), encoding="utf-8")


def _resolve_index_reference(reference: str) -> tuple[Path, Path]:
    candidate = Path(reference).expanduser()
    if candidate.exists():
        resolved = candidate.resolve()
        index_dir = resolved if resolved.is_dir() else resolved.parent
        index_path = resolved if resolved.is_file() else index_dir / "index.sqlite"
        if index_path.name != "index.sqlite" or not index_path.exists():
            raise RuntimeError(f"missing external index SQLite file in: {index_dir}")
        return index_dir, index_path

    pointer = _label_dir(reference) / "latest.json"
    if not pointer.exists():
        raise RuntimeError(f"external index not found by label or path: {reference}")
    payload = json.loads(pointer.read_text(encoding="utf-8"))
    index_dir = Path(str(payload["index_dir"])).expanduser().resolve()
    index_path = index_dir / "index.sqlite"
    if not index_path.exists():
        raise RuntimeError(f"stale external index pointer for {reference}: {index_dir}")
    return index_dir, index_path


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


def _chunks(values: tuple[str, ...], size: int) -> Iterable[tuple[str, ...]]:
    for index in range(0, len(values), size):
        yield values[index : index + size]


def _log(message: str) -> None:
    print(message, file=sys.stderr, flush=True)
