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
        session_id: None,
        submission_id: None,
        user_message_id: None,
        assistant_message_id: None,
        resume: None,
        model_id: Some("local-qwen".into()),
        endpoint: Some("http://127.0.0.1:8080/v1".into()),
        api_key: Some("sk-override".into()),
        protocol: Some("openai_chat".into()),
        permission_profile: None,
        reasoning_effort: Some("high".into()),
        temperature: None,
        top_p: None,
        max_output_tokens: None,
        max_iterations: None,
        context_window_tokens: None,
    };
    let route = resolve_route_with_overrides(&cfg, &opts, Some(&overrides)).unwrap();
    assert_eq!(route.model, "local-qwen");
    assert_eq!(route.endpoint, "http://127.0.0.1:8080/v1");
    assert_eq!(route.api_key, "sk-override");
    assert_eq!(route.protocol, ModelProtocol::OpenAiChat);
    assert_eq!(route.reasoning_effort.as_deref(), Some("high"));

    // Partial override still applies on top of default model
    let partial = HostRunOverrides {
        session_id: None,
        submission_id: None,
        user_message_id: None,
        assistant_message_id: None,
        resume: None,
        model_id: Some("gpt-5".into()),
        endpoint: None,
        api_key: None,
        protocol: Some("gemini".into()),
        permission_profile: None,
        reasoning_effort: Some("medium".into()),
        temperature: None,
        top_p: None,
        max_output_tokens: None,
        max_iterations: None,
        context_window_tokens: None,
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
