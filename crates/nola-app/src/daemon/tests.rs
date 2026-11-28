// SPDX-License-Identifier: Apache-2.0

use super::*;

#[test]
fn daemon_model_agent_room_and_top_level_gap_commands_work() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    let suite = ModulesSuite::new(store.clone(), dir.path().to_path_buf(), None);

    // 1. Model domain commands
    let acct = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({"operation": "save_account", "id": "acct-1", "label": "Work"}),
    )
    .unwrap();
    assert_eq!(acct["ok"], true);
    assert_eq!(acct["id"], "acct-1");

    let conn = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({
            "operation": "save_connection",
            "id": "conn-1",
            "endpoint": "https://example.invalid/v1",
            "protocol": "siwc_responses",
            "models": ["gpt-5"]
        }),
    )
    .unwrap();
    assert_eq!(conn["ok"], true);

    let cap = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({
            "operation": "set_connection_capability_override",
            "connection_id": "conn-1",
            "model_id": "gpt-5",
            "capabilities": {"reasoning": true}
        }),
    )
    .unwrap();
    assert_eq!(cap["ok"], true);

    let catalog = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({
            "operation": "refresh_connection_catalog",
            "connection_id": "conn-1",
            "models": ["gpt-5", "gpt-5-mini"]
        }),
    )
    .unwrap();
    assert_eq!(catalog["models"].as_array().unwrap().len(), 2);

    let cfg_models = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({
            "operation": "configure_connection_models",
            "connection_id": "conn-1",
            "enabled_model_ids": ["conn-1:gpt-5"],
            "selected_model_id": "conn-1:gpt-5"
        }),
    )
    .unwrap();
    assert_eq!(cfg_models["ok"], true);

    // 2. Agent room create + submission lifecycle
    let room = dispatch_agent_room_command(
        &store,
        &suite,
        &json!({"command": {"type": "create", "id": "room-1", "name": "War Room"}}),
    )
    .unwrap();
    assert_eq!(room["room"]["id"], "room-1");

    let sub = dispatch_agent_room_command(
        &store,
        &suite,
        &json!({"command": {"type": "submit", "room_id": "room-1", "content": "Review PR"}}),
    )
    .unwrap();
    let sub_id = sub["submission_id"].as_str().unwrap().to_string();

    let got_sub = dispatch_agent_room_command(
        &store,
        &suite,
        &json!({"command": {"type": "submission", "submission_id": sub_id}}),
    )
    .unwrap();
    assert_eq!(got_sub["submission_id"], sub_id);

    let subs = dispatch_agent_room_command(
        &store,
        &suite,
        &json!({"command": {"type": "submissions", "room_id": "room-1"}}),
    )
    .unwrap();
    assert_eq!(subs.as_array().unwrap().len(), 1);

    let sel = dispatch_agent_room_command(
        &store,
        &suite,
        &json!({"command": {"type": "select_recipient", "room_id": "room-1", "submission_id": sub_id, "recipient_id": "reviewer"}}),
    )
    .unwrap();
    assert_eq!(sel["recipient_id"], "reviewer");

    let prep = dispatch_agent_room_command(
        &store,
        &suite,
        &json!({"command": {"type": "prepare_execution", "room_id": "room-1", "submission_id": sub_id}}),
    )
    .unwrap();
    assert_eq!(prep["ready"], true);

    let claimed = dispatch_agent_room_command(
        &store,
        &suite,
        &json!({"command": {"type": "claim_execution", "room_id": "room-1", "submission_id": sub_id}}),
    )
    .unwrap();
    assert_eq!(claimed["claimed"], true);

    let intent = dispatch_agent_room_command(
        &store,
        &suite,
        &json!({"command": {"type": "execution_intent", "room_id": "room-1"}}),
    )
    .unwrap();
    assert_eq!(intent["found"], true);

    let reconciled = dispatch_agent_room_command(
        &store,
        &suite,
        &json!({"command": {"type": "reconcile_execution", "submission_id": sub_id, "status": "completed"}}),
    )
    .unwrap();
    assert_eq!(reconciled["reconciled"], true);

    let sub2 = dispatch_agent_room_command(
        &store,
        &suite,
        &json!({"command": {"type": "submit", "room_id": "room-1", "content": "Second"}}),
    )
    .unwrap();
    let sub2_id = sub2["submission_id"].as_str().unwrap().to_string();
    let failed = dispatch_agent_room_command(
        &store,
        &suite,
        &json!({"command": {"type": "fail_submission", "submission_id": sub2_id, "error": "boom"}}),
    )
    .unwrap();
    assert_eq!(failed["failed"], true);

    let _ = dispatch_agent_room_command(
        &store,
        &suite,
        &json!({"command": {"type": "submit", "room_id": "room-1", "content": "Third"}}),
    )
    .unwrap();
    let cancelled = dispatch_agent_room_command(
        &store,
        &suite,
        &json!({"command": {"type": "cancel_pending", "room_id": "room-1"}}),
    )
    .unwrap();
    assert_eq!(cancelled["cancelled_count"], 1);

    // 3. Top-level commands
    let g = dispatch_command(
        &store,
        &suite,
        "upsert_permission_grant",
        &json!({"conversation_id": "c1", "grant": {"id": "g1", "tool": "bash"}}),
    )
    .unwrap();
    assert_eq!(g["grants"].as_array().unwrap().len(), 1);

    let rg = dispatch_command(
        &store,
        &suite,
        "revoke_permission_grant",
        &json!({"conversation_id": "c1", "grant_id": "g1"}),
    )
    .unwrap();
    assert_eq!(rg["revoked"], true);

    let imp = dispatch_command(
        &store,
        &suite,
        "import_conversation_transcript",
        &json!({"conversation_id": "c1", "messages": [{"role": "user", "content": "hi"}, {"role": "assistant", "content": "hello"}]}),
    )
    .unwrap();
    assert_eq!(imp["imported_count"], 2);

    let hist = dispatch_command(
        &store,
        &suite,
        "query_canonical_history",
        &json!({"conversation_id": "c1"}),
    )
    .unwrap();
    assert_eq!(hist["total"], 2);

    let adm = dispatch_command(
        &store,
        &suite,
        "admit_host_output",
        &json!({
            "arguments": {
                "routes": [{
                    "route": {
                        "kind": "native",
                        "adapter": "mlx",
                        "model_id": "fixture-native"
                    },
                    "max_output_tokens": 4096
                }],
                "preference": {"explicit": 2048}
            }
        }),
    )
    .unwrap();
    assert_eq!(adm["resolved"]["source"], "explicit_preference");
    assert_eq!(adm["resolved"]["tokens"], 2048);

    let guid = dispatch_command(
        &store,
        &suite,
        "submit_task_guidance",
        &json!({"task_id": "t1", "guidance": "focus"}),
    )
    .unwrap();
    assert_eq!(guid["ok"], true);
    let load_guid = dispatch_command(
        &store,
        &suite,
        "load_task_guidance_receipt",
        &json!({"task_id": "t1"}),
    )
    .unwrap();
    assert_eq!(load_guid["found"], true);

    let cancel_owned = dispatch_command(
        &store,
        &suite,
        "request_owned_cancellation",
        &json!({"run_id": "run-1"}),
    )
    .unwrap();
    assert_eq!(cancel_owned["cancelled"], true);

    let cur = dispatch_command(
        &store,
        &suite,
        "current_run",
        &json!({"conversation_id": "c1"}),
    )
    .unwrap();
    assert_eq!(cur["active"], false);

    let ctx_usage = dispatch_command(
        &store,
        &suite,
        "local_run_context_usage",
        &json!({"conversation_id": "c1"}),
    )
    .unwrap();
    assert_eq!(ctx_usage["conversation_id"], "c1");

    let sources = dispatch_command(
        &store,
        &suite,
        "read_conversation_tool_sources",
        &json!({"conversation_id": "c1"}),
    )
    .unwrap();
    assert!(sources["mcp_servers"].is_array());

    let stg = dispatch_command(
        &store,
        &suite,
        "stage_archive_manifest",
        &json!({"entries": []}),
    )
    .unwrap();
    assert_eq!(stg["staged"], true);

    let beg = dispatch_command(&store, &suite, "begin_archive_restore", &json!({})).unwrap();
    let rid = beg["restore_id"].as_str().unwrap().to_string();
    let com = dispatch_command(
        &store,
        &suite,
        "commit_archive_restore",
        &json!({"restore_id": rid}),
    )
    .unwrap();
    assert_eq!(com["status"], "committed");
    let rev = dispatch_command(&store, &suite, "archive_directory_revision", &json!({})).unwrap();
    assert_eq!(rev, json!(2));

    // Canonical resolve_chat_model state machine & put_product_setting_checked CAS/validation
    let bad_default = dispatch_command(
        &store,
        &suite,
        "resolve_chat_model",
        &json!({"intent": {"kind": "inherit"}, "default_value_json": "{}", "candidates": []}),
    )
    .unwrap();
    assert_eq!(bad_default, json!({"kind": "invalid_configuration"}));
    let explicit_overrides_bad = dispatch_command(
        &store,
        &suite,
        "resolve_chat_model",
        &json!({
            "intent": {"kind": "explicit", "profile_id": "api:valid"},
            "default_value_json": "{}",
            "candidates": [{"profile_id": "api:valid", "allowed_for_chat": true, "availability": "available"}]
        }),
    )
    .unwrap();
    assert_eq!(
        explicit_overrides_bad,
        json!({"kind": "resolved", "profile_id": "api:valid", "origin": "conversation"})
    );
    let unavail = dispatch_command(
        &store,
        &suite,
        "resolve_chat_model",
        &json!({
            "intent": {"kind": "explicit", "profile_id": "api:restricted"},
            "candidates": [{"profile_id": "api:restricted", "allowed_for_chat": false, "availability": "available"}]
        }),
    )
    .unwrap();
    assert_eq!(
        unavail,
        json!({"kind": "unavailable", "profile_id": "api:restricted", "origin": "conversation", "reason": "not_allowed_for_chat"})
    );
    assert!(dispatch_command(
        &store,
        &suite,
        "put_product_setting_checked",
        &json!({"key": "chat.default_model", "value_json": "{}", "updated_at_ms": 1}),
    )
    .is_err());
    let valid_profile = r#"{"version":1,"selection":{"kind":"profile","profile_id":"local:b"}}"#;
    let saved = dispatch_command(
        &store,
        &suite,
        "put_product_setting_checked",
        &json!({"key": "chat.default_model", "value_json": valid_profile, "updated_at_ms": 10}),
    )
    .unwrap();
    assert_eq!(saved["revision"], 1);
    assert!(dispatch_command(
        &store,
        &suite,
        "put_product_setting_checked",
        &json!({"key": "chat.default_model", "value_json": valid_profile, "updated_at_ms": 11}),
    )
    .is_err());
    let valid_auto = r#"{"version":1,"selection":{"kind":"automatic"}}"#;
    let updated = dispatch_command(
        &store,
        &suite,
        "put_product_setting_checked",
        &json!({"key": "chat.default_model", "value_json": valid_auto, "expected_revision": 1, "updated_at_ms": 12}),
    )
    .unwrap();
    assert_eq!(updated["revision"], 2);
    let no_cand = dispatch_command(
        &store,
        &suite,
        "resolve_chat_model",
        &json!({"intent": {"kind": "inherit"}, "default_value_json": valid_auto, "candidates": []}),
    )
    .unwrap();
    assert_eq!(no_cand, json!({"kind": "no_automatic_candidate"}));

    let up = dispatch_command(
        &store,
        &suite,
        "begin_content_upload",
        &json!({"owner": {"conversation_id": "c1", "import_id": "imp1", "part_id": "p1"}, "expected_utf8_bytes": 5}),
    )
    .unwrap();
    let uid = up["upload_id"].as_str().unwrap().to_string();
    let app_c = dispatch_command(
        &store,
        &suite,
        "append_content_chunk",
        &json!({"upload_id": uid, "owner": {"conversation_id": "c1", "import_id": "imp1", "part_id": "p1"}, "offset_bytes": 0, "text": "hello"}),
    )
    .unwrap();
    assert_eq!(app_c["committed_utf8_bytes"], 5);
    let read_c = dispatch_command(
        &store,
        &suite,
        "read_content_upload_chunk",
        &json!({"upload_id": uid, "owner": {"conversation_id": "c1", "import_id": "imp1", "part_id": "p1"}, "offset_bytes": 0}),
    )
    .unwrap();
    assert_eq!(read_c["text"], "hello");
    let seal = dispatch_command(
        &store,
        &suite,
        "seal_content_upload",
        &json!({"upload_id": uid, "owner": {"conversation_id": "c1", "import_id": "imp1", "part_id": "p1"}}),
    )
    .unwrap();
    assert_eq!(seal["state"], "sealed");
    let prog = dispatch_command(
        &store,
        &suite,
        "content_upload_progress",
        &json!({"upload_id": uid, "owner": {"conversation_id": "c1", "import_id": "imp1", "part_id": "p1"}}),
    )
    .unwrap();
    assert_eq!(prog["state"], "sealed");

    let rt = dispatch_command(
        &store,
        &suite,
        "runtime_task",
        &json!({"operation": "start", "task_id": "rt-1", "conversation_id": "c1"}),
    )
    .unwrap();
    assert_eq!(rt["task"]["status"], "running");
    let rt_list = dispatch_command(
        &store,
        &suite,
        "runtime_task",
        &json!({"operation": "list", "conversation_id": "c1"}),
    )
    .unwrap();
    assert_eq!(rt_list["tasks"].as_array().unwrap().len(), 1);
}

