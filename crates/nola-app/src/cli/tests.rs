// SPDX-License-Identifier: Apache-2.0

use super::*;

#[test]
fn resolve_route_with_overrides_prioritizes_explicit_overrides_and_reasoning_effort() {
    let mut cfg = AppliedConfig::default();
    cfg.llm.connections.push(ConnectionConfig {
        id: "default-conn".into(),
        endpoint: "https://api.openai.com/v1".into(),
        protocol: Some("responses".into()),
        provider: None,
        api_key: Some("sk-default".into()),
        api_key_env: None,
        models: vec![ModelEntry {
            id: "gpt-4.1".into(),
            context_window: None,
            max_output_tokens: None,
            temperature: None,
            top_p: None,
            reasoning_effort: None,
            capabilities: None,
        }],
    });
    cfg.llm.default_model = Some(DefaultModelSelection {
        connection: "default-conn".into(),
        model: "gpt-4.1".into(),
    });

    let opts = CliOptions::default();
    let overrides = HostRunOverrides {
        model_id: Some("local-qwen".into()),
        endpoint: Some("http://127.0.0.1:8080/v1".into()),
        api_key: Some("sk-override".into()),
        protocol: Some("openai_chat".into()),
        reasoning_effort: Some("high".into()),
        ..Default::default()
    };
    let route = resolve_route_with_overrides(&cfg, &opts, Some(&overrides)).unwrap();
    assert_eq!(route.model, "local-qwen");
    assert_eq!(route.endpoint, "http://127.0.0.1:8080/v1");
    assert_eq!(route.api_key, "sk-override");
    assert_eq!(route.protocol, ModelProtocol::OpenAiChat);
    assert_eq!(route.reasoning_effort.as_deref(), Some("high"));

    // Explicit connection_id override disambiguates two connections sharing the same endpoint URL
    cfg.llm.connections.push(ConnectionConfig {
        id: "lan-conn-a".into(),
        endpoint: "http://192.168.31.18:8081/v1".into(),
        protocol: Some("chat_completions".into()),
        provider: None,
        api_key: None,
        api_key_env: None,
        models: vec![],
    });
    cfg.llm.connections.push(ConnectionConfig {
        id: "lan-conn-b".into(),
        endpoint: "http://192.168.31.18:8081/v1".into(),
        protocol: Some("chat_completions".into()),
        provider: None,
        api_key: None,
        api_key_env: None,
        models: vec![],
    });
    let disambig = HostRunOverrides {
        connection_id: Some("lan-conn-b".into()),
        model_id: Some("qwen3.8-flash-next-iq3_s".into()),
        endpoint: Some("http://192.168.31.18:8081/v1".into()),
        protocol: Some("chat_completions".into()),
        ..Default::default()
    };
    let disambig_route = resolve_route_with_overrides(&cfg, &opts, Some(&disambig)).unwrap();
    assert_eq!(disambig_route.connection_id, "lan-conn-b");

    // Partial override still applies on top of default model
    let partial = HostRunOverrides {
        model_id: Some("gpt-5".into()),
        protocol: Some("gemini".into()),
        reasoning_effort: Some("medium".into()),
        ..Default::default()
    };
    let route2 = resolve_route_with_overrides(&cfg, &opts, Some(&partial)).unwrap();
    assert_eq!(route2.model, "gpt-5");
    assert_eq!(route2.endpoint, "https://api.openai.com/v1");
    assert_eq!(route2.protocol, ModelProtocol::Gemini);
    assert_eq!(route2.reasoning_effort.as_deref(), Some("medium"));

    // Without overrides, falls back to cfg.llm.default_model
    let route3 = resolve_route_with_overrides(&cfg, &opts, None).unwrap();
    assert_eq!(route3.model, "gpt-4.1");
    assert_eq!(route3.endpoint, "https://api.openai.com/v1");
}

#[test]
fn expert_mentions_host_context_attachments_and_lan_error_formatting_work() {
    let store = Store::open_in_memory().unwrap();
    store
        .put_module_record(
            "expert_peer",
            "exp-leijun",
            &json!({
                "id": "exp-leijun",
                "name": "雷军",
                "role": "specialist",
                "description": "小米创始人视角，擅长产品策略与硬件工程分析",
                "system_prompt": "",
                "availability": "active",
                "allowed_tools": [],
                "updated_at_ms": 1795794933650i64
            }),
        )
        .unwrap();

    let ov = HostRunOverrides {
        context_components: vec![
            json!({
                "kind": "platform_data",
                "id": "calendar",
                "content": "Today 10:00 Product Review",
                "source": {"title": "Calendar"}
            }),
            json!({
                "kind": "main_expert_selection",
                "id": "nola.main-expert-selection",
                "content": {"selectedExperts": []}
            }),
        ],
        attachments: vec![json!({
            "name": "spec.pdf",
            "path": "/tmp/spec.pdf",
            "mime_type": "application/pdf"
        })],
        ..Default::default()
    };
    let mut blocks = Vec::new();
    append_expert_and_host_context_blocks(&store, "@雷军 你能做什么", Some(&ov), &mut blocks);
    assert!(
        blocks.iter().any(|b| b.id == "saved_experts_directory" && b.text.contains("雷军")),
        "saved_experts_directory block should list 雷军"
    );
    assert!(
        blocks.iter().any(|b| b.id == "active_expert_mentions" && b.text.contains("雷军") && b.text.contains("小米创始人")),
        "active_expert_mentions block should inject matched @雷军 persona"
    );
    assert!(
        blocks.iter().any(|b| b.id == "host_context_components" && b.text.contains("Product Review")),
        "host_context_components block should include Calendar context"
    );
    assert!(
        !blocks.iter().any(|b| b.id == "attached_files"),
        "per-turn attachments must stay in user message, not frozen system prefix"
    );
    let turn_prompt = format_turn_prompt_with_attachments("看下这篇论文", Some(&ov));
    assert!(
        turn_prompt.contains("看下这篇论文") && turn_prompt.contains("/tmp/spec.pdf"),
        "turn prompt should include user attachment path directly: {turn_prompt}"
    );

    let friendly = format_user_facing_run_error(
        "llm error: http error: error trying to connect: tcp connect error: No route to host (os error 65)",
        "http://192.168.31.18:8081/v1",
    );
    assert!(friendly.contains("本地网络"), "expected friendly Local Network guidance: {friendly}");
}

#[test]
fn execution_backends_includes_nola_and_external_entries() {
    let val = list_execution_backends_json();
    let arr = val.as_array().unwrap();
    assert_eq!(arr[0]["id"], "nola");
    assert_eq!(arr[0]["availability"], "available");
    assert_eq!(arr[0]["capabilities"]["supports_permission_bridge"], true);
    let ids: Vec<&str> = arr.iter().filter_map(|b| b["id"].as_str()).collect();
    assert!(ids.contains(&"codex"));
    assert!(ids.contains(&"claude_code"));
    assert!(ids.contains(&"opencode"));
}

