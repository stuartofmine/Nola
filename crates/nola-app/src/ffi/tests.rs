// SPDX-License-Identifier: Apache-2.0

use super::*;
use std::ffi::CString;

#[test]
fn ffi_open_execute_redact_and_mobile_roundtrip() {
    assert_eq!(nola_core_application_abi_version(), 1);
    assert_eq!(nola_core_application_service_protocol_version(), 1);
    assert_eq!(nola_core_context_engine_abi_version(), 1);
    assert_eq!(nola_core_prompt_composer_abi_version(), 1);
    let sample = CString::new("你好hello").unwrap();
    assert_eq!(unsafe { nola_v2_estimate_tokens(sample.as_ptr()) }, 4);

    // Open error reporting (borrowed const char* - MUST NOT be freed!)
    let bad = unsafe { nola_core_application_open(std::ptr::null(), 0) };
    assert!(bad.is_null());
    let err_ptr = nola_core_application_last_open_error();
    assert!(!err_ptr.is_null());
    let err_str = unsafe { CStr::from_ptr(err_ptr) }.to_str().unwrap().to_string();
    assert!(err_str.contains("empty database path"));

    let dir = tempfile::tempdir().unwrap();
    let db_str = dir.path().join("ffi.db").display().to_string();
    let app = unsafe { nola_core_application_open(db_str.as_ptr(), db_str.len()) };
    assert!(!app.is_null());

    // 1. Mobile submit envelope matches strict Android/iOS receipt validation
    let submit_req = json!({
        "protocol_version": 1,
        "client_id": "test-mobile",
        "request_id": "req-1",
        "command": {"type": "session_create", "arguments": {"title": "Mobile Chat"}}
    })
    .to_string();
    let submit_ptr =
        unsafe { nola_core_application_submit(app, submit_req.as_ptr(), submit_req.len()) };
    let submit_val: Value = serde_json::from_str(
        unsafe { CStr::from_ptr(submit_ptr) }.to_str().unwrap(),
    )
    .unwrap();
    unsafe { nola_core_free_c_string(submit_ptr) };
    assert_eq!(submit_val["ok"], true);
    let receipt = &submit_val["result"];
    assert_eq!(receipt["status"], "committed");
    assert!(receipt["received_at"].as_str().is_some());
    assert!(receipt["completed_at"].as_str().is_some());
    assert!(receipt["committed_revision"].as_u64().unwrap_or(0) >= 1);

    // 2. Resource access MUST reject cross-resource grants
    let ra_req = json!({
        "resource": {
            "kind": "skill",
            "resource_id": "secret-skill",
            "owner": {"kind": "local_user", "id": "alice"},
            "revision": 2
        },
        "actor": {"kind": "local_user", "id": "bob"},
        "permission": "read_content",
        "now_epoch_ms": 1000,
        "grants": [{
            "grant_id": "g1",
            "resource_kind": "skill",
            "resource_id": "other-skill",
            "principal": {"kind": "local_user", "id": "bob"},
            "permissions": ["read_content"]
        }]
    })
    .to_string();
    let ra_ptr = unsafe { nola_core_resource_access_evaluate_json(ra_req.as_ptr(), ra_req.len()) };
    let ra_val: Value =
        serde_json::from_str(unsafe { CStr::from_ptr(ra_ptr) }.to_str().unwrap()).unwrap();
    unsafe { nola_core_free_c_string(ra_ptr) };
    assert_eq!(ra_val["decision"]["allowed"], false);

    // 3. Tool policy matches tool_policy_v1_golden.json contract + MCP fail-closed
    let tp_req = json!({
        "schema_version": 1,
        "platform": "ios",
        "now_ms": 1000000,
        "runtime": {
            "expires_at_ms": 1060000,
            "authorizations": {"files_read": "granted", "files_write": "granted"},
            "tools": [
                {"tool_id": "device_read_file", "capability": "files_read", "risk": "read_only", "route": "device", "available": true, "schema": {}},
                {"tool_id": "device_write_file", "capability": "files_write", "risk": "local_write", "route": "device", "available": true, "schema": {}}
            ]
        },
        "user_policy": {"max_risk": "read_only", "allow_result_egress_routes": ["device"]}
    })
    .to_string();
    let tp_ptr = unsafe { nola_core_tool_policy_resolve(tp_req.as_ptr(), tp_req.len()) };
    let tp_val: Value =
        serde_json::from_str(unsafe { CStr::from_ptr(tp_ptr) }.to_str().unwrap()).unwrap();
    unsafe { nola_core_free_c_string(tp_ptr) };
    assert_eq!(tp_val["platform"], "ios");
    assert_eq!(tp_val["enabled_tools"].as_array().unwrap().len(), 1);
    assert_eq!(tp_val["enabled_tools"][0]["tool_id"], "device_read_file");
    assert_eq!(tp_val["disabled_tools"][0]["tool_id"], "device_write_file");
    assert_eq!(tp_val["disabled_tools"][0]["reason"], "risk_exceeds_user_policy");
    assert_eq!(tp_val["policy_digest"].as_str().unwrap().len(), 64);
    let mcp_empty = b"{}";
    assert!(!unsafe { nola_core_tool_policy_mcp_allowed(mcp_empty.as_ptr(), mcp_empty.len()) });

    // 4. Memory select emits exact iOS enum reject reasons
    let ms_req = json!({
        "mode": "on_demand",
        "policy": {"long_term_enabled": true, "minimum_confidence": 0.8, "search_limit": 1},
        "records": [
            {"id": "low", "content": "a", "confidence": 0.2},
            {"id": "ok1", "content": "b", "confidence": 0.9},
            {"id": "over", "content": "c", "confidence": 0.95}
        ]
    })
    .to_string();
    let ms_ptr = unsafe { nola_core_memory_select_json(ms_req.as_ptr(), ms_req.len()) };
    let ms_val: Value =
        serde_json::from_str(unsafe { CStr::from_ptr(ms_ptr) }.to_str().unwrap()).unwrap();
    unsafe { nola_core_free_c_string(ms_ptr) };
    assert_eq!(ms_val["rejected"][0]["reason"], "below_confidence");
    assert_eq!(ms_val["rejected"][1]["reason"], "limit_exceeded");

    // 5. Model output parser extracts <think> & <tool_call> and filters disallowed tools
    let mo_req = json!({
        "content": "<think>step 1</think>Answer<tool_call>{\"name\":\"read_file\",\"arguments\":{\"path\":\"a\"}}</tool_call>",
        "reasoning": "",
        "native_tool_calls": [],
        "allowed_tool_names": ["read_file"]
    })
    .to_string();
    let mo_ptr = unsafe { nola_core_parse_agent_model_output(mo_req.as_ptr(), mo_req.len()) };
    let mo_val: Value =
        serde_json::from_str(unsafe { CStr::from_ptr(mo_ptr) }.to_str().unwrap()).unwrap();
    unsafe { nola_core_free_c_string(mo_ptr) };
    assert_eq!(mo_val["reasoning"], "step 1");
    assert_eq!(mo_val["content"], "Answer");
    assert_eq!(mo_val["tool_calls"].as_array().unwrap().len(), 1);
    assert_eq!(mo_val["finish_reason"], "tool_calls");

    // 6. Context prepare compacts when exceeding prompt_token_limit
    let big = "word ".repeat(300);
    let cp_req = json!({
        "messages": [
            {"role": "system", "content": "sys"},
            {"role": "user", "content": big},
            {"role": "assistant", "content": big},
            {"role": "user", "content": "latest"}
        ],
        "tools": [],
        "budget": {"prompt_token_limit": 80, "keep_recent_messages": 2}
    })
    .to_string();
    let cp_ptr = unsafe { nola_core_context_prepare(cp_req.as_ptr(), cp_req.len()) };
    let cp_val: Value =
        serde_json::from_str(unsafe { CStr::from_ptr(cp_ptr) }.to_str().unwrap()).unwrap();
    unsafe { nola_core_free_c_string(cp_ptr) };
    assert_eq!(cp_val["changed"], true);
    assert_eq!(cp_val["action"], "compacted");
    assert!(cp_val["metrics"]["archive_message_count"].as_u64().unwrap() >= 1);

    // 7. Personal intelligence ingest/recall/snapshot contract
    let ing_req = json!({
        "events": [{"id": "e1", "source": "chat", "kind": "note", "summary": "User prefers Rust"}],
        "memory_candidates": [{"semantic_key": "lang", "kind": "semantic", "statement": "Prefers Rust v2"}],
        "outcomes": [{"id": "o1", "task_type": "code", "route": "local", "model": "m1", "success": true}]
    })
    .to_string();
    let ing_ptr = unsafe {
        nola_core_application_intelligence_ingest(app, ing_req.as_ptr(), ing_req.len())
    };
    let ing_val: Value =
        serde_json::from_str(unsafe { CStr::from_ptr(ing_ptr) }.to_str().unwrap()).unwrap();
    unsafe { nola_core_free_c_string(ing_ptr) };
    assert_eq!(ing_val["ingested_memories"], 1);
    let rec_req = json!({"query": "Rust", "limit": 5}).to_string();
    let rec_ptr = unsafe {
        nola_core_application_intelligence_recall(app, rec_req.as_ptr(), rec_req.len())
    };
    let rec_val: Value =
        serde_json::from_str(unsafe { CStr::from_ptr(rec_ptr) }.to_str().unwrap()).unwrap();
    unsafe { nola_core_free_c_string(rec_ptr) };
    assert!(rec_val["context"].as_str().unwrap().contains("Rust"));
    let snap_ptr = unsafe { nola_core_application_intelligence_snapshot(app) };
    let snap_val: Value =
        serde_json::from_str(unsafe { CStr::from_ptr(snap_ptr) }.to_str().unwrap()).unwrap();
    unsafe { nola_core_free_c_string(snap_ptr) };
    assert_eq!(snap_val["schema_version"], 1);
    assert_eq!(snap_val["events"].as_array().unwrap().len(), 1);
    assert_eq!(snap_val["memories"].as_array().unwrap().len(), 1);
    assert_eq!(snap_val["outcomes"].as_array().unwrap().len(), 1);

    // 8. Software update contract matches AppleSoftwareUpdateClient (truthful unconfigured!)
    let su_req = json!({"operation": "check", "target": "ios"}).to_string();
    let su_ptr = unsafe { nola_core_software_update_json(su_req.as_ptr(), su_req.len()) };
    let su_val: Value =
        serde_json::from_str(unsafe { CStr::from_ptr(su_ptr) }.to_str().unwrap()).unwrap();
    unsafe { nola_core_free_c_string(su_ptr) };
    assert_eq!(su_val["ok"], true);
    assert_eq!(su_val["value"]["status"], "unconfigured");
    assert_eq!(su_val["value"]["target"], "ios");
    assert!(nola_core_application_last_open_error().is_null());

    // 9. Nested macOS MCP policy check, mobile MCP cancel with null app, attest result envelope,
    //    child pump result envelope, stateless intelligence recall, and GitHub skill package
    let mac_mcp = json!({
        "policy": {"enabled": true, "denied": false, "authorization": "granted"},
        "descriptor": {"server_id": "docs", "tool_name": "search_docs"}
    })
    .to_string();
    assert!(unsafe { nola_core_tool_policy_mcp_allowed(mac_mcp.as_ptr(), mac_mcp.len()) });

    let cancel_req = json!({"type": "cancel"}).to_string();
    let cancel_ptr = unsafe {
        nola_core_mobile_mcp_host(std::ptr::null_mut(), cancel_req.as_ptr(), cancel_req.len())
    };
    let cancel_val: Value =
        serde_json::from_str(unsafe { CStr::from_ptr(cancel_ptr) }.to_str().unwrap()).unwrap();
    unsafe { nola_core_free_c_string(cancel_ptr) };
    assert_eq!(cancel_val["ok"], true);
    assert_eq!(cancel_val["result"]["cancelled"], true);

    let created_conv_id = submit_val["result"]["result"]["session"]["id"]
        .as_str()
        .or_else(|| submit_val["result"]["result"]["conversation"]["id"].as_str())
        .unwrap()
        .to_string();
    let target_bytes = created_conv_id.as_bytes();
    let cfg_str = json!({
        "core_config": {"conversation_id": created_conv_id, "model": "gpt-4o"},
        "submission": {"conversation_id": created_conv_id, "type": "user_input", "text": "retry"}
    })
    .to_string();
    let cfg_bytes = cfg_str.as_bytes();
    let att_ptr = unsafe {
        nola_core_application_attest_resend_host_policy(
            app,
            target_bytes.as_ptr(),
            target_bytes.len(),
            cfg_bytes.as_ptr(),
            cfg_bytes.len(),
        )
    };
    let att_val: Value =
        serde_json::from_str(unsafe { CStr::from_ptr(att_ptr) }.to_str().unwrap()).unwrap();
    unsafe { nola_core_free_c_string(att_ptr) };
    assert_eq!(att_val["ok"], true);
    assert_eq!(att_val["result"]["attested"], true);
    assert_eq!(att_val["result"]["policy_digest"].as_str().unwrap().len(), 64);

    // Missing conversation MUST be rejected by attest_resend_host_policy
    let bad_target = b"missing-conv-999";
    let bad_att_ptr = unsafe {
        nola_core_application_attest_resend_host_policy(
            app,
            bad_target.as_ptr(),
            bad_target.len(),
            cfg_bytes.as_ptr(),
            cfg_bytes.len(),
        )
    };
    let bad_att_val: Value =
        serde_json::from_str(unsafe { CStr::from_ptr(bad_att_ptr) }.to_str().unwrap()).unwrap();
    unsafe { nola_core_free_c_string(bad_att_ptr) };
    assert_eq!(bad_att_val["ok"], false);
    assert_eq!(bad_att_val["error"]["code"], "resend_host_policy_rejected");

    let pump = unsafe {
        nola_core_child_pump_start(app, std::ptr::null(), 0, std::ptr::null_mut())
    };
    assert!(!pump.is_null());
    let poll_ptr = unsafe { nola_core_child_pump_poll(pump, 10) };
    let poll_val: Value =
        serde_json::from_str(unsafe { CStr::from_ptr(poll_ptr) }.to_str().unwrap()).unwrap();
    unsafe { nola_core_free_c_string(poll_ptr) };
    assert_eq!(poll_val["ok"], true);
    assert_eq!(poll_val["result"]["kind"], "idle");
    let resp_ptr = unsafe { nola_core_child_pump_respond(pump, b"{}".as_ptr(), 2) };
    let resp_val: Value =
        serde_json::from_str(unsafe { CStr::from_ptr(resp_ptr) }.to_str().unwrap()).unwrap();
    unsafe { nola_core_free_c_string(resp_ptr) };
    assert_eq!(resp_val["ok"], true);
    assert_eq!(resp_val["result"]["committed"], true);
    unsafe {
        nola_core_child_pump_close(pump);
        nola_core_child_pump_free(pump);
    }

    let recall_req = json!({
        "query": "Rust memory safety",
        "limit": 1,
        "items": [
            {"title": "Cooking", "content": "Pasta recipe", "updated_at_ms": 100},
            {"title": "Systems", "content": "Rust ensures memory safety", "updated_at_ms": 200}
        ]
    })
    .to_string();
    let recall_ptr =
        unsafe { nola_core_intelligence_recall(recall_req.as_ptr(), recall_req.len()) };
    let recall_val: Value =
        serde_json::from_str(unsafe { CStr::from_ptr(recall_ptr) }.to_str().unwrap()).unwrap();
    unsafe { nola_core_free_c_string(recall_ptr) };
    assert_eq!(recall_val["items"].as_array().unwrap().len(), 1);
    assert!(recall_val["context"].as_str().unwrap().contains("Rust"));

    let root_c = CString::new(dir.path().to_str().unwrap()).unwrap();
    let pkg_req_c =
        CString::new(r#"{"url":"https://github.com/acme/nola-skills/blob/main/audit/SKILL.md"}"#)
            .unwrap();
    let pkg_ptr =
        unsafe { nola_mobile_github_skill_package_json(root_c.as_ptr(), pkg_req_c.as_ptr()) };
    let pkg_val: Value =
        serde_json::from_str(unsafe { CStr::from_ptr(pkg_ptr) }.to_str().unwrap()).unwrap();
    unsafe { nola_core_free_c_string(pkg_ptr) };
    assert_eq!(pkg_val["ok"], true);
    assert_eq!(pkg_val["value"]["name"], "audit");

    // Retain + free safety
    let retained = unsafe { nola_core_mobile_mcp_retain(app) };
    assert!(!retained.is_null());
    unsafe { nola_core_application_free(retained) };

    let cmd = r#"{"request_id":"r1","client_id":"c1","command":{"type":"health","arguments":{}}}"#;
    let out_ptr = unsafe { nola_core_application_submit(app, cmd.as_ptr(), cmd.len()) };
    let out_str = unsafe { CStr::from_ptr(out_ptr) }.to_str().unwrap().to_string();
    unsafe { nola_core_free_c_string(out_ptr) };
    assert!(out_str.contains("\"status\":\"committed\""));

    // Mobile MCP host: prepare + execute on configured server
    let add_mcp = json!({
        "request_id": "mcp-add-1",
        "client_id": "c1",
        "command": {
            "type": "mcp",
            "arguments": {
                "operation": "upsert",
                "server": {
                    "id": "srv-1",
                    "name": "Test MCP",
                    "transport": "streamable_http",
                    "url": "http://127.0.0.1:1/mcp",
                    "enabled": true
                }
            }
        }
    })
    .to_string();
    let add_ptr = unsafe { nola_core_application_submit(app, add_mcp.as_ptr(), add_mcp.len()) };
    unsafe { nola_core_free_c_string(add_ptr) };

    let prep_req = json!({
        "operation": "prepare",
        "server_id": "srv-1",
        "tool_name": "echo",
        "arguments": {"msg": "hello"}
    })
    .to_string();
    let prep_ptr =
        unsafe { nola_core_mobile_mcp_host(app, prep_req.as_ptr(), prep_req.len()) };
    let prep_val: Value =
        serde_json::from_str(unsafe { CStr::from_ptr(prep_ptr) }.to_str().unwrap()).unwrap();
    unsafe { nola_core_free_c_string(prep_ptr) };
    assert_eq!(prep_val["ok"], true);
    assert_eq!(prep_val["result"]["prepared"], true);
    assert_eq!(prep_val["result"]["server_id"], "srv-1");
    assert_eq!(prep_val["result"]["tool_name"], "echo");

    // execute aliases call_tool and returns clean not_found for missing server
    let exec_req = json!({
        "operation": "execute",
        "server_id": "missing-srv",
        "tool_name": "echo"
    })
    .to_string();
    let exec_ptr =
        unsafe { nola_core_mobile_mcp_host(app, exec_req.as_ptr(), exec_req.len()) };
    let exec_val: Value =
        serde_json::from_str(unsafe { CStr::from_ptr(exec_ptr) }.to_str().unwrap()).unwrap();
    unsafe { nola_core_free_c_string(exec_ptr) };
    assert_eq!(exec_val["ok"], false);
    assert_eq!(exec_val["error"]["code"], "not_found");

    // Prompt composer
    let prompt_in = r#"{"schema_version":1,"fragments":[{"id":"sys","layer":"identity","trust":"host_instruction","cacheable":true,"content":"You are Nola."},{"id":"dyn","layer":"runtime_state","trust":"host_fact","cacheable":false,"content":"Now"}]}"#;
    let comp_ptr =
        unsafe { nola_core_compose_agent_prompt_json(prompt_in.as_ptr(), prompt_in.len()) };
    let comp_str = unsafe { CStr::from_ptr(comp_ptr) }.to_str().unwrap().to_string();
    unsafe { nola_core_free_c_string(comp_ptr) };
    let comp_val: Value = serde_json::from_str(&comp_str).unwrap();
    assert_eq!(comp_val["composition"]["stable_prefix"], "You are Nola.");
    assert_eq!(comp_val["composition"]["dynamic_suffix"], "Now");

    // Continuation stitching overlap removal
    let stitch_in = r#"{"accumulated":"Hello world","continuation":"world!"}"#;
    let st_ptr =
        unsafe { nola_core_stitch_agent_model_continuation(stitch_in.as_ptr(), stitch_in.len()) };
    let st_str = unsafe { CStr::from_ptr(st_ptr) }.to_str().unwrap().to_string();
    unsafe { nola_core_free_c_string(st_ptr) };
    assert_eq!(serde_json::from_str::<Value>(&st_str).unwrap()["append"], "!");

    // Privacy redaction (4-arg + credential scrub)
    let payload = r#"{"prompt":"key=sk-1234567890abcdefghijklmnop token=custom-secret-999"}"#;
    let cred = "custom-secret-999";
    let red_ptr = unsafe {
        nola_core_redact_model_request_json(
            payload.as_ptr(),
            payload.len(),
            cred.as_ptr(),
            cred.len(),
        )
    };
    let red_str = unsafe { CStr::from_ptr(red_ptr) }.to_str().unwrap().to_string();
    unsafe { nola_model_privacy_free(red_ptr) };
    let parsed: Value = serde_json::from_str(&red_str).unwrap();
    assert_eq!(
        parsed["prompt"],
        "key=[CREDENTIAL] token=[CREDENTIAL]"
    );

    unsafe { nola_core_application_free(app) };
}

#[test]
fn data_source_resolve_enforces_all_consent_policies_and_freshness() {
    let eval = |payload: Value| -> Value {
        let raw = payload.to_string();
        let ptr = unsafe { nola_core_data_source_resolve(raw.as_ptr(), raw.len()) };
        assert!(!ptr.is_null());
        let parsed: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(ptr) }.to_str().unwrap()).unwrap();
        unsafe { nola_core_free_c_string(ptr) };
        parsed
    };

    // 1. opt_in MUST be disabled & not usable without explicit conversationOverride: true
    let opt_in_default = eval(json!({
        "sourceId": "system:calendar",
        "policy": "opt_in",
        "conversationOverride": null,
        "contract": {
            "sourceId": "system:calendar",
            "lastRefreshedAt": "2026-11-25T12:00:00Z",
            "ttlSeconds": 600
        },
        "now": "2026-11-25T12:05:00Z"
    }));
    assert_eq!(opt_in_default["enabled"], false);
    assert_eq!(opt_in_default["usable"], false);
    assert_eq!(opt_in_default["shouldRequestConsent"], false);
    assert_eq!(opt_in_default["reason"], "conversation_opt_in_required");

    let opt_in_enabled = eval(json!({
        "sourceId": "system:calendar",
        "policy": "opt_in",
        "conversationOverride": true,
        "contract": {
            "sourceId": "system:calendar",
            "lastRefreshedAt": "2026-11-25T12:00:00Z",
            "ttlSeconds": 600
        },
        "now": "2026-11-25T12:05:00Z"
    }));
    assert_eq!(opt_in_enabled["enabled"], true);
    assert_eq!(opt_in_enabled["usable"], true);
    assert_eq!(opt_in_enabled["stale"], false);
    assert_eq!(opt_in_enabled["shouldRefresh"], false);

    // 2. ask_each_time MUST require consent and be unusable until explicitly consented
    let ask_default = eval(json!({
        "sourceId": "system:contacts",
        "policy": "ask_each_time",
        "conversationOverride": null,
        "contract": {
            "sourceId": "system:contacts",
            "lastRefreshedAt": "2026-11-25T12:00:00Z",
            "ttlSeconds": 600
        },
        "now": "2026-11-25T12:01:00Z"
    }));
    assert_eq!(ask_default["enabled"], false);
    assert_eq!(ask_default["usable"], false);
    assert_eq!(ask_default["shouldRequestConsent"], true);
    assert_eq!(ask_default["reason"], "consent_required");

    let ask_consented = eval(json!({
        "sourceId": "system:contacts",
        "policy": "ask_each_time",
        "conversationOverride": true,
        "contract": {
            "sourceId": "system:contacts",
            "lastRefreshedAt": "2026-11-25T12:00:00Z",
            "ttlSeconds": 600
        },
        "now": "2026-11-25T12:01:00Z"
    }));
    assert_eq!(ask_consented["enabled"], true);
    assert_eq!(ask_consented["usable"], true);
    assert_eq!(ask_consented["shouldRequestConsent"], false);

    // 3. disabled MUST always stay disabled even with conversationOverride: true
    let disabled = eval(json!({
        "sourceId": "system:mail",
        "policy": "disabled",
        "conversationOverride": true,
        "contract": {"sourceId": "system:mail", "lastRefreshedAt": "2026-11-25T12:00:00Z", "ttlSeconds": 600}
    }));
    assert_eq!(disabled["enabled"], false);
    assert_eq!(disabled["usable"], false);
    assert_eq!(disabled["reason"], "blocked");

    // 4. Stale contract evaluation via now vs lastRefreshedAt + ttlSeconds
    let stale = eval(json!({
        "sourceId": "system:notes",
        "policy": "default_enabled",
        "contract": {
            "sourceId": "system:notes",
            "lastRefreshedAt": "2026-11-25T10:00:00Z",
            "ttlSeconds": 300
        },
        "now": "2026-11-25T12:00:00Z"
    }));
    assert_eq!(stale["enabled"], true);
    assert_eq!(stale["usable"], false);
    assert_eq!(stale["stale"], true);
    assert_eq!(stale["shouldRefresh"], true);
    assert_eq!(stale["reason"], "stale");

    // 5. Missing contract -> enabled true (for Swift isEnabledForConversation), usable false, shouldRefresh true
    let missing_contract = eval(json!({
        "sourceId": "system:notes",
        "policy": "default_enabled"
    }));
    assert_eq!(missing_contract["enabled"], true);
    assert_eq!(missing_contract["usable"], false);
    assert_eq!(missing_contract["shouldRefresh"], true);
    assert_eq!(missing_contract["reason"], "contract_missing");
}