#[test]
fn daemon_real_sync_engine_branch_state_recovery_and_canonical_rpcs() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    let suite = ModulesSuite::new(store.clone(), dir.path().to_path_buf(), None);

    // 1. Real Sync Engine backed by Store sync_changes & sync_cursors
    store
        .record_sync_change(&SyncChangeRecord {
            id: "chg-local-1".into(),
            scope: "conv-sync".into(),
            entity_kind: "kv".into(),
            entity_id: "k1".into(),
            payload_json: r#"{"v":1}"#.into(),
            created_at_ms: 1000,
            resolved: false,
        })
        .unwrap();
    let pending = dispatch_command(&store, &suite, "pending_sync_changes", &json!({})).unwrap();
    assert_eq!(pending.as_array().unwrap().len(), 1);
    assert_eq!(pending[0]["id"], "chg-local-1");

    let resolved = dispatch_command(
        &store,
        &suite,
        "resolve_sync_change",
        &json!({"id": "chg-local-1"}),
    )
    .unwrap();
    assert_eq!(resolved, json!(true));
    let pending_after =
        dispatch_command(&store, &suite, "pending_sync_changes", &json!({})).unwrap();
    assert!(pending_after.as_array().unwrap().is_empty());

    let saved_cur = dispatch_command(
        &store,
        &suite,
        "save_sync_cursor",
        &json!({"replica_id": "peer-a", "cursor": 42}),
    )
    .unwrap();
    assert_eq!(saved_cur["cursor"], 42);
    let got_cur = dispatch_command(
        &store,
        &suite,
        "get_sync_cursor",
        &json!({"replica_id": "peer-a"}),
    )
    .unwrap();
    assert_eq!(got_cur["cursor"], 42);

    // apply_remote_sync_page applies idempotently and updates cursor
    let page = json!({
        "input": {
            "replica_id": "peer-a",
            "next_cursor": 55,
            "changes": [
                {"id": "rem-1", "scope": "global", "entity_kind": "kv", "entity_id": "synced_key", "payload": {"hello": "world"}},
                {"id": "rem-2", "scope": "conv-rem", "entity_kind": "event", "entity_id": "ev-1", "payload": {"kind": "user_message", "payload": {"content": "from remote"}}}
            ]
        }
    });
    let r1 = dispatch_command(&store, &suite, "apply_remote_sync_page", &page).unwrap();
    assert_eq!(r1["applied"], 2);
    assert_eq!(r1["skipped"], 0);
    assert_eq!(r1["cursor"], 55);
    assert_eq!(store.get_kv("synced_key").unwrap().unwrap()["hello"], "world");

    // Re-applying same page MUST skip idempotently!
    let r2 = dispatch_command(&store, &suite, "apply_remote_sync_page", &page).unwrap();
    assert_eq!(r2["applied"], 0);
    assert_eq!(r2["skipped"], 2);

    // 2. branch_conversation copies session_state and supports up_to_seq truncation
    let sess = Session {
        id: "conv-src".into(),
        title: "Original".into(),
        workspace_root: dir.path().display().to_string(),
        model_connection: None,
        model_id: None,
        created_at_ms: 1000,
        updated_at_ms: 1000,
    };
    store.upsert_session(&sess).unwrap();
    let e1 = store
        .append_event("conv-src", "user_message", json!({"id": "u1", "role": "user", "content": "first"}), 50_000)
        .unwrap();
    let e2 = store
        .append_event("conv-src", "assistant_message", json!({"id": "a1", "role": "assistant", "content": "second"}), 50_000)
        .unwrap();
    let _e3 = store
        .append_event("conv-src", "user_message", json!({"id": "u2", "role": "user", "content": "third"}), 50_000)
        .unwrap();
    store
        .set_kv(
            "session_state:conv-src",
            &json!({
                "history": [
                    {"role": "user", "content": "first"},
                    {"role": "assistant", "content": "second"},
                    {"role": "user", "content": "third"}
                ],
                "pinned": true
            }),
        )
        .unwrap();

    let branched = dispatch_command(
        &store,
        &suite,
        "branch_conversation",
        &json!({"conversation_id": "conv-src", "up_to_seq": e2.seq}),
    )
    .unwrap();
    let new_id = branched["conversation"]["id"].as_str().unwrap().to_string();
    let new_state = store.get_kv(&format!("session_state:{new_id}")).unwrap().unwrap();
    assert_eq!(new_state["pinned"], true);
    assert_eq!(new_state["history"].as_array().unwrap().len(), 2);

    // truncate_conversation & prepare_edited_resend atomically trim session_state
    let _ = dispatch_command(
        &store,
        &suite,
        "prepare_edited_resend",
        &json!({"conversation_id": "conv-src", "from_seq": e2.seq}),
    )
    .unwrap();
    let trimmed_state = store.get_kv("session_state:conv-src").unwrap().unwrap();
    assert_eq!(trimmed_state["history"].as_array().unwrap().len(), 1);
    assert_eq!(e1.seq, 1);

    // 3. Active run tracking, durable permission_grant projection, & recovery inspection
    store
        .put_module_record(
            "permission_grant",
            "pg-1",
            &json!({"id": "pg-1", "session_id": "conv-src", "tool": "bash", "active": true}),
        )
        .unwrap();
    let perm_proj = dispatch_command(
        &store,
        &suite,
        "load_conversation_permission_projection_v1",
        &json!({"conversation_id": "conv-src"}),
    )
    .unwrap();
    assert_eq!(perm_proj["grants"].as_array().unwrap().len(), 1);
    assert_eq!(perm_proj["grants"][0]["id"], "pg-1");

    store
        .put_module_record(
            "durable_task",
            "dt-1",
            &json!({"task_id": "dt-1", "session_id": "conv-src", "status": "interrupted", "role": "worker"}),
        )
        .unwrap();
    store
        .set_kv(
            "turn_verification:conv-src",
            &json!({
                "all_modified_verified": false,
                "modified_files": [{"uri": "file:///tmp/a.rs", "operation": "write", "last_modified_step": 1, "verified_after_edit": false}]
            }),
        )
        .unwrap();
    let rec_run = dispatch_command(
        &store,
        &suite,
        "inspect_run_recovery",
        &json!({"conversation_id": "conv-src"}),
    )
    .unwrap();
    assert_eq!(rec_run["recoverable"], true);
    assert_eq!(rec_run["state"], "interrupted");

    let rec_eff = dispatch_command(
        &store,
        &suite,
        "inspect_unknown_effect_recovery",
        &json!({"conversation_id": "conv-src"}),
    )
    .unwrap();
    assert_eq!(rec_eff["pending"], true);

    let rec_owned = dispatch_command(
        &store,
        &suite,
        "inspect_owned_task_recovery",
        &json!({"conversation_id": "conv-src"}),
    )
    .unwrap();
    assert_eq!(rec_owned["pending"], true);

    // 4. Direct event range / FTS historical reads
    let hist_part = dispatch_command(
        &store,
        &suite,
        "read_historical_part",
        &json!({"conversation_id": "conv-src", "seq": 1}),
    )
    .unwrap();
    assert_eq!(hist_part["found"], true);
    assert_eq!(hist_part["content"], "first");

    let range_fts = dispatch_command(
        &store,
        &suite,
        "read_conversation_text_range",
        &json!({"conversation_id": "conv-src", "query": "first"}),
    )
    .unwrap();
    assert!(range_fts["text"].as_str().unwrap().contains("first"));

    // 5. Missing canonical & resource RPCs
    let pdf = dispatch_command(
        &store,
        &suite,
        "put_conversation_pdf_resource",
        &json!({"conversation_id": "conv-src", "resource_id": "pdf-1", "filename": "spec.pdf", "data_base64": "JVBERi0=", "page_count": 3}),
    )
    .unwrap();
    assert_eq!(pdf["resource_id"], "pdf-1");
    let pdf_read = dispatch_command(
        &store,
        &suite,
        "read_conversation_resource",
        &json!({"conversation_id": "conv-src", "resource_id": "pdf-1"}),
    )
    .unwrap();
    assert_eq!(pdf_read["resource"]["filename"], "spec.pdf");
    let pdf_list = dispatch_command(
        &store,
        &suite,
        "list_conversation_resources",
        &json!({"conversation_id": "conv-src"}),
    )
    .unwrap();
    assert_eq!(pdf_list["resources"].as_array().unwrap().len(), 1);
    let pdf_del = dispatch_command(
        &store,
        &suite,
        "delete_conversation_resource",
        &json!({"conversation_id": "conv-src", "resource_id": "pdf-1"}),
    )
    .unwrap();
    assert_eq!(pdf_del["deleted"], true);

    let adm = dispatch_command(
        &store,
        &suite,
        "admit_conversation_user_message",
        &json!({"input": {"conversation_id": "conv-src", "message_id": "m-admit-1", "content": "queued hello"}}),
    )
    .unwrap();
    assert_eq!(adm["admitted"], true);

    let begun = dispatch_command(
        &store,
        &suite,
        "begin_queued_user_turn",
        &json!({"input": {"conversation_id": "conv-src", "command_id": "run-q-1", "core_config": {"model": "gpt-4o"}}}),
    )
    .unwrap();
    assert_eq!(begun["run_id"], "run-q-1");
    let active = dispatch_command(
        &store,
        &suite,
        "active_run",
        &json!({"conversation_id": "conv-src"}),
    )
    .unwrap();
    assert_eq!(active["active"], true);
    assert_eq!(active["run_id"], "run-q-1");

    let exact = dispatch_command(
        &store,
        &suite,
        "load_exact_run_invocation",
        &json!({"conversation_id": "conv-src", "expected_run_id": "run-q-1", "assistant_message_id": "run-q-1:assistant"}),
    )
    .unwrap();
    assert_eq!(exact["run_id"], "run-q-1");
    assert_eq!(exact["invocation_config_digest"].as_str().unwrap().len(), 64);

    let staged = dispatch_command(
        &store,
        &suite,
        "stage_assistant_journal_content",
        &json!({
            "input": {
                "record": {"identity": {"conversation_id": "conv-src", "run_id": "run-q-1", "assistant_message_id": "run-q-1:assistant"}},
                "part_id": "part-1",
                "content": {"content_id": "c-1"},
                "chunks": [{"text": "Hello journal world"}]
            }
        }),
    )
    .unwrap();
    assert_eq!(staged["staged"], true);

    let j_parts = dispatch_command(
        &store,
        &suite,
        "read_assistant_journal_parts",
        &json!({"snapshot": {"identity": {"conversation_id": "conv-src", "run_id": "run-q-1"}, "at": {"revision": "1"}}}),
    )
    .unwrap();
    assert_eq!(j_parts["parts"].as_array().unwrap().len(), 1);

    let j_changes = dispatch_command(
        &store,
        &suite,
        "read_assistant_journal_part_changes",
        &json!({"snapshot": {"identity": {"conversation_id": "conv-src", "run_id": "run-q-1"}, "at": {"revision": "1"}}, "base": {"revision": "0"}}),
    )
    .unwrap();
    assert_eq!(j_changes["changes"].as_array().unwrap().len(), 1);

    let j_range = dispatch_command(
        &store,
        &suite,
        "read_assistant_journal_range",
        &json!({"snapshot": {"identity": {"conversation_id": "conv-src", "run_id": "run-q-1"}}, "part_id": "part-1", "content": {"content_id": "c-1"}, "offset_bytes": "6", "limit_bytes": 7}),
    )
    .unwrap();
    assert_eq!(j_range["text"], "journal");
    assert_eq!(j_range["end_of_content"], false);

    clear_active_run(&store, "conv-src");
}

