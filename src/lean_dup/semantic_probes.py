"""Lean-backed semantic probes for ranked duplicate candidates."""

from __future__ import annotations

import json
import sqlite3
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from hashlib import sha256
from pathlib import Path
from typing import Iterable

from lean_dup.models import Declaration, DuplicateGroup
from lean_dup.probes import ProbeResult, declaration_probe_key
from lean_dup.workspace import Workspace

PROBE_SCHEMA_VERSION = "semantic-probes.v4"
MAX_PROBE_PAIRS = 5000


@dataclass(frozen=True)
class ProbePair:
    """One declaration pair selected for semantic probing."""

    first: Declaration
    second: Declaration

    @property
    def key(self) -> frozenset[str]:
        """Return the pair key used by ranking."""

        return frozenset({declaration_probe_key(self.first), declaration_probe_key(self.second)})


def probe_candidate_groups(
    *,
    workspace: Workspace,
    groups: Iterable[DuplicateGroup],
    declarations_by_name: dict[str, Declaration],
    enabled: bool,
    progress: bool = False,
) -> dict[frozenset[str], ProbeResult]:
    """Return Lean-backed probe results for declaration pairs in candidate groups."""

    pairs = _candidate_pairs(groups=groups, declarations_by_name=declarations_by_name)
    if not pairs:
        return {}
    if not enabled:
        return {}
    if len(pairs) > MAX_PROBE_PAIRS:
        pairs = pairs[:MAX_PROBE_PAIRS]
    cache = _ProbeCache(workspace.root)
    cached: dict[frozenset[str], ProbeResult] = {}
    missing: list[ProbePair] = []
    for pair in pairs:
        result = cache.get(pair)
        if result is None:
            missing.append(pair)
        else:
            cached[pair.key] = result
    if progress:
        _log(f"lean-dup: semantic probes cache hit {len(cached)}/{len(pairs)} pair(s)")
    if missing:
        fresh = _run_lean_probe(workspace=workspace, pairs=missing, progress=progress)
        for pair in missing:
            result = fresh.get(pair.key)
            if result is None:
                result = ProbeResult(
                    unavailable=True,
                    source="lean",
                    message="probe runner did not return this pair",
                )
            cache.put(pair, result)
            cached[pair.key] = result
    return cached


def semantic_probe_path() -> Path:
    """Return the bundled Lean semantic probe runner path."""

    return Path(__file__).resolve().parent / "lean_runtime" / "SemanticProbe.lean"


class _ProbeCache:
    """Small SQLite cache for pair probe results."""

    def __init__(self, workspace_root: Path) -> None:
        self.context = {
            "workspace": str(workspace_root),
            "lean_toolchain": _file_text(workspace_root / "lean-toolchain"),
        }
        self.path = workspace_root / ".lean-dup" / "cache" / "probes.sqlite"
        self.path.parent.mkdir(parents=True, exist_ok=True)
        with sqlite3.connect(self.path) as connection:
            connection.execute(
                "CREATE TABLE IF NOT EXISTS probes "
                "(cache_key TEXT PRIMARY KEY, payload TEXT NOT NULL)"
            )

    def get(self, pair: ProbePair) -> ProbeResult | None:
        key = _cache_key(pair, self.context)
        try:
            with sqlite3.connect(self.path) as connection:
                row = connection.execute(
                    "SELECT payload FROM probes WHERE cache_key = ?",
                    (key,),
                ).fetchone()
        except sqlite3.Error:
            return None
        if row is None:
            return None
        return _probe_result_from_json(json.loads(str(row[0])))

    def put(self, pair: ProbePair, result: ProbeResult) -> None:
        payload = json.dumps(_probe_result_to_json(result), sort_keys=True)
        with sqlite3.connect(self.path) as connection:
            connection.execute(
                "INSERT OR REPLACE INTO probes VALUES (?, ?)",
                (_cache_key(pair, self.context), payload),
            )


def _candidate_pairs(
    *,
    groups: Iterable[DuplicateGroup],
    declarations_by_name: dict[str, Declaration],
) -> list[ProbePair]:
    pairs: dict[frozenset[str], ProbePair] = {}
    for group in groups:
        declarations = [
            declarations_by_name[_member_probe_key(member)]
            for member in group.members
            if _member_probe_key(member) in declarations_by_name
        ]
        for index, first in enumerate(declarations):
            for second in declarations[index + 1 :]:
                if first.name == second.name:
                    if declaration_probe_key(first) == declaration_probe_key(second):
                        continue
                if first.kind not in {"theorem", "axiom", "def", "abbrev"}:
                    continue
                if second.kind not in {"theorem", "axiom", "def", "abbrev"}:
                    continue
                key = frozenset({first.name, second.name})
                pairs.setdefault(key, ProbePair(first=first, second=second))
    return list(pairs.values())