#[test]
fn child_pump_real_host_roundtrip_and_cancellation() {
    let dir = tempfile::tempdir().unwrap();
    let db_str = dir.path().join("child_pump.db").display().to_string();
    let app = unsafe { nola_core_application_open(db_str.as_ptr(), db_str.len()) };
    assert!(!app.is_null());

    let start_req = json!({
        "task_id": "task-child-1",
        "conversation_id": "conv-child-1",
        "run_id": "run-child-1",
        "assistant_message_id": "run-child-1:assistant",
        "prompt": "Summarize workspace",
        "host_driven": true
    })
    .to_string();
    let mut err_ptr: *mut c_char = std::ptr::null_mut();
    let pump = unsafe {
        nola_core_child_pump_start(app, start_req.as_ptr(), start_req.len(), &mut err_ptr)
    };
    assert!(!pump.is_null());
    assert!(err_ptr.is_null());

    // 1. Poll should yield real host_request ticket with pump_id & transport_finished=false
    let p1_ptr = unsafe { nola_core_child_pump_poll(pump, 1000) };
    let p1: Value =
        serde_json::from_str(unsafe { CStr::from_ptr(p1_ptr) }.to_str().unwrap()).unwrap();
    unsafe { nola_core_free_c_string(p1_ptr) };
    assert_eq!(p1["ok"], true);
    assert_eq!(p1["result"]["transport_finished"], false);
    assert_eq!(p1["result"]["kind"], "host_request");
    let pump_id = p1["result"]["pump_id"].as_str().unwrap().to_string();
    assert!(!pump_id.is_empty());
    let ticket = p1["result"]["request"]["ticket"].as_u64().unwrap();
    assert_eq!(
        p1["result"]["request"]["envelope"]["payload"]["type"],
        "model_stream"
    );

    // 2. Deliver host response back to running child pump
    let reply = json!({
        "pump_id": pump_id,
        "ticket": ticket,
        "response": {"content": "Child summary complete."}
    })
    .to_string();
    let r_ptr = unsafe { nola_core_child_pump_respond(pump, reply.as_ptr(), reply.len()) };
    let r_val: Value =
        serde_json::from_str(unsafe { CStr::from_ptr(r_ptr) }.to_str().unwrap()).unwrap();
    unsafe { nola_core_free_c_string(r_ptr) };
    assert_eq!(r_val["ok"], true);
    assert_eq!(r_val["result"]["committed"], true);

    // 3. Poll receives turn_completed event, then transport_finished=true
    let p2_ptr = unsafe { nola_core_child_pump_poll(pump, 1000) };
    let p2: Value =
        serde_json::from_str(unsafe { CStr::from_ptr(p2_ptr) }.to_str().unwrap()).unwrap();
    unsafe { nola_core_free_c_string(p2_ptr) };
    assert_eq!(p2["result"]["kind"], "turn_completed");
    assert_eq!(p2["result"]["event"]["final_text"], "Child summary complete.");

    let p3_ptr = unsafe { nola_core_child_pump_poll(pump, 1000) };
    let p3: Value =
        serde_json::from_str(unsafe { CStr::from_ptr(p3_ptr) }.to_str().unwrap()).unwrap();
    unsafe { nola_core_free_c_string(p3_ptr) };
    assert_eq!(p3["result"]["transport_finished"], true);
    assert_eq!(p3["result"]["pump_id"], pump_id);

    unsafe {
        nola_core_child_pump_close(pump);
        nola_core_child_pump_free(pump);
        nola_core_application_free(app);
    }
}

