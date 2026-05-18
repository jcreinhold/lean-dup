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


def test_version_reports_semantic_probe(worker_bin: Path) -> None:
    completed = _run_worker(worker_bin, _request("version", pairs=[]))
    assert completed.returncode == 0, completed.stderr + completed.stdout
    envelopes = _envelopes(completed)

    assert envelopes[0]["kind"] == "version_result"
    assert envelopes[0]["payload"]["semantic_versions"]["probe"] == "probe.semantic.v1"
    assert envelopes[-1]["kind"] == "complete"


def test_probe_result_protocol_shape(worker_bin: Path) -> None:
    row = _probe_row(worker_bin, "shape", "Tiny.same_left", "Tiny.same_right")
    allowed = {
        "pair_id",
        "left_declaration_id",
        "right_declaration_id",
        "status",
        "same_statement",
        "same_up_to_safe_reordering",
        "connective_equivalent",
        "specializes_left_to_right",
        "specializes_right_to_left",
        "mutual_implication_shape",
        "same_reducible_definition",
        "message",
    }

    assert set(row) == allowed
    assert row["pair_id"] == "shape"
    assert row["status"] == "ok"
    assert row["message"] is None


def test_same_statement_confirmed_by_lean(worker_bin: Path) -> None:
    row = _probe_row(worker_bin, "same", "Tiny.same_left", "Tiny.same_right")

    assert row["status"] == "ok"
    assert row["same_statement"] is True
    assert row["same_up_to_safe_reordering"] is False


def test_safe_reordered_hypotheses(worker_bin: Path) -> None:
    row = _probe_row(
        worker_bin,
        "reordered",
        "Tiny.independent_arrow_left",
        "Tiny.independent_arrow_right",
    )

    assert row["status"] == "ok"
    assert row["same_statement"] is False
    assert row["same_up_to_safe_reordering"] is True
    assert row["mutual_implication_shape"] is True


def test_dependent_binder_reordering_is_not_confirmed(worker_bin: Path) -> None:
    row = _probe_row(worker_bin, "dependent", "Tiny.dependent_left", "Tiny.dependent_right")

    assert row["status"] == "ok"
    assert row["same_statement"] is False
    assert row["same_up_to_safe_reordering"] is False
    assert row["mutual_implication_shape"] is False


def test_connective_equivalence_for_and_and_eq(worker_bin: Path) -> None:
    rows = _probe_rows(
        worker_bin,
        [
            _pair("and", "Tiny.connective_and_left", "Tiny.connective_and_right"),
            _pair("eq", "Tiny.symmetric_eq_left", "Tiny.symmetric_eq_right"),
        ],
    )

    assert rows["and"]["status"] == "ok"
    assert rows["and"]["connective_equivalent"] is True
    assert rows["eq"]["status"] == "ok"
    assert rows["eq"]["connective_equivalent"] is True


def test_structural_specialization_direction(worker_bin: Path) -> None:
    row = _probe_row(
        worker_bin,
        "specializes",
        "Tiny.specialization_specific",
        "Tiny.specialization_general",
    )

    assert row["status"] == "ok"
    assert row["specializes_left_to_right"] is True
    assert row["specializes_right_to_left"] is False
    assert row["mutual_implication_shape"] is False


def test_same_conclusion_false_positive_is_not_specialized(worker_bin: Path) -> None:
    row = _probe_row(
        worker_bin,
        "same-conclusion",
        "Tiny.same_conclusion_nat_domain",
        "Tiny.same_conclusion_bool_domain",
    )

    assert row["status"] == "ok"
    assert row["same_statement"] is False
    assert row["specializes_left_to_right"] is False
    assert row["specializes_right_to_left"] is False
    assert row["mutual_implication_shape"] is False


def test_reducible_definition_match_below_guard(worker_bin: Path) -> None:
    row = _probe_row(
        worker_bin,
        "reducible",
        "Tiny.probe_small_def_left",
        "Tiny.probe_small_def_right",
    )

    assert row["status"] == "ok"
    assert row["same_reducible_definition"] is True
    assert row["same_statement"] is False


def test_private_declarations_are_available_when_requested(worker_bin: Path) -> None:
    row = _probe_row(
        worker_bin,
        "private",
        "_private.Tiny.Basic.0.Tiny.private_dup_left",
        "_private.Tiny.Basic.0.Tiny.private_dup_right",
    )

    assert row["status"] == "ok"
    assert row["same_statement"] is True


