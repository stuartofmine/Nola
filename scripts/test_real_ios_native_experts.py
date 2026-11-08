#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Actual native iOS expert UI acceptance with an isolated current Store."""
import json
import hashlib
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import time
import uuid

REPO = Path(__file__).resolve().parent.parent
scenario = os.environ.get("NOLA_NATIVE_EXPERT_SCENARIO", "template_capture")
if scenario not in {"template_capture", "public_reasoning_stop"}:
    raise SystemExit("Unknown native expert acceptance scenario")
run_id = str(uuid.uuid4())
runtime = Path("/tmp") / ("nola-room-real-ui-" + run_id)
runtime.mkdir(mode=0o700)
home = runtime / "home"
(home / ".nola").mkdir(parents=True)
project = runtime / "project"
project.mkdir()
database = runtime / "nola.db"
derived = Path(os.environ.get("NOLA_IOS_REAL_DERIVED_DATA", "/tmp/nola-native-expert-derived-" + run_id))
binary = Path(os.environ.get("NOLA_REAL_CLI_BINARY", str(REPO / "target/debug/nola")))
config = json.loads(Path(os.environ.get("NOLA_CONFIG_PATH", str(Path.home() / ".nola/config.json"))).read_text())["llm"]
selection = config["default_model"]
connections = [c for c in config["connections"] if c["id"] == selection["connection"]]
if len(connections) != 1:
    raise SystemExit("The selected canonical connection must be unique")
connection = connections[0]
models = [m for m in connection["models"] if m["id"] == selection["model"]]
if len(models) != 1 or connection["protocol"] != "chat_completions":
    raise SystemExit("This native acceptance requires the explicitly selected Chat Completions model")
secret = os.environ.get(connection["api_key_env"], "")
if not secret:
    raise SystemExit("The selected provider credential is unavailable")

devices = json.loads(subprocess.check_output(["xcrun", "simctl", "list", "devices", "available", "--json"]))
(runtime / "available-simulators.json").write_text(json.dumps(devices))
device = os.environ.get("NOLA_IOS_REAL_DEVICE_ID")
owned_device = False
if device:
    if not any(d["udid"] == device for group in devices["devices"].values() for d in group):
        raise SystemExit("The requested available simulator does not exist")
else:
    runtimes = json.loads(subprocess.check_output(["xcrun", "simctl", "list", "runtimes", "--json"]))["runtimes"]
    ios = next(r for r in runtimes if r.get("isAvailable") and ".iOS-" in r["identifier"])
    kinds = json.loads(subprocess.check_output(["xcrun", "simctl", "list", "devicetypes", "--json"]))["devicetypes"]
    phone = next(d for d in kinds if d["name"].startswith("iPhone"))
    device = subprocess.check_output(["xcrun", "simctl", "create", "Nola Native Experts " + run_id,
        phone["identifier"], ios["identifier"]], text=True).strip()
    owned_device = True

probe_log = open(runtime / "provider-probe.log", "w")
probe_command = [sys.executable, str(REPO / "scripts/e2e/provider_probe.py"),
    "--upstream", connection["endpoint"], "--log", str(runtime / "provider.ndjson"),
    "--ready", str(runtime / "provider-ready.json")]
if scenario == "template_capture":
    probe_command += ["--expected-request-instructions", str(runtime / "native-capture-fixture.json")]