#[test]
fn mobile_mcp_host_prepare_nested_execute_and_cancel() {
    let dir = tempfile::tempdir().unwrap();
    let db_str = dir.path().join("mcp_host.db").display().to_string();
    let app = unsafe { nola_core_application_open(db_str.as_ptr(), db_str.len()) };
    assert!(!app.is_null());

    let app_ref = unsafe { &*app };
    app_ref
        .inner
        .store
        .upsert_mcp_server(&nola_types::McpServerConfig {
            id: "docs-srv".into(),
            name: "Docs MCP".into(),
            transport: nola_types::McpTransport::StreamableHttp,
            command: None,
            args: Vec::new(),
            env: BTreeMap::new(),
            url: Some("http://127.0.0.1:1/mcp".into()),
            headers: BTreeMap::new(),
            oauth_client_id: None,
            oauth_scopes: Vec::new(),
            enabled: true,
        })
        .unwrap();
    // Seed tool cache for offline deterministic discovery verification
    app_ref
        .inner
        .store
        .set_kv(
            "mcp_tools_cache:docs-srv",
            &json!([
                {"name": "search", "description": "Search docs", "inputSchema": {"type": "object"}},
                {"name": "blocked_op", "description": "Blocked", "inputSchema": {"type": "object"}}
            ]),
        )
        .unwrap();

    // 1. iOS multi-server prepare (host_id + session_id + policy)
    let prep_req = json!({
        "type": "prepare",
        "host_id": "ios-host-1",
        "session_id": "sess-1",
        "policy": {"user_policy": {"deny_tools": ["blocked_op"]}}
    })
    .to_string();
    let prep_ptr =
        unsafe { nola_core_mobile_mcp_host(app, prep_req.as_ptr(), prep_req.len()) };
    let prep: Value =
        serde_json::from_str(unsafe { CStr::from_ptr(prep_ptr) }.to_str().unwrap()).unwrap();
    unsafe { nola_core_free_c_string(prep_ptr) };
    assert_eq!(prep["ok"], true);
    assert_eq!(prep["result"]["prepared"], true);
    assert!(prep["result"]["capabilities"]["mcp__docs-srv__search"].is_object());
    assert!(prep["result"]["capabilities"]["mcp__docs-srv__blocked_op"].is_null());
    assert_eq!(prep["result"]["tools"].as_array().unwrap().len(), 1);

    // 2. Pre-cancel request_id via cancel (works with null app!) then execute nested request -> aborted immediately!
    let cancel_req = json!({"type": "cancel", "request_id": "req-cancel-42"}).to_string();
    let c_ptr = unsafe {
        nola_core_mobile_mcp_host(std::ptr::null_mut(), cancel_req.as_ptr(), cancel_req.len())
    };
    let c_val: Value =
        serde_json::from_str(unsafe { CStr::from_ptr(c_ptr) }.to_str().unwrap()).unwrap();
    unsafe { nola_core_free_c_string(c_ptr) };
    assert_eq!(c_val["ok"], true);
    assert_eq!(c_val["result"]["cancelled"], true);

    let exec_req = json!({
        "type": "execute",
        "host_id": "ios-host-1",
        "session_id": "sess-1",
        "request": {
            "server_id": "docs-srv",
            "tool_name": "search",
            "arguments": {"q": "rust"},
            "request_id": "req-cancel-42"
        }
    })
    .to_string();
    let e_ptr = unsafe { nola_core_mobile_mcp_host(app, exec_req.as_ptr(), exec_req.len()) };
    let e_val: Value =
        serde_json::from_str(unsafe { CStr::from_ptr(e_ptr) }.to_str().unwrap()).unwrap();
    unsafe { nola_core_free_c_string(e_ptr) };
    assert_eq!(e_val["ok"], false);
    assert_eq!(e_val["error"]["code"], "cancelled");

    unsafe { nola_core_application_free(app) };
}

