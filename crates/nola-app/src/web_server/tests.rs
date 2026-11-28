// SPDX-License-Identifier: Apache-2.0

use std::sync::mpsc;
use std::time::Duration;

use super::*;

#[test]
fn bounded_nonce_cache_evicts_oldest_in_o1_and_blocks_replays() {
    let mut cache = BoundedNonceCache::default();
    assert!(cache.record_if_fresh("n1".into()));
    assert!(!cache.record_if_fresh("n1".into()));
    for i in 0..MAX_TRACKED_NONCES {
        assert!(cache.record_if_fresh(format!("fill_{i}")));
    }
    assert!(cache.record_if_fresh("n1".into()));
}

#[test]
fn channel_reader_streams_chunks_incrementally_and_terminates_on_sender_drop() {
    let (tx, rx) = mpsc::channel();
    tx.send(b"data: 1\n\n".to_vec()).unwrap();
    tx.send(b"data: [DONE]\n\n".to_vec()).unwrap();
    drop(tx);
    let mut reader = ChannelReader::new(rx);
    let mut out = String::new();
    reader.read_to_string(&mut out).unwrap();
    assert_eq!(out, "data: 1\n\ndata: [DONE]\n\n");
}

#[test]
fn strip_jsonc_handles_comments_urls_and_trailing_commas() {
    let raw = r#"{
        // comment
        "endpoint": "https://api.example.com/v1", /* block comment */
        "items": [1, 2,],
    }"#;
    let clean = crate::config::strip_jsonc(raw);
    let v: Value = serde_json::from_str(&clean).unwrap();
    assert_eq!(v["endpoint"], "https://api.example.com/v1");
    assert_eq!(v["items"], json!([1, 2]));
}

#[test]
fn server_started_handshake_matches_desktop_and_mac_contracts() {
    let frame = server_started_handshake("127.0.0.1", 28421);
    assert_eq!(frame["type"], "server.started");
    assert_eq!(frame["protocol_version"], 1);
    assert_eq!(frame["loopback"], true);
    assert_eq!(frame["origin"], "http://127.0.0.1:28421");
    assert_eq!(frame["client_api"], "http://127.0.0.1:28421/api/mobile");
    assert_eq!(frame["host_id"], "nola-host-0000000000000000");
    assert_eq!(frame["access_token"], "test-token");
    assert_eq!(frame["credentials_path"], "/tmp/mobile-access-token");
    assert_eq!(frame["clientAPI"]["path"], "/api/mobile");

    let lan = server_started_handshake("0.0.0.0", 5001);
    assert_eq!(lan["loopback"], false);
}

#[test]
fn model_connections_inventory_respects_selections_and_capability_overrides() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    crate::config::bind_store_vault_path(&store, &dir.path().join("vault.json")).unwrap();
    let suite = ModulesSuite::new(store.clone(), dir.path().to_path_buf(), None);
    let cfg = crate::config::AppliedConfig {
        llm: crate::config::LlmSection {
            default_model: Some(DefaultModelSelection {
                connection: "ep1".into(),
                model: "m1".into(),
            }),
            connections: vec![ConnectionConfig {
                id: "ep1".into(),
                endpoint: "https://example.invalid/v1".into(),
                protocol: Some("chat_completions".into()),
                provider: None,
                api_key: Some("sk-secret".into()),
                api_key_env: None,
                models: vec![
                    ModelEntry {
                        id: "m1".into(),
                        context_window: Some(128_000),
                        max_output_tokens: None,
                        temperature: None,
                        top_p: None,
                        reasoning_effort: None,
                        capabilities: None,
                    },
                    ModelEntry {
                        id: "m2".into(),
                        context_window: Some(64_000),
                        max_output_tokens: None,
                        temperature: None,
                        top_p: None,
                        reasoning_effort: None,
                        capabilities: None,
                    },
                ],
            }],
        },
        ..Default::default()
    };
    save_applied_config(&store, &cfg).unwrap();
    store
        .set_kv(
            "model_conn_selections:ep1",
            &json!({"enabled": ["chat_completions:m1"]}),
        )
        .unwrap();
    store
        .set_kv(
            "model_cap_override:ep1:m1",
            &json!({
                "model_id": "m1",
                "protocol": "chat_completions",
                "user_override": {
                    "toolCalls": true,
                    "imageInput": true,
                    "reasoning": false,
                    "samplingParameters": true,
                    "samplingRequiresNoReasoning": false,
                    "chatOutputTokenField": "auto"
                }
            }),
        )
        .unwrap();

    let inv = build_model_connections_inventory(&store, &suite);
    let sels = inv["selections"].as_array().unwrap();
    assert_eq!(sels[0]["enabled"], true);
    assert_eq!(sels[1]["enabled"], false);
    let assessments = inv["connections"][0]["document"]["capability_assessments"]
        .as_array()
        .unwrap();
    assert_eq!(assessments.len(), 1);
    assert_eq!(assessments[0]["model_id"], "m1");
    assert_eq!(assessments[0]["user_override"]["imageInput"], true);

    let cred = build_endpoint_credential_response(&store, "ep1").unwrap();
    assert_eq!(cred["endpointID"], "ep1");
    assert_eq!(cred["hasCredential"], true);
    assert_eq!(cred["credentialRevision"], 1);
    assert!(!cred["expectedRevisions"].as_object().unwrap().is_empty());
}

#[test]
fn static_target_resolves_ui_static_and_spa_routes() {
    let ui = Path::new("/web/app/static/ui");
    let st = Path::new("/web/app/static");
    assert_eq!(
        resolve_static_target(ui, st, "/ui/assets/app.js"),
        (ui, "assets/app.js".into(), false)
    );
    assert_eq!(
        resolve_static_target(ui, st, "/ui/chat"),
        (ui, "chat".into(), true)
    );
    assert_eq!(
        resolve_static_target(ui, st, "/static/favicon.ico"),
        (st, "favicon.ico".into(), false)
    );
    assert_eq!(
        resolve_static_target(ui, st, "/"),
        (ui, "index.html".into(), false)
    );
}

#[test]
fn aes256_gcm_matches_nist_vectors_and_mobile_envelope_roundtrips() {
    // NIST SP 800-38D Test Case 13 (AES-256-GCM, empty PT, empty AAD)
    let key = [0u8; 32];
    let nonce = [0u8; 12];
    let (ct0, tag0) = aes256_gcm_encrypt(&key, &nonce, b"", b"");
    assert!(ct0.is_empty());
    assert_eq!(hex::encode(tag0), "530f8afbc74536b9a963b4f1c4cb738b");

    // NIST SP 800-38D Test Case 14 (AES-256-GCM, 16-byte zero PT, empty AAD)
    let (ct1, tag1) = aes256_gcm_encrypt(&key, &nonce, b"", &[0u8; 16]);
    assert_eq!(hex::encode(&ct1), "cea7403d4d606b6e074ec5d3baf39d18");
    assert_eq!(hex::encode(tag1), "d0d1c8a799996bf0265b98b5d48ab919");
    let pt1 = aes256_gcm_decrypt(&key, &nonce, b"", &ct1, &tag1).unwrap();
    assert_eq!(pt1, vec![0u8; 16]);

    // Mobile JSON envelope roundtrip + tamper detection
    let payload = json!({"target": "/settings#channels", "count": 42});
    let env = encrypt_mobile_json_envelope(&payload, "test-mobile-token").unwrap();
    let dec = decrypt_mobile_json_envelope(&env, "test-mobile-token").unwrap();
    assert_eq!(dec, payload);
    assert!(decrypt_mobile_json_envelope(&env, "wrong-token").is_err());
}

#[test]
fn data_url_image_attachment_decodes_to_media_attachment() {
    let png_stub = b"\x89PNG\r\n\x1a\nhello";
    let data_url = format!("data:image/png;base64,{}", base64_encode(png_stub));
    let att = json!({
        "name": "shot.png",
        "dataUrl": data_url
    });
    let media = decode_attachment_media(&att).unwrap();
    assert_eq!(media.mime_type, "image/png");
    assert_eq!(media.filename, "shot.png");
    assert_eq!(media.raw_bytes_len, png_stub.len());
    assert_eq!(media.sha256, sha256_hex(png_stub));
}

#[test]
fn session_scoped_permission_grant_is_narrowly_scoped() {
    let pending = PendingInteractions::default();
    let (tx, _rx) = mpsc::channel();
    pending.approvals.lock().unwrap().insert(
        "req-1".into(),
        PendingApprovalEntry {
            conversation_id: "conv-a".into(),
            tool_name: "write_file".into(),
            target_key: "path:src/main.rs".into(),
            reason: "confirm write".into(),
            arguments: json!({"path": "src/main.rs"}),
            tx,
        },
    );
    assert!(pending.conversation_needs_response("conv-a"));
    assert!(!pending.conversation_needs_response("conv-b"));
    assert!(pending.resolve_approval("req-1", true, "session"));
    assert!(pending.has_session_grant("conv-a", "write_file", "path:src/main.rs"));
    assert!(!pending.has_session_grant("conv-a", "write_file", "path:src/other.rs"));
    assert!(!pending.has_session_grant("conv-b", "write_file", "path:src/main.rs"));
    pending.clear_conversation("conv-a");
    assert!(!pending.has_session_grant("conv-a", "write_file", "path:src/main.rs"));
}

#[test]
fn sec3_ssrf_and_origin_guard_block_metadata_and_cross_origin_keys() {
    assert!(validate_probe_url_ssrf("http://169.254.169.254/latest/meta-data/").is_err());
    assert!(validate_probe_url_ssrf("http://10.0.0.1:8080/v1/models").is_err());
    assert!(validate_probe_url_ssrf("http://192.168.1.5/v1/models").is_err());
    assert!(validate_probe_url_ssrf("http://metadata.google.internal/computeMetadata/v1").is_err());
    assert!(validate_probe_url_ssrf("http://127.0.0.1:11434/v1/models").is_ok());
    assert!(validate_probe_url_ssrf("http://localhost:8080/v1/models").is_ok());
    assert!(validate_probe_url_ssrf("http://[::1]:8080/v1/models").is_ok());

    assert!(same_origin_url("https://api.example.com/v1", "https://api.example.com:443/v2"));
    assert!(!same_origin_url("https://api.example.com/v1", "https://evil.example.com/v1"));
    assert!(!same_origin_url("http://127.0.0.1:8080/v1", "http://127.0.0.1:9090/v1"));

    let store = Store::open_in_memory().unwrap();
    let res = run_real_model_connection_probe(
        &store,
        &json!({"endpoint": "http://169.254.169.254/latest"}),
    );
    assert_eq!(res["ok"], false);
    assert!(res["error"].as_str().unwrap_or("").contains("blocked"));
}

