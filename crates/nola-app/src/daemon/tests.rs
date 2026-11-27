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