#[test]
fn context_prepare_preserves_tool_pairs_and_emits_component_metrics() {
    let huge = "payload ".repeat(200);
    let req = json!({
        "messages": [
            {"role": "system", "content": "sys"},
            {"role": "assistant", "content": huge, "tool_calls": [{"id": "tc-1", "name": "read", "arguments": {}}]},
            {"role": "tool", "tool_call_id": "tc-1", "content": "result-1"},
            {"role": "user", "content": "follow up"},
            {"role": "assistant", "content": "done"}
        ],
        "tools": [{"name": "read", "parameters": {}}],
        "budget": {"prompt_token_limit": 80, "keep_recent_messages": 2}
    })
    .to_string();
    let ptr = unsafe { nola_core_context_prepare(req.as_ptr(), req.len()) };
    let val: Value =
        serde_json::from_str(unsafe { CStr::from_ptr(ptr) }.to_str().unwrap()).unwrap();
    unsafe { nola_core_free_c_string(ptr) };
    assert_eq!(val["changed"], true);
    let msgs = val["messages"].as_array().unwrap();
    // Both assistant(tool_calls: tc-1) and tool(tool_call_id: tc-1) MUST be pruned atomically together!
    assert!(!msgs.iter().any(|m| m["role"] == "tool"));
    assert!(!msgs
        .iter()
        .any(|m| m.get("tool_calls").and_then(|v| v.as_array()).is_some()));
    assert!(!val["component_metrics"].as_array().unwrap().is_empty());
}

