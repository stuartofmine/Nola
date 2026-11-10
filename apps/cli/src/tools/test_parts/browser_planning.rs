#[test]
fn chrome_native_action_mapping_and_validation_match_web_surface() {
    let expected = [
        ("chrome_native_status", "status"),
        ("chrome_native_inspect", "inspect"),
        ("chrome_native_screenshot", "screenshot"),
        ("chrome_native_dom_snapshot", "dom_snapshot"),
        (
            "chrome_native_accessibility_snapshot",
            "accessibility_snapshot",
        ),
        ("chrome_native_storage_state", "storage_state"),
        ("chrome_native_console_logs", "console_logs"),
        ("chrome_native_network_events", "network_events"),
        ("chrome_native_list_pages", "list_pages"),
        ("chrome_native_switch_page", "switch_page"),
        ("chrome_native_open_page", "open_page"),
        ("chrome_native_hover", "hover"),
        ("chrome_native_click", "click"),
        ("chrome_native_drag", "drag"),
        ("chrome_native_type_text", "type_text"),
        ("chrome_native_select_option", "select_option"),
        ("chrome_native_set_checked", "set_checked"),
        ("chrome_native_press_key", "press_key"),
        ("chrome_native_scroll", "scroll"),
        ("chrome_native_assert", "assert"),
        ("chrome_native_wait_for", "wait_for"),
    ];

    for (tool_name, action) in expected {
        assert_eq!(chrome_native_action(tool_name), Some(action), "{tool_name}");
    }

    assert!(validate_chrome_native_tool_args("status", &Map::new()).is_ok());
    assert!(validate_chrome_native_tool_args("list_pages", &Map::new()).is_ok());
    assert!(validate_chrome_native_tool_args("console_logs", &Map::new()).is_ok());
    assert!(validate_chrome_native_tool_args("network_events", &Map::new()).is_ok());
    let mut status_args = Map::new();
    status_args.insert("screenshot".to_string(), Value::Bool(false));
    assert!(validate_chrome_native_tool_args("status", &status_args).is_err());
    assert!(validate_chrome_native_tool_args("list_pages", &status_args).is_err());

    let mut diagnostic_args = Map::new();
    diagnostic_args.insert("level".to_string(), Value::String("errors".to_string()));
    diagnostic_args.insert("max_entries".to_string(), Value::from(1));
    assert!(validate_chrome_native_tool_args("console_logs", &diagnostic_args).is_ok());
    assert!(validate_chrome_native_tool_args("network_events", &diagnostic_args).is_ok());
    diagnostic_args.insert("clear".to_string(), Value::String("false".to_string()));
    assert!(validate_chrome_native_tool_args("console_logs", &diagnostic_args).is_err());

    let mut switch_args = Map::new();
    switch_args.insert("index".to_string(), Value::from(2));
    assert!(validate_chrome_native_tool_args("switch_page", &switch_args).is_ok());
    switch_args.insert("title".to_string(), Value::String("Docs".to_string()));
    assert!(validate_chrome_native_tool_args("switch_page", &switch_args).is_err());

    let mut open_args = Map::new();
    open_args.insert(
        "url".to_string(),
        Value::String("https://example.com".to_string()),
    );
    open_args.insert("new_tab".to_string(), Value::Bool(true));
    assert!(validate_chrome_native_tool_args("open_page", &open_args).is_ok());
    open_args.insert(
        "url".to_string(),
        Value::String("file:///tmp/index.html".to_string()),
    );
    assert!(validate_chrome_native_tool_args("open_page", &open_args).is_err());

    let mut click_args = Map::new();
    click_args.insert(
        "selector".to_string(),
        Value::String("button.save".to_string()),
    );
    assert!(validate_chrome_native_tool_args("click", &click_args).is_ok());
    click_args.insert("button".to_string(), Value::String("right".to_string()));
    click_args.insert("clicks".to_string(), Value::from(2));
    click_args.insert("modifiers".to_string(), json!(["Meta", "Shift"]));
    click_args.insert("wait_after_ms".to_string(), Value::from(0));
    assert!(validate_chrome_native_tool_args("click", &click_args).is_ok());
    click_args.insert("clicks".to_string(), Value::from(0));
    assert!(validate_chrome_native_tool_args("click", &click_args).is_err());
    click_args.insert("clicks".to_string(), Value::from(2));
    click_args.insert("modifiers".to_string(), json!(["Hyper"]));
    assert!(validate_chrome_native_tool_args("click", &click_args).is_err());
    click_args.insert("modifiers".to_string(), json!(["Meta", "Shift"]));
    click_args.insert("wait_after_ms".to_string(), Value::from(5001));
    assert!(validate_chrome_native_tool_args("click", &click_args).is_err());
    click_args.insert("wait_after_ms".to_string(), Value::from(0));
    click_args.insert("text".to_string(), Value::String("Save".to_string()));
    assert!(validate_chrome_native_tool_args("click", &click_args).is_err());

    let mut select_args = Map::new();
    select_args.insert("label".to_string(), Value::String("Region".to_string()));
    select_args.insert("option_label".to_string(), Value::String("US".to_string()));
    assert!(validate_chrome_native_tool_args("select_option", &select_args).is_ok());
    select_args.insert("value".to_string(), Value::String("us".to_string()));
    assert!(validate_chrome_native_tool_args("select_option", &select_args).is_err());

    let mut scroll_args = Map::new();
    scroll_args.insert("direction".to_string(), Value::String("right".to_string()));
    scroll_args.insert("amount".to_string(), Value::from(320));
    assert!(validate_chrome_native_tool_args("scroll", &scroll_args).is_ok());
    scroll_args.insert(
        "direction".to_string(),
        Value::String("sideways".to_string()),
    );
    assert!(validate_chrome_native_tool_args("scroll", &scroll_args).is_err());

    let mut wait_args = Map::new();
    wait_args.insert("text".to_string(), Value::String("Done".to_string()));
    wait_args.insert("timeout_ms".to_string(), Value::from(1000));
    assert!(validate_chrome_native_tool_args("wait_for", &wait_args).is_ok());
    assert_eq!(chrome_native_timeout_sec("wait_for", &wait_args), 45.0);
}

#[test]
fn web_host_base_url_normalizes_api_paths_for_native_browser_bridge() {
    assert_eq!(
        normalize_web_host_base_url(" http://127.0.0.1:5001/api ").unwrap(),
        "http://127.0.0.1:5001"
    );
    assert_eq!(
        normalize_web_host_base_url("http://localhost:5001/api/mobile/login?x=1#frag").unwrap(),
        "http://localhost:5001"
    );
    assert_eq!(
        normalize_web_host_base_url("https://host.local/nola/api/mobile/login").unwrap(),
        "https://host.local/nola"
    );
    assert_eq!(
        normalize_web_host_base_url("https://host.local/nola/api").unwrap(),
        "https://host.local/nola"
    );
    assert_eq!(
        normalize_web_host_base_url("https://host.local/console/").unwrap(),
        "https://host.local/console"
    );
    assert!(normalize_web_host_base_url("file:///tmp/nola").is_err());
    assert!(normalize_web_host_base_url("https://user:pass@host.local").is_err());
}

#[test]
fn chrome_native_result_saves_screenshot_and_omits_data_url() {
    let root = temp_root("chrome-native-result");
    let ctx = ToolContext {
        memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
        language_services: Default::default(),
        skills: Default::default(),
        allowed_dirs: vec![root.clone()],
        policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), true, "allow".to_string(), Vec::new()),
        permission_scope: Default::default(),
        cancellation: Default::default(),
        plan_state: Default::default(),
        output_store: crate::tools::ToolOutputStore::for_test(),
        browser: BrowserRuntime::new(root.join("browser-state")),
    };
    let mut result = Map::new();
    result.insert("ok".to_string(), Value::Bool(true));
    result.insert("title".to_string(), Value::String("Nola".to_string()));
    result.insert(
        "final_url".to_string(),
        Value::String("https://example.com".to_string()),
    );
    result.insert(
        "visible_text".to_string(),
        Value::String("Hello from real Chrome".to_string()),
    );
    result.insert(
        "screenshot_data_url".to_string(),
        Value::String("data:image/png;base64,iVBORw0KGgo=".to_string()),
    );

    let outcome = format_chrome_native_result("inspect", result, &ctx, true);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(!outcome.text.contains("screenshot_data_url"));
    let structured = outcome.data.as_ref().expect("structured browser result");
    assert_eq!(structured["visible_text"], "Hello from real Chrome");
    assert!(structured.get("text").is_none(), "presentation must not duplicate business evidence");
    let human_text = &outcome.text;
    assert!(human_text.contains("Screenshot:"), "{human_text}");
    assert_eq!(outcome.artifacts.len(), 1, "{outcome:#?}");
    assert_eq!(outcome.artifacts[0].mime_type.as_deref(), Some("image/png"));
    assert_eq!(outcome.artifacts[0].operation.as_deref(), Some("observe"));
    let screenshot_path = url::Url::parse(&outcome.artifacts[0].uri).unwrap()
        .to_file_path().expect("local screenshot artifact URI");
    assert!(screenshot_path.is_file());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn computer_tool_schema_dispatch_and_helper_actions_stay_in_sync() {
    let schema_names = specs()
        .into_iter()
        .filter_map(|spec| spec.name.strip_prefix("computer_").map(ToOwned::to_owned))
        .collect::<std::collections::BTreeSet<_>>();
    let helper_block = COMPUTER_HELPER
        .split("COMPUTER_ACTIONS = [")
        .nth(1)
        .and_then(|tail| tail.split("]\n\nKEY_CODES").next())
        .expect("computer helper action list should be parseable");
    let helper_names = helper_block
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            line.strip_prefix('"')
                .and_then(|tail| tail.split('"').next())
                .map(ToOwned::to_owned)
        })
        .collect::<std::collections::BTreeSet<_>>();
    let source = include_str!("../parts/approval.rs");

    assert_eq!(schema_names, helper_names);
    for action in schema_names {
        let name = format!("computer_{action}");
        assert!(
            source.contains(&format!("\"{name}\" =>")),
            "{name} is missing from execute_tool dispatch"
        );
        assert!(
            source.contains(&format!("run_computer_tool(\"{name}\"")),
            "{name} dispatch does not call run_computer_tool"
        );
    }
}

#[test]
fn browser_url_validation_allows_local_dev_targets() {
    assert!(validate_browser_http_url("http://localhost:3000").is_ok());
    assert!(validate_browser_http_url("http://127.0.0.1:3000").is_ok());
    assert!(validate_browser_http_url("http://[::1]:3000").is_ok());
    assert!(validate_browser_http_url("http://0.0.0.0:3000").is_err());
    assert!(validate_browser_http_url("http://169.254.169.254/latest/meta-data").is_err());
    assert!(validate_browser_http_url("file:///tmp/index.html").is_err());
    assert!(validate_browser_page_url(&url::Url::from_file_path(std::env::temp_dir().join("index.html")).unwrap().to_string()).is_ok());
    let mut local_file = url::Url::from_file_path(std::env::temp_dir().join("index.html")).unwrap();
    local_file.set_host(Some("localhost")).unwrap();
    assert!(validate_browser_page_url(local_file.as_str()).is_ok());
    assert!(validate_browser_page_url("file://remote-host/tmp/index.html").is_err());
    assert!(validate_browser_http_url("https://example.com").is_ok());
    assert!(validate_browser_http_url("http://192.168.1.10:8080").is_ok());
}

#[test]
fn public_web_contracts_do_not_require_per_query_confirmation() {
    for name in ["web_fetch","web_search"] {
        let contract = builtin_permission_contract(name,true);
        assert!(contract.resource_plan.is_none());
        for input in [json!({"query":"ordinary"}),json!({"query":"検索 follow-up","operation_kind":"execute"})] {
            assert!(!nola_core::agent::permissions::permission_contract_requires_confirmation(&contract.for_input(&input),None));
        }
    }
}

#[test]
fn independent_web_reads_are_safe_for_host_batch_execution() {
    assert!(tool_supports_isolated_parallel_execution("system_time"));
    assert!(tool_supports_isolated_parallel_execution("web_search"));
    assert!(tool_supports_isolated_parallel_execution("web_fetch"));
    assert!(!tool_supports_isolated_parallel_execution("write_file"));
    assert!(!tool_supports_isolated_parallel_execution("bash"));
}

#[test]
fn permission_ask_reports_missing_transport_and_yes_cannot_forge_a_grant() {
    let _guard = test_lock();
    let _env_guard = crate::store::test_env_lock();
    std::env::set_var("NOLA_CLI_INTERACTION_MODE", "never");
    let root = temp_root("permission-ask-interaction");
    let target = root.join("note.txt");
    fs::write(&target, "hello").unwrap();
    let request = ToolRequest {
        id: "call-read".to_string(),
        name: "read_file".to_string(),
        args: object_args(json!({"path": target.display().to_string()})),
    };
    let rules = vec![crate::runtime_config::PermissionRule {
        permission: "read".to_string(),
        pattern: "*".to_string(),
        action: crate::runtime_config::PermissionAction::Ask,
    }];

    let denied = execute_tool(
        &request,
        &ToolContext {
            memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
            language_services: Default::default(),
            skills: Default::default(),
            allowed_dirs: vec![root.clone()],
            policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), false, "allow".to_string(), rules.clone()),
            permission_scope: Default::default(),
            cancellation: Default::default(),
            plan_state: Default::default(),
            output_store: crate::tools::ToolOutputStore::for_test(),
            browser: BrowserRuntime::new(root.join("browser-state-denied")),
        },
    );
    assert!(denied.error);
    assert!(denied.text.contains("Permission transport interrupted (Unavailable)"));
    assert!(!denied.text.contains("denied by user"));

    let approved = execute_tool(
        &request,
        &ToolContext {
            memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
            language_services: Default::default(),
            skills: Default::default(),
            allowed_dirs: vec![root.clone()],
            policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), true, "allow".to_string(), rules),
            permission_scope: Default::default(),
            cancellation: Default::default(),
            plan_state: Default::default(),
            output_store: crate::tools::ToolOutputStore::for_test(),
            browser: BrowserRuntime::new(root.join("browser-state-approved")),
        },
    );
    assert!(approved.error, "auto-approve is not a formal permission profile");
    assert!(approved.text.contains("Permission transport interrupted (Unavailable)"));
    assert!(!approved.text.contains("denied by user"));

    std::env::remove_var("NOLA_CLI_INTERACTION_MODE");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn computer_screenshot_output_path_is_sandboxed() {
    let root = temp_root("computer-output-path");
    let outside = temp_root("computer-output-outside");
    let ctx = ToolContext {
        memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
        language_services: Default::default(),
        skills: Default::default(),
        allowed_dirs: vec![root.clone()],
        policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), true, "allow".to_string(), Vec::new()),
        permission_scope: Default::default(),
        cancellation: Default::default(),
        plan_state: Default::default(),
        output_store: crate::tools::ToolOutputStore::for_test(),
        browser: BrowserRuntime::new(temp_root("browser-runtime-computer-output")),
    };
    let mut args = Map::new();
    let inside_target = root.join("captures").join("screen");
    args.insert(
        "path".to_string(),
        Value::String(inside_target.display().to_string()),
    );

    let prepared = prepare_computer_tool_args("computer_screenshot", &args, &ctx).unwrap();

    assert_eq!(
        prepared.get("path").and_then(Value::as_str).unwrap(),
        inside_target.to_string_lossy().as_ref()
    );

    args.insert(
        "path".to_string(),
        Value::String(outside.join("screen.png").display().to_string()),
    );
    assert!(prepare_computer_tool_args("computer_screenshot", &args, &ctx).is_ok());

    args.insert(
        "path".to_string(),
        Value::String(outside.join("wait.png").display().to_string()),
    );
    args.insert("text".to_string(), Value::String("Ready".to_string()));
    assert!(prepare_computer_tool_args("computer_wait_for_text", &args, &ctx).is_ok());

    args.insert(
        "path".to_string(),
        Value::String(root.join("bad\nname.png").display().to_string()),
    );
    let err = prepare_computer_tool_args("computer_screenshot", &args, &ctx).unwrap_err();
    assert!(err.text.contains("path is invalid"));

    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(outside);
}

#[test]
fn computer_open_item_local_paths_are_sandboxed() {
    let root = temp_root("computer-open-item");
    let file = root.join("document.txt");
    let folder = root.join("folder");
    fs::write(&file, "Nola").unwrap();
    fs::create_dir_all(&folder).unwrap();
    let outside = temp_root("computer-open-outside");
    let outside_file = outside.join("secret.txt");
    fs::write(&outside_file, "secret").unwrap();
    let ctx = ToolContext {
        memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
        language_services: Default::default(),
        skills: Default::default(),
        allowed_dirs: vec![root.clone()],
        policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), true, "allow".to_string(), Vec::new()),
        permission_scope: Default::default(),
        cancellation: Default::default(),
        plan_state: Default::default(),
        output_store: crate::tools::ToolOutputStore::for_test(),
        browser: BrowserRuntime::new(temp_root("browser-runtime-open-item")),
    };
    let mut args = Map::new();
    args.insert(
        "target".to_string(),
        Value::String(file_url_for_path(&file).unwrap()),
    );

    let prepared = prepare_computer_tool_args("computer_open_item", &args, &ctx).unwrap();
    assert_eq!(
        prepared.get("target").and_then(Value::as_str).unwrap(),
        file.to_string_lossy().as_ref()
    );

    args.insert(
        "target".to_string(),
        Value::String(folder.display().to_string()),
    );
    let prepared = prepare_computer_tool_args("computer_open_item", &args, &ctx).unwrap();
    assert_eq!(
        prepared.get("target").and_then(Value::as_str).unwrap(),
        folder.to_string_lossy().as_ref()
    );

    args.insert(
        "target".to_string(),
        Value::String("https://example.com".to_string()),
    );
    let prepared = prepare_computer_tool_args("computer_open_item", &args, &ctx).unwrap();
    assert_eq!(
        prepared.get("target").and_then(Value::as_str),
        Some("https://example.com")
    );

    args.insert(
        "target".to_string(),
        Value::String(outside_file.display().to_string()),
    );
    assert!(prepare_computer_tool_args("computer_open_item", &args, &ctx).is_ok());

    args.insert(
        "target".to_string(),
        Value::String(root.join("missing.txt").display().to_string()),
    );
    let err = prepare_computer_tool_args("computer_open_item", &args, &ctx).unwrap_err();
    assert!(err.text.contains("path does not exist"));

    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(outside);
}