#[test]
fn daemon_expert_room_fixes_and_mobile_delegated_task_lifecycle() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    let suite = ModulesSuite::new(store.clone(), dir.path().to_path_buf(), None);

    // 1. Expert description preservation + explicit model_override clear + delete_agent alias with nested input
    let exp = dispatch_agent_room_command(
        &store,
        &suite,
        &json!({
            "command": {
                "type": "configure_agent",
                "input": {
                    "id": "exp-1",
                    "name": "Reviewer",
                    "role": "reviewer",
                    "description": "Careful Rust code reviewer",
                    "model_override": "gpt-5"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(exp["expert"]["description"], "Careful Rust code reviewer");
    assert_eq!(exp["expert"]["model_override"], "gpt-5");

    let cleared = dispatch_agent_room_command(
        &store,
        &suite,
        &json!({
            "command": {
                "type": "configure_agent",
                "input": {
                    "id": "exp-1",
                    "model_override": null
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(cleared["expert"]["description"], "Careful Rust code reviewer");
    assert!(cleared["expert"]["model_override"].is_null());

    let del_agent = dispatch_agent_room_command(
        &store,
        &suite,
        &json!({
            "command": {
                "type": "delete_agent",
                "input": {"id": "exp-1", "expert_id": "exp-1"}
            }
        }),
    )
    .unwrap();
    assert_eq!(del_agent["deleted"], true);

    // 2. Room session_id attach + detach/null clear + delete_room with nested input
    let _ = dispatch_agent_room_command(
        &store,
        &suite,
        &json!({"command": {"type": "create_room", "input": {"id": "rm-2", "name": "Ops"}}}),
    )
    .unwrap();
    let attached = dispatch_agent_room_command(
        &store,
        &suite,
        &json!({"command": {"type": "update_room", "input": {"id": "rm-2", "conversation_id": "conv-attach-1"}}}),
    )
    .unwrap();
    assert_eq!(attached["room"]["session_id"], "conv-attach-1");
    let detached = dispatch_agent_room_command(
        &store,
        &suite,
        &json!({"command": {"type": "update_room", "input": {"id": "rm-2", "detach": true}}}),
    )
    .unwrap();
    assert_eq!(detached["room"]["session_id"], "");
    let attached2 = dispatch_agent_room_command(
        &store,
        &suite,
        &json!({"command": {"type": "update_room", "input": {"id": "rm-2", "session_id": "conv-attach-2"}}}),
    )
    .unwrap();
    assert_eq!(attached2["room"]["session_id"], "conv-attach-2");
    let cleared_null = dispatch_agent_room_command(
        &store,
        &suite,
        &json!({"command": {"type": "update_room", "input": {"id": "rm-2", "session_id": null}}}),
    )
    .unwrap();
    assert_eq!(cleared_null["room"]["session_id"], "");
    let del_room = dispatch_agent_room_command(
        &store,
        &suite,
        &json!({"command": {"type": "delete_room", "input": {"id": "rm-2", "room_id": "rm-2"}}}),
    )
    .unwrap();
    assert_eq!(del_room["deleted"], true);

    // 3. All 6 mobile delegated task commands
    let captured = dispatch_command(
        &store,
        &suite,
        "capture_mobile_delegation_configuration",
        &json!({
            "config": {
                "session_id": "parent-sess-1",
                "tools": [{"function": {"name": "read_file"}}],
                "host_catalog": {"host_id": "ios-host-1"},
                "context_window": {"context_window_tokens": 128000, "reserved_output_tokens": 4096},
                "turn_identity": {
                    "host_invocation": {"primary": {}, "generation": {}, "output_admission": {}},
                    "host_dependencies": {},
                    "model_capability": {}
                }
            },
            "settings": {"project_root": "ios-device"}
        }),
    )
    .unwrap();
    assert_eq!(
        captured["turn_identity"]["delegation"]["project_root"],
        "ios-device"
    );

    let prepared = dispatch_command(
        &store,
        &suite,
        "prepare_mobile_delegated_task",
        &json!({
            "input": {
                "request": {
                    "conversation_id": "parent-sess-1",
                    "payload": {
                        "call_id": "call-spawn-1",
                        "capability_id": "spawn_agent",
                        "input": {"prompt": "Inspect logs", "title": "Log Inspector"}
                    }
                },
                "now_ms": 1000
            }
        }),
    )
    .unwrap();
    let task_id = prepared["prepared"]["task"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(prepared["response"]["type"], "task_promoted");
    assert_eq!(prepared["prepared"]["task"]["owner_session_id"], "parent-sess-1");

    let child_cfg = dispatch_command(
        &store,
        &suite,
        "load_mobile_task_child_configuration",
        &json!({"task_id": task_id}),
    )
    .unwrap();
    assert_eq!(
        child_cfg["session_id"],
        prepared["prepared"]["task"]["child_session_id"]
    );
    assert_eq!(child_cfg["host_catalog"]["host_id"], "ios-host-1");

    let admitted = dispatch_command(
        &store,
        &suite,
        "admit_mobile_task_child",
        &json!({
            "input": {
                "task_id": task_id,
                "parent_conversation_id": "parent-sess-1",
                "worker": {"pid": 42},
                "now_ms": 2000,
                "lease_until_ms": 62000,
                "candidate_config": child_cfg,
                "current_tools": []
            }
        }),
    )
    .unwrap();
    assert_eq!(admitted["task_id"], task_id);
    assert_eq!(admitted["attempt"], 1);
    assert_eq!(admitted["cancellation_only"], false);
    assert_eq!(
        admitted["invocation"]["core_session_id"],
        child_cfg["session_id"]
    );

    let guided = dispatch_command(
        &store,
        &suite,
        "prepare_mobile_task_guidance",
        &json!({
            "input": {
                "request": {
                    "payload": {
                        "call_id": "call-guide-1",
                        "input": {"task_id": task_id, "message": "Check stderr too"}
                    }
                },
                "now_ms": 2500
            }
        }),
    )
    .unwrap();
    assert_eq!(guided["type"], "tool_completed");
    assert_eq!(guided["outcome"], "succeeded");

    let observed = dispatch_command(
        &store,
        &suite,
        "observe_mobile_task",
        &json!({
            "input": {
                "request": {
                    "payload": {
                        "call_id": "call-obs-1",
                        "capability_id": "wait_task",
                        "input": {"task_id": task_id, "wait_ms": 3000}
                    }
                },
                "now_ms": 3000
            }
        }),
    )
    .unwrap();
    assert_eq!(observed["terminal"], false);
    assert_eq!(observed["wait_ms"], 3000);
    assert_eq!(observed["response"]["type"], "tool_completed");
}

#[test]
fn daemon_v2_cross_platform_canonical_contracts_hold() {
    let dir = std::env::temp_dir().join(format!("nola-v2-contracts-{}", new_id()));
    let _ = std::fs::create_dir_all(&dir);
    let db = dir.join("nola.db");
    let store = Store::open(&db).unwrap();
    let suite = ModulesSuite::new(store.clone(), dir.clone(), Some(dir.clone()));

    // 1. Conversation creation, case-insensitive lookup, load_conversation vs projection_v1 parts, UTF-8 export slicing
    let conv_id = "11111111-2222-3333-4444-555555555555";
    let _ = dispatch_command(
        &store,
        &suite,
        "create_conversation",
        &json!({"id": conv_id, "title": "V2 Contracts"}),
    )
    .unwrap();
    let utf8_text = "你好世界🌍".repeat(20);
    let _ = dispatch_command(
        &store,
        &suite,
        "import_conversation_transcript",
        &json!({
            "conversation_id": conv_id,
            "messages": [{"id": "m-1", "role": "user", "parts": [{"id": "p-1", "kind": "text", "content_text": utf8_text}]}]
        }),
    )
    .unwrap();
    let loaded = dispatch_command(
        &store,
        &suite,
        "load_conversation",
        &json!({"conversation_id": conv_id.to_uppercase()}),
    )
    .unwrap();
    assert_eq!(loaded["messages"][0]["id"], "m-1");
    assert!(loaded["messages"][0]["parts"][0].get("id").is_some());
    let proj = dispatch_command(
        &store,
        &suite,
        "load_conversation_projection_v1",
        &json!({"conversation_id": conv_id}),
    )
    .unwrap();
    let p0 = proj["messages"][0]["parts"][0].as_object().unwrap();
    assert_eq!(p0.len(), 5);
    assert!(p0.contains_key("part_id") && p0.contains_key("markdown") && !p0.contains_key("id"));

    let page = dispatch_command(
        &store,
        &suite,
        "load_conversation_export_page",
        &json!({"conversation_id": conv_id}),
    )
    .unwrap();
    assert_eq!(page["conversation"]["id"], conv_id);
    let range = dispatch_command(
        &store,
        &suite,
        "read_conversation_export_range",
        &json!({"conversation_id": conv_id, "message_id": "m-1", "part_id": "p-1", "field": "content_text", "offset_bytes": 0, "limit_bytes": 10}),
    )
    .unwrap();
    assert_eq!(range["end_of_content"], false);
    assert!(range["text"].as_str().unwrap().len() <= 10);

    // 2. Capabilities + strict core_config validation + strict 2-key load_run_identity + permission projection control owner
    let ui_cap = dispatch_command(&store, &suite, "read_ui_capability", &json!({})).unwrap();
    assert_eq!(ui_cap["descriptor"]["id"], "render_ui");
    let peer_cap = dispatch_command(&store, &suite, "read_expert_exchange_capability", &json!({})).unwrap();
    assert_eq!(peer_cap["tool"]["function"]["name"], "delegate_expert");
    assert!(dispatch_command(
        &store,
        &suite,
        "begin_assistant_turn",
        &json!({"input": {"conversation_id": conv_id, "core_config": {"host_id": "bad"}, "submission": {"type": "user_input"}}}),
    )
    .is_err());
    let begun = dispatch_command(
        &store,
        &suite,
        "begin_assistant_turn",
        &json!({"input": {"conversation_id": conv_id, "core_config": {"session_id": "s1", "model": "m1"}, "submission": {"type": "user_input", "submission_id": "run-1"}}}),
    )
    .unwrap();
    assert_eq!(begun["execution"]["control"]["session_id"], "s1");
    let ident = dispatch_command(&store, &suite, "load_run_identity", &json!({"run_id": "run-1"})).unwrap();
    assert_eq!(ident.as_object().unwrap().len(), 2);
    assert_eq!(ident["run_id"], "run-1");
    let perm = dispatch_command(
        &store,
        &suite,
        "load_conversation_permission_projection_v1",
        &json!({"conversation_id": conv_id, "run_id": "run-1"}),
    )
    .unwrap();
    assert_eq!(perm["session_id"], "s1");
    assert_eq!(perm["permissions"]["revision"], 1);

    // 3. In-place resend idempotency, fabricated compaction lineage rejection, compaction tight window rejection
    let resend = dispatch_command(
        &store,
        &suite,
        "prepare_in_place_resend",
        &json!({"input": {"operation_id": "op-1", "conversation_id": conv_id, "message_id": "m-1", "edited_text": "edited"}}),
    )
    .unwrap();
    assert_eq!(resend["run_id"], "op-1");
    assert!(resend.get("conversation_id").is_none());
    assert!(dispatch_command(
        &store,
        &suite,
        "context",
        &json!({"command": {"type": "commit_maintenance", "input": {"source": {"conversation_id": conv_id}, "summary": "bad", "original_input": {"messages": [{"id": "fabricated-id"}]}}}}),
    )
    .is_err());
    assert!(dispatch_command(
        &store,
        &suite,
        "step_compaction_summary",
        &json!({"step": {"type": "start", "messages": [{"role": "user", "content": "hi"}], "context_window_tokens": 8000, "output_tokens": 4096, "target_tokens": 1024}}),
    )
    .is_err());

    // 4. Skill sync idempotency, hidden visibility CAS, tombstone non-resurrection
    let sync_in = json!({"command": {"type": "synchronize_built_ins", "inputs": [{"id": "s-bi", "source": "built_in", "display_name": "BuiltIn", "summary": "sum", "category": "general", "instructions_ref": "builtin://x", "instructions_text": "body", "required_tools": [], "default_mode": "inline", "version": "1.0.0"}]}});
    let s1 = dispatch_command(&store, &suite, "skill", &sync_in).unwrap();
    let s2 = dispatch_command(&store, &suite, "skill", &sync_in).unwrap();
    assert_eq!(s1[0]["revision"], 1);
    assert_eq!(s2[0]["revision"], 1);
    let removed = dispatch_command(
        &store,
        &suite,
        "skill",
        &json!({"command": {"type": "remove_built_in", "id": "s-bi", "expected_revision": 1}}),
    )
    .unwrap();
    assert_eq!(removed["record"]["enabled"], false);
    assert!(dispatch_command(
        &store,
        &suite,
        "skill",
        &json!({"command": {"type": "restore_built_ins", "expected_revisions": {"s-bi": 2}}}),
    )
    .is_err());

    // 5. Memory snapshot conflict & read-back parity + auxiliary observe_source receipt unwrap
    let m1 = dispatch_command(
        &store,
        &suite,
        "memory",
        &json!({"command": {"type": "replace", "expected": [], "candidates": [{"record": {"id": "mem-1", "content": "alpha", "kind": "preference"}}]}}),
    )
    .unwrap();
    let m_list = dispatch_command(&store, &suite, "memory", &json!({"command": {"type": "list"}})).unwrap();
    assert_eq!(m1[0]["created_at_ms"], m_list["items"][0]["created_at_ms"]);
    assert!(dispatch_command(
        &store,
        &suite,
        "memory",
        &json!({"command": {"type": "replace", "expected": [], "candidates": []}}),
    )
    .is_err());

    let obs = dispatch_command(
        &store,
        &suite,
        "auxiliary_invocation",
        &json!({"command": {"type": "observe_source", "local_owner": {"kind": "account", "account_id": "a1"}, "observation": {"source_host_id": "h1", "operation_id": "op1", "attempt_id": "op1:physical", "receipt": {"operation": {"id": "op1", "scenario": "event_summary", "state": "running", "revision": 2, "attempts_started": 1}, "attempt": {"id": "op1:physical", "state": "running", "revision": 1}, "dispatch_permitted": false}}}}),
    )
    .unwrap();
    assert_eq!(obs["attempt"]["id"], "op1:physical");
    let slot = dispatch_command(
        &store,
        &suite,
        "load_auxiliary_source_slot",
        &json!({"local_owner": {"kind": "account", "account_id": "a1"}, "scenario": "event_summary"}),
    )
    .unwrap();
    assert_eq!(slot["attempt_id"], "op1:physical");

    // 6. Compaction prepare_maintenance cannot_fit on tight 8K/4K window, expert preview_capture rejection & load_manifest(message_id),
    //    and permission request consumption + resource-scoped allow_conversation grant reuse vs different-file re-prompt.
    let tight_plan = dispatch_command(
        &store,
        &suite,
        "context",
        &json!({"command": {"type": "prepare_maintenance", "source": {"conversation_id": conv_id}, "input": {"mode": "force_summary", "messages": [{"id": "m-1", "role": "user", "content": "a"}, {"id": "m-2", "role": "assistant", "content": "b"}, {"id": "m-3", "role": "user", "content": "c"}], "budget": {"context_window_tokens": 8000, "reserved_output_tokens": 4096, "keep_recent_messages": 1}}}}),
    )
    .unwrap();
    assert_eq!(tight_plan["output"]["action"], "cannot_fit");

    let preview_err = dispatch_command(
        &store,
        &suite,
        "native_expert_host_operation",
        &json!({"operation": {"kind": "preview_capture", "input": {"conversation_id": conv_id}}}),
    )
    .unwrap_err();
    assert_eq!(preview_err.0, "requires_main_agent_turn");
    let missing_manifest = dispatch_command(
        &store,
        &suite,
        "native_expert_host_operation",
        &json!({"operation": {"kind": "load_manifest", "conversation_id": conv_id, "message_id": "m-1"}}),
    )
    .unwrap();
    assert!(missing_manifest.is_null());

    let perm_begun = dispatch_command(
        &store,
        &suite,
        "begin_assistant_turn",
        &json!({"input": {"conversation_id": conv_id, "core_config": {"session_id": "s-perm", "model": "m1", "host_catalog": {"capabilities": {"device_read_file": {"permission_domain": "files_read"}}}}, "submission": {"type": "user_input", "submission_id": "run-perm"}}}),
    )
    .unwrap();
    assert_eq!(perm_begun["execution"]["control"]["session_id"], "s-perm");
    let reqs0 = dispatch_command(&store, &suite, "claim_committed_host_requests", &json!({"input": {"run_id": "run-perm"}})).unwrap();
    let m0_id = reqs0["requests"][0]["request_id"].as_str().unwrap().to_string();
    dispatch_command(
        &store,
        &suite,
        "reduce_and_accept_host_response",
        &json!({"input": {"conversation_id": conv_id, "run_id": "run-perm", "response": {"request_id": m0_id, "sequence": 2, "payload": {"type": "model_completed", "items": [], "tool_calls": [{"call_id": "c1", "name": "device_read_file", "arguments": {"uri": "file:///a.txt"}}]}}}}),
    )
    .unwrap();
    let reqs1 = dispatch_command(&store, &suite, "claim_committed_host_requests", &json!({"input": {"run_id": "run-perm"}})).unwrap();
    assert_eq!(reqs1["requests"].as_array().unwrap().len(), 1);
    assert_eq!(reqs1["requests"][0]["payload"]["type"], "request_permission");
    let perm_id = reqs1["requests"][0]["payload"]["permission_request_id"].as_str().unwrap().to_string();
    dispatch_command(
        &store,
        &suite,
        "reduce_and_commit_active_submission",
        &json!({"input": {"run_id": "run-perm", "submission": {"type": "permission_response", "request_id": perm_id, "decision": "allow_conversation", "grant": {"scope": "conversation"}}}}),
    )
    .unwrap();
    let reqs2 = dispatch_command(&store, &suite, "claim_committed_host_requests", &json!({"input": {"run_id": "run-perm"}})).unwrap();
    assert_eq!(reqs2["requests"].as_array().unwrap().len(), 1);
    assert_eq!(reqs2["requests"][0]["payload"]["type"], "execute_tool");
    let t1_id = reqs2["requests"][0]["request_id"].as_str().unwrap().to_string();
    dispatch_command(
        &store,
        &suite,
        "reduce_and_accept_host_response",
        &json!({"input": {"conversation_id": conv_id, "run_id": "run-perm", "response": {"request_id": t1_id, "sequence": 3, "payload": {"type": "tool_completed", "call_id": "c1"}}}}),
    )
    .unwrap();
    let reqs3 = dispatch_command(&store, &suite, "claim_committed_host_requests", &json!({"input": {"run_id": "run-perm"}})).unwrap();
    let m1_id = reqs3["requests"][0]["request_id"].as_str().unwrap().to_string();
    // Same URI reuses conversation grant without re-prompting; different URI must ask permission again.
    dispatch_command(
        &store,
        &suite,
        "reduce_and_accept_host_response",
        &json!({"input": {"conversation_id": conv_id, "run_id": "run-perm", "response": {"request_id": m1_id, "sequence": 4, "payload": {"type": "model_completed", "items": [], "tool_calls": [{"call_id": "c2", "name": "device_read_file", "arguments": {"uri": "file:///a.txt"}}, {"call_id": "c3", "name": "device_read_file", "arguments": {"uri": "file:///b.txt"}}]}}}}),
    )
    .unwrap();
    let reqs4 = dispatch_command(&store, &suite, "claim_committed_host_requests", &json!({"input": {"run_id": "run-perm"}})).unwrap();
    let types: Vec<&str> = reqs4["requests"].as_array().unwrap().iter().filter_map(|r| r.pointer("/payload/type").and_then(|v| v.as_str())).collect();
    assert_eq!(types, vec!["execute_tool", "request_permission"]);

    // 7. Assistant journal publication, snapshot/parts/range reader, message admissions, terminal seal, and expired lease recovery.
    let j_sub_id = json!({
        "source_host_id": "h-j", "client_id": "c-j", "conversation_id": conv_id,
        "submission_id": "run-j", "user_message_id": "u-j", "assistant_message_id": "a-j"
    });
    let reg = dispatch_command(
        &store,
        &suite,
        "register_source_submission",
        &json!({"identity": j_sub_id, "request": {"conversationId": conv_id}}),
    )
    .unwrap();
    assert_eq!(reg["created"], true);
    let _ = dispatch_command(
        &store,
        &suite,
        "begin_assistant_turn",
        &json!({"input": {"conversation_id": conv_id, "user_message_id": "u-j", "assistant_message_id": "a-j", "user_text": "hello", "core_config": {"session_id": "s-j", "model": "m1", "permission_rules": {"rules": [{"capability_id": "fixture_read", "decision": "ask"}]}}, "submission": {"type": "user_input", "submission_id": "run-j"}}}),
    )
    .unwrap();
    let adm_head = dispatch_command(
        &store,
        &suite,
        "local_run_message_admission_head",
        &json!({"conversation_id": conv_id, "run_id": "run-j"}),
    )
    .unwrap();
    assert_eq!(adm_head["sequence"], "1");
    let rec_state = dispatch_command(
        &store,
        &suite,
        "run_message_admission_reception_state",
        &json!({"source_host_id": "h-j", "through": adm_head}),
    )
    .unwrap();
    let adm_page = dispatch_command(
        &store,
        &suite,
        "read_run_message_admissions",
        &json!({"after": rec_state, "through": adm_head}),
    )
    .unwrap();
    assert_eq!(adm_page["complete"], true);
    let _ = dispatch_command(
        &store,
        &suite,
        "receive_run_message_admissions",
        &json!({"source_host_id": "h-j", "page": adm_page}),
    )
    .unwrap();
    // Model emits reasoning delta, then rule-gated permission, then after lease expiry recovers with fresh delivery, resolves permission, completes tool & answer, and seals.
    let j_reqs0 = dispatch_command(
        &store,
        &suite,
        "claim_committed_host_requests",
        &json!({"input": {"run_id": "run-j", "worker_generation": "g1", "lease_owner": "o1", "claimed_at_ms": 1000, "lease_expires_at_ms": 2000}}),
    )
    .unwrap();
    let jm0_id = j_reqs0["requests"][0]["request_id"].as_str().unwrap().to_string();
    assert_eq!(j_reqs0["requests"][0]["attempt_started"], false);
    let _ = dispatch_command(
        &store,
        &suite,
        "reduce_and_accept_host_response",
        &json!({"input": {"conversation_id": conv_id, "run_id": "run-j", "response": {"request_id": jm0_id, "payload": {"type": "model_attempt_started", "provider": "local", "model": "m1", "reason": "initial"}}}}),
    )
    .unwrap();
    let j_reqs0_again = dispatch_command(
        &store,
        &suite,
        "claim_committed_host_requests",
        &json!({"input": {"run_id": "run-j", "worker_generation": "g1", "lease_owner": "o1", "claimed_at_ms": 1005, "lease_expires_at_ms": 2005}}),
    )
    .unwrap();
    assert_eq!(j_reqs0_again["requests"][0], j_reqs0["requests"][0]);
    let jm0_r_pid = j_reqs0["requests"][0]["payload"]["reasoning_part_id"].as_str().unwrap().to_string();
    let jm0_t_pid = j_reqs0["requests"][0]["payload"]["text_part_id"].as_str().unwrap().to_string();
    let _ = dispatch_command(
        &store,
        &suite,
        "reduce_and_accept_host_response",
        &json!({"input": {"conversation_id": conv_id, "run_id": "run-j", "response": {"request_id": jm0_id, "payload": {"type": "model_delta", "channel": "reasoning", "part_id": jm0_r_pid, "delta": "thinking"}}}}),
    )
    .unwrap();
    let perm_tr = dispatch_command(
        &store,
        &suite,
        "reduce_and_accept_host_response",
        &json!({"input": {"conversation_id": conv_id, "run_id": "run-j", "response": {"request_id": jm0_id, "payload": {"type": "model_completed", "items": [], "tool_calls": [{"call_id": "jc1", "name": "fixture_read", "arguments": {}}]}}}}),
    )
    .unwrap();
    assert_eq!(perm_tr["control"]["turn_status"], "awaiting_permission");
    let awaiting_proj = dispatch_command(
        &store,
        &suite,
        "load_conversation_projection_v1",
        &json!({"conversation_id": conv_id}),
    )
    .unwrap();
    assert_eq!(awaiting_proj["active_run"]["state"], "waiting_for_permission");
    assert!(awaiting_proj["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["message_id"] == "a-j" && m["state"] == "running"));
    let j_preqs = dispatch_command(
        &store,
        &suite,
        "claim_committed_host_requests",
        &json!({"input": {"run_id": "run-j", "worker_generation": "g1", "lease_owner": "o1", "claimed_at_ms": 1000, "lease_expires_at_ms": 2000}}),
    )
    .unwrap();
    assert_eq!(j_preqs["requests"][0]["payload"]["type"], "request_permission");
    assert_eq!(j_preqs["requests"][0]["payload"]["allow_conversation"], true);
    let inspected_perm = dispatch_command(
        &store,
        &suite,
        "inspect_mobile_permission_request",
        &json!({"request": j_preqs["requests"][0], "now_ms": 1500}),
    )
    .unwrap();
    assert_eq!(inspected_perm["conversation_id"], conv_id);
    assert_eq!(inspected_perm["run_id"], "run-j");
    assert_eq!(inspected_perm["payload"], j_preqs["requests"][0]["payload"]);
    let exact_j = dispatch_command(
        &store,
        &suite,
        "load_exact_run_invocation",
        &json!({"conversation_id": conv_id, "expected_run_id": "run-j", "assistant_message_id": "a-j"}),
    )
    .unwrap();
    let auth_receipt = dispatch_command(
        &store,
        &suite,
        "validate_mobile_run_recovery_authority",
        &json!({
            "conversation_id": conv_id,
            "expected_run_id": "run-j",
            "assistant_message_id": "a-j",
            "expected_invocation_config_digest": exact_j["invocation_config_digest"]
        }),
    )
    .unwrap();
    assert_eq!(auth_receipt.as_object().unwrap().len(), 4);
    assert_eq!(auth_receipt["run_id"], "run-j");
    let perm_proj_j = dispatch_command(
        &store,
        &suite,
        "load_conversation_permission_projection_v1",
        &json!({"conversation_id": conv_id, "run_id": "run-j"}),
    )
    .unwrap();
    let pend0 = &perm_proj_j["permissions"]["pending"][0];
    assert_eq!(pend0["allow_conversation"], true);
    assert!(pend0["permission_domain"].is_string() && pend0["rationale"].is_string() && pend0["evaluation"].is_object());
    let sched = dispatch_command(
        &store,
        &suite,
        "load_runtime_effect_work_schedule",
        &json!({"input": {"conversation_ids": [conv_id], "now_ms": 3000}}),
    )
    .unwrap();
    assert_eq!(sched["runs"][0]["reason"], "recover_expired_lease");
    let exec_st = dispatch_command(
        &store,
        &suite,
        "load_host_execution_state",
        &json!({"conversation_id": conv_id}),
    )
    .unwrap();
    assert!(exec_st["control"]["revision"].as_u64().unwrap() >= 1);
    assert_eq!(exec_st["control"]["turn_status"], "awaiting_permission");
    let recov = dispatch_command(
        &store,
        &suite,
        "recover_expired_run_deliveries",
        &json!({"conversation_id": conv_id, "expected_run_id": "run-j", "interrupted_at_ms": 3000}),
    )
    .unwrap();
    assert_eq!(recov["recovered_count"], 1);
    let j_preqs2 = dispatch_command(
        &store,
        &suite,
        "claim_committed_host_requests",
        &json!({"input": {"run_id": "run-j", "worker_generation": "g2", "lease_owner": "o2", "claimed_at_ms": 3001, "lease_expires_at_ms": 5000}}),
    )
    .unwrap();
    let jp_req_id = j_preqs2["requests"][0]["request_id"].as_str().unwrap().to_string();
    let jp_perm_id = j_preqs2["requests"][0]["payload"]["permission_request_id"].as_str().unwrap().to_string();
    let _ = dispatch_command(
        &store,
        &suite,
        "reduce_and_accept_host_response",
        &json!({"input": {"conversation_id": conv_id, "run_id": "run-j", "response": {"request_id": jp_req_id, "payload": {"type": "permission_resolved", "request_id": jp_perm_id, "resolution": {"decision": "allow_once"}}}}}),
    )
    .unwrap();
    let jt_reqs = dispatch_command(&store, &suite, "claim_committed_host_requests", &json!({"input": {"run_id": "run-j"}})).unwrap();
    assert_eq!(jt_reqs["requests"][0]["payload"]["permission_profile"], "cautious");
    let jt_id = jt_reqs["requests"][0]["request_id"].as_str().unwrap().to_string();
    let _ = dispatch_command(
        &store,
        &suite,
        "reduce_and_accept_host_response",
        &json!({"input": {"conversation_id": conv_id, "run_id": "run-j", "response": {"request_id": jt_id, "payload": {"type": "tool_completed", "call_id": "jc1", "output": {"text": "ok"}, "artifacts": []}}}}),
    )
    .unwrap();
    let jm1_reqs = dispatch_command(&store, &suite, "claim_committed_host_requests", &json!({"input": {"run_id": "run-j"}})).unwrap();
    let jm1_id = jm1_reqs["requests"][0]["request_id"].as_str().unwrap().to_string();
    let _ = dispatch_command(
        &store,
        &suite,
        "reduce_and_accept_host_response",
        &json!({"input": {"conversation_id": conv_id, "run_id": "run-j", "response": {"request_id": jm1_id, "payload": {"type": "model_committed_delta", "part_id": jm0_t_pid, "delta": "Final answer"}}}}),
    )
    .unwrap();
    let term_tr = dispatch_command(
        &store,
        &suite,
        "reduce_and_accept_host_response",
        &json!({"input": {"conversation_id": conv_id, "run_id": "run-j", "response": {"request_id": jm1_id, "payload": {"type": "model_completed", "items": [{"type": "message", "id": jm0_t_pid, "role": "assistant", "content": "Final answer"}], "tool_calls": []}}}}),
    )
    .unwrap();
    assert_eq!(term_tr["control"]["turn_status"], "completed");
    let j_heads = dispatch_command(
        &store,
        &suite,
        "local_run_assistant_journal_heads",
        &json!({"conversation_id": conv_id, "run_id": "run-j", "limit": 64}),
    )
    .unwrap();
    let snap = dispatch_command(
        &store,
        &suite,
        "open_assistant_journal_snapshot",
        &json!({"at": j_heads[0]}),
    )
    .unwrap();
    let j_parts = dispatch_command(
        &store,
        &suite,
        "read_assistant_journal_parts",
        &json!({"snapshot": snap, "max_records": 64}),
    )
    .unwrap();
    assert!(j_parts["parts"].as_array().unwrap().len() >= 3);
    let text_part = j_parts["parts"].as_array().unwrap().iter().find(|p| p["metadata"]["kind"] == "text").unwrap();
    let j_range = dispatch_command(
        &store,
        &suite,
        "read_assistant_journal_range",
        &json!({"snapshot": snap, "part_id": text_part["part_id"], "content": text_part["bodies"][0], "offset_bytes": "0", "limit_bytes": 1024}),
    )
    .unwrap();
    assert_eq!(j_range["text"], "Final answer");
    let seal = dispatch_command(
        &store,
        &suite,
        "local_assistant_run_terminal_seal",
        &json!({"conversation_id": conv_id, "run_id": "run-j"}),
    )
    .unwrap();
    assert_eq!(seal["protocol_version"], 1);
    assert_eq!(seal["result"]["value"]["kind"], "answered");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn daemon_p0_p1_macos_ios_android_contracts_work() {
    let store = Store::open_in_memory().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let suite = ModulesSuite::new(store.clone(), dir.path().to_path_buf(), None);

    // Create conversation
    let conv_id = "conv-contract-1";
    let _ = dispatch_command(
        &store,
        &suite,
        "create_conversation",
        &json!({"conversation_id": conv_id, "title": "Initial"}),
    )
    .unwrap();

    // 2a. list_open_user_input_waits returns top-level JSON array
    let waits = dispatch_command(
        &store,
        &suite,
        "list_open_user_input_waits",
        &json!({"conversation_id": conv_id}),
    )
    .unwrap();
    assert!(waits.is_array());

    // 2c. count_checkpoints returns top-level integer
    let cp_count = dispatch_command(
        &store,
        &suite,
        "context",
        &json!({"command": {"type": "count_checkpoints", "conversation_id": conv_id}}),
    )
    .unwrap();
    assert_eq!(cp_count, json!(0));

    // 2b. projection_snapshot (conversation + conversations), projection_cursor_wait, projection_event_suffix
    let snap = dispatch_command(
        &store,
        &suite,
        "projection_snapshot",
        &json!({
            "scope": {"type": "conversation", "conversation_id": conv_id},
            "page": {"offset": 0, "limit": 64}
        }),
    )
    .unwrap();
    assert_eq!(snap["records"][0]["entity_type"], "conversation_projection");
    assert_eq!(snap["records"][0]["entity_id"], conv_id);
    assert_eq!(snap["page"]["has_more"], false);
    let init_seq = snap["cursor"]["sequence"].as_u64().unwrap();

    let list_snap = dispatch_command(
        &store,
        &suite,
        "projection_snapshot",
        &json!({"scope": {"type": "conversations"}}),
    )
    .unwrap();
    assert_eq!(list_snap["records"][0]["entity_type"], "conversation_summary");
    assert_eq!(list_snap["records"][0]["payload"]["id"], conv_id);

    let _ = dispatch_command(
        &store,
        &suite,
        "rename_conversation",
        &json!({"conversation_id": conv_id, "title": "Renamed Live"}),
    )
    .unwrap();

    let waited = dispatch_command(
        &store,
        &suite,
        "projection_cursor_wait",
        &json!({
            "after": {"scope": {"type": "conversation", "conversation_id": conv_id}, "sequence": init_seq},
            "wait_ms": 50
        }),
    )
    .unwrap();
    assert!(waited["sequence"].as_u64().unwrap() > init_seq);

    let suffix = dispatch_command(
        &store,
        &suite,
        "projection_event_suffix",
        &json!({
            "scope": {"type": "conversation", "conversation_id": conv_id},
            "after": {"scope": {"type": "conversation", "conversation_id": conv_id}, "sequence": init_seq},
            "limit": 256
        }),
    )
    .unwrap();
    let events = suffix["events"].as_array().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["mutation"]["type"], "conversation");
    assert_eq!(events[0]["mutation"]["mutation"]["type"], "set_title");
    assert_eq!(events[0]["mutation"]["mutation"]["title"], "Renamed Live");
    assert_eq!(suffix["next_cursor"]["sequence"], events[0]["cursor"]["sequence"]);

    // 3. Missing daemon RPCs
    let room_null = dispatch_command(
        &store,
        &suite,
        "reconcile_room_routing",
        &json!({"room_id": "r1", "submission_id": "missing", "at_ms": 100}),
    )
    .unwrap();
    assert!(room_null.is_null());

    let owner = json!({"conversation_id": conv_id, "message_id": "m1", "part_id": "p1"});
    let deriv = dispatch_command(
        &store,
        &suite,
        "read_conversation_part_derivation",
        &json!({"owner": owner, "expected_conversation_revision": 2}),
    )
    .unwrap();
    assert!(deriv.is_null());

    store
        .set_kv(
            &format!("tool_source_body:{conv_id}:m1:p1:item-1"),
            &json!("hello tool source"),
        )
        .unwrap();
    let ts_range = dispatch_command(
        &store,
        &suite,
        "read_conversation_tool_source_range",
        &json!({
            "owner": owner,
            "source_item_id": "item-1",
            "expected_conversation_revision": 2,
            "offset_bytes": 0,
            "limit_bytes": 5
        }),
    )
    .unwrap();
    assert_eq!(ts_range["text"], "hello");
    assert_eq!(ts_range["next_offset_bytes"], 5);
    assert_eq!(ts_range["total_utf8_bytes"], 17);
    assert_eq!(ts_range["end_of_content"], false);

    // Terminal reception lifecycle
    let seal = json!({
        "protocol_version": 1,
        "identity": {"conversation_id": conv_id, "run_id": "run-t1", "assistant_message_id": "a1", "source_store_epoch": "ep1"},
        "entry_count": "1",
        "seal_digest": "a".repeat(64)
    });
    let r1 = dispatch_command(
        &store,
        &suite,
        "begin_receive_assistant_run_terminal",
        &json!({"source_host_id": "host-1", "seal": seal}),
    )
    .unwrap();
    assert_eq!(r1["manifest_complete"], false);
    assert_eq!(r1["committed"], false);
    let r2 = dispatch_command(
        &store,
        &suite,
        "stage_assistant_run_terminal_manifest",
        &json!({"source_host_id": "host-1", "page": {"seal": seal, "after_entry": "0", "next_entry": "1", "complete": true, "entries": []}}),
    )
    .unwrap();
    assert_eq!(r2["manifest_complete"], true);
    let r3 = dispatch_command(
        &store,
        &suite,
        "commit_received_assistant_run_terminal",
        &json!({"source_host_id": "host-1", "seal": seal}),
    )
    .unwrap();
    assert_eq!(r3["committed"], true);

    // list_conversation_auxiliary_sources + runtime_task list_all
    let aux = dispatch_command(
        &store,
        &suite,
        "list_conversation_auxiliary_sources",
        &json!({"conversation_id": conv_id, "limit": 16}),
    )
    .unwrap();
    assert!(aux["references"].is_array());
    let all_tasks = dispatch_command(
        &store,
        &suite,
        "runtime_task",
        &json!({"command": {"type": "list_all"}}),
    )
    .unwrap();
    assert!(all_tasks.is_array());

    // Agent command queue lease cycle
    store
        .set_kv(
            &format!("agent_cmd_queue:{conv_id}"),
            &json!([{"command_id": "cmd-1", "payload_json": "{\"type\":\"test\"}"}]),
        )
        .unwrap();
    let claimed = dispatch_command(
        &store,
        &suite,
        "claim_next_agent_command",
        &json!({"session_id": conv_id, "lease_owner": "w1", "now_ms": 1000, "lease_until_ms": 5000}),
    )
    .unwrap();
    assert_eq!(claimed["command_id"], "cmd-1");
    assert_eq!(claimed["delivery_attempt"], 1);
    let rel = dispatch_command(
        &store,
        &suite,
        "release_agent_command_lease",
        &json!({"session_id": conv_id, "command_id": "cmd-1", "lease_owner": "w1"}),
    )
    .unwrap();
    assert_eq!(rel["released"], true);

    // UI tool, wake plan, task cancellation strict 2-key receipt, process owners, guidance, admission
    let ui_res = dispatch_command(
        &store,
        &suite,
        "execute_ui_tool",
        &json!({"input": {"effect_id": "e1", "request_id": "r1", "delivery": {}, "now_ms": 1000}}),
    )
    .unwrap();
    assert_eq!(ui_res["type"], "completed");

    let wake = dispatch_command(
        &store,
        &suite,
        "load_mobile_task_wake_plan",
        &json!({"now_ms": 1000}),
    )
    .unwrap();
    assert!(wake["candidates"].is_array());

    let cancel_eff = dispatch_command(
        &store,
        &suite,
        "execute_task_cancellation_for_effect",
        &json!({"request": {"payload": {"type": "cancel_task", "task_id": "t-1"}}, "now_ms": 1000}),
    )
    .unwrap();
    assert_eq!(cancel_eff.as_object().unwrap().len(), 2);
    assert_eq!(cancel_eff["kind"], "applied");
    assert_eq!(cancel_eff["cancel_requested"], true);

    let plan = dispatch_command(
        &store,
        &suite,
        "inspect_cancellation_process_owners",
        &json!({
            "identity": {"conversation_id": conv_id, "expected_run_id": "r1", "assistant_message_id": "a1"},
            "control": {"session_id": conv_id, "turn_id": "r1", "payload": {"target_effect_id": "eff-9"}}
        }),
    )
    .unwrap();
    let validated = dispatch_command(
        &store,
        &suite,
        "validate_cancellation_process_observations",
        &json!({"plan": plan, "current_process": {}, "observations": []}),
    )
    .unwrap();
    assert_eq!(validated["effect_id"], "eff-9");
    assert_eq!(validated["session_id"], conv_id);

    let guidance = dispatch_command(
        &store,
        &suite,
        "queue_task_guidance",
        &json!({"task_id": "t-1", "message_id": "gm-1", "parent_session_id": conv_id, "parent_turn_id": "r1", "content": "focus", "enqueued_at_epoch_ms": 1234}),
    )
    .unwrap();
    assert_eq!(guidance["status"], "accepted");

    let adm = dispatch_command(
        &store,
        &suite,
        "receive_remote_expert_parent_admission",
        &json!({"expected_source_host_id": "h1", "expected_source_store_epoch": "ep1", "admission": {"source_store_epoch": "ep1", "conversation_id": conv_id, "run_id": "r1"}}),
    )
    .unwrap();
    assert_eq!(adm["admitted"], true);

    let no_ident = dispatch_command(
        &store,
        &suite,
        "read_admitted_remote_run_identity",
        &json!({"source_host_id": "h1", "client_id": "c1", "conversation_id": conv_id, "run_id": "r1", "assistant_message_id": "a1"}),
    )
    .unwrap();
    assert!(no_ident.is_null());
}

#[test]
fn model_connection_discover_enable_and_override_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    let suite = ModulesSuite::new(store.clone(), dir.path().to_path_buf(), None);

    // 1. Save account + connection (matching MacModelConnectionService.create)
    let acct = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({
            "command": {
                "type": "save_account",
                "input": {
                    "id": "acct-openrouter",
                    "display_name": "OpenRouter",
                    "source": "manual",
                    "credential_ref": "account:00000000-0000-4000-8000-000000000001",
                    "state": "ready"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(acct["revision"], 1);

    let conn = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({
            "command": {
                "type": "save_connection",
                "input": {
                    "id": "conn-openrouter",
                    "display_name": "OpenRouter",
                    "base_url": "https://openrouter.ai/api/v1",
                    "protocols": ["chat_completions"],
                    "account_id": "acct-openrouter",
                    "source": "manual",
                    "source_id": Value::Null,
                    "enabled": true
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(conn["revision"], 1);
    assert_eq!(conn["document"]["connection"]["id"], "conn-openrouter");

    // 2. Refresh catalog with AppleModelCatalogDiscovery entries (keyed by `model_id`, not `id`)
    let refreshed = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({
            "command": {
                "type": "refresh_connection_catalog",
                "connection_id": "conn-openrouter",
                "expected_revision": 1,
                "observed_at_ms": 1_795_782_200_000i64,
                "models": [
                    {
                        "model_id": "openrouter/free",
                        "display_name": "OpenRouter Free",
                        "protocol": "chat_completions",
                        "context_window": 128000,
                        "output_limit": 8192,
                        "modalities": ["text"],
                        "reasoning_levels": [],
                        "summary_supported": Value::Null,
                        "execution_capabilities": {
                            "toolCalls": true,
                            "imageInput": false,
                            "reasoning": false,
                            "samplingParameters": true
                        }
                    },
                    {
                        "model_id": "qwen/qwen3-coder:free",
                        "display_name": "Qwen3 Coder",
                        "protocol": "chat_completions",
                        "context_window": Value::Null,
                        "output_limit": Value::Null,
                        "modalities": ["text"],
                        "reasoning_levels": [],
                        "summary_supported": Value::Null,
                        "execution_capabilities": Value::Null
                    }
                ]
            }
        }),
    )
    .unwrap();
    assert_eq!(refreshed["revision"], 2);
    assert_eq!(
        refreshed["document"]["catalog"]["models"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        refreshed["document"]["catalog"]["models"][0]["model_id"],
        "openrouter/free"
    );

    // 2b. Before any model is enabled (zero canonical `model` records), `list_profiles` must
    // return an empty array and MUST NOT fall back to Web-UI camelCase profiles missing canonical fields.
    let before_enable_profiles = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({"command": {"type": "list_profiles"}}),
    )
    .unwrap();
    assert!(
        before_enable_profiles["profiles"].as_array().unwrap().is_empty(),
        "list_profiles must return empty when no canonical model profiles are enabled, got: {}",
        before_enable_profiles["profiles"]
    );

    // 3. Disconnect & reconnect preserves catalog
    let disconnected = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({
            "command": {
                "type": "save_connection",
                "input": {
                    "id": "conn-openrouter",
                    "display_name": "OpenRouter",
                    "base_url": "https://openrouter.ai/api/v1",
                    "protocols": ["chat_completions"],
                    "account_id": "acct-openrouter",
                    "source": "manual",
                    "source_id": Value::Null,
                    "enabled": false
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        disconnected["document"]["catalog"]["models"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let _ = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({
            "command": {
                "type": "save_connection",
                "input": {
                    "id": "conn-openrouter",
                    "display_name": "OpenRouter",
                    "base_url": "https://openrouter.ai/api/v1",
                    "protocols": ["chat_completions"],
                    "account_id": "acct-openrouter",
                    "source": "manual",
                    "source_id": Value::Null,
                    "enabled": true
                }
            }
        }),
    )
    .unwrap();

    // 4. Enable both models -> materializes canonical `model` profiles readable by macOS
    let enabled = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({
            "command": {
                "type": "configure_connection_models",
                "connection_id": "conn-openrouter",
                "enabled_model_ids": [
                    "chat_completions:openrouter/free",
                    "chat_completions:qwen/qwen3-coder:free"
                ],
                "selected_model_id": Value::Null
            }
        }),
    )
    .unwrap();
    assert_eq!(enabled["ok"], true);

    let listed = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({"command": {"type": "list_profiles"}}),
    )
    .unwrap();
    let profiles = listed["profiles"].as_array().unwrap();
    assert_eq!(profiles.len(), 2);
    for p in profiles {
        assert_eq!(p["provider"], "openai-compatible");
        assert_eq!(p["runtime"], "remote");
        assert_eq!(p["enabled"], true);
        let src: Value = serde_json::from_str(p["source_json"].as_str().unwrap()).unwrap();
        assert_eq!(src["schemaVersion"], 1);
        assert_eq!(src["connectionID"], "conn-openrouter");
        assert_eq!(src["wireProtocol"], "chat_completions");
        assert!(src["catalogKey"].as_str().unwrap().starts_with("chat_completions:"));
        assert!(src["executionCapabilities"].is_object());
    }

    // 5. Override capability returns full snapshot and updates profile
    let overridden = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({
            "command": {
                "type": "set_connection_capability_override",
                "connection_id": "conn-openrouter",
                "model_id": "qwen/qwen3-coder:free",
                "protocol": "chat_completions",
                "capabilities": {
                    "toolCalls": false,
                    "imageInput": false,
                    "reasoning": true,
                    "samplingParameters": false
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(overridden["document"]["connection"]["id"], "conn-openrouter");
    assert_eq!(
        overridden["document"]["capability_assessments"][0]["model_id"],
        "qwen/qwen3-coder:free"
    );

    // 6. Uncheck one model -> soft-deletes it from active list_profiles
    let _ = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({
            "command": {
                "type": "configure_connection_models",
                "connection_id": "conn-openrouter",
                "enabled_model_ids": ["chat_completions:openrouter/free"],
                "selected_model_id": Value::Null
            }
        }),
    )
    .unwrap();
    let listed_after = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({"command": {"type": "list_profiles"}}),
    )
    .unwrap();
    assert_eq!(listed_after["profiles"].as_array().unwrap().len(), 1);
    assert_eq!(listed_after["profiles"][0]["model_name"], "openrouter/free");

    // 7. Verify snapshot slimness (no duplicate catalog inside document.connection or top-level)
    let fetched = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({"command": {"type": "get_connection", "id": "conn-openrouter"}}),
    )
    .unwrap();
    assert!(
        fetched["document"]["connection"].get("catalog").is_none(),
        "document.connection must not duplicate catalog payload"
    );
    assert!(
        fetched.get("catalog").is_none(),
        "top-level snapshot must not duplicate catalog outside document"
    );
    assert_eq!(
        fetched["document"]["catalog"]["models"].as_array().unwrap().len(),
        2
    );

    // 8. Save connection with clear_catalog=true resets catalog on endpoint change
    let edited = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({
            "command": {
                "type": "save_connection",
                "clear_catalog": true,
                "input": {
                    "id": "conn-openrouter",
                    "display_name": "OpenRouter Renamed",
                    "base_url": "https://new-gateway.example.invalid/v1",
                    "protocols": ["responses"],
                    "account_id": "acct-openrouter",
                    "source": "manual",
                    "source_id": Value::Null,
                    "enabled": true
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(edited["document"]["connection"]["display_name"], "OpenRouter Renamed");
    assert!(edited["document"]["catalog"].is_null());

    // 9. delete_connection removes connection, soft-deletes linked profiles, cleans applied_config & unreferenced account
    let _ = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({
            "command": {
                "type": "save_account",
                "input": {
                    "id": "acct-openrouter",
                    "display_name": "OpenRouter",
                    "source": "manual",
                    "credential_ref": "account:test-ref",
                    "state": "ready"
                }
            }
        }),
    )
    .unwrap();
    let deleted = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({
            "command": {
                "type": "delete_connection",
                "id": "conn-openrouter",
                "delete_account": true
            }
        }),
    )
    .unwrap();
    assert_eq!(deleted["ok"], true);
    assert_eq!(deleted["deleted"], true);
    assert_eq!(deleted["removed_profiles"], 1);
    assert_eq!(deleted["deleted_account_id"], "acct-openrouter");

    let conns_after_delete = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({"command": {"type": "list_connections"}}),
    )
    .unwrap();
    assert!(
        !conns_after_delete["connections"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["document"]["connection"]["id"] == "conn-openrouter"),
        "deleted connection must not reappear in list_connections"
    );
    let profiles_after_delete = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({"command": {"type": "list_profiles"}}),
    )
    .unwrap();
    assert!(
        !profiles_after_delete["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["model_name"] == "openrouter/free"),
        "profiles belonging to deleted connection must be soft-deleted, got: {}",
        profiles_after_delete["profiles"]
    );
    let accts_after_delete = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({"command": {"type": "list_accounts"}}),
    )
    .unwrap();
    assert!(
        accts_after_delete["accounts"].as_array().unwrap().is_empty(),
        "unreferenced account must be deleted when delete_account=true"
    );
}

#[test]
fn list_activity_includes_waiting_and_automation_runs_and_filters_by_source() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    let suite = ModulesSuite::new(store.clone(), dir.path().to_path_buf(), None);

    let now = now_ms();
    store
        .upsert_session(&Session {
            id: "conv-wait".into(),
            title: "Waiting Chat".into(),
            workspace_root: dir.path().display().to_string(),
            model_connection: None,
            model_id: None,
            created_at_ms: now - 2_000,
            updated_at_ms: now - 1_000,
        })
        .unwrap();
    store
        .set_kv("open_waits:conv-wait", &json!([{"wait_id": "w-1"}]))
        .unwrap();
    store
        .upsert_session(&Session {
            id: "conv-done".into(),
            title: "Done Chat".into(),
            workspace_root: dir.path().display().to_string(),
            model_connection: None,
            model_id: None,
            created_at_ms: now - 3_000,
            updated_at_ms: now - 2_000,
        })
        .unwrap();
    suite
        .upsert_record(
            "automation_run",
            "auto-run-1",
            &json!({
                "id": "auto-run-1",
                "automation_id": "auto-daily",
                "title": "Daily Digest",
                "summary": "Generated digest",
                "status": "completed",
                "scheduled_for_ms": now,
                "updated_at_ms": now
            }),
        )
        .unwrap();

    // Absent needs_response or false MUST NOT filter out waiting conversations, and MUST include automation_run
    let all = dispatch_command(&store, &suite, "list_activity", &json!({"needs_response": false}))
        .unwrap();
    let entries = all["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 3);
    assert!(entries.iter().any(|e| e["id"] == "conv-wait" && e["needs_response"] == true));
    assert!(entries.iter().any(|e| e["id"] == "conv-done" && e["needs_response"] == false));
    assert!(entries.iter().any(|e| e["id"] == "auto-run-1" && e["source"] == "automation"));

    // needs_response == true filters ONLY to waiting items
    let waiting_only =
        dispatch_command(&store, &suite, "list_activity", &json!({"needs_response": true}))
            .unwrap();
    let wait_entries = waiting_only["entries"].as_array().unwrap();
    assert_eq!(wait_entries.len(), 1);
    assert_eq!(wait_entries[0]["id"], "conv-wait");

    // source == "automation" filters strictly to automation runs
    let auto_only = dispatch_command(
        &store,
        &suite,
        "list_activity",
        &json!({"source": "automation"}),
    )
    .unwrap();
    let auto_entries = auto_only["entries"].as_array().unwrap();
    assert_eq!(auto_entries.len(), 1);
    assert_eq!(auto_entries[0]["source"], "automation");
}

#[test]
fn query_automation_occurrences_computes_real_scheduled_timestamps() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    let suite = ModulesSuite::new(store.clone(), dir.path().to_path_buf(), None);

    // 2026-06-01T08:00:00Z (Monday)
    let after_ms = chrono::DateTime::parse_from_rfc3339("2026-06-01T08:00:00Z")
        .unwrap()
        .timestamp_millis();
    let through_ms = after_ms + 65 * 86_400_000;

    // Daily at 09:30 UTC
    let daily = dispatch_command(
        &store,
        &suite,
        "query_automation_occurrences",
        &json!({
            "input": {
                "automation_id": "a-daily",
                "frequency": "daily",
                "time": "09:30",
                "timezone": "UTC",
                "after_ms": after_ms,
                "through_ms": through_ms,
                "limit": 2
            }
        }),
    )
    .unwrap();
    let occs = daily["occurrences"].as_array().unwrap();
    assert_eq!(occs.len(), 2);
    let expected_first = chrono::DateTime::parse_from_rfc3339("2026-06-01T09:30:00Z")
        .unwrap()
        .timestamp_millis();
    assert_eq!(occs[0]["scheduled_for_ms"].as_i64(), Some(expected_first));
    assert_eq!(
        occs[1]["scheduled_for_ms"].as_i64(),
        Some(expected_first + 86_400_000)
    );

    // Weekly on Wednesday (ISO 3) at 10:00 UTC
    let weekly = dispatch_command(
        &store,
        &suite,
        "query_automation_occurrences",
        &json!({
            "input": {
                "automation_id": "a-weekly",
                "frequency": "weekly",
                "time": "10:00",
                "weekday": 3,
                "timezone": "UTC",
                "after_ms": after_ms,
                "through_ms": through_ms,
                "limit": 1
            }
        }),
    )
    .unwrap();
    let expected_wed = chrono::DateTime::parse_from_rfc3339("2026-06-03T10:00:00Z")
        .unwrap()
        .timestamp_millis();
    assert_eq!(
        weekly["occurrences"][0]["scheduled_for_ms"].as_i64(),
        Some(expected_wed)
    );

    // Hourly, Monthly, Interval
    for (freq, extra) in [
        ("hourly", json!({"minute": 15})),
        ("monthly", json!({"day_of_month": 15, "time": "12:00"})),
        ("interval", json!({"interval_minutes": 30})),
    ] {
        let mut inp = json!({
            "automation_id": format!("a-{freq}"),
            "frequency": freq,
            "timezone": "UTC",
            "after_ms": after_ms,
            "through_ms": through_ms,
            "limit": 2
        });
        for (k, v) in extra.as_object().unwrap() {
            inp[k] = v.clone();
        }
        let res = dispatch_command(
            &store,
            &suite,
            "query_automation_occurrences",
            &json!({"input": inp}),
        )
        .unwrap();
        let arr = res["occurrences"].as_array().unwrap();
        assert_eq!(arr.len(), 2, "frequency {freq} should produce 2 occurrences");
        assert!(arr[0]["scheduled_for_ms"].as_i64().unwrap() > after_ms);
        assert!(
            arr[1]["scheduled_for_ms"].as_i64().unwrap()
                > arr[0]["scheduled_for_ms"].as_i64().unwrap()
        );
    }
}

#[test]
fn load_today_facts_and_model_connections_and_draft_ds_override() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    let suite = ModulesSuite::new(store.clone(), dir.path().to_path_buf(), None);

    let now = now_ms();
    store
        .upsert_session(&Session {
            id: "conv-today".into(),
            title: "Morning Sync".into(),
            workspace_root: dir.path().display().to_string(),
            model_connection: None,
            model_id: None,
            created_at_ms: now,
            updated_at_ms: now,
        })
        .unwrap();
    let today = dispatch_command(&store, &suite, "load_today_facts", &json!({})).unwrap();
    assert!(today["date"].is_string());
    assert!(today["facts"].is_array());
    let conv = &today["conversations"][0];
    assert_eq!(conv["conversation_id"], "conv-today");
    assert_eq!(conv["title"], "Morning Sync");
    assert_eq!(conv["updated_at_ms"], now);
    assert_eq!(conv["needs_response"], false);
    assert!(conv["active_run"].is_null());
    assert!(conv["permission_projection"].is_null());
    assert!(conv["open_waits"].is_array());
    assert!(conv["owned_task_waits"].is_array());
    assert!(conv["owned_task_permissions"].is_array());
    assert!(conv["expert_contexts"].is_array());

    // Config-only connection is returned inside canonical document envelope
    let mut cfg = crate::config::AppliedConfig::default();
    cfg.llm.connections.push(crate::config::ConnectionConfig {
        id: "cfg-only".into(),
        endpoint: "https://api.example.invalid/v1".into(),
        protocol: Some("responses".into()),
        provider: None,
        api_key: None,
        api_key_env: None,
        models: vec![crate::config::ModelEntry {
            id: "gpt-5".into(),
            context_window: None,
            max_output_tokens: None,
            temperature: None,
            top_p: None,
            reasoning_effort: None,
            capabilities: None,
        }],
    });
    crate::config::save_applied_config(&store, &cfg).unwrap();

    let listed = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({"command": {"type": "list_connections"}}),
    )
    .unwrap();
    let conns = listed["connections"].as_array().unwrap();
    let cfg_snap = conns
        .iter()
        .find(|c| c["document"]["connection"]["id"] == "cfg-only")
        .expect("cfg-only connection should be wrapped in canonical snapshot");
    assert_eq!(cfg_snap["document"]["format_version"], 1);
    assert_eq!(
        cfg_snap["document"]["connection"]["base_url"],
        "https://api.example.invalid/v1"
    );
    assert_eq!(cfg_snap["document"]["connection"]["enabled"], true);

    let got = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({"command": {"type": "get_connection", "id": "cfg-only"}}),
    )
    .unwrap();
    assert_eq!(got["document"]["format_version"], 1);
    assert_eq!(got["document"]["connection"]["id"], "cfg-only");

    // import_connection also syncs cfg.llm.connections
    let _ = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({
            "command": {
                "type": "import_connection",
                "connection": {
                    "id": "imp-conn",
                    "display_name": "Imported",
                    "base_url": "https://imp.example.invalid/v1",
                    "protocols": ["chat_completions"],
                    "source": "open_code",
                    "enabled": true
                },
                "models": ["llama-3.3"]
            }
        }),
    )
    .unwrap();
    let reloaded = crate::config::load_active_config(&store);
    assert!(reloaded.llm.connections.iter().any(|c| c.id == "imp-conn"));

    // Draft conversation data_source override succeeds without persisted session
    let ds_ov = dispatch_command(
        &store,
        &suite,
        "data_source",
        &json!({
            "command": {
                "type": "set_conversation_override",
                "conversation_id": "unsaved-draft-1",
                "source_id": "ds-1",
                "value": "enabled"
            }
        }),
    )
    .unwrap();
    assert_eq!(ds_ov, json!(true));
}

#[test]
fn resend_expert_host_peer_and_missing_ffi_commands_and_goals_work() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    let suite = ModulesSuite::new(store.clone(), dir.path().to_path_buf(), None);

    // 1. prepare_in_place_resend normalizes experts with non-null participant_id & context_conversation_id
    let in_place = dispatch_command(
        &store,
        &suite,
        "prepare_in_place_resend",
        &json!({
            "input": {
                "operation_id": "op-1",
                "conversation_id": "conv-1",
                "message_id": "m-1",
                "edited_text": "hello",
                "selected_experts": [{"agent_id": "analyst", "participant_id": Value::Null, "context_conversation_id": Value::Null}]
            }
        }),
    )
    .unwrap();
    assert!(in_place["experts"][0]["participant_id"].is_string());
    assert!(in_place["experts"][0]["context_conversation_id"].is_string());

    // 2. prepare_edited_resend & load_prepared_edited_resend return ApplePreparedEditedResend directly
    let edited = dispatch_command(
        &store,
        &suite,
        "prepare_edited_resend",
        &json!({
            "input": {
                "operation_id": "ed-1",
                "source_conversation_id": "conv-src",
                "source_message_id": "msg-1",
                "expected_source_revision": 1,
                "expected_source_message_revision": 1,
                "edited_text": "revised prompt",
                "target_conversation_id": "conv-tgt",
                "title": "Branched",
                "created_at_ms": 1000
            }
        }),
    )
    .unwrap();
    assert_eq!(edited["input"]["target_conversation_id"], "conv-tgt");
    assert_eq!(edited["source"]["conversation_id"], "conv-src");
    assert!(edited["user_message_id"].is_string());
    assert!(edited["assistant_message_id"].is_string());
    assert_eq!(edited["run_id"], "ed-1");
    assert!(edited["admitted_run_id"].is_null());

    let loaded_ed = dispatch_command(
        &store,
        &suite,
        "load_prepared_edited_resend",
        &json!({"conversation_id": "conv-tgt"}),
    )
    .unwrap();
    assert_eq!(loaded_ed["run_id"], "ed-1");

    // 3. host_expert_template_operation: readable_templates + peer ops
    let readable = dispatch_command(
        &store,
        &suite,
        "host_expert_template_operation",
        &json!({"operation": {"kind": "readable_templates", "authority": {"actor": {"kind": "local_user", "id": "local"}, "request_host_id": Value::Null}}}),
    )
    .unwrap();
    assert!(readable["revision"].as_i64().unwrap() >= 1);
    assert!(readable["templates"].is_array());

    for op_val in [
        json!({"kind": "derive_peer_authority", "request": {"host_id": "h1"}}),
        json!({"kind": "peer_directory", "source_context_id": "ctx-1", "source_run_id": "r-1"}),
        json!({"kind": "prepare_peer_task", "exchange_id": "ex-1", "project_root": "/tmp"}),
        json!({"kind": "admit_peer", "input": {"room_id": "room-1", "exchange_id": "ex-1"}}),
        json!({"kind": "load_peer", "room_id": "room-1", "exchange_id": "ex-1"}),
        json!({"kind": "load_peer_for_submission", "room_id": "room-1", "exchange_id": "ex-1"}),
        json!({"kind": "load_peer_for_effect", "request": {"payload": {"input": {"exchange_id": "ex-1"}}}}),
        json!({"kind": "peer_task_action", "request": {"payload": {"input": {"action": "read", "exchange_id": "ex-1"}}}}),
        json!({"kind": "preview_peer", "input": {"target_participant_id": "p-1", "source_context_id": "ctx-1"}}),
        json!({"kind": "clarify_peer", "exchange_id": "ex-1", "text": "clarification"}),
        json!({"kind": "abort_peer_task_preparation", "exchange_id": "ex-1"}),
        json!({"kind": "load_child_instructions", "conversation_id": "conv-1", "expected_run_id": "r-1"}),
        json!({"kind": "load_prepared_resend", "conversation_id": "conv-tgt"}),
    ] {
        let res = dispatch_command(
            &store,
            &suite,
            "host_expert_template_operation",
            &json!({"operation": op_val}),
        );
        assert!(res.is_ok(), "expert host op {:?} failed: {:?}", op_val["kind"], res);
    }

    // 4. Goal execution contracts matching MacApplicationPort
    let goal_prep = dispatch_command(
        &store,
        &suite,
        "prepare_goal_execution",
        &json!({"conversation_id": "conv-g1", "goal_id": "goal-1"}),
    )
    .unwrap();
    assert_eq!(goal_prep["identity"]["conversation_id"], "conv-g1");
    assert_eq!(goal_prep["identity"]["goal_id"], "goal-1");
    assert_eq!(
        goal_prep["template"]["core_config"]["turn_identity"]["host_invocation"]["primary"]["kind"],
        "native"
    );
    let goal_run_id = goal_prep["identity"]["run_id"].as_str().unwrap();
    let goal_owner = dispatch_command(
        &store,
        &suite,
        "load_goal_execution_owner",
        &json!({"conversation_id": "conv-g1", "goal_id": "goal-1", "run_id": goal_run_id}),
    )
    .unwrap();
    assert_eq!(goal_owner["goal_id"], "goal-1");
    assert_eq!(goal_owner["run_id"], goal_run_id);
    assert!(!goal_owner["contract_id"].as_str().unwrap().is_empty());
    assert!(!goal_owner["assistant_message_id"].as_str().unwrap().is_empty());
    assert!(!goal_owner["control_token"].as_str().unwrap().is_empty());

    // 5. Missing local FFI commands & interrupt submission
    store
        .set_kv(
            "committed_run:run-int",
            &json!({
                "conversation_id": "conv-1",
                "session_id": "conv-1",
                "turn_id": "run-int",
                "turn_status": "running",
                "sequence": 1,
                "pending_permissions": [],
                "requests": []
            }),
        )
        .unwrap();
    let interrupted = dispatch_command(
        &store,
        &suite,
        "reduce_and_commit_active_submission",
        &json!({
            "input": {
                "conversation_id": "conv-1",
                "run_id": "run-int",
                "submission": {"type": "interrupt", "cancellation_id": "c-1", "reason": "user_stop"}
            }
        }),
    )
    .unwrap();
    assert_eq!(interrupted["control"]["turn_status"], "cancelled");

    let trace_page = dispatch_command(
        &store,
        &suite,
        "load_exact_run_trace_replay",
        &json!({
            "conversation_id": "conv-1",
            "run_id": "run-int",
            "assistant_message_id": "run-int:assistant",
            "after_sequence": 0,
            "limit": 256
        }),
    )
    .unwrap();
    let keys: HashSet<&str> = trace_page
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        HashSet::from(["trace_id", "through_sequence", "next_sequence", "events"])
    );

    let child_win = dispatch_command(
        &store,
        &suite,
        "load_task_child_conversation_window",
        &json!({
            "destination": {
                "task_id": "t-1",
                "parent_conversation_id": "conv-1",
                "parent_run_id": "r-1",
                "child_conversation_id": "child-conv-1",
                "child_run_id": "cr-1"
            },
            "limit": 30
        }),
    )
    .unwrap();
    assert_eq!(child_win["projection"]["conversation_id"], "child-conv-1");
    assert_eq!(
        child_win["projection"]["execution_ownership"]["kind"],
        "task_child"
    );

    let orphan = dispatch_command(
        &store,
        &suite,
        "recover_observed_orphaned_run",
        &json!({"conversation_id": "conv-1", "expected_run_id": "run-int"}),
    )
    .unwrap();
    assert_eq!(orphan["recovered"], true);

    // Memory worker job lifecycle
    store
        .set_kv(
            "memory_jobs:conv-1",
            &json!([{"id": "mj-1", "conversation_id": "conv-1", "status": "pending"}]),
        )
        .unwrap();
    let jobs = dispatch_command(
        &store,
        &suite,
        "query_claimable_memory_jobs",
        &json!({"input": {"conversation_id": "conv-1", "now_epoch_ms": now_ms()}}),
    )
    .unwrap();
    assert_eq!(jobs.as_array().unwrap().len(), 1);
    let plan = dispatch_command(
        &store,
        &suite,
        "build_memory_worker_plan",
        &json!({"input": {"conversation_id": "conv-1", "job_id": "mj-1"}}),
    )
    .unwrap();
    assert!(plan["input_watermark"].is_string());

    for sub in [
        json!({"type": "claim_memory_worker", "job_id": "mj-1", "worker_id": "w1", "ownership_token": "tok-1"}),
        json!({"type": "heartbeat_memory_worker", "job_id": "mj-1", "ownership_token": "tok-1"}),
        json!({"type": "memory_worker_completed", "job_id": "mj-1", "ownership_token": "tok-1", "completion_watermark": "seq:0", "output": {"memories": []}}),
    ] {
        let res = dispatch_command(
            &store,
            &suite,
            "reduce_and_commit_idle_submission",
            &json!({"input": {"conversation_id": "conv-1", "submission": sub}}),
        )
        .unwrap();
        assert_eq!(res["ok"], true);
    }
}

#[test]
fn list_profiles_overlays_fresher_connection_catalog_without_rediscover() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    let suite = ModulesSuite::new(store.clone(), dir.path().to_path_buf(), None);

    // Seed a connection whose catalog already has /props reasoning metadata (observed_at_ms = 2000),
    // while the stored model profile is stale from an earlier sync (updated_at_ms = 1000, reasoning=false).
    suite
        .upsert_record(
            "model_connection",
            "conn-local",
            &json!({
                "id": "conn-local",
                "display_name": "local",
                "base_url": "http://192.168.31.18:8081/v1",
                "protocols": ["chat_completions"],
                "enabled": true,
                "revision": 2,
                "updated_at_ms": 2000,
                "catalog": {
                    "observed_at_ms": 2000,
                    "source": "discovery",
                    "models": [{
                        "model_id": "qwen3.8-flash-next-iq3_s",
                        "display_name": "qwen3.8-flash-next-iq3_s",
                        "protocol": "chat_completions",
                        "context_window": 262144,
                        "output_limit": 16384,
                        "reasoning_levels": ["none", "low", "medium", "high"],
                        "execution_capabilities": {
                            "toolCalls": true,
                            "imageInput": true,
                            "reasoning": true,
                            "samplingParameters": false
                        }
                    }]
                }
            }),
        )
        .unwrap();
    suite
        .upsert_record(
            "model",
            "conn:conn-local:chat_completions:qwen3.8-flash-next-iq3_s",
            &json!({
                "id": "conn:conn-local:chat_completions:qwen3.8-flash-next-iq3_s",
                "display_name": "qwen3.8-flash-next-iq3_s",
                "provider": "openai-compatible",
                "model_name": "qwen3.8-flash-next-iq3_s",
                "runtime": "remote",
                "source_json": serde_json::to_string(&json!({
                    "connectionID": "conn-local",
                    "catalogKey": "chat_completions:qwen3.8-flash-next-iq3_s",
                    "endpointID": "conn-local",
                    "endpointName": "local",
                    "baseURL": "http://192.168.31.18:8081/v1",
                    "endpointEnabled": true,
                    "modelEnabled": true,
                    "wireProtocol": "chat_completions",
                    "executionCapabilities": {
                        "toolCalls": true,
                        "imageInput": false,
                        "reasoning": false,
                        "samplingParameters": true
                    }
                })).unwrap(),
                "parameters_json": "{\"maxContextLength\":262144}",
                "selected": true,
                "enabled": true,
                "revision": 1,
                "created_at_ms": 1000,
                "updated_at_ms": 1000,
                "deleted_at_ms": null
            }),
        )
        .unwrap();

    let listed = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({"command": {"type": "list_profiles"}}),
    )
    .unwrap();
    let prof = &listed["items"][0];
    let src: Value = serde_json::from_str(prof["source_json"].as_str().unwrap()).unwrap();
    assert_eq!(src["executionCapabilities"]["reasoning"], true);
    assert_eq!(src["executionCapabilities"]["imageInput"], true);
    assert_eq!(src["reasoning"]["control"], "effort");
    assert_eq!(src["reasoning"]["efforts"], json!(["none", "low", "medium", "high"]));
    let params: Value = serde_json::from_str(prof["parameters_json"].as_str().unwrap()).unwrap();
    assert_eq!(params["maxOutputTokens"], 16384);
}

#[test]
fn real_scenario_model_connection_full_lifecycle_reasoning_discovery_override_clear_delete() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    // Isolate from developer host ~/.nola/config.json so list_connections starts clean
    crate::config::save_applied_config(&store, &crate::config::AppliedConfig::default()).unwrap();
    let suite = ModulesSuite::new(store.clone(), dir.path().to_path_buf(), None);

    // 1. Create account & connection
    let acct = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({
            "command": {
                "type": "save_account",
                "input": {
                    "id": "acct-lifecycle",
                    "display_name": "Local Llama",
                    "source": "manual",
                    "credential_ref": "account:local-llama",
                    "state": "ready"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(acct["ok"], true);

    let saved_conn = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({
            "command": {
                "type": "save_connection",
                "input": {
                    "id": "conn-lifecycle",
                    "display_name": "Local Llama Server",
                    "base_url": "http://127.0.0.1:8081/v1",
                    "protocols": ["chat_completions"],
                    "account_id": "acct-lifecycle",
                    "source": "manual",
                    "enabled": true
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(saved_conn["document"]["connection"]["id"], "conn-lifecycle");

    // 2. Refresh catalog WITHOUT reasoning (reasoning=false, reasoning_levels=[])
    let _ = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({
            "command": {
                "type": "refresh_connection_catalog",
                "connection_id": "conn-lifecycle",
                "observed_at_ms": 1_000_000i64,
                "models": [{
                    "model_id": "qwen3-32b-local",
                    "display_name": "Qwen3 32B Local",
                    "protocol": "chat_completions",
                    "context_window": 131072,
                    "output_limit": 8192,
                    "reasoning_levels": [],
                    "execution_capabilities": {
                        "toolCalls": true,
                        "imageInput": false,
                        "reasoning": false,
                        "samplingParameters": true
                    }
                }]
            }
        }),
    )
    .unwrap();

    // 3. Enable model via configure_connection_models
    let _ = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({
            "command": {
                "type": "configure_connection_models",
                "connection_id": "conn-lifecycle",
                "updated_at_ms": 1_000_001i64,
                "enabled_model_ids": ["chat_completions:qwen3-32b-local"],
                "selected_model_id": "chat_completions:qwen3-32b-local"
            }
        }),
    )
    .unwrap();

    // 4. Verify list_profiles has NO reasoning control initially
    let list_initial = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({"command": {"type": "list_profiles"}}),
    )
    .unwrap();
    let p_init = &list_initial["profiles"][0];
    let src_init: Value = serde_json::from_str(p_init["source_json"].as_str().unwrap()).unwrap();
    assert_eq!(src_init["executionCapabilities"]["reasoning"], false);
    assert!(src_init.get("reasoning").is_none_or(Value::is_null));

    // 5. Refresh catalog with `/props` reasoning levels ["none","low","medium","high"] WITHOUT re-enabling!
    let _ = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({
            "command": {
                "type": "refresh_connection_catalog",
                "connection_id": "conn-lifecycle",
                "observed_at_ms": 2_000_000i64,
                "models": [{
                    "model_id": "qwen3-32b-local",
                    "display_name": "Qwen3 32B Local",
                    "protocol": "chat_completions",
                    "context_window": 131072,
                    "output_limit": 16384,
                    "reasoning_levels": ["none", "low", "medium", "high"],
                    "execution_capabilities": {
                        "toolCalls": true,
                        "imageInput": true,
                        "reasoning": true,
                        "samplingParameters": false
                    }
                }]
            }
        }),
    )
    .unwrap();

    // 6. Verify list_profiles IMMEDIATELY exposes reasoning effort control!
    let list_after_refresh = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({"command": {"type": "list_profiles"}}),
    )
    .unwrap();
    let src_refreshed: Value =
        serde_json::from_str(list_after_refresh["profiles"][0]["source_json"].as_str().unwrap())
            .unwrap();
    assert_eq!(src_refreshed["executionCapabilities"]["reasoning"], true);
    assert_eq!(src_refreshed["reasoning"]["control"], "effort");
    assert_eq!(
        src_refreshed["reasoning"]["efforts"],
        json!(["none", "low", "medium", "high"])
    );

    // 7. set_connection_capability_override toggling reasoning=false -> list_profiles removes reasoning!
    let _ = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({
            "command": {
                "type": "set_connection_capability_override",
                "connection_id": "conn-lifecycle",
                "model_id": "qwen3-32b-local",
                "protocol": "chat_completions",
                "updated_at_ms": 3_000_000i64,
                "capabilities": {
                    "toolCalls": true,
                    "imageInput": true,
                    "reasoning": false,
                    "samplingParameters": false
                }
            }
        }),
    )
    .unwrap();
    let list_disabled_r = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({"command": {"type": "list_profiles"}}),
    )
    .unwrap();
    let src_disabled: Value =
        serde_json::from_str(list_disabled_r["profiles"][0]["source_json"].as_str().unwrap())
            .unwrap();
    assert_eq!(src_disabled["executionCapabilities"]["reasoning"], false);
    assert!(
        src_disabled.get("reasoning").is_none_or(Value::is_null),
        "toggling reasoning override to false must remove reasoning object: {src_disabled}"
    );

    // 8. set_connection_capability_override toggling reasoning=true -> list_profiles restores reasoning!
    let _ = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({
            "command": {
                "type": "set_connection_capability_override",
                "connection_id": "conn-lifecycle",
                "model_id": "qwen3-32b-local",
                "protocol": "chat_completions",
                "updated_at_ms": 4_000_000i64,
                "capabilities": {
                    "toolCalls": true,
                    "imageInput": true,
                    "reasoning": true,
                    "samplingParameters": false
                }
            }
        }),
    )
    .unwrap();
    let list_restored_r = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({"command": {"type": "list_profiles"}}),
    )
    .unwrap();
    let src_restored: Value =
        serde_json::from_str(list_restored_r["profiles"][0]["source_json"].as_str().unwrap())
            .unwrap();
    assert_eq!(src_restored["executionCapabilities"]["reasoning"], true);
    assert_eq!(src_restored["reasoning"]["control"], "effort");
    assert_eq!(
        src_restored["reasoning"]["efforts"],
        json!(["none", "low", "medium", "high"])
    );

    // 9. save_connection with clear_catalog=true clears stale catalog & capability_assessments
    let cleared = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({
            "command": {
                "type": "save_connection",
                "clear_catalog": true,
                "input": {
                    "id": "conn-lifecycle",
                    "display_name": "New Endpoint",
                    "base_url": "https://new-gateway.example.invalid/v1",
                    "protocols": ["chat_completions"],
                    "account_id": "acct-lifecycle",
                    "source": "manual",
                    "enabled": true
                }
            }
        }),
    )
    .unwrap();
    assert!(cleared["document"]["catalog"].is_null());
    assert!(cleared["document"]["capability_assessments"]
        .as_array()
        .is_none_or(|a| a.is_empty()));

    // 10. delete_connection removes connection + enabled profiles + unreferenced account cleanly
    let del = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({
            "command": {
                "type": "delete_connection",
                "id": "conn-lifecycle",
                "delete_account": true
            }
        }),
    )
    .unwrap();
    assert_eq!(del["deleted"], true);
    assert_eq!(del["removed_profiles"], 1);
    assert_eq!(del["deleted_account_id"], "acct-lifecycle");

    let conns_after = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({"command": {"type": "list_connections"}}),
    )
    .unwrap();
    assert!(conns_after["connections"].as_array().unwrap().is_empty());
    let profs_after = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({"command": {"type": "list_profiles"}}),
    )
    .unwrap();
    assert!(profs_after["profiles"].as_array().unwrap().is_empty());
    let accts_after = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({"command": {"type": "list_accounts"}}),
    )
    .unwrap();
    assert!(accts_after["accounts"].as_array().unwrap().is_empty());
}

#[test]
fn real_scenario_500_model_openrouter_catalog_stress_test_latency_and_json_slimness() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    crate::config::save_applied_config(&store, &crate::config::AppliedConfig::default()).unwrap();
    let suite = ModulesSuite::new(store.clone(), dir.path().to_path_buf(), None);

    let _ = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({
            "command": {
                "type": "save_connection",
                "input": {
                    "id": "conn-or-500",
                    "display_name": "OpenRouter 500",
                    "base_url": "https://openrouter.ai/api/v1",
                    "protocols": ["chat_completions"],
                    "source": "manual",
                    "enabled": true
                }
            }
        }),
    )
    .unwrap();

    // Build 500 unique models + 10 intentional duplicates to exercise dedup & throughput
    let mut models: Vec<Value> = (0..500)
        .map(|i| {
            json!({
                "model_id": format!("provider-{i:03}/model-{i:03}"),
                "display_name": format!("Provider {i} Model {i}"),
                "protocol": "chat_completions",
                "context_window": 128_000,
                "output_limit": 8_192,
                "modalities": ["text"],
                "reasoning_levels": if i % 5 == 0 { json!(["low", "medium", "high"]) } else { json!([]) },
                "execution_capabilities": {
                    "toolCalls": true,
                    "imageInput": i % 3 == 0,
                    "reasoning": i % 5 == 0,
                    "samplingParameters": true
                }
            })
        })
        .collect();
    for dup in 0..10 {
        models.push(models[dup].clone());
    }

    let start = std::time::Instant::now();
    let refreshed = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({
            "command": {
                "type": "refresh_connection_catalog",
                "connection_id": "conn-or-500",
                "models": models
            }
        }),
    )
    .unwrap();
    let listed = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({"command": {"type": "list_connections"}}),
    )
    .unwrap();
    let elapsed = start.elapsed();

    assert!(
        elapsed < std::time::Duration::from_millis(1000),
        "500-model refresh + list_connections took {:?}, expected <1000ms",
        elapsed
    );
    assert_eq!(
        refreshed["document"]["catalog"]["models"].as_array().unwrap().len(),
        500
    );

    let conn_snap = listed["connections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["document"]["connection"]["id"] == "conn-or-500")
        .unwrap();
    assert!(conn_snap.get("catalog").is_none());
    assert!(conn_snap["document"]["connection"].get("catalog").is_none());
    assert_eq!(
        conn_snap["document"]["catalog"]["models"].as_array().unwrap().len(),
        500
    );

    // Ensure no duplicate model keys bloated the list_connections JSON!
    let listed_json = serde_json::to_string(&listed).unwrap();
    assert_eq!(
        listed_json.matches("provider-499/model-499").count(),
        1,
        "list_connections must not duplicate catalog entries"
    );
}

#[test]
fn real_scenario_strict_two_mode_permission_enforcement_across_daemon_commands() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    let suite = ModulesSuite::new(store.clone(), dir.path().to_path_buf(), None);

    let conv_id = "conv-perm-2mode";
    let _ = dispatch_command(
        &store,
        &suite,
        "create_conversation",
        &json!({"id": conv_id, "title": "Strict 2-Mode"}),
    )
    .unwrap();

    // 1. set_conversation_permission accepts strictly "cautious" and "allow_all"
    let r_allow = dispatch_command(
        &store,
        &suite,
        "set_conversation_permission",
        &json!({"conversation_id": conv_id, "permission_profile": "allow_all"}),
    )
    .unwrap();
    assert_eq!(r_allow["status"], "committed");
    assert_eq!(r_allow["permission_profile"], "allow_all");

    let r_cautious = dispatch_command(
        &store,
        &suite,
        "set_conversation_permission",
        &json!({"conversation_id": conv_id, "permission_profile": "cautious"}),
    )
    .unwrap();
    assert_eq!(r_cautious["status"], "committed");
    assert_eq!(r_cautious["permission_profile"], "cautious");

    // Rejected non-canonical / removed modes (including accept_edits!)
    for bad in ["accept_edits", "accept-edits", "yolo", "auto", "ask", ""] {
        let err = dispatch_command(
            &store,
            &suite,
            "set_conversation_permission",
            &json!({"conversation_id": conv_id, "permission_profile": bad}),
        );
        assert!(
            err.is_err(),
            "set_conversation_permission must reject invalid mode '{bad}'"
        );
    }

    // 2. patch_conversation_configuration accepts cautious/allow_all/null and rejects accept_edits
    let p_ok = dispatch_command(
        &store,
        &suite,
        "patch_conversation_configuration",
        &json!({"conversation_id": conv_id, "permissionProfile": "allow_all"}),
    )
    .unwrap();
    assert!(p_ok.is_object());
    let p_bad = dispatch_command(
        &store,
        &suite,
        "patch_conversation_configuration",
        &json!({"conversation_id": conv_id, "permissionProfile": "accept_edits"}),
    );
    assert!(
        p_bad.is_err(),
        "patch_conversation_configuration must reject 'accept_edits'"
    );
    let _ = dispatch_command(
        &store,
        &suite,
        "patch_conversation_configuration",
        &json!({"conversation_id": conv_id, "permissionProfile": Value::Null}),
    )
    .unwrap();

    // 3. Stale/corrupted KV value `"accept_edits"` normalizes strictly to `"cautious"` on read!
    store
        .set_kv(&format!("conv_perm:{conv_id}"), &json!("accept_edits"))
        .unwrap();
    let proj = dispatch_command(
        &store,
        &suite,
        "load_conversation_permission_projection_v1",
        &json!({"conversation_id": conv_id}),
    )
    .unwrap();
    assert_eq!(proj["permission_profile"], "cautious");

    // 4. begin_assistant_turn + execute_tool dispatch normalizes non-canonical `accept_edits` to `cautious`
    for (cfg_profile, expected_wire) in [
        ("allow_all", "allow_all"),
        ("cautious", "cautious"),
        ("accept_edits", "cautious"),
    ] {
        let run_id = format!("run-perm-{cfg_profile}");
        let _ = dispatch_command(
            &store,
            &suite,
            "begin_assistant_turn",
            &json!({
                "input": {
                    "conversation_id": conv_id,
                    "core_config": {
                        "session_id": conv_id,
                        "model": "m1",
                        "permission_profile": cfg_profile
                    },
                    "submission": {"type": "user_input", "submission_id": run_id}
                }
            }),
        )
        .unwrap();
        let reqs0 = dispatch_command(
            &store,
            &suite,
            "claim_committed_host_requests",
            &json!({"input": {"run_id": run_id}}),
        )
        .unwrap();
        let m0_id = reqs0["requests"][0]["request_id"]
            .as_str()
            .unwrap()
            .to_string();
        let _ = dispatch_command(
            &store,
            &suite,
            "reduce_and_accept_host_response",
            &json!({
                "input": {
                    "conversation_id": conv_id,
                    "run_id": run_id,
                    "response": {
                        "request_id": m0_id,
                        "payload": {
                            "type": "model_completed",
                            "items": [],
                            "tool_calls": [{"call_id": "tc1", "name": "read_file", "arguments": {"path": "a.txt"}}]
                        }
                    }
                }
            }),
        )
        .unwrap();
        let treqs = dispatch_command(
            &store,
            &suite,
            "claim_committed_host_requests",
            &json!({"input": {"run_id": run_id}}),
        )
        .unwrap();
        assert_eq!(treqs["requests"][0]["payload"]["type"], "execute_tool");
        assert_eq!(
            treqs["requests"][0]["payload"]["permission_profile"], expected_wire,
            "core_config.permission_profile={cfg_profile} must emit {expected_wire}"
        );
    }
}

#[test]
fn real_scenario_multi_connection_model_isolation_and_shared_account_retention_on_delete() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    crate::config::save_applied_config(&store, &crate::config::AppliedConfig::default()).unwrap();
    let suite = ModulesSuite::new(store.clone(), dir.path().to_path_buf(), None);

    // 1 shared account referenced by 2 connections
    let _ = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({
            "command": {
                "type": "save_account",
                "input": {
                    "id": "acct-shared",
                    "display_name": "Shared Key",
                    "source": "manual",
                    "credential_ref": "account:shared",
                    "state": "ready"
                }
            }
        }),
    )
    .unwrap();

    for (cid, model_id) in [("conn-a", "model-a"), ("conn-b", "model-b")] {
        let _ = dispatch_command(
            &store,
            &suite,
            "model",
            &json!({
                "command": {
                    "type": "save_connection",
                    "input": {
                        "id": cid,
                        "display_name": cid,
                        "base_url": format!("https://{cid}.example.invalid/v1"),
                        "protocols": ["chat_completions"],
                        "account_id": "acct-shared",
                        "source": "manual",
                        "enabled": true
                    }
                }
            }),
        )
        .unwrap();
        let _ = dispatch_command(
            &store,
            &suite,
            "model",
            &json!({
                "command": {
                    "type": "refresh_connection_catalog",
                    "connection_id": cid,
                    "models": [{"model_id": model_id, "protocol": "chat_completions"}]
                }
            }),
        )
        .unwrap();
        let _ = dispatch_command(
            &store,
            &suite,
            "model",
            &json!({
                "command": {
                    "type": "configure_connection_models",
                    "connection_id": cid,
                    "enabled_model_ids": [format!("chat_completions:{model_id}")]
                }
            }),
        )
        .unwrap();
    }

    // Deleting conn-a removes conn-a + model-a, keeps conn-b + model-b AND retains acct-shared!
    let del_a = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({"command": {"type": "delete_connection", "id": "conn-a", "delete_account": true}}),
    )
    .unwrap();
    assert_eq!(del_a["removed_profiles"], 1);
    assert!(del_a["deleted_account_id"].is_null());

    let profs_mid = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({"command": {"type": "list_profiles"}}),
    )
    .unwrap();
    assert_eq!(profs_mid["profiles"].as_array().unwrap().len(), 1);
    assert_eq!(profs_mid["profiles"][0]["model_name"], "model-b");

    let accts_mid = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({"command": {"type": "list_accounts"}}),
    )
    .unwrap();
    assert_eq!(accts_mid["accounts"].as_array().unwrap().len(), 1);

    // Deleting conn-b now deletes the unreferenced acct-shared cleanly!
    let del_b = dispatch_command(
        &store,
        &suite,
        "model",
        &json!({"command": {"type": "delete_connection", "id": "conn-b", "delete_account": true}}),
    )
    .unwrap();
    assert_eq!(del_b["removed_profiles"], 1);
    assert_eq!(del_b["deleted_account_id"], "acct-shared");
}

