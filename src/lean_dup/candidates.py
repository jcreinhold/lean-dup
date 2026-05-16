"""Bounded candidate generation for near-duplicate declarations."""

from __future__ import annotations

import heapq
from collections import Counter, defaultdict
from itertools import combinations

from lean_dup.external_index import ExternalIndex
from lean_dup.features import pair_signal_score
from lean_dup.matching import MAX_BUCKET_SIZE, near_index_keys
from lean_dup.models import Declaration

MAX_NEAR_CANDIDATES_PER_DECLARATION = 80


def local_near_candidates(
    declarations: tuple[Declaration, ...],
    *,
    warnings: list[str],
    progress: bool,
) -> set[tuple[Declaration, Declaration]]:
    """Return high-value local near candidates without global pair caps."""

    constants_by_decl = {
        declaration.name: set(declaration.constants) for declaration in declarations
    }
    frequency = Counter(
        constant for constants in constants_by_decl.values() for constant in constants
    )
    index: dict[str, list[Declaration]] = defaultdict(list)
    for declaration in declarations:
        for key in near_index_keys(declaration, constants_frequency=frequency):
            index[key].append(declaration)

    heaps: dict[str, list[tuple[float, str, int, Declaration, Declaration]]] = defaultdict(list)
    for key_index, (key, members) in enumerate(index.items(), start=1):
        if progress and (key_index == 1 or key_index % 500 == 0):
            _progress(f"lean-dup: local near bucket {key_index}/{len(index)}")
        unique = tuple(dict.fromkeys(members))
        if not any(_is_workspace(declaration) for declaration in unique):
            continue
        if len(unique) > MAX_BUCKET_SIZE:
            warnings.append(
                f"pruned near bucket {key}: {len(unique)} declarations exceeds {MAX_BUCKET_SIZE}"
            )
            continue
        for first, second in combinations(sorted(unique, key=lambda item: item.name), 2):
            if not (_is_workspace(first) or _is_workspace(second)):
                continue
            _add_candidate(heaps, first, second)
    return _heap_pairs(heaps)


def external_near_candidates(
    workspace_declarations: tuple[Declaration, ...],
    external_indexes: tuple[ExternalIndex, ...],
    *,
    warnings: list[str],
    progress: bool,
) -> set[tuple[Declaration, Declaration]]:
    """Return top external candidates per workspace declaration."""

    if not workspace_declarations or not external_indexes:
        return set()
    constants_frequency = Counter(
        constant for declaration in workspace_declarations for constant in declaration.constants
    )
    workspace_keys_by_decl = {
        declaration.name: near_index_keys(declaration, constants_frequency=constants_frequency)
        for declaration in workspace_declarations
    }
    workspace_keys = {key for keys in workspace_keys_by_decl.values() for key in keys}
    if progress:
        _progress(f"lean-dup: querying {len(workspace_keys)} external near bucket key(s)")

    external_buckets: dict[str, tuple[Declaration, ...]] = {}
    for index in external_indexes:
        buckets, bucket_warnings = index.near_matches(keys=workspace_keys)
        warnings.extend(bucket_warnings)
        if progress:
            matched = sum(len(members) for members in buckets.values())
            _progress(f"lean-dup: external near buckets matched {matched} declaration reference(s)")
        for key, declarations in buckets.items():
            external_buckets[key] = (*external_buckets.get(key, ()), *declarations)

    heaps: dict[str, list[tuple[float, str, int, Declaration, Declaration]]] = defaultdict(list)
    for declaration_index, declaration in enumerate(workspace_declarations, start=1):
        if progress and (declaration_index == 1 or declaration_index % 500 == 0):
            _progress(
                "lean-dup: external near candidates "
                f"{declaration_index}/{len(workspace_declarations)} workspace statement(s)"
            )
        seen_external: set[str] = set()
        for key in workspace_keys_by_decl[declaration.name]:
            members = tuple(dict.fromkeys(external_buckets.get(key, ())))
            if len(members) > MAX_BUCKET_SIZE:
                warnings.append(
                    f"pruned external near bucket {key}: {len(members)} declarations exceeds {MAX_BUCKET_SIZE}"
                )
                continue
            for external in members:
                if external.name in seen_external:
                    continue
                seen_external.add(external.name)
                _add_candidate(heaps, declaration, external)
    return _heap_pairs(heaps)


def _add_candidate(
    heaps: dict[str, list[tuple[float, str, int, Declaration, Declaration]]],
    first: Declaration,
    second: Declaration,
) -> None:
    score, _evidence = pair_signal_score(first, second)
    if score <= 0:
        return
    entry = (score, f"{first.name}\0{second.name}", id(first) ^ id(second), first, second)
    for name in (first.name, second.name):
        heap = heaps[name]
        if len(heap) < MAX_NEAR_CANDIDATES_PER_DECLARATION:
            heapq.heappush(heap, entry)
        elif score > heap[0][0]:
            heapq.heapreplace(heap, entry)


def _heap_pairs(
    heaps: dict[str, list[tuple[float, str, int, Declaration, Declaration]]],
) -> set[tuple[Declaration, Declaration]]:
    pairs: dict[frozenset[str], tuple[Declaration, Declaration]] = {}
    for heap in heaps.values():
        for _score, _key, _tie, first, second in heap:
            pairs[frozenset({first.name, second.name})] = (first, second)
    return set(pairs.values())


def _is_workspace(declaration: Declaration) -> bool:
    return declaration.origin == "workspace"


def _progress(message: str) -> None:
    import sys

    print(message, file=sys.stderr, flush=True)
