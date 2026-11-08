import json

import aggregate_real_e2e_reports as subject


def test_failed_attempt_is_diagnostic_and_later_valid_receipt_can_cover_scenario(
    tmp_path, monkeypatch
):
    scenario = {"id": "real-chain", "suite": ["full"]}
    monkeypatch.setattr(subject, "load_matrix", lambda _: [scenario])
    monkeypatch.setattr(
        subject,
        "validate_receipt",
        lambda path, selected, execution_id: ({"platforms": ["cli"]}, []),
    )
    monkeypatch.setattr(
        subject,
        "audit_coverage",
        lambda *args, **kwargs: {"complete": True},
    )

    failed = tmp_path / "failed.json"
    failed.write_text(json.dumps({
        "results": [{"id": "real-chain", "passed": False}],
    }), encoding="utf-8")
    passed = tmp_path / "passed.json"
    passed.write_text(json.dumps({
        "results": [{
            "id": "real-chain",
            "passed": True,
            "receipt": str(tmp_path / "receipt.json"),
            "evidence": {"executionId": "execution-1"},
        }],
    }), encoding="utf-8")

    result = subject.aggregate_reports(
        [failed, passed], matrix_path=tmp_path / "matrix.json", suite="full"
    )

    assert result["complete"] is True
    assert result["errors"] == []
    assert result["acceptedScenarios"] == ["real-chain"]
    assert result["rejectedResults"] == [{
        "scenarioId": "real-chain",
        "report": str(failed),
        "reason": "scenario did not pass",
    }]
