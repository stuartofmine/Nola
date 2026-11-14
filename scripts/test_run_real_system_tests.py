import importlib.util
import json
from pathlib import Path


MODULE_PATH = Path(__file__).with_name("run_real_system_tests.py")
SPEC = importlib.util.spec_from_file_location("run_real_system_tests", MODULE_PATH)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)

EVENTS_PATH = Path(__file__).with_name("real_e2e_events.py")
EVENTS_SPEC = importlib.util.spec_from_file_location("real_e2e_events", EVENTS_PATH)
EVENTS = importlib.util.module_from_spec(EVENTS_SPEC)
assert EVENTS_SPEC.loader is not None
EVENTS_SPEC.loader.exec_module(EVENTS)


def test_matrix_has_unique_auditable_cross_platform_scenarios():
    scenarios = MODULE.load_matrix(Path(__file__).with_name("real_e2e_matrix.json"))
    identifiers = [scenario["id"] for scenario in scenarios]
    assert len(identifiers) == len(set(identifiers))
    assert {"core", "cli", "web", "mac", "ios", "android"} == {
        platform for scenario in scenarios for platform in scenario["platforms"]
    }
    assert all(scenario["layer"] for scenario in scenarios)
    assert all(scenario["timeoutSeconds"] > 0 for scenario in scenarios)


def test_smoke_slice_contains_a_real_chain_for_every_client_platform():
    scenarios = MODULE.load_matrix(Path(__file__).with_name("real_e2e_matrix.json"))
    selected = MODULE.select_scenarios(scenarios, "smoke", set(), set())
    real_platforms = {
        platform
        for scenario in selected
        if scenario.get("realModel")
        for platform in scenario["platforms"]
    }
    assert real_platforms == {"cli", "web", "mac", "ios", "android"}
    assert not any("backend" in scenario["platforms"] for scenario in selected)


def test_explicit_scenario_selection_is_not_hidden_by_default_smoke_suite():
    scenarios = MODULE.load_matrix(Path(__file__).with_name("real_e2e_matrix.json"))
    selected = MODULE.select_scenarios(scenarios, None, set(), {"cli-memory"})
    assert [scenario["id"] for scenario in selected] == ["cli-memory"]


def test_invalid_or_duplicate_matrix_is_rejected(tmp_path):
    invalid = tmp_path / "matrix.json"
    invalid.write_text(json.dumps({
        "schemaVersion": 1,
        "contracts": {"contract": {"requiredAssertions": ["passed"], "validators": [], "requiredObservationKinds": []}},
        "scenarios": [
            {"id": "same", "contractId": "contract", "platforms": ["cli"], "layer": "system", "command": "true", "timeoutSeconds": 1, "execution": {"runtime":"native-process","productionCode":True,"productEntrypoint":True,"realProvider":False,"realPersistence":False,"realUI":False,"semanticJudge":False,"isolation":"temporary-profile"}},
            {"id": "same", "contractId": "contract", "platforms": ["web"], "layer": "system", "command": "true", "timeoutSeconds": 1, "execution": {"runtime":"http-service","productionCode":True,"productEntrypoint":True,"realProvider":False,"realPersistence":False,"realUI":False,"semanticJudge":False,"isolation":"temporary-profile"}},
        ],
    }))
    try:
        MODULE.load_matrix(invalid)
    except ValueError as error:
        assert "duplicate scenario id" in str(error)
    else:
        raise AssertionError("duplicate real E2E scenario was accepted")


def test_deleted_repository_entrypoints_cannot_contribute_declared_coverage(tmp_path):
    value = json.loads(Path(__file__).with_name("real_e2e_matrix.json").read_text())
    invalid = tmp_path / "matrix.json"
    for command in ("./scripts/deleted-audit-fixture.sh", "python3 scripts/deleted-audit-fixture.py"):
        value["scenarios"][0]["command"] = command
        invalid.write_text(json.dumps(value))
        try:
            MODULE.load_matrix(invalid)
        except ValueError as error:
            assert "references missing repository script" in str(error)
        else:
            raise AssertionError("a missing entrypoint was counted as a declared scenario")