probe = subprocess.Popen(probe_command, stdout=probe_log, stderr=subprocess.STDOUT)
print("Native expert isolated evidence:", runtime, flush=True)
try:
    for _ in range(100):
        if (runtime / "provider-ready.json").exists():
            break
        if probe.poll() is not None:
            raise RuntimeError("The actual provider relay exited")
        time.sleep(0.05)
    endpoint = json.loads((runtime / "provider-ready.json").read_text())["endpoint"]
    connection["endpoint"] = endpoint
    current_config = home / ".nola/config.json"
    current_config.write_text(json.dumps({"llm": {"connections": [connection], "default_model": selection}}))
    current_config.chmod(0o600)
    preparation_environment = dict(os.environ, HOME=str(home), NOLA_DB_PATH=str(database))
    with open(runtime / "model-import.log", "w") as log:
        subprocess.run([str(binary), "model", "apply-config", str(current_config)], cwd=project,
            env=preparation_environment, stdout=log, stderr=subprocess.STDOUT, check=True)
    env = dict(os.environ, CARGO_BUILD_JOBS="2", INCLUDE_UI_TESTS="1", PARALLEL_TEST_WORKERS="1",
        SIMULATOR_SIGNING="adhoc", DESTINATION="platform=iOS Simulator,id=" + device,
        DERIVED_DATA=str(derived), RESULT_BUNDLE=str(runtime / "native-experts.xcresult"),
        SKIP_BUILD="1" if os.environ.get("NOLA_SKIP_IOS_BUILD") == "1" else "0",
        XCODE_OPTS=os.environ.get("XCODE_OPTS", "-configuration Test -jobs 2 -collect-test-diagnostics never"))
    for key, value in {"ROOT": str(runtime), "DATABASE": str(database), "ENDPOINT": endpoint,
        "MODEL": selection["model"], "KEY": secret, "NAMESPACE": "native-experts-" + run_id}.items():
        env["TEST_RUNNER_NOLA_NATIVE_EXPERT_" + key] = value
    with open(runtime / "xctest.log", "w") as log:
        test_case = "testNativeExpertPublicReasoningGrowsAndStops" if scenario == "public_reasoning_stop" \
            else "testNativeExpertsStreamRecoverAndKeepConversationDrafts"
        tested = subprocess.run([str(REPO / "scripts/run_client_tests.sh"),
            "-only-testing:NolaUITests/IOSRealExpertAcceptanceTests/" + test_case],
            cwd=REPO, env=env, stdout=log, stderr=subprocess.STDOUT)
    print("Native expert XCTest exit:", tested.returncode, flush=True)
    if tested.returncode:
        raise SystemExit(tested.returncode)
    if scenario == "public_reasoning_stop":
        result = json.loads((runtime / "native-public-reasoning-result.json").read_text())
        source = sqlite3.connect("file:" + str(database) + "?mode=ro", uri=True)
        source.row_factory = sqlite3.Row
        runs = [dict(row) for row in source.execute("SELECT id,conversation_id,assistant_message_id,status FROM runs")]
        checks = {"ui_passed": result.get("passed") is True, "one_original_native_run": len(runs) == 1,
            "no_remote_mirror": source.execute("SELECT count(*) FROM remote_run_sources").fetchone()[0] == 0}
        progress = []
        if len(runs) == 1:
            run = runs[0]
            seal_row = source.execute("SELECT seal_json FROM assistant_run_terminal_seals WHERE run_id=?", (run["id"],)).fetchone()
            seal = json.loads(seal_row[0]) if seal_row else None
            checks["exact_cancelled_seal"] = run["status"] == "sealed" and bool(seal) \
                and seal["identity"]["conversation_id"] == run["conversation_id"] \
                and seal["identity"]["assistant_message_id"] == run["assistant_message_id"] \
                and seal["result"]["value"]["kind"] == "cancelled"
            checks["exact_visible_assistant_owner"] = result["assistantRowID"] == "room.expert.reply." + run["assistant_message_id"]
            for row in source.execute("SELECT sequence,head_json FROM assistant_journal_part_versions ORDER BY sequence"):
                head = json.loads(row["head_json"])
                if head["identity"]["run_id"] == run["id"] and head["metadata"]["kind"] == "reasoning" \
                        and head["metadata"]["data"]["state"] == "running":
                    for body in head["bodies"]:
                        if body["field"] == "reasoning_summary" and int(body["utf8_bytes"]) > 0:
                            progress.append({"sequence": row["sequence"], "bytes": int(body["utf8_bytes"])})
            checks["core_public_reasoning_grew_before_terminal"] = len({item["bytes"] for item in progress}) >= 2
            configs = [json.loads(row[0]) for row in source.execute("SELECT config_json FROM run_invocation_payloads")]
            checks["actual_original_model_binding"] = len(configs) == 1 and all(
                item["turn_identity"]["host_invocation"]["version"] == 8
                and item["turn_identity"]["host_invocation"]["primary"]["kind"] == "http"
                and item["turn_identity"]["host_invocation"]["primary"]["protocol"] == "chat_completions"
                and item["turn_identity"]["host_invocation"]["primary"]["model"] == selection["model"]
                and item["turn_identity"]["host_invocation"]["primary"]["endpoint"] == endpoint
                and item["turn_identity"]["host_invocation"]["primary"]["credential_sha256"] == hashlib.sha256(secret.encode()).hexdigest()
                for item in configs)
        samples = result["samples"]
        checks["visible_public_reasoning_grew_before_terminal"] = len(samples) >= 2 \
            and all(sample["state"] == "running" for sample in samples) \
            and max(sample["characters"] for sample in samples) >= samples[0]["characters"] + 32
        responses = [json.loads(line) for line in (runtime / "provider.ndjson").read_text().splitlines() if line.strip()]
        checks["actual_provider_public_reasoning"] = bool(responses) and any(
            event.get("status") == 200 and event.get("responseReasoningDeltaCount", 0) > 1 for event in responses)
        audit = {"passed": all(checks.values()), "scenario": scenario, "assertions": checks,
            "runs": runs, "core_reasoning_progress": progress, "ui_samples": samples,
            "does_not_replace_original_double_expert_long_task": True}
        (runtime / "native-public-reasoning-audit.json").write_text(json.dumps(audit, indent=2))
        print("Native public reasoning / exact cancellation audit:", audit["passed"], flush=True)
        raise SystemExit(0 if audit["passed"] else 1)
    result = json.loads((runtime / "native-ui-result.json").read_text())
    source = sqlite3.connect("file:" + str(database) + "?mode=ro", uri=True)
    source.row_factory = sqlite3.Row
    runs = [dict(row) for row in source.execute("SELECT id,conversation_id,assistant_message_id,status FROM runs")]
    assertions = {"ui_passed": result.get("passed") is True, "two_actual_child_runs": len(runs) == 2,
        "no_remote_mirror": source.execute("SELECT count(*) FROM remote_run_sources").fetchone()[0] == 0}
    seals = {row["run_id"]: json.loads(row["seal_json"]) for row in source.execute("SELECT run_id,seal_json FROM assistant_run_terminal_seals")}
    assertions["two_exact_answered_seals"] = len(runs) == 2 and all(r["status"] == "sealed"
        and r["id"] in seals and seals[r["id"]]["identity"]["conversation_id"] == r["conversation_id"]
        and seals[r["id"]]["identity"]["assistant_message_id"] == r["assistant_message_id"]
        and seals[r["id"]]["result"]["value"]["kind"] == "answered" for r in runs)
    configurations = [json.loads(row[0]) for row in source.execute("SELECT config_json FROM run_invocation_payloads")]
    # Preserve full private witnesses for local auditing; only assertion booleans
    # are printed. No provider credential or endpoint is written to stdout.
    (runtime / "native-invocation-witnesses.json").write_text(json.dumps(configurations))
    assertions["actual_native_cloud_factories"] = len(configurations) == 2 and all(
        c["turn_identity"]["host_invocation"]["version"] == 8
        and c["turn_identity"]["host_invocation"]["primary"]["kind"] == "http"
        and c["turn_identity"]["host_invocation"]["primary"]["protocol"] == "chat_completions"
        and c["turn_identity"]["host_invocation"]["primary"]["model"] == selection["model"]
        and c["turn_identity"]["host_invocation"]["primary"]["endpoint"] == endpoint
        and c["turn_identity"]["host_invocation"]["primary"]["credential_sha256"] == hashlib.sha256(secret.encode()).hexdigest()
        for c in configurations)
    manifests = [json.loads(row[0]) for row in source.execute(
        "SELECT value_json FROM product_settings WHERE key LIKE 'edited_resend/expert/source/%'")]
    assertions["complete_original_manifest"] = len(manifests) == 1 and len(manifests[0]["entries"]) == 2 \
        and {e["child_submission_id"] for e in manifests[0]["entries"]} == {r["id"] for r in runs} \
        and {e["participant"]["agent_id"] for e in manifests[0]["entries"]} == set(result["expertIDs"]) \
        and all(e["model_witness"] is not None for e in manifests[0]["entries"])
    fixture = json.loads((runtime / "native-capture-fixture.json").read_text())
    template_snapshot = json.loads(source.execute("SELECT value_json FROM product_settings WHERE key='prompt.templates'").fetchone()[0])
    templates = [template for template in template_snapshot["customTemplates"] if template["name"] == fixture["templateName"]]
    assertions["ui_created_owned_custom_template"] = len(templates) == 1 and templates[0]["content"] == fixture["templateContent"] \
        and templates[0].get("accessRevision") == 1 and bool(templates[0].get("accessOwnerID"))
    captures = [json.loads(row[0]) for row in source.execute(
        "SELECT value_json FROM product_settings WHERE key LIKE 'edited_resend/expert/template_capture/%'")]
    assertions["new_complete_private_capture"] = len(captures) == 1 and len(captures[0]["entries"]) == 2 \
        and {entry["child_submission_id"] for entry in captures[0]["entries"]} == {run["id"] for run in runs}
    host_instructions = {"ios:user-response-style": fixture["styleInstruction"], "ios:user-custom-instructions": fixture["customInstruction"]}
    assertions["actual_begin_exact_template_and_user_guidance"] = assertions["new_complete_private_capture"] and assertions["complete_original_manifest"]
    if assertions["actual_begin_exact_template_and_user_guidance"]:
        for entry in manifests[0]["entries"]:
            managed = entry["expected_prompt_definition"]
            definitions = entry["instruction_witness"]["prompt_definitions"] if entry.get("instruction_witness") else []
            expected = dict(host_instructions)
            if managed:
                expected[managed["component_id"]] = fixture["templateContent"]
            assertions["actual_begin_exact_template_and_user_guidance"] &= bool(managed) and managed["content"] == fixture["templateContent"] \
                and len(definitions) == 3 and {item["component_id"]: item["content"] for item in definitions} == expected \
                and all(item["content_sha256"] == hashlib.sha256(item["content"].encode()).hexdigest() for item in definitions)
        for entry in captures[0]["entries"]:
            instructions = entry["host_user_instructions"]
            assertions["actual_begin_exact_template_and_user_guidance"] &= entry["origin"]["kind"] == "custom" \
                and entry["origin"]["resource"]["owner"]["id"] == captures[0]["actor"]["id"] \
                and len(instructions) == 2 and {item["component_id"]: item["content"] for item in instructions} == host_instructions
    provider_events = [json.loads(line) for line in (runtime / "provider.ndjson").read_text().splitlines() if line.strip()]
    actual_inferences = [event for event in provider_events if event.get("event") == "provider.response" and event.get("requestInputPresent")]
    assertions["actual_provider_stream_bytes"] = len(actual_inferences) >= 2 and all(event.get("status") == 200 for event in actual_inferences) \
        and any(event.get("responseTextDeltaCount", 0) > 1 for event in actual_inferences)
    assertions["all_frozen_instructions_reached_actual_provider"] = bool(actual_inferences) and all(
        event.get("requestInstructionObservation") == "observed"
        and set(event.get("requestInstructionMatches", {})) == {"templateContent", "styleInstruction", "customInstruction"}
        and all(item.get("present") is True for item in event["requestInstructionMatches"].values())
        for event in actual_inferences)
    for run in runs:
        read = subprocess.run([str(binary), "session", "show", run["conversation_id"], "--json"],
            cwd=project, env=preparation_environment, capture_output=True, text=True, check=True)
        projection = json.loads(read.stdout)["conversation_projection"]
        (runtime / ("child-" + run["id"] + ".json")).write_text(read.stdout)
        assertions["complete_content_" + run["id"]] = any(m["role"] == "assistant" and m["state"] == "completed"
            and any(p["kind"] == "text" and result["end"] in p["markdown"] for p in m["parts"]) for m in projection["messages"])
        rendered = json.loads((runtime / "native-ui-complete-texts.json").read_text())
        canonical = [message for message in projection["messages"] if message["message_id"] == run["assistant_message_id"]]
        expected_text = " ".join(part["markdown"] for message in canonical for part in message["parts"] if part["kind"] == "text")
        ui_text = rendered.get("room.expert.reply." + run["assistant_message_id"], "")
        assertions["entire_rendered_answer_" + run["id"]] = len(canonical) == 1 and bool(ui_text) \
            and " ".join(ui_text.split()) == " ".join(expected_text.split())
    audit = {"passed": all(assertions.values()), "assertions": assertions, "runs": runs}
    (runtime / "native-core-audit.json").write_text(json.dumps(audit, indent=2))
    print("Native canonical audit:", audit["passed"], flush=True)
    if not audit["passed"]:
        raise SystemExit(1)
finally:
    probe.terminate()
    try:
        probe.wait(timeout=5)
    except subprocess.TimeoutExpired:
        probe.kill(); probe.wait()
    probe_log.close()
    if owned_device:
        subprocess.run(["xcrun", "simctl", "shutdown", device], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        subprocess.run(["xcrun", "simctl", "delete", device], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
