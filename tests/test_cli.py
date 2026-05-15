from __future__ import annotations

import json
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
FIXTURE = ROOT / "tests" / "fixtures" / "tiny"


def test_cli_audit_json() -> None:
    completed = subprocess.run(
        [
            "uv",
            "run",
            "lean-dup",
            "audit",
            "--workspace",
            str(FIXTURE),
            "--module",
            "Tiny",
            "--format",
            "json",
        ],
        cwd=ROOT,
        check=False,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    assert completed.returncode == 0, completed.stderr + completed.stdout
    payload = json.loads(completed.stdout)
    assert payload["declaration_count"] >= 4
    assert "groups" in payload
