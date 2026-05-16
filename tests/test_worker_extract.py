from __future__ import annotations

import json
import subprocess
from pathlib import Path
from typing import Any

import pytest


ROOT = Path(__file__).resolve().parents[1]
WORKER_ROOT = ROOT / "lean"
FIXTURE = ROOT / "tests" / "fixtures" / "tiny"


@pytest.fixture(scope="module")
def worker_bin() -> Path:
    completed_worker = subprocess.run(
        ["lake", "build"],
        cwd=WORKER_ROOT,
        check=False,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    assert completed_worker.returncode == 0, completed_worker.stderr + completed_worker.stdout

    completed_fixture = subprocess.run(
        ["lake", "build"],
        cwd=FIXTURE,
        check=False,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    assert completed_fixture.returncode == 0, completed_fixture.stderr + completed_fixture.stdout

    return WORKER_ROOT / ".lake" / "build" / "bin" / "lean_dup_worker"


def test_extract_emits_declaration_rows_and_complete(worker_bin: Path) -> None:
    envelopes = _extract(
        worker_bin,
        modules=[
            {"module": "Tiny.Basic", "origin": "workspace"},
            {"module": "Tiny.More", "origin": "workspace"},
        ],
        include_private=True,
        include_generated=False,
    )

    assert envelopes
    assert envelopes[-1]["kind"] == "complete"
    assert sum(envelope["kind"] == "complete" for envelope in envelopes) == 1
    assert not any(envelope["kind"] == "feature_row" for envelope in envelopes)
    assert all(envelope["kind"] == "declaration_row" for envelope in envelopes[:-1])
    assert envelopes[-1]["payload"]["row_counts"]["declaration_row"] == len(envelopes) - 1

    payloads = [envelope["payload"] for envelope in envelopes[:-1]]
    same_left = _row(payloads, "same_left")
    assert same_left["origin"] == "workspace"
    assert same_left["module"] == "Tiny.Basic"
    assert same_left["qualified_name"] == "Tiny.same_left"
    assert same_left["source_span"] is not None
    assert same_left["source_span"]["file"].endswith("Tiny/Basic.lean")
    assert "theorem same_left :" in same_left["statement_text"]

    forbidden = {
        "fingerprints",
        "role_features",
        "normalized_type",
        "type_fingerprint",
        "permutation_fingerprint",
        "connective_fingerprint",
        "conclusion_fingerprint",
        "constants",
        "binder_count",
        "low_signal_markers",
        "source_fingerprint",
    }
    assert all(forbidden.isdisjoint(payload) for payload in payloads)


def test_extract_filters_private_declarations(worker_bin: Path) -> None:
    public_only = _extract(
        worker_bin,
        modules=[{"module": "Tiny.Basic", "origin": "workspace"}],
        include_private=False,
        include_generated=False,
    )
    public_payloads = [envelope["payload"] for envelope in public_only[:-1]]
    assert not any(payload["visibility"] == "private" for payload in public_payloads)

    with_private = _extract(
        worker_bin,
        modules=[{"module": "Tiny.Basic", "origin": "workspace"}],
        include_private=True,
        include_generated=False,
    )
    private_payloads = [envelope["payload"] for envelope in with_private[:-1]]
    private_left = _row(private_payloads, "private_dup_left")
    assert private_left["visibility"] == "private"
    assert private_left["display_name"] == "private_dup_left"
    assert "_private" in private_left["qualified_name"]
    assert "private" in private_left["modifiers"]


def test_extract_filters_generated_declarations(worker_bin: Path) -> None:
    without_generated = _extract(
        worker_bin,
        modules=[{"module": "Tiny.Basic", "origin": "workspace"}],
        include_private=True,
        include_generated=False,
    )
    without_payloads = [envelope["payload"] for envelope in without_generated[:-1]]
    assert not any("generated" in payload["status_flags"] for payload in without_payloads)
    assert not any(
        payload["qualified_name"].startswith("Tiny.GeneratedProbe.")
        and payload["display_name"] in {"rec", "recOn", "casesOn", "noConfusion", "ctorIdx"}
        for payload in without_payloads
    )

    with_generated = _extract(
        worker_bin,
        modules=[{"module": "Tiny.Basic", "origin": "workspace"}],
        include_private=True,
        include_generated=True,
    )
    generated_payloads = [
        envelope["payload"]
        for envelope in with_generated[:-1]
        if "generated" in envelope["payload"]["status_flags"]
    ]
    assert generated_payloads
    assert any(
        payload["qualified_name"].startswith("Tiny.GeneratedProbe.")
        and payload["display_name"] in {"rec", "recOn", "casesOn", "noConfusion"}
        for payload in generated_payloads
    )


def test_extract_preserves_requested_import_origin(worker_bin: Path) -> None:
    envelopes = _extract(
        worker_bin,
        modules=[
            {"module": "Tiny.Basic", "origin": "workspace"},
            {"module": "Other", "origin": "direct-import"},
        ],
        include_private=False,
        include_generated=False,
    )
    payloads = [envelope["payload"] for envelope in envelopes[:-1]]
    imported = _row(payloads, "imported_dup")
    assert imported["qualified_name"] == "Other.imported_dup"
    assert imported["origin"] == "direct-import"
    assert imported["module"] == "Other"


def _extract(
    worker_bin: Path,
    *,
    modules: list[dict[str, str]],
    include_private: bool,
    include_generated: bool,
) -> list[dict[str, Any]]:
    request = {
        "schema_version": "lean-dup.worker.v1",
        "request_id": "extract-fixture",
        "command": "extract",
        "payload": {
            "workspace_root": str(FIXTURE),
            "modules": modules,
            "include_private": include_private,
            "include_generated": include_generated,
        },
    }
    completed = subprocess.run(
        ["lake", "env", str(worker_bin)],
        cwd=FIXTURE,
        check=False,
        input=json.dumps(request),
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    assert completed.returncode == 0, completed.stderr + completed.stdout
    return [
        json.loads(line)
        for line in completed.stdout.splitlines()
        if line.strip()
    ]


def _row(payloads: list[dict[str, Any]], display_name: str) -> dict[str, Any]:
    matches = [payload for payload in payloads if payload["display_name"] == display_name]
    assert len(matches) == 1, display_name
    return matches[0]