#[test]
fn parse_args_supports_extended_cli_flags() {
    let args: Vec<String> = [
        "--profile",
        "allow_all",
        "--continue",
        "--reasoning-effort",
        "high",
        "--no-tools",
        "--output",
        "/tmp/out.txt",
        "--debug",
        "--lang",
        "zh-CN",
        "hello",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let opts = parse_args(&args).unwrap();
    assert_eq!(opts.profile.as_deref(), Some("allow_all"));
    assert!(opts.continue_last);
    assert!(opts.resume);
    assert_eq!(opts.reasoning_effort.as_deref(), Some("high"));
    assert!(opts.no_tools);
    assert_eq!(opts.output_file, Some(PathBuf::from("/tmp/out.txt")));
    assert!(opts.debug);
    assert_eq!(opts.lang.as_deref(), Some("zh-CN"));
    assert_eq!(resolve_profile(&opts), PermissionProfile::AllowAll);
}

#[test]
fn host_stdin_parsers_handle_v2_and_macos_legacy_shapes() {
    assert_eq!(
        parse_permission_allow(&json!({
            "request_id": "r1",
            "resolution": { "type": "authorization", "decision": "allow_conversation" }
        })),
        Some(true)
    );
    assert_eq!(
        parse_permission_allow(&json!({ "decision": "deny" })),
        Some(false)
    );
    assert_eq!(
        parse_user_input_answer(&json!({ "answer": "yes please" }), Some("q1")),
        Some(Some("yes please".to_string()))
    );
    assert_eq!(
        parse_user_input_answer(&json!({ "cancelled": true }), Some("q1")),
        Some(None)
    );
    assert_eq!(
        extract_reasoning_effort_from_req(&json!({ "reasoning": { "effort": "high" } })),
        Some("high".to_string())
    );
    assert_eq!(
        extract_reasoning_effort_from_req(&json!({ "reasoning_selection": { "kind": "effort", "value": "low" } })),
        Some("low".to_string())
    );
}

#[test]
fn desktop_manage_supports_macos_runtime_profile_action_order_and_shorthand() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    bind_store_vault_path(&store, &dir.path().join("vault.json")).unwrap();
    let suite = Arc::new(ModulesSuite::new(store.clone(), dir.path().to_path_buf(), None));

    let plugin_src = dir.path().join("demo-plugin");
    fs::create_dir_all(&plugin_src).unwrap();
    fs::write(
        plugin_src.join("nola-plugin.json"),
        r#"{"id":"demo-plugin","name":"Demo Plugin","version":"1.0.0","runtime":"dsh","profile":"web"}"#,
    )
    .unwrap();

    let db_path = dir.path().join("nola.db");

    // macOS 4-arg order: desktop-manage <runtime> <profile> <action> <source>
    let rc = handle_plugin_cmd(
        &store,
        &suite,
        &db_path,
        &[
            "desktop-manage".into(),
            "dsh".into(),
            "web".into(),
            "install".into(),
            plugin_src.display().to_string(),
        ],
    )
    .unwrap();
    assert_eq!(rc, 0);
    let rec = suite
        .get_record::<PluginRecord>(KIND_PLUGIN, "demo-plugin")
        .unwrap()
        .expect("plugin should be installed");
    assert!(rec.enabled);

    // Disable via macOS order
    let rc = handle_plugin_cmd(
        &store,
        &suite,
        &db_path,
        &[
            "desktop-manage".into(),
            "dsh".into(),
            "web".into(),
            "disable".into(),
            "demo-plugin".into(),
        ],
    )
    .unwrap();
    assert_eq!(rc, 0);
    let rec2 = suite
        .get_record::<PluginRecord>(KIND_PLUGIN, "demo-plugin")
        .unwrap()
        .unwrap();
    assert!(!rec2.enabled);

    // Enable via CLI shorthand
    let rc = handle_plugin_cmd(
        &store,
        &suite,
        &db_path,
        &[
            "desktop-manage".into(),
            "enable".into(),
            "demo-plugin".into(),
        ],
    )
    .unwrap();
    assert_eq!(rc, 0);
    assert!(
        suite
            .get_record::<PluginRecord>(KIND_PLUGIN, "demo-plugin")
            .unwrap()
            .unwrap()
            .enabled
    );

    // Harness desktop-pages scanning under <db>.plugins/harness/dsh/profile/profiles/web
    let web_profile = db_path
        .with_extension("plugins")
        .join("harness/dsh/profile/profiles/web");
    let pkg_dir = web_profile.join("node_modules/test-board");
    fs::create_dir_all(&pkg_dir).unwrap();
    fs::write(
        pkg_dir.join("package.json"),
        r#"{"name":"test-board","version":"1.0.0","nola":{"pages":[{"id":"board","title":"Task board","entry":"board.html"},{"id":"archive","title":"Completed tasks","entry":"board.html"}]}}"#,
    )
    .unwrap();
    fs::write(pkg_dir.join("board.html"), "<h1>Task board</h1>").unwrap();
    fs::write(
        web_profile.join("package.json"),
        r#"{"dependencies":{"test-board":"1.0.0"}}"#,
    )
    .unwrap();

    let (pages, profiles) = collect_desktop_plugin_pages(dir.path(), &db_path);
    let board = pages
        .iter()
        .find(|p| p.get("title").and_then(|v| v.as_str()) == Some("Task board"))
        .expect("Task board page should be discovered");
    assert_eq!(board["package"], "test-board");
    assert_eq!(board["html"], "<h1>Task board</h1>");
    assert!(profiles
        .iter()
        .any(|p| p.runtime == "dsh" && p.profile == "web" && !p.packages.is_empty()));

    // Removing dependency from profile package.json removes the pages immediately
    fs::write(web_profile.join("package.json"), r#"{"dependencies":{}}"#).unwrap();
    let (pages_after, _) = collect_desktop_plugin_pages(dir.path(), &db_path);
    assert!(pages_after
        .iter()
        .all(|p| p.get("title").and_then(|v| v.as_str()) != Some("Task board")));
}

#[test]
fn goal_lifecycle_and_sessions_and_wiki_subcommands_work() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("lib.rs"), "pub fn greet() -> &'static str { \"hi\" }\n").unwrap();
    let store = Store::open_in_memory().unwrap();
    bind_store_vault_path(&store, &dir.path().join("vault.json")).unwrap();
    let suite = Arc::new(ModulesSuite::new(store.clone(), dir.path().to_path_buf(), None));
    let opts = CliOptions::default();

    // Goal create -> status -> pause -> cancel
    handle_goal_cmd(
        &store,
        &suite,
        &opts,
        &[
            "create".into(),
            "Ship".into(),
            "v2".into(),
            "--id".into(),
            "g-1".into(),
            "--objective".into(),
            "Finish CLI".into(),
        ],
    )
    .unwrap();
    handle_goal_cmd(&store, &suite, &opts, &["status".into(), "g-1".into()]).unwrap();
    handle_goal_cmd(
        &store,
        &suite,
        &opts,
        &["pause".into(), "g-1".into(), "--note".into(), "hold".into()],
    )
    .unwrap();
    let paused = suite
        .get_record::<ExtendedGoalRecord>("goal", "g-1")
        .unwrap()
        .unwrap();
    assert_eq!(paused.status, GoalStatus::Paused);
    handle_goal_cmd(&store, &suite, &opts, &["cancel".into(), "g-1".into()]).unwrap();
    let cancelled = suite
        .get_record::<ExtendedGoalRecord>("goal", "g-1")
        .unwrap()
        .unwrap();
    assert_eq!(cancelled.status, GoalStatus::Failed);

    // Sessions show / part / sources
    let ts = now_ms();
    store
        .upsert_session(&nola_types::Session {
            id: "s-1".into(),
            title: "Demo".into(),
            workspace_root: dir.path().display().to_string(),
            model_connection: None,
            model_id: None,
            created_at_ms: ts,
            updated_at_ms: ts,
        })
        .unwrap();
    let ev = store
        .append_event("s-1", "user_message", json!({"content": "hello"}), ts)
        .unwrap();
    handle_sessions_cmd(&store, &["show".into(), "s-1".into()], true).unwrap();
    handle_sessions_cmd(
        &store,
        &["part".into(), "s-1".into(), ev.seq.to_string()],
        true,
    )
    .unwrap();
    handle_sessions_cmd(&store, &["sources".into(), "s-1".into()], true).unwrap();

    // Wiki generation
    let out_md = dir.path().join("WIKI.md");
    let wiki_opts = CliOptions {
        output_file: Some(out_md.clone()),
        ..Default::default()
    };
    handle_wiki_cmd(&store, dir.path(), &wiki_opts, &[]).unwrap();
    let wiki_text = fs::read_to_string(&out_md).unwrap();
    assert!(wiki_text.contains("Codebase Architecture Wiki"));

    // Model acquire hardware / engine-status / recommend (explicit & default args)
    handle_model_acquire_cmd(&store, &["hardware".into()]).unwrap();
    handle_model_acquire_cmd(&store, &["engine-status".into()]).unwrap();
    handle_model_acquire_cmd(&store, &["engine-status".into(), "llama.cpp".into()]).unwrap();
    handle_model_acquire_cmd(&store, &["recommend".into()]).unwrap();
    handle_model_acquire_cmd(
        &store,
        &["recommend".into(), "llama.cpp".into(), "huggingface".into()],
    )
    .unwrap();

    // Memory create / revise via CLI flags (must never block on stdin)
    handle_memory_cmd(
        &store,
        &[
            "create".into(),
            "--id".into(),
            "mem-cli-1".into(),
            "--title".into(),
            "CLI Fact".into(),
            "--content".into(),
            "Remember 42".into(),
            "--kind".into(),
            "fact".into(),
            "--scope".into(),
            "project".into(),
            "--tags".into(),
            "cli,audit".into(),
        ],
    )
    .unwrap();
    let created_mem = store
        .search_memory("Remember 42", None, 10)
        .unwrap()
        .into_iter()
        .find(|m| m.id == "mem-cli-1")
        .expect("flag-created memory must persist");
    assert_eq!(created_mem.title, "CLI Fact");
    assert_eq!(created_mem.namespace, "project:fact");
    assert_eq!(created_mem.tags, vec!["cli".to_string(), "audit".to_string()]);

    handle_memory_cmd(
        &store,
        &[
            "revise".into(),
            "mem-cli-1".into(),
            "--content=Remember 43".into(),
        ],
    )
    .unwrap();
    let revised_mem = store
        .search_memory("Remember 43", None, 10)
        .unwrap()
        .into_iter()
        .find(|m| m.id == "mem-cli-1")
        .expect("revised memory must persist");
    assert_eq!(revised_mem.title, "CLI Fact");
    assert_eq!(revised_mem.content, "Remember 43");
    assert_eq!(revised_mem.namespace, "project:fact");
}