#[test]
fn real_scenario_conversation_lifecycle_projection_and_cas_conflict() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    let suite = ModulesSuite::new(store.clone(), dir.path().to_path_buf(), None);

    let conv_id = "conv-cas-proj";
    let _ = dispatch_command(
        &store,
        &suite,
        "create_conversation",
        &json!({"id": conv_id, "title": "CAS Test"}),
    )
    .unwrap();

    // First permission change with expected_revision=1 succeeds -> revision=2
    let ok1 = dispatch_command(
        &store,
        &suite,
        "set_conversation_permission",
        &json!({
            "conversation_id": conv_id,
            "permission_profile": "allow_all",
            "expected_revision": 1
        }),
    )
    .unwrap();
    assert_eq!(ok1["status"], "committed");
    assert_eq!(ok1["configuration_revision"], 2);

    // Stale expected_revision=1 returns structured conflict without overwriting!
    let conflict = dispatch_command(
        &store,
        &suite,
        "set_conversation_permission",
        &json!({
            "conversation_id": conv_id,
            "permission_profile": "cautious",
            "expected_revision": 1
        }),
    )
    .unwrap();
    assert_eq!(conflict["status"], "conflict");
    assert_eq!(conflict["actual_revision"], 2);

    // Verify stored permission is still allow_all
    let perm = dispatch_command(
        &store,
        &suite,
        "load_conversation_permission_projection_v1",
        &json!({"conversation_id": conv_id}),
    )
    .unwrap();
    assert_eq!(perm["permission_profile"], "allow_all");

    // Matching expected_revision=2 succeeds -> revision=3
    let ok2 = dispatch_command(
        &store,
        &suite,
        "set_conversation_permission",
        &json!({
            "conversation_id": conv_id,
            "permission_profile": "cautious",
            "expected_revision": 2
        }),
    )
    .unwrap();
    assert_eq!(ok2["status"], "committed");
    assert_eq!(ok2["configuration_revision"], 3);
}

