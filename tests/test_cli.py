from __future__ import annotations

import json
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
FIXTURE = ROOT / "tests" / "fixtures" / "tiny"
EXTERNAL = ROOT / "tests" / "fixtures" / "external"


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
            "--profile",
        ],
        cwd=ROOT,
        check=False,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    assert completed.returncode == 0, completed.stderr + completed.stdout
    payload = json.loads(completed.stdout)
    assert payload["declaration_count"] >= 10
    assert "groups" in payload
    assert "warnings" in payload


def test_cli_external_index(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("LEAN_DUP_CACHE_DIR", str(tmp_path))
    index_completed = subprocess.run(
        [
            "uv",
            "run",
            "lean-dup",
            "index",
            "--workspace",
            str(EXTERNAL),
            "--module",
            "External",
            "--label",
            "fixture",
        ],
        cwd=ROOT,
        check=False,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    assert index_completed.returncode == 0, index_completed.stderr + index_completed.stdout
    assert "label: fixture" in index_completed.stdout

    audit_completed = subprocess.run(
        [
            "uv",
            "run",
            "lean-dup",
            "audit",
            "--workspace",
            str(FIXTURE),
            "--module",
            "Tiny",
            "--compare-index",
            "fixture",
            "--format",
            "json",
        ],
        cwd=ROOT,
        check=False,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    assert audit_completed.returncode == 0, audit_completed.stderr + audit_completed.stdout
    payload = json.loads(audit_completed.stdout)
    assert payload["external_indexes"][0]["label"] == "fixture"
    assert any(
        member["origin"] == "external:fixture"
        for group in payload["groups"]
        for member in group["members"]
    )


def test_cli_text_filters_by_priority() -> None:
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
            "text",
            "--min-priority",
            "high",
        ],
        cwd=ROOT,
        check=False,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    assert completed.returncode == 0, completed.stderr + completed.stdout
    assert "priority=high" in completed.stdout
    assert "priority=low" not in completed.stdout
