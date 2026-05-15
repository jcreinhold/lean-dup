"""Lake workspace discovery."""

from __future__ import annotations

import re
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class Workspace:
    """Resolved Lake workspace facts."""

    root: Path
    modules: tuple[str, ...]


def resolve_workspace(path: Path, module_root: str | None) -> Workspace:
    """Resolve modules to audit for one Lake workspace."""

    root = path.expanduser().resolve()
    if not root.exists():
        msg = f"workspace does not exist: {root}"
        raise RuntimeError(msg)
    if not ((root / "lakefile.toml").exists() or (root / "lakefile.lean").exists()):
        msg = f"not a Lake workspace: {root}"
        raise RuntimeError(msg)
    roots = (module_root,) if module_root else _infer_roots(root)
    modules = sorted({module for root_module in roots for module in _modules_under(root, root_module)})
    if not modules:
        msg = f"no Lean modules found under {root}"
        raise RuntimeError(msg)
    return Workspace(root=root, modules=tuple(modules))


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
