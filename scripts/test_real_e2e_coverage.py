from real_e2e_coverage import (
    audit_coverage,
    observed_scenario_lanes,
    scenario_lanes,
    validate_execution_claim,
)


def _scenario(identifier: str, platform: str, **overrides):
    execution = {
        "runtime": "native-app",
        "productionCode": True,
        "productEntrypoint": True,
        "realProvider": True,
        "realPersistence": True,
        "realUI": True,
        "semanticJudge": True,
        "isolation": "temporary-profile",
    }
    execution.update(overrides.pop("execution", {}))
    value = {
        "id": identifier,
        "platforms": [platform],
        "layer": "product-ui",
        "realModel": True,
        "execution": execution,
        "_contract": {"requiredObservationKinds": [
            "ui_screenshot", "accessibility_tree", "ui_timeline"
        ]},
        "evidence": {},
    }
    value.update(overrides)
    return value


def test_execution_claim_requires_explicit_fidelity_dimensions():
    errors = validate_execution_claim({"id": "opaque"})
    assert errors == ["scenario opaque lacks execution fidelity metadata"]


def test_real_ui_claim_requires_product_layer_and_raw_ui_observations():
    scenario = _scenario("bad-ui", "mac", layer="system", _contract={})
    errors = validate_execution_claim(scenario)
    assert any("outside product-ui" in error for error in errors)
    assert any("lacks observations" in error for error in errors)


def test_terminal_lane_requires_a_real_pty_ui_entrypoint():
    scenario = _scenario(
        "cli-terminal", "cli",
        execution={"runtime": "terminal-pty"},
    )
    assert "terminal-ui" in scenario_lanes(scenario)


def test_smoke_coverage_exposes_missing_platform_lanes():
    web = _scenario("web-ui", "web", execution={"runtime": "browser"})
    web["evidence"]["modules"] = [
        "agent", "core", "provider", "tools", "skills", "persistence", "ui",
    ]
    coverage = audit_coverage([web], suite="smoke", passed_scenario_ids={"web-ui"})
    assert coverage["complete"] is False
    assert coverage["platforms"]["web"]["complete"] is True
    assert "terminal-ui" in coverage["platforms"]["cli"]["missing"]
    assert "product-ui" in coverage["platforms"]["android"]["missing"]


def test_failed_scenario_contributes_no_verified_lane():
    web = _scenario("web-ui", "web", execution={"runtime": "browser"})
    coverage = audit_coverage([web], suite="smoke", passed_scenario_ids=set())
    assert coverage["platforms"]["web"]["verified"] == []


def test_real_backend_http_service_satisfies_full_backend_lanes():
    backend = _scenario(
        "backend-http", "backend", layer="system", realModel=False,
        execution={
            "runtime": "http-service", "realProvider": False,
            "realUI": False, "semanticJudge": False,
        },
        _contract={"requiredObservationKinds": ["http_transcript", "persistence_store"]},
    )
    backend["evidence"]["modules"] = ["auth", "chat", "database", "http"]
    coverage = audit_coverage([backend], suite="full", passed_scenario_ids={"backend-http"})
    assert coverage["platforms"]["backend"]["complete"] is True


def test_smoke_requires_backend_and_platform_modules():
    scenarios = [
        _scenario("web-ui", "web", execution={"runtime": "browser"}),
    ]
    scenarios[0]["evidence"]["modules"] = ["agent", "core", "provider", "ui"]
    coverage = audit_coverage(scenarios, suite="smoke", passed_scenario_ids={"web-ui"})
    assert coverage["platforms"]["backend"]["complete"] is False
    assert "skills" in coverage["platforms"]["web"]["missingModules"]
    assert "tools" in coverage["platforms"]["web"]["missingModules"]


def test_runtime_platform_mismatch_and_legacy_chain_platforms_are_rejected():
    scenario = _scenario(
        "bad-runtime",
        "cli",
        execution={"runtime": "simulator-and-emulator", "semanticJudge": False},
        evidence={
            "requiredObservationKinds": [
                "ui_screenshot", "accessibility_tree", "ui_timeline",
                "persistence_store", "persistence_audit",
            ],
            "requiredChains": [{"platforms": ["cli"]}],
        },
    )
    errors = validate_execution_claim(scenario)
    assert any("incompatible with platforms" in error for error in errors)
    assert any("invalid platforms field" in error for error in errors)


def test_semantic_lane_requires_a_real_semantic_audit_observation():
    scenario = _scenario("mac-semantic", "mac")
    receipt = {
        "platforms": ["mac"],
        "modules": ["agent"],
        "realProviderRequests": 1,
        "steps": [{"observations": [
            {"kind": "persistence_store"},
            {"kind": "persistence_audit"},
            {"kind": "ui_screenshot"},
            {"kind": "accessibility_tree"},
            {"kind": "ui_timeline"},
        ]}],
    }
    assert "semantic-judge" not in observed_scenario_lanes(scenario, receipt)
    receipt["steps"][0]["observations"].append({"kind": "semantic_audit"})
    assert "semantic-judge" in observed_scenario_lanes(scenario, receipt)


def test_executed_coverage_uses_receipt_modules_not_matrix_modules():
    web = _scenario("web-ui", "web", execution={"runtime": "browser", "semanticJudge": False})
    web["evidence"]["modules"] = [
        "agent", "core", "provider", "tools", "skills", "persistence", "ui",
    ]
    receipt = {
        "platforms": ["web"],
        "modules": ["agent", "provider", "ui"],
        "realProviderRequests": 1,
        "steps": [{"observations": [
            {"kind": "persistence_store"}, {"kind": "persistence_audit"},
            {"kind": "ui_screenshot"}, {"kind": "accessibility_tree"},
            {"kind": "ui_timeline"},
        ]}],
    }
    coverage = audit_coverage(
        [web], suite="smoke", passed_scenario_ids={"web-ui"},
        evidence_by_scenario={"web-ui": receipt},
    )
    assert coverage["evidenceMode"] == "executed-receipts"
    assert "skills" in coverage["platforms"]["web"]["missingModules"]


def test_screenshot_capture_without_actual_visual_review_proves_no_ui_lane():
    scenario = _scenario("mac-ui", "mac", execution={"semanticJudge": False})
    observations = [
        {"kind": "persistence_store"},
        {"kind": "ui_screenshot"},
        {"kind": "accessibility_tree"},
        {"kind": "ui_timeline"},
    ]
    receipt = {
        "platforms": ["mac"],
        "modules": ["ui"],
        "realProviderRequests": 1,
        "steps": [{"observations": observations}],
    }
    assert "product-ui" not in observed_scenario_lanes(scenario, receipt)

    observations[1]["visualReview"] = {
        "observer": "visual-agent",
        "observedAt": "2026-08-23T12:00:00Z",
        "method": "opened_image",
        "verdict": "passed",
        "findings": ["Messages remain ordered and the composer does not cover the answer."],
    }
    assert "product-ui" in observed_scenario_lanes(scenario, receipt)
