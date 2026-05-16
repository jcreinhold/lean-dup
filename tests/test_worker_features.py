from __future__ import annotations

import json
import subprocess
import time
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


def test_features_emit_non_placeholder_fingerprints(worker_bin: Path) -> None:
    envelopes = _features(worker_bin)

    assert envelopes
    assert envelopes[-1]["kind"] == "complete"
    assert sum(envelope["kind"] == "complete" for envelope in envelopes) == 1
    assert envelopes[-1]["payload"]["row_counts"]["feature_row"] == len(envelopes) - 1

    payloads = [envelope["payload"] for envelope in envelopes[:-1]]
    same_left = _row(payloads, "Tiny.same_left")
    assert same_left["feature_version"] == "features.canonical.v1"
    assert same_left["binder_count"] > 0
    assert same_left["role_features"] == []
    assert same_left["low_signal_markers"] == []
    assert set(same_left["fingerprints"]) == {
        "statement",
        "safe_binder_permutation",
        "connective_shape",
        "conclusion_shape",
    }
    assert all(
        isinstance(value, str) and value and "placeholder" not in value
        for value in same_left["fingerprints"].values()
    )


def test_extract_remains_display_only(worker_bin: Path) -> None:
    envelopes = _extract(worker_bin)
    payloads = [envelope["payload"] for envelope in envelopes[:-1]]
    forbidden = {"fingerprints", "role_features", "binder_count"}

    assert payloads
    assert all(forbidden.isdisjoint(payload) for payload in payloads)


def test_exact_statement_fingerprint_is_alpha_stable(worker_bin: Path) -> None:
    payloads = _feature_payloads(worker_bin)

    left = _row(payloads, "Tiny.same_left")
    right = _row(payloads, "Tiny.same_right")

    assert left["fingerprints"]["statement"] == right["fingerprints"]["statement"]


def test_independent_hypotheses_share_safe_permutation(worker_bin: Path) -> None:
    payloads = _feature_payloads(worker_bin)

    left = _row(payloads, "Tiny.independent_arrow_left")
    right = _row(payloads, "Tiny.independent_arrow_right")

    assert left["fingerprints"]["statement"] != right["fingerprints"]["statement"]
    assert (
        left["fingerprints"]["safe_binder_permutation"]
        == right["fingerprints"]["safe_binder_permutation"]
    )


def test_dependent_binders_are_not_reordered(worker_bin: Path) -> None:
    payloads = _feature_payloads(worker_bin)

    left = _row(payloads, "Tiny.dependent_left")
    right = _row(payloads, "Tiny.dependent_right")

    assert (
        left["fingerprints"]["safe_binder_permutation"]
        != right["fingerprints"]["safe_binder_permutation"]
    )


def test_commutative_prop_connectives_share_connective_shape(worker_bin: Path) -> None:
    payloads = _feature_payloads(worker_bin)

    left = _row(payloads, "Tiny.connective_and_left")
    right = _row(payloads, "Tiny.connective_and_right")

    assert left["fingerprints"]["statement"] != right["fingerprints"]["statement"]
    assert left["fingerprints"]["connective_shape"] == right["fingerprints"]["connective_shape"]


def test_symmetric_eq_shares_connective_shape(worker_bin: Path) -> None:
    payloads = _feature_payloads(worker_bin)

    left = _row(payloads, "Tiny.symmetric_eq_left")
    right = _row(payloads, "Tiny.symmetric_eq_right")

    assert left["fingerprints"]["statement"] != right["fingerprints"]["statement"]
    assert left["fingerprints"]["connective_shape"] == right["fingerprints"]["connective_shape"]


def test_different_domains_do_not_collapse_to_broad_key(worker_bin: Path) -> None:
    payloads = _feature_payloads(worker_bin)

    nat_row = _row(payloads, "Tiny.nat_domain_key")
    bool_row = _row(payloads, "Tiny.bool_domain_key")

    assert nat_row["fingerprints"]["connective_shape"] != bool_row["fingerprints"]["connective_shape"]
    assert nat_row["fingerprints"]["conclusion_shape"] != bool_row["fingerprints"]["conclusion_shape"]


def test_universe_structure_is_preserved_in_lean_fingerprints(worker_bin: Path) -> None:
    payloads = _feature_payloads(worker_bin)

    left = _row(payloads, "Tiny.universe_structure_left")
    right = _row(payloads, "Tiny.universe_structure_right")

    assert left["fingerprints"]["statement"] != right["fingerprints"]["statement"]
    assert (
        left["fingerprints"]["safe_binder_permutation"]
        != right["fingerprints"]["safe_binder_permutation"]
    )


def test_unknown_declaration_id_is_invalid_request(worker_bin: Path) -> None:
    request = _request("features")
    request["payload"]["declaration_ids"] = ["workspace:Tiny.Basic:Tiny.not_real"]
    completed = _run_worker(worker_bin, request)

    assert completed.returncode != 0
    envelopes = [json.loads(line) for line in completed.stdout.splitlines() if line.strip()]
    assert envelopes[-1]["kind"] == "error"
    assert envelopes[-1]["payload"]["code"] == "invalid_request"


def test_features_command_has_no_fixture_scale_regression(worker_bin: Path) -> None:
    started = time.perf_counter()
    envelopes = _features(worker_bin)
    elapsed = time.perf_counter() - started

    assert envelopes[-1]["kind"] == "complete"
    assert elapsed < 5.0


def _feature_payloads(worker_bin: Path) -> list[dict[str, Any]]:
    return [envelope["payload"] for envelope in _features(worker_bin)[:-1]]


def _features(worker_bin: Path) -> list[dict[str, Any]]:
    completed = _run_worker(worker_bin, _request("features"))
    assert completed.returncode == 0, completed.stderr + completed.stdout
    return [json.loads(line) for line in completed.stdout.splitlines() if line.strip()]


def _extract(worker_bin: Path) -> list[dict[str, Any]]:
    completed = _run_worker(worker_bin, _request("extract"))
    assert completed.returncode == 0, completed.stderr + completed.stdout
    return [json.loads(line) for line in completed.stdout.splitlines() if line.strip()]


def _request(command: str) -> dict[str, Any]:
    return {
        "schema_version": "lean-dup.worker.v1",
        "request_id": f"{command}-fixture",
        "command": command,
        "payload": {
            "workspace_root": str(FIXTURE),
            "modules": [{"module": "Tiny.Basic", "origin": "workspace"}],
            "include_private": True,
            "include_generated": False,
        },
    }


def _run_worker(worker_bin: Path, request: dict[str, Any]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["lake", "env", str(worker_bin)],
        cwd=FIXTURE,
        check=False,
        input=json.dumps(request),
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )


def _row(payloads: list[dict[str, Any]], qualified_name: str) -> dict[str, Any]:
    suffix = f":{qualified_name}"
    matches = [payload for payload in payloads if payload["declaration_id"].endswith(suffix)]
    assert len(matches) == 1, qualified_name
    return matches[0]