#[test]
fn real_scenario_agent_room_multi_expert_submission_routing_and_reconciliation() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    let suite = ModulesSuite::new(store.clone(), dir.path().to_path_buf(), None);

    for (id, name, role, desc) in [
        ("exp-arch", "Architect", "architect", "System design specialist"),
        ("exp-qa", "Tester", "qa", "Regression & integration test specialist"),
    ] {
        let _ = dispatch_agent_room_command(
            &store,
            &suite,
            &json!({
                "command": {
                    "type": "configure_agent",
                    "input": {"id": id, "name": name, "role": role, "description": desc}
                }
            }),
        )
        .unwrap();
    }

    let _ = dispatch_agent_room_command(
        &store,
        &suite,
        &json!({"command": {"type": "create_room", "input": {"id": "room-release", "name": "Release War Room"}}}),
    )
    .unwrap();

    // Submit 3 tasks: #1 completed, #2 failed, #3 cancelled via cancel_pending
    let s1 = dispatch_agent_room_command(
        &store,
        &suite,
        &json!({"command": {"type": "submit", "room_id": "room-release", "content": "Audit schema"}}),
    )
    .unwrap()["submission_id"]
        .as_str()
        .unwrap()
        .to_string();
    let s2 = dispatch_agent_room_command(
        &store,
        &suite,
        &json!({"command": {"type": "submit", "room_id": "room-release", "content": "Run stress suite"}}),
    )
    .unwrap()["submission_id"]
        .as_str()
        .unwrap()
        .to_string();
    let s3 = dispatch_agent_room_command(
        &store,
        &suite,
        &json!({"command": {"type": "submit", "room_id": "room-release", "content": "Pending low-priority item"}}),
    )
    .unwrap()["submission_id"]
        .as_str()
        .unwrap()
        .to_string();

    let _ = dispatch_agent_room_command(
        &store,
        &suite,
        &json!({"command": {"type": "select_recipient", "room_id": "room-release", "submission_id": s1, "recipient_id": "exp-arch"}}),
    )
    .unwrap();
    let _ = dispatch_agent_room_command(
        &store,
        &suite,
        &json!({"command": {"type": "prepare_execution", "room_id": "room-release", "submission_id": s1}}),
    )
    .unwrap();
    let _ = dispatch_agent_room_command(
        &store,
        &suite,
        &json!({"command": {"type": "claim_execution", "room_id": "room-release", "submission_id": s1}}),
    )
    .unwrap();
    let _ = dispatch_agent_room_command(
        &store,
        &suite,
        &json!({"command": {"type": "reconcile_execution", "submission_id": s1, "status": "completed"}}),
    )
    .unwrap();

    let _ = dispatch_agent_room_command(
        &store,
        &suite,
        &json!({"command": {"type": "select_recipient", "room_id": "room-release", "submission_id": s2, "recipient_id": "exp-qa"}}),
    )
    .unwrap();
    let _ = dispatch_agent_room_command(
        &store,
        &suite,
        &json!({"command": {"type": "claim_execution", "room_id": "room-release", "submission_id": s2}}),
    )
    .unwrap();
    let _ = dispatch_agent_room_command(
        &store,
        &suite,
        &json!({"command": {"type": "fail_submission", "submission_id": s2, "error": "timeout"}}),
    )
    .unwrap();

    let cancelled = dispatch_agent_room_command(
        &store,
        &suite,
        &json!({"command": {"type": "cancel_pending", "room_id": "room-release"}}),
    )
    .unwrap();
    assert_eq!(cancelled["cancelled_count"], 1);

    let all_subs = dispatch_agent_room_command(
        &store,
        &suite,
        &json!({"command": {"type": "submissions", "room_id": "room-release"}}),
    )
    .unwrap();
    let arr = all_subs.as_array().unwrap();
    assert_eq!(arr.len(), 3);
    assert!(arr.iter().any(|s| s["submission_id"] == s1 && s["status"] == "completed"));
    assert!(arr.iter().any(|s| s["submission_id"] == s2 && s["status"] == "failed"));
    assert!(arr.iter().any(|s| s["submission_id"] == s3 && s["status"] == "cancelled"));
}