#[test]
fn siwc_snapshot_probe_metadata_and_a2ui_surface_frames_match_mac_contracts() {
    let store = Store::open_in_memory().unwrap();

    // 1. siwc_account_snapshot emits state="ready" and credential_ref="siwc:<id>"
    let snap = siwc_account_snapshot("acct-42", "Work", 3, true);
    assert_eq!(snap["account"]["state"], "ready");
    assert_eq!(snap["account"]["credential_ref"], "siwc:acct-42");
    let disconnected = siwc_account_snapshot("acct-42", "Work", 4, false);
    assert_eq!(disconnected["account"]["state"], "disconnected");
    assert!(disconnected["account"]["credential_ref"].is_null());

    // 2. probe_prepare / probe metadata resolves connection & account from module_records + kv
    store
        .put_module_record(
            "model_connection",
            "conn-store-1",
            &json!({
                "id": "conn-store-1",
                "display_name": "Stored OpenAI",
                "base_url": "https://api.stored.example/v1",
                "account_id": "acct-99"
            }),
        )
        .unwrap();
    store
        .put_module_record(
            "model_account",
            "acct-99",
            &json!({
                "id": "acct-99",
                "revision": 7,
                "credential_ref": "siwc:acct-99"
            }),
        )
        .unwrap();
    let cfg = load_active_config(&store);
    let meta = resolve_probe_connection_metadata(&store, &cfg, "conn-store-1", &json!({}));
    assert_eq!(meta.base_url, "https://api.stored.example/v1");
    assert_eq!(meta.display_name, "Stored OpenAI");
    assert_eq!(meta.account_id.as_deref(), Some("acct-99"));
    assert_eq!(meta.account_revision, Some(7));

    // 3. collect_a2ui_surface_frames emits ui_surface frames for rendered surfaces
    let owner = nola_modules::a2ui::UiSurfaceOwner {
        authority_id: "local-host".into(),
        conversation_id: "sess-a2ui-1".into(),
        participant_id: "assistant".into(),
        context_id: "conversation".into(),
        originating_run_id: "run-a2ui-1".into(),
        message_id: "msg-a2ui-1".into(),
        part_id: "part-a2ui-1".into(),
    };
    let (root_id, nodes) = nola_modules::a2ui::build_nodes_from_spec(&json!({
        "components": [
            {"id": "t1", "component": "Text", "text": "Hello A2UI"}
        ]
    }))
    .unwrap();
    nola_modules::a2ui::upsert_ui_surface(
        &store,
        owner,
        "card-1",
        "Summary Card",
        root_id,
        nodes,
        json!({}),
    )
    .unwrap();
    let frames = collect_a2ui_surface_frames(&store, "sess-a2ui-1", "sess-a2ui-1");
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0]["type"], "ui_surface");
    assert_eq!(frames[0]["session_id"], "sess-a2ui-1");
    assert_eq!(frames[0]["snapshot"]["identity"]["surface_id"], "card-1");
}

