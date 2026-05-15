"""Stable report models for `lean-dup`."""

from __future__ import annotations

from dataclasses import asdict, dataclass
from enum import StrEnum
from pathlib import Path
from typing import Any


class DuplicateKind(StrEnum):
    """Kinds of duplication reported by v1."""

    EXACT_STATEMENT = "exact-statement"
    NEAR_STATEMENT = "near-statement"
    SOURCE_CLONE = "source-clone"
    SUBSUMPTION_CANDIDATE = "subsumption-candidate"


@dataclass(frozen=True)
class SourcePoint:
    """One 1-based source position."""

    line: int
    column: int


@dataclass(frozen=True)
class SourceSpan:
    """One source span."""

    start: SourcePoint
    end: SourcePoint


@dataclass(frozen=True)
class Declaration:
    """One Lean declaration row used by the auditor."""

    workspace: Path
    module: str
    name: str
    short_name: str
    kind: str
    file: Path
    span: SourceSpan
    type_text: str
    normalized_type: str
    type_fingerprint: str
    conclusion_fingerprint: str
    constants: tuple[str, ...]
    type_heads: tuple[str, ...]
    binder_count: int
    source_fingerprint: str | None


@dataclass(frozen=True)
class DuplicateMember:
    """One declaration inside a duplicate group."""

    name: str
    module: str
    file: Path
    line: int
    kind: str
    type_text: str


@dataclass(frozen=True)
class DuplicateGroup:
    """One reported cluster of related declarations."""

    id: str
    kind: DuplicateKind
    confidence: float
    reason: str
    members: tuple[DuplicateMember, ...]


@dataclass(frozen=True)
class AuditReport:
    """Complete audit result."""

    workspace: Path
    module_root: str | None
    declaration_count: int
    cache_hit: bool
    groups: tuple[DuplicateGroup, ...]

    def to_jsonable(self) -> dict[str, Any]:
        """Return a JSON-serializable representation."""

        def convert(value: Any) -> Any:
            if isinstance(value, Path):
                return str(value)
            if isinstance(value, StrEnum):
                return str(value)
            if isinstance(value, tuple):
                return [convert(item) for item in value]
            if isinstance(value, list):
                return [convert(item) for item in value]
            if isinstance(value, dict):
                return {key: convert(item) for key, item in value.items()}
            if hasattr(value, "__dataclass_fields__"):
                return convert(asdict(value))
            return value

        return convert(self)
