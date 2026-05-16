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
    row = _row(payloads, "Tiny.role_marker_conclusion")
    assert row["feature_version"] == "features.roles.v1"
    assert row["binder_count"] > 0
    assert row["role_features"]
    assert row["low_signal_markers"] == []
    assert set(row["fingerprints"]) == {
        "statement",
        "safe_binder_permutation",
        "connective_shape",
        "conclusion_shape",
    }
    assert all(
        isinstance(value, str) and value and "placeholder" not in value
        for value in row["fingerprints"].values()
    )


def test_extract_remains_display_only(worker_bin: Path) -> None:
    envelopes = _extract(worker_bin)
    payloads = [envelope["payload"] for envelope in envelopes[:-1]]
    forbidden = {"fingerprints", "role_features", "binder_count", "low_signal_markers"}

    assert payloads
    assert all(forbidden.isdisjoint(payload) for payload in payloads)


def test_role_features_follow_protocol_shape(worker_bin: Path) -> None:
    payloads = _feature_payloads(worker_bin)
    row = _row(payloads, "Tiny.role_marker_conclusion")
    allowed_payload_fields = {
        "declaration_id",
        "feature_version",
        "fingerprints",
        "role_features",
        "binder_count",
        "low_signal_markers",
    }
    allowed_role_fields = {"role", "key", "display"}

    assert set(row) == allowed_payload_fields
    assert row["role_features"]
    for feature in row["role_features"]:
        assert set(feature).issubset(allowed_role_fields)
        assert feature["role"] in {
            "conclusion_const",
            "conclusion_head",
            "hypothesis_const",
            "hypothesis_head",
            "binder_domain_head",
        }
        assert isinstance(feature["key"], str) and feature["key"]


def test_same_constant_role_separation(worker_bin: Path) -> None:
    payloads = _feature_payloads(worker_bin)
    conclusion = _row(payloads, "Tiny.role_marker_conclusion")
    hypothesis = _row(payloads, "Tiny.role_marker_hypothesis")

    conclusion_key = _role_key(conclusion, "conclusion_const", "Tiny.RoleMarker")
    hypothesis_key = _role_key(hypothesis, "hypothesis_const", "Tiny.RoleMarker")

    assert conclusion_key is not None
    assert hypothesis_key is not None
    assert conclusion_key != hypothesis_key


def test_broad_heads_are_marked_low_signal(worker_bin: Path) -> None:
    payloads = _feature_payloads(worker_bin)

    eq_row = _row(payloads, "Tiny.broad_eq_only")
    iff_row = _row(payloads, "Tiny.broad_iff_only")

    assert "broad_head:Eq" in eq_row["low_signal_markers"]
    assert "broad_head:Iff" in iff_row["low_signal_markers"]


def test_generated_feature_rows_stay_protocol_only(worker_bin: Path) -> None:
    payloads = [envelope["payload"] for envelope in _features(worker_bin, include_generated=True)[:-1]]
    generated_rows = [
        payload
        for payload in payloads
        if ":Tiny.GeneratedProbe.rec" in payload["declaration_id"]
        or ":Tiny.GeneratedProbe.casesOn" in payload["declaration_id"]
    ]
    forbidden = {
        "name_tokens",
        "namespace_path",
        "source_fingerprint",
        "source_skeleton",
        "proof_skeleton",
    }

    assert generated_rows
    for row in generated_rows:
        assert forbidden.isdisjoint(row)
        assert all(forbidden.isdisjoint(feature) for feature in row["role_features"])


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


def _features(worker_bin: Path, *, include_generated: bool = False) -> list[dict[str, Any]]:
    completed = _run_worker(worker_bin, _request("features", include_generated=include_generated))
    assert completed.returncode == 0, completed.stderr + completed.stdout
    return [json.loads(line) for line in completed.stdout.splitlines() if line.strip()]


def _extract(worker_bin: Path) -> list[dict[str, Any]]:
    completed = _run_worker(worker_bin, _request("extract"))
    assert completed.returncode == 0, completed.stderr + completed.stdout
    return [json.loads(line) for line in completed.stdout.splitlines() if line.strip()]


def _request(command: str, *, include_generated: bool = False) -> dict[str, Any]:
    return {
        "schema_version": "lean-dup.worker.v1",
        "request_id": f"{command}-fixture",
        "command": command,
        "payload": {
            "workspace_root": str(FIXTURE),
            "modules": [{"module": "Tiny.Basic", "origin": "workspace"}],
            "include_private": True,
            "include_generated": include_generated,
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


def _role_key(row: dict[str, Any], role: str, display: str) -> str | None:
    matches = [
        feature["key"]
        for feature in row["role_features"]
        if feature["role"] == role and feature.get("display") == display
    ]
    assert len(matches) <= 1
    return matches[0] if matches else None