def _run_lean_probe(
    *,
    workspace: Workspace,
    pairs: list[ProbePair],
    progress: bool,
) -> dict[frozenset[str], ProbeResult]:
    if progress:
        _log(f"lean-dup: running Lean semantic probes for {len(pairs)} pair(s)")
    with tempfile.NamedTemporaryFile(
        "w", encoding="utf-8", suffix=".json", delete=False
    ) as manifest:
        json.dump(
            {
                "modules": list(workspace.workspace_modules),
                "pairs": [_pair_to_json(pair) for pair in pairs],
            },
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
        str(semantic_probe_path()),
        str(manifest_path),
        str(output_path),
    ]
    try:
        completed = subprocess.run(
            command,
            cwd=workspace.root,
            check=False,
            text=True,
            stdout=sys.stderr if progress else subprocess.PIPE,
            stderr=sys.stderr if progress else subprocess.PIPE,
        )
    finally:
        manifest_path.unlink(missing_ok=True)
    results: dict[frozenset[str], ProbeResult] = {}
    if completed.returncode != 0:
        message = _completed_message(completed) or "Lean semantic probe failed"
        output_path.unlink(missing_ok=True)
        return {
            pair.key: ProbeResult(unavailable=True, source="lean", message=message)
            for pair in pairs
        }
    try:
        for line in output_path.read_text(encoding="utf-8").splitlines():
            if not line.strip():
                continue
            row = json.loads(line)
            key = frozenset({str(row["first_key"]), str(row["second_key"])})
            results[key] = _probe_result_from_json(row)
    finally:
        output_path.unlink(missing_ok=True)
    return results


def _pair_to_json(pair: ProbePair) -> dict[str, str]:
    return {
        "first_key": declaration_probe_key(pair.first),
        "second_key": declaration_probe_key(pair.second),
        "first": pair.first.name,
        "second": pair.second.name,
        "first_kind": pair.first.kind,
        "second_kind": pair.second.kind,
        "first_type_fingerprint": pair.first.type_fingerprint,
        "second_type_fingerprint": pair.second.type_fingerprint,
        "first_permutation_fingerprint": pair.first.permutation_fingerprint,
        "second_permutation_fingerprint": pair.second.permutation_fingerprint,
        "first_connective_fingerprint": pair.first.connective_fingerprint,
        "second_connective_fingerprint": pair.second.connective_fingerprint,
        "first_conclusion_fingerprint": pair.first.conclusion_fingerprint,
        "second_conclusion_fingerprint": pair.second.conclusion_fingerprint,
        "first_origin": pair.first.origin,
        "second_origin": pair.second.origin,
    }


def _cache_key(pair: ProbePair, context: dict[str, str | None]) -> str:
    payload = {
        "schema": PROBE_SCHEMA_VERSION,
        "context": context,
        "first": _cache_decl(pair.first),
        "second": _cache_decl(pair.second),
    }
    return sha256(json.dumps(payload, sort_keys=True).encode("utf-8")).hexdigest()


def _cache_decl(declaration: Declaration) -> dict[str, str]:
    return {
        "name": declaration.name,
        "kind": declaration.kind,
        "origin": declaration.origin,
        "workspace": str(declaration.workspace),
        "type": declaration.type_fingerprint,
        "permutation": declaration.permutation_fingerprint,
        "connective": declaration.connective_fingerprint,
        "conclusion": declaration.conclusion_fingerprint,
    }


def _member_probe_key(member) -> str:
    return "\0".join((member.origin, member.name, str(member.file), str(member.line)))


def _probe_result_to_json(result: ProbeResult) -> dict[str, object]:
    return {
        "same_statement": result.same_statement,
        "same_up_to_reordering": result.same_up_to_reordering,
        "connective_equivalent": result.connective_equivalent,
        "specializes": result.specializes,
        "specializes_left_to_right": result.specializes_left_to_right,
        "specializes_right_to_left": result.specializes_right_to_left,
        "mutual_implication_shape": result.mutual_implication_shape,
        "same_reducible_def": result.same_reducible_def,
        "unavailable": result.unavailable,
        "source": result.source,
        "message": result.message,
    }


def _probe_result_from_json(row: dict[str, object]) -> ProbeResult:
    return ProbeResult(
        same_statement=bool(row.get("same_statement", False)),
        same_up_to_reordering=bool(row.get("same_up_to_reordering", False)),
        connective_equivalent=bool(row.get("connective_equivalent", False)),
        specializes=bool(row.get("specializes", False)),
        specializes_left_to_right=bool(row.get("specializes_left_to_right", False)),
        specializes_right_to_left=bool(row.get("specializes_right_to_left", False)),
        mutual_implication_shape=bool(row.get("mutual_implication_shape", False)),
        same_reducible_def=bool(row.get("same_reducible_def", False)),
        unavailable=bool(row.get("unavailable", False)),
        source=str(row.get("source", "lean")),
        message=str(row["message"]) if row.get("message") is not None else None,
    )


def _completed_message(completed: subprocess.CompletedProcess[str]) -> str:
    return "\n".join(
        part
        for part in ((completed.stdout or "").strip(), (completed.stderr or "").strip())
        if part
    )


def _file_text(path: Path) -> str | None:
    if not path.exists():
        return None
    return path.read_text(encoding="utf-8").strip()


def _log(message: str) -> None:
    print(message, file=sys.stderr, flush=True)