#[test]
fn compose_agent_prompt_enforces_layer_order_and_trust_invariants() {
    // 1. Canonical layer ordering (identity before response_style even when supplied reversed)
    let ordered_in = json!({
        "schema_version": 1,
        "fragments": [
            {"id": "style", "layer": "response_style", "trust": "host_instruction", "cacheable": true, "content": "Be concise."},
            {"id": "ident", "layer": "identity", "trust": "host_instruction", "cacheable": true, "content": "You are Nola."},
            {"id": "rt", "layer": "runtime_state", "trust": "host_fact", "cacheable": false, "content": "cwd=/tmp"}
        ]
    })
    .to_string();
    let ptr = unsafe { nola_core_compose_agent_prompt_json(ordered_in.as_ptr(), ordered_in.len()) };
    let val: Value =
        serde_json::from_str(unsafe { CStr::from_ptr(ptr) }.to_str().unwrap()).unwrap();
    unsafe { nola_core_free_c_string(ptr) };
    assert_eq!(val["composition"]["fragment_ids"], json!(["ident", "style", "rt"]));

    // 2. Reject ReferenceMarkedCacheable
    let bad_ref = json!({
        "schema_version": 1,
        "fragments": [
            {"id": "ref1", "layer": "conversation_context", "trust": "reference", "cacheable": true, "content": "untrusted"}
        ]
    })
    .to_string();
    let p_ref = unsafe { nola_core_compose_agent_prompt_json(bad_ref.as_ptr(), bad_ref.len()) };
    let v_ref: Value =
        serde_json::from_str(unsafe { CStr::from_ptr(p_ref) }.to_str().unwrap()).unwrap();
    unsafe { nola_core_free_c_string(p_ref) };
    assert_eq!(v_ref["error"]["code"], "reference_marked_cacheable");

    // 3. Reject RuntimeStateMarkedCacheable
    let bad_rt_cache = json!({
        "schema_version": 1,
        "fragments": [
            {"id": "rt1", "layer": "runtime_state", "trust": "host_fact", "cacheable": true, "content": "now"}
        ]
    })
    .to_string();
    let p_rtc =
        unsafe { nola_core_compose_agent_prompt_json(bad_rt_cache.as_ptr(), bad_rt_cache.len()) };
    let v_rtc: Value =
        serde_json::from_str(unsafe { CStr::from_ptr(p_rtc) }.to_str().unwrap()).unwrap();
    unsafe { nola_core_free_c_string(p_rtc) };
    assert_eq!(v_rtc["error"]["code"], "runtime_state_marked_cacheable");

    // 4. Reject RuntimeStateInvalidTrust
    let bad_rt_trust = json!({
        "schema_version": 1,
        "fragments": [
            {"id": "rt2", "layer": "runtime_state", "trust": "user_instruction", "cacheable": false, "content": "now"}
        ]
    })
    .to_string();
    let p_rtt =
        unsafe { nola_core_compose_agent_prompt_json(bad_rt_trust.as_ptr(), bad_rt_trust.len()) };
    let v_rtt: Value =
        serde_json::from_str(unsafe { CStr::from_ptr(p_rtt) }.to_str().unwrap()).unwrap();
    unsafe { nola_core_free_c_string(p_rtt) };
    assert_eq!(v_rtt["error"]["code"], "runtime_state_invalid_trust");
}