#[test]
fn browser_condition_tools_validate_args() {
    let mut args = Map::new();
    assert!(validate_browser_tool_args("browser_inspect", &args).is_ok());
    args.insert("max_text_chars".to_string(), Value::from(499));
    assert!(validate_browser_tool_args("browser_inspect", &args).is_err());
    args.clear();

    assert!(validate_browser_tool_args("browser_console_logs", &args).is_ok());
    args.insert("level".to_string(), Value::String("errors".to_string()));
    args.insert("max_entries".to_string(), Value::from(1));
    args.insert("clear".to_string(), Value::Bool(true));
    assert!(validate_browser_tool_args("browser_console_logs", &args).is_ok());
    args.insert("level".to_string(), Value::String("verbose".to_string()));
    assert!(validate_browser_tool_args("browser_console_logs", &args).is_err());
    args.insert("level".to_string(), Value::String("all".to_string()));
    args.insert("max_entries".to_string(), Value::from(0));
    assert!(validate_browser_tool_args("browser_console_logs", &args).is_err());
    args.insert("max_entries".to_string(), Value::from(1));
    args.insert("clear".to_string(), Value::String("false".to_string()));
    assert!(validate_browser_tool_args("browser_console_logs", &args).is_err());
    args.clear();

    assert!(validate_browser_tool_args("browser_network_events", &args).is_ok());
    args.insert("level".to_string(), Value::String("errors".to_string()));
    args.insert("max_entries".to_string(), Value::from(1));
    args.insert("clear".to_string(), Value::Bool(true));
    assert!(validate_browser_tool_args("browser_network_events", &args).is_ok());
    args.insert("level".to_string(), Value::String("verbose".to_string()));
    assert!(validate_browser_tool_args("browser_network_events", &args).is_err());
    args.insert("level".to_string(), Value::String("warnings".to_string()));
    args.insert("max_entries".to_string(), Value::from(0));
    assert!(validate_browser_tool_args("browser_network_events", &args).is_err());
    args.insert("max_entries".to_string(), Value::from(1));
    args.insert("clear".to_string(), Value::String("false".to_string()));
    assert!(validate_browser_tool_args("browser_network_events", &args).is_err());
    args.clear();

    args.insert(
        "selector".to_string(),
        Value::String("button.primary".to_string()),
    );
    assert!(validate_browser_tool_args("browser_dom_snapshot", &args).is_ok());
    args.insert("text".to_string(), Value::String("Login".to_string()));
    assert!(validate_browser_tool_args("browser_dom_snapshot", &args).is_err());
    args.clear();
    args.insert("text".to_string(), Value::String("Login".to_string()));
    args.insert("include_styles".to_string(), Value::Bool(false));
    args.insert("include_html".to_string(), Value::Bool(false));
    args.insert("include_children".to_string(), Value::Bool(true));
    args.insert("max_html_chars".to_string(), Value::from(0));
    assert!(validate_browser_tool_args("browser_dom_snapshot", &args).is_ok());
    args.insert("max_html_chars".to_string(), Value::from(20_001));
    assert!(validate_browser_tool_args("browser_dom_snapshot", &args).is_err());
    args.clear();
    args.insert("x".to_string(), Value::from(10));
    assert!(validate_browser_tool_args("browser_dom_snapshot", &args).is_err());
    args.insert("y".to_string(), Value::from(20));
    assert!(validate_browser_tool_args("browser_dom_snapshot", &args).is_ok());
    args.insert(
        "include_styles".to_string(),
        Value::String("false".to_string()),
    );
    assert!(validate_browser_tool_args("browser_dom_snapshot", &args).is_err());
    args.clear();

    assert!(validate_browser_tool_args("browser_storage_state", &args).is_ok());
    args.insert("include_values".to_string(), Value::Bool(true));
    args.insert(
        "key_filter".to_string(),
        Value::String("locale".to_string()),
    );
    args.insert("max_items".to_string(), Value::from(1));
    args.insert("max_value_chars".to_string(), Value::from(0));
    assert!(validate_browser_tool_args("browser_storage_state", &args).is_ok());
    args.insert("max_items".to_string(), Value::from(0));
    assert!(validate_browser_tool_args("browser_storage_state", &args).is_err());
    args.insert("max_items".to_string(), Value::from(1));
    args.insert("max_value_chars".to_string(), Value::from(5001));
    assert!(validate_browser_tool_args("browser_storage_state", &args).is_err());
    args.insert("max_value_chars".to_string(), Value::from(500));
    args.insert(
        "key_filter".to_string(),
        Value::String("bad\nkey".to_string()),
    );
    assert!(validate_browser_tool_args("browser_storage_state", &args).is_err());
    args.clear();

    assert!(validate_browser_tool_args("browser_accessibility_snapshot", &args).is_ok());
    args.insert("selector".to_string(), Value::String("main".to_string()));
    args.insert("include_hidden".to_string(), Value::Bool(false));
    args.insert("include_bounds".to_string(), Value::Bool(true));
    args.insert("include_selectors".to_string(), Value::Bool(true));
    args.insert("include_values".to_string(), Value::Bool(false));
    args.insert("max_nodes".to_string(), Value::from(1));
    args.insert("max_name_chars".to_string(), Value::from(20));
    assert!(validate_browser_tool_args("browser_accessibility_snapshot", &args).is_ok());
    args.insert("text".to_string(), Value::String("Login".to_string()));
    assert!(validate_browser_tool_args("browser_accessibility_snapshot", &args).is_err());
    args.clear();
    args.insert("x".to_string(), Value::from(10));
    assert!(validate_browser_tool_args("browser_accessibility_snapshot", &args).is_err());
    args.insert("y".to_string(), Value::from(20));
    assert!(validate_browser_tool_args("browser_accessibility_snapshot", &args).is_ok());
    args.insert(
        "include_bounds".to_string(),
        Value::String("false".to_string()),
    );
    assert!(validate_browser_tool_args("browser_accessibility_snapshot", &args).is_err());
    args.clear();
    args.insert("max_nodes".to_string(), Value::from(0));
    assert!(validate_browser_tool_args("browser_accessibility_snapshot", &args).is_err());
    args.clear();
    args.insert("max_name_chars".to_string(), Value::from(19));
    assert!(validate_browser_tool_args("browser_accessibility_snapshot", &args).is_err());
    args.clear();

    assert!(validate_browser_tool_args("browser_list_pages", &args).is_ok());
    args.insert("index".to_string(), Value::from(1));
    assert!(validate_browser_tool_args("browser_list_pages", &args).is_ok());
    args.clear();

    args.insert("index".to_string(), Value::from(1));
    assert!(validate_browser_tool_args("browser_switch_page", &args).is_ok());
    args.insert(
        "url_contains".to_string(),
        Value::String("/callback".to_string()),
    );
    assert!(validate_browser_tool_args("browser_switch_page", &args).is_err());
    args.clear();
    args.insert(
        "title_contains".to_string(),
        Value::String("OAuth".to_string()),
    );
    assert!(validate_browser_tool_args("browser_switch_page", &args).is_ok());
    args.clear();
    args.insert(
        "url_contains".to_string(),
        Value::String("/callback".to_string()),
    );
    args.insert("max_text_chars".to_string(), Value::from(500));
    assert!(validate_browser_tool_args("browser_switch_page", &args).is_ok());
    args.insert("max_text_chars".to_string(), Value::from(499));
    assert!(validate_browser_tool_args("browser_switch_page", &args).is_err());
    args.clear();
    assert!(validate_browser_tool_args("browser_switch_page", &args).is_err());
    args.insert("index".to_string(), Value::from(0));
    assert!(validate_browser_tool_args("browser_switch_page", &args).is_err());
    args.clear();
    args.insert(
        "title_contains".to_string(),
        Value::String("bad\n".to_string()),
    );
    assert!(validate_browser_tool_args("browser_switch_page", &args).is_err());
    args.clear();

    assert!(validate_browser_tool_args("browser_screenshot", &args).is_ok());
    args.insert("full_page".to_string(), Value::Bool(true));
    assert!(validate_browser_tool_args("browser_screenshot", &args).is_ok());
    args.insert("x".to_string(), Value::from(10));
    args.insert("y".to_string(), Value::from(20));
    args.insert("width".to_string(), Value::from(300));
    args.insert("height".to_string(), Value::from(200));
    assert!(validate_browser_tool_args("browser_screenshot", &args).is_err());
    args.insert("full_page".to_string(), Value::Bool(false));
    assert!(validate_browser_tool_args("browser_screenshot", &args).is_ok());
    args.remove("height");
    assert!(validate_browser_tool_args("browser_screenshot", &args).is_err());
    args.clear();

    args.insert("selector".to_string(), Value::String("button".to_string()));
    assert!(validate_browser_tool_args("browser_hover", &args).is_ok());
    args.insert("text".to_string(), Value::String("Menu".to_string()));
    assert!(validate_browser_tool_args("browser_hover", &args).is_err());
    args.clear();
    args.insert("x".to_string(), Value::from(10));
    assert!(validate_browser_tool_args("browser_hover", &args).is_err());
    args.insert("y".to_string(), Value::from(20));
    args.insert("wait_after_ms".to_string(), Value::from(0));
    assert!(validate_browser_tool_args("browser_hover", &args).is_ok());
    args.insert("wait_after_ms".to_string(), Value::from(5001));
    assert!(validate_browser_tool_args("browser_hover", &args).is_err());
    args.clear();

    args.insert("text".to_string(), Value::String("Delete".to_string()));
    args.insert(
        "dialog_action".to_string(),
        Value::String("accept".to_string()),
    );
    args.insert("expect_download".to_string(), Value::Bool(true));
    args.insert("download_timeout_ms".to_string(), Value::from(1000));
    assert!(validate_browser_tool_args("browser_click", &args).is_ok());
    args.insert("clicks".to_string(), Value::from(2));
    args.insert("modifiers".to_string(), json!(["Command", "Shift"]));
    assert!(validate_browser_tool_args("browser_click", &args).is_ok());
    args.insert("clicks".to_string(), Value::from(0));
    assert!(validate_browser_tool_args("browser_click", &args).is_err());
    args.insert("clicks".to_string(), Value::from(2));
    args.insert("modifiers".to_string(), json!(["Hyper"]));
    assert!(validate_browser_tool_args("browser_click", &args).is_err());
    args.insert("modifiers".to_string(), json!(["Command", "Shift"]));
    args.insert(
        "dialog_prompt_text".to_string(),
        Value::String("Nola".to_string()),
    );
    assert!(validate_browser_tool_args("browser_click", &args).is_ok());
    args.insert(
        "dialog_action".to_string(),
        Value::String("dismiss".to_string()),
    );
    assert!(validate_browser_tool_args("browser_click", &args).is_err());
    args.insert(
        "dialog_action".to_string(),
        Value::String("close".to_string()),
    );
    args.remove("dialog_prompt_text");
    assert!(validate_browser_tool_args("browser_click", &args).is_err());
    args.insert(
        "dialog_action".to_string(),
        Value::String("accept".to_string()),
    );
    args.insert("download_timeout_ms".to_string(), Value::from(999));
    assert!(validate_browser_tool_args("browser_click", &args).is_err());
    args.insert("download_timeout_ms".to_string(), Value::from(1000));
    args.insert(
        "expect_download".to_string(),
        Value::String("yes".to_string()),
    );
    assert!(validate_browser_tool_args("browser_click", &args).is_err());
    args.insert("expect_download".to_string(), Value::Bool(false));
    args.insert("expect_popup".to_string(), Value::Bool(true));
    args.insert("popup_timeout_ms".to_string(), Value::from(1000));
    assert!(validate_browser_tool_args("browser_click", &args).is_ok());
    args.insert("popup_timeout_ms".to_string(), Value::from(999));
    assert!(validate_browser_tool_args("browser_click", &args).is_err());
    args.insert("popup_timeout_ms".to_string(), Value::from(1000));
    args.insert("expect_popup".to_string(), Value::String("yes".to_string()));
    assert!(validate_browser_tool_args("browser_click", &args).is_err());
    args.insert("expect_popup".to_string(), Value::Bool(true));
    args.insert("expect_download".to_string(), Value::Bool(true));
    assert!(validate_browser_tool_args("browser_click", &args).is_err());
    args.clear();

    args.insert(
        "source_selector".to_string(),
        Value::String(".card".to_string()),
    );
    args.insert(
        "target_selector".to_string(),
        Value::String(".done".to_string()),
    );
    assert!(validate_browser_tool_args("browser_drag", &args).is_ok());
    args.insert("steps".to_string(), Value::from(24));
    args.insert("modifiers".to_string(), json!(["Shift"]));
    assert!(validate_browser_tool_args("browser_drag", &args).is_ok());
    args.insert("steps".to_string(), Value::from(0));
    assert!(validate_browser_tool_args("browser_drag", &args).is_err());
    args.insert("steps".to_string(), Value::from(24));
    args.insert("modifiers".to_string(), json!(["Hyper"]));
    assert!(validate_browser_tool_args("browser_drag", &args).is_err());
    args.insert("modifiers".to_string(), json!(["Shift"]));
    args.insert(
        "source_text".to_string(),
        Value::String("Backlog".to_string()),
    );
    assert!(validate_browser_tool_args("browser_drag", &args).is_err());
    args.clear();
    args.insert(
        "source_text".to_string(),
        Value::String("Backlog".to_string()),
    );
    args.insert("target_text".to_string(), Value::String("Done".to_string()));
    assert!(validate_browser_tool_args("browser_drag", &args).is_ok());
    args.clear();
    args.insert("start_x".to_string(), Value::from(10));
    args.insert("start_y".to_string(), Value::from(20));
    args.insert("end_x".to_string(), Value::from(300));
    args.insert("end_y".to_string(), Value::from(400));
    args.insert("wait_after_ms".to_string(), Value::from(0));
    assert!(validate_browser_tool_args("browser_drag", &args).is_ok());
    args.remove("end_y");
    assert!(validate_browser_tool_args("browser_drag", &args).is_err());
    args.insert("end_y".to_string(), Value::from(400));
    args.insert("button".to_string(), Value::String("primary".to_string()));
    assert!(validate_browser_tool_args("browser_drag", &args).is_err());
    args.insert("button".to_string(), Value::String("left".to_string()));
    args.insert("screenshot".to_string(), Value::String("false".to_string()));
    assert!(validate_browser_tool_args("browser_drag", &args).is_err());
    args.clear();

    args.insert("direction".to_string(), Value::String("right".to_string()));
    args.insert("amount".to_string(), Value::from(320));
    args.insert("wait_after_ms".to_string(), Value::from(0));
    assert!(validate_browser_tool_args("browser_scroll", &args).is_ok());
    args.insert("direction".to_string(), Value::String("left".to_string()));
    assert!(validate_browser_tool_args("browser_scroll", &args).is_ok());
    args.insert(
        "direction".to_string(),
        Value::String("sideways".to_string()),
    );
    assert!(validate_browser_tool_args("browser_scroll", &args).is_err());
    args.clear();

    args.insert(
        "selector".to_string(),
        Value::String("#country".to_string()),
    );
    args.insert("value".to_string(), Value::String("cn".to_string()));
    assert!(validate_browser_tool_args("browser_select_option", &args).is_ok());
    args.insert(
        "option_label".to_string(),
        Value::String("China".to_string()),
    );
    assert!(validate_browser_tool_args("browser_select_option", &args).is_err());
    args.clear();
    args.insert("label".to_string(), Value::String("Country".to_string()));
    args.insert("index".to_string(), Value::from(2));
    args.insert("wait_after_ms".to_string(), Value::from(0));
    assert!(validate_browser_tool_args("browser_select_option", &args).is_ok());
    args.insert("index".to_string(), Value::from(-1));
    assert!(validate_browser_tool_args("browser_select_option", &args).is_err());
    args.clear();
    args.insert(
        "selector".to_string(),
        Value::String("#country".to_string()),
    );
    assert!(validate_browser_tool_args("browser_select_option", &args).is_err());
    args.insert("label".to_string(), Value::String("Country".to_string()));
    args.insert("value".to_string(), Value::String("cn".to_string()));
    assert!(validate_browser_tool_args("browser_select_option", &args).is_err());
    args.clear();

    args.insert(
        "selector".to_string(),
        Value::String("#remember".to_string()),
    );
    assert!(validate_browser_tool_args("browser_set_checked", &args).is_ok());
    args.insert(
        "label".to_string(),
        Value::String("Remember me".to_string()),
    );
    assert!(validate_browser_tool_args("browser_set_checked", &args).is_err());
    args.clear();
    args.insert(
        "label".to_string(),
        Value::String("Remember me".to_string()),
    );
    args.insert("checked".to_string(), Value::Bool(false));
    args.insert("wait_after_ms".to_string(), Value::from(0));
    assert!(validate_browser_tool_args("browser_set_checked", &args).is_ok());
    args.insert("checked".to_string(), Value::String("false".to_string()));
    assert!(validate_browser_tool_args("browser_set_checked", &args).is_err());
    args.insert("checked".to_string(), Value::Bool(true));
    args.insert("wait_after_ms".to_string(), Value::from(5001));
    assert!(validate_browser_tool_args("browser_set_checked", &args).is_err());
    args.clear();

    args.insert(
        "selector".to_string(),
        Value::String("input[type=file]".to_string()),
    );
    args.insert(
        "path".to_string(),
        Value::String("/tmp/avatar.png".to_string()),
    );
    assert!(validate_browser_tool_args("browser_upload_file", &args).is_ok());
    args.insert(
        "paths".to_string(),
        Value::Array(vec![Value::String("/tmp/other.png".to_string())]),
    );
    assert!(validate_browser_tool_args("browser_upload_file", &args).is_err());
    args.clear();
    args.insert("label".to_string(), Value::String("Avatar".to_string()));
    args.insert(
        "paths".to_string(),
        Value::Array(vec![
            Value::String("/tmp/a.png".to_string()),
            Value::String("/tmp/b.png".to_string()),
        ]),
    );
    args.insert("wait_after_ms".to_string(), Value::from(0));
    assert!(validate_browser_tool_args("browser_upload_file", &args).is_ok());
    args.insert(
        "paths".to_string(),
        Value::Array(vec![
            Value::String("/tmp/a.png".to_string()),
            Value::from(3),
        ]),
    );
    assert!(validate_browser_tool_args("browser_upload_file", &args).is_err());
    args.insert(
        "paths".to_string(),
        Value::Array(vec![Value::String("/tmp/a.png".to_string())]),
    );
    args.insert("wait_after_ms".to_string(), Value::from(5001));
    assert!(validate_browser_tool_args("browser_upload_file", &args).is_err());
    args.clear();

    args.insert("text".to_string(), Value::String("Saved".to_string()));
    assert!(validate_browser_tool_args("browser_assert", &args).is_ok());
    args.insert("selector".to_string(), Value::String("#toast".to_string()));
    assert!(validate_browser_tool_args("browser_assert", &args).is_err());
    args.clear();
    assert!(validate_browser_tool_args("browser_assert", &args).is_err());

    args.insert(
        "url_contains".to_string(),
        Value::String("/done".to_string()),
    );
    args.insert("match_mode".to_string(), Value::String("exact".to_string()));
    assert!(validate_browser_tool_args("browser_assert", &args).is_ok());
    args.insert("match_mode".to_string(), Value::String("regex".to_string()));
    assert!(validate_browser_tool_args("browser_assert", &args).is_err());

    args.clear();
    args.insert("selector".to_string(), Value::String("#toast".to_string()));
    args.insert("timeout_ms".to_string(), Value::from(1000));
    args.insert("interval_ms".to_string(), Value::from(100));
    assert!(validate_browser_tool_args("browser_wait_for", &args).is_ok());
    args.insert("timeout_ms".to_string(), Value::from(60_001));
    assert!(validate_browser_tool_args("browser_wait_for", &args).is_err());
    args.insert("timeout_ms".to_string(), Value::from(1000));
    args.insert("interval_ms".to_string(), Value::from(99));
    assert!(validate_browser_tool_args("browser_wait_for", &args).is_err());
}

#[test]
fn browser_tool_validation_rejects_string_booleans() {
    assert_browser_boolean_arg_is_strict(
        "browser_open_page",
        json!({"url": "https://example.com"}),
        "screenshot",
    );
    assert_browser_boolean_arg_is_strict(
        "browser_open_page",
        json!({"url": "https://example.com"}),
        "full_page",
    );
    assert_browser_boolean_arg_is_strict("browser_inspect", json!({}), "screenshot");
    assert_browser_boolean_arg_is_strict("browser_inspect", json!({}), "full_page");
    assert_browser_boolean_arg_is_strict("browser_console_logs", json!({}), "clear");
    assert_browser_boolean_arg_is_strict("browser_network_events", json!({}), "clear");
    assert_browser_boolean_arg_is_strict(
        "browser_dom_snapshot",
        json!({"selector": "button"}),
        "include_styles",
    );
    assert_browser_boolean_arg_is_strict(
        "browser_dom_snapshot",
        json!({"selector": "button"}),
        "include_html",
    );
    assert_browser_boolean_arg_is_strict(
        "browser_dom_snapshot",
        json!({"selector": "button"}),
        "include_children",
    );
    assert_browser_boolean_arg_is_strict(
        "browser_storage_state",
        json!({}),
        "include_local_storage",
    );
    assert_browser_boolean_arg_is_strict(
        "browser_storage_state",
        json!({}),
        "include_session_storage",
    );
    assert_browser_boolean_arg_is_strict("browser_storage_state", json!({}), "include_cookies");
    assert_browser_boolean_arg_is_strict("browser_storage_state", json!({}), "include_values");
    assert_browser_boolean_arg_is_strict(
        "browser_accessibility_snapshot",
        json!({}),
        "include_hidden",
    );
    assert_browser_boolean_arg_is_strict(
        "browser_accessibility_snapshot",
        json!({}),
        "include_bounds",
    );
    assert_browser_boolean_arg_is_strict(
        "browser_accessibility_snapshot",
        json!({}),
        "include_selectors",
    );
    assert_browser_boolean_arg_is_strict(
        "browser_accessibility_snapshot",
        json!({}),
        "include_values",
    );
    assert_browser_boolean_arg_is_strict("browser_switch_page", json!({"index": 1}), "screenshot");
    assert_browser_boolean_arg_is_strict("browser_switch_page", json!({"index": 1}), "full_page");
    assert_browser_boolean_arg_is_strict("browser_screenshot", json!({}), "full_page");
    assert_browser_boolean_arg_is_strict(
        "browser_hover",
        json!({"selector": "button"}),
        "screenshot",
    );
    assert_browser_boolean_arg_is_strict("browser_click", json!({"text": "Delete"}), "screenshot");
    assert_browser_boolean_arg_is_strict(
        "browser_drag",
        json!({"source_selector": ".card", "target_selector": ".done"}),
        "screenshot",
    );
    assert_browser_boolean_arg_is_strict(
        "browser_type_text",
        json!({"selector": "input", "text": "Nola"}),
        "clear_first",
    );
    assert_browser_boolean_arg_is_strict(
        "browser_type_text",
        json!({"selector": "input", "text": "Nola"}),
        "press_enter",
    );
    assert_browser_boolean_arg_is_strict(
        "browser_type_text",
        json!({"selector": "input", "text": "Nola"}),
        "screenshot",
    );
    assert_browser_boolean_arg_is_strict(
        "browser_select_option",
        json!({"selector": "select", "value": "cn"}),
        "screenshot",
    );
    assert_browser_boolean_arg_is_strict(
        "browser_set_checked",
        json!({"label": "Remember me"}),
        "screenshot",
    );
    assert_browser_boolean_arg_is_strict(
        "browser_upload_file",
        json!({"selector": "input[type=file]", "path": "/tmp/avatar.png"}),
        "screenshot",
    );
    assert_browser_boolean_arg_is_strict(
        "browser_press_key",
        json!({"key": "Enter"}),
        "screenshot",
    );
    assert_browser_boolean_arg_is_strict("browser_scroll", json!({}), "screenshot");
    assert_browser_boolean_arg_is_strict("browser_assert", json!({"text": "Saved"}), "exists");
    assert_browser_boolean_arg_is_strict("browser_assert", json!({"text": "Saved"}), "screenshot");
    assert_browser_boolean_arg_is_strict(
        "browser_wait_for",
        json!({"selector": "#toast"}),
        "exists",
    );
    assert_browser_boolean_arg_is_strict(
        "browser_wait_for",
        json!({"selector": "#toast"}),
        "screenshot",
    );
}

