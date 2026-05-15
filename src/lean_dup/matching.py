"""Shared deterministic matching features for declaration comparison."""

from __future__ import annotations

from typing import Protocol

from lean_dup.models import Declaration

MAX_BUCKET_SIZE = 400
LOW_SIGNAL_NAME_TOKENS = frozenset(
    {
        "of",
        "eq",
        "ne",
        "not",
        "one",
        "two",
        "left",
        "right",
        "iff",
        "mp",
        "mpr",
        "aux",
        "proof",
    }
)


class FrequencyLookup(Protocol):
    """Minimal protocol for near-key frequency lookups."""

    def __getitem__(self, key: str) -> int: ...


def near_index_keys(declaration: Declaration, *, constants_frequency: FrequencyLookup) -> tuple[str, ...]:
    """Return deterministic near-bucket keys for one declaration."""

    keys: list[str] = []
    for constant in declaration.constants:
        if 1 < constants_frequency[constant] <= MAX_BUCKET_SIZE:
            keys.append(f"const:{constant}")
    keys.extend(f"head:{head}" for head in declaration.type_heads)
    keys.extend(f"name:{token}" for token in name_tokens(declaration.short_name) if token not in LOW_SIGNAL_NAME_TOKENS)
    keys.append(f"conclusion:{declaration.conclusion_fingerprint}")
    return tuple(keys)


def name_tokens(name: str) -> set[str]:
    """Return low-effort declaration-name tokens."""

    return {part for part in name.replace("'", "").split("_") if part}


def jaccard(first: set[str], second: set[str]) -> float:
    """Return Jaccard overlap for two finite feature sets."""

    if not first and not second:
        return 0.0
    return len(first & second) / len(first | second)