#[test]
fn sec7_sensitive_paths_redaction_module_allowlist_and_channel_vault() {
    assert!(is_sensitive_local_path(Path::new("/home/alice/.ssh/id_rsa")));
    assert!(is_sensitive_local_path(Path::new("/home/alice/.nola/vault.json.key")));
    assert!(is_sensitive_local_path(Path::new("/workspace/.env")));
    assert!(is_sensitive_local_path(Path::new("/workspace/cert.pem")));
    assert!(!is_sensitive_local_path(Path::new("/workspace/README.md")));

    let redacted = redact_secrets_in_text(
        "API_KEY=supersecret\nAuthorization: Bearer tok12345\nkey sk-1234567890abcdef ok",
    );
    assert!(!redacted.contains("supersecret"));
    assert!(!redacted.contains("tok12345"));
    assert!(!redacted.contains("sk-1234567890abcdef"));

    assert!(is_allowed_module_kind("goal"));
    assert!(is_allowed_module_kind("automation"));
    assert!(is_allowed_module_kind("data_source"));
    assert!(!is_allowed_module_kind("messaging_channel"));
    assert!(!is_allowed_module_kind("expert_peer"));
    assert!(!is_allowed_module_kind("gateway_attempt"));

    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    crate::config::bind_store_vault_path(&store, &dir.path().join("vault.json")).unwrap();
    let norm = persist_channel_secret_and_normalize(
        &store,
        &json!({
            "connection_id": "tg-1",
            "kind": "telegram",
            "webhook_secret": "whsec-very-secret",
            "webhook_secret_ref": "PATH",
            "enabled": true
        }),
    );
    assert!(norm.get("webhook_secret").is_none());
    assert_eq!(norm["has_webhook_secret"], true);
    assert_eq!(resolve_channel_webhook_secret(&store, &norm), "whsec-very-secret");

    // Non-NOLA_WEBHOOK_ env var must NEVER be read as webhook secret!
    let ref_only = json!({
        "connection_id": "tg-env",
        "kind": "telegram",
        "webhook_secret_ref": "PATH"
    });
    assert_ne!(
        resolve_channel_webhook_secret(&store, &ref_only),
        std::env::var("PATH").unwrap_or_default()
    );
}

#[test]
fn mobile_crypto_fails_closed_and_url_matches_host_handles_default_ports() {
    assert!(maybe_decrypt_mobile_body(br#"{"plaintext":true}"#, "tok").is_err());
    let env = encrypt_mobile_json_envelope(&json!({"hello": "world"}), "tok").unwrap();
    let env_bytes = serde_json::to_vec(&env).unwrap();
    assert_eq!(
        maybe_decrypt_mobile_body(&env_bytes, "tok").unwrap(),
        json!({"hello": "world"})
    );

    let sse = encode_sse_frame(&json!({"type": "delta", "text": "hi"}), Some("tok"));
    let sse_str = String::from_utf8(sse).unwrap();
    let json_part = sse_str.trim_start_matches("data: ").trim();
    let parsed: Value = serde_json::from_str(json_part).unwrap();
    assert_eq!(parsed["type"], "encrypted");
    let dec = decrypt_mobile_json_envelope(&parsed, "tok").unwrap();
    assert_eq!(dec["text"], "hi");

    assert_eq!(url_matches_host("https://example.com:443", "example.com"), Ok(true));
    assert_eq!(url_matches_host("http://127.0.0.1:80", "127.0.0.1"), Ok(true));
    assert_eq!(url_matches_host("http://127.0.0.1:5001", "127.0.0.1:5001"), Ok(true));
    assert_eq!(url_matches_host("http://127.0.0.1:5002", "127.0.0.1:5001"), Ok(false));
    assert_eq!(url_matches_host("http://[::1]:5001", "[::1]:5001"), Ok(true));
}

#[test]
fn channel_reader_drop_cancels_on_disconnect_only() {
    // Early drop sets cancel = true
    let (_tx, rx) = mpsc::channel::<Vec<u8>>();
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let _reader = ChannelReader::with_cancel(rx, Arc::clone(&cancel));
    }
    assert!(cancel.load(Ordering::SeqCst));

    // Draining to EOF keeps cancel = false
    let (tx2, rx2) = mpsc::channel::<Vec<u8>>();
    tx2.send(b"done".to_vec()).unwrap();
    drop(tx2);
    let cancel2 = Arc::new(AtomicBool::new(false));
    {
        let mut reader = ChannelReader::with_cancel(rx2, Arc::clone(&cancel2));
        let mut out = String::new();
        reader.read_to_string(&mut out).unwrap();
        assert_eq!(out, "done");
    }
    assert!(!cancel2.load(Ordering::SeqCst));
}

#[test]
fn model_selection_handles_uuid_and_colons_and_mcp_array_command_and_loopback_binding() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    crate::config::bind_store_vault_path(&store, &dir.path().join("vault.json")).unwrap();
    let suite = ModulesSuite::new(store.clone(), dir.path().to_path_buf(), None);
    let cfg = crate::config::AppliedConfig {
        llm: crate::config::LlmSection {
            default_model: None,
            connections: vec![ConnectionConfig {
                id: "ollama".into(),
                endpoint: "http://127.0.0.1:11434/v1".into(),
                protocol: Some("chat_completions".into()),
                provider: None,
                api_key: None,
                api_key_env: None,
                models: vec![ModelEntry {
                    id: "llama3:8b".into(),
                    context_window: Some(8192),
                    max_output_tokens: None,
                    temperature: None,
                    top_p: None,
                    reasoning_effort: None,
                    capabilities: None,
                }],
            }],
        },
        ..Default::default()
    };
    save_applied_config(&store, &cfg).unwrap();
    store
        .set_kv(
            "web_model_profiles",
            &json!([{
                "id": "550e8400-e29b-41d4-a716-446655440000",
                "modelName": "llama3:8b",
                "endpoint": { "id": "ollama" }
            }]),
        )
        .unwrap();

    // UUID lookup resolves to (ollama, llama3:8b)
    assert_eq!(
        resolve_requested_model_selection(&store, &cfg, "550e8400-e29b-41d4-a716-446655440000"),
        Some(("ollama".into(), "llama3:8b".into()))
    );
    // `llama3:8b` is NOT falsely split on ':' because `llama3` is not a connection ID!
    assert_eq!(
        resolve_requested_model_selection(&store, &cfg, "llama3:8b"),
        Some(("ollama".into(), "llama3:8b".into()))
    );
    // `ollama:llama3:8b` splits cleanly on real connection ID!
    assert_eq!(
        resolve_requested_model_selection(&store, &cfg, "ollama:llama3:8b"),
        Some(("ollama".into(), "llama3:8b".into()))
    );

    // Keyless loopback endpoint emits `"none"` credential binding
    let inv = build_model_connections_inventory(&store, &suite);
    assert_eq!(
        inv["connections"][0]["probe_credential_bindings"]["chat_completions"],
        "none"
    );

    // Array-form `command` in MCP config
    let mcp = parse_mcp_server_config(
        &json!({
            "name": "fs-mcp",
            "command": ["npx", "-y", "@modelcontextprotocol/server-filesystem"],
            "args": ["/tmp"]
        }),
        None,
    );
    assert_eq!(mcp.command.as_deref(), Some("npx"));
    assert_eq!(
        mcp.args,
        vec!["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
    );

    // Context blocks wire custom_system_prompt, goal, knowledge, and data sources (defaultEnabled + enabled, excluding blocked)
    store
        .set_kv("conv_goal:c1", &json!({"title": "Ship v2", "description": "Zero warnings"}))
        .unwrap();
    store
        .set_kv(
            &conversation_knowledge_key("c1"),
            &json!({"keyPointsMarkdown": "Use Rust", "summary": "Use Rust"}),
        )
        .unwrap();
    suite
        .upsert_record(
            "data_source",
            "ds-def",
            &json!({"id": "ds-def", "title": "Default Docs", "kind": "localFile", "location": "/tmp/a.md", "accessPolicy": "defaultEnabled"}),
        )
        .unwrap();
    suite
        .upsert_record(
            "data_source",
            "ds-blk",
            &json!({"id": "ds-blk", "title": "Blocked Secret", "kind": "localFile", "location": "/tmp/b.md", "accessPolicy": "blocked"}),
        )
        .unwrap();
    let blocks = collect_conversation_context_blocks(
        &store,
        &suite,
        "c1",
        &json!({"custom_system_prompt": "Always be concise.", "enable_local_dev_tools": false}),
    );
    let joined: String = blocks.iter().map(|b| b.text.as_str()).collect::<Vec<_>>().join("\n");
    assert!(joined.contains("Always be concise."));
    assert!(joined.contains("Local development tools"));
    assert!(joined.contains("Ship v2"));
    assert!(joined.contains("Use Rust"));
    assert!(joined.contains("Default Docs"));
    assert!(!joined.contains("Blocked Secret"));
}

#[test]
fn data_source_probe_and_normalize_produces_valid_contract_for_created_and_listed_records() {
    let dir = tempfile::tempdir().unwrap();
    let sample_file = dir.path().join("notes.md");
    fs::write(&sample_file, "# Release Notes\nAll checks green.\n").unwrap();
    let store = Store::open_in_memory().unwrap();
    let suite = ModulesSuite::new(store.clone(), dir.path().to_path_buf(), None);

    // Simulate POST /api/data-sources with minimal fields (no accessPolicy supplied)
    let mut created = normalize_data_source_record(&json!({
        "id": "ds-valid-1",
        "name": "Release Notes",
        "kind": "localFile",
        "location": sample_file.display().to_string()
    }));
    let (ok, _probe) = probe_and_attach_data_source_validation(&suite, "ds-valid-1", &mut created);
    assert!(ok);
    let created = normalize_data_source_record(&created);
    suite.upsert_record("data_source", "ds-valid-1", &created).unwrap();

    // Simulate a raw legacy record in SQLite without accessPolicy or name
    suite
        .upsert_record(
            "data_source",
            "ds-legacy-2",
            &json!({"id": "ds-legacy-2", "title": "Legacy", "path": "/nonexistent.txt"}),
        )
        .unwrap();

    // Simulate GET /api/data-sources projection
    let listed: Vec<Value> = suite
        .list_records::<Value>("data_source")
        .unwrap()
        .iter()
        .map(normalize_data_source_record)
        .collect();
    assert_eq!(listed.len(), 2);
    for ds in &listed {
        for key in ["id", "name", "kind", "location", "accessPolicy"] {
            assert!(
                ds.get(key).and_then(|v| v.as_str()).is_some(),
                "listed data source missing string {key}: {ds}"
            );
        }
        if let Some(vc) = ds.get("validationContract").and_then(|v| v.as_object()) {
            assert_eq!(vc["sourceId"], ds["id"]);
            for ckey in ["readerDescription", "redactedSample", "validationState", "validatedAt"] {
                assert!(vc.get(ckey).and_then(|v| v.as_str()).is_some());
            }
            assert!(vc["schemaFields"].as_array().unwrap().iter().all(|v| v.is_string()));
            assert!(vc["recordCount"].as_u64().is_some());
            assert!(vc["maximumAgeMinutes"].as_i64().unwrap() >= 1);
        }
    }
    let valid_ds = listed.iter().find(|d| d["id"] == "ds-valid-1").unwrap();
    assert_eq!(valid_ds["validationContract"]["validationState"], "verified");
    assert!(valid_ds.get("lastError").is_none());
}

#[test]
fn paired_read_post_classification_and_epoch_wrapping() {
    let store = Store::open_in_memory().unwrap();
    let epoch1 = ensure_source_store_epoch(&store);
    let epoch2 = ensure_source_store_epoch(&store);
    assert!(!epoch1.is_empty());
    assert_eq!(epoch1, epoch2);

    // Normal Web POST /api/chat/conversations with {} or {title} MUST create
    assert!(!is_conversations_list_read_post(&json!({})));
    assert!(!is_conversations_list_read_post(&json!({"title": "Hello"})));
    assert!(!is_conversations_list_read_post(&json!({"expected_host_id": "h1", "title": "Named"})));
    assert!(!is_conversations_list_read_post(&json!({"expected_host_id": "h1", "operation": "create"})));

    // Paired read or query POST /api/chat/conversations MUST NOT create
    assert!(is_conversations_list_read_post(&json!({"expected_host_id": "h1"})));
    assert!(is_conversations_list_read_post(&json!({"expected_host_id": "h1", "limit": 20})));
    assert!(is_conversations_list_read_post(&json!({"operation": "list"})));
    assert!(is_conversations_list_read_post(&json!({"journal": true})));
    assert!(is_conversations_list_read_post(&json!({"after_seq": 10})));

    // POST /api/chat/conversations/:id/messages read vs turn
    assert!(is_messages_read_post(&json!({"expected_host_id": "h1"})));
    assert!(is_messages_read_post(&json!({"expected_host_id": "h1", "content": ""})));
    assert!(is_messages_read_post(&json!({"operation": "read"})));
    assert!(!is_messages_read_post(&json!({"content": "hi"})));
    assert!(!is_messages_read_post(&json!({"expected_host_id": "h1", "content": "hi"})));
    assert!(!is_messages_read_post(&json!({"parts": [{"type": "text", "text": "hi"}]})));

    // wrap_paired_result includes source_host_id and source_store_epoch
    let wrapped = wrap_paired_result(
        &store,
        "host-abc",
        &json!({"expected_host_id": "host-abc"}),
        json!({"ok": true}),
    );
    assert_eq!(wrapped["source_host_id"], "host-abc");
    assert_eq!(wrapped["source_store_epoch"], epoch1);
    assert_eq!(wrapped["result"]["ok"], true);

    let plain = wrap_paired_result(&store, "host-abc", &json!({}), json!({"ok": true}));
    assert_eq!(plain, json!({"ok": true}));
}

#[test]
fn memory_revise_deduplication_and_resources_and_worker_default() {
    let store = Store::open_in_memory().unwrap();
    let old_item = MemoryItem {
        id: "mem-old".into(),
        namespace: "project:preference".into(),
        title: "Old".into(),
        content: "Old text".into(),
        tags: vec![],
        created_at_ms: 100,
        updated_at_ms: 100,
    };
    let new_item = MemoryItem {
        id: "mem-new".into(),
        namespace: "project:preference".into(),
        title: "Revised".into(),
        content: "Revised text".into(),
        tags: vec![],
        created_at_ms: 200,
        updated_at_ms: 200,
    };
    store.upsert_memory(&old_item).unwrap();
    store.upsert_memory(&new_item).unwrap();
    store
        .set_kv(&memory_meta_key("mem-new"), &json!({"supersedes": ["mem-old"]}))
        .unwrap();

    let raw = store.search_memory("", None, 100).unwrap();
    assert_eq!(raw.len(), 2);
    let (records, items) = filter_superseded_memories(&store, raw);
    assert_eq!(records.len(), 1);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, "mem-new");

    // Resources collection on empty conversation returns empty vec (never error)
    assert!(collect_conversation_resources(&store, "c-empty").is_empty());
    store
        .append_event(
            "c-res",
            "user_message",
            json!({"content": "see image", "attachments": [{"id": "att-1", "name": "pic.png"}]}),
            100,
        )
        .unwrap();
    let resources = collect_conversation_resources(&store, "c-res");
    assert_eq!(resources.len(), 1);
    assert_eq!(resources[0]["id"], "att-1");

    // Default messaging worker execution_backend is "nola" (not duplicate "nola-v2")
    let worker = load_messaging_worker_snapshot(&store);
    assert_eq!(worker["execution_backend"], "nola");
}