#[test]
fn managed_browser_rust_adapter_end_to_end_when_runtime_is_available() {
    let _lock = test_lock();
    let (Ok(node), Ok(bridge)) = (
        std::env::var("NOLA_PLAYWRIGHT_NODE"),
        std::env::var("NOLA_PLAYWRIGHT_BRIDGE"),
    ) else {
        // The runtime is bundled by the macOS product build. Source-only CI
        // jobs without that artifact still exercise schema and routing tests.
        return;
    };
    assert!(Path::new(&node).is_file(), "bundled Node is missing");
    assert!(Path::new(&bridge).is_file(), "Playwright bridge is missing");

    let state = temp_root("managed-browser-rust-e2e");
    let permission_scope = SessionPermissionScope::default();
    permission_scope.bind_owner_session(&format!("rust-e2e-{}", uuid::Uuid::new_v4()));
    let ctx = ToolContext {
        memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
        language_services: Default::default(),
        skills: Default::default(),
        allowed_dirs: vec![state.clone()],
        policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), true, "allow".to_string(), Vec::new()),
        permission_scope,
        cancellation: Default::default(),
        plan_state: Default::default(),
        output_store: crate::tools::ToolOutputStore::for_test(),
        browser: BrowserRuntime::new(state.clone()),
    };
    let args = |extra: Value| {
        let mut value = json!({
            "target": "managed",
        });
        if let (Some(target), Some(source)) = (value.as_object_mut(), extra.as_object()) {
            target.extend(source.clone());
        }
        value.as_object().cloned().expect("browser args")
    };

    let started = run_browser_tool(
        "browser_start",
        &args(json!({"headless": true})),
        &ctx,
    );
    assert!(!started.error, "{}", started.text);
    let snapshot = run_browser_tool("browser_snapshot", &args(json!({})), &ctx);
    assert!(!snapshot.error, "{}", snapshot.text);
    assert!(snapshot.text.contains("Final URL:"));
    let screenshot = run_browser_tool(
        "browser_screenshot",
        &args(json!({"full_page": true})),
        &ctx,
    );
    assert!(!screenshot.error, "{}", screenshot.text);
    assert_eq!(screenshot.artifacts.len(), 1);
    assert_eq!(screenshot.artifacts[0].mime_type.as_deref(), Some("image/png"));
    let stopped = run_browser_tool("browser_stop", &args(json!({})), &ctx);
    assert!(!stopped.error, "{}", stopped.text);

    let _ = std::fs::remove_dir_all(state);
}

#[test]
fn update_agent_todos_tracks_active_work_and_verification_warning() {
    let _guard = test_lock();
    let plan_state = AgentPlanState::default();
    let mut args = Map::new();
    args.insert(
            "todos".to_string(),
            json!([
                {"id": "inspect", "content": "Inspect files", "status": "completed", "priority": "high", "dependencyIds": [], "approvalStatus": "not_required", "capabilityRefs": ["builtin:read_file"], "expectedArtifactRefs": [], "successCriteria": ["Relevant files identified"], "evidenceRefs": ["tool:read_file:1"], "verificationStatus": "passed", "terminationReason": "success"},
                {"id": "patch", "content": "Patch implementation", "activeForm": "Patching implementation", "status": "in_progress", "dependencyIds": ["inspect"], "approvalStatus": "approved", "capabilityRefs": ["builtin:write_file"], "expectedArtifactRefs": ["file:src/lib.rs"], "successCriteria": ["Implementation matches the contract"], "verificationStatus": "pending"},
                {"id": "test", "content": "Run tests", "status": "pending", "dependencyIds": ["patch"], "approvalStatus": "not_required", "capabilityRefs": ["builtin:bash"], "expectedArtifactRefs": ["test-result:targeted"], "successCriteria": ["Targeted tests pass"], "verificationStatus": "pending"}
            ]),
        );

    let outcome = update_agent_todos(&args, &plan_state);

    assert!(!outcome.error, "{}", outcome.text);
    assert_eq!(outcome.display, "Patching implementation");
    assert!(outcome.text.contains("1. [completed] Inspect files"));
    assert!(outcome.text.contains("priority: high"));
    assert!(outcome
        .text
        .contains("2. [in_progress] Patch implementation"));

    args.insert(
        "todos".to_string(),
        json!([
            {"id": "inspect", "content": "Inspect files", "status": "completed", "dependencyIds": [], "approvalStatus": "not_required", "capabilityRefs": ["builtin:read_file"], "expectedArtifactRefs": [], "successCriteria": ["Relevant files identified"], "evidenceRefs": ["tool:read_file:1"], "verificationStatus": "passed", "terminationReason": "success"},
            {"id": "patch", "content": "Patch implementation", "status": "completed", "dependencyIds": ["inspect"], "approvalStatus": "approved", "capabilityRefs": ["builtin:write_file"], "expectedArtifactRefs": ["file:src/lib.rs"], "successCriteria": ["Implementation matches the contract"], "evidenceRefs": ["file:src/lib.rs"], "verificationStatus": "passed", "terminationReason": "success"},
            {"id": "document", "content": "Document result", "status": "completed", "dependencyIds": ["patch"], "approvalStatus": "not_required", "capabilityRefs": [], "expectedArtifactRefs": [], "successCriteria": ["Result is documented"], "verificationStatus": "not_required", "terminationReason": "verification_not_applicable"},
            {"id": "obsolete", "content": "Drop obsolete path", "status": "cancelled", "priority": "low", "dependencyIds": [], "approvalStatus": "rejected", "capabilityRefs": [], "expectedArtifactRefs": [], "successCriteria": ["Obsolete path is removed if still applicable"], "verificationStatus": "pending", "terminationReason": "cancelled"}
        ]),
    );
    let outcome = update_agent_todos(&args, &plan_state);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("[cancelled] Drop obsolete path"));
    assert!(outcome.text.contains("priority: low"));

    args.insert(
        "todos".to_string(),
        json!([
            {"id": "inspect", "content": "Inspect files", "status": "completed", "dependencyIds": [], "approvalStatus": "not_required", "capabilityRefs": ["builtin:read_file"], "expectedArtifactRefs": [], "successCriteria": ["Relevant files identified"], "evidenceRefs": ["tool:read_file:1"], "verificationStatus": "passed", "terminationReason": "success"},
            {"id": "patch", "content": "Patch implementation", "status": "completed", "dependencyIds": ["inspect"], "approvalStatus": "approved", "capabilityRefs": ["builtin:write_file"], "expectedArtifactRefs": ["file:src/lib.rs"], "successCriteria": ["Implementation matches the contract"], "evidenceRefs": ["file:src/lib.rs"], "verificationStatus": "passed", "terminationReason": "success"},
            {"id": "document", "content": "Document result", "status": "completed", "dependencyIds": ["patch"], "approvalStatus": "not_required", "capabilityRefs": [], "expectedArtifactRefs": [], "successCriteria": ["Result is documented"], "verificationStatus": "not_required", "terminationReason": "verification_not_applicable"}
        ]),
    );
    let outcome = update_agent_todos(&args, &plan_state);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome
        .text
        .contains("without an explicit verification/test item"));
}

#[test]
fn update_agent_todos_isolates_independent_agent_runs() {
    let first_state = AgentPlanState::default();
    let second_state = AgentPlanState::default();
    let first_args = json!({
        "todos": [{"id": "first", "content": "First run work", "status": "in_progress", "dependencyIds": [], "approvalStatus": "not_required", "capabilityRefs": [], "expectedArtifactRefs": [], "successCriteria": ["First run completes"], "verificationStatus": "pending"}]
    });
    let second_args = json!({
        "todos": [{"id": "second", "content": "Second run work", "status": "in_progress", "dependencyIds": [], "approvalStatus": "not_required", "capabilityRefs": [], "expectedArtifactRefs": [], "successCriteria": ["Second run completes"], "verificationStatus": "pending"}]
    });

    let first = update_agent_todos(first_args.as_object().unwrap(), &first_state);
    let second = update_agent_todos(second_args.as_object().unwrap(), &second_state);
    let first_again = update_agent_todos(second_args.as_object().unwrap(), &first_state);

    assert!(!first.error, "{}", first.text);
    assert!(!second.error, "{}", second.text);
    assert!(!second.text.contains("First run work"));
    assert!(first_again.text.contains("First run work"));
}

#[test]
fn completed_agent_todos_reopen_when_validation_is_pending() {
    let plan_state = AgentPlanState::default();
    let args = json!({
        "todos": [
            {"id": "inspect", "content": "Inspect files", "status": "completed", "dependencyIds": [], "approvalStatus": "not_required", "capabilityRefs": ["builtin:read_file"], "expectedArtifactRefs": [], "successCriteria": ["Relevant files identified"], "evidenceRefs": ["tool:read_file:1"], "verificationStatus": "passed", "terminationReason": "success"},
            {"id": "patch", "content": "Patch implementation", "status": "completed", "dependencyIds": ["inspect"], "approvalStatus": "approved", "capabilityRefs": ["builtin:write_file"], "expectedArtifactRefs": ["file:src/lib.rs"], "successCriteria": ["Implementation matches the contract"], "evidenceRefs": ["file:src/lib.rs"], "verificationStatus": "passed", "terminationReason": "success"}
        ]
    });
    let outcome = update_agent_todos(args.as_object().unwrap(), &plan_state);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(plan_state.ensure_validation_pending().unwrap());
    let snapshot = plan_state.snapshot().unwrap();
    assert_eq!(snapshot.completed, 2);
    assert_eq!(snapshot.total, 3);
    assert_eq!(snapshot.current.as_deref(), Some("Verifying the result"));
    assert_eq!(snapshot.steps.last().unwrap().status, "in_progress");
    assert!(!plan_state.ensure_validation_pending().unwrap());
}

#[test]
fn completed_todo_cannot_omit_one_of_its_declared_artifacts() {
    let plan_state = AgentPlanState::default();
    let args = json!({
        "todos": [{
            "id": "deliver",
            "content": "Create all declared outputs",
            "status": "completed",
            "dependencyIds": [],
            "approvalStatus": "not_required",
            "capabilityRefs": ["builtin:write_file"],
            "expectedArtifactRefs": ["file:primary", "file:companion"],
            "artifactRefs": ["file:primary"],
            "successCriteria": ["Both outputs exist"],
            "evidenceRefs": ["file:primary"],
            "verificationStatus": "passed",
            "terminationReason": "success"
        }]
    });

    let rejected = update_agent_todos(args.as_object().unwrap(), &plan_state);

    assert!(rejected.error);
    assert!(rejected
        .text
        .contains("evidence for every expectedArtifactRef"));
}

#[test]
fn plan_approval_is_an_explicit_user_state_transition() {
    let state = AgentPlanState::default();
    let args = json!({
        "todos": [
            {"id": "inspect", "content": "Inspect", "status": "pending", "dependencyIds": [], "approvalStatus": "pending", "capabilityRefs": ["builtin:read_file"], "expectedArtifactRefs": [], "successCriteria": ["Files are identified"], "verificationStatus": "pending"},
            {"id": "write", "content": "Write", "status": "pending", "dependencyIds": ["inspect"], "approvalStatus": "pending", "capabilityRefs": ["builtin:write_file"], "expectedArtifactRefs": ["file:result"], "successCriteria": ["Result is written"], "verificationStatus": "pending"}
        ]
    });
    state
        .restore_from_args(args.as_object().unwrap())
        .expect("restore plan");

    assert_eq!(
        state
            .set_plan_approval(&["inspect".to_string()], true)
            .expect("approve selected step"),
        1
    );
    assert_eq!(
        state
            .set_plan_approval(&["write".to_string()], false)
            .expect("reject selected step"),
        1
    );
    let snapshot = state.snapshot().unwrap();
    assert_eq!(snapshot.steps[0].approval_status, "approved");
    assert_eq!(snapshot.steps[1].approval_status, "rejected");
    assert_eq!(snapshot.steps[1].status, "cancelled");
    assert!(state
        .set_plan_approval(&["missing".to_string()], true)
        .unwrap_err()
        .contains("unknown plan step"));
    assert!(state
        .set_plan_approval(&["inspect".to_string()], true)
        .unwrap_err()
        .contains("not awaiting approval"));
}

#[test]
fn changed_plan_semantics_cannot_reuse_old_completion_evidence() {
    let state = AgentPlanState::default();
    let completed = json!({
        "todos": [{"id": "build", "content": "Build A", "status": "completed", "dependencyIds": [], "approvalStatus": "not_required", "capabilityRefs": ["builtin:write_file"], "expectedArtifactRefs": ["file:a"], "successCriteria": ["A exists"], "evidenceRefs": ["file:a"], "verificationStatus": "passed", "terminationReason": "success"}]
    });
    let initial = update_agent_todos(completed.as_object().unwrap(), &state);
    assert!(!initial.error, "{}", initial.text);

    let stale = json!({
        "todos": [{"id": "build", "content": "Build B", "status": "completed", "dependencyIds": [], "approvalStatus": "not_required", "capabilityRefs": ["builtin:write_file"], "expectedArtifactRefs": ["file:b"], "successCriteria": ["B exists"], "evidenceRefs": ["file:b"], "verificationStatus": "passed", "terminationReason": "success"}]
    });
    let rejected = update_agent_todos(stale.as_object().unwrap(), &state);
    assert!(rejected.error);
    assert!(rejected.text.contains("changed semantics"));

    let replanned = json!({
        "todos": [{"id": "build", "content": "Build B", "status": "pending", "dependencyIds": [], "approvalStatus": "not_required", "capabilityRefs": ["builtin:write_file"], "expectedArtifactRefs": ["file:b"], "successCriteria": ["B exists"], "evidenceRefs": [], "verificationStatus": "pending"}]
    });
    let accepted = update_agent_todos(replanned.as_object().unwrap(), &state);
    assert!(!accepted.error, "{}", accepted.text);
}

#[test]
fn active_plan_stage_cannot_drop_attempt_or_committed_runtime_references() {
    let state = AgentPlanState::default();
    let initial = json!({
        "todos": [{
            "id": "write",
            "content": "Write the durable result",
            "activeForm": "Writing the durable result",
            "status": "in_progress",
            "dependencyIds": [],
            "approvalStatus": "not_required",
            "capabilityRefs": ["builtin:write_file"],
            "expectedArtifactRefs": ["file:result.md"],
            "artifactRefs": ["artifact:result-draft"],
            "workStepTransactionRefs": ["work-step:write:2"],
            "successCriteria": ["The result is persisted"],
            "verificationStatus": "pending",
            "attempt": 2,
            "executorRef": "host:cli",
            "resumeKey": "checkpoint:write:prepared"
        }]
    });
    let accepted = update_agent_todos(initial.as_object().unwrap(), &state);
    assert!(!accepted.error, "{}", accepted.text);

    let omitted = json!({"todos": []});
    let rejected_omission = update_agent_todos(omitted.as_object().unwrap(), &state);
    assert!(rejected_omission.error);
    assert!(rejected_omission.text.contains("cannot discard persisted"));
    let retained = state.current_todos().unwrap();
    assert_eq!(retained.len(), 1);
    assert_eq!(retained[0].artifact_refs, vec!["artifact:result-draft"]);
    assert_eq!(retained[0].work_step_transaction_refs, vec!["work-step:write:2"]);

    let decreased_attempt = json!({
        "todos": [{
            "id": "write",
            "content": "Write the durable result",
            "activeForm": "Writing the durable result",
            "status": "in_progress",
            "dependencyIds": [],
            "approvalStatus": "not_required",
            "capabilityRefs": ["builtin:write_file"],
            "expectedArtifactRefs": ["file:result.md"],
            "artifactRefs": ["artifact:result-draft"],
            "workStepTransactionRefs": ["work-step:write:2"],
            "successCriteria": ["The result is persisted"],
            "verificationStatus": "pending",
            "attempt": 1,
            "executorRef": "host:cli",
            "resumeKey": "checkpoint:write:prepared"
        }]
    });
    let rejected_attempt = update_agent_todos(decreased_attempt.as_object().unwrap(), &state);
    assert!(rejected_attempt.error);
    assert!(rejected_attempt.text.contains("attempt"));

    let dropped_references = json!({
        "todos": [{
            "id": "write",
            "content": "Write the durable result",
            "activeForm": "Writing the durable result",
            "status": "in_progress",
            "dependencyIds": [],
            "approvalStatus": "not_required",
            "capabilityRefs": ["builtin:write_file"],
            "expectedArtifactRefs": ["file:result.md"],
            "artifactRefs": [],
            "workStepTransactionRefs": [],
            "successCriteria": ["The result is persisted"],
            "verificationStatus": "pending",
            "attempt": 2,
            "executorRef": "host:cli",
            "resumeKey": "checkpoint:write:prepared"
        }]
    });
    let rejected_references = update_agent_todos(dropped_references.as_object().unwrap(), &state);
    assert!(rejected_references.error);
    assert!(rejected_references
        .text
        .to_ascii_lowercase()
        .contains("artifact"));
}

#[test]
fn agent_task_status_requires_evidence_for_validated_status() {
    let state = AgentPlanState::default();
    state.record_mutation();
    let missing_evidence = agent_task_status(
        &json!({
            "status": "validated",
            "summary": "done",
            "evidence": ["model claims cargo test passed"]
        })
        .as_object()
        .unwrap()
        .clone(),
        &state,
    );
    assert!(missing_evidence.error);
    assert!(missing_evidence
        .text
        .contains("successful validation command after the latest edit"));

    state.record_validation_evidence("cargo test --quiet (exit=0)".to_string());

    let outcome = agent_task_status(
        &json!({
            "status": "validated",
            "summary": "change verified",
            "evidence": ["cargo test --quiet exited 0"]
        })
        .as_object()
        .unwrap()
        .clone(),
        &state,
    );

    assert!(!outcome.error, "{}", outcome.text);
    assert_eq!(outcome.display, "task status: validated");
    assert!(outcome.text.contains("agent_task_status: validated"));
    assert!(outcome.text.contains("cargo test --quiet (exit=0)"));
    assert!(!outcome.text.contains("cargo test --quiet exited 0"));

    state.record_mutation();
    let invalidated = agent_task_status(
        &json!({
            "status": "validated",
            "summary": "stale validation must not survive another edit",
            "evidence": ["cargo test --quiet (exit=0)"]
        })
        .as_object()
        .unwrap()
        .clone(),
        &state,
    );
    assert!(invalidated.error);
}

#[test]
fn agent_task_status_accepts_concrete_read_only_audit_evidence() {
    let state = AgentPlanState::default();
    let outcome = agent_task_status(
        &json!({
            "status": "validated",
            "summary": "read-only audit complete",
            "evidence": ["diff_review inspected the complete staged and unstaged change set"]
        })
        .as_object()
        .unwrap()
        .clone(),
        &state,
    );

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome
        .text
        .contains("diff_review inspected the complete staged and unstaged change set"));
}