def test_standalone_coverage_excludes_missing_scripts_and_exits_nonzero(tmp_path):
    import subprocess
    import sys
    value = json.loads(Path(__file__).with_name("real_e2e_matrix.json").read_text())
    missing = value["scenarios"][0]
    missing["command"] = "python3 scripts/deleted-audit-fixture.py"
    matrix = tmp_path / "matrix.json"
    matrix.write_text(json.dumps(value))
    result = subprocess.run([sys.executable, str(Path(__file__).with_name("real_e2e_coverage.py")),
        "--matrix", str(matrix), "--suite", "full"], capture_output=True, text=True, check=False)
    assert result.returncode == 1
    report = json.loads(result.stdout)
    assert report["complete"] is False
    assert missing["id"] not in report["consideredScenarios"]
    assert any("references missing repository script" in error for error in report["matrixErrors"])


def test_product_ui_scenario_without_ui_boundary_is_rejected(tmp_path):
    invalid = tmp_path / "matrix.json"
    invalid.write_text(json.dumps({
        "schemaVersion": 1,
        "contracts": {"contract": {"requiredAssertions": ["passed"], "validators": [], "requiredObservationKinds": []}},
        "scenarios": [{
            "id": "fake-ui",
            "contractId": "contract",
            "platforms": ["android"],
            "layer": "product-ui",
            "command": "true",
            "timeoutSeconds": 1,
            "realModel": True,
            "evidence": {
                "platforms": ["android"],
                "modules": ["chat"],
                "boundaries": ["android-provider"],
                "requiredObservationKinds": ["ui_screenshot", "accessibility_tree", "ui_timeline"],
            },
            "execution": {"runtime":"emulator","productionCode":True,"productEntrypoint":True,"realProvider":True,"realPersistence":False,"realUI":True,"semanticJudge":False,"isolation":"temporary-profile"},
        }],
    }))
    try:
        MODULE.load_matrix(invalid)
    except ValueError as error:
        assert "lacks a real UI boundary" in str(error)
    else:
        raise AssertionError("provider-only scenario was accepted as product UI")


def test_schema_six_contract_without_validators_is_rejected(tmp_path):
    invalid = tmp_path / "matrix.json"
    invalid.write_text(json.dumps({
        "schemaVersion": 1,
        "contracts": {"contract": {"requiredAssertions": ["passed"], "requiredObservationKinds": []}},
        "scenarios": [],
    }))
    try:
        MODULE.load_matrix(invalid)
    except ValueError as error:
        assert "invalid central validators" in str(error)
    else:
        raise AssertionError("schema 6 accepted a contract without validators")


def test_runner_rejects_skip_only_success(tmp_path):
    scenario = {
        "id": "skip-only",
        "platforms": ["ios"],
        "layer": "product",
        "timeoutSeconds": 5,
        "command": "printf '1 skipped in 0.01s\\n'",
        "evidence": {"platforms": ["ios"], "modules": ["chat"], "boundaries": ["ios-ui"]},
    }
    result = MODULE.run_scenario(scenario, tmp_path, {})
    assert result["returnCode"] == 0
    assert not result["passed"]
    assert result["skipMarkers"]


def test_runner_rejects_zero_exit_without_structured_evidence(tmp_path):
    scenario = {
        "id": "no-evidence",
        "platforms": ["cli"],
        "layer": "product",
        "timeoutSeconds": 5,
        "realModel": True,
        "command": "true",
        "evidence": {"platforms": ["cli"], "modules": ["chat"], "boundaries": ["cli"]},
    }
    result = MODULE.run_scenario(scenario, tmp_path, {})
    assert not result["passed"]
    assert result["evidence"]["steps"][0]["id"] == "scenario-process"
    assert result["evidence"]["steps"][0]["status"] == "failed"
    assert result["evidence"]["steps"][0]["observations"][0]["kind"] == "process_log"
    assert Path(result["evidence"]["steps"][0]["observations"][0]["path"]).is_file()
    assert "step did not pass: scenario-process" in result["evidenceErrors"]


