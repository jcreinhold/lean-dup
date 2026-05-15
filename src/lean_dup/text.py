"""Text normalization helpers."""

from __future__ import annotations

import hashlib
import re

LINE_COMMENT_RE = re.compile(r"--.*?$", re.MULTILINE)
SPACE_RE = re.compile(r"\s+")


def stable_hash(text: str) -> str:
    """Return one short stable hash for report grouping."""

    return hashlib.sha256(text.encode("utf-8")).hexdigest()[:24]


def strip_lean_comments(text: str) -> str:
    """Remove Lean line and nested block comments from source text."""

    without_lines = LINE_COMMENT_RE.sub("", text)
    output: list[str] = []
    index = 0
    depth = 0
    while index < len(without_lines):
        two = without_lines[index : index + 2]
        if two == "/-":
            depth += 1
            index += 2
        elif two == "-/" and depth:
            depth -= 1
            index += 2
        else:
            if depth == 0:
                output.append(without_lines[index])
            index += 1
    return "".join(output)


def normalize_source(text: str) -> str:
    """Return a whitespace/comment-insensitive source skeleton."""

    return SPACE_RE.sub(" ", strip_lean_comments(text)).strip()