#[test]
fn model_import_extracts_codex_kimi_and_opencode_with_credentials() {
    let dir = tempfile::tempdir().unwrap();

    // 1. Codex config.toml + auth.json (API key)
    let codex_cfg = dir.path().join("codex-config.toml");
    std::fs::write(
        &codex_cfg,
        r#"
model = "gpt-5.3-codex"
model_provider = "custom"

[model_providers.custom]
name = "My Codex Gateway"
base_url = "https://gateway.example.com/v1"
wire_api = "responses"
env_key = "CUSTOM_CODEX_KEY"
"#,
    )
    .unwrap();
    let codex_auth = dir.path().join("codex-auth.json");
    std::fs::write(&codex_auth, r#"{"OPENAI_API_KEY": "sk-codex-test-123"}"#).unwrap();

    let codex_cands = extract_import_candidates_with_options(
        codex_cfg.to_str().unwrap(),
        "codex",
        Some(codex_auth.to_str().unwrap()),
        true,
        false,
    )
    .unwrap();
    assert_eq!(codex_cands.len(), 1);
    assert_eq!(codex_cands[0].display_name, "My Codex Gateway");
    assert_eq!(codex_cands[0].endpoint, "https://gateway.example.com/v1");
    assert_eq!(codex_cands[0].protocol, "responses");
    assert_eq!(codex_cands[0].models, vec!["gpt-5.3-codex"]);
    assert_eq!(codex_cands[0].credential_status, "available");
    assert_eq!(codex_cands[0].secret.as_deref(), Some("sk-codex-test-123"));
    assert!(codex_cands[0].issue.is_none());

    // Without include_credentials -> "not_requested" and no secret
    let codex_no_cred = extract_import_candidates_with_options(
        codex_cfg.to_str().unwrap(),
        "codex",
        Some(codex_auth.to_str().unwrap()),
        false,
        false,
    )
    .unwrap();
    assert_eq!(codex_no_cred[0].credential_status, "not_requested");
    assert!(codex_no_cred[0].secret.is_none());

    // Subscription-only auth.json -> flags subscription_credential_reuse_not_supported
    let codex_sub_auth = dir.path().join("codex-sub-auth.json");
    std::fs::write(
        &codex_sub_auth,
        r#"{"auth_mode": "chatgpt", "tokens": {"access_token": "jwt"}}"#,
    )
    .unwrap();
    let codex_sub = extract_import_candidates_with_options(
        codex_cfg.to_str().unwrap(),
        "codex",
        Some(codex_sub_auth.to_str().unwrap()),
        true,
        false,
    )
    .unwrap();
    assert_eq!(
        codex_sub[0].issue.as_deref(),
        Some("subscription_credential_reuse_not_supported")
    );

    // 2. Kimi Code CLI config.toml
    let kimi_cfg = dir.path().join("kimi-config.toml");
    std::fs::write(
        &kimi_cfg,
        r#"
[providers.moonshot]
type = "openai_legacy"
base_url = "https://api.moonshot.ai/v1"
api_key = "sk-kimi-999"

[models.kimi-k2]
provider = "moonshot"
model = "kimi-k2-thinking"
"#,
    )
    .unwrap();
    let kimi_cands = extract_import_candidates_with_options(
        kimi_cfg.to_str().unwrap(),
        "kimi_code",
        None,
        true,
        false,
    )
    .unwrap();
    assert_eq!(kimi_cands.len(), 1);
    assert_eq!(kimi_cands[0].endpoint, "https://api.moonshot.ai/v1");
    assert_eq!(kimi_cands[0].protocol, "chat_completions");
    assert_eq!(kimi_cands[0].models, vec!["kimi-k2-thinking"]);
    assert_eq!(kimi_cands[0].credential_status, "available");
    assert_eq!(kimi_cands[0].secret.as_deref(), Some("sk-kimi-999"));

    // 3. OpenCode JSONC + separate auth.json
    let oc_cfg = dir.path().join("opencode.jsonc");
    std::fs::write(
        &oc_cfg,
        r#"{
            // comment
            "provider": {
                "openrouter": {
                    "name": "OpenRouter",
                    "options": { "baseURL": "https://openrouter.ai/api/v1" },
                    "models": { "openrouter/free": {} }
                }
            }
        }"#,
    )
    .unwrap();
    let oc_auth = dir.path().join("opencode-auth.json");
    std::fs::write(&oc_auth, r#"{"openrouter": {"key": "sk-or-test"}}"#).unwrap();
    let oc_cands = extract_import_candidates_with_options(
        oc_cfg.to_str().unwrap(),
        "open_code",
        Some(oc_auth.to_str().unwrap()),
        true,
        false,
    )
    .unwrap();
    assert_eq!(oc_cands.len(), 1);
    assert_eq!(oc_cands[0].credential_status, "available");
    assert_eq!(oc_cands[0].secret.as_deref(), Some("sk-or-test"));
}

#[test]
fn commit_import_candidates_skips_empty_endpoint_and_syncs_module_records() {
    let store = Store::open_in_memory().unwrap();
    let cands = vec![
        ExtractedImportCandidate {
            candidate_id: "c-empty".into(),
            connection_id: "empty-conn".into(),
            display_name: "Should Be Skipped".into(),
            source: "open_code".into(),
            endpoint: "".into(),
            protocol: "chat_completions".into(),
            models: vec!["m1".into()],
            secret: None,
            credential_status: "missing".into(),
            issue: Some("Missing base URL".into()),
        },
        ExtractedImportCandidate {
            candidate_id: "c-valid".into(),
            connection_id: "valid-conn".into(),
            display_name: "Valid Provider".into(),
            source: "open_code".into(),
            endpoint: "https://api.valid.invalid/v1".into(),
            protocol: "chat_completions".into(),
            models: vec!["gpt-4o".into()],
            secret: None,
            credential_status: "not_requested".into(),
            issue: None,
        },
    ];
    let imported = commit_import_candidates(&store, &cands, false).unwrap();
    assert_eq!(imported, vec!["valid-conn".to_string()]);
    let cfg = load_active_config(&store);
    assert!(!cfg.connection_extras.contains_key("empty-conn"));
    let mod_rec: Value = store
        .get_module_record("model_connection", "valid-conn")
        .unwrap()
        .expect("model_connection module record should be upserted");
    assert_eq!(mod_rec["base_url"], "https://api.valid.invalid/v1");
    assert_eq!(mod_rec["catalog"]["models"][0]["model_id"], "gpt-4o");
}

#[test]
fn host_goal_control_and_local_catalog_and_siwc_callback_and_memory_flags() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    let suite = ModulesSuite::new(store.clone(), dir.path().to_path_buf(), None);

    // 1. goal_control transitions
    let ts = now_ms();
    let goal = ExtendedGoalRecord {
        id: "g-ctl-1".into(),
        title: "Ship release".into(),
        objective: "Verify everything".into(),
        workspace_root: dir.path().display().to_string(),
        status: GoalStatus::Pending,
        progress_notes: Vec::new(),
        created_at_ms: ts,
        updated_at_ms: ts,
        lease_until_ms: None,
        retry_count: 0,
        next_retry_at_ms: None,
        needs_user_question: None,
        acceptance_criteria: Vec::new(),
        accepted: false,
    };
    suite.upsert_record("goal", &goal.id, &goal).unwrap();
    let paused = handle_host_goal_control_request(
        &store,
        &suite,
        &json!({"operation": "goal_control", "goal_id": "g-ctl-1", "action": "pause"}),
    )
    .unwrap();
    assert_eq!(paused["status"], "paused");
    let resumed = handle_host_goal_control_request(
        &store,
        &suite,
        &json!({"operation": "goal_control", "goal_id": "g-ctl-1", "action": "resume"}),
    )
    .unwrap();
    assert_eq!(resumed["status"], "running");
    let sched = handle_host_goal_control_request(
        &store,
        &suite,
        &json!({"operation": "goal_control", "goal_id": "g-ctl-1", "action": "schedule_resume", "resume_at_ms": 999999}),
    )
    .unwrap();
    assert_eq!(sched["status"], "paused");

    // 2. MacLocalModelCatalog catalog.json + snapshot.json roundtrip
    let models_root = dir.path().join("models");
    let snap_dir = models_root.join("qwen_mlx");
    fs::create_dir_all(&snap_dir).unwrap();
    fs::write(snap_dir.join("config.json"), b"{}").unwrap();
    write_local_snapshot_manifest(&snap_dir, &[("config.json".into(), 2)], 2).unwrap();
    upsert_mac_local_model_catalog_entry(
        &models_root,
        &MacLocalCatalogRecordInput {
            id: "m-mlx-1",
            name: "mlx-community/qwen",
            repository: "mlx-community/qwen",
            runtime: "mlx",
            format: "mlx",
            quantization: Some("4bit"),
            context_window_tokens: 32768,
            recommended_device: "unified",
            snapshot_path: &snap_dir,
            artifact_path: None,
            artifact_bytes: 2,
        },
    )
    .unwrap();
    let cat: Value =
        serde_json::from_str(&fs::read_to_string(models_root.join("catalog.json")).unwrap())
            .unwrap();
    assert_eq!(cat["schemaVersion"], 2);
    assert_eq!(cat["models"].as_array().unwrap().len(), 1);
    assert_eq!(cat["models"][0]["runtime"], "mlx");
    remove_mac_local_model_catalog_entry(&models_root, "m-mlx-1", None).unwrap();
    let cat_after: Value =
        serde_json::from_str(&fs::read_to_string(models_root.join("catalog.json")).unwrap())
            .unwrap();
    assert!(cat_after["models"].as_array().unwrap().is_empty());

    // 3. SIWC real local TCP callback listener + module_records sync
    let (port, cb_res, cb_stop) = spawn_siwc_callback_listener("state-xyz".into());
    {
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream
            .write_all(b"GET /auth/callback?code=auth-code-123&state=state-xyz HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .unwrap();
        let mut resp_buf = String::new();
        let _ = stream.read_to_string(&mut resp_buf);
        assert!(resp_buf.contains("200 OK"));
    }
    let mut got_code = None;
    for _ in 0..40 {
        if let Some(res) = cb_res.lock().unwrap().clone() {
            got_code = Some(res);
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    cb_stop.store(true, Ordering::SeqCst);
    assert_eq!(got_code, Some(Ok("auth-code-123".to_string())));

    upsert_siwc_model_account_record(&store, "acct-siwc", "ChatGPT", 1, true);
    let conn_snap = siwc_default_connection_snapshot("acct-siwc", 1);
    upsert_siwc_model_connection_record(&store, &conn_snap);
    assert!(store
        .get_module_record::<Value>("model_account", "acct-siwc")
        .unwrap()
        .is_some());
    assert!(store
        .get_module_record::<Value>("model_connection", "siwc-acct-siwc")
        .unwrap()
        .is_some());

    // 4. Memory CLI extended flags
    let (_, parsed) = parse_memory_cli_flags(
        false,
        &[
            "--title".into(),
            "Prefs".into(),
            "--content".into(),
            "Uses Rust".into(),
            "--confidence".into(),
            "0.9".into(),
            "--importance".into(),
            "0.8".into(),
            "--sensitivity".into(),
            "low".into(),
            "--source".into(),
            "user".into(),
            "--provenance-kind".into(),
            "manual".into(),
        ],
    )
    .unwrap();
    let draft = parsed.unwrap();
    assert_eq!(draft["confidence"], 0.9);
    assert_eq!(draft["importance"], 0.8);
    assert_eq!(draft["sensitivity"], "low");
    assert_eq!(draft["provenance_kind"], "manual");

    // 5. DataSourceRecord serde defaults allow canonical records without top-level location
    let ds: nola_types::DataSourceRecord =
        serde_json::from_value(json!({"id": "ds-1", "name": "Docs", "kind": "http"})).unwrap();
    assert_eq!(ds.id, "ds-1");
    assert_eq!(ds.location, "");
}

/// Minimal local HTTP/1.1 mock OpenAI SSE server for end-to-end `canonical_mac_host` tests.
/// Reads exact incoming JSON request bodies (supporting Content-Length and chunked transfer encoding)
/// and streams back scripted SSE responses or HTTP status responses.
struct MockSseServer {
    port: u16,
    recorded_bodies: Arc<Mutex<Vec<Value>>>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl MockSseServer {
    fn spawn<F>(expected_requests: usize, handler: F) -> Self
    where
        F: Fn(usize, &Value) -> (u16, &'static str, Vec<String>) + Send + 'static,
    {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let recorded_bodies: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let rec_clone = Arc::clone(&recorded_bodies);

        let handle = std::thread::spawn(move || {
            for idx in 0..expected_requests {
                let (mut stream, _) = listener.accept().unwrap();
                let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
                let raw_body = read_http_request_body(&mut stream);
                let parsed: Value =
                    serde_json::from_slice(&raw_body).unwrap_or_else(|_| json!({"raw": String::from_utf8_lossy(&raw_body)}));
                rec_clone.lock().unwrap().push(parsed.clone());

                let (status, content_type, chunks) = handler(idx, &parsed);
                let reason = match status {
                    200 => "OK",
                    400 => "Bad Request",
                    500 => "Internal Server Error",
                    _ => "Error",
                };
                if content_type == "text/event-stream" {
                    let header = format!(
                        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n"
                    );
                    let _ = stream.write_all(header.as_bytes());
                    for chunk in chunks {
                        let _ = stream.write_all(chunk.as_bytes());
                        let _ = stream.flush();
                    }
                } else {
                    let body_str = chunks.concat();
                    let resp = format!(
                        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body_str}",
                        body_str.len()
                    );
                    let _ = stream.write_all(resp.as_bytes());
                    let _ = stream.flush();
                }
            }
        });

        Self {
            port,
            recorded_bodies,
            handle: Some(handle),
        }
    }

    fn endpoint(&self) -> String {
        format!("http://127.0.0.1:{}/v1", self.port)
    }

    fn join_and_take_bodies(mut self) -> Vec<Value> {
        if let Some(h) = self.handle.take() {
            h.join().unwrap();
        }
        self.recorded_bodies.lock().unwrap().clone()
    }
}

fn read_http_request_body(stream: &mut std::net::TcpStream) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let header_end;
    loop {
        let n = stream.read(&mut tmp).unwrap_or(0);
        if n == 0 {
            return Vec::new();
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            header_end = pos + 4;
            break;
        }
    }
    let header_str = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let content_length: Option<usize> = header_str.lines().find_map(|line| {
        let (k, v) = line.split_once(':')?;
        if k.trim().eq_ignore_ascii_case("content-length") {
            v.trim().parse::<usize>().ok()
        } else {
            None
        }
    });
    let is_chunked = header_str.lines().any(|line| {
        line.split_once(':').is_some_and(|(k, v)| {
            k.trim().eq_ignore_ascii_case("transfer-encoding")
                && v.to_ascii_lowercase().contains("chunked")
        })
    });
    let mut body = buf[header_end..].to_vec();
    if let Some(cl) = content_length {
        while body.len() < cl {
            let n = stream.read(&mut tmp).unwrap_or(0);
            if n == 0 {
                break;
            }
            body.extend_from_slice(&tmp[..n]);
        }
        body.truncate(cl);
        return body;
    }
    if is_chunked {
        while !body.windows(5).any(|w| w == b"0\r\n\r\n") {
            let n = stream.read(&mut tmp).unwrap_or(0);
            if n == 0 {
                break;
            }
            body.extend_from_slice(&tmp[..n]);
        }
        let mut decoded = Vec::new();
        let mut cur = body.as_slice();
        while let Some(pos) = cur.windows(2).position(|w| w == b"\r\n") {
            let size_hex = String::from_utf8_lossy(&cur[..pos]);
            let size = usize::from_str_radix(size_hex.trim(), 16).unwrap_or(0);
            cur = &cur[pos + 2..];
            if size == 0 || cur.len() < size {
                break;
            }
            decoded.extend_from_slice(&cur[..size]);
            cur = if cur.len() >= size + 2 {
                &cur[size + 2..]
            } else {
                &[]
            };
        }
        return decoded;
    }
    body
}

#[test]
fn real_scenario_canonical_mac_host_e2e_streaming_sse_journal_and_attachments() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().join("workspace");
    fs::create_dir_all(&ws).unwrap();
    let doc_path = ws.join("design_doc.md");
    fs::write(&doc_path, "# Nola v2\nArchitecture v2 uses SQLite WAL + FTS5.\n").unwrap();

    let db_path = dir.path().join("nola-e2e.db");
    let store = Store::open(&db_path).unwrap();
    bind_store_vault_path(&store, &dir.path().join("vault.json")).unwrap();
    let suite = Arc::new(ModulesSuite::new(store.clone(), ws.clone(), None));

    let doc_path_str = doc_path.display().to_string();
    let doc_path_for_srv = doc_path_str.clone();
    let server = MockSseServer::spawn(2, move |idx, _req| {
        if idx == 0 {
            let args_json = json!({"path": doc_path_for_srv}).to_string();
            let split_at = args_json.len() / 2;
            let part1 = &args_json[..split_at];
            let part2 = &args_json[split_at..];
            (
                200,
                "text/event-stream",
                vec![
                    format!(
                        "data: {}\n\n",
                        json!({"id": "c1", "model": "qwen3-local", "choices": [{"delta": {"reasoning_content": "Let me inspect "}}]})
                    ),
                    format!(
                        "data: {}\n\n",
                        json!({"id": "c1", "model": "qwen3-local", "choices": [{"delta": {"reasoning_content": "the attached doc."}}]})
                    ),
                    format!(
                        "data: {}\n\n",
                        json!({"id": "c1", "model": "qwen3-local", "choices": [{"delta": {"tool_calls": [{"index": 0, "id": "call_read_1", "type": "function", "function": {"name": "read_file", "arguments": part1}}]}}]})
                    ),
                    format!(
                        "data: {}\n\n",
                        json!({"id": "c1", "model": "qwen3-local", "choices": [{"delta": {"tool_calls": [{"index": 0, "function": {"arguments": part2}}]}, "finish_reason": "tool_calls"}]})
                    ),
                    "data: [DONE]\n\n".to_string(),
                ],
            )
        } else {
            (
                200,
                "text/event-stream",
                vec![
                    format!(
                        "data: {}\n\n",
                        json!({"id": "c2", "model": "qwen3-local", "choices": [{"delta": {"reasoning_content": "Found SQLite WAL + FTS5."}}]})
                    ),
                    format!(
                        "data: {}\n\n",
                        json!({"id": "c2", "model": "qwen3-local", "choices": [{"delta": {"content": "# Summary\n\n"}}]})
                    ),
                    format!(
                        "data: {}\n\n",
                        json!({"id": "c2", "model": "qwen3-local", "choices": [{"delta": {"content": "Architecture v2 uses **SQLite WAL + FTS5**."}, "finish_reason": "stop"}]})
                    ),
                    "data: [DONE]\n\n".to_string(),
                ],
            )
        }
    });

    let conv_id = "conv-mac-host-e2e-1";
    let run_id = "run-mac-host-e2e-1";
    let user_msg_id = "msg-user-e2e-1";
    let asst_msg_id = "msg-asst-e2e-1";
    let opts = CliOptions {
        host_request_stdin: true,
        ..Default::default()
    };
    let overrides = HostRunOverrides {
        session_id: Some(conv_id.into()),
        submission_id: Some(run_id.into()),
        user_message_id: Some(user_msg_id.into()),
        assistant_message_id: Some(asst_msg_id.into()),
        resume: Some(false),
        model_id: Some("qwen3-local".into()),
        endpoint: Some(server.endpoint()),
        api_key: Some("sk-local-test".into()),
        protocol: Some("openai_chat".into()),
        permission_profile: Some(PermissionProfile::AllowAll),
        reasoning_effort: Some("high".into()),
        attachments: vec![json!({
            "name": "design_doc.md",
            "path": doc_path_str,
            "mime_type": "text/markdown"
        })],
        ..Default::default()
    };

    let rc = execute_agent_turn(
        &store,
        &suite,
        &db_path,
        &ws,
        &None,
        &opts,
        "Summarize the attached design doc",
        Some(overrides),
    )
    .expect("canonical_mac_host turn should succeed");
    assert_eq!(rc, 0);

    let bodies = server.join_and_take_bodies();
    assert_eq!(bodies.len(), 2, "expected 2 upstream LLM calls");

    // 1. Upstream request #1 verifies reasoning_effort="high", stream=true,
    //    and attachment path in USER message (NOT in system prompt)!
    assert_eq!(bodies[0]["reasoning_effort"], "high");
    assert_eq!(bodies[0]["stream"], true);
    let msgs0 = bodies[0]["messages"].as_array().unwrap();
    let sys_text = msgs0[0]["content"].as_str().unwrap_or("");
    assert!(
        !sys_text.contains("[Attached files]") && !sys_text.contains(&doc_path_str),
        "per-turn attachment path must NOT leak into system prompt"
    );
    let user_msg_text = msgs0
        .iter()
        .find(|m| m["role"] == "user")
        .unwrap()["content"]
        .as_str()
        .unwrap();
    assert!(user_msg_text.contains("Summarize the attached design doc"));
    assert!(
        user_msg_text.contains(&doc_path_str),
        "upstream user message must include attachment path: {user_msg_text}"
    );

    // 2. Upstream request #2 contains tool result from reading the temp workspace file!
    let req2_str = serde_json::to_string(&bodies[1]).unwrap();
    assert!(
        req2_str.contains("SQLite WAL + FTS5"),
        "step 2 request must contain read_file tool result"
    );

    // 3. SQLite `chat_user` event stored CLEAN display prompt WITHOUT `[Attached files]` suffix!
    let evts = match store.events_after(conv_id, 0, 100).unwrap() {
        nola_store::EventPage::Events(e) => e,
        other => panic!("unexpected event page: {other:?}"),
    };
    let chat_user = evts
        .iter()
        .find(|e| e.kind == "chat_user")
        .expect("chat_user event must exist");
    assert_eq!(
        chat_user.payload["content"],
        "Summarize the attached design doc"
    );
    assert!(!chat_user.payload["content"]
        .as_str()
        .unwrap()
        .contains("[Attached files]"));

    let chat_asst = evts
        .iter()
        .find(|e| e.kind == "chat_assistant")
        .expect("chat_assistant event must exist");
    assert_eq!(
        chat_asst.payload["content"],
        "# Summary\n\nArchitecture v2 uses **SQLite WAL + FTS5**."
    );
    assert_eq!(chat_asst.payload["toolCalls"][0]["name"], "read_file");
    assert_eq!(chat_asst.payload["toolCalls"][0]["is_error"], false);

    // 4. Verify live committed SQLite assistant journal parts in strict chronological order:
    //    reasoning:0 -> tool:call_read_1 -> reasoning:1 -> text
    let heads = crate::daemon::dispatch_command(
        &store,
        &suite,
        "local_run_assistant_journal_heads",
        &json!({"conversation_id": conv_id, "run_id": run_id, "limit": 64}),
    )
    .unwrap();
    assert_eq!(heads.as_array().unwrap().len(), 1);
    let snap = crate::daemon::dispatch_command(
        &store,
        &suite,
        "open_assistant_journal_snapshot",
        &json!({"at": heads[0]}),
    )
    .unwrap();
    let j_parts = crate::daemon::dispatch_command(
        &store,
        &suite,
        "read_assistant_journal_parts",
        &json!({"snapshot": snap, "max_records": 64}),
    )
    .unwrap();
    let mut parts = j_parts["parts"].as_array().unwrap().clone();
    parts.sort_by_key(|p| {
        p.pointer("/order_key/position")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(u64::MAX)
    });
    assert_eq!(parts.len(), 4, "expected 4 journal parts, got: {parts:?}");
    assert_eq!(parts[0]["part_id"], format!("{asst_msg_id}:reasoning:0"));
    assert_eq!(parts[0]["metadata"]["kind"], "reasoning");
    assert_eq!(parts[0]["metadata"]["data"]["state"], "completed");

    assert_eq!(parts[1]["part_id"], "tool:call_read_1");
    assert_eq!(parts[1]["metadata"]["kind"], "tool");
    assert_eq!(parts[1]["metadata"]["data"]["state"], "completed");
    assert_eq!(parts[1]["metadata"]["data"]["capability_id"], "read_file");

    assert_eq!(parts[2]["part_id"], format!("{asst_msg_id}:reasoning:1"));
    assert_eq!(parts[2]["metadata"]["kind"], "reasoning");
    assert_eq!(parts[2]["metadata"]["data"]["state"], "completed");

    assert_eq!(parts[3]["part_id"], format!("{asst_msg_id}:text"));
    assert_eq!(parts[3]["metadata"]["kind"], "text");
    assert_eq!(parts[3]["metadata"]["data"]["is_final"], true);

    // Read committed markdown body via `read_assistant_journal_range`
    let text_range = crate::daemon::dispatch_command(
        &store,
        &suite,
        "read_assistant_journal_range",
        &json!({
            "snapshot": snap,
            "part_id": parts[3]["part_id"],
            "content": parts[3]["bodies"][0],
            "offset_bytes": "0",
            "limit_bytes": 4096
        }),
    )
    .unwrap();
    assert_eq!(
        text_range["text"],
        "# Summary\n\nArchitecture v2 uses **SQLite WAL + FTS5**."
    );

    // 5. Terminal seal persisted cleanly
    let seal = crate::daemon::dispatch_command(
        &store,
        &suite,
        "local_assistant_run_terminal_seal",
        &json!({"conversation_id": conv_id, "run_id": run_id}),
    )
    .unwrap();
    assert_eq!(seal["protocol_version"], 1);
    assert_eq!(seal["result"]["value"]["kind"], "answered");
}

#[test]
fn real_scenario_canonical_mac_host_reasoning_selection_none_sends_exact_wire_field() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().join("ws");
    fs::create_dir_all(&ws).unwrap();
    let db_path = dir.path().join("nola-none.db");
    let store = Store::open(&db_path).unwrap();
    bind_store_vault_path(&store, &dir.path().join("vault.json")).unwrap();
    let suite = Arc::new(ModulesSuite::new(store.clone(), ws.clone(), None));

    // Verify extraction from macOS `reasoning_selection: {"kind": "effort", "value": "none"}`
    let extracted = extract_reasoning_effort_from_req(&json!({
        "reasoning_selection": {"kind": "effort", "value": "none"}
    }));
    assert_eq!(extracted.as_deref(), Some("none"));

    let server = MockSseServer::spawn(1, |_idx, _req| {
        (
            200,
            "text/event-stream",
            vec![
                format!(
                    "data: {}\n\n",
                    json!({"id": "rn1", "model": "qwen3-local", "choices": [{"delta": {"content": "Direct answer without thinking."}, "finish_reason": "stop"}]})
                ),
                "data: [DONE]\n\n".to_string(),
            ],
        )
    });

    let opts = CliOptions {
        host_request_stdin: true,
        ..Default::default()
    };
    let overrides = HostRunOverrides {
        session_id: Some("conv-none-1".into()),
        submission_id: Some("run-none-1".into()),
        user_message_id: Some("u-none-1".into()),
        assistant_message_id: Some("a-none-1".into()),
        resume: Some(false),
        model_id: Some("qwen3-local".into()),
        endpoint: Some(server.endpoint()),
        api_key: Some("sk-none".into()),
        protocol: Some("openai_chat".into()),
        permission_profile: Some(PermissionProfile::Cautious),
        reasoning_effort: extracted,
        ..Default::default()
    };

    let rc = execute_agent_turn(
        &store,
        &suite,
        &db_path,
        &ws,
        &None,
        &opts,
        "2+2?",
        Some(overrides),
    )
    .unwrap();
    assert_eq!(rc, 0);

    let bodies = server.join_and_take_bodies();
    assert_eq!(bodies.len(), 1);
    assert_eq!(
        bodies[0]["reasoning_effort"],
        json!("none"),
        "reasoning_selection none MUST send exact \"reasoning_effort\": \"none\" in upstream JSON body"
    );
}

#[test]
fn real_scenario_expert_mentions_selected_experts_list_and_delegate_expert_e2e() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().join("ws");
    fs::create_dir_all(&ws).unwrap();
    let db_path = dir.path().join("nola-experts.db");
    let store = Store::open(&db_path).unwrap();
    bind_store_vault_path(&store, &dir.path().join("vault.json")).unwrap();
    let suite = Arc::new(ModulesSuite::new(store.clone(), ws.clone(), None));

    // Seed 2 active experts + 1 disabled expert
    store
        .put_module_record(
            "expert_peer",
            "exp-leijun",
            &json!({
                "id": "exp-leijun",
                "name": "雷军",
                "role": "specialist",
                "description": "小米创始人视角，专注硬件工程与性价比",
                "system_prompt": "",
                "availability": "active",
                "allowed_tools": [],
                "updated_at_ms": 1_795_800_000_000i64
            }),
        )
        .unwrap();
    store
        .put_module_record(
            "expert_peer",
            "exp-rust",
            &json!({
                "id": "exp-rust",
                "name": "RustReviewer",
                "role": "reviewer",
                "description": "Strict Rust memory-safety reviewer",
                "system_prompt": "Check lifetimes and unsafe blocks.",
                "availability": "active",
                "allowed_tools": [],
                "updated_at_ms": 1_795_800_000_000i64
            }),
        )
        .unwrap();
    store
        .put_module_record(
            "expert_peer",
            "exp-disabled",
            &json!({
                "id": "exp-disabled",
                "name": "RetiredBot",
                "role": "legacy",
                "description": "Should not appear",
                "system_prompt": "",
                "availability": "disabled",
                "allowed_tools": [],
                "updated_at_ms": 1_795_800_000_000i64
            }),
        )
        .unwrap();

    // 1. Verify @mention + selected_experts system block injection excludes disabled experts
    let ov_check = HostRunOverrides {
        selected_experts: vec![json!({"agent_id": "exp-rust", "name": "RustReviewer"})],
        ..Default::default()
    };
    let mut blocks = Vec::new();
    append_expert_and_host_context_blocks(
        &store,
        "@雷军 请给出建议",
        Some(&ov_check),
        &mut blocks,
    );
    let dir_block = blocks
        .iter()
        .find(|b| b.id == "saved_experts_directory")
        .expect("saved_experts_directory expected");
    assert!(dir_block.text.contains("雷军") && dir_block.text.contains("RustReviewer"));
    assert!(!dir_block.text.contains("RetiredBot"));
    let active_block = blocks
        .iter()
        .find(|b| b.id == "active_expert_mentions")
        .expect("active_expert_mentions expected");
    assert!(active_block.text.contains("雷军") && active_block.text.contains("RustReviewer"));
    assert!(active_block.text.contains("Check lifetimes and unsafe blocks."));

    // 2. Full E2E run over local HTTP SSE: parent calls list_experts -> delegate_expert -> child answers -> parent summarizes
    let server = MockSseServer::spawn(4, |idx, req| {
        let sys = req["messages"][0]["content"].as_str().unwrap_or("");
        if sys.contains("小米创始人视角") && !sys.contains("Saved ExpertPeer personas") {
            // Child delegate_expert call!
            return (
                200,
                "text/event-stream",
                vec![
                    format!(
                        "data: {}\n\n",
                        json!({"id": "child_1", "model": "qwen3-local", "choices": [{"delta": {"content": "极致性价比与用户体验优先。"}, "finish_reason": "stop"}]})
                    ),
                    "data: [DONE]\n\n".to_string(),
                ],
            );
        }
        match idx {
            0 => (
                200,
                "text/event-stream",
                vec![
                    format!(
                        "data: {}\n\n",
                        json!({"id": "p0", "model": "qwen3-local", "choices": [{"delta": {"tool_calls": [{"index": 0, "id": "call_le", "type": "function", "function": {"name": "list_experts", "arguments": "{}"}}]}, "finish_reason": "tool_calls"}]})
                    ),
                    "data: [DONE]\n\n".to_string(),
                ],
            ),
            1 => (
                200,
                "text/event-stream",
                vec![
                    format!(
                        "data: {}\n\n",
                        json!({"id": "p1", "model": "qwen3-local", "choices": [{"delta": {"tool_calls": [{"index": 0, "id": "call_de", "type": "function", "function": {"name": "delegate_expert", "arguments": json!({"expert_name": "雷军", "prompt": "分析产品定位", "background": false}).to_string()}}]}, "finish_reason": "tool_calls"}]})
                    ),
                    "data: [DONE]\n\n".to_string(),
                ],
            ),
            _ => (
                200,
                "text/event-stream",
                vec![
                    format!(
                        "data: {}\n\n",
                        json!({"id": "p3", "model": "qwen3-local", "choices": [{"delta": {"content": "雷军专家回复：极致性价比与用户体验优先。"}, "finish_reason": "stop"}]})
                    ),
                    "data: [DONE]\n\n".to_string(),
                ],
            ),
        }
    });

    let opts = CliOptions {
        host_request_stdin: true,
        force_tool_set: ForceToolSet::All,
        ..Default::default()
    };
    let overrides = HostRunOverrides {
        session_id: Some("conv-exp-e2e".into()),
        submission_id: Some("run-exp-e2e".into()),
        user_message_id: Some("u-exp-1".into()),
        assistant_message_id: Some("a-exp-1".into()),
        resume: Some(false),
        model_id: Some("qwen3-local".into()),
        endpoint: Some(server.endpoint()),
        api_key: Some("sk-exp".into()),
        protocol: Some("openai_chat".into()),
        permission_profile: Some(PermissionProfile::AllowAll),
        selected_experts: vec![json!({"agent_id": "exp-rust", "name": "RustReviewer"})],
        ..Default::default()
    };

    let rc = execute_agent_turn(
        &store,
        &suite,
        &db_path,
        &ws,
        &None,
        &opts,
        "@雷军 结合 RustReviewer 评审产品",
        Some(overrides),
    )
    .unwrap();
    assert_eq!(rc, 0);

    let bodies = server.join_and_take_bodies();
    assert_eq!(bodies.len(), 4, "expected 3 parent calls + 1 child delegate_expert call");
    // Parent call #0 system prompt has both experts injected
    let sys0 = bodies[0]["messages"][0]["content"].as_str().unwrap();
    assert!(sys0.contains("雷军") && sys0.contains("RustReviewer"));
    // Child call received 雷军's persona prompt
    assert!(bodies.iter().any(|b| {
        let s = b["messages"][0]["content"].as_str().unwrap_or("");
        s.contains("小米创始人视角") && !s.contains("Saved ExpertPeer personas")
    }));

    // Verify SQLite chat_assistant recorded both list_experts & delegate_expert succeeded
    let evts = match store.events_after("conv-exp-e2e", 0, 100).unwrap() {
        nola_store::EventPage::Events(e) => e,
        other => panic!("unexpected event page: {other:?}"),
    };
    let chat_asst = evts.iter().find(|e| e.kind == "chat_assistant").unwrap();
    let tcs = chat_asst.payload["toolCalls"].as_array().unwrap();
    assert_eq!(tcs.len(), 2);
    assert_eq!(tcs[0]["name"], "list_experts");
    assert_eq!(tcs[0]["is_error"], false);
    assert_eq!(tcs[1]["name"], "delegate_expert");
    assert_eq!(tcs[1]["is_error"], false);
}