def test_runner_records_timeout_metadata_when_adapter_never_reports(tmp_path):
    scenario = {
        "id": "timeout-before-evidence",
        "platforms": ["mac"],
        "layer": "product-ui",
        "timeoutSeconds": 1,
        "command": "sleep 5",
        "evidence": {
            "platforms": ["mac"],
            "modules": ["chat"],
            "boundaries": ["mac-ui"],
        },
    }
    result = MODULE.run_scenario(scenario, tmp_path, {})
    step = result["evidence"]["steps"][0]
    assert result["timedOut"] is True
    assert step["metrics"]["returnCode"] == 124
    assert step["metrics"]["timedOut"] is True


def test_runner_emits_heartbeat_for_a_long_running_scenario(tmp_path, capsys):
    scenario = {
        "id": "heartbeat",
        "platforms": ["web"],
        "layer": "system",
        "timeoutSeconds": 5,
        "command": "sleep 1.2",
        "evidence": {"platforms": ["web"], "modules": ["http"], "boundaries": ["host-http"]},
    }
    MODULE.run_scenario(scenario, tmp_path, {"NOLA_E2E_HEARTBEAT_SECONDS": "1"})
    assert "RUNNING" in capsys.readouterr().out


def test_runner_accepts_only_current_complete_evidence(tmp_path):
    writer = Path(__file__).with_name("real_e2e_evidence.py")
    provider_trace = tmp_path / "provider.ndjson"
    provider_trace.write_text(json.dumps({
        "event": "provider.response",
        "upstreamHost": "api.example.test",
        "upstreamPath": "/v1/chat/completions",
        "status": 200,
        "responseBytes": 42,
        "model": "real-model",
    }) + "\n")
    scenario = {
        "id": "structured",
        "platforms": ["cli"],
        "layer": "product",
        "timeoutSeconds": 5,
        "realModel": True,
        "command": (
            f"python3 -c \"import sys; sys.path.insert(0, '{writer.parent}'); "
            "from real_e2e_evidence import record_step; "
            "record_step('turn', platforms=['cli'], modules=['chat','tools'], "
            "boundaries=['cli','provider'], assertions={'terminal': True}, "
            f"observations=[{{'id':'provider','kind':'provider_trace','path':'{provider_trace}'}}])\""
        ),
        "evidence": {
            "platforms": ["cli"],
            "modules": ["chat", "tools"],
            "boundaries": ["cli", "provider"],
        },
        "_contract": {"requiredAssertions": ["terminal"], "perPlatformSteps": True},
    }
    result = MODULE.run_scenario(scenario, tmp_path, {})
    assert result["passed"], result
    assert result["evidence"]["executionId"]


def test_claimed_provider_count_cannot_satisfy_observed_provider_requirement(tmp_path):
    writer = Path(__file__).with_name("real_e2e_evidence.py")
    scenario = {
        "id": "self-reported-provider",
        "platforms": ["cli"],
        "layer": "system",
        "timeoutSeconds": 5,
        "realModel": True,
        "command": (
            f"python3 -c \"import sys; sys.path.insert(0, '{writer.parent}'); "
            "from real_e2e_evidence import record_step; "
            "record_step('turn', platforms=['cli'], modules=['agent'], boundaries=['provider'], "
            "real_provider_requests=99, assertions={'answer': True})\""
        ),
        "evidence": {
            "platforms": ["cli"],
            "modules": ["agent"],
            "boundaries": ["provider"],
            "requiredObservationKinds": ["provider_trace"],
        },
        "_contract": {"requiredAssertions": ["answer"]},
    }
    result = MODULE.run_scenario(scenario, tmp_path, {})
    assert not result["passed"]
    assert result["evidence"]["claimedRealProviderRequests"] == 99
    assert result["evidence"]["realProviderRequests"] == 0
    assert "missing observation kinds: provider_trace" in result["evidenceErrors"]


