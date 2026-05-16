"""Lake workspace discovery."""

from __future__ import annotations

import re
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class Workspace:
    """Resolved Lake workspace facts."""

    root: Path
    workspace_modules: tuple[str, ...]
    extraction_modules: tuple[ModuleEntry, ...]

    @property
    def modules(self) -> tuple[str, ...]:
        """Return local workspace modules for compatibility with older callers."""

        return self.workspace_modules


@dataclass(frozen=True)
class ModuleEntry:
    """One module included in extraction."""

    name: str
    origin: str


def resolve_workspace(
    path: Path,
    module_root: str | None,
    *,
    include_imports: bool = False,
    import_roots: tuple[str, ...] = (),
) -> Workspace:
    """Resolve modules to audit for one Lake workspace."""

    root = path.expanduser().resolve()
    if not root.exists():
        msg = f"workspace does not exist: {root}"
        raise RuntimeError(msg)
    if not ((root / "lakefile.toml").exists() or (root / "lakefile.lean").exists()):
        msg = f"not a Lake workspace: {root}"
        raise RuntimeError(msg)
    roots = (module_root,) if module_root else _infer_roots(root)
    modules = sorted(
        {module for root_module in roots for module in _modules_under(root, root_module)}
    )
    if not modules:
        msg = f"no Lean modules found under {root}"
        raise RuntimeError(msg)
    entries = [ModuleEntry(name=module, origin="workspace") for module in modules]
    if include_imports:
        workspace_set = set(modules)
        direct_imports = sorted(
            {
                imported
                for module in modules
                for imported in _direct_imports(module_to_file(root, module))
                if imported not in workspace_set
            }
        )
        entries.extend(
            ModuleEntry(name=module, origin="direct-import") for module in direct_imports
        )
    entries.extend(ModuleEntry(name=module, origin="named-import") for module in import_roots)
    deduped = tuple(dict.fromkeys(entries))
    return Workspace(root=root, workspace_modules=tuple(modules), extraction_modules=deduped)


def module_to_file(root: Path, module: str) -> Path:
    """Return the source file path for a module."""

    return root / Path(*module.split(".")).with_suffix(".lean")


def _infer_roots(root: Path) -> tuple[str, ...]:
    toml = root / "lakefile.toml"
    if toml.exists():
        text = toml.read_text(encoding="utf-8")
        names = re.findall(r"(?m)^\s*name\s*=\s*\"([A-Za-z_][\w.']*)\"", text)
        if names:
            return tuple(dict.fromkeys(names))
    top_level = sorted(path.stem for path in root.glob("*.lean") if path.name != "lakefile.lean")
    if top_level:
        return tuple(top_level)
    msg = "could not infer Lean library root; pass --module"
    raise RuntimeError(msg)


def _modules_under(root: Path, module_root: str) -> tuple[str, ...]:
    root_file = module_to_file(root, module_root)
    module_dir = root / Path(*module_root.split("."))
    modules: list[str] = []
    if root_file.exists():
        modules.append(module_root)
    if module_dir.exists():
        for path in module_dir.rglob("*.lean"):
            rel = path.relative_to(root).with_suffix("")
            modules.append(".".join(rel.parts))
    return tuple(modules)


def _direct_imports(path: Path) -> tuple[str, ...]:
    if not path.exists():
        return ()
    imports: list[str] = []
    for line in path.read_text(encoding="utf-8").splitlines():
        stripped = line.strip()
        if not stripped.startswith("import "):
            continue
        imports.extend(part for part in stripped.removeprefix("import ").split() if part)
    return tuple(imports)