#[test]
fn intelligence_recall_without_items_queries_store_and_strategies_and_policy_ignores_tool_id_keywords() {
    let dir = tempfile::tempdir().unwrap();
    let db_str = dir.path().join("intel.db").display().to_string();
    let app = unsafe { nola_core_application_open(db_str.as_ptr(), db_str.len()) };
    assert!(!app.is_null());

    let app_ref = unsafe { &*app };
    app_ref
        .inner
        .store
        .upsert_memory(&MemoryItem {
            id: "m-mac-1".into(),
            namespace: "intelligence:memory".into(),
            title: "Build Preference".into(),
            content: "Always use cargo test --lib for Rust unit checks".into(),
            tags: vec!["rust".into()],
            created_at_ms: 1000,
            updated_at_ms: 2000,
        })
        .unwrap();
    app_ref
        .inner
        .store
        .put_module_record(
            "memory_strategy",
            "strat-1",
            &json!({"id": "strat-1", "title": "Rust Verification", "summary": "Run cargo test after edits"}),
        )
        .unwrap();

    // macOS path: omits `items` completely!
    let recall_req = json!({
        "query": "cargo test",
        "limit": 6,
        "minimum_score": 0.2,
        "db_path": db_str
    })
    .to_string();
    let r_ptr =
        unsafe { nola_core_intelligence_recall(recall_req.as_ptr(), recall_req.len()) };
    let r_val: Value =
        serde_json::from_str(unsafe { CStr::from_ptr(r_ptr) }.to_str().unwrap()).unwrap();
    unsafe { nola_core_free_c_string(r_ptr) };
    assert_eq!(r_val["ok"], true);
    assert!(!r_val["items"].as_array().unwrap().is_empty());
    assert!(!r_val["strategies"].as_array().unwrap().is_empty());
    assert!(r_val["context"].as_str().unwrap().contains("cargo test"));

    // Snapshot includes real strategies
    let snap_ptr = unsafe { nola_core_application_intelligence_snapshot(app) };
    let snap: Value =
        serde_json::from_str(unsafe { CStr::from_ptr(snap_ptr) }.to_str().unwrap()).unwrap();
    unsafe { nola_core_free_c_string(snap_ptr) };
    assert_eq!(snap["strategies"].as_array().unwrap().len(), 1);

    // Tool policy MUST NOT use tool_id substring keywords (AGENTS.md guardrail)
    let tp_req = json!({
        "schema_version": 1,
        "platform": "ios",
        "now_ms": 1000,
        "runtime": {
            "expires_at_ms": 999999,
            "tools": [
                {"tool_id": "write_delete_exec_name", "effect": "read_only", "capability": "safe_cap", "route": "device", "available": true},
                {"tool_id": "read_list_get_name", "effect": "high", "capability": "danger_cap", "route": "device", "available": true}
            ]
        },
        "user_policy": {"max_risk": "read_only", "allow_result_egress_routes": ["device"]}
    })
    .to_string();
    let tp_ptr = unsafe { nola_core_tool_policy_resolve(tp_req.as_ptr(), tp_req.len()) };
    let tp: Value =
        serde_json::from_str(unsafe { CStr::from_ptr(tp_ptr) }.to_str().unwrap()).unwrap();
    unsafe { nola_core_free_c_string(tp_ptr) };
    assert_eq!(tp["enabled_tools"].as_array().unwrap().len(), 1);
    assert_eq!(tp["enabled_tools"][0]["tool_id"], "write_delete_exec_name");
    assert_eq!(tp["disabled_tools"][0]["tool_id"], "read_list_get_name");
    assert_eq!(tp["disabled_tools"][0]["reason"], "risk_exceeds_user_policy");

    unsafe { nola_core_application_free(app) };
}