def test_observation_hash_and_assertion_references_are_enforced(tmp_path):
    writer = Path(__file__).with_name("real_e2e_evidence.py")
    observed = tmp_path / "events.ndjson"
    observed.write_text('{"type":"done"}\n')
    scenario = {
        "id": "audited-observation",
        "platforms": ["cli"],
        "layer": "system",
        "timeoutSeconds": 5,
        "command": (
            f"python3 -c \"import sys; sys.path.insert(0, '{writer.parent}'); "
            "from real_e2e_evidence import record_step; "
            f"record_step('turn', platforms=['cli'], modules=['agent'], boundaries=['cli'], assertions={{'answer': True}}, "
            f"assertion_evidence={{'answer':['events']}}, observations=[{{'id':'events','kind':'event_stream','path':'{observed}'}}])\""
        ),
        "evidence": {
            "platforms": ["cli"],
            "modules": ["agent"],
            "boundaries": ["cli"],
            "requiredObservationKinds": ["event_stream"],
            "requireAssertionEvidence": True,
        },
        "_contract": {"requiredAssertions": ["answer"]},
    }
    result = MODULE.run_scenario(scenario, tmp_path, {})
    assert result["passed"], result
    receipt = json.loads(Path(result["receipt"]).read_text(encoding="utf-8"))
    durable_observation = Path(
        receipt["steps"][0]["observations"][0]["path"]
    )
    assert durable_observation.is_file()
    observed.write_text('{"type":"tampered"}\n')
    _, errors = MODULE.validate_receipt(
        Path(result["receipt"]), scenario, result["evidence"]["executionId"]
    )
    assert errors == []
    durable_observation.write_text('{"type":"tampered"}\n')
    _, errors = MODULE.validate_receipt(
        Path(result["receipt"]), scenario, result["evidence"]["executionId"]
    )
    assert "observation changed after recording: events" in errors


def test_receipt_must_satisfy_named_contract_assertions(tmp_path):
    writer = Path(__file__).with_name("real_e2e_evidence.py")
    scenario = {
        "id": "missing-checkpoint",
        "platforms": ["cli"],
        "layer": "system",
        "timeoutSeconds": 5,
        "command": (
            f"python3 -c \"import sys; sys.path.insert(0, '{writer.parent}'); "
            "from real_e2e_evidence import record_step; "
            "record_step('turn', platforms=['cli'], modules=['chat'], boundaries=['cli'], "
            "real_provider_requests=0, assertions={'some_other_check': True})\""
        ),
        "evidence": {"platforms": ["cli"], "modules": ["chat"], "boundaries": ["cli"]},
        "_contract": {"requiredAssertions": ["semantic_result_validated"]},
    }
    result = MODULE.run_scenario(scenario, tmp_path, {})
    assert not result["passed"]
    assert "missing contract assertions: semantic_result_validated" in result["evidenceErrors"]


def test_semantic_audit_must_be_a_passing_real_model_verdict(tmp_path):
    writer = Path(__file__).with_name("real_e2e_evidence.py")
    audit = tmp_path / "semantic.json"
    audit.write_text(json.dumps({
        "kind": "real_model_response_judgement",
        "judge_model": "judge-model",
        "passed": False,
    }))
    scenario = {
        "id": "failed-semantic-verdict",
        "platforms": ["mac"],
        "layer": "system",
        "timeoutSeconds": 5,
        "command": (
            f"python3 -c \"import sys; sys.path.insert(0, '{writer.parent}'); "
            "from real_e2e_evidence import record_step; "
            "record_step('turn', platforms=['mac'], modules=['agent'], boundaries=['mac'], "
            "assertions={'answer': True}, "
            f"observations=[{{'id':'semantic','kind':'semantic_audit','path':'{audit}'}}])\""
        ),
        "evidence": {
            "platforms": ["mac"],
            "modules": ["agent"],
            "boundaries": ["mac"],
            "requiredObservationKinds": ["semantic_audit"],
        },
        "_contract": {"requiredAssertions": ["answer"]},
    }
    result = MODULE.run_scenario(scenario, tmp_path, {})
    assert not result["passed"]
    assert "semantic audit did not pass: semantic" in result["evidenceErrors"]


