"""Read-only import and replacement guidance for duplicate groups."""

from __future__ import annotations

import re
from dataclasses import replace
from pathlib import Path

from lean_dup.models import (
    Declaration,
    DuplicateGroup,
    DuplicateKind,
    ReplacementHint,
    ReviewPriority,
    SourceReference,
)
from lean_dup.workspace import Workspace, module_to_file

MAX_REFERENCES_SHOWN = 12
TRANSITIONAL_ALIAS_REFERENCE_THRESHOLD = 8


def add_replacement_hints(
    *,
    workspace: Workspace,
    groups: tuple[DuplicateGroup, ...],
    declarations_by_key: dict[str, Declaration],
    enabled: bool,
) -> tuple[DuplicateGroup, ...]:
    """Attach replacement hints to actionable groups."""

    if not enabled:
        return groups
    catalog = _SourceCatalog.build(workspace)
    hinted: list[DuplicateGroup] = []
    for group in groups:
        hint = _hint_for_group(
            group=group,
            catalog=catalog,
            declarations_by_key=declarations_by_key,
        )
        hinted.append(replace(group, replacement_hint=hint) if hint is not None else group)
    return tuple(hinted)


def _hint_for_group(
    *,
    group: DuplicateGroup,
    catalog: "_SourceCatalog",
    declarations_by_key: dict[str, Declaration],
) -> ReplacementHint | None:
    if not _eligible(group):
        return None
    target = _target_member(group)
    if target is None:
        return None
    workspace_declarations = [
        declaration
        for member in group.members
        if member.origin == "workspace"
        if (declaration := declarations_by_key.get(_member_key(member))) is not None
    ]
    if not workspace_declarations:
        return None

    references = catalog.references_to(workspace_declarations)
    import_status, notes = catalog.import_status(
        target_module=target.module,
        declarations=workspace_declarations,
    )
    action = _replacement_action(group, workspace_declarations, len(references))
    blockers = _hint_blockers(group)
    return ReplacementHint(
        action=action,
        target_decl=target.name,
        target_module=target.module,
        import_line=f"import {target.module}",
        import_status=import_status,
        references_shown=tuple(references[:MAX_REFERENCES_SHOWN]),
        reference_count=len(references),
        notes=tuple(notes),
        blockers=tuple(blockers),
    )


def _eligible(group: DuplicateGroup) -> bool:
    if group.kind is DuplicateKind.SOURCE_CLONE:
        return False
    if group.review_priority is not ReviewPriority.HIGH:
        return False
    if any(blocker.startswith("generated-declarations=") for blocker in group.blockers):
        return False
    if not any(member.origin != "workspace" for member in group.members):
        return False
    if group.recommended_action == "already-in-mathlib":
        return True
    return (
        group.kind is DuplicateKind.EXACT_STATEMENT
        and (
            "probe:same-statement" in group.signals
            or "probe:same-reducible-def" in group.signals
        )
    )


def _target_member(group: DuplicateGroup):
    external = [member for member in group.members if member.origin != "workspace"]
    if not external:
        return None
    return sorted(external, key=lambda member: (0 if member.origin == "mathlib" else 1, member.name))[0]


def _replacement_action(
    group: DuplicateGroup,
    declarations: list[Declaration],
    reference_count: int,
) -> str:
    if any(_is_backport(declaration) for declaration in declarations):
        return "delete-local-backport"
    if reference_count >= TRANSITIONAL_ALIAS_REFERENCE_THRESHOLD:
        return "keep-transitional-alias"
    if reference_count:
        return "replace-local-uses"
    if group.recommended_action == "already-in-mathlib":
        return "replace-with-import"
    return "manual-review"


def _hint_blockers(group: DuplicateGroup) -> list[str]:
    blockers: list[str] = []
    if group.kind is not DuplicateKind.EXACT_STATEMENT and "probe:same-statement" not in group.signals:
        blockers.append("non-exact-match")
    if group.probe_summary and "unavailable" in group.probe_summary:
        blockers.append("lean-probe-unavailable")
    return blockers


def _is_backport(declaration: Declaration) -> bool:
    return ".Mathlib4Backports." in declaration.module or declaration.module.endswith(
        "Mathlib4Backports"
    )


def _member_key(member) -> str:
    return "\0".join((member.origin, member.name, str(member.file), str(member.line)))