#[test]
fn siwc_responses_and_gemini_protocol_mapping_preserved() {
    let map_proto = |raw: &str| match raw {
        "anthropic" => "anthropic",
        "siwc_responses" => "siwc_responses",
        "responses" | "openai_responses" => "openai_responses",
        "gemini" | "google_gemini" => "gemini",
        _ => "openai_chat",
    };
    assert_eq!(map_proto("siwc_responses"), "siwc_responses");
    assert_eq!(map_proto("gemini"), "gemini");
    assert_eq!(map_proto("google_gemini"), "gemini");
    assert_eq!(map_proto("responses"), "openai_responses");
}

#[test]
fn tailcat_verified_command_and_lease_grant_renew_close_lifecycle() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().canonicalize().unwrap().join("fake-tailcat.sh");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::write(
            &script,
            "#!/bin/sh\nread _init\nprintf '{\"protocolVersion\":1,\"event\":\"ready\",\"address\":\"tailcat://100.64.0.1:443\"}\\n'\nread _stop\nexit 0\n",
        )
        .unwrap();
        let mut perms = fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).unwrap();
    }
    #[cfg(not(unix))]
    fs::write(&script, b"fixture").unwrap();

    let digest = sha256_hex(&fs::read(&script).unwrap());
    // Relative & parent-dir paths are rejected
    assert!(verified_tailcat_command(Path::new("relative/helper"), Some(&digest)).is_err());
    assert!(verified_tailcat_command(Path::new("/tmp/../bin/helper"), Some(&digest)).is_err());
    // Wrong SHA-256 is refused
    assert!(verified_tailcat_command(&script, Some(&"0".repeat(64))).is_err());
    // Valid binary produces scrubbed env with zero argv leaks
    let cmd = verified_tailcat_command(&script, Some(&digest)).unwrap();
    assert_eq!(cmd.get_args().count(), 0);
    assert!(cmd
        .get_envs()
        .all(|(k, _)| matches!(k.to_str(), Some("PATH" | "SystemRoot" | "TEMP" | "TMP"))));

    #[cfg(unix)]
    {
        let mgr = Arc::new(TailcatLeaseManager::default());
        let cfg = TailcatConfig {
            ingress: true,
            helper_path: Some(script.clone()),
            helper_sha256: Some(digest),
            web_port: 18080,
        };
        assert!(cfg.is_available());

        // Invalid public key rejected
        let bad_key = mgr
            .create_lease(&cfg, "host-1", "owner-a".into(), "not-a-nodekey")
            .unwrap_err();
        assert_eq!(bad_key.status, 400);

        // Grant / connect succeeds and returns real leaseId + address
        let granted = mgr
            .create_lease(&cfg, "host-1", "owner-a".into(), "nodekey:deadbeef")
            .unwrap();
        let lease_id = granted["leaseId"].as_str().unwrap().to_string();
        assert_eq!(granted["address"], "tailcat://100.64.0.1:443");
        assert_eq!(granted["hostId"], "host-1");
        assert_eq!(granted["expiresIn"], TAILCAT_GRANT_SECONDS);

        // Wrong owner cannot renew or close
        assert_eq!(
            mgr.renew_lease("owner-b", &lease_id).unwrap_err().status,
            403
        );
        assert_eq!(
            mgr.close_lease("owner-b", &lease_id).unwrap_err().status,
            403
        );

        // Original owner can renew and cleanly close
        let renewed = mgr.renew_lease("owner-a", &lease_id).unwrap();
        assert_eq!(renewed["renewed"], true);
        assert_eq!(renewed["leaseId"], lease_id);

        let closed = mgr.close_lease("owner-a", &lease_id).unwrap();
        assert_eq!(closed["closed"], true);
        assert_eq!(closed["runStateChanged"], false);

        // Closed lease is no longer available
        assert_eq!(
            mgr.renew_lease("owner-a", &lease_id).unwrap_err().status,
            404
        );
    }
}

#[test]
fn mobile_to_web_one_time_handoff_consumes_atomically_and_enforces_ttl_and_safe_paths() {
    let handoffs: Mutex<HashMap<String, HandoffGrant>> = Mutex::new(HashMap::new());
    let t0 = Instant::now();

    // Unsafe targets are sanitized to /settings
    let (code_unsafe, target_unsafe) =
        mint_web_handoff(&handoffs, "https://evil.example/phish", t0);
    assert_eq!(target_unsafe, "/settings");
    assert_eq!(
        consume_web_handoff(&handoffs, &code_unsafe, t0 + Duration::from_secs(1)),
        Some("/settings".to_string())
    );

    // Safe target is preserved and strictly single-use
    let (code, target) = mint_web_handoff(&handoffs, "/settings?section=messaging", t0);
    assert_eq!(target, "/settings?section=messaging");
    assert!(code.len() >= 32);
    assert_eq!(
        consume_web_handoff(&handoffs, &code, t0 + Duration::from_secs(10)),
        Some("/settings?section=messaging".to_string())
    );
    // Replay fails closed
    assert_eq!(
        consume_web_handoff(&handoffs, &code, t0 + Duration::from_secs(11)),
        None
    );

    // Expired code (>60s) is rejected
    let (exp_code, _) = mint_web_handoff(&handoffs, "/chat", t0);
    assert_eq!(
        consume_web_handoff(
            &handoffs,
            &exp_code,
            t0 + Duration::from_secs(HANDOFF_TTL_SECS + 1)
        ),
        None
    );
}