#[test]
fn ffi_second_pass_contracts_context_expert_updater_mcp_and_child_pump() {
    let dir = tempfile::tempdir().unwrap();
    let db_str = dir.path().join("second_pass.db").display().to_string();
    let app = unsafe { nola_core_application_open(db_str.as_ptr(), db_str.len()) };
    assert!(!app.is_null());

    // 1. context_prepare emits all 7 canonical MacCoreComponentMetric keys
    let cp_req = json!({
        "messages": [
            {"id": "m1", "role": "user", "content": "hi"},
            {"id": "m2", "role": "assistant", "content": "hello"}
        ],
        "tools": [{"name": "read_file", "parameters": {}}],
        "components": [{"id": "sys-1", "kind": "system", "content": "You are Nola"}]
    })
    .to_string();
    let cp_ptr = unsafe { nola_core_context_prepare(cp_req.as_ptr(), cp_req.len()) };
    let cp_val: Value =
        serde_json::from_str(unsafe { CStr::from_ptr(cp_ptr) }.to_str().unwrap()).unwrap();
    unsafe { nola_core_free_c_string(cp_ptr) };
    let metrics = cp_val["component_metrics"].as_array().unwrap();
    assert!(!metrics.is_empty());
    for m in metrics {
        assert!(m["kind"].is_string());
        assert!(m["used_tokens"].is_number());
        assert!(m["dropped_tokens"].is_number());
        assert!(m["truncated_tokens"].is_number());
        assert!(m["used_ids"].is_array());
        assert!(m["dropped_ids"].is_array());
        assert!(m["truncated_ids"].is_array());
    }

    // 2. expert_host_operation reads operation.kind and sets error.outcome_pending = false on rejection
    let exp_list = json!({
        "request_id": "exp-req-1",
        "operation": {"kind": "list"}
    })
    .to_string();
    let el_ptr = unsafe {
        nola_core_application_expert_host_operation(app, exp_list.as_ptr(), exp_list.len())
    };
    let el_val: Value =
        serde_json::from_str(unsafe { CStr::from_ptr(el_ptr) }.to_str().unwrap()).unwrap();
    unsafe { nola_core_free_c_string(el_ptr) };
    assert_eq!(el_val["ok"], true);

    let exp_bad = json!({
        "request_id": "exp-req-2",
        "operation": {"kind": "nonexistent_op_xyz"}
    })
    .to_string();
    let eb_ptr = unsafe {
        nola_core_application_expert_host_operation(app, exp_bad.as_ptr(), exp_bad.len())
    };
    let eb_val: Value =
        serde_json::from_str(unsafe { CStr::from_ptr(eb_ptr) }.to_str().unwrap()).unwrap();
    unsafe { nola_core_free_c_string(eb_ptr) };
    assert_eq!(eb_val["ok"], false);
    assert_eq!(eb_val["error"]["outcome_pending"], false);

    // 3. software_update passes req_target into check_for_update and marks android-store as kind="store"
    let su_req = json!({
        "operation": "check",
        "target": "android-store",
        "current_version": "2.0.0",
        "current_build": 200
    })
    .to_string();
    let su_ptr = unsafe { nola_core_software_update_json(su_req.as_ptr(), su_req.len()) };
    let su_val: Value =
        serde_json::from_str(unsafe { CStr::from_ptr(su_ptr) }.to_str().unwrap()).unwrap();
    unsafe { nola_core_free_c_string(su_ptr) };
    assert_eq!(su_val["ok"], true);
    assert_eq!(su_val["value"]["target"], "android-store");

    let fake_update = nola_modules::UpdateCheckResult {
        status: "update_available".into(),
        current_version: "2.0.0".into(),
        target: "android-store".into(),
        update_available: true,
        latest: Some(nola_modules::UpdateManifest {
            version: "2.1.0".into(),
            download_url: "https://play.google.com/store/apps/details?id=com.nola".into(),
            sha256: "a".repeat(64),
            notes: "Update".into(),
            signature: Some("sig".into()),
            build: Some(210),
            size: None,
            filename: None,
            release_id: Some("rel-210".into()),
        }),
    };
    let formatted_ptr =
        format_update_check_response(fake_update, "2.0.0", 200, "stable", "android-store");
    let formatted: Value =
        serde_json::from_str(unsafe { CStr::from_ptr(formatted_ptr) }.to_str().unwrap())
            .unwrap();
    unsafe { nola_core_free_c_string(formatted_ptr) };
    assert_eq!(formatted["value"]["release"]["artifact"]["kind"], "store");

    // 4. mobile_mcp_host nested error carries tool_completed / call_id / outcome="failed"
    let mcp_exec = json!({
        "type": "execute",
        "host_id": "ios-host-1",
        "session_id": "sess-1",
        "request": {
            "effect_id": "eff-mcp-1",
            "payload": {
                "call_id": "call-mcp-9",
                "capability_id": "mcp__missing-srv__lookup",
                "input": {"q": "rust"}
            }
        }
    })
    .to_string();
    let me_ptr = unsafe { nola_core_mobile_mcp_host(app, mcp_exec.as_ptr(), mcp_exec.len()) };
    let me_val: Value =
        serde_json::from_str(unsafe { CStr::from_ptr(me_ptr) }.to_str().unwrap()).unwrap();
    unsafe { nola_core_free_c_string(me_ptr) };
    assert_eq!(me_val["ok"], false);
    assert_eq!(me_val["result"]["type"], "tool_completed");
    assert_eq!(me_val["result"]["call_id"], "call-mcp-9");
    assert_eq!(me_val["result"]["outcome"], "failed");

    // 5. child_pump loads prompt from prepared mobile_delegated_task, emits full envelope fields,
    //    keeps ticket alive across execution_started + model_delta, and parses resolution.decision!
    let app_ref = unsafe { &*app };
    app_ref
        .inner
        .suite
        .upsert_record(
            "mobile_delegated_task",
            "task-mobile-1",
            &json!({
                "id": "task-mobile-1",
                "task": {
                    "id": "task-mobile-1",
                    "owner_session_id": "parent-1",
                    "child_session_id": "child-task-mobile-1",
                    "prompt": "Summarize release notes",
                    "status": "running"
                }
            }),
        )
        .unwrap();
    let start_req = json!({
        "task_id": "task-mobile-1",
        "conversation_id": "child-task-mobile-1",
        "core_session_id": "core-child-1",
        "run_id": "run-mobile-1",
        "assistant_message_id": "run-mobile-1:assistant",
        "attempt": 1,
        "worker": {"pid": 7}
    })
    .to_string();
    let mut err_ptr: *mut c_char = std::ptr::null_mut();
    let pump = unsafe {
        nola_core_child_pump_start(app, start_req.as_ptr(), start_req.len(), &mut err_ptr)
    };
    assert!(!pump.is_null());
    let p1_ptr = unsafe { nola_core_child_pump_poll(pump, 1000) };
    let p1: Value =
        serde_json::from_str(unsafe { CStr::from_ptr(p1_ptr) }.to_str().unwrap()).unwrap();
    unsafe { nola_core_free_c_string(p1_ptr) };
    assert_eq!(p1["result"]["kind"], "host_request");
    let pump_id = p1["result"]["pump_id"].as_str().unwrap().to_string();
    let ticket = p1["result"]["request"]["ticket"].as_u64().unwrap();
    let env = &p1["result"]["request"]["envelope"];
    assert_eq!(env["session_id"], "core-child-1");
    assert!(!env["turn_id"].as_str().unwrap().is_empty());
    assert!(env["delivery"].is_object());
    assert_eq!(env["payload"]["prompt"], "Summarize release notes");
    assert!(!env["payload"]["text_part_id"].as_str().unwrap().is_empty());
    assert!(!env["payload"]["reasoning_part_id"]
        .as_str()
        .unwrap()
        .is_empty());

    // Intermediate responses MUST NOT remove ticket!
    for inter in [
        json!({"pump_id": pump_id, "ticket": ticket, "response": {"type": "execution_started"}}),
        json!({"pump_id": pump_id, "ticket": ticket, "response": {"type": "model_delta", "channel": "text", "delta": "Part 1 "}}),
        json!({"pump_id": pump_id, "ticket": ticket, "response": {"type": "model_delta", "channel": "text", "delta": "Part 2"}}),
    ] {
        let s = inter.to_string();
        let rp = unsafe { nola_core_child_pump_respond(pump, s.as_ptr(), s.len()) };
        let rv: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(rp) }.to_str().unwrap()).unwrap();
        unsafe { nola_core_free_c_string(rp) };
        assert_eq!(rv["ok"], true);
        assert_eq!(rv["result"]["committed"], true);
    }
    // Terminal response completes the ticket
    let term = json!({
        "pump_id": pump_id,
        "ticket": ticket,
        "response": {"type": "model_completed"}
    })
    .to_string();
    let tp = unsafe { nola_core_child_pump_respond(pump, term.as_ptr(), term.len()) };
    let tv: Value =
        serde_json::from_str(unsafe { CStr::from_ptr(tp) }.to_str().unwrap()).unwrap();
    unsafe { nola_core_free_c_string(tp) };
    assert_eq!(tv["ok"], true);

    // Drain events until turn_completed and check accumulated delta text
    let mut got_final = String::new();
    for _ in 0..10 {
        let ep = unsafe { nola_core_child_pump_poll(pump, 1000) };
        let ev: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(ep) }.to_str().unwrap()).unwrap();
        unsafe { nola_core_free_c_string(ep) };
        if ev["result"]["kind"] == "turn_completed" {
            got_final = ev["result"]["event"]["final_text"]
                .as_str()
                .unwrap()
                .to_string();
        }
        if ev["result"]["transport_finished"] == true {
            break;
        }
    }
    assert_eq!(got_final, "Part 1 Part 2");

    // Permission resolution parser handles allow_once / allow_conversation / deny
    assert!(parse_child_permission_decision(&json!({
        "response": {"type": "permission_resolved", "resolution": {"decision": "allow_once"}}
    })));
    assert!(parse_child_permission_decision(&json!({
        "response": {"type": "permission_resolved", "resolution": {"decision": "allow_conversation"}}
    })));
    assert!(!parse_child_permission_decision(&json!({
        "response": {"type": "permission_resolved", "resolution": {"decision": "deny"}}
    })));

    unsafe {
        nola_core_child_pump_close(pump);
        nola_core_child_pump_free(pump);
        nola_core_application_free(app);
    }
}