#[test]
fn validation_evidence_survives_the_real_tool_execution_sequence() {
    let _environment = crate::store::test_env_lock();
    let root = temp_root("validation-evidence-sequence");
    fs::create_dir_all(&root).unwrap();
    std::env::set_var("NOLA_DB_PATH", root.join("nola.db"));
    let permission_scope = SessionPermissionScope::default();
    permission_scope.bind_owner_session("validation-evidence-sequence");
    // This execution-evidence fixture explicitly grants the conversation's
    // Nola policy; legacy auto_approve is not authorization for a process.
    permission_scope.configure_profile(crate::permissions::PermissionProfile::AllowAll);
    let ctx = ToolContext {
        memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
        language_services: Default::default(),
        skills: Default::default(),
        allowed_dirs: vec![root.clone()],
        policy: crate::tools::ToolExecutionPolicy::new(vec!["python3".to_string()], true, "allow".to_string(), Vec::new()),
        permission_scope,
        cancellation: Default::default(),
        plan_state: Default::default(),
        output_store: crate::tools::ToolOutputStore::for_test(),
        browser: BrowserRuntime::new(root.join("browser-runtime")),
    };

    let write = execute_tool(
        &ToolRequest {
            id: "write-validator".to_string(),
            name: "write_file".to_string(),
            args: object_args(json!({
                "path": root.join("validate_receipt.py").display().to_string(),
                "content": "print('8/8 checks passed')\n"
            })),
        },
        &ctx,
    );
    assert!(!write.error, "{}", write.text);

    let validation = execute_tool(
        &ToolRequest {
            id: "run-validator".to_string(),
            name: "bash".to_string(),
            args: object_args(json!({
                "command": "python3 validate_receipt.py",
                "cwd": root.display().to_string(),
                "operation_kind": "validate",
                "validation_targets": [root.join("validate_receipt.py").display().to_string()]
            })),
        },
        &ctx,
    );
    assert!(!validation.error, "{}", validation.text);

    let status = execute_tool(
        &ToolRequest {
            id: "report-validation".to_string(),
            name: "agent_task_status".to_string(),
            args: object_args(json!({
                "status": "validated",
                "summary": "receipt verified"
            })),
        },
        &ctx,
    );
    assert!(!status.error, "{}", status.text);
    assert!(status.text.contains("python3 validate_receipt.py"));
    std::env::remove_var("NOLA_DB_PATH");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn agent_task_status_requires_next_action_for_terminal_non_validated_status() {
    let state = AgentPlanState::default();
    let missing_next = agent_task_status(
        &json!({
            "status": "blocked",
            "summary": "waiting for secret"
        })
        .as_object()
        .unwrap()
        .clone(),
        &state,
    );
    assert!(missing_next.error);
    assert!(missing_next
        .text
        .contains("next_action is required for blocked, failed, or needs_user status"));

    let progress = agent_task_status(
        &json!({
            "status": "in_progress",
            "summary": "continuing investigation"
        })
        .as_object()
        .unwrap()
        .clone(),
        &state,
    );
    assert!(!progress.error, "{}", progress.text);
    assert!(progress.text.contains("agent_task_status: in_progress"));

    let outcome = agent_task_status(
        &json!({
            "status": "needs_user",
            "summary": "need target environment",
            "next_action": "ask user for deployment target"
        })
        .as_object()
        .unwrap()
        .clone(),
        &state,
    );
    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("agent_task_status: needs_user"));
    assert!(outcome
        .text
        .contains("agent_task_next_action: ask user for deployment target"));
}

#[test]
fn goal_complete_requires_status_and_summary() {
    let missing_status = goal_complete(
        &json!({
            "summary": "work done"
        })
        .as_object()
        .unwrap()
        .clone(),
    );
    assert!(missing_status.error);
    assert!(missing_status.text.contains(
        "status must be one of: completed, continue, incomplete, blocked, needs_user, failed"
    ));

    let missing_summary = goal_complete(
        &json!({
            "status": "completed"
        })
        .as_object()
        .unwrap()
        .clone(),
    );
    assert!(missing_summary.error);
    assert!(missing_summary.text.contains("summary is required"));

    let missing_evidence = goal_complete(
        &json!({
            "status": "completed",
            "summary": "work done"
        })
        .as_object()
        .unwrap()
        .clone(),
    );
    assert!(missing_evidence.error);
    assert!(missing_evidence
        .text
        .contains("completed status requires at least one concrete evidence item"));

    let needs_user = goal_complete(
        &json!({
            "status": "needs_user",
            "summary": "need deployment target",
            "next_action": "ask user for target"
        })
        .as_object()
        .unwrap()
        .clone(),
    );
    assert!(!needs_user.error, "{}", needs_user.text);
}

#[test]
fn goal_complete_emits_structured_output() {
    let outcome = goal_complete(
        &json!({
            "status": "completed",
            "summary": "all tasks done",
            "claims": [{"requirement_id":"r", "requirement_revision":1, "summary":"tests passed", "evidence_refs":["tool:tests"],"execution_evidence_refs":[],"task_replacements":[]}],
            "evidence": ["pytest passed", "build ok"],
            "blockers": [],
            "next_action": "none"
        })
        .as_object()
        .unwrap()
        .clone(),
    );
    assert!(!outcome.error, "{}", outcome.text);
    assert_eq!(outcome.display, "goal status: completed");
    assert!(outcome.text.contains("goal_complete: completed"));
    assert!(outcome.text.contains("goal_summary: all tasks done"));
    assert!(outcome.text.contains("goal_evidence:"));
    assert!(outcome.text.contains("1. pytest passed"));
    assert!(outcome.text.contains("goal_next_action: none"));
}

#[test]
fn set_thinking_mode_emits_agent_control_metadata() {
    let outcome = set_thinking_mode(
        &json!({
            "enabled": true,
            "reason": "Need deeper root-cause analysis"
        })
        .as_object()
        .unwrap()
        .clone(),
    );

    assert!(!outcome.error, "{}", outcome.text);
    assert_eq!(outcome.display, "thinking enabled");
    assert!(outcome.text.contains("agent_thinking_enabled: true"));
    assert!(outcome.text.contains("agent_thinking_mode: enabled"));

    let missing_reason = set_thinking_mode(
        &json!({
            "enabled": false
        })
        .as_object()
        .unwrap()
        .clone(),
    );
    assert!(missing_reason.error);
    assert!(missing_reason.text.contains("reason is required"));
}

#[test]
fn update_agent_todos_rejects_multiple_active_items() {
    let _guard = test_lock();
    let plan_state = AgentPlanState::default();
    let mut args = Map::new();
    args.insert(
        "todos".to_string(),
        json!([
            {"id": "first", "content": "First", "status": "in_progress", "dependencyIds": [], "approvalStatus": "not_required", "capabilityRefs": [], "expectedArtifactRefs": [], "successCriteria": ["First completes"], "verificationStatus": "pending"},
            {"id": "second", "content": "Second", "status": "in_progress", "dependencyIds": [], "approvalStatus": "not_required", "capabilityRefs": [], "expectedArtifactRefs": [], "successCriteria": ["Second completes"], "verificationStatus": "pending"}
        ]),
    );

    let outcome = update_agent_todos(&args, &plan_state);

    assert!(outcome.error);
    assert!(outcome.text.contains("only one todo may be in_progress"));

    args.insert(
        "todos".to_string(),
        json!([{"id": "first", "content": "First", "status": "pending", "priority": "urgent", "dependencyIds": [], "approvalStatus": "not_required", "capabilityRefs": [], "expectedArtifactRefs": [], "successCriteria": ["First completes"], "verificationStatus": "pending"}]),
    );
    let outcome = update_agent_todos(&args, &plan_state);

    assert!(outcome.error);
    assert!(outcome
        .text
        .contains("priority must be high, medium, or low"));
}

#[test]
fn propose_agent_plan_formats_valid_plan() {
    let mut args = Map::new();
    args.insert(
        "title".to_string(),
        Value::String("Computer Use Parity".to_string()),
    );
    args.insert(
        "summary".to_string(),
        Value::String("Close the largest CLI and Web computer-use gaps.".to_string()),
    );
    args.insert(
        "steps".to_string(),
        json!([
            "Inspect host bridge behavior",
            "Patch CLI and Web affordances",
            "Run targeted validation"
        ]),
    );
    args.insert(
        "risks".to_string(),
        json!(["Desktop permissions may block host actions"]),
    );
    args.insert(
        "verification".to_string(),
        Value::String("Run CLI tests and review host-tool smoke coverage.".to_string()),
    );

    let outcome = propose_agent_plan(&args);

    assert!(!outcome.error, "{}", outcome.text);
    assert_eq!(outcome.display, "Plan published");
    assert!(outcome.text.contains("Publication is not approval"));
    assert!(outcome.text.contains("# Computer Use Parity"));
    assert!(outcome.text.contains("steps:"));
    assert!(outcome.text.contains("1. Inspect host bridge behavior"));
    assert!(outcome.text.contains("risks:"));
    assert!(outcome.text.contains("verification: Run CLI tests"));
}

#[test]
fn agent_plan_file_previews_writes_reads_and_guards_stale_plans() {
    let root = temp_root("agent-plan-file");
    let ctx = ToolContext {
        memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
        language_services: Default::default(),
        skills: Default::default(),
        allowed_dirs: vec![root.clone()],
        policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), true, "allow".to_string(), Vec::new()),
        permission_scope: Default::default(),
        cancellation: Default::default(),
        plan_state: Default::default(),
        output_store: crate::tools::ToolOutputStore::for_test(),
        browser: BrowserRuntime::new(temp_root("browser-runtime")),
    };
    let content = "# Fix parser\n\nsteps:\n1. Inspect parser\n2. Patch parser\n3. Run tests\n";
    let mut args = Map::new();
    args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );
    args.insert("action".to_string(), Value::String("write".to_string()));
    args.insert("slug".to_string(), Value::String("fix-parser".to_string()));
    args.insert("content".to_string(), Value::String(content.to_string()));

    let preview = agent_plan_file(&args, &ctx);

    assert!(!preview.error, "{}", preview.text);
    assert!(preview.text.contains("dry_run: true"));
    assert!(preview.text.contains("## Diff"));
    let plan = root.join(".nola").join("plans").join("fix-parser.md");
    assert!(!plan.exists());

    args.insert("dry_run".to_string(), Value::Bool(false));
    let written = agent_plan_file(&args, &ctx);

    assert!(!written.error, "{}", written.text);
    assert_eq!(fs::read_to_string(&plan).unwrap(), content);
    assert!(written.text.contains("path: .nola/plans/fix-parser.md"));
    let digest = file_sha256_hex(&plan).unwrap();

    let modified = fs::metadata(&plan).unwrap().modified().unwrap();
    let unchanged = agent_plan_file(&args, &ctx);
    assert!(!unchanged.error, "{}", unchanged.text);
    assert_eq!(unchanged.data.as_ref().unwrap()["changed"], false);
    assert_eq!(unchanged.data.as_ref().unwrap()["sha256"], digest);
    assert_eq!(fs::metadata(&plan).unwrap().modified().unwrap(), modified);

    let mut read_args = Map::new();
    read_args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );
    read_args.insert("action".to_string(), Value::String("read".to_string()));
    read_args.insert(
        "slug".to_string(),
        Value::String("fix-parser.md".to_string()),
    );
    let read = agent_plan_file(&read_args, &ctx);

    assert!(!read.error, "{}", read.text);
    assert!(!read.text.contains("sha256:"));
    assert!(read.text.contains("Inspect parser"));

    let mut list_args = Map::new();
    list_args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );
    let listing = agent_plan_file(&list_args, &ctx);

    assert!(!listing.error, "{}", listing.text);
    assert!(listing
        .text
        .contains("fix-parser path=.nola/plans/fix-parser.md sha256="));

    fs::write(&plan, "# User edit\n").unwrap();
    args.insert(
        "expected_sha256".to_string(),
        Value::String(digest.to_string()),
    );
    let stale = agent_plan_file(&args, &ctx);

    assert!(stale.error);
    assert!(stale.text.contains("expected_sha256 mismatch"));
    assert_eq!(fs::read_to_string(&plan).unwrap(), "# User edit\n");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn propose_agent_plan_rejects_invalid_plan() {
    let mut missing_steps = Map::new();
    missing_steps.insert("summary".to_string(), Value::String("Do work".to_string()));

    let outcome = propose_agent_plan(&missing_steps);

    assert!(outcome.error);
    assert!(outcome
        .text
        .contains("steps must contain at least one item"));

    let mut too_many_steps = Map::new();
    too_many_steps.insert("summary".to_string(), Value::String("Do work".to_string()));
    too_many_steps.insert(
        "steps".to_string(),
        Value::Array(
            (0..=MAX_PLAN_STEPS)
                .map(|index| Value::String(format!("Step {index}")))
                .collect(),
        ),
    );

    let outcome = propose_agent_plan(&too_many_steps);

    assert!(outcome.error);
    assert!(outcome.text.contains("steps must contain at most 12 items"));
}