class _SourceCatalog:
    def __init__(self, files: dict[Path, "_SourceFile"]) -> None:
        self._files = files

    @classmethod
    def build(cls, workspace: Workspace) -> "_SourceCatalog":
        files: dict[Path, _SourceFile] = {}
        for module in workspace.workspace_modules:
            path = module_to_file(workspace.root, module)
            if path.exists():
                files[path] = _SourceFile.read(path)
        return cls(files)

    def import_status(
        self,
        *,
        target_module: str,
        declarations: list[Declaration],
    ) -> tuple[str, list[str]]:
        notes: list[str] = []
        statuses: list[str] = []
        for declaration in declarations:
            source = self._files.get(declaration.file)
            if source is None:
                statuses.append("unknown")
                notes.append(f"{declaration.file}: source unavailable")
                continue
            if target_module in source.imports:
                statuses.append("direct")
                notes.append(f"{declaration.file}: imports {target_module} directly")
            else:
                statuses.append("missing")
                notes.append(f"{declaration.file}: add `import {target_module}` if replacing here")
        notes = list(dict.fromkeys(notes))
        if statuses and all(status == "direct" for status in statuses):
            return "direct", notes
        if "missing" in statuses:
            return "missing", notes
        return "unknown", notes

    def references_to(self, declarations: list[Declaration]) -> list[SourceReference]:
        references: dict[tuple[Path, int, int, str], SourceReference] = {}
        for declaration in declarations:
            tokens = _reference_tokens(declaration)
            if not tokens:
                continue
            for source in self._files.values():
                for reference in source.references_to(
                    tokens=tokens,
                    declaration_file=declaration.file,
                    declaration_start=declaration.span.start.line,
                    declaration_end=declaration.span.end.line,
                ):
                    references[(reference.file, reference.line, reference.column, reference.text)] = reference
        return sorted(references.values(), key=lambda item: (str(item.file), item.line, item.column))


class _SourceFile:
    def __init__(self, *, path: Path, lines: tuple[str, ...], stripped_lines: tuple[str, ...]) -> None:
        self.path = path
        self.lines = lines
        self.stripped_lines = stripped_lines
        self.imports = tuple(_parse_imports(stripped_lines))

    @classmethod
    def read(cls, path: Path) -> "_SourceFile":
        text = path.read_text(encoding="utf-8")
        lines = tuple(text.splitlines())
        return cls(path=path, lines=lines, stripped_lines=tuple(_strip_comments_by_line(lines)))

    def references_to(
        self,
        *,
        tokens: tuple[str, ...],
        declaration_file: Path,
        declaration_start: int,
        declaration_end: int,
    ) -> list[SourceReference]:
        pattern = re.compile(r"(?<![A-Za-z0-9_'.])(" + "|".join(map(re.escape, tokens)) + r")(?![A-Za-z0-9_'.])")
        references: list[SourceReference] = []
        for index, stripped in enumerate(self.stripped_lines, start=1):
            if self.path == declaration_file and declaration_start <= index <= declaration_end:
                continue
            match = pattern.search(stripped)
            if match is None:
                continue
            references.append(
                SourceReference(
                    file=self.path,
                    line=index,
                    column=match.start() + 1,
                    text=self.lines[index - 1].strip(),
                )
            )
        return references


def _reference_tokens(declaration: Declaration) -> tuple[str, ...]:
    tokens = [declaration.name]
    if declaration.short_name and declaration.short_name != declaration.name:
        tokens.append(declaration.short_name)
    if declaration.display_name and declaration.display_name not in tokens:
        tokens.append(declaration.display_name)
    return tuple(sorted(set(tokens), key=lambda token: (-len(token), token)))


def _parse_imports(lines: tuple[str, ...]) -> list[str]:
    imports: list[str] = []
    for line in lines:
        stripped = line.strip()
        if not stripped.startswith("import "):
            continue
        imports.extend(part for part in stripped.removeprefix("import ").split() if part)
    return imports


def _strip_comments_by_line(lines: tuple[str, ...]) -> list[str]:
    stripped_lines: list[str] = []
    depth = 0
    for line in lines:
        output: list[str] = []
        index = 0
        while index < len(line):
            two = line[index : index + 2]
            if depth == 0 and two == "--":
                break
            if two == "/-":
                depth += 1
                index += 2
                continue
            if two == "-/" and depth:
                depth -= 1
                index += 2
                continue
            if depth == 0:
                output.append(line[index])
            index += 1
        stripped_lines.append("".join(output))
    return stripped_lines