#[test]
fn real_scenario_canonical_mac_host_plan_mode_context_component_blocks_writes() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().join("ws");
    fs::create_dir_all(&ws).unwrap();
    fs::write(ws.join("existing.txt"), "safe_content\n").unwrap();

    let db_path = dir.path().join("nola-plan.db");
    let store = Store::open(&db_path).unwrap();
    bind_store_vault_path(&store, &dir.path().join("vault.json")).unwrap();
    let suite = Arc::new(ModulesSuite::new(store.clone(), ws.clone(), None));

    let server = MockSseServer::spawn(2, |idx, _req| {
        if idx == 0 {
            (
                200,
                "text/event-stream",
                vec![
                    format!(
                        "data: {}\n\n",
                        json!({"id": "pl1", "model": "qwen3-local", "choices": [{"delta": {"tool_calls": [
                            {"index": 0, "id": "call_w", "type": "function", "function": {"name": "write_file", "arguments": r#"{"path":"should_not_create.txt","content":"bad"}"#}},
                            {"index": 1, "id": "call_r", "type": "function", "function": {"name": "read_file", "arguments": r#"{"path":"existing.txt"}"#}}
                        ]}, "finish_reason": "tool_calls"}]})
                    ),
                    "data: [DONE]\n\n".to_string(),
                ],
            )
        } else {
            (
                200,
                "text/event-stream",
                vec![
                    format!(
                        "data: {}\n\n",
                        json!({"id": "pl2", "model": "qwen3-local", "choices": [{"delta": {"content": "Read existing.txt; write blocked by plan mode."}, "finish_reason": "stop"}]})
                    ),
                    "data: [DONE]\n\n".to_string(),
                ],
            )
        }
    });

    let opts = CliOptions {
        host_request_stdin: true,
        ..Default::default()
    };
    let overrides = HostRunOverrides {
        session_id: Some("conv-plan-1".into()),
        submission_id: Some("run-plan-1".into()),
        user_message_id: Some("u-plan-1".into()),
        assistant_message_id: Some("a-plan-1".into()),
        resume: Some(false),
        model_id: Some("qwen3-local".into()),
        endpoint: Some(server.endpoint()),
        api_key: Some("sk-plan".into()),
        protocol: Some("openai_chat".into()),
        permission_profile: Some(PermissionProfile::AllowAll),
        context_components: vec![json!({
            "kind": "plan_mode",
            "id": "nola.plan-mode",
            "content": "Read-only planning mode active"
        })],
        ..Default::default()
    };

    let rc = execute_agent_turn(
        &store,
        &suite,
        &db_path,
        &ws,
        &None,
        &opts,
        "Plan edits",
        Some(overrides),
    )
    .unwrap();
    assert_eq!(rc, 0);
    let _ = server.join_and_take_bodies();

    assert!(!ws.join("should_not_create.txt").exists());
    let heads = crate::daemon::dispatch_command(
        &store,
        &suite,
        "local_run_assistant_journal_heads",
        &json!({"conversation_id": "conv-plan-1", "run_id": "run-plan-1", "limit": 64}),
    )
    .unwrap();
    let snap = crate::daemon::dispatch_command(
        &store,
        &suite,
        "open_assistant_journal_snapshot",
        &json!({"at": heads[0]}),
    )
    .unwrap();
    let parts = crate::daemon::dispatch_command(
        &store,
        &suite,
        "read_assistant_journal_parts",
        &json!({"snapshot": snap, "max_records": 64}),
    )
    .unwrap();
    let arr = parts["parts"].as_array().unwrap();
    let w_part = arr.iter().find(|p| p["part_id"] == "tool:call_w").unwrap();
    let r_part = arr.iter().find(|p| p["part_id"] == "tool:call_r").unwrap();
    assert_eq!(w_part["metadata"]["data"]["state"], "failed");
    assert_eq!(r_part["metadata"]["data"]["state"], "completed");
}

#[test]
fn real_scenario_canonical_mac_host_multi_turn_resume_preserves_conversation_history() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().join("ws");
    fs::create_dir_all(&ws).unwrap();
    let db_path = dir.path().join("nola-resume.db");
    let store = Store::open(&db_path).unwrap();
    bind_store_vault_path(&store, &dir.path().join("vault.json")).unwrap();
    let suite = Arc::new(ModulesSuite::new(store.clone(), ws.clone(), None));

    let server = MockSseServer::spawn(2, |idx, _req| {
        let reply = if idx == 0 {
            "Noted: favorite language is Rust."
        } else {
            "Your favorite language is Rust."
        };
        (
            200,
            "text/event-stream",
            vec![
                format!(
                    "data: {}\n\n",
                    json!({"id": format!("mt{idx}"), "model": "qwen3-local", "choices": [{"delta": {"content": reply}, "finish_reason": "stop"}]})
                ),
                "data: [DONE]\n\n".to_string(),
            ],
        )
    });

    let opts = CliOptions {
        host_request_stdin: true,
        ..Default::default()
    };
    let ep = server.endpoint();

    // Turn 1
    assert_eq!(
        execute_agent_turn(
            &store,
            &suite,
            &db_path,
            &ws,
            &None,
            &opts,
            "My favorite language is Rust.",
            Some(HostRunOverrides {
                session_id: Some("conv-mt-1".into()),
                submission_id: Some("run-mt-1".into()),
                user_message_id: Some("u-mt-1".into()),
                assistant_message_id: Some("a-mt-1".into()),
                resume: Some(false),
                model_id: Some("qwen3-local".into()),
                endpoint: Some(ep.clone()),
                api_key: Some("sk-mt".into()),
                protocol: Some("openai_chat".into()),
                permission_profile: Some(PermissionProfile::Cautious),
                ..Default::default()
            }),
        )
        .unwrap(),
        0
    );

    // Turn 2 with resume=true
    assert_eq!(
        execute_agent_turn(
            &store,
            &suite,
            &db_path,
            &ws,
            &None,
            &opts,
            "What is my favorite language?",
            Some(HostRunOverrides {
                session_id: Some("conv-mt-1".into()),
                submission_id: Some("run-mt-2".into()),
                user_message_id: Some("u-mt-2".into()),
                assistant_message_id: Some("a-mt-2".into()),
                resume: Some(true),
                model_id: Some("qwen3-local".into()),
                endpoint: Some(ep),
                api_key: Some("sk-mt".into()),
                protocol: Some("openai_chat".into()),
                permission_profile: Some(PermissionProfile::Cautious),
                ..Default::default()
            }),
        )
        .unwrap(),
        0
    );

    let bodies = server.join_and_take_bodies();
    assert_eq!(bodies.len(), 2);
    let req2_msgs = serde_json::to_string(&bodies[1]["messages"]).unwrap();
    assert!(req2_msgs.contains("My favorite language is Rust."));
    assert!(req2_msgs.contains("Noted: favorite language is Rust."));
    assert!(req2_msgs.contains("What is my favorite language?"));

    let seal1 = crate::daemon::dispatch_command(
        &store,
        &suite,
        "local_assistant_run_terminal_seal",
        &json!({"conversation_id": "conv-mt-1", "run_id": "run-mt-1"}),
    )
    .unwrap();
    let seal2 = crate::daemon::dispatch_command(
        &store,
        &suite,
        "local_assistant_run_terminal_seal",
        &json!({"conversation_id": "conv-mt-1", "run_id": "run-mt-2"}),
    )
    .unwrap();
    assert_eq!(seal1["result"]["value"]["kind"], "answered");
    assert_eq!(seal2["result"]["value"]["kind"], "answered");
}

#[test]
fn real_scenario_canonical_mac_host_upstream_http_error_commits_failed_seal_and_event() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().join("ws");
    fs::create_dir_all(&ws).unwrap();
    let db_path = dir.path().join("nola-err.db");
    let store = Store::open(&db_path).unwrap();
    bind_store_vault_path(&store, &dir.path().join("vault.json")).unwrap();
    let suite = Arc::new(ModulesSuite::new(store.clone(), ws.clone(), None));

    // HTTP 400 fails immediately without retry backoff
    let server = MockSseServer::spawn(1, |_idx, _req| {
        (
            400,
            "application/json",
            vec![r#"{"error":{"message":"invalid model request"}}"#.to_string()],
        )
    });

    let opts = CliOptions {
        host_request_stdin: true,
        ..Default::default()
    };
    let overrides = HostRunOverrides {
        session_id: Some("conv-err-1".into()),
        submission_id: Some("run-err-1".into()),
        user_message_id: Some("u-err-1".into()),
        assistant_message_id: Some("a-err-1".into()),
        resume: Some(false),
        model_id: Some("bad-model".into()),
        endpoint: Some(server.endpoint()),
        api_key: Some("sk-err".into()),
        protocol: Some("openai_chat".into()),
        permission_profile: Some(PermissionProfile::Cautious),
        ..Default::default()
    };

    let res = execute_agent_turn(
        &store,
        &suite,
        &db_path,
        &ws,
        &None,
        &opts,
        "Hello",
        Some(overrides),
    );
    assert!(res.is_err(), "HTTP 400 must fail execute_agent_turn");
    let _ = server.join_and_take_bodies();

    let evts = match store.events_after("conv-err-1", 0, 50).unwrap() {
        nola_store::EventPage::Events(e) => e,
        other => panic!("unexpected event page: {other:?}"),
    };
    assert!(evts.iter().any(|e| e.kind == "run_error" && e.payload["status"] == "failed"));

    let seal = crate::daemon::dispatch_command(
        &store,
        &suite,
        "local_assistant_run_terminal_seal",
        &json!({"conversation_id": "conv-err-1", "run_id": "run-err-1"}),
    )
    .unwrap();
    assert_eq!(seal["protocol_version"], 1);
    assert_ne!(seal["result"]["value"]["kind"], "answered");
}
