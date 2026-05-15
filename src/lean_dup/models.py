"""Stable report models for `lean-dup`."""

from __future__ import annotations

from dataclasses import asdict, dataclass
from enum import StrEnum
from pathlib import Path
from typing import Any


class JsonableDataclass:
    """Shared JSON conversion for report dataclasses."""

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


class DuplicateKind(StrEnum):
    """Kinds of duplication reported by v1."""

    EXACT_STATEMENT = "exact-statement"
    PERMUTED_STATEMENT = "permuted-statement"
    CONNECTIVE_EQUIVALENT = "connective-equivalent"
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
    display_name: str
    short_name: str
    kind: str
    visibility: str
    origin: str
    modifiers: tuple[str, ...]
    file: Path
    span: SourceSpan
    type_text: str
    normalized_type: str
    type_fingerprint: str
    permutation_fingerprint: str
    connective_fingerprint: str
    conclusion_fingerprint: str
    constants: tuple[str, ...]
    type_heads: tuple[str, ...]
    binder_count: int
    source_fingerprint: str | None


@dataclass(frozen=True)
class DuplicateMember:
    """One declaration inside a duplicate group."""

    name: str
    display_name: str
    module: str
    file: Path
    line: int
    kind: str
    visibility: str
    origin: str
    type_text: str


@dataclass(frozen=True)
class DuplicateGroup:
    """One reported cluster of related declarations."""

    id: str
    kind: DuplicateKind
    confidence: float
    reason: str
    evidence: tuple[str, ...]
    members: tuple[DuplicateMember, ...]


@dataclass(frozen=True)
class AuditReport(JsonableDataclass):
    """Complete audit result."""

    workspace: Path
    module_root: str | None
    declaration_count: int
    cache_hit: bool
    external_indexes: tuple[ExternalIndexMetadata, ...]
    warnings: tuple[str, ...]
    groups: tuple[DuplicateGroup, ...]


@dataclass(frozen=True)
class ExternalIndexMetadata(JsonableDataclass):
    """Metadata for one external comparison index used by an audit."""

    label: str
    path: Path
    workspace: Path
    module_root: str
    declaration_count: int
    cache_hit: bool


@dataclass(frozen=True)
class AuditOptions(JsonableDataclass):
    """Options for one audit run."""

    workspace: Path
    module_root: str | None = None
    include_private: bool = True
    include_imports: bool = False
    import_roots: tuple[str, ...] = ()
    compare_indexes: tuple[str, ...] = ()
    compare_mathlib: bool = False
    mathlib_workspace: Path | None = None
    threshold: float = 0.78
    profile: bool = False