#[test]
fn computer_tool_validation_rejects_unsafe_args() {
    let mut args = Map::new();
    assert!(validate_computer_tool_args("computer_status", &args).is_ok());
    assert!(validate_computer_tool_args("computer_observe", &args).is_ok());
    args.insert("include_screenshot".to_string(), Value::Bool(false));
    args.insert("include_accessibility".to_string(), Value::Bool(true));
    args.insert("include_text".to_string(), Value::Bool(true));
    args.insert("max_elements".to_string(), Value::from(80));
    args.insert("max_text_results".to_string(), Value::from(20));
    args.insert("text_min_confidence".to_string(), Value::from(0.5));
    assert!(validate_computer_tool_args("computer_observe", &args).is_ok());
    args.insert(
        "include_screenshot".to_string(),
        Value::String("yes".to_string()),
    );
    assert!(validate_computer_tool_args("computer_observe", &args).is_err());
    args.insert("include_screenshot".to_string(), Value::Bool(false));
    args.insert("include_text".to_string(), Value::String("yes".to_string()));
    assert!(validate_computer_tool_args("computer_observe", &args).is_err());
    args.insert("include_text".to_string(), Value::Bool(true));
    args.insert("max_elements".to_string(), Value::from(0));
    assert!(validate_computer_tool_args("computer_observe", &args).is_err());
    args.insert("max_elements".to_string(), Value::from(80));
    args.insert("max_text_results".to_string(), Value::from(0));
    assert!(validate_computer_tool_args("computer_observe", &args).is_err());
    args.insert("max_text_results".to_string(), Value::from(20));
    args.insert("text_min_confidence".to_string(), Value::from(1.1));
    assert!(validate_computer_tool_args("computer_observe", &args).is_err());
    args.clear();
    assert!(validate_computer_tool_args("computer_list_displays", &args).is_ok());

    args.insert("x".to_string(), Value::from(10));
    args.insert("y".to_string(), Value::from(20));
    args.insert("width".to_string(), Value::from(300));
    assert!(validate_computer_tool_args("computer_screenshot", &args).is_err());
    args.insert("height".to_string(), Value::from(200));
    assert!(validate_computer_tool_args("computer_screenshot", &args).is_ok());
    args.insert("x".to_string(), Value::from(-1200));
    args.insert("y".to_string(), Value::from(-100));
    assert!(validate_computer_tool_args("computer_screenshot", &args).is_ok());
    args.clear();
    args.insert("display_index".to_string(), Value::from(1));
    assert!(validate_computer_tool_args("computer_screenshot", &args).is_ok());
    args.insert("display_id".to_string(), Value::from(2));
    assert!(validate_computer_tool_args("computer_screenshot", &args).is_err());
    args.remove("display_id");
    args.insert("x".to_string(), Value::from(0));
    args.insert("y".to_string(), Value::from(0));
    args.insert("width".to_string(), Value::from(100));
    args.insert("height".to_string(), Value::from(100));
    assert!(validate_computer_tool_args("computer_screenshot", &args).is_err());
    args.clear();
    args.insert("display_index".to_string(), Value::from(0));
    assert!(validate_computer_tool_args("computer_screenshot", &args).is_err());
    args.clear();
    args.insert("display_id".to_string(), Value::from(2));
    assert!(validate_computer_tool_args("computer_screenshot", &args).is_ok());
    args.clear();

    args.insert("query".to_string(), Value::String("Open".to_string()));
    args.insert("role".to_string(), Value::String("AXButton".to_string()));
    args.insert("padding".to_string(), Value::from(8));
    assert!(validate_computer_tool_args("computer_screenshot_element", &args).is_ok());
    args.insert("padding".to_string(), Value::from(5001));
    assert!(validate_computer_tool_args("computer_screenshot_element", &args).is_err());
    args.insert("padding".to_string(), Value::from(8));
    args.insert("match_mode".to_string(), Value::String("regex".to_string()));
    assert!(validate_computer_tool_args("computer_screenshot_element", &args).is_err());
    args.clear();
    assert!(validate_computer_tool_args("computer_screenshot_element", &args).is_err());

    args.insert("title".to_string(), Value::String("Nola".to_string()));
    args.insert("padding".to_string(), Value::from(8));
    assert!(validate_computer_tool_args("computer_screenshot_window", &args).is_ok());
    args.insert("padding".to_string(), Value::from(5001));
    assert!(validate_computer_tool_args("computer_screenshot_window", &args).is_err());
    args.insert("padding".to_string(), Value::from(8));
    args.insert("match_mode".to_string(), Value::String("regex".to_string()));
    assert!(validate_computer_tool_args("computer_screenshot_window", &args).is_err());
    args.clear();
    assert!(validate_computer_tool_args("computer_screenshot_window", &args).is_err());

    assert!(validate_computer_tool_args("computer_read_text", &args).is_ok());
    args.insert("include_ocr".to_string(), Value::Bool(false));
    args.insert("max_results".to_string(), Value::from(5));
    assert!(validate_computer_tool_args("computer_read_text", &args).is_ok());
    args.insert("max_results".to_string(), Value::from(501));
    assert!(validate_computer_tool_args("computer_read_text", &args).is_err());
    args.insert("max_results".to_string(), Value::from(5));
    args.insert("min_confidence".to_string(), Value::from(1.1));
    assert!(validate_computer_tool_args("computer_read_text", &args).is_err());
    args.insert("min_confidence".to_string(), Value::from(0.25));
    args.insert("include_accessibility".to_string(), Value::Bool(false));
    assert!(validate_computer_tool_args("computer_read_text", &args).is_err());
    args.insert("include_ocr".to_string(), Value::Bool(true));
    args.insert("x".to_string(), Value::from(10));
    assert!(validate_computer_tool_args("computer_read_text", &args).is_err());
    args.clear();

    args.insert("text".to_string(), Value::String("Open".to_string()));
    assert!(validate_computer_tool_args("computer_find_text", &args).is_ok());
    args.insert("include_ocr".to_string(), Value::Bool(false));
    args.insert("max_results".to_string(), Value::from(5));
    assert!(validate_computer_tool_args("computer_find_text", &args).is_ok());
    args.insert("max_results".to_string(), Value::from(0));
    assert!(validate_computer_tool_args("computer_find_text", &args).is_err());
    args.insert("max_results".to_string(), Value::from(5));
    args.insert("min_confidence".to_string(), Value::from(1.1));
    assert!(validate_computer_tool_args("computer_find_text", &args).is_err());
    args.insert("min_confidence".to_string(), Value::from(0.25));
    args.insert("match_mode".to_string(), Value::String("regex".to_string()));
    assert!(validate_computer_tool_args("computer_find_text", &args).is_err());
    args.insert(
        "match_mode".to_string(),
        Value::String("contains".to_string()),
    );
    args.insert("include_accessibility".to_string(), Value::Bool(false));
    assert!(validate_computer_tool_args("computer_find_text", &args).is_err());
    args.insert("include_ocr".to_string(), Value::Bool(true));
    args.insert("x".to_string(), Value::from(10));
    assert!(validate_computer_tool_args("computer_find_text", &args).is_err());
    args.insert("y".to_string(), Value::from(20));
    args.insert("width".to_string(), Value::from(300));
    args.insert("height".to_string(), Value::from(200));
    assert!(validate_computer_tool_args("computer_find_text", &args).is_ok());
    assert!(validate_computer_tool_args("computer_move_to_text", &args).is_ok());
    assert!(validate_computer_tool_args("computer_click_text", &args).is_ok());
    args.insert("source".to_string(), Value::String("ocr".to_string()));
    args.insert("occurrence".to_string(), Value::from(2));
    args.insert("offset_x".to_string(), Value::from(-4));
    args.insert("offset_y".to_string(), Value::from(3));
    args.insert("wait_after_ms".to_string(), Value::from(0));
    assert!(validate_computer_tool_args("computer_move_to_text", &args).is_ok());
    args.insert("button".to_string(), Value::String("left".to_string()));
    args.insert("clicks".to_string(), Value::from(2));
    args.insert("modifiers".to_string(), json!(["Command"]));
    assert!(validate_computer_tool_args("computer_click_text", &args).is_ok());
    assert!(validate_computer_tool_args("computer_select_text", &args).is_ok());
    args.insert("start_offset_x".to_string(), Value::from(2));
    args.insert("end_offset_x".to_string(), Value::from(-2));
    args.insert("duration_ms".to_string(), Value::from(350));
    assert!(validate_computer_tool_args("computer_select_text", &args).is_ok());
    args.insert("duration_ms".to_string(), Value::from(10_001));
    assert!(validate_computer_tool_args("computer_select_text", &args).is_err());
    args.insert("duration_ms".to_string(), Value::from(350));
    args.insert("source".to_string(), Value::String("vision".to_string()));
    assert!(validate_computer_tool_args("computer_move_to_text", &args).is_err());
    assert!(validate_computer_tool_args("computer_click_text", &args).is_err());
    assert!(validate_computer_tool_args("computer_select_text", &args).is_err());
    args.insert("source".to_string(), Value::String("ocr".to_string()));
    args.insert("occurrence".to_string(), Value::from(0));
    assert!(validate_computer_tool_args("computer_move_to_text", &args).is_err());
    assert!(validate_computer_tool_args("computer_click_text", &args).is_err());
    assert!(validate_computer_tool_args("computer_select_text", &args).is_err());
    args.insert("occurrence".to_string(), Value::from(2));
    args.insert("button".to_string(), Value::String("side".to_string()));
    assert!(validate_computer_tool_args("computer_click_text", &args).is_err());
    args.insert("button".to_string(), Value::String("left".to_string()));
    assert!(validate_computer_tool_args("computer_assert_text", &args).is_ok());
    args.insert("exists".to_string(), Value::Bool(false));
    assert!(validate_computer_tool_args("computer_assert_text", &args).is_ok());
    assert!(validate_computer_tool_args("computer_wait_for_text", &args).is_ok());
    args.insert("timeout_ms".to_string(), Value::from(60_001));
    assert!(validate_computer_tool_args("computer_wait_for_text", &args).is_err());
    args.insert("timeout_ms".to_string(), Value::from(5000));
    args.insert("interval_ms".to_string(), Value::from(99));
    assert!(validate_computer_tool_args("computer_wait_for_text", &args).is_err());
    args.insert("interval_ms".to_string(), Value::from(500));
    args.insert("exists".to_string(), Value::String("false".to_string()));
    assert!(validate_computer_tool_args("computer_assert_text", &args).is_err());
    args.clear();
    assert!(validate_computer_tool_args("computer_find_text", &args).is_err());

    assert!(validate_computer_tool_args("computer_wait", &args).is_ok());
    args.insert("until_stable".to_string(), Value::Bool(true));
    args.insert("timeout_ms".to_string(), Value::from(1000));
    args.insert("stable_ms".to_string(), Value::from(100));
    args.insert("interval_ms".to_string(), Value::from(100));
    args.insert("display_index".to_string(), Value::from(1));
    assert!(validate_computer_tool_args("computer_wait", &args).is_ok());
    args.insert("until_stable".to_string(), Value::String("yes".to_string()));
    assert!(validate_computer_tool_args("computer_wait", &args).is_err());
    args.insert("until_stable".to_string(), Value::Bool(true));
    args.insert("interval_ms".to_string(), Value::from(99));
    assert!(validate_computer_tool_args("computer_wait", &args).is_err());
    args.insert("interval_ms".to_string(), Value::from(100));
    args.insert("x".to_string(), Value::from(10));
    assert!(validate_computer_tool_args("computer_wait", &args).is_err());
    args.remove("x");
    args.insert("duration_ms".to_string(), Value::from(30_000));
    assert!(validate_computer_tool_args("computer_wait", &args).is_ok());
    args.insert("duration_ms".to_string(), Value::from(30_001));
    assert!(validate_computer_tool_args("computer_wait", &args).is_err());
    args.clear();

    assert!(validate_computer_tool_args("computer_accessibility_snapshot", &args).is_ok());
    args.insert("max_elements".to_string(), Value::from(500));
    args.insert("include_actions".to_string(), Value::Bool(true));
    args.insert("include_state".to_string(), Value::Bool(true));
    assert!(validate_computer_tool_args("computer_accessibility_snapshot", &args).is_ok());
    args.insert(
        "include_state".to_string(),
        Value::String("yes".to_string()),
    );
    assert!(validate_computer_tool_args("computer_accessibility_snapshot", &args).is_err());
    args.insert("include_state".to_string(), Value::Bool(true));
    args.insert("max_elements".to_string(), Value::from(0));
    assert!(validate_computer_tool_args("computer_accessibility_snapshot", &args).is_err());
    args.clear();

    args.insert("x".to_string(), Value::from(10));
    args.insert("y".to_string(), Value::from(20));
    assert!(validate_computer_tool_args("computer_element_at", &args).is_ok());
    args.insert(
        "include_actions".to_string(),
        Value::String("yes".to_string()),
    );
    assert!(validate_computer_tool_args("computer_element_at", &args).is_err());
    args.insert("include_actions".to_string(), Value::Bool(true));
    args.insert("max_elements".to_string(), Value::from(0));
    assert!(validate_computer_tool_args("computer_element_at", &args).is_err());
    args.remove("max_elements");
    args.remove("y");
    assert!(validate_computer_tool_args("computer_element_at", &args).is_err());
    args.clear();

    args.insert("x".to_string(), Value::from(10));
    args.insert("y".to_string(), Value::from(20));
    args.insert("screenshot".to_string(), Value::Bool(false));
    assert!(validate_computer_tool_args("computer_press_at", &args).is_ok());
    args.insert("screenshot".to_string(), Value::String("false".to_string()));
    assert!(validate_computer_tool_args("computer_press_at", &args).is_err());
    args.insert("screenshot".to_string(), Value::Bool(false));
    args.insert("wait_after_ms".to_string(), Value::from(5001));
    assert!(validate_computer_tool_args("computer_press_at", &args).is_err());
    args.remove("wait_after_ms");
    args.remove("x");
    assert!(validate_computer_tool_args("computer_press_at", &args).is_err());
    args.clear();

    args.insert("x".to_string(), Value::from(10));
    args.insert("y".to_string(), Value::from(20));
    args.insert("text".to_string(), Value::String("Hello".to_string()));
    args.insert("paste".to_string(), Value::Bool(false));
    args.insert("clear".to_string(), Value::Bool(false));
    args.insert("press_enter".to_string(), Value::Bool(true));
    args.insert("paste_settle_ms".to_string(), Value::from(0));
    args.insert("screenshot".to_string(), Value::Bool(false));
    assert!(validate_computer_tool_args("computer_type_at", &args).is_ok());
    args.insert("paste".to_string(), Value::String("false".to_string()));
    assert!(validate_computer_tool_args("computer_type_at", &args).is_err());
    args.insert("paste".to_string(), Value::Bool(false));
    args.insert("paste_settle_ms".to_string(), Value::from(2001));
    assert!(validate_computer_tool_args("computer_type_at", &args).is_err());
    args.insert("paste_settle_ms".to_string(), Value::from(0));
    args.remove("text");
    assert!(validate_computer_tool_args("computer_type_at", &args).is_err());
    args.clear();

    args.insert("x".to_string(), Value::from(10));
    args.insert("y".to_string(), Value::from(20));
    args.insert("direction".to_string(), Value::String("down".to_string()));
    args.insert("amount".to_string(), Value::from(300));
    args.insert("modifiers".to_string(), json!(["Shift"]));
    args.insert("screenshot".to_string(), Value::Bool(false));
    assert!(validate_computer_tool_args("computer_scroll_at", &args).is_ok());
    args.insert(
        "direction".to_string(),
        Value::String("diagonal".to_string()),
    );
    assert!(validate_computer_tool_args("computer_scroll_at", &args).is_err());
    args.insert("direction".to_string(), Value::String("down".to_string()));
    args.insert("modifiers".to_string(), json!(["Hyper"]));
    assert!(validate_computer_tool_args("computer_scroll_at", &args).is_err());
    args.insert("modifiers".to_string(), json!(["Shift"]));
    args.insert("amount".to_string(), Value::from(5001));
    assert!(validate_computer_tool_args("computer_scroll_at", &args).is_err());
    args.remove("amount");
    args.remove("y");
    assert!(validate_computer_tool_args("computer_scroll_at", &args).is_err());
    args.clear();

    args.insert("query".to_string(), Value::String("Open".to_string()));
    assert!(validate_computer_tool_args("computer_click_element", &args).is_ok());
    args.insert("match_mode".to_string(), Value::String("exact".to_string()));
    args.insert("role".to_string(), Value::String("AXButton".to_string()));
    args.insert("occurrence".to_string(), Value::from(2));
    args.insert("offset_x".to_string(), Value::from(-12.5));
    args.insert("offset_y".to_string(), Value::from(4));
    args.insert("modifiers".to_string(), json!(["Command", "Shift"]));
    assert!(validate_computer_tool_args("computer_click_element", &args).is_ok());
    args.insert("offset_x".to_string(), Value::from(-10_001));
    assert!(validate_computer_tool_args("computer_click_element", &args).is_err());
    args.insert("offset_x".to_string(), Value::from(-12.5));
    args.insert("modifiers".to_string(), json!(["Hyper"]));
    assert!(validate_computer_tool_args("computer_click_element", &args).is_err());
    args.insert(
        "modifiers".to_string(),
        json!(["Command", "Shift", "Option", "Control", "Alt"]),
    );
    assert!(validate_computer_tool_args("computer_click_element", &args).is_err());
    args.insert("modifiers".to_string(), json!(["Command"]));
    args.insert("match_mode".to_string(), Value::String("regex".to_string()));
    assert!(validate_computer_tool_args("computer_click_element", &args).is_err());
    args.insert(
        "match_mode".to_string(),
        Value::String("contains".to_string()),
    );
    args.insert("occurrence".to_string(), Value::from(0));
    assert!(validate_computer_tool_args("computer_click_element", &args).is_err());
    args.clear();
    assert!(validate_computer_tool_args("computer_click_element", &args).is_err());
    args.insert("index".to_string(), Value::from(12));
    assert!(validate_computer_tool_args("computer_click_element", &args).is_ok());
    args.insert("index".to_string(), Value::from(0));
    assert!(validate_computer_tool_args("computer_click_element", &args).is_err());

    args.clear();
    args.insert("query".to_string(), Value::String("Open".to_string()));
    assert!(validate_computer_tool_args("computer_press_element", &args).is_ok());
    args.insert("role".to_string(), Value::String("AXButton".to_string()));
    args.insert("match_mode".to_string(), Value::String("exact".to_string()));
    args.insert("occurrence".to_string(), Value::from(2));
    args.insert("wait_after_ms".to_string(), Value::from(0));
    assert!(validate_computer_tool_args("computer_press_element", &args).is_ok());
    args.insert("match_mode".to_string(), Value::String("regex".to_string()));
    assert!(validate_computer_tool_args("computer_press_element", &args).is_err());
    args.insert(
        "match_mode".to_string(),
        Value::String("contains".to_string()),
    );
    args.insert("occurrence".to_string(), Value::from(0));
    assert!(validate_computer_tool_args("computer_press_element", &args).is_err());
    args.clear();
    assert!(validate_computer_tool_args("computer_press_element", &args).is_err());
    args.insert("index".to_string(), Value::from(12));
    assert!(validate_computer_tool_args("computer_press_element", &args).is_ok());
    args.insert("index".to_string(), Value::from(0));
    assert!(validate_computer_tool_args("computer_press_element", &args).is_err());

    args.clear();
    args.insert("query".to_string(), Value::String("Search".to_string()));
    assert!(validate_computer_tool_args("computer_focus_element", &args).is_ok());
    args.insert("role".to_string(), Value::String("AXTextField".to_string()));
    args.insert("match_mode".to_string(), Value::String("exact".to_string()));
    args.insert("occurrence".to_string(), Value::from(2));
    args.insert("wait_after_ms".to_string(), Value::from(0));
    assert!(validate_computer_tool_args("computer_focus_element", &args).is_ok());
    args.insert("match_mode".to_string(), Value::String("regex".to_string()));
    assert!(validate_computer_tool_args("computer_focus_element", &args).is_err());
    args.insert(
        "match_mode".to_string(),
        Value::String("contains".to_string()),
    );
    args.insert("occurrence".to_string(), Value::from(0));
    assert!(validate_computer_tool_args("computer_focus_element", &args).is_err());
    args.clear();
    assert!(validate_computer_tool_args("computer_focus_element", &args).is_err());
    args.insert("index".to_string(), Value::from(12));
    assert!(validate_computer_tool_args("computer_focus_element", &args).is_ok());
    args.insert("index".to_string(), Value::from(0));
    assert!(validate_computer_tool_args("computer_focus_element", &args).is_err());

    args.clear();
    args.insert("query".to_string(), Value::String("Menu".to_string()));
    args.insert(
        "action".to_string(),
        Value::String("AXShowMenu".to_string()),
    );
    assert!(validate_computer_tool_args("computer_perform_element_action", &args).is_ok());
    args.insert(
        "role".to_string(),
        Value::String("AXPopUpButton".to_string()),
    );
    args.insert("match_mode".to_string(), Value::String("exact".to_string()));
    args.insert("occurrence".to_string(), Value::from(2));
    args.insert("wait_after_ms".to_string(), Value::from(0));
    assert!(validate_computer_tool_args("computer_perform_element_action", &args).is_ok());
    args.insert("action".to_string(), Value::String("AXDelete".to_string()));
    assert!(validate_computer_tool_args("computer_perform_element_action", &args).is_err());
    args.insert(
        "action".to_string(),
        Value::String("AXShowMenu".to_string()),
    );
    args.insert("match_mode".to_string(), Value::String("regex".to_string()));
    assert!(validate_computer_tool_args("computer_perform_element_action", &args).is_err());
    args.clear();
    assert!(validate_computer_tool_args("computer_perform_element_action", &args).is_err());

    args.clear();
    args.insert("query".to_string(), Value::String("Open".to_string()));
    args.insert("wait_after_ms".to_string(), Value::from(0));
    args.insert("offset_x".to_string(), Value::from(10));
    args.insert("offset_y".to_string(), Value::from(-5));
    assert!(validate_computer_tool_args("computer_move_to_element", &args).is_ok());
    args.insert("offset_y".to_string(), Value::from(10_001));
    assert!(validate_computer_tool_args("computer_move_to_element", &args).is_err());
    args.insert("offset_y".to_string(), Value::from(-5));
    args.insert("match_mode".to_string(), Value::String("regex".to_string()));
    assert!(validate_computer_tool_args("computer_move_to_element", &args).is_err());
    args.insert(
        "match_mode".to_string(),
        Value::String("contains".to_string()),
    );
    args.insert("wait_after_ms".to_string(), Value::from(5001));
    assert!(validate_computer_tool_args("computer_move_to_element", &args).is_err());
    args.clear();
    assert!(validate_computer_tool_args("computer_move_to_element", &args).is_err());
    args.insert("index".to_string(), Value::from(2));
    assert!(validate_computer_tool_args("computer_move_to_element", &args).is_ok());

    args.clear();
    args.insert("index".to_string(), Value::from(7));
    args.insert("text".to_string(), Value::String("Nola".to_string()));
    assert!(validate_computer_tool_args("computer_type_element", &args).is_ok());
    args.insert("query".to_string(), Value::String("Search".to_string()));
    args.insert("text".to_string(), Value::String("Nola".to_string()));
    assert!(validate_computer_tool_args("computer_type_element", &args).is_ok());
    args.insert("match_mode".to_string(), Value::String("exact".to_string()));
    args.insert("role".to_string(), Value::String("AXTextField".to_string()));
    args.insert("offset_x".to_string(), Value::from(-10));
    args.insert("offset_y".to_string(), Value::from(4));
    args.insert("paste".to_string(), Value::Bool(false));
    args.insert("clear".to_string(), Value::Bool(false));
    args.insert("press_enter".to_string(), Value::Bool(true));
    args.insert("paste_settle_ms".to_string(), Value::from(0));
    assert!(validate_computer_tool_args("computer_type_element", &args).is_ok());
    args.insert("paste_settle_ms".to_string(), Value::from(2001));
    assert!(validate_computer_tool_args("computer_type_element", &args).is_err());
    args.insert("paste_settle_ms".to_string(), Value::from(0));
    args.insert("clear".to_string(), Value::String("false".to_string()));
    assert!(validate_computer_tool_args("computer_type_element", &args).is_err());
    args.insert("clear".to_string(), Value::Bool(false));
    args.insert("offset_y".to_string(), Value::from(10_001));
    assert!(validate_computer_tool_args("computer_type_element", &args).is_err());
    args.insert("offset_y".to_string(), Value::from(4));
    args.insert("match_mode".to_string(), Value::String("regex".to_string()));
    assert!(validate_computer_tool_args("computer_type_element", &args).is_err());
    args.insert(
        "match_mode".to_string(),
        Value::String("contains".to_string()),
    );
    args.insert("wait_after_ms".to_string(), Value::from(5001));
    assert!(validate_computer_tool_args("computer_type_element", &args).is_err());
    args.clear();
    assert!(validate_computer_tool_args("computer_type_element", &args).is_err());
    args.insert("index".to_string(), Value::from(3));
    args.insert("text".to_string(), Value::String("Nola".to_string()));
    assert!(validate_computer_tool_args("computer_type_element", &args).is_ok());

    args.clear();
    args.insert("index".to_string(), Value::from(7));
    args.insert("value".to_string(), Value::String("Nola".to_string()));
    assert!(validate_computer_tool_args("computer_set_element_value", &args).is_ok());
    args.insert("query".to_string(), Value::String("Search".to_string()));
    args.insert("role".to_string(), Value::String("AXTextField".to_string()));
    args.insert("match_mode".to_string(), Value::String("exact".to_string()));
    args.insert("wait_after_ms".to_string(), Value::from(0));
    args.insert("press_enter".to_string(), Value::Bool(true));
    args.insert("screenshot".to_string(), Value::Bool(false));
    assert!(validate_computer_tool_args("computer_set_element_value", &args).is_ok());
    args.insert(
        "press_enter".to_string(),
        Value::String("false".to_string()),
    );
    assert!(validate_computer_tool_args("computer_set_element_value", &args).is_err());
    args.insert("press_enter".to_string(), Value::Bool(true));
    args.insert("screenshot".to_string(), Value::String("false".to_string()));
    assert!(validate_computer_tool_args("computer_set_element_value", &args).is_err());
    args.insert("screenshot".to_string(), Value::Bool(false));
    args.insert("match_mode".to_string(), Value::String("regex".to_string()));
    assert!(validate_computer_tool_args("computer_set_element_value", &args).is_err());
    args.insert(
        "match_mode".to_string(),
        Value::String("contains".to_string()),
    );
    args.insert("wait_after_ms".to_string(), Value::from(5001));
    assert!(validate_computer_tool_args("computer_set_element_value", &args).is_err());
    args.clear();
    assert!(validate_computer_tool_args("computer_set_element_value", &args).is_err());
    args.insert("query".to_string(), Value::String("Search".to_string()));
    assert!(validate_computer_tool_args("computer_set_element_value", &args).is_err());

    args.clear();
    args.insert("query".to_string(), Value::String("Saved".to_string()));
    assert!(validate_computer_tool_args("computer_assert_element", &args).is_ok());
    args.insert("exists".to_string(), Value::Bool(false));
    args.insert(
        "role".to_string(),
        Value::String("AXStaticText".to_string()),
    );
    args.insert(
        "value_contains".to_string(),
        Value::String("Done".to_string()),
    );
    args.insert("enabled".to_string(), Value::Bool(true));
    args.insert("focused".to_string(), Value::Bool(false));
    args.insert("selected".to_string(), Value::Bool(false));
    assert!(validate_computer_tool_args("computer_assert_element", &args).is_ok());
    args.insert("focused".to_string(), Value::String("false".to_string()));
    assert!(validate_computer_tool_args("computer_assert_element", &args).is_err());
    args.insert("focused".to_string(), Value::Bool(false));
    args.insert("match_mode".to_string(), Value::String("regex".to_string()));
    assert!(validate_computer_tool_args("computer_assert_element", &args).is_err());
    args.insert(
        "match_mode".to_string(),
        Value::String("contains".to_string()),
    );
    args.insert(
        "value_contains".to_string(),
        Value::String("bad\nvalue".to_string()),
    );
    assert!(validate_computer_tool_args("computer_assert_element", &args).is_err());
    args.clear();
    assert!(validate_computer_tool_args("computer_assert_element", &args).is_err());
    args.insert("index".to_string(), Value::from(9));
    assert!(validate_computer_tool_args("computer_assert_element", &args).is_ok());

    args.insert("query".to_string(), Value::String("Loaded".to_string()));
    assert!(validate_computer_tool_args("computer_wait_for_element", &args).is_ok());
    args.insert("exists".to_string(), Value::Bool(false));
    args.insert("focused".to_string(), Value::Bool(true));
    args.insert("timeout_ms".to_string(), Value::from(1000));
    args.insert("interval_ms".to_string(), Value::from(100));
    assert!(validate_computer_tool_args("computer_wait_for_element", &args).is_ok());
    args.insert("selected".to_string(), Value::String("false".to_string()));
    assert!(validate_computer_tool_args("computer_wait_for_element", &args).is_err());
    args.remove("selected");
    args.insert("timeout_ms".to_string(), Value::from(60_001));
    assert!(validate_computer_tool_args("computer_wait_for_element", &args).is_err());
    args.insert("timeout_ms".to_string(), Value::from(1000));
    args.insert("interval_ms".to_string(), Value::from(99));
    assert!(validate_computer_tool_args("computer_wait_for_element", &args).is_err());
    args.clear();
    assert!(validate_computer_tool_args("computer_wait_for_element", &args).is_err());
    args.insert("index".to_string(), Value::from(4));
    assert!(validate_computer_tool_args("computer_wait_for_element", &args).is_ok());

    args.clear();
    args.insert("x".to_string(), Value::from(-50));
    assert!(validate_computer_tool_args("computer_move_mouse", &args).is_err());
    args.insert("y".to_string(), Value::from(200));
    assert!(validate_computer_tool_args("computer_move_mouse", &args).is_ok());
    args.insert("wait_after_ms".to_string(), Value::from(5001));
    assert!(validate_computer_tool_args("computer_move_mouse", &args).is_err());
    args.insert("wait_after_ms".to_string(), Value::from(0));
    args.insert("x".to_string(), Value::from(-100001));
    assert!(validate_computer_tool_args("computer_move_mouse", &args).is_err());

    args.clear();
    assert!(validate_computer_tool_args("computer_mouse_down", &args).is_ok());
    args.insert("x".to_string(), Value::from(100));
    assert!(validate_computer_tool_args("computer_mouse_down", &args).is_err());
    args.insert("y".to_string(), Value::from(200));
    args.insert("button".to_string(), Value::String("right".to_string()));
    args.insert("modifiers".to_string(), json!(["Shift"]));
    args.insert("wait_after_ms".to_string(), Value::from(0));
    assert!(validate_computer_tool_args("computer_mouse_down", &args).is_ok());
    assert!(validate_computer_tool_args("computer_mouse_up", &args).is_ok());
    args.insert("modifiers".to_string(), json!(["Hyper"]));
    assert!(validate_computer_tool_args("computer_mouse_up", &args).is_err());

    args.clear();
    args.insert("x".to_string(), Value::from(100));
    assert!(validate_computer_tool_args("computer_click", &args).is_err());
    args.insert("y".to_string(), Value::from(200));
    assert!(validate_computer_tool_args("computer_click", &args).is_ok());
    args.insert("modifiers".to_string(), json!(["Option"]));
    args.insert("wait_after_ms".to_string(), Value::from(0));
    assert!(validate_computer_tool_args("computer_click", &args).is_ok());
    args.insert(
        "modifiers".to_string(),
        Value::String("Command".to_string()),
    );
    assert!(validate_computer_tool_args("computer_click", &args).is_err());
    args.remove("modifiers");
    args.insert("wait_after_ms".to_string(), Value::from(5001));
    assert!(validate_computer_tool_args("computer_click", &args).is_err());
    args.insert("wait_after_ms".to_string(), Value::from(0));
    args.insert("button".to_string(), Value::String("side".to_string()));
    assert!(validate_computer_tool_args("computer_click", &args).is_err());

    args.clear();
    args.insert("start_x".to_string(), Value::from(10));
    args.insert("start_y".to_string(), Value::from(20));
    args.insert("end_x".to_string(), Value::from(30));
    args.insert("end_y".to_string(), Value::from(40));
    args.insert("wait_after_ms".to_string(), Value::from(0));
    assert!(validate_computer_tool_args("computer_drag", &args).is_ok());
    args.insert("wait_after_ms".to_string(), Value::from(5001));
    assert!(validate_computer_tool_args("computer_drag", &args).is_err());

    let mut scroll_args = Map::new();
    scroll_args.insert("direction".to_string(), Value::String("down".to_string()));
    scroll_args.insert("amount".to_string(), Value::from(400));
    scroll_args.insert("x".to_string(), Value::from(100));
    scroll_args.insert("y".to_string(), Value::from(200));
    scroll_args.insert("wait_after_ms".to_string(), Value::from(0));
    scroll_args.insert("modifiers".to_string(), json!(["Shift"]));
    assert!(validate_computer_tool_args("computer_scroll", &scroll_args).is_ok());
    scroll_args.remove("y");
    assert!(validate_computer_tool_args("computer_scroll", &scroll_args).is_err());
    scroll_args.insert("y".to_string(), Value::from(200));
    scroll_args.insert("modifiers".to_string(), json!(["Hyper"]));
    assert!(validate_computer_tool_args("computer_scroll", &scroll_args).is_err());
    scroll_args.insert("modifiers".to_string(), json!(["Shift"]));
    scroll_args.insert("wait_after_ms".to_string(), Value::from(5001));
    assert!(validate_computer_tool_args("computer_scroll", &scroll_args).is_err());
    scroll_args.remove("wait_after_ms");
    scroll_args.insert("amount".to_string(), Value::from(5001));
    assert!(validate_computer_tool_args("computer_scroll", &scroll_args).is_err());

    let mut scroll_element_args = Map::new();
    scroll_element_args.insert("query".to_string(), Value::String("List".to_string()));
    scroll_element_args.insert("direction".to_string(), Value::String("down".to_string()));
    scroll_element_args.insert("amount".to_string(), Value::from(300));
    scroll_element_args.insert("offset_x".to_string(), Value::from(-10));
    scroll_element_args.insert("offset_y".to_string(), Value::from(4));
    scroll_element_args.insert("modifiers".to_string(), json!(["Shift"]));
    assert!(validate_computer_tool_args("computer_scroll_element", &scroll_element_args).is_ok());
    scroll_element_args.insert("offset_x".to_string(), Value::from(-10_001));
    assert!(validate_computer_tool_args("computer_scroll_element", &scroll_element_args).is_err());
    scroll_element_args.insert("offset_x".to_string(), Value::from(-10));
    scroll_element_args.insert(
        "direction".to_string(),
        Value::String("diagonal".to_string()),
    );
    assert!(validate_computer_tool_args("computer_scroll_element", &scroll_element_args).is_err());
    scroll_element_args.clear();
    assert!(validate_computer_tool_args("computer_scroll_element", &scroll_element_args).is_err());

    let mut drag_args = Map::new();
    drag_args.insert("start_x".to_string(), Value::from(10));
    drag_args.insert("start_y".to_string(), Value::from(20));
    drag_args.insert("end_x".to_string(), Value::from(30));
    assert!(validate_computer_tool_args("computer_drag", &drag_args).is_err());
    drag_args.insert("end_y".to_string(), Value::from(40));
    assert!(validate_computer_tool_args("computer_drag", &drag_args).is_ok());
    drag_args.insert("modifiers".to_string(), json!(["Shift"]));
    assert!(validate_computer_tool_args("computer_drag", &drag_args).is_ok());
    drag_args.insert("modifiers".to_string(), json!([1]));
    assert!(validate_computer_tool_args("computer_drag", &drag_args).is_err());
    drag_args.remove("modifiers");
    drag_args.insert("duration_ms".to_string(), Value::from(-1));
    assert!(validate_computer_tool_args("computer_drag", &drag_args).is_err());

    let mut drag_element_args = Map::new();
    drag_element_args.insert("query".to_string(), Value::String("Item".to_string()));
    drag_element_args.insert("target_x".to_string(), Value::from(300));
    drag_element_args.insert("target_y".to_string(), Value::from(400));
    drag_element_args.insert("wait_after_ms".to_string(), Value::from(0));
    drag_element_args.insert("offset_x".to_string(), Value::from(-8));
    drag_element_args.insert("offset_y".to_string(), Value::from(2));
    assert!(validate_computer_tool_args("computer_drag_element", &drag_element_args).is_ok());
    drag_element_args.insert("offset_x".to_string(), Value::from(10_001));
    assert!(validate_computer_tool_args("computer_drag_element", &drag_element_args).is_err());
    drag_element_args.insert("offset_x".to_string(), Value::from(-8));
    drag_element_args.insert("target_offset_x".to_string(), Value::from(1));
    assert!(validate_computer_tool_args("computer_drag_element", &drag_element_args).is_err());
    drag_element_args.remove("target_offset_x");
    drag_element_args.insert(
        "target_query".to_string(),
        Value::String("Drop".to_string()),
    );
    assert!(validate_computer_tool_args("computer_drag_element", &drag_element_args).is_err());
    drag_element_args.remove("target_x");
    drag_element_args.remove("target_y");
    drag_element_args.insert("target_offset_x".to_string(), Value::from(10));
    drag_element_args.insert("target_offset_y".to_string(), Value::from(-4));
    assert!(validate_computer_tool_args("computer_drag_element", &drag_element_args).is_ok());
    drag_element_args.insert("target_offset_y".to_string(), Value::from(-10_001));
    assert!(validate_computer_tool_args("computer_drag_element", &drag_element_args).is_err());
    drag_element_args.insert("target_offset_y".to_string(), Value::from(-4));
    drag_element_args.insert(
        "target_match_mode".to_string(),
        Value::String("regex".to_string()),
    );
    assert!(validate_computer_tool_args("computer_drag_element", &drag_element_args).is_err());
    drag_element_args.insert(
        "target_match_mode".to_string(),
        Value::String("contains".to_string()),
    );
    drag_element_args.insert("modifiers".to_string(), json!(["Shift"]));
    assert!(validate_computer_tool_args("computer_drag_element", &drag_element_args).is_ok());
    drag_element_args.insert("modifiers".to_string(), json!(["Hyper"]));
    assert!(validate_computer_tool_args("computer_drag_element", &drag_element_args).is_err());
    drag_element_args.clear();
    drag_element_args.insert("target_x".to_string(), Value::from(300));
    drag_element_args.insert("target_y".to_string(), Value::from(400));
    assert!(validate_computer_tool_args("computer_drag_element", &drag_element_args).is_err());

    let mut type_args = Map::new();
    type_args.insert("text".to_string(), Value::String("hello".to_string()));
    assert!(validate_computer_tool_args("computer_type_text", &type_args).is_ok());
    type_args.insert("paste".to_string(), Value::Bool(false));
    type_args.insert("paste_settle_ms".to_string(), Value::from(0));
    type_args.insert("clear".to_string(), Value::Bool(true));
    type_args.insert("press_enter".to_string(), Value::Bool(true));
    type_args.insert("wait_after_ms".to_string(), Value::from(0));
    type_args.insert("screenshot".to_string(), Value::Bool(false));
    assert!(validate_computer_tool_args("computer_type_text", &type_args).is_ok());
    type_args.insert("clear".to_string(), Value::String("false".to_string()));
    assert!(validate_computer_tool_args("computer_type_text", &type_args).is_err());
    type_args.insert("clear".to_string(), Value::Bool(false));
    type_args.insert("press_enter".to_string(), Value::String("true".to_string()));
    assert!(validate_computer_tool_args("computer_type_text", &type_args).is_err());
    type_args.insert("press_enter".to_string(), Value::Bool(true));
    type_args.insert("paste_settle_ms".to_string(), Value::from(2001));
    assert!(validate_computer_tool_args("computer_type_text", &type_args).is_err());
    type_args.insert("paste_settle_ms".to_string(), Value::from(0));
    type_args.insert("wait_after_ms".to_string(), Value::from(5001));
    assert!(validate_computer_tool_args("computer_type_text", &type_args).is_err());
    type_args.insert("wait_after_ms".to_string(), Value::from(0));
    type_args.insert("paste".to_string(), Value::String("yes".to_string()));
    assert!(validate_computer_tool_args("computer_type_text", &type_args).is_err());

    let mut clipboard_args = Map::new();
    assert!(validate_computer_tool_args("computer_get_clipboard", &clipboard_args).is_ok());
    clipboard_args.insert("max_chars".to_string(), Value::from(50_001));
    assert!(validate_computer_tool_args("computer_get_clipboard", &clipboard_args).is_err());
    clipboard_args.clear();
    clipboard_args.insert("text".to_string(), Value::String("clip".to_string()));
    assert!(validate_computer_tool_args("computer_set_clipboard", &clipboard_args).is_ok());
    clipboard_args.insert("text".to_string(), Value::String("x".repeat(100_001)));
    assert!(validate_computer_tool_args("computer_set_clipboard", &clipboard_args).is_err());
    clipboard_args.clear();
    assert!(validate_computer_tool_args("computer_set_clipboard", &clipboard_args).is_err());
    assert!(validate_computer_tool_args("computer_copy_selection", &clipboard_args).is_ok());
    clipboard_args.insert("wait_after_ms".to_string(), Value::from(5001));
    assert!(validate_computer_tool_args("computer_copy_selection", &clipboard_args).is_err());
    clipboard_args.clear();
    clipboard_args.insert("text".to_string(), Value::String("done".to_string()));
    assert!(validate_computer_tool_args("computer_assert_clipboard", &clipboard_args).is_ok());
    assert!(validate_computer_tool_args("computer_wait_for_clipboard", &clipboard_args).is_ok());
    clipboard_args.insert("match_mode".to_string(), Value::String("regex".to_string()));
    assert!(validate_computer_tool_args("computer_assert_clipboard", &clipboard_args).is_err());
    clipboard_args.insert(
        "match_mode".to_string(),
        Value::String("contains".to_string()),
    );
    clipboard_args.insert("timeout_ms".to_string(), Value::from(60_001));
    assert!(validate_computer_tool_args("computer_wait_for_clipboard", &clipboard_args).is_err());
    clipboard_args.clear();
    clipboard_args.insert("text".to_string(), Value::String(String::new()));
    assert!(validate_computer_tool_args("computer_assert_clipboard", &clipboard_args).is_err());
    clipboard_args.insert("match_mode".to_string(), Value::String("exact".to_string()));
    assert!(validate_computer_tool_args("computer_assert_clipboard", &clipboard_args).is_ok());

    let mut key_args = Map::new();
    key_args.insert("key".to_string(), Value::String("Meta+A".to_string()));
    assert!(validate_computer_tool_args("computer_press_key", &key_args).is_ok());
    key_args.insert("key".to_string(), Value::String("Command+A".to_string()));
    assert!(validate_computer_tool_args("computer_press_key", &key_args).is_ok());
    key_args.insert(
        "key".to_string(),
        Value::String("Option+ArrowLeft".to_string()),
    );
    assert!(validate_computer_tool_args("computer_press_key", &key_args).is_ok());
    key_args.insert("key".to_string(), Value::String("F12".to_string()));
    assert!(validate_computer_tool_args("computer_press_key", &key_args).is_ok());
    key_args.insert("wait_after_ms".to_string(), Value::from(0));
    key_args.insert("screenshot".to_string(), Value::Bool(false));
    assert!(validate_computer_tool_args("computer_press_key", &key_args).is_ok());
    key_args.insert("wait_after_ms".to_string(), Value::from(5001));
    assert!(validate_computer_tool_args("computer_press_key", &key_args).is_err());
    key_args.insert("wait_after_ms".to_string(), Value::from(0));
    key_args.insert(
        "key".to_string(),
        Value::String("AudioVolumeUp".to_string()),
    );
    assert!(validate_computer_tool_args("computer_press_key", &key_args).is_err());
    key_args.insert("key".to_string(), Value::String("S".to_string()));
    key_args.insert("modifiers".to_string(), json!(["Command"]));
    key_args.insert("wait_after_ms".to_string(), Value::from(0));
    assert!(validate_computer_tool_args("computer_hotkey", &key_args).is_ok());
    key_args.insert(
        "modifiers".to_string(),
        Value::String("Command".to_string()),
    );
    assert!(validate_computer_tool_args("computer_hotkey", &key_args).is_err());
    key_args.insert("modifiers".to_string(), json!([]));
    assert!(validate_computer_tool_args("computer_hotkey", &key_args).is_err());
    key_args.insert("modifiers".to_string(), json!(["Hyper"]));
    assert!(validate_computer_tool_args("computer_hotkey", &key_args).is_err());
    key_args.insert("modifiers".to_string(), json!(["Command"]));
    key_args.insert("key".to_string(), Value::String("Shift".to_string()));
    assert!(validate_computer_tool_args("computer_hotkey", &key_args).is_err());
    key_args.insert("key".to_string(), Value::String("Shift".to_string()));
    key_args.insert("wait_after_ms".to_string(), Value::from(0));
    assert!(validate_computer_tool_args("computer_key_down", &key_args).is_ok());
    assert!(validate_computer_tool_args("computer_key_up", &key_args).is_ok());
    key_args.insert("key".to_string(), Value::String("Space".to_string()));
    assert!(validate_computer_tool_args("computer_key_down", &key_args).is_ok());
    key_args.insert("key".to_string(), Value::String("Meta+A".to_string()));
    assert!(validate_computer_tool_args("computer_key_down", &key_args).is_err());
    key_args.insert(
        "key".to_string(),
        Value::String("AudioVolumeUp".to_string()),
    );
    assert!(validate_computer_tool_args("computer_key_up", &key_args).is_err());
    key_args.insert("key".to_string(), Value::String("A".to_string()));
    key_args.insert("wait_after_ms".to_string(), Value::from(5001));
    assert!(validate_computer_tool_args("computer_key_down", &key_args).is_err());

    let mut window_args = Map::new();
    window_args.insert("max_apps".to_string(), Value::from(20));
    assert!(validate_computer_tool_args("computer_list_apps", &window_args).is_ok());
    window_args.insert("include_background".to_string(), Value::Bool(true));
    assert!(validate_computer_tool_args("computer_list_apps", &window_args).is_ok());
    window_args.insert("max_apps".to_string(), Value::from(0));
    assert!(validate_computer_tool_args("computer_list_apps", &window_args).is_err());
    window_args.insert("max_apps".to_string(), Value::from(20));
    window_args.insert(
        "include_background".to_string(),
        Value::String("yes".to_string()),
    );
    assert!(validate_computer_tool_args("computer_list_apps", &window_args).is_err());
    window_args.clear();
    window_args.insert("app".to_string(), Value::String("Safari".to_string()));
    assert!(validate_computer_tool_args("computer_assert_app", &window_args).is_ok());
    window_args.insert("frontmost".to_string(), Value::Bool(true));
    assert!(validate_computer_tool_args("computer_assert_app", &window_args).is_ok());
    window_args.insert("frontmost".to_string(), Value::String("true".to_string()));
    assert!(validate_computer_tool_args("computer_assert_app", &window_args).is_err());
    window_args.clear();
    window_args.insert(
        "bundle_id".to_string(),
        Value::String("com.apple.Safari".to_string()),
    );
    window_args.insert("timeout_ms".to_string(), Value::from(1000));
    window_args.insert("interval_ms".to_string(), Value::from(100));
    assert!(validate_computer_tool_args("computer_wait_for_app", &window_args).is_ok());
    window_args.insert("interval_ms".to_string(), Value::from(99));
    assert!(validate_computer_tool_args("computer_wait_for_app", &window_args).is_err());
    window_args.clear();
    window_args.insert("max_windows".to_string(), Value::from(20));
    assert!(validate_computer_tool_args("computer_list_windows", &window_args).is_ok());
    window_args.insert("max_windows".to_string(), Value::from(0));
    assert!(validate_computer_tool_args("computer_list_windows", &window_args).is_err());
    window_args.clear();
    window_args.insert("index".to_string(), Value::from(1));
    assert!(validate_computer_tool_args("computer_focus_window", &window_args).is_ok());
    window_args.insert("index".to_string(), Value::from(0));
    assert!(validate_computer_tool_args("computer_focus_window", &window_args).is_err());
    window_args.clear();
    window_args.insert("app".to_string(), Value::String("Safari".to_string()));
    window_args.insert("title".to_string(), Value::String("Nola".to_string()));
    window_args.insert("match_mode".to_string(), Value::String("exact".to_string()));
    assert!(validate_computer_tool_args("computer_focus_window", &window_args).is_ok());
    window_args.insert("match_mode".to_string(), Value::String("regex".to_string()));
    assert!(validate_computer_tool_args("computer_focus_window", &window_args).is_err());
    window_args.clear();
    window_args.insert("index".to_string(), Value::from(1));
    window_args.insert("x".to_string(), Value::from(0));
    window_args.insert("y".to_string(), Value::from(24));
    window_args.insert("width".to_string(), Value::from(900));
    window_args.insert("height".to_string(), Value::from(700));
    assert!(validate_computer_tool_args("computer_set_window_bounds", &window_args).is_ok());
    window_args.insert("width".to_string(), Value::from(0));
    assert!(validate_computer_tool_args("computer_set_window_bounds", &window_args).is_err());
    window_args.insert("width".to_string(), Value::from(900));
    window_args.remove("index");
    assert!(validate_computer_tool_args("computer_set_window_bounds", &window_args).is_err());
    window_args.clear();
    window_args.insert("index".to_string(), Value::from(1));
    window_args.insert("action".to_string(), Value::String("minimize".to_string()));
    window_args.insert("wait_after_ms".to_string(), Value::from(0));
    assert!(validate_computer_tool_args("computer_perform_window_action", &window_args).is_ok());
    window_args.insert("action".to_string(), Value::String("tile".to_string()));
    assert!(validate_computer_tool_args("computer_perform_window_action", &window_args).is_err());
    window_args.insert("action".to_string(), Value::String("zoom".to_string()));
    window_args.insert("wait_after_ms".to_string(), Value::from(5001));
    assert!(validate_computer_tool_args("computer_perform_window_action", &window_args).is_err());
    window_args.clear();
    window_args.insert("action".to_string(), Value::String("minimize".to_string()));
    assert!(validate_computer_tool_args("computer_perform_window_action", &window_args).is_err());
    window_args.clear();
    window_args.insert("index".to_string(), Value::from(1));
    window_args.insert("wait_after_ms".to_string(), Value::from(0));
    assert!(validate_computer_tool_args("computer_close_window", &window_args).is_ok());
    window_args.insert("wait_after_ms".to_string(), Value::from(5001));
    assert!(validate_computer_tool_args("computer_close_window", &window_args).is_err());
    window_args.clear();
    window_args.insert("title".to_string(), Value::String("Nola".to_string()));
    assert!(validate_computer_tool_args("computer_assert_window", &window_args).is_ok());
    window_args.insert("focused".to_string(), Value::Bool(true));
    window_args.insert("x".to_string(), Value::from(0));
    window_args.insert("y".to_string(), Value::from(24));
    window_args.insert("width".to_string(), Value::from(900));
    window_args.insert("height".to_string(), Value::from(700));
    window_args.insert("tolerance".to_string(), Value::from(8));
    window_args.insert("minimized".to_string(), Value::Bool(false));
    window_args.insert("full_screen".to_string(), Value::Bool(false));
    assert!(validate_computer_tool_args("computer_assert_window", &window_args).is_ok());
    window_args.insert("tolerance".to_string(), Value::from(501));
    assert!(validate_computer_tool_args("computer_assert_window", &window_args).is_err());
    window_args.insert("tolerance".to_string(), Value::from(8));
    window_args.insert("minimized".to_string(), Value::String("false".to_string()));
    assert!(validate_computer_tool_args("computer_assert_window", &window_args).is_err());
    window_args.clear();
    window_args.insert("index".to_string(), Value::from(1));
    window_args.insert("timeout_ms".to_string(), Value::from(1000));
    window_args.insert("interval_ms".to_string(), Value::from(100));
    assert!(validate_computer_tool_args("computer_wait_for_window", &window_args).is_ok());
    window_args.insert("interval_ms".to_string(), Value::from(99));
    assert!(validate_computer_tool_args("computer_wait_for_window", &window_args).is_err());

    let mut list_menu_args = Map::new();
    list_menu_args.insert("app".to_string(), Value::String("Safari".to_string()));
    assert!(validate_computer_tool_args("computer_list_menus", &list_menu_args).is_ok());
    list_menu_args.insert("top_menu".to_string(), Value::String("File".to_string()));
    list_menu_args.insert("max_items".to_string(), Value::from(25));
    assert!(validate_computer_tool_args("computer_list_menus", &list_menu_args).is_ok());
    list_menu_args.insert("max_items".to_string(), Value::from(0));
    assert!(validate_computer_tool_args("computer_list_menus", &list_menu_args).is_err());
    list_menu_args.insert("max_items".to_string(), Value::from(25));
    list_menu_args.insert(
        "top_menu".to_string(),
        Value::String("Bad\nMenu".to_string()),
    );
    assert!(validate_computer_tool_args("computer_list_menus", &list_menu_args).is_err());

    let mut menu_args = Map::new();
    menu_args.insert("app".to_string(), Value::String("Safari".to_string()));
    menu_args.insert(
        "menu_path".to_string(),
        Value::Array(vec![
            Value::String("File".to_string()),
            Value::String("New Window".to_string()),
        ]),
    );
    assert!(validate_computer_tool_args("computer_select_menu", &menu_args).is_ok());
    menu_args.insert(
        "menu_path".to_string(),
        Value::Array(vec![Value::String("File".to_string())]),
    );
    assert!(validate_computer_tool_args("computer_select_menu", &menu_args).is_err());
    menu_args.insert(
        "menu_path".to_string(),
        Value::Array(vec![
            Value::String("File".to_string()),
            Value::String("".to_string()),
        ]),
    );
    assert!(validate_computer_tool_args("computer_select_menu", &menu_args).is_err());

    let mut app_args = Map::new();
    app_args.insert("app".to_string(), Value::String("Safari".to_string()));
    assert!(validate_computer_tool_args("computer_activate_app", &app_args).is_ok());
    app_args.insert("app".to_string(), Value::String("".to_string()));
    assert!(validate_computer_tool_args("computer_activate_app", &app_args).is_err());
    app_args.clear();
    app_args.insert("app".to_string(), Value::String("Safari".to_string()));
    app_args.insert("wait_after_ms".to_string(), Value::from(0));
    assert!(validate_computer_tool_args("computer_quit_app", &app_args).is_ok());
    app_args.insert("wait_after_ms".to_string(), Value::from(5001));
    assert!(validate_computer_tool_args("computer_quit_app", &app_args).is_err());

    let mut open_args = Map::new();
    open_args.insert("target".to_string(), Value::from("https://example.com"));
    open_args.insert("app".to_string(), Value::from("Safari"));
    open_args.insert("screenshot".to_string(), Value::Bool(false));
    open_args.insert("wait_after_ms".to_string(), Value::from(0));
    assert!(validate_computer_tool_args("computer_open_item", &open_args).is_ok());
    open_args.insert("target".to_string(), Value::from("javascript:alert(1)"));
    assert!(validate_computer_tool_args("computer_open_item", &open_args).is_err());
    open_args.insert("target".to_string(), Value::from("https:///missing-host"));
    assert!(validate_computer_tool_args("computer_open_item", &open_args).is_err());
    open_args.insert(
        "target".to_string(),
        Value::from("https://user:pass@example.com"),
    );
    assert!(validate_computer_tool_args("computer_open_item", &open_args).is_err());
    open_args.insert("target".to_string(), Value::from("https://exa mple.com"));
    assert!(validate_computer_tool_args("computer_open_item", &open_args).is_err());
    open_args.insert("target".to_string(), Value::from("https://[::1"));
    assert!(validate_computer_tool_args("computer_open_item", &open_args).is_err());
    open_args.insert(
        "target".to_string(),
        Value::from("file://remote-host/tmp/example.txt"),
    );
    assert!(validate_computer_tool_args("computer_open_item", &open_args).is_err());
    open_args.insert(
        "target".to_string(),
        Value::from("file:///tmp/example.txt?download=1"),
    );
    assert!(validate_computer_tool_args("computer_open_item", &open_args).is_err());
    open_args.clear();
    open_args.insert("target".to_string(), Value::from("/tmp/example.txt"));
    open_args.insert("reveal".to_string(), Value::Bool(true));
    assert!(validate_computer_tool_args("computer_open_item", &open_args).is_ok());
    open_args.insert("app".to_string(), Value::from("Finder"));
    assert!(validate_computer_tool_args("computer_open_item", &open_args).is_err());
}