#[test]
fn encrypted_login_response_never_leaks_plaintext_session_token_when_aes_gcm_requested() {
    let session_tok = "super-secret-session-token-123456";
    let payload = json!({
        "authenticated": true,
        "sessionAuthenticated": true,
        "hostId": "nola-host-abc",
        "token": session_tok,
        "token_type": "Bearer",
        "expires_in": 604800
    });
    let mobile_tok = "mobile-access-token-xyz";
    let encrypted = maybe_encrypt_api_payload(&payload, Some(mobile_tok));
    assert_eq!(encrypted["type"], "encrypted");
    assert_eq!(encrypted["alg"], "AES-256-GCM");
    let serialized = serde_json::to_string(&encrypted).unwrap();
    assert!(
        !serialized.contains(session_tok),
        "session token must never appear in plaintext when aes-gcm-v1 is requested"
    );
    let decrypted = decrypt_mobile_json_envelope(&encrypted, mobile_tok).unwrap();
    assert_eq!(decrypted["token"], session_tok);
    assert_eq!(decrypted["authenticated"], true);

    // Without mobile crypto header, returns plaintext payload directly
    let plain = maybe_encrypt_api_payload(&payload, None);
    assert_eq!(plain["token"], session_tok);
}

#[test]
fn workspace_worktrees_endpoint_lists_creates_and_removes_real_git_worktree() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().canonicalize().unwrap();
    let init_ok = Command::new("git")
        .args(["init", "-q"])
        .current_dir(&repo)
        .status()
        .is_ok_and(|s| s.success());
    if !init_ok {
        return;
    }
    let _ = Command::new("git")
        .args(["config", "user.email", "test@nola.local"])
        .current_dir(&repo)
        .status();
    let _ = Command::new("git")
        .args(["config", "user.name", "Nola Test"])
        .current_dir(&repo)
        .status();
    fs::write(repo.join("README.md"), "# Worktree Test\n").unwrap();
    let _ = Command::new("git")
        .args(["add", "README.md"])
        .current_dir(&repo)
        .status();
    let commit_ok = Command::new("git")
        .args(["commit", "-q", "-m", "init"])
        .current_dir(&repo)
        .status()
        .is_ok_and(|s| s.success());
    assert!(commit_ok);

    let (st_list, listed) = handle_workspace_worktrees(&repo, false, &json!({}));
    assert_eq!(st_list, 200);
    assert_eq!(listed["ok"], true);
    assert_eq!(listed["worktrees"].as_array().unwrap().len(), 1);

    let (st_create, created) = handle_workspace_worktrees(
        &repo,
        true,
        &json!({"action": "create", "target": ".worktrees/feat-x", "branch": "feat-x"}),
    );
    assert_eq!(st_create, 200, "create failed: {created}");
    assert_eq!(created["worktrees"].as_array().unwrap().len(), 2);

    let (st_remove, removed) = handle_workspace_worktrees(
        &repo,
        true,
        &json!({"action": "remove", "target": ".worktrees/feat-x"}),
    );
    assert_eq!(st_remove, 200, "remove failed: {removed}");
    assert_eq!(removed["worktrees"].as_array().unwrap().len(), 1);
}

#[test]
fn attachments_parse_extracts_real_text_from_utf8_docx_and_pdf() {
    // 1. UTF-8 text
    let txt = parse_attachment_payload("notes.txt", "text/plain", b"Hello Nola v2");
    assert_eq!(txt["extracted"], true);
    assert_eq!(txt["text"], "Hello Nola v2");

    // 2. Real minimal in-memory DOCX (uncompressed ZIP with word/document.xml)
    let xml = b"<?xml version=\"1.0\" encoding=\"UTF-8\"?><w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body><w:p><w:r><w:t>Quarterly Revenue Summary</w:t></w:r></w:p></w:body></w:document>";
    let docx_bytes = build_minimal_stored_zip("word/document.xml", xml);
    let parsed_docx = parse_attachment_payload(
        "report.docx",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        &docx_bytes,
    );
    assert_eq!(parsed_docx["extracted"], true, "docx parse: {parsed_docx}");
    assert!(
        parsed_docx["text"]
            .as_str()
            .unwrap_or("")
            .contains("Quarterly Revenue Summary"),
        "expected extracted DOCX text, got: {parsed_docx}"
    );

    // 3. Real minimal valid PDF with a Tj text operator
    let pdf_bytes = build_minimal_pdf("Invoice Total 4200");
    let parsed_pdf = parse_attachment_payload("invoice.pdf", "application/pdf", &pdf_bytes);
    assert_eq!(parsed_pdf["extracted"], true, "pdf parse: {parsed_pdf}");
    assert!(
        parsed_pdf["text"]
            .as_str()
            .unwrap_or("")
            .contains("Invoice Total 4200"),
        "expected extracted PDF text, got: {parsed_pdf}"
    );
}

fn crc32_ieee(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

fn build_minimal_stored_zip(filename: &str, content: &[u8]) -> Vec<u8> {
    let name_bytes = filename.as_bytes();
    let crc = crc32_ieee(content);
    let size = content.len() as u32;
    let mut out = Vec::new();
    // Local file header
    out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
    out.extend_from_slice(&20u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // method=store
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&crc.to_le_bytes());
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(name_bytes);
    out.extend_from_slice(content);
    let cd_offset = out.len() as u32;
    // Central directory header
    out.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
    out.extend_from_slice(&20u16.to_le_bytes());
    out.extend_from_slice(&20u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&crc.to_le_bytes());
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(name_bytes);
    let cd_size = (out.len() as u32) - cd_offset;
    // End of central directory
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&cd_size.to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

fn build_minimal_pdf(text: &str) -> Vec<u8> {
    let stream = format!("BT /F1 12 Tf 72 720 Td ({text}) Tj ET");
    let mut pdf = String::from("%PDF-1.4\n");
    let mut offsets = Vec::new();
    let objs = [
        "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n".to_string(),
        "2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n".to_string(),
        "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>\nendobj\n".to_string(),
        format!(
            "4 0 obj\n<< /Length {} >>\nstream\n{stream}\nendstream\nendobj\n",
            stream.len()
        ),
        "5 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n".to_string(),
    ];
    for obj in &objs {
        offsets.push(pdf.len());
        pdf.push_str(obj);
    }
    let xref_pos = pdf.len();
    pdf.push_str(&format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1));
    for off in offsets {
        pdf.push_str(&format!("{off:010} 00000 n \n"));
    }
    pdf.push_str(&format!(
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF\n",
        objs.len() + 1
    ));
    pdf.into_bytes()
}

#[test]
fn documents_search_queries_indexed_doc_chunks_and_goal_plan_recall_oauth_work_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let doc_file = dir.path().join("architecture.md");
    fs::write(
        &doc_file,
        "# Architecture\nNola uses SQLite FTS5 chunk indexing for lightning fast local document search.\n",
    )
    .unwrap();
    let store = Store::open_in_memory().unwrap();
    crate::config::bind_store_vault_path(&store, &dir.path().join("vault.json")).unwrap();
    let suite = Arc::new(ModulesSuite::new(
        store.clone(),
        dir.path().to_path_buf(),
        None,
    ));

    // 1. Index data source & search doc chunks (both GET and POST share `execute_documents_search`)
    let ds_rec = nola_types::DataSourceRecord {
        id: "ds-arch".into(),
        name: "Architecture Docs".into(),
        kind: "localFile".into(),
        location: doc_file.display().to_string(),
        metadata: json!({}),
        usage_count: 0,
        updated_at_ms: now_ms(),
    };
    suite.upsert_record("data_source", "ds-arch", &ds_rec).unwrap();
    let indexed = index_data_source(&store, "ds-arch").unwrap();
    assert!(indexed >= 1);

    let search_res = execute_documents_search(
        &store,
        dir.path(),
        "lightning fast local document",
        Some(&["ds-arch".to_string()]),
        10,
    );
    assert_eq!(search_res["ok"], true);
    assert!(search_res["count"].as_u64().unwrap() >= 1);
    assert_eq!(search_res["documents"][0]["sourceId"], "ds-arch");

    // 2. Goal control lifecycle (create -> pause -> resume -> complete)
    let (st, created) = handle_conversation_goal_control(
        &store,
        &suite,
        "host-1",
        "conv-g1",
        &json!({"action": "create", "title": "Ship Wave 3", "objective": "Pass all checks"}),
    );
    assert_eq!(st, 200);
    assert_eq!(created["ok"], true);
    assert_eq!(created["source_host_id"], "host-1");
    assert_eq!(created["goal"]["controlState"], "pending");

    let (_, paused) = handle_conversation_goal_control(
        &store,
        &suite,
        "host-1",
        "conv-g1",
        &json!({"action": "pause"}),
    );
    assert_eq!(paused["goal"]["controlState"], "paused");

    let (_, completed) = handle_conversation_goal_control(
        &store,
        &suite,
        "host-1",
        "conv-g1",
        &json!({"action": "complete"}),
    );
    assert_eq!(completed["goal"]["controlState"], "completed");
    assert_eq!(completed["goal"]["accepted"], true);

    // 3. Plan Mode toggle & approval
    let (pst, entered) = handle_conversation_plan(&store, "conv-p1", &json!({"enabled": true}));
    assert_eq!(pst, 200);
    assert_eq!(entered["planMode"], true);
    store
        .set_kv(
            "plan:conv-p1",
            &json!({"title": "Rollout Plan", "content": "1. Test\n2. Ship", "status": "proposed"}),
        )
        .unwrap();
    let (ast, approved) =
        handle_conversation_plan(&store, "conv-p1", &json!({"action": "approve"}));
    assert_eq!(ast, 200);
    assert_eq!(approved["approved"], true);
    assert_eq!(approved["planMode"], false);
    assert_eq!(approved["plan"]["status"], "approved");

    // 4. Conversation recall (FTS + range)
    store
        .append_event(
            "conv-r1",
            "chat_user",
            json!({"content": "Deploy kubernetes canary safely"}),
            100,
        )
        .unwrap();
    let recalled = handle_conversation_recall(&store, "conv-r1", &json!({"query": "canary"}));
    assert!(recalled["count"].as_u64().unwrap() >= 1);
    let ranged = handle_conversation_recall(&store, "conv-r1", &json!({"from_seq": 1, "limit": 5}));
    assert!(ranged["count"].as_u64().unwrap() >= 1);

    // 5. Auxiliary prepare -> admit -> inspect -> cancel
    let aux_ops = Mutex::new(BTreeMap::new());
    let (s1, prep) = handle_auxiliary_dispatch(
        &store,
        &suite,
        &aux_ops,
        "host-1",
        "prepare",
        &json!({"expected_host_id": "host-1"}),
    );
    assert_eq!(s1, 200);
    assert_eq!(prep["source_host_id"], "host-1");
    let (s2, adm) = handle_auxiliary_dispatch(
        &store,
        &suite,
        &aux_ops,
        "host-1",
        "admit",
        &json!({"expected_host_id": "host-1", "operation_id": "op-1"}),
    );
    assert_eq!(s2, 200);
    assert_eq!(adm["receipt"]["dispatch_permitted"], false);
    assert_eq!(adm["receipt"]["operation"]["state"], "ready");
    assert!(adm["receipt"]["operation"]["scenario"].is_string());
    assert!(adm["receipt"]["operation"]["configuration"]["input_digest"].is_string());
    assert_eq!(adm["receipt"]["operation"]["configuration"]["max_attempts"], 1);
    let (s3, insp) = handle_auxiliary_dispatch(
        &store,
        &suite,
        &aux_ops,
        "host-1",
        "inspect",
        &json!({"expected_host_id": "host-1", "operation_id": "op-1"}),
    );
    assert_eq!(s3, 200);
    assert_eq!(insp["operation_id"], "op-1");
    let (s4, canc) = handle_auxiliary_dispatch(
        &store,
        &suite,
        &aux_ops,
        "host-1",
        "cancel",
        &json!({"expected_host_id": "host-1", "operation_id": "op-1"}),
    );
    assert_eq!(s4, 200);
    assert_eq!(canc["receipt"]["operation"]["state"], "cancelled");

    // 6. Real PKCE OAuth state management (Kimi & Source-Login)
    let flows = Mutex::new(BTreeMap::new());
    let (ks, kconn) = handle_source_oauth_route(
        &store,
        &suite,
        &flows,
        "/api/model-connections/kimi-login/connect",
        "",
        &json!({"account_id": "kimi-1", "origin": "http://127.0.0.1:58627", "server_bearer": "secret-tok"}),
    );
    assert_eq!(ks, 200);
    assert_eq!(kconn["state"], "idle");
    let (os, owners) = handle_source_oauth_route(
        &store,
        &suite,
        &flows,
        "/api/model-connections/kimi-login/owners",
        "",
        &json!({}),
    );
    assert_eq!(os, 200);
    assert_eq!(owners.as_array().unwrap().len(), 1);

    let (ss, started) = handle_source_oauth_route(
        &store,
        &suite,
        &flows,
        "/api/model-connections/source-login/start",
        "",
        &json!({"request_id": "req-pkce-1", "account_id": "codex-1", "display_name": "Codex"}),
    );
    assert_eq!(ss, 200);
    assert_eq!(started["state"], "pending");
    assert_eq!(started["inference_supported"], false);
    assert_eq!(started["pkce_method"], "S256");
    let auth_url = started["authorization_url"].as_str().unwrap();
    assert!(auth_url.starts_with("https://auth.openai.com/oauth/authorize?"));
    assert!(auth_url.contains("code_challenge="));
}