def test_opaque_definition_is_unavailable_with_bounded_message(worker_bin: Path) -> None:
    row = _probe_row(
        worker_bin,
        "opaque",
        "Tiny.probe_opaque_def_left",
        "Tiny.probe_opaque_def_right",
    )

    assert row["status"] == "unavailable"
    assert row["same_reducible_definition"] is False
    assert isinstance(row["message"], str)
    assert "opaque" in row["message"]
    assert len(row["message"]) <= 180


def test_missing_declaration_is_pair_local_unavailable(worker_bin: Path) -> None:
    rows = _probe_rows(
        worker_bin,
        [_pair("missing", "Tiny.not_real", "Tiny.same_left")],
    )

    assert rows["missing"]["status"] == "unavailable"
    assert rows["missing"]["same_statement"] is False
    assert len(rows["missing"]["message"]) <= 180


def test_invalid_pair_is_nonfatal(worker_bin: Path) -> None:
    rows = _probe_rows(
        worker_bin,
        [_pair("invalid", "Tiny.same_left", "Tiny.same_left")],
    )

    assert rows["invalid"]["status"] == "invalid_pair"
    assert rows["invalid"]["same_statement"] is False
    assert len(rows["invalid"]["message"]) <= 180


def test_max_pairs_is_fatal_invalid_request(worker_bin: Path) -> None:
    request = _request(
        "probe",
        pairs=[
            _pair("first", "Tiny.same_left", "Tiny.same_right"),
            _pair("second", "Tiny.connective_and_left", "Tiny.connective_and_right"),
        ],
    )
    request["payload"]["max_pairs"] = 1
    completed = _run_worker(worker_bin, request)

    assert completed.returncode != 0
    envelopes = _envelopes(completed)
    assert envelopes[-1]["kind"] == "error"
    assert envelopes[-1]["payload"]["code"] == "invalid_request"


def test_probe_batch_has_no_fixture_scale_regression(worker_bin: Path) -> None:
    started = time.perf_counter()
    envelopes = _probe_envelopes(
        worker_bin,
        [
            _pair("same", "Tiny.same_left", "Tiny.same_right"),
            _pair("reordered", "Tiny.independent_arrow_left", "Tiny.independent_arrow_right"),
            _pair("and", "Tiny.connective_and_left", "Tiny.connective_and_right"),
            _pair("specializes", "Tiny.specialization_specific", "Tiny.specialization_general"),
            _pair("reducible", "Tiny.probe_small_def_left", "Tiny.probe_small_def_right"),
        ],
    )
    elapsed = time.perf_counter() - started

    assert envelopes[-1]["kind"] == "complete"
    assert elapsed < 5.0


def _probe_row(worker_bin: Path, pair_id: str, left: str, right: str) -> dict[str, Any]:
    return _probe_rows(worker_bin, [_pair(pair_id, left, right)])[pair_id]


def _probe_rows(worker_bin: Path, pairs: list[dict[str, str]]) -> dict[str, dict[str, Any]]:
    envelopes = _probe_envelopes(worker_bin, pairs)
    rows = [envelope["payload"] for envelope in envelopes[:-1]]
    return {row["pair_id"]: row for row in rows}


def _probe_envelopes(worker_bin: Path, pairs: list[dict[str, str]]) -> list[dict[str, Any]]:
    completed = _run_worker(worker_bin, _request("probe", pairs=pairs))
    assert completed.returncode == 0, completed.stderr + completed.stdout
    envelopes = _envelopes(completed)

    assert envelopes
    assert envelopes[-1]["kind"] == "complete"
    assert sum(envelope["kind"] == "complete" for envelope in envelopes) == 1
    assert all(envelope["kind"] in {"progress", "probe_result", "complete"} for envelope in envelopes)
    assert envelopes[-1]["payload"]["row_counts"]["probe_result"] == len(pairs)
    return [envelope for envelope in envelopes if envelope["kind"] in {"probe_result", "complete"}]


def _pair(pair_id: str, left: str, right: str) -> dict[str, str]:
    return {
        "pair_id": pair_id,
        "left_declaration_id": _decl_id(left),
        "right_declaration_id": _decl_id(right),
    }


def _decl_id(qualified_name: str) -> str:
    return f"workspace:Tiny.Basic:{qualified_name}"


def _request(command: str, *, pairs: list[dict[str, str]]) -> dict[str, Any]:
    return {
        "schema_version": "lean-dup.worker.v1",
        "request_id": f"{command}-fixture",
        "command": command,
        "payload": {
            "workspace_root": str(FIXTURE),
            "modules": [{"module": "Tiny.Basic", "origin": "workspace"}],
            "include_private": True,
            "include_generated": False,
            "pairs": pairs,
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


def _envelopes(completed: subprocess.CompletedProcess[str]) -> list[dict[str, Any]]:
    return [json.loads(line) for line in completed.stdout.splitlines() if line.strip()]