#[test]
fn computer_tool_validation_rejects_string_booleans_for_desktop_actions() {
    assert_computer_boolean_arg_is_strict(
        "computer_move_to_element",
        json!({"query": "Open"}),
        "screenshot",
    );
    assert_computer_boolean_arg_is_strict(
        "computer_click_element",
        json!({"query": "Open"}),
        "screenshot",
    );
    assert_computer_boolean_arg_is_strict(
        "computer_press_at",
        json!({"x": 100, "y": 200}),
        "screenshot",
    );
    assert_computer_boolean_arg_is_strict(
        "computer_type_at",
        json!({"x": 100, "y": 200, "text": "hello"}),
        "screenshot",
    );
    assert_computer_boolean_arg_is_strict(
        "computer_scroll_at",
        json!({"x": 100, "y": 200}),
        "screenshot",
    );
    assert_computer_boolean_arg_is_strict(
        "computer_move_to_text",
        json!({"text": "Open"}),
        "screenshot",
    );
    assert_computer_boolean_arg_is_strict(
        "computer_select_text",
        json!({"text": "Open"}),
        "screenshot",
    );
    assert_computer_boolean_arg_is_strict(
        "computer_focus_element",
        json!({"query": "Search"}),
        "screenshot",
    );
    assert_computer_boolean_arg_is_strict(
        "computer_perform_element_action",
        json!({"query": "Menu", "action": "AXShowMenu"}),
        "screenshot",
    );
    assert_computer_boolean_arg_is_strict(
        "computer_click",
        json!({"x": 100, "y": 200}),
        "screenshot",
    );
    assert_computer_boolean_arg_is_strict(
        "computer_drag",
        json!({"start_x": 10, "start_y": 20, "end_x": 30, "end_y": 40}),
        "screenshot",
    );
    assert_computer_boolean_arg_is_strict(
        "computer_drag_element",
        json!({"query": "Item", "target_x": 300, "target_y": 400}),
        "screenshot",
    );
    assert_computer_boolean_arg_is_strict("computer_scroll", json!({}), "screenshot");
    assert_computer_boolean_arg_is_strict(
        "computer_scroll_element",
        json!({"query": "List"}),
        "screenshot",
    );
    assert_computer_boolean_arg_is_strict(
        "computer_copy_selection",
        json!({}),
        "restore_clipboard",
    );
    assert_computer_boolean_arg_is_strict(
        "computer_focus_window",
        json!({"index": 1}),
        "screenshot",
    );
    assert_computer_boolean_arg_is_strict(
        "computer_set_window_bounds",
        json!({"index": 1, "x": 0, "y": 24, "width": 900, "height": 700}),
        "screenshot",
    );
    assert_computer_boolean_arg_is_strict(
        "computer_perform_window_action",
        json!({"index": 1, "action": "minimize"}),
        "screenshot",
    );
    assert_computer_boolean_arg_is_strict(
        "computer_close_window",
        json!({"index": 1}),
        "screenshot",
    );
    assert_computer_boolean_arg_is_strict(
        "computer_select_menu",
        json!({"app": "Safari", "menu_path": ["File", "New Window"]}),
        "screenshot",
    );
    assert_computer_boolean_arg_is_strict(
        "computer_activate_app",
        json!({"app": "Safari"}),
        "screenshot",
    );
}