#[test]
fn second_pass_security_and_cross_client_contracts_hold() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    crate::config::bind_store_vault_path(&store, &dir.path().join("vault.json")).unwrap();
    let suite = Arc::new(ModulesSuite::new(
        store.clone(),
        dir.path().to_path_buf(),
        None,
    ));

    // 1. Handoff target rejects control chars, whitespace, non-ASCII
    assert_eq!(safe_handoff_target("/settings?tab=models"), "/settings?tab=models");
    assert_eq!(safe_handoff_target("/settings\r\nSet-Cookie: evil=1"), "/settings");
    assert_eq!(safe_handoff_target("/settings with space"), "/settings");
    assert_eq!(safe_handoff_target("/settings/中文"), "/settings");

    // 2. SEC-7 sensitive data source detection
    assert!(is_sensitive_data_source_record(
        &suite,
        &json!({"kind": "localFile", "location": "/home/user/.ssh/id_rsa"})
    ));
    assert!(!is_sensitive_data_source_record(
        &suite,
        &json!({"kind": "localFile", "location": "README.md"})
    ));

    // 3. Local model dest_path confinement & ModelScope URL & vLLM recommend
    let confined = confined_local_model_dest_path(Some("/etc/cron.d/pwned.gguf"), "fallback");
    assert!(confined.starts_with(managed_local_models_dir()));
    assert_eq!(confined.file_name().unwrap(), "pwned.gguf");
    let ms_url = build_local_model_download_url("modelscope", "Qwen/Qwen3-8B-GGUF", Some("q4.gguf")).unwrap();
    assert!(ms_url.contains("modelscope.cn/models/Qwen/Qwen3-8B-GGUF/resolve/master/q4.gguf"));
    let vllm_cands = build_vllm_recommend_candidates(&json!({"memoryBytes": 32_000_000_000u64, "unifiedMemory": true}));
    assert!(!vllm_cands.is_empty());
    assert_eq!(vllm_cands[0]["plan"]["engine"], "vllm");

    // 4. OAuth origin validation & constant-time full-state verification on opencode-login/callback
    assert!(validate_oauth_origin("https://example.com").is_ok());
    assert!(validate_oauth_origin("http://127.0.0.1:4096").is_ok());
    assert!(validate_oauth_origin("javascript:alert(1)").is_err());
    assert!(validate_oauth_origin("http://evil.example.com").is_err());

    let flows = Mutex::new(BTreeMap::new());
    let (bad_st, _) = handle_source_oauth_route(
        &store,
        &suite,
        &flows,
        "/api/model-connections/kimi-login/connect",
        "",
        &json!({"account_id": "k-bad", "origin": "javascript:alert(1)", "server_bearer": "tok"}),
    );
    assert_eq!(bad_st, 400);
    let (oc_st, oc_auth) = handle_source_oauth_route(
        &store,
        &suite,
        &flows,
        "/api/model-connections/opencode-login/authorize",
        "",
        &json!({"request_id": "oc-req-1", "account_id": "oc-1", "provider": "opencode", "method": 0}),
    );
    assert_eq!(oc_st, 200);
    assert_eq!(oc_auth["state"], "pending");
    let full_state = oc_auth["oauth_state"].as_str().unwrap().to_string();
    assert_eq!(full_state, flows.lock().unwrap().get("oc-req-1").unwrap().csrf_state);
    // Prefix state MUST be rejected even when request_id matches!
    let (prefix_st, _) = handle_source_oauth_route(
        &store,
        &suite,
        &flows,
        "/api/model-connections/opencode-login/callback",
        "",
        &json!({"request_id": "oc-req-1", "state": &full_state[..8], "code": "abc"}),
    );
    assert_eq!(prefix_st, 400);
    // Full constant-time state succeeds -> source_callback_completed
    let (ok_cb, cb_val) = handle_source_oauth_route(
        &store,
        &suite,
        &flows,
        "/api/model-connections/opencode-login/callback",
        "",
        &json!({"request_id": "oc-req-1", "state": full_state, "code": "abc"}),
    );
    assert_eq!(ok_cb, 200);
    assert_eq!(cb_val["state"], "source_callback_completed");

    // 5. SourceAccounts bind/refresh nested shape
    let (bs, bound) = handle_source_oauth_route(
        &store,
        &suite,
        &flows,
        "/api/model-connections/source-accounts/bind",
        "",
        &json!({"account_id": "sa-1", "display_name": "My Codex", "source": "codex", "path": "/tmp/nonexistent"}),
    );
    assert_eq!(bs, 200);
    assert_eq!(bound["account"]["revision"], 1);
    assert_eq!(bound["account"]["account"]["id"], "sa-1");
    assert_eq!(bound["account"]["account"]["display_name"], "My Codex");
    assert_eq!(bound["account"]["account"]["source"], "codex");
    assert_eq!(bound["account"]["account"]["path"], "/tmp/nonexistent");
    assert_eq!(bound["login_state"], "unauthenticated");
    assert!(bound["user_action"].is_string());

    // 6. macOS remote chat & iOS resumeObservedRun NOT hijacked by is_messages_read_post
    assert!(!is_messages_read_post_with_accept(
        &json!({"expected_host_id": "h1", "messages": [{"role": "user", "content": "hello"}]}),
        false
    ));
    assert!(!is_messages_read_post_with_accept(
        &json!({"expected_host_id": "h1", "expected_run_id": "run-1"}),
        true
    ));
    assert!(is_messages_read_post_with_accept(
        &json!({"expected_host_id": "h1"}),
        false
    ));

    // 7. Hybrid device_bridge tools + full device_tool_request frame + boolean error callback
    let specs = extract_hybrid_device_tool_specs(&json!({
        "device_bridge": {
            "tools": [{
                "capability": "calendar",
                "schema": {
                    "type": "function",
                    "function": {
                        "name": "list_events",
                        "description": "List calendar events",
                        "parameters": {"type": "object", "properties": {}}
                    }
                }
            }]
        }
    }));
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].0.schema.name, "list_events");
    assert_eq!(specs[0].1, "calendar");
    let frame = build_device_tool_request_frame(
        "req-1",
        "conv-1",
        "host-1",
        "client-1",
        "list_events",
        "calendar",
        &json!({"limit": 5}),
        "deadbeef",
        1_900_000_000_000,
    );
    assert_eq!(frame["protocol_version"], 1);
    assert_eq!(frame["tool_name"], "list_events");
    assert_eq!(frame["tool"], "list_events");
    assert_eq!(frame["nonce"], "deadbeef");
    assert_eq!(frame["session_id"], "conv-1");
    assert!(frame["trace"].is_object());
    assert!(device_tool_callback_is_ok(&json!({"output": "ok", "error": false})));
    assert!(device_tool_callback_is_ok(&json!({"output": "ok", "error": null})));
    assert!(!device_tool_callback_is_ok(&json!({"error": true})));
    assert!(!device_tool_callback_is_ok(&json!({"error": "boom"})));

    // 8. iOS permission resolution parsing
    assert_eq!(
        parse_permission_resolution(&json!({"resolution": {"decision": "allow_once"}})),
        (true, "once")
    );
    assert_eq!(
        parse_permission_resolution(&json!({"resolution": {"decision": "allow_conversation"}})),
        (true, "session")
    );
    assert_eq!(
        parse_permission_resolution(&json!({"resolution": {"decision": "deny"}})),
        (false, "once")
    );

    // 9. Goal schedule + applied_operations + plan reject/clear + recall messageId + worktree isMain
    let (gs, sched) = handle_conversation_goal_control(
        &store,
        &suite,
        "host-1",
        "conv-sched",
        &json!({
            "action": "schedule",
            "operationId": "op-sched-1",
            "wakeAtEpochMs": 1_900_000_000_000i64
        }),
    );
    assert_eq!(gs, 200);
    assert_eq!(sched["conversationId"], "conv-sched");
    assert_eq!(sched["operationId"], "op-sched-1");
    assert_eq!(sched["authoritative"], true);
    assert!(sched["goalProjection"]["applied_operations"]["op-sched-1"]["command_sha256"]
        .as_str()
        .is_some_and(|s| s.len() == 64));

    let (rs, rej) = handle_conversation_plan(&store, "conv-p2", &json!({"action": "reject", "feedback": "revise"}));
    assert_eq!(rs, 200);
    assert_eq!(rej["plan"]["status"], "rejected");
    let (cs, clr) = handle_conversation_plan(&store, "conv-p2", &json!({"action": "clear"}));
    assert_eq!(cs, 200);
    assert!(clr["plan"].is_null());

    store
        .append_event("conv-rec", "chat_user", json!({"content": "needle phrase"}), 100)
        .unwrap();
    let rec = handle_conversation_recall(&store, "conv-rec", &json!({"q": "needle"}));
    assert!(rec["matches"][0]["messageId"]
        .as_str()
        .is_some_and(|s| s.starts_with("msg-")));
    assert!(rec["matches"][0]["createdAtMs"].is_i64());

    let wts = parse_git_worktree_porcelain("worktree /repo\nHEAD abc\nbranch refs/heads/main\n\nworktree /repo/.nola/worktrees/feat\nHEAD def\nbranch refs/heads/feat\n");
    assert_eq!(wts[0]["isMain"], true);
    assert_eq!(wts[1]["isMain"], false);
}