#[test]
fn real_scenario_assistant_journal_multi_part_streaming_revision_diff_and_utf8_range_reads() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    let suite = ModulesSuite::new(store.clone(), dir.path().to_path_buf(), None);

    let conv_id = "conv-j-utf8";
    let run_id = "run-j-utf8";
    let asst_id = "asst-j-utf8";
    let mut run_state = json!({
        "conversation_id": conv_id,
        "run_id": run_id,
        "assistant_message_id": asst_id,
        "session_id": conv_id,
        "created_at_ms": now_ms(),
        "journal_identity": journal_identity_value(&store, conv_id, run_id, asst_id),
        "journal_seq": 0u64,
        "sequence": 1u64,
        "status": "running"
    });

    // Publish reasoning:0 -> tool:c1 -> reasoning:1 -> text with CJK + emoji UTF-8 markdown
    let c1 = publish_journal_part(
        &store,
        &mut run_state,
        &format!("{asst_id}:reasoning:0"),
        0,
        "reasoning",
        json!({"state": "completed", "title": {"kind": "inline", "value": "Thinking"}}),
        &[("reasoning_summary", "分析需求中…".into())],
    );
    let _c2 = publish_journal_part(
        &store,
        &mut run_state,
        "tool:c1",
        1,
        "tool",
        json!({"tool_call_id": "c1", "capability_id": "read_file", "state": "completed"}),
        &[("tool_output_text", "Read spec.md".into())],
    );
    let _c3 = publish_journal_part(
        &store,
        &mut run_state,
        &format!("{asst_id}:reasoning:1"),
        2,
        "reasoning",
        json!({"state": "completed", "title": {"kind": "inline", "value": "Thinking"}}),
        &[("reasoning_summary", "汇总结果。".into())],
    );
    let utf8_markdown = "结论：架构升级完成🚀，所有检查项均通过✅。";
    let c4 = publish_journal_part(
        &store,
        &mut run_state,
        &format!("{asst_id}:text"),
        3,
        "text",
        json!({"visibility": "public", "is_final": true}),
        &[("markdown", utf8_markdown.into())],
    );

    run_state["status"] = json!("completed");
    let _seal = build_and_store_terminal_seal(&store, &run_state, &format!("{asst_id}:text"));

    // Open snapshot at latest cursor (c4) and diff against base cursor (c1)
    let snap_final = dispatch_command(
        &store,
        &suite,
        "open_assistant_journal_snapshot",
        &json!({"at": c4}),
    )
    .unwrap();
    let changes = dispatch_command(
        &store,
        &suite,
        "read_assistant_journal_part_changes",
        &json!({"snapshot": snap_final, "base": c1}),
    )
    .unwrap();
    assert!(
        changes["changes"].as_array().unwrap().len() >= 3,
        "diff from seq 1 -> seq 4 must include tool, reasoning:1, and text parts"
    );

    // Reassemble multi-byte UTF-8 text via small 9-byte range slices!
    let parts = dispatch_command(
        &store,
        &suite,
        "read_assistant_journal_parts",
        &json!({"snapshot": snap_final, "max_records": 32}),
    )
    .unwrap();
    let text_part = parts["parts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["part_id"] == format!("{asst_id}:text"))
        .unwrap();

    let mut offset: u64 = 0;
    let mut reconstructed = String::new();
    loop {
        let r = dispatch_command(
            &store,
            &suite,
            "read_assistant_journal_range",
            &json!({
                "snapshot": snap_final,
                "part_id": text_part["part_id"],
                "content": text_part["bodies"][0],
                "offset_bytes": offset.to_string(),
                "limit_bytes": 9
            }),
        )
        .unwrap();
        let chunk = r["text"].as_str().unwrap();
        reconstructed.push_str(chunk);
        if r["end_of_content"].as_bool().unwrap() {
            break;
        }
        offset += chunk.len() as u64;
    }
    assert_eq!(reconstructed, utf8_markdown);

    let seal = dispatch_command(
        &store,
        &suite,
        "local_assistant_run_terminal_seal",
        &json!({"conversation_id": conv_id, "run_id": run_id}),
    )
    .unwrap();
    assert_eq!(seal["result"]["value"]["kind"], "answered");
}
