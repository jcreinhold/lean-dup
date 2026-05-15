from __future__ import annotations

from pathlib import Path

from lean_dup.features import is_generated
from lean_dup.models import Declaration, SourcePoint, SourceSpan


def test_generated_declaration_detection() -> None:
    declaration = _declaration(name="Example.T.casesOn", short_name="casesOn")
    assert is_generated(declaration)
    macro = _declaration(
        name="Example._aux_Example___macroRules_term_foo_1",
        short_name="_aux_Example___macroRules_term_foo_1",
    )
    assert is_generated(macro)
    user = _declaration(name="Example.realLemma", short_name="realLemma")
    assert not is_generated(user)


def _declaration(*, name: str, short_name: str) -> Declaration:
    return Declaration(
        workspace=Path("/tmp"),
        module="Example",
        name=name,
        display_name=short_name,
        short_name=short_name,
        kind="theorem",
        visibility="public",
        origin="workspace",
        modifiers=(),
        file=Path("/tmp/Example.lean"),
        span=SourceSpan(start=SourcePoint(line=1, column=1), end=SourcePoint(line=1, column=1)),
        type_text="True",
        normalized_type="True",
        type_fingerprint="a",
        permutation_fingerprint="a",
        connective_fingerprint="a",
        conclusion_fingerprint="a",
        constants=(),
        type_heads=("True",),
        binder_count=0,
        source_fingerprint=None,
    )