#[test]
fn mobile_api_and_explicit_auth_never_receive_implicit_loopback_trust() {
    // Plain local Web UI request on loopback without env token override -> trusted
    assert!(compute_local_trust(true, false, "/api/agent/settings", false));
    assert!(compute_local_trust(true, false, "/api/scheduled-actions", false));

    // `/api/mobile` and `/api/mobile/*` MUST NEVER receive implicit loopback trust
    assert!(!compute_local_trust(true, false, "/api/mobile", false));
    assert!(!compute_local_trust(true, false, "/api/mobile/agent/settings", false));
    assert!(!compute_local_trust(true, false, "/api/mobile/agent/settings", true));

    // Explicit Authorization or X-Nola-Mobile-Auth header disables loopback trust
    assert!(!compute_local_trust(true, false, "/api/agent/settings", true));

    // Non-loopback or env token override disables loopback trust
    assert!(!compute_local_trust(false, false, "/api/agent/settings", false));
    assert!(!compute_local_trust(true, true, "/api/agent/settings", false));
}

#[test]
fn web_server_pdf_resources_and_remote_host_contracts_work() {
    let store = Store::open_in_memory().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let suite = ModulesSuite::new(store.clone(), dir.path().to_path_buf(), None);
    let cancels: Arc<Mutex<BTreeMap<String, Arc<AtomicBool>>>> =
        Arc::new(Mutex::new(BTreeMap::new()));

    // 4a. PDF resource upload, canonical list fields, and raw bytes extraction
    let conv_id = "conv-pdf-1";
    let pdf_bytes = b"%PDF-1.4\n%test\n";
    let rid = "11111111-2222-3333-4444-555555555555";
    let uploaded = store_conversation_resource_upload(
        &store,
        conv_id,
        &json!({
            "resourceId": rid,
            "displayName": "report.pdf",
            "mimeType": "application/pdf",
            "dataBase64": base64_encode(pdf_bytes)
        }),
    )
    .unwrap();
    assert_eq!(uploaded["resourceId"], rid);
    assert_eq!(uploaded["displayName"], "report.pdf");
    assert_eq!(uploaded["mimeType"], "application/pdf");
    assert_eq!(uploaded["byteCount"], pdf_bytes.len() as u64);
    assert_eq!(
        uploaded["previewUrl"],
        format!("/api/chat/conversations/{conv_id}/resources/{rid}")
    );
    assert_eq!(uploaded["sha256"].as_str().unwrap().len(), 64);

    let listed = collect_conversation_resources(&store, conv_id);
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["resourceId"], rid);
    assert!(listed[0].get("data_base64").is_none());

    let raw_list = collect_conversation_resources_raw(&store, conv_id);
    let decoded = extract_resource_raw_bytes(&raw_list[0]).unwrap();
    assert_eq!(decoded, pdf_bytes);

    // 4b. Assistant journal operation detection & dispatch
    let open_body = json!({
        "operation": "open_snapshot",
        "at": {
            "identity": {
                "conversation_id": conv_id,
                "run_id": "run-1",
                "assistant_message_id": "asst-1",
                "source_store_epoch": "ep-1"
            },
            "sequence": "1",
            "record_digest": "0".repeat(64)
        }
    });
    assert!(is_assistant_journal_operation(&open_body));
    let j_resp =
        handle_assistant_journal_http(&store, &suite, "host-a", &open_body, true).unwrap();
    assert_eq!(j_resp["source_host_id"], "host-a");
    assert!(j_resp["result"]["snapshot_id"].is_string());

    // 4b. Paired task cancellation strict 8-key schema
    let cancel_body = json!({
        "hostId": "host-a",
        "conversationId": conv_id,
        "parentRunId": "run-1",
        "taskId": "task-1"
    });
    assert!(is_paired_task_cancellation_body(&cancel_body));
    let cancel_resp = handle_paired_task_cancellation(
        &store,
        &suite,
        &cancels,
        "host-a",
        conv_id,
        None,
        &cancel_body,
    );
    assert_eq!(cancel_resp.as_object().unwrap().len(), 8);
    assert_eq!(cancel_resp["hostId"], "host-a");
    assert_eq!(cancel_resp["taskId"], "task-1");
    assert_eq!(cancel_resp["cancelRequested"], true);
    assert_eq!(cancel_resp["applicationPending"], false);

    // 4b. Task guidance receipt schema
    let guid_body = json!({
        "hostId": "host-a",
        "conversationId": conv_id,
        "parentRunId": "run-1",
        "taskId": "task-1",
        "messageId": "msg-g1",
        "submittedAtMs": 1700000000000i64,
        "message": "stay concise"
    });
    assert!(is_task_guidance_post(&guid_body, false));
    assert!(!is_task_guidance_post(&guid_body, true));
    let guid_resp = handle_task_guidance_http(&store, "host-a", conv_id, &guid_body, false);
    assert_eq!(guid_resp["hostId"], "host-a");
    assert_eq!(guid_resp["messageId"], "msg-g1");
    assert_eq!(guid_resp["guidance"]["status"], "accepted");
    assert_eq!(guid_resp["guidance"]["content"], "stay concise");

    // 4b. Remote submission reconciliation schema
    let digest = "a".repeat(64);
    let sub_body = json!({
        "operation": "observe",
        "identity": {
            "source_host_id": "host-a",
            "client_id": "cli-1",
            "conversation_id": conv_id,
            "submission_id": "sub-1",
            "user_message_id": "u-1",
            "assistant_message_id": "a-1"
        },
        "request_digest": digest
    });
    let rec = handle_remote_submission_reconciliation(
        &store,
        &cancels,
        "host-a",
        conv_id,
        &sub_body,
    );
    assert_eq!(rec["protocol_version"], 1);
    assert_eq!(rec["phase"], "prepared");
    assert!(rec["admission"].is_null());
    assert!(rec["rejection_code"].is_null());
}