def test_multiplatform_receipt_requires_an_independent_step_per_platform(tmp_path):
    writer = Path(__file__).with_name("real_e2e_evidence.py")
    scenario = {
        "id": "combined-platform-claim",
        "platforms": ["ios", "android"],
        "layer": "system",
        "timeoutSeconds": 5,
        "command": (
            f"python3 -c \"import sys; sys.path.insert(0, '{writer.parent}'); "
            "from real_e2e_evidence import record_step; "
            "record_step('combined', platforms=['ios','android'], modules=['chat'], "
            "boundaries=['provider'], real_provider_requests=0, assertions={'passed': True})\""
        ),
        "evidence": {
            "platforms": ["ios", "android"],
            "modules": ["chat"],
            "boundaries": ["provider"],
        },
        "_contract": {"requiredAssertions": ["passed"], "perPlatformSteps": True},
    }
    result = MODULE.run_scenario(scenario, tmp_path, {})
    assert not result["passed"]
    assert "missing independent platform step: ios" in result["evidenceErrors"]
    assert "missing independent platform step: android" in result["evidenceErrors"]


def test_declared_modules_and_boundaries_must_form_one_observed_chain(tmp_path):
    writer = Path(__file__).with_name("real_e2e_evidence.py")
    scenario = {
        "id": "stitched-chain",
        "platforms": ["web"],
        "layer": "system",
        "timeoutSeconds": 5,
        "command": (
            f"python3 -c \"import sys; sys.path.insert(0, '{writer.parent}'); "
            "from real_e2e_evidence import record_step; "
            "record_step('provider', platforms=['web'], modules=['agent','provider'], boundaries=['web-ui'], assertions={'answer': True}); "
            "record_step('storage', platforms=['web'], modules=['skills','persistence'], boundaries=['persistence'], assertions={'saved': True})\""
        ),
        "evidence": {
            "platforms": ["web"],
            "modules": ["agent", "provider", "skills", "persistence"],
            "boundaries": ["web-ui", "persistence"],
        },
        "_contract": {"requiredAssertions": ["answer", "saved"], "perPlatformSteps": True},
    }
    result = MODULE.run_scenario(scenario, tmp_path, {})
    assert not result["passed"]
    assert any(
        "required product chain was not observed" in error
        for error in result["evidenceErrors"]
    )


def test_event_summary_reads_canonical_tool_and_text_parts(tmp_path):
    path = tmp_path / "events.ndjson"
    path.write_text("\n".join([
        json.dumps({"type": "agent_trace", "trace": {"kind": "tool.started", "sequence": 2, "payload": {"tool_name": "read_file"}}}),
        json.dumps({"type": "agent_trace", "trace": {"kind": "tool.completed", "sequence": 3, "payload": {"tool_name": "read_file"}}}),
        json.dumps({"type": "core_part_updated", "cursor": 4, "part": {"kind": "text", "text": "OK", "is_complete": True}}),
        json.dumps({"type": "agent_trace", "trace": {"kind": "run.completed", "sequence": 5, "payload": {}}}),
    ]))
    summary = EVENTS.summarize(path)
    assert summary["assistant"] == "OK"
    assert summary["toolsStarted"] == ["read_file"]
    assert summary["toolsCompleted"] == ["read_file"]
    assert summary["doneCount"] >= 1
def test_runner_uses_non_login_shell_to_preserve_audited_path():
    source = MODULE_PATH.read_text()
    assert '["bash", "-c", scenario["command"]]' in source
    assert '["bash", "-lc", scenario["command"]]' not in source