#[test]
fn computer_helper_status_smoke_test() {
    let has_python = python_candidates()
        .into_iter()
        .any(|python| Command::new(python).arg("--version").output().is_ok());
    if !has_python {
        return;
    }

    let root = temp_root("computer-helper");
    let outcome = run_python_once(
        COMPUTER_HELPER,
        &json!({
            "action": "computer_status",
            "args": {},
            "state_dir": root.display().to_string(),
        }),
        5,
        &RuntimeExecutionControl::default(),
    )
    .expect("computer helper should return JSON");

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("Platform:"));
    assert!(outcome.text.contains("Accessibility permission:"));
    assert!(outcome.text.contains("Screen recording permission:"));
    assert!(outcome.text.contains("observe"));
    assert!(outcome.text.contains("accessibility_snapshot"));
    assert!(outcome.text.contains("read_text"));
    assert!(outcome.text.contains("find_text"));
    assert!(outcome.text.contains("move_to_text"));
    assert!(outcome.text.contains("click_text"));
    assert!(outcome.text.contains("assert_text"));
    assert!(outcome.text.contains("wait_for_text"));
    assert!(outcome.text.contains("click_element"));
    assert!(outcome.text.contains("element_at"));
    assert!(outcome.text.contains("press_at"));
    assert!(outcome.text.contains("type_at"));
    assert!(outcome.text.contains("scroll_at"));
    assert!(outcome.text.contains("type_element"));
    assert!(outcome.text.contains("assert_element"));
    assert!(outcome.text.contains("wait_for_element"));
    assert!(outcome.text.contains("mouse_down"));
    assert!(outcome.text.contains("mouse_up"));
    assert!(outcome.text.contains("hotkey"));
    assert!(outcome.text.contains("key_down"));
    assert!(outcome.text.contains("key_up"));
    assert!(outcome.text.contains("get_clipboard"));
    assert!(outcome.text.contains("set_clipboard"));
    assert!(outcome.text.contains("copy_selection"));
    assert!(outcome.text.contains("assert_clipboard"));
    assert!(outcome.text.contains("wait_for_clipboard"));
    drop(outcome);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn computer_status_production_dispatch_reports_real_permission_snapshot() {
    let has_python = python_candidates()
        .into_iter()
        .any(|python| Command::new(python).arg("--version").output().is_ok());
    if !has_python {
        return;
    }
    let root = temp_root("computer-status-production");
    let ctx = ToolContext {
        memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
        language_services: Default::default(),
        skills: Default::default(),
        allowed_dirs: vec![root.clone()],
        policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), true, "allow".to_string(), Vec::new()),
        permission_scope: Default::default(),
        cancellation: Default::default(),
        plan_state: Default::default(),
        output_store: crate::tools::ToolOutputStore::for_test(),
        browser: BrowserRuntime::new(root.join("browser-state")),
    };

    let outcome = run_computer_tool("computer_status", &Map::new(), &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("Accessibility permission:"));
    assert!(outcome.text.contains("Screen recording permission:"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn computer_helper_uses_native_click_state_for_multi_clicks() {
    assert!(COMPUTER_HELPER.contains("CG_MOUSE_EVENT_CLICK_STATE_FIELD = 1"));
    assert!(COMPUTER_HELPER
        .contains("def post_mouse(cg, event_type, point, button, flags=0, click_count=0):"));
    assert!(COMPUTER_HELPER.contains("CGEventSetIntegerValueField"));
    assert!(COMPUTER_HELPER.contains("for click_index in range(1, clicks + 1):"));
    assert!(COMPUTER_HELPER.contains("click_count=click_index"));
    assert!(COMPUTER_HELPER.contains("click_count=1"));
}

#[test]
fn computer_helper_reports_macos_permission_state() {
    assert!(COMPUTER_HELPER.contains("AXIsProcessTrustedWithOptions"));
    assert!(COMPUTER_HELPER.contains("CGPreflightScreenCaptureAccess"));
    assert!(COMPUTER_HELPER.contains("Accessibility permission:"));
    assert!(COMPUTER_HELPER.contains("Screen recording permission:"));
    assert!(COMPUTER_HELPER.contains("accessibility_permission"));
    assert!(COMPUTER_HELPER.contains("screen_recording_permission"));
    assert!(COMPUTER_HELPER.contains("Accessibility permission is required"));
    assert!(COMPUTER_HELPER.contains("Automation permission is required"));
    assert!(COMPUTER_HELPER.contains("Screen Recording permission is required"));
}

#[test]
fn run_python_once_retries_empty_success_output() {
    let has_python = python_candidates()
        .into_iter()
        .any(|python| Command::new(python).arg("--version").output().is_ok());
    if !has_python {
        return;
    }

    let root = temp_root("python-empty-retry");
    let script = r#"
import json
import pathlib
import sys

payload = json.loads(sys.stdin.read())
root = pathlib.Path(payload["state_dir"])
root.mkdir(parents=True, exist_ok=True)
marker = root / "first"
if not marker.exists():
    marker.write_text("seen", encoding="utf-8")
    sys.exit(0)

print(json.dumps({"ok": True, "text": "retry worked", "display": "retry ok"}))
"#;
    let outcome = run_python_once(
        script,
        &json!({"state_dir": root.display().to_string()}),
        5,
        &RuntimeExecutionControl::default(),
    )
    .expect("helper should retry empty stdout once");

    assert!(!outcome.error, "{}", outcome.text);
    assert_eq!(outcome.text, "retry worked");
    assert_eq!(outcome.display, "retry ok");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn computer_helper_wait_without_screenshot_smoke_test() {
    let has_python = python_candidates()
        .into_iter()
        .any(|python| Command::new(python).arg("--version").output().is_ok());
    if !has_python {
        return;
    }

    let root = temp_root("computer-helper-wait");
    let outcome = run_python_once(
        COMPUTER_HELPER,
        &json!({
            "action": "computer_wait",
            "args": {"duration_ms": 0, "screenshot": false},
            "state_dir": root.display().to_string(),
        }),
        5,
        &RuntimeExecutionControl::default(),
    )
    .expect("computer helper should return JSON");

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("Computer action: wait"));
    assert!(outcome.text.contains("Waited ms: 0"));
    drop(outcome);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn approval_summary_redacts_sensitive_tool_args() {
    let mut args = Map::new();
    args.insert(
        "text".to_string(),
        Value::String("super-secret-password".to_string()),
    );
    args.insert(
        "selector".to_string(),
        Value::String("#password".to_string()),
    );
    args.insert(
        "api_token".to_string(),
        Value::String("tok_123".to_string()),
    );
    let request = ToolRequest {
        id: "tool-1".to_string(),
        name: "chrome_native_type_text".to_string(),
        args,
    };

    let sanitized = sanitized_approval_args(&request);
    let serialized = Value::Object(sanitized).to_string();

    assert!(!approval_summary(&request)[0].trim().is_empty());
    assert!(serialized.contains("[redacted: 21 chars]"));
    assert!(serialized.contains("[redacted: 7 chars]"));
    assert!(!serialized.contains("super-secret-password"));
    assert!(!serialized.contains("tok_123"));
}

#[test]
fn tool_confirmation_prompt_includes_question_summary_and_redacted_args() {
    let mut args = Map::new();
    args.insert(
        "text".to_string(),
        Value::String("super-secret-password".to_string()),
    );
    args.insert(
        "selector".to_string(),
        Value::String("#password".to_string()),
    );
    let request = ToolRequest {
        id: "tool-confirmation".to_string(),
        name: "chrome_native_type_text".to_string(),
        args,
    };

    let prompt = tool_confirmation_prompt(&request);

    assert!(prompt.contains(&crate::i18n::localize_tool_display_name(&request.name)));
    assert!(!prompt.contains("chrome_native_type_text"));
    // Harness exports approval data; numeric choices belong to the CLI view.
    let contract = tool_permission_prompt(&request);
    assert_eq!(prompt, permission_summary(&contract));
    assert!(!contract.rows.is_empty());
    assert!(prompt.contains("[redacted:"));
    // The adaptive permission panel may wrap between the count and unit on a
    // narrow terminal. Verify the complete redaction metadata without tying
    // the security contract to one physical terminal line.
    assert!(prompt.contains("21"));
    assert!(prompt.contains("chars]"));
    assert!(!prompt.contains("super-secret-password"));
}

#[test]
fn approval_summary_redacts_multi_edit_args() {
    let mut args = Map::new();
    args.insert("path".to_string(), Value::String("secret.txt".to_string()));
    args.insert(
        "edits".to_string(),
        json!([
            {"old_text": "api_key = \"secret\"", "new_text": "api_key = \"redacted\""}
        ]),
    );
    let request = ToolRequest {
        id: "tool-3".to_string(),
        name: "multi_edit_file".to_string(),
        args,
    };

    let sanitized = Value::Object(sanitized_approval_args(&request)).to_string();

    assert_eq!(
        approval_summary(&request)[0],
        "multi-edit file: secret.txt (1 edits)"
    );
    assert!(sanitized.contains("[redacted: 18 chars]"));
    assert!(sanitized.contains("[redacted: 20 chars]"));
    assert!(!sanitized.contains("api_key"));
}

#[test]
fn approval_summary_describes_desktop_coordinates() {
    let mut args = Map::new();
    args.insert("start_x".to_string(), Value::from(-1200));
    args.insert("start_y".to_string(), Value::from(50));
    args.insert("end_x".to_string(), Value::from(400));
    args.insert("end_y".to_string(), Value::from(800));
    let request = ToolRequest {
        id: "tool-2".to_string(),
        name: "computer_drag".to_string(),
        args,
    };

    assert_eq!(
        approval_summary(&request)[0],
        "desktop drag: -1200,50 -> 400,800"
    );

    let mut modified_args = Map::new();
    modified_args.insert("x".to_string(), Value::from(40));
    modified_args.insert("y".to_string(), Value::from(60));
    modified_args.insert("modifiers".to_string(), json!(["Command", "Shift"]));
    let modified_request = ToolRequest {
        id: "tool-2b".to_string(),
        name: "computer_click".to_string(),
        args: modified_args,
    };

    assert_eq!(
        approval_summary(&modified_request)[0],
        "desktop click: 40,60 (left) modifiers=[Command,Shift]"
    );

    let mut element_at_args = Map::new();
    element_at_args.insert("x".to_string(), Value::from(40));
    element_at_args.insert("y".to_string(), Value::from(60));
    let element_at_request = ToolRequest {
        id: "tool-2element-at".to_string(),
        name: "computer_element_at".to_string(),
        args: element_at_args,
    };

    assert_eq!(
        approval_summary(&element_at_request)[0],
        "desktop element at: 40,60"
    );

    let mut press_at_args = Map::new();
    press_at_args.insert("x".to_string(), Value::from(40));
    press_at_args.insert("y".to_string(), Value::from(60));
    let press_at_request = ToolRequest {
        id: "tool-2press-at".to_string(),
        name: "computer_press_at".to_string(),
        args: press_at_args,
    };

    assert_eq!(
        approval_summary(&press_at_request)[0],
        "desktop press at: 40,60"
    );

    let mut type_at_args = Map::new();
    type_at_args.insert("x".to_string(), Value::from(40));
    type_at_args.insert("y".to_string(), Value::from(60));
    type_at_args.insert("text".to_string(), Value::String("hello".to_string()));
    let type_at_request = ToolRequest {
        id: "tool-2type-at".to_string(),
        name: "computer_type_at".to_string(),
        args: type_at_args,
    };

    assert_eq!(
        approval_summary(&type_at_request)[0],
        "desktop type at: 40,60 (5 chars)"
    );

    let mut scroll_at_args = Map::new();
    scroll_at_args.insert("x".to_string(), Value::from(40));
    scroll_at_args.insert("y".to_string(), Value::from(60));
    scroll_at_args.insert("direction".to_string(), Value::String("down".to_string()));
    scroll_at_args.insert("amount".to_string(), Value::from(300));
    scroll_at_args.insert("modifiers".to_string(), json!(["Shift"]));
    let scroll_at_request = ToolRequest {
        id: "tool-2scroll-at".to_string(),
        name: "computer_scroll_at".to_string(),
        args: scroll_at_args,
    };

    assert_eq!(
        approval_summary(&scroll_at_request)[0],
        "desktop scroll at: 40,60 down 300px modifiers=[Shift]"
    );

    let mut mouse_down_args = Map::new();
    mouse_down_args.insert("button".to_string(), Value::String("left".to_string()));
    mouse_down_args.insert("modifiers".to_string(), json!(["Shift"]));
    let mouse_down_request = ToolRequest {
        id: "tool-2down".to_string(),
        name: "computer_mouse_down".to_string(),
        args: mouse_down_args,
    };

    assert_eq!(
        approval_summary(&mouse_down_request)[0],
        "desktop mouse down: current position (left) modifiers=[Shift]"
    );

    let mut mouse_up_args = Map::new();
    mouse_up_args.insert("x".to_string(), Value::from(70));
    mouse_up_args.insert("y".to_string(), Value::from(90));
    let mouse_up_request = ToolRequest {
        id: "tool-2up".to_string(),
        name: "computer_mouse_up".to_string(),
        args: mouse_up_args,
    };

    assert_eq!(
        approval_summary(&mouse_up_request)[0],
        "desktop mouse up: 70,90 (left)"
    );

    let mut key_down_args = Map::new();
    key_down_args.insert("key".to_string(), Value::String("Space".to_string()));
    let key_down_request = ToolRequest {
        id: "tool-2keydown".to_string(),
        name: "computer_key_down".to_string(),
        args: key_down_args,
    };

    assert_eq!(
        approval_summary(&key_down_request)[0],
        "desktop key down: Space"
    );

    let mut hotkey_args = Map::new();
    hotkey_args.insert("key".to_string(), Value::String("S".to_string()));
    hotkey_args.insert("modifiers".to_string(), json!(["Command", "Shift"]));
    let hotkey_request = ToolRequest {
        id: "tool-2hotkey".to_string(),
        name: "computer_hotkey".to_string(),
        args: hotkey_args,
    };

    assert_eq!(
        approval_summary(&hotkey_request)[0],
        "desktop hotkey: Command+Shift+S"
    );

    let mut key_up_args = Map::new();
    key_up_args.insert("key".to_string(), Value::String("Space".to_string()));
    let key_up_request = ToolRequest {
        id: "tool-2keyup".to_string(),
        name: "computer_key_up".to_string(),
        args: key_up_args,
    };

    assert_eq!(
        approval_summary(&key_up_request)[0],
        "desktop key up: Space"
    );

    let mut observe_args = Map::new();
    observe_args.insert("include_screenshot".to_string(), Value::Bool(false));
    observe_args.insert("include_apps".to_string(), Value::Bool(true));
    let observe_request = ToolRequest {
        id: "tool-2observe".to_string(),
        name: "computer_observe".to_string(),
        args: observe_args,
    };

    assert_eq!(
        approval_summary(&observe_request)[0],
        "desktop observe: screenshot=false accessibility=true windows=true apps=true text=true"
    );

    let mut read_text_args = Map::new();
    read_text_args.insert("include_ocr".to_string(), Value::Bool(false));
    read_text_args.insert("max_results".to_string(), Value::from(10));
    let read_text_request = ToolRequest {
        id: "tool-2read".to_string(),
        name: "computer_read_text".to_string(),
        args: read_text_args,
    };

    assert_eq!(
        approval_summary(&read_text_request)[0],
        "desktop read visible text: accessibility=true ocr=false max=10"
    );

    let mut find_text_args = Map::new();
    find_text_args.insert("text".to_string(), Value::String("Open".to_string()));
    find_text_args.insert("include_ocr".to_string(), Value::Bool(false));
    let find_text_request = ToolRequest {
        id: "tool-2find".to_string(),
        name: "computer_find_text".to_string(),
        args: find_text_args,
    };

    assert_eq!(
        approval_summary(&find_text_request)[0],
        "desktop find text: 'Open' match_mode=contains ocr=false"
    );

    let mut move_text_args = Map::new();
    move_text_args.insert("text".to_string(), Value::String("Open".to_string()));
    move_text_args.insert("source".to_string(), Value::String("ocr".to_string()));
    move_text_args.insert("occurrence".to_string(), Value::from(2));
    let move_text_request = ToolRequest {
        id: "tool-2movetext".to_string(),
        name: "computer_move_to_text".to_string(),
        args: move_text_args,
    };

    assert_eq!(
        approval_summary(&move_text_request)[0],
        "desktop move to text: 'Open' source=ocr occurrence=2"
    );

    let mut click_text_args = Map::new();
    click_text_args.insert("text".to_string(), Value::String("Open".to_string()));
    click_text_args.insert("source".to_string(), Value::String("ocr".to_string()));
    click_text_args.insert("occurrence".to_string(), Value::from(2));
    click_text_args.insert("modifiers".to_string(), json!(["Command"]));
    let click_text_request = ToolRequest {
        id: "tool-2clicktext".to_string(),
        name: "computer_click_text".to_string(),
        args: click_text_args,
    };

    assert_eq!(
        approval_summary(&click_text_request)[0],
        "desktop click text: 'Open' source=ocr occurrence=2 modifiers=[Command]"
    );

    let mut assert_text_args = Map::new();
    assert_text_args.insert("text".to_string(), Value::String("Saved".to_string()));
    assert_text_args.insert("exists".to_string(), Value::Bool(false));
    let assert_text_request = ToolRequest {
        id: "tool-2asserttext".to_string(),
        name: "computer_assert_text".to_string(),
        args: assert_text_args,
    };

    assert_eq!(
        approval_summary(&assert_text_request)[0],
        "desktop assert text: 'Saved' exists=false match_mode=contains"
    );

    let mut wait_text_args = Map::new();
    wait_text_args.insert("text".to_string(), Value::String("Loaded".to_string()));
    wait_text_args.insert("timeout_ms".to_string(), Value::from(2500));
    let wait_text_request = ToolRequest {
        id: "tool-2waittext".to_string(),
        name: "computer_wait_for_text".to_string(),
        args: wait_text_args,
    };

    assert_eq!(
        approval_summary(&wait_text_request)[0],
        "desktop wait for text: 'Loaded' exists=true timeout=2500ms"
    );

    let mut scroll_args = Map::new();
    scroll_args.insert("direction".to_string(), Value::String("down".to_string()));
    scroll_args.insert("amount".to_string(), Value::from(300));
    scroll_args.insert("x".to_string(), Value::from(500));
    scroll_args.insert("y".to_string(), Value::from(600));
    scroll_args.insert("modifiers".to_string(), json!(["Option"]));
    let scroll_request = ToolRequest {
        id: "tool-2c".to_string(),
        name: "computer_scroll".to_string(),
        args: scroll_args,
    };

    assert_eq!(
        approval_summary(&scroll_request)[0],
        "desktop scroll: down 300 at 500,600 modifiers=[Option]"
    );

    let mut drag_element_args = Map::new();
    drag_element_args.insert("query".to_string(), Value::String("Item".to_string()));
    drag_element_args.insert(
        "target_query".to_string(),
        Value::String("Drop".to_string()),
    );
    drag_element_args.insert("modifiers".to_string(), json!(["Shift"]));
    let drag_element_request = ToolRequest {
        id: "tool-2d".to_string(),
        name: "computer_drag_element".to_string(),
        args: drag_element_args,
    };

    assert_eq!(
        approval_summary(&drag_element_request)[0],
        "desktop drag element: target='Item' -> Drop modifiers=[Shift]"
    );

    let mut window_action_args = Map::new();
    window_action_args.insert("title".to_string(), Value::String("Nola".to_string()));
    window_action_args.insert("action".to_string(), Value::String("minimize".to_string()));
    let window_action_request = ToolRequest {
        id: "tool-2e".to_string(),
        name: "computer_perform_window_action".to_string(),
        args: window_action_args,
    };

    assert_eq!(
        approval_summary(&window_action_request)[0],
        "desktop perform window action: action=minimize target=Nola"
    );
}

#[test]
fn approval_summary_describes_agent_plan() {
    let mut args = Map::new();
    args.insert(
        "title".to_string(),
        Value::String("Browser Automation".to_string()),
    );
    args.insert("steps".to_string(), json!(["Inspect", "Verify"]));
    let request = ToolRequest {
        id: "tool-4".to_string(),
        name: "propose_agent_plan".to_string(),
        args,
    };

    assert_eq!(
        approval_summary(&request)[0],
        "approve plan: Browser Automation (2 steps)"
    );
}

#[test]
fn approval_summary_describes_browser_conditions() {
    let mut click_args = Map::new();
    click_args.insert("text".to_string(), Value::String("Open".to_string()));
    click_args.insert("clicks".to_string(), Value::from(2));
    click_args.insert("modifiers".to_string(), json!(["Command", "Shift"]));
    let click_request = ToolRequest {
        id: "tool-browser-click".to_string(),
        name: "chrome_native_click".to_string(),
        args: click_args,
    };
    assert_eq!(
        approval_summary(&click_request)[0],
        "browser click: text Open 2x modifiers=[Command,Shift]"
    );

    let mut drag_args = Map::new();
    drag_args.insert(
        "source_text".to_string(),
        Value::String("Backlog".to_string()),
    );
    drag_args.insert("target_text".to_string(), Value::String("Done".to_string()));
    drag_args.insert("modifiers".to_string(), json!(["Shift"]));
    let drag_request = ToolRequest {
        id: "tool-browser-drag".to_string(),
        name: "browser_drag".to_string(),
        args: drag_args,
    };
    assert_eq!(
        approval_summary(&drag_request)[0],
        "browser drag: source text Backlog -> target text Done modifiers=[Shift]"
    );

    let mut assert_args = Map::new();
    assert_args.insert("text".to_string(), Value::String("Saved".to_string()));
    assert_args.insert("exists".to_string(), Value::Bool(false));
    let assert_request = ToolRequest {
        id: "tool-browser-assert".to_string(),
        name: "chrome_native_assert".to_string(),
        args: assert_args,
    };
    assert_eq!(
        approval_summary(&assert_request)[0],
        "browser assert: text='Saved' exists=false"
    );

    let mut wait_args = Map::new();
    wait_args.insert("selector".to_string(), Value::String("#toast".to_string()));
    wait_args.insert("timeout_ms".to_string(), Value::from(2500));
    let wait_request = ToolRequest {
        id: "tool-browser-wait".to_string(),
        name: "chrome_native_wait_for".to_string(),
        args: wait_args,
    };
    assert_eq!(
        approval_summary(&wait_request)[0],
        "browser wait for: selector='#toast' exists=true timeout=2500ms"
    );
}

#[test]
fn approval_summary_describes_computer_wait() {
    let mut args = Map::new();
    args.insert("duration_ms".to_string(), Value::from(1500));
    args.insert("screenshot".to_string(), Value::Bool(false));
    let request = ToolRequest {
        id: "tool-5".to_string(),
        name: "computer_wait".to_string(),
        args,
    };

    assert_eq!(
        approval_summary(&request)[0],
        "desktop wait: 1500ms (stable=false screenshot=false)"
    );
    let mut stable_request = request;
    stable_request
        .args
        .insert("until_stable".to_string(), Value::Bool(true));
    assert_eq!(
        approval_summary(&stable_request)[0],
        "desktop wait: 1500ms (stable=true screenshot=false)"
    );
}

#[test]
fn approval_summary_describes_accessibility_snapshot() {
    let mut args = Map::new();
    args.insert("max_elements".to_string(), Value::from(25));
    args.insert("include_values".to_string(), Value::Bool(true));
    let request = ToolRequest {
        id: "tool-6".to_string(),
        name: "computer_accessibility_snapshot".to_string(),
        args,
    };

    assert_eq!(
        approval_summary(&request)[0],
        "desktop accessibility snapshot: up to 25 elements (values=true)"
    );
}

#[test]
fn approval_summary_describes_click_element() {
    let mut args = Map::new();
    args.insert("query".to_string(), Value::String("Open".to_string()));
    args.insert("role".to_string(), Value::String("AXButton".to_string()));
    args.insert("occurrence".to_string(), Value::from(2));
    let request = ToolRequest {
        id: "tool-7".to_string(),
        name: "computer_click_element".to_string(),
        args,
    };

    assert_eq!(
        approval_summary(&request)[0],
        "desktop click element: target='Open' role='AXButton' occurrence=2"
    );
}

#[test]
fn approval_summary_redacts_type_element_text() {
    let mut args = Map::new();
    args.insert("query".to_string(), Value::String("Search".to_string()));
    args.insert("role".to_string(), Value::String("AXTextField".to_string()));
    args.insert(
        "text".to_string(),
        Value::String("private text".to_string()),
    );
    let request = ToolRequest {
        id: "tool-8".to_string(),
        name: "computer_type_element".to_string(),
        args,
    };

    assert_eq!(
        approval_summary(&request)[0],
        "desktop type element: target='Search' role='AXTextField' (12 chars)"
    );
    let sanitized = Value::Object(sanitized_approval_args(&request)).to_string();
    assert!(sanitized.contains("[redacted: 12 chars]"));
    assert!(!sanitized.contains("private text"));
}

#[test]
fn approval_summary_describes_perform_element_action() {
    let mut args = Map::new();
    args.insert("query".to_string(), Value::String("Menu".to_string()));
    args.insert(
        "role".to_string(),
        Value::String("AXPopUpButton".to_string()),
    );
    args.insert(
        "action".to_string(),
        Value::String("AXShowMenu".to_string()),
    );
    let request = ToolRequest {
        id: "tool-8a".to_string(),
        name: "computer_perform_element_action".to_string(),
        args,
    };

    assert_eq!(
            approval_summary(&request)[0],
            "desktop perform element action: action=AXShowMenu target='Menu' role='AXPopUpButton' occurrence=?"
        );
}

#[test]
fn approval_summary_describes_focus_element() {
    let mut args = Map::new();
    args.insert("query".to_string(), Value::String("Search".to_string()));
    args.insert("role".to_string(), Value::String("AXTextField".to_string()));
    args.insert("occurrence".to_string(), Value::from(2));
    let request = ToolRequest {
        id: "tool-8focus".to_string(),
        name: "computer_focus_element".to_string(),
        args,
    };

    assert_eq!(
        approval_summary(&request)[0],
        "desktop focus element: target='Search' role='AXTextField' occurrence=2"
    );
}

#[test]
fn approval_summary_redacts_set_element_value() {
    let mut args = Map::new();
    args.insert("query".to_string(), Value::String("Search".to_string()));
    args.insert("role".to_string(), Value::String("AXTextField".to_string()));
    args.insert(
        "value".to_string(),
        Value::String("private value".to_string()),
    );
    let request = ToolRequest {
        id: "tool-8b".to_string(),
        name: "computer_set_element_value".to_string(),
        args,
    };

    assert_eq!(
        approval_summary(&request)[0],
        "desktop set element value: target='Search' role='AXTextField' (13 chars)"
    );
    let sanitized = Value::Object(sanitized_approval_args(&request)).to_string();
    assert!(sanitized.contains("[redacted: 13 chars]"));
    assert!(!sanitized.contains("private value"));
}

#[test]
fn approval_summary_describes_assert_element() {
    let mut args = Map::new();
    args.insert("query".to_string(), Value::String("Saved".to_string()));
    args.insert(
        "role".to_string(),
        Value::String("AXStaticText".to_string()),
    );
    args.insert("exists".to_string(), Value::Bool(false));
    let request = ToolRequest {
        id: "tool-9".to_string(),
        name: "computer_assert_element".to_string(),
        args,
    };

    assert_eq!(
        approval_summary(&request)[0],
        "desktop assert element: target='Saved' role='AXStaticText' exists=false"
    );
}

#[test]
fn approval_summary_describes_wait_for_element() {
    let mut args = Map::new();
    args.insert("query".to_string(), Value::String("Loaded".to_string()));
    args.insert("role".to_string(), Value::String("AXButton".to_string()));
    args.insert("exists".to_string(), Value::Bool(true));
    args.insert("timeout_ms".to_string(), Value::from(2500));
    let request = ToolRequest {
        id: "tool-10".to_string(),
        name: "computer_wait_for_element".to_string(),
        args,
    };

    assert_eq!(
        approval_summary(&request)[0],
        "desktop wait for element: target='Loaded' role='AXButton' exists=true timeout=2500ms"
    );
}

#[test]
fn approval_summary_describes_clipboard_tools() {
    let mut get_args = Map::new();
    get_args.insert("max_chars".to_string(), Value::from(1200));
    let get_request = ToolRequest {
        id: "tool-clipboard-get".to_string(),
        name: "computer_get_clipboard".to_string(),
        args: get_args,
    };
    assert_eq!(
        approval_summary(&get_request)[0],
        "desktop read clipboard: up to 1200 chars"
    );

    let mut set_args = Map::new();
    set_args.insert("text".to_string(), Value::String("secret clip".to_string()));
    let set_request = ToolRequest {
        id: "tool-clipboard-set".to_string(),
        name: "computer_set_clipboard".to_string(),
        args: set_args,
    };
    assert_eq!(
        approval_summary(&set_request)[0],
        "desktop set clipboard: 11 chars"
    );
    let sanitized = Value::Object(sanitized_approval_args(&set_request)).to_string();
    assert!(sanitized.contains("[redacted: 11 chars]"));
    assert!(!sanitized.contains("secret clip"));

    let mut copy_args = Map::new();
    copy_args.insert("max_chars".to_string(), Value::from(800));
    copy_args.insert("restore_clipboard".to_string(), Value::Bool(false));
    let copy_request = ToolRequest {
        id: "tool-clipboard-copy".to_string(),
        name: "computer_copy_selection".to_string(),
        args: copy_args,
    };
    assert_eq!(
        approval_summary(&copy_request)[0],
        "desktop copy selected text: up to 800 chars (restore_clipboard=false)"
    );

    let mut assert_args = Map::new();
    assert_args.insert("text".to_string(), Value::String("done".to_string()));
    assert_args.insert("exists".to_string(), Value::Bool(false));
    let assert_request = ToolRequest {
        id: "tool-clipboard-assert".to_string(),
        name: "computer_assert_clipboard".to_string(),
        args: assert_args,
    };
    assert_eq!(
        approval_summary(&assert_request)[0],
        "desktop assert clipboard: 4 chars exists=false match_mode=contains"
    );
    let sanitized_assert = Value::Object(sanitized_approval_args(&assert_request)).to_string();
    assert!(sanitized_assert.contains("[redacted: 4 chars]"));
    assert!(!sanitized_assert.contains("done"));

    let mut wait_args = Map::new();
    wait_args.insert("text".to_string(), Value::String("ready".to_string()));
    wait_args.insert("timeout_ms".to_string(), Value::from(1500));
    let wait_request = ToolRequest {
        id: "tool-clipboard-wait".to_string(),
        name: "computer_wait_for_clipboard".to_string(),
        args: wait_args,
    };
    assert_eq!(
        approval_summary(&wait_request)[0],
        "desktop wait for clipboard: 5 chars exists=true timeout=1500ms"
    );
}

fn temp_root(name: &str) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let nonce = TEMP_ROOT_COUNTER.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "nola-cli-test-{name}-{}-{stamp}-{nonce}",
        process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root.canonicalize().unwrap()
}