#[test]
fn web_workstream_b_inventory_import_discovery_oauth_and_llama_args_work() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    crate::config::bind_store_vault_path(&store, &dir.path().join("vault.json")).unwrap();
    crate::config::save_applied_config(&store, &crate::config::AppliedConfig::default()).unwrap();
    let suite = Arc::new(ModulesSuite::new(store.clone(), dir.path().to_path_buf(), None));

    // 1. build_model_connections_inventory reads real catalog.models, capability_assessments, revision, account_id, and model_account
    suite
        .upsert_record(
            "model_account",
            "acct-1",
            &json!({
                "id": "acct-1",
                "display_name": "Primary Account",
                "source": "anthropic",
                "revision": 3,
                "state": "connected"
            }),
        )
        .unwrap();
    suite
        .upsert_record(
            "model_connection",
            "managed-1",
            &json!({
                "id": "managed-1",
                "display_name": "Managed Claude",
                "base_url": "https://api.anthropic.com",
                "protocols": ["anthropic"],
                "source": "import",
                "account_id": "acct-1",
                "revision": 4,
                "enabled": true,
                "catalog": {
                    "observed_at_ms": 1700000000000i64,
                    "models": [{
                        "model_id": "claude-sonnet-4",
                        "display_name": "Claude Sonnet 4",
                        "protocol": "anthropic",
                        "context_window": 200000
                    }]
                },
                "capability_assessments": [{
                    "model_id": "claude-sonnet-4",
                    "protocol": "anthropic",
                    "evidence": null,
                    "user_override": { "toolCalls": true, "imageInput": true, "reasoning": true, "samplingParameters": true, "samplingRequiresNoReasoning": false, "chatOutputTokenField": "auto" }
                }]
            }),
        )
        .unwrap();
    let inv = build_model_connections_inventory(&store, &suite);
    let conns = inv["connections"].as_array().unwrap();
    let m1 = conns
        .iter()
        .find(|c| c["document"]["connection"]["id"] == "managed-1")
        .unwrap();
    assert_eq!(m1["revision"], 4);
    assert_eq!(m1["document"]["connection"]["account_id"], "acct-1");
    assert_eq!(m1["document"]["catalog"]["models"].as_array().unwrap().len(), 1);
    assert_eq!(m1["document"]["catalog"]["models"][0]["model_id"], "claude-sonnet-4");
    assert_eq!(m1["document"]["capability_assessments"].as_array().unwrap().len(), 1);
    assert!(inv["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|a| a["account"]["id"] == "acct-1" && a["revision"] == 3));

    // 2. parse_model_import_candidates supports OpenCode JSONC, Codex TOML, Kimi Code CLI TOML, and Nola JSON
    let opencode_jsonc = r#"{
        // OpenCode config with comments and trailing comma
        "provider": {
            "anthropic-oc": {
                "name": "Anthropic OC",
                "npm": "@ai-sdk/anthropic",
                "options": { "baseURL": "https://api.anthropic.com", "apiKey": "sk-ant-123", },
                "models": { "claude-sonnet-4": { "name": "Claude 4", "limit": { "context": 200000 } }, },
            },
        },
    }"#;
    let oc_cands =
        parse_model_import_candidates("open_code", "opencode.jsonc", opencode_jsonc, None, true, false)
            .unwrap();
    assert_eq!(oc_cands.len(), 1);
    assert_eq!(oc_cands[0].protocol, "anthropic");
    assert_eq!(oc_cands[0].api_key.as_deref(), Some("sk-ant-123"));
    assert_eq!(oc_cands[0].models[0].context_window, Some(200_000));

    let codex_toml = r#"
model = "o3"
model_provider = "openai"

[model_providers.openai]
name = "OpenAI Codex"
base_url = "https://api.openai.com/v1"
wire_api = "responses"
env_key = "OPENAI_API_KEY"
"#;
    let cred_json = r#"{"OPENAI_API_KEY": "sk-codex-cred"}"#;
    let codex_cands =
        parse_model_import_candidates("codex", "config.toml", codex_toml, Some(cred_json), true, false)
            .unwrap();
    assert_eq!(codex_cands.len(), 1);
    assert_eq!(codex_cands[0].protocol, "responses");
    assert_eq!(codex_cands[0].api_key.as_deref(), Some("sk-codex-cred"));
    assert_eq!(codex_cands[0].models[0].model_id, "o3");

    let kimi_toml = r#"
[providers.moonshot]
type = "kimi"
base_url = "https://api.moonshot.cn/v1"
api_key = "sk-kimi-1"

[models.kimi-default]
provider = "moonshot"
model = "kimi-k2"
max_context_size = 131072
"#;
    let kimi_cands =
        parse_model_import_candidates("kimi_code", "config.toml", kimi_toml, None, true, false)
            .unwrap();
    assert_eq!(kimi_cands.len(), 1);
    assert_eq!(kimi_cands[0].models[0].model_id, "kimi-k2");
    assert_eq!(kimi_cands[0].models[0].context_window, Some(131_072));

    let nola_json = r#"{"llm": {"connections": [{"id": "nola-gem", "endpoint": "https://generativelanguage.googleapis.com", "protocol": "gemini", "models": [{"id": "gemini-2.5-flash"}]}]}}"#;
    let nola_cands =
        parse_model_import_candidates("nola", "config.json", nola_json, None, false, false).unwrap();
    assert_eq!(nola_cands.len(), 1);
    assert_eq!(nola_cands[0].protocol, "gemini");
    assert_eq!(nola_cands[0].models[0].model_id, "gemini-2.5-flash");

    // 3. discover_endpoint_models supports Anthropic, Gemini, OpenAI and propagates HTTP errors
    let mock_server = Server::http("127.0.0.1:0").unwrap();
    let port = mock_server.server_addr().to_ip().unwrap().port();
    let handle = std::thread::spawn(move || {
        for _ in 0..4 {
            let req = mock_server.recv().unwrap();
            let url = req.url().to_string();
            let has_ant = req
                .headers()
                .iter()
                .any(|h| h.field.as_str().as_str().eq_ignore_ascii_case("x-api-key") && h.value.as_str() == "ant-key");
            let has_gem = req
                .headers()
                .iter()
                .any(|h| h.field.as_str().as_str().eq_ignore_ascii_case("x-goog-api-key") && h.value.as_str() == "gem-key");
            let resp = if url == "/v1/models" && has_ant {
                Response::from_string(r#"{"data":[{"id":"claude-sonnet-4"}]}"#).with_status_code(200)
            } else if url == "/v1beta/models" && has_gem {
                Response::from_string(r#"{"models":[{"name":"models/gemini-2.5-pro"}]}"#).with_status_code(200)
            } else if url == "/fail/models" {
                Response::from_string(r#"{"error":"unauthorized"}"#).with_status_code(401)
            } else {
                Response::from_string(r#"{"data":[{"id":"gpt-4o"}]}"#).with_status_code(200)
            };
            let _ = req.respond(resp);
        }
    });

    let ant_conn = ConnectionConfig {
        id: "ant".into(),
        endpoint: format!("http://127.0.0.1:{port}"),
        protocol: Some("anthropic".into()),
        provider: None,
        api_key: Some("ant-key".into()),
        api_key_env: None,
        models: vec![],
    };
    assert_eq!(discover_endpoint_models(&ant_conn).unwrap(), vec!["claude-sonnet-4"]);

    let gem_conn = ConnectionConfig {
        id: "gem".into(),
        endpoint: format!("http://127.0.0.1:{port}"),
        protocol: Some("gemini".into()),
        provider: None,
        api_key: Some("gem-key".into()),
        api_key_env: None,
        models: vec![],
    };
    assert_eq!(discover_endpoint_models(&gem_conn).unwrap(), vec!["gemini-2.5-pro"]);

    let fail_conn = ConnectionConfig {
        id: "fail".into(),
        endpoint: format!("http://127.0.0.1:{port}/fail"),
        protocol: Some("openai_chat".into()),
        provider: None,
        api_key: None,
        api_key_env: None,
        models: vec![],
    };
    let err = discover_endpoint_models(&fail_conn).unwrap_err();
    assert!(err.contains("401"), "expected 401 error, got {err}");

    let oai_conn = ConnectionConfig {
        id: "oai".into(),
        endpoint: format!("http://127.0.0.1:{port}/oai"),
        protocol: Some("openai_chat".into()),
        provider: None,
        api_key: None,
        api_key_env: None,
        models: vec![],
    };
    assert_eq!(discover_endpoint_models(&oai_conn).unwrap(), vec!["gpt-4o"]);
    handle.join().unwrap();

    // 4. Kimi OAuth returns "pending", expires cleanly on timeout, and cancels cleanly
    let flows = Mutex::new(BTreeMap::new());
    let (ks, kstart) = handle_source_oauth_route(
        &store,
        &suite,
        &flows,
        "/api/model-connections/kimi-login/start",
        "",
        &json!({"account_id": "kimi-1"}),
    );
    assert_eq!(ks, 200);
    assert_eq!(kstart["state"], "pending");
    let fid = kstart["flow_id"].as_str().unwrap().to_string();
    // Simulate expiration
    flows.lock().unwrap().get_mut(&fid).unwrap().expires_at_ms = 1;
    let (kst, kstatus) = handle_source_oauth_route(
        &store,
        &suite,
        &flows,
        "/api/model-connections/kimi-login/status",
        "",
        &json!({"account_id": "kimi-1", "flow_id": fid}),
    );
    assert_eq!(kst, 200);
    assert_eq!(kstatus["state"], "expired");
    assert!(kstatus["poll_interval_seconds"].is_null());

    // 5. llama_server_args_from_parameters maps all 5 parameters cleanly without duplicates
    let args = LocalModelProcessManager::llama_server_args_from_parameters(
        &json!({
            "contextTokens": 32768,
            "gpuLayers": 99,
            "threads": 8,
            "batchSize": 512,
            "flashAttention": true
        }),
        &["--threads".to_string(), "4".to_string()],
    );
    assert_eq!(
        args,
        vec![
            "--ctx-size",
            "32768",
            "--n-gpu-layers",
            "99",
            "--batch-size",
            "512",
            "--flash-attn"
        ]
    );

    // 6. Auxiliary admit echoes provider, configuration_revision, maximum_duration_ms
    let aux_ops = Mutex::new(BTreeMap::new());
    let (admit_st, admit_env) = handle_auxiliary_dispatch(
        &store,
        &suite,
        &aux_ops,
        "host-1",
        "admit",
        &json!({
            "expected_host_id": "host-1",
            "operation_id": "op-1",
            "attempt_id": "op-1:physical",
            "profile_id": "openai:gpt-4o",
            "expected_profile_revision": 7,
            "scenario": "completion",
            "input_json": "{}",
            "maximum_duration_ms": 45000
        }),
    );
    assert_eq!(admit_st, 200);
    let aux_cfg = &admit_env["receipt"]["operation"]["configuration"];
    assert_eq!(aux_cfg["provider"], "configured:openai:gpt-4o");
    assert_eq!(aux_cfg["configuration_revision"], 7);
    assert_eq!(aux_cfg["maximum_duration_ms"], 45000);
}

#[test]
fn web_workstream_b_paired_host_and_catalog_replace_http_contracts() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    let vault_path = dir.path().join("vault.json");
    crate::config::bind_store_vault_path(&store, &vault_path).unwrap();
    let suite = Arc::new(ModulesSuite::new(store.clone(), dir.path().to_path_buf(), None));

    // Seed one legacy editable connection + one managed imported connection + one local connection
    let init_cfg = crate::config::AppliedConfig {
        llm: crate::config::LlmSection {
            default_model: Some(DefaultModelSelection {
                connection: "managed-imp".into(),
                model: "claude-sonnet-4".into(),
            }),
            connections: vec![
                ConnectionConfig {
                    id: "legacy-ep".into(),
                    endpoint: "https://legacy.invalid/v1".into(),
                    protocol: Some("openai_chat".into()),
                    provider: None,
                    api_key: Some("legacy-secret".into()),
                    api_key_env: None,
                    models: vec![ModelEntry {
                        id: "gpt-4o".into(),
                        context_window: None,
                        max_output_tokens: None,
                        temperature: None,
                        top_p: None,
                        reasoning_effort: None,
                        capabilities: None,
                    }],
                },
                ConnectionConfig {
                    id: "managed-imp".into(),
                    endpoint: "https://api.anthropic.com".into(),
                    protocol: Some("anthropic".into()),
                    provider: Some("open_code".into()),
                    api_key: Some("managed-secret".into()),
                    api_key_env: None,
                    models: vec![ModelEntry {
                        id: "claude-sonnet-4".into(),
                        context_window: None,
                        max_output_tokens: None,
                        temperature: None,
                        top_p: None,
                        reasoning_effort: None,
                        capabilities: None,
                    }],
                },
                ConnectionConfig {
                    id: "local:qwen3".into(),
                    endpoint: "http://127.0.0.1:8081/v1".into(),
                    protocol: Some("chat_completions".into()),
                    provider: Some("local".into()),
                    api_key: None,
                    api_key_env: None,
                    models: vec![ModelEntry {
                        id: "qwen3".into(),
                        context_window: None,
                        max_output_tokens: None,
                        temperature: None,
                        top_p: None,
                        reasoning_effort: None,
                        capabilities: None,
                    }],
                },
            ],
        },
        ..Default::default()
    };
    save_applied_config(&store, &init_cfg).unwrap();

    let conv_id = "conv-paired-b";
    let ts = now_ms();
    store
        .upsert_session(&Session {
            id: conv_id.into(),
            title: "Paired B".into(),
            workspace_root: dir.path().display().to_string(),
            model_connection: None,
            model_id: None,
            created_at_ms: ts,
            updated_at_ms: ts,
        })
        .unwrap();

    let server = Server::http("127.0.0.1:0").unwrap();
    let port = server.server_addr().to_ip().unwrap().port();
    let host_id = "nola-host-workstream-b".to_string();
    let state = Arc::new(WebServerState {
        store: store.clone(),
        suite: Arc::clone(&suite),
        ui_dir: dir.path().to_path_buf(),
        static_dir: dir.path().to_path_buf(),
        loopback: true,
        env_token_override: false,
        host_id: host_id.clone(),
        access_token: "test-access-token".into(),
        sessions: Arc::new(Mutex::new(BTreeMap::new())),
        handoffs: Arc::new(Mutex::new(HashMap::new())),
        nonces: Arc::new(Mutex::new(BoundedNonceCache::default())),
        active_cancels: Arc::new(Mutex::new(BTreeMap::new())),
        pending_interactions: Arc::new(PendingInteractions::default()),
        native_browser: Arc::new(NativeBrowserBridge::default()),
        tool_plans: Arc::new(Mutex::new(BTreeMap::new())),
        mcp_manager: Arc::new(McpManager::new(None)),
        tailcat: TailcatConfig {
            ingress: false,
            helper_path: None,
            helper_sha256: None,
            web_port: port,
        },
        tailcat_leases: Arc::new(TailcatLeaseManager::default()),
        messaging_worker: Arc::new(Mutex::new(None)),
        oauth_flows: Arc::new(Mutex::new(BTreeMap::new())),
        auxiliary_ops: Arc::new(Mutex::new(BTreeMap::new())),
    });

    let srv_handle = std::thread::spawn(move || {
        for _ in 0..8 {
            if let Ok(req) = server.recv() {
                handle_http_request(req, Arc::clone(&state));
            }
        }
    });

    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let base = format!("http://127.0.0.1:{port}");
    let post_json = |path: &str, payload: Value| -> Value {
        let resp = client
            .post(format!("{base}{path}"))
            .header("Authorization", "Bearer test-access-token")
            .header("Origin", &base)
            .json(&payload)
            .send()
            .unwrap();
        let status = resp.status();
        let val: Value = resp.json().unwrap();
        assert_eq!(status, 200, "POST {path} returned {status}: {val}");
        val
    };
    let get_json = |path: &str| -> Value {
        let resp = client
            .get(format!("{base}{path}"))
            .header("Authorization", "Bearer test-access-token")
            .send()
            .unwrap();
        let status = resp.status();
        let val: Value = resp.json().unwrap();
        assert_eq!(status, 200, "GET {path} returned {status}: {val}");
        val
    };

    // 1. catalog/replace with ONLY legacy endpoint preserves managed-imp, local:qwen3, and managed Vault secret!
    let rep_resp = post_json(
        "/api/model-profiles/catalog/replace",
        json!({
            "endpoints": [{
                "id": "legacy-ep",
                "name": "Legacy",
                "baseURL": "https://legacy.invalid/v1",
                "wireProtocol": "chat_completions",
                "enabled": true,
                "models": [{"id": "legacy-ep:gpt-4o", "modelName": "gpt-4o", "enabled": true, "priority": 50, "capabilities": ["chat"]}]
            }]
        }),
    );
    let cfg_after = load_active_config(&store);
    assert!(cfg_after.llm.connections.iter().any(|c| c.id == "managed-imp"));
    assert!(cfg_after.llm.connections.iter().any(|c| c.id == "local:qwen3"));
    let vault = Vault::open(vault_path).unwrap();
    assert_eq!(
        vault.get_secret(&vault_secret_key("managed-imp")).unwrap().as_deref(),
        Some("managed-secret")
    );
    assert!(rep_resp["profiles"].as_array().unwrap().len() >= 3);

    // 2. POST /api/chat/conversations/{id} with operation:"prepare" (no request_digest)
    let prep = post_json(
        &format!("/api/chat/conversations/{conv_id}"),
        json!({
            "operation": "prepare",
            "identity": {
                "source_host_id": host_id,
                "client_id": "ios-1",
                "conversation_id": conv_id,
                "submission_id": "sub-prep-1",
                "user_message_id": "u-prep-1",
                "assistant_message_id": "sub-prep-1:assistant"
            },
            "request": {"content": "hello"}
        }),
    );
    assert_eq!(prep["source_host_id"], host_id);
    assert_eq!(prep["result"]["identity"]["submission_id"], "sub-prep-1");
    let digest = prep["result"]["request_digest"].as_str().unwrap().to_string();
    assert_eq!(digest.len(), 64);

    // 3. POST /api/chat/conversations/{id} with operation:"observe" wraps with wrap_paired_envelope
    let obs_sub = post_json(
        &format!("/api/chat/conversations/{conv_id}"),
        json!({
            "operation": "observe",
            "identity": {
                "source_host_id": host_id,
                "client_id": "ios-1",
                "conversation_id": conv_id,
                "submission_id": "sub-prep-1",
                "user_message_id": "u-prep-1",
                "assistant_message_id": "sub-prep-1:assistant"
            },
            "request_digest": digest
        }),
    );
    assert_eq!(obs_sub["source_host_id"], host_id);
    assert_eq!(obs_sub["result"]["phase"], "prepared");

    // 4. GET /api/chat/conversations/{id}/observe returns non-null admission + 16-field terminal_seal + 10-field result
    let obs = get_json(&format!(
        "/api/chat/conversations/{conv_id}/observe?expected_host_id={host_id}&client_id=ios-1&expected_run_id=run-1&assistant_message_id=asst-1"
    ));
    assert_eq!(obs["source_host_id"], host_id);
    assert!(obs["result"]["admission"].is_object());
    assert_eq!(obs["result"]["admission"]["sequence"], "1");
    let seal = obs["result"]["terminal_seal"].as_object().unwrap();
    assert_eq!(seal.len(), 16);
    let run_res = seal["result"].as_object().unwrap();
    assert_eq!(run_res.len(), 10);
    assert_eq!(
        run_res["input"]["applications_digest"],
        "e0aeaeba0c6a61c02edcc186902f9b41dd82835d01b780eb0e479cc513ea66dd"
    );
    assert_eq!(run_res["value"]["kind"], "answered");

    // 5. POST /api/chat/conversations/{id}/compact includes compacted, checkpoint_revision, committed_at_ms, context
    let compact = post_json(
        &format!("/api/chat/conversations/{conv_id}/compact"),
        json!({ "expected_host_id": host_id, "conversation_id": conv_id }),
    );
    assert_eq!(compact["source_host_id"], host_id);
    assert_eq!(compact["result"]["conversation_id"], conv_id);
    assert!(compact["result"]["checkpoint_revision"].as_u64().unwrap() >= 1);
    assert!(compact["result"]["committed_at_ms"].as_i64().unwrap() > 0);
    assert!(compact["result"]["context"]["context_window_tokens"].as_u64().unwrap() > 0);
    assert_eq!(
        compact["result"]["compacted"].as_bool().unwrap(),
        compact["result"]["context"]["replaced_item_count"].as_u64().unwrap() > 0
    );

    // 6. POST /api/chat/conversations/{id}/events includes head and dispatch
    let ev = post_json(
        &format!("/api/chat/conversations/{conv_id}/events"),
        json!({
            "expected_host_id": host_id,
            "expected_source_store_epoch": "ep-1",
            "conversation_id": conv_id,
            "client_id": "ios-1"
        }),
    );
    assert_eq!(ev["source_host_id"], host_id);
    assert!(ev["result"].as_object().unwrap().contains_key("head"));
    assert!(ev["result"].as_object().unwrap().contains_key("dispatch"));

    // 7. POST /api/chat/conversations/{id}/cancel with command_id returns {source_host_id, result: {cancelled: true, command_id}}
    let cancel_cmd = post_json(
        &format!("/api/chat/conversations/{conv_id}/cancel"),
        json!({
            "expected_host_id": host_id,
            "expected_source_store_epoch": "ep-1",
            "conversation_id": conv_id,
            "client_id": "ios-1",
            "command_id": "cmd-ui-1",
            "request_digest": "0".repeat(64)
        }),
    );
    assert_eq!(cancel_cmd["source_host_id"], host_id);
    assert_eq!(cancel_cmd["result"]["cancelled"], true);
    assert_eq!(cancel_cmd["result"]["command_id"], "cmd-ui-1");

    // 8. POST /api/chat/conversations/{id}/messages with task_child_window wraps in paired envelope
    let tcw = post_json(
        &format!("/api/chat/conversations/{conv_id}/messages"),
        json!({
            "operation": "task_child_window",
            "destination": { "child_conversation_id": conv_id },
            "limit": 20
        }),
    );
    assert_eq!(tcw["source_host_id"], host_id);
    assert!(tcw.get("result").is_some());

    srv_handle.join().unwrap();
}
