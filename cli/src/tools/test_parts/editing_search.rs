#[test]
fn find_up_locates_nearest_manifest_from_deep_file() {
    let root = temp_root("find-up");
    let source_dir = root.join("packages").join("app").join("src");
    fs::create_dir_all(&source_dir).unwrap();
    let package = root.join("package.json");
    let agents = root.join("AGENTS.md");
    let target = source_dir.join("main.ts");
    fs::write(&package, "{\"scripts\":{\"test\":\"vitest\"}}\n").unwrap();
    fs::write(&agents, "instructions\n").unwrap();
    fs::write(&target, "export const value = 1\n").unwrap();
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
    let mut args = Map::new();
    args.insert(
        "start".to_string(),
        Value::String(target.display().to_string()),
    );
    args.insert(
        "patterns".to_string(),
        Value::Array(vec![
            Value::String("package.json".to_string()),
            Value::String("AGENTS.md".to_string()),
        ]),
    );

    let outcome = find_up(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains(&package.display().to_string()));
    assert!(outcome.text.contains(&agents.display().to_string()));
    assert!(outcome.text.contains("scanned_dirs: 4"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn validation_plan_suggests_related_tests_and_project_commands() {
    let root = temp_root("validation-plan");
    fs::write(root.join("pyproject.toml"), "[tool.pytest.ini_options]\n").unwrap();
    let src = root.join("src");
    let tests = root.join("tests");
    fs::create_dir_all(&src).unwrap();
    fs::create_dir_all(&tests).unwrap();
    let target = src.join("user_service.py");
    let test_file = tests.join("test_user_service.py");
    fs::write(&target, "def load_user():\n    pass\n").unwrap();
    fs::write(&test_file, "from src.user_service import load_user\n").unwrap();
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
    let mut args = Map::new();
    args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );
    args.insert(
        "paths".to_string(),
        Value::Array(vec![Value::String(target.display().to_string())]),
    );

    let outcome = validation_plan(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains(&format!("- {}", test_file.display())));
    assert!(outcome
        .text
        .contains("-m pytest tests/test_user_service.py"));
    assert!(outcome.text.contains("-m pytest"));
    assert!(outcome.text.contains("code_diagnostics root="));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn validation_plan_uses_git_changed_files_when_paths_missing() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }

    let root = temp_root("validation-plan-git");
    fs::write(root.join("pyproject.toml"), "[tool.pytest.ini_options]\n").unwrap();
    let src = root.join("src");
    let tests = root.join("tests");
    fs::create_dir_all(&src).unwrap();
    fs::create_dir_all(&tests).unwrap();
    let target = src.join("user_service.py");
    let test_file = tests.join("test_user_service.py");
    fs::write(&target, "def load_user():\n    return None\n").unwrap();
    fs::write(&test_file, "from src.user_service import load_user\n").unwrap();
    assert!(Command::new("git")
        .arg("init")
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("git")
        .arg("add")
        .arg(".")
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("git")
        .args([
            "-c",
            "user.name=Nola",
            "-c",
            "user.email=nola@example.com",
            "commit",
            "-m",
            "init",
        ])
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    fs::write(&target, "def load_user():\n    return {'id': 1}\n").unwrap();
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
    let mut args = Map::new();
    args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );

    let outcome = validation_plan(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("## Changed/target files"));
    assert!(outcome.text.contains(&format!("- {}", target.display())));
    assert!(outcome.text.contains(&format!("- {}", test_file.display())));
    assert!(outcome
        .text
        .contains("-m pytest tests/test_user_service.py"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn test_context_summarizes_frameworks_related_tests_and_commands() {
    let root = temp_root("test-context");
    fs::write(root.join("pyproject.toml"), "[tool.pytest.ini_options]\n").unwrap();
    let src = root.join("src");
    let tests = root.join("tests");
    fs::create_dir_all(&src).unwrap();
    fs::create_dir_all(&tests).unwrap();
    let target = src.join("user_service.py");
    let test_file = tests.join("test_user_service.py");
    fs::write(&target, "def load_user():\n    return {'id': 1}\n").unwrap();
    fs::write(
            &test_file,
            "import pytest\n\nfrom src.user_service import load_user\n\n\ndef test_load_user():\n    assert load_user()['id'] == 1\n",
        )
        .unwrap();
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
    let mut args = Map::new();
    args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );
    args.insert(
        "paths".to_string(),
        Value::Array(vec![Value::String("src/user_service.py".to_string())]),
    );
    args.insert("max_tests".to_string(), Value::Number(4.into()));
    args.insert("max_symbols_per_test".to_string(), Value::Number(10.into()));

    let outcome = test_context(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("pytest via project config"));
    assert!(outcome.text.contains("- tests/test_user_service.py"));
    assert!(outcome.text.contains("### tests/test_user_service.py"));
    assert!(outcome.text.contains("function test_load_user"));
    assert!(outcome.text.contains("import pytest"));
    assert!(outcome
        .text
        .contains("-m pytest tests/test_user_service.py"));
    assert!(outcome.text.contains("-m pytest"));
    assert!(outcome.display.contains("1 tests"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn format_plan_suggests_language_specific_commands() {
    let root = temp_root("format-plan");
    let src = root.join("src");
    fs::create_dir_all(&src).unwrap();
    let ts_file = src.join("app.ts");
    let py_file = src.join("service.py");
    let rs_file = src.join("lib.rs");
    fs::write(&ts_file, "export const app = 1\n").unwrap();
    fs::write(&py_file, "def service():\n    return 1\n").unwrap();
    fs::write(&rs_file, "pub fn service() -> i32 { 1 }\n").unwrap();
    fs::write(
        root.join("package.json"),
        r#"{"scripts":{"format":"prettier --write .","lint":"eslint ."}}"#,
    )
    .unwrap();
    fs::write(root.join(".prettierrc"), "{}\n").unwrap();
    fs::write(root.join("pyproject.toml"), "[tool.ruff]\n").unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
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
    let mut args = Map::new();
    args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );
    args.insert(
        "paths".to_string(),
        Value::Array(vec![
            Value::String(ts_file.display().to_string()),
            Value::String(py_file.display().to_string()),
            Value::String(rs_file.display().to_string()),
        ]),
    );

    let outcome = format_plan(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("- npm run format"));
    assert!(outcome.text.contains("- npm run lint"));
    assert!(outcome.text.contains("- npx prettier --write src/app.ts"));
    assert!(outcome.text.contains("-m ruff format src/service.py"));
    assert!(outcome.text.contains("-m ruff check src/service.py"));
    assert!(outcome.text.contains("- cargo fmt"));
    assert!(!outcome.text.contains("-m ruff format src/app.ts"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn format_files_runs_inferred_formatter_commands() {
    let root = temp_root("format-files");
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    let src = root.join("src");
    fs::create_dir_all(&src).unwrap();
    let target = src.join("main.rs");
    fs::write(&target, "fn main(){println!(\"hi\");}\n").unwrap();
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
    let mut args = Map::new();
    args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );
    args.insert(
        "paths".to_string(),
        Value::Array(vec![Value::String(target.display().to_string())]),
    );
    args.insert("dry_run".to_string(), Value::Bool(false));

    let outcome = format_files(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("cargo fmt"));
    assert!(outcome.text.contains("## Results"));
    assert_eq!(
        fs::read_to_string(&target).unwrap(),
        "fn main() {\n    println!(\"hi\");\n}\n"
    );
    assert!(outcome.display.contains("Formatted files"));
    assert!(outcome.text.contains("## Commands"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn context_pack_builds_coding_context_bundle() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }

    let root = temp_root("context-pack");
    fs::write(root.join("pyproject.toml"), "[tool.pytest.ini_options]\n").unwrap();
    let src = root.join("src");
    let tests = root.join("tests");
    fs::create_dir_all(&src).unwrap();
    fs::create_dir_all(&tests).unwrap();
    fs::write(root.join("AGENTS.md"), "root coding rules\n").unwrap();
    fs::write(src.join("CLAUDE.md"), "src coding rules\n").unwrap();
    let skill_dir = root.join("skills").join("review");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
            skill_dir.join("SKILL.md"),
            "# Skill: Review\ndescription: Review changes for correctness.\n\nUse this when reviewing a patch.\n",
        )
        .unwrap();
    let target = src.join("service.py");
    fs::write(
        &target,
        "import json\n\n\ndef load_user():\n    return None\n",
    )
    .unwrap();
    fs::write(
        tests.join("test_service.py"),
        "from src.service import load_user\n",
    )
    .unwrap();
    assert!(Command::new("git")
        .arg("init")
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("git")
        .arg("add")
        .arg(".")
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("git")
        .args([
            "-c",
            "user.name=Nola",
            "-c",
            "user.email=nola@example.com",
            "commit",
            "-m",
            "init",
        ])
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    fs::write(
        &target,
        "import json\n\n\ndef load_user():\n    return {'id': 1}\n",
    )
    .unwrap();
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
    let mut args = Map::new();
    args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );
    args.insert("max_files".to_string(), Value::Number(5.into()));

    let outcome = context_pack(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("# Context pack:"));
    assert!(outcome.text.contains("src/service.py"));
    assert!(outcome.text.contains("function load_user"));
    assert!(outcome.text.contains("import json"));
    assert!(outcome.text.contains("tests/test_service.py"));
    assert!(outcome.text.contains("## Instructions"));
    assert!(outcome.text.contains("### AGENTS.md"));
    assert!(outcome.text.contains("root coding rules"));
    assert!(!outcome.text.contains("src coding rules"));
    assert!(outcome.text.contains("## Skills"));
    assert!(outcome.text.contains("Review (skills/review/SKILL.md)"));
    assert!(outcome
        .text
        .contains("Call skill_context with name or query"));
    assert!(outcome.text.contains("## Changed symbols"));
    assert!(outcome.text.contains("-m pytest"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn context_pack_selection_is_explicit_while_focused_context_discovers_matches() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let known = root.join("known.rs");
    let related = root.join("related.rs");
    fs::write(&known, "fn stable_entry() {}\n").unwrap();
    fs::write(&related, "fn decode_packet() {}\n").unwrap();
    assert_eq!(context_pack_paths(root, &[known.clone()], false, 10), vec![known.clone()]);
    assert!(context_pack_paths(root, &[], false, 10).is_empty());
    let (ranked, _) = focused_context_candidates(root, "decode_packet", &[known.clone()], 10, 100);
    assert!(ranked.iter().any(|(_, path, _)| path == &known));
    assert!(ranked.iter().any(|(_, path, _)| path == &related));
}

#[test]
fn context_catalog_removes_redundant_brief_without_removing_source_tools() {
    let catalog = specs();
    assert!(!catalog.iter().any(|spec| spec.name == "implementation_brief"));
    for name in ["context_pack", "focused_context", "instruction_context", "test_context", "code_diagnostics", "validation_plan"] {
        assert!(catalog.iter().any(|spec| spec.name == name), "missing {name}");
    }
    let args = serde_json::from_value(json!({"query":"select:implementation_brief,focused_context"})).unwrap();
    let outcome = tool_search(&args, &crate::skill_runtime::SkillRuntimeContext::default());
    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("missing: implementation_brief"));
    assert!(outcome.text.contains(r#""name":"focused_context""#));
    assert!(!outcome.text.contains(r#""name":"implementation_brief""#));
    let directory = tempfile::tempdir().unwrap();
    let ctx = ToolContext {
        memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
        language_services: Default::default(),
        skills: Default::default(),
        allowed_dirs: vec![directory.path().to_path_buf()],
        policy: ToolExecutionPolicy::new(Vec::new(), true, "deny".into(), Vec::new()),
        permission_scope: Default::default(),
        cancellation: Default::default(),
        plan_state: Default::default(),
        output_store: ToolOutputStore::for_test(),
        browser: BrowserRuntime::new(directory.path().join("browser")),
    };
    let request = ToolRequest {
        id: "removed-context-entry".into(),
        name: "implementation_brief".into(),
        args: serde_json::from_value(json!({"root": directory.path(), "task": "inspect"})).unwrap(),
    };
    let rejected = execute_authorized_tool(&request, &ctx, ExecutionAuthority::CoreApproved(nola_domain::PermissionProfile::Cautious));
    assert!(rejected.error);
    assert!(rejected.text.contains("Unknown tool 'implementation_brief'"));
}

#[test]
fn focused_context_builds_query_ranked_exploration_bundle() {
    let root = temp_root("focused-context");
    fs::write(root.join("pyproject.toml"), "[tool.pytest.ini_options]\n").unwrap();
    let src = root.join("src");
    let tests = root.join("tests");
    fs::create_dir_all(&src).unwrap();
    fs::create_dir_all(&tests).unwrap();
    fs::write(root.join("AGENTS.md"), "root rule: use Decimal for money\n").unwrap();
    fs::write(
        src.join("CLAUDE.md"),
        "billing code must keep explicit rounding tests\n",
    )
    .unwrap();
    let target = src.join("billing_service.py");
    let unrelated = src.join("profile.py");
    let test_file = tests.join("test_billing_service.py");
    fs::write(
            &target,
            "import decimal\n\n\ndef calculate_invoice_total(items):\n    subtotal = sum(item['amount'] for item in items)\n    return decimal.Decimal(subtotal)\n",
        )
        .unwrap();
    fs::write(
        &unrelated,
        "def render_profile(user):\n    return user['name']\n",
    )
    .unwrap();
    fs::write(
            &test_file,
            "from src.billing_service import calculate_invoice_total\n\n\ndef test_calculate_invoice_total():\n    assert calculate_invoice_total([{'amount': '4'}])\n",
        )
        .unwrap();
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
    let mut args = Map::new();
    args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );
    args.insert(
        "query".to_string(),
        Value::String("fix calculate invoice total decimal rounding".to_string()),
    );
    args.insert("max_files".to_string(), Value::Number(3.into()));

    let outcome = focused_context(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("# Focused context:"));
    assert!(outcome.text.contains("src/billing_service.py"));
    assert!(outcome.text.contains("function calculate_invoice_total"));
    assert!(outcome.text.contains("decimal"));
    assert!(outcome.text.contains("## Applicable instructions"));
    assert!(outcome.text.contains("### AGENTS.md"));
    assert!(outcome.text.contains("root rule: use Decimal for money"));
    assert!(!outcome
        .text
        .contains("billing code must keep explicit rounding tests"));
    assert!(outcome.text.contains("tests/test_billing_service.py"));
    assert!(outcome
        .text
        .contains("-m pytest tests/test_billing_service.py"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn impact_analysis_summarizes_refs_calls_and_validation() {
    let root = temp_root("impact-analysis");
    fs::write(root.join("pyproject.toml"), "[tool.pytest.ini_options]\n").unwrap();
    let src = root.join("src");
    let tests = root.join("tests");
    fs::create_dir_all(&src).unwrap();
    fs::create_dir_all(&tests).unwrap();
    let target = src.join("service.py");
    let consumer = src.join("api.py");
    let test_file = tests.join("test_service.py");
    fs::write(&target, "def load_user():\n    return query_db()\n").unwrap();
    fs::write(
        &consumer,
        "from src.service import load_user\n\ndef render():\n    return load_user()\n",
    )
    .unwrap();
    fs::write(&test_file, "from src.service import load_user\n").unwrap();
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
    let mut args = Map::new();
    args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );
    args.insert(
        "paths".to_string(),
        Value::Array(vec![Value::String(target.display().to_string())]),
    );
    args.insert(
        "symbols".to_string(),
        Value::Array(vec![Value::String("load_user".to_string())]),
    );
    args.insert("glob".to_string(), Value::String("**/*.py".to_string()));

    let outcome = impact_analysis(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("## Related files and tests"));
    assert!(outcome.text.contains("tests/test_service.py"));
    assert!(
        outcome.text.contains("src/api.py")
            || outcome.text.contains(&consumer.display().to_string())
    );
    assert!(outcome.text.contains("incoming calls:"));
    assert!(outcome.text.contains("render at line 3 -> load_user"));
    assert!(outcome.text.contains("outgoing calls:"));
    assert!(outcome.text.contains("query_db"));
    assert!(outcome.text.contains("-m pytest"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn related_files_finds_tests_and_references() {
    let root = temp_root("related-files");
    let src = root.join("src");
    let tests = root.join("tests");
    fs::create_dir_all(&src).unwrap();
    fs::create_dir_all(&tests).unwrap();
    let target = src.join("user_service.py");
    let test_file = tests.join("test_user_service.py");
    let companion = src.join("user_service.md");
    let consumer = src.join("api.py");
    fs::write(&target, "def load_user():\n    pass\n").unwrap();
    fs::write(&test_file, "from src.user_service import load_user\n").unwrap();
    fs::write(&companion, "notes\n").unwrap();
    fs::write(&consumer, "from src.user_service import load_user\n").unwrap();
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
    let mut args = Map::new();
    args.insert(
        "path".to_string(),
        Value::String(target.display().to_string()),
    );
    args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );

    let outcome = related_files(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains(&format!("- {}", test_file.display())));
    assert!(outcome.text.contains(&format!("- {}", companion.display())));
    assert!(outcome.text.contains(&format!("{}:1:", consumer.display())));
    assert!(outcome.display.contains("1 tests"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn edit_file_handles_line_endings_and_single_line_indent() {
    let root = temp_root("smart-edit");
    let crlf_file = root.join("crlf.txt");
    fs::write(&crlf_file, b"one\r\ntwo\r\n").unwrap();
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
    let mut args = Map::new();
    args.insert(
        "path".to_string(),
        Value::String(crlf_file.display().to_string()),
    );
    args.insert(
        "old_text".to_string(),
        Value::String("one\ntwo\n".to_string()),
    );
    args.insert(
        "new_text".to_string(),
        Value::String("alpha\nbeta\n".to_string()),
    );
    let original_digest = file_sha256_hex(&crlf_file).unwrap();
    args.insert("dry_run".to_string(), Value::Bool(true));

    let preview = edit_file(&args, &ctx);

    assert!(!preview.error, "{}", preview.text);
    assert!(preview.text.contains("dry_run: true"));
    assert!(preview.text.contains("## Diff"));
    assert!(preview.text.contains("-one"));
    assert!(preview.text.contains("+alpha"));
    assert_eq!(fs::read(&crlf_file).unwrap(), b"one\r\ntwo\r\n");

    fs::write(&crlf_file, b"one user\r\ntwo\r\n").unwrap();
    args.insert(
        "expected_sha256".to_string(),
        Value::String(original_digest.clone()),
    );
    let stale = edit_file(&args, &ctx);
    assert!(stale.error);
    assert!(stale.text.contains("expected_sha256 mismatch"));
    assert_eq!(fs::read(&crlf_file).unwrap(), b"one user\r\ntwo\r\n");
    fs::write(&crlf_file, b"one\r\ntwo\r\n").unwrap();

    args.insert("dry_run".to_string(), Value::Bool(false));
    args.insert(
        "expected_sha256".to_string(),
        Value::String(file_sha256_hex(&crlf_file).unwrap()),
    );
    let crlf_outcome = edit_file(&args, &ctx);
    assert!(!crlf_outcome.error, "{}", crlf_outcome.text);
    assert_eq!(fs::read(&crlf_file).unwrap(), b"alpha\r\nbeta\r\n");

    let indent_file = root.join("indent.py");
    fs::write(&indent_file, "def value():\n    return 1\n").unwrap();
    args.insert(
        "path".to_string(),
        Value::String(indent_file.display().to_string()),
    );
    args.insert(
        "old_text".to_string(),
        Value::String("return 1".to_string()),
    );
    args.insert(
        "new_text".to_string(),
        Value::String("return 2".to_string()),
    );
    args.remove("dry_run");
    args.remove("expected_sha256");

    let indent_outcome = edit_file(&args, &ctx);

    assert!(!indent_outcome.error, "{}", indent_outcome.text);
    assert_eq!(
        fs::read_to_string(&indent_file).unwrap(),
        "def value():\n    return 2\n"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn edit_file_rejects_ambiguous_non_exact_replacement() {
    let root = temp_root("smart-edit-ambiguous");
    let file = root.join("ambiguous.py");
    fs::write(&file, "def a():\n    return 1\ndef b():\n    return 1\n").unwrap();
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
    let mut args = Map::new();
    args.insert(
        "path".to_string(),
        Value::String(file.display().to_string()),
    );
    args.insert(
        "old_text".to_string(),
        Value::String("return 1 ".to_string()),
    );
    args.insert(
        "new_text".to_string(),
        Value::String("return 2".to_string()),
    );

    let outcome = edit_file(&args, &ctx);

    assert!(outcome.error);
    assert!(outcome.text.contains("old_text_not_found"));
    assert!(outcome.text.contains("edit_match_count: 0"));
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        "def a():\n    return 1\ndef b():\n    return 1\n"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn multi_edit_file_applies_edits_atomically() {
    let root = temp_root("multi-edit");
    let file = root.join("note.txt");
    fs::write(&file, "alpha\nbeta\ngamma\n").unwrap();
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
    let mut args = Map::new();
    args.insert(
        "path".to_string(),
        Value::String(file.display().to_string()),
    );
    args.insert(
        "edits".to_string(),
        json!([
            {"old_text": "alpha", "new_text": "one"},
            {"old_text": "gamma", "new_text": "three"}
        ]),
    );
    args.insert("dry_run".to_string(), Value::Bool(true));

    let preview = multi_edit_file(&args, &ctx);

    assert!(!preview.error, "{}", preview.text);
    assert!(preview.text.contains("dry_run: true"));
    assert!(preview.text.contains("-alpha"));
    assert!(preview.text.contains("+one"));
    assert_eq!(fs::read_to_string(&file).unwrap(), "alpha\nbeta\ngamma\n");

    args.insert("dry_run".to_string(), Value::Bool(false));
    let outcome = multi_edit_file(&args, &ctx);
    assert!(!outcome.error, "{}", outcome.text);
    assert_eq!(fs::read_to_string(&file).unwrap(), "one\nbeta\nthree\n");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn multi_edit_file_does_not_write_on_failure() {
    let root = temp_root("multi-edit-failure");
    let file = root.join("note.txt");
    let original = "alpha\nbeta\ngamma\n";
    fs::write(&file, original).unwrap();
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
    let mut args = Map::new();
    args.insert(
        "path".to_string(),
        Value::String(file.display().to_string()),
    );
    args.insert(
        "edits".to_string(),
        json!([
            {"old_text": "alpha", "new_text": "one"},
            {"old_text": "missing", "new_text": "nope"}
        ]),
    );

    let outcome = multi_edit_file(&args, &ctx);

    assert!(outcome.error);
    assert!(outcome.text.contains("edit 2 failed"));
    assert_eq!(fs::read_to_string(&file).unwrap(), original);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn apply_patch_file_previews_by_default_then_applies_unified_diff() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }
    let root = temp_root("apply-patch");
    let file = root.join("note.txt");
    fs::write(&file, "alpha\nbeta\ngamma\n").unwrap();
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
    let mut args = Map::new();
    args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );
    args.insert(
            "patch".to_string(),
            Value::String(
                "diff --git a/note.txt b/note.txt\n--- a/note.txt\n+++ b/note.txt\n@@ -1,3 +1,3 @@\n alpha\n-beta\n+Nola\n gamma\n".to_string(),
            ),
        );

    let preview = apply_patch_file(&args, &ctx);

    assert!(!preview.error, "{}", preview.text);
    assert!(preview.text.contains("dry_run: true"));
    assert!(preview.text.contains("## Patch"));
    assert!(preview.text.contains("sha256="));
    assert_eq!(fs::read_to_string(&file).unwrap(), "alpha\nbeta\ngamma\n");

    let old_sha = file_sha256_hex(&file).unwrap();
    fs::write(&file, "alpha\nuser\ngamma\n").unwrap();
    args.insert("dry_run".to_string(), Value::Bool(false));
    args.insert(
        "expected_sha256_by_path".to_string(),
        json!({"note.txt": old_sha}),
    );
    let stale = apply_patch_file(&args, &ctx);

    assert!(stale.error);
    assert!(stale.text.contains("expected_sha256_by_path mismatch"));
    assert_eq!(fs::read_to_string(&file).unwrap(), "alpha\nuser\ngamma\n");

    fs::write(&file, "alpha\nbeta\ngamma\n").unwrap();
    args.insert(
        "expected_sha256_by_path".to_string(),
        json!({"note.txt": file_sha256_hex(&file).unwrap()}),
    );
    let outcome = apply_patch_file(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert_eq!(fs::read_to_string(&file).unwrap(), "alpha\nNola\ngamma\n");
    assert!(outcome.text.contains("- note.txt sha256="));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn patch_paths_rejects_parent_traversal() {
    let err = patch_paths(
            "diff --git a/../outside.txt b/../outside.txt\n--- a/../outside.txt\n+++ b/../outside.txt\n@@ -1 +1 @@\n-a\n+b\n",
        )
        .unwrap_err();

    assert!(err.text.contains("unsafe patch path"));
}

#[test]
fn patch_paths_rejects_hidden_metadata_effects_and_ambiguous_paths() {
    let ordinary = "--- a/visible.txt\n+++ b/visible.txt\n@@ -1 +1 @@\n-old\n+new\n";
    for extra in [
        "diff --git a/hidden.txt b/hidden.txt\nold mode 100644\nnew mode 100755\n",
        "diff --git a/source.txt b/destination.txt\nsimilarity index 100%\nrename from source.txt\nrename to destination.txt\n",
        "diff --git a/secret.bin b/secret.bin\nGIT binary patch\nliteral 1\n",
        "diff --git a/link b/link\nnew file mode 120000\n",
        "--- x/other.txt\n+++ x/other.txt\n@@ -1 +1 @@\n-a\n+b\n",
        "--- a/name with spaces\n+++ b/name with spaces\n@@ -1 +1 @@\n-a\n+b\n",
    ] {
        assert!(patch_paths(&format!("{ordinary}{extra}")).is_err(), "accepted {extra}");
    }
    assert_eq!(patch_paths("diff --git a/old.txt b/new.txt\nrename from old.txt\nrename to new.txt\n--- a/old.txt\n+++ b/new.txt\n@@ -1 +1 @@\n-old\n+new\n").unwrap(), vec!["new.txt", "old.txt"]);
}

#[test]
fn patch_failure_records_observed_effects_without_claiming_not_started() {
    let root = temp_root("patch-failure-effects").canonicalize().unwrap();
    let old = root.join("old.txt");
    let new = root.join("new.txt");
    fs::write(&old, "before").unwrap();
    let paths = vec![old.clone(), new.clone()];
    let before = observe_patch_targets(&paths).unwrap();
    fs::remove_file(&old).unwrap();
    fs::write(&new, "after").unwrap();
    let failed = observe_patch_effects(error("cancelled".into()), &paths, &before);
    assert!(failed.error);
    let receipt = failed.execution.unwrap();
    assert_eq!(receipt.state, nola_host_protocol::tool_result::ExecutionState::Partial);
    assert_eq!(receipt.effects.len(), 2);
    assert_eq!(receipt.effects[0].operation, "delete");
    assert_eq!(receipt.effects[0].after_version.as_deref(), Some("absent:v1"));
    assert_eq!(receipt.effects[1].operation, "create");
    assert!(receipt.effects[1].after_version.as_ref().unwrap().starts_with("sha256:"));
    let current = observe_patch_targets(&paths).unwrap();
    let unknown = observe_patch_effects(error("wait failed".into()), &paths, &current);
    assert_eq!(unknown.execution.unwrap().state, nola_host_protocol::tool_result::ExecutionState::Unknown);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn patch_targets_reject_symlink_ancestors() {
    #[cfg(unix)] {
        let root = temp_root("patch-symlink-parent").canonicalize().unwrap();
        fs::create_dir(root.join("real")).unwrap();
        std::os::unix::fs::symlink(root.join("real"), root.join("alias")).unwrap();
        assert!(observe_patch_targets(&[root.join("alias/new.txt")]).is_err());
        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn patch_parent_observation_failure_is_unknown_and_changes_are_not_attributed() {
    let root = temp_root("patch-parent-observation");
    let parent = root.join("parent");
    // Another actor changed the formerly absent parent to a regular file.
    fs::write(&parent, "not a directory").unwrap();
    let result = observe_patch_parent_changes(error("execution interrupted".into()), &[parent.clone()]);
    assert!(result.error);
    let receipt = result.execution.unwrap();
    assert_eq!(receipt.state, nola_host_protocol::tool_result::ExecutionState::Unknown);
    assert_eq!(receipt.effects.len(), 1);
    assert_eq!(receipt.effects[0].operation, "observed_directory_change");
    assert_eq!(receipt.effects[0].after_version, None);
    fs::remove_file(&parent).unwrap();
    fs::create_dir(&parent).unwrap();
    let result = observe_patch_parent_changes(ok("applied".into(), "applied".into()), &[parent]);
    assert!(result.text.contains("creation ownership is not established"));
    assert_eq!(result.execution.unwrap().effects[0].operation, "observed_directory_change");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn apply_patch_precancelled_execution_does_not_start_git() {
    let cancellation = RuntimeExecutionControl::default();
    cancellation.request();
    let root = temp_root("patch-pre-cancel");
    for check in [true, false] {
        let failure = run_git_apply_once(&root.join("nonexistent"), "invalid patch", check, false, &cancellation).unwrap_err();
        assert!(failure.text.contains("cancelled before process start"));
        assert_eq!(failure.execution.unwrap().state, nola_host_protocol::tool_result::ExecutionState::NotStarted);
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn apply_patch_rename_preserves_before_after_versions() {
    let root = temp_root("patch-rename-versions");
    fs::write(root.join("old.txt"), "old\n").unwrap();
    let ctx = file_evidence_context(&root);
    let args = json!({"root":root,"dry_run":false,
        "patch":"diff --git a/old.txt b/new.txt\nrename from old.txt\nrename to new.txt\n--- a/old.txt\n+++ b/new.txt\n@@ -1 +1 @@\n-old\n+new\n"});
    let outcome = apply_patch_file(args.as_object().unwrap(), &ctx);
    assert!(!outcome.error, "{}", outcome.text);
    assert!(!root.join("old.txt").exists());
    assert_eq!(fs::read_to_string(root.join("new.txt")).unwrap(), "new\n");
    let receipt = outcome.execution.unwrap();
    assert_eq!(receipt.state, nola_host_protocol::tool_result::ExecutionState::Completed);
    assert_eq!(receipt.effects.len(), 2);
    assert!(receipt.effects.iter().all(|effect| effect.before_version.is_some() && effect.after_version.is_some()));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn apply_patch_accepts_wrapped_unified_diff_and_recounts_hunks() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }
    let root = temp_root("apply-patch-recount");
    let file = root.join("note.txt");
    fs::write(&file, "alpha\nbeta\ngamma\n").unwrap();
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
    let args = json!({
        "root": root,
        "dry_run": false,
        "patch": "*** Begin Patch\n--- a/note.txt\n+++ b/note.txt\n@@ -40,6 +40,7 @@\n alpha\n-beta\n+Nola\n gamma\n*** End Patch"
    });
    let outcome = apply_patch_file(args.as_object().unwrap(), &ctx);
    assert!(!outcome.error, "{}", outcome.text);
    assert_eq!(fs::read_to_string(&file).unwrap(), "alpha\nNola\ngamma\n");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn git_diff_reports_staged_unstaged_and_untracked() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }

    let root = temp_root("git-diff");
    let tracked = root.join("tracked.txt");
    let staged = root.join("staged.txt");
    let untracked = root.join("new.txt");
    fs::write(&tracked, "alpha\n").unwrap();
    fs::write(&staged, "old staged\n").unwrap();
    assert!(Command::new("git")
        .arg("init")
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("git")
        .arg("add")
        .arg(".")
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("git")
        .args([
            "-c",
            "user.name=Nola",
            "-c",
            "user.email=nola@example.com",
            "commit",
            "-m",
            "init",
        ])
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    fs::write(&staged, "new staged\n").unwrap();
    assert!(Command::new("git")
        .arg("add")
        .arg("staged.txt")
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    fs::write(&tracked, "beta\n").unwrap();
    fs::write(&untracked, "fresh file\n").unwrap();
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
    let mut args = Map::new();
    args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );

    let outcome = git_diff(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("## Unstaged diff"));
    assert!(outcome.text.contains("## Staged diff"));
    assert!(outcome.text.contains("## Untracked files"));
    assert!(outcome.text.contains("-alpha"));
    assert!(outcome.text.contains("+beta"));
    assert!(outcome.text.contains("-old staged"));
    assert!(outcome.text.contains("+new staged"));
    assert!(outcome.text.contains(&format!("- {}", untracked.display())));
    assert!(outcome.text.contains("+fresh file"));
    assert_eq!(outcome.display, "Git diff (3 changed files)");
    assert!(outcome.text.find("## Changed file inventory").unwrap() < outcome.text.find("## Unstaged diff").unwrap());

    // No producer-side cap may turn a later section into `(none)` or make
    // the standard full-output reference point at an already truncated diff.
    fs::write(&tracked, format!("{}\nUNSTAGED_TAIL\n", "变更内容\n".repeat(10_000))).unwrap();
    fs::write(&untracked, format!("{}\nUNTRACKED_TAIL\n", "new content\n".repeat(100_000))).unwrap();
    let unusual_name = root.join(" 空格与中文\nfile.txt");
    fs::write(&unusual_name, "UNUSUAL_NAME_CONTENT\n").unwrap();
    fs::write(root.join("artifact.bin"), b"\0\xff\x80BINARY_NOT_PROSE").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("missing-target", root.join("dangling-link")).unwrap();
    let complete = git_diff(&args, &ctx);
    assert!(!complete.error, "{}", complete.text);
    for marker in ["UNSTAGED_TAIL", "+new staged", "UNTRACKED_TAIL", "UNUSUAL_NAME_CONTENT", "Binary files"] {
        assert!(complete.text.contains(marker), "missing {marker}");
    }
    assert!(!complete.text.contains("BINARY_NOT_PROSE"));
    assert!(!complete.text.contains("diff truncated"));
    #[cfg(unix)]
    {
        assert!(complete.text.contains("new file mode 120000"));
        assert!(complete.text.contains("+missing-target"));
    }
    let original = complete.text.clone();
    let bounded = clamp_outcome_with_writer(complete, 2_000, |text| ctx.output_store.persist(text));
    let saved = full_output_ref_from_outcome(&bounded).expect("persisted complete diff");
    assert_eq!(fs::read_to_string(&saved).unwrap(), original);
    let mut read_args = Map::new();
    read_args.insert("path".into(), Value::String(saved.clone()));
    read_args.insert("offset".into(), Value::from(original.len().saturating_sub(100)));
    read_args.insert("max_bytes".into(), Value::from(100));
    let tail = tool_output_read_in_store(&read_args, &ctx.output_store);
    assert!(!tail.error, "{}", tail.text);
    let page: Value = serde_json::from_str(&tail.text).unwrap();
    assert_eq!(page["has_more"], false);
    assert_eq!(page["content"], &original[original.len() - 100..]);
    // The persisted evidence, rather than the preview, contains every section.
    assert!(bounded.activation.unwrap().capability_ids.contains(&"tool_output_read".into()));
    fs::remove_file(saved).unwrap();

    args.insert(
        "paths".to_string(),
        Value::Array(vec![Value::String("tracked.txt".to_string())]),
    );
    let relative_path_outcome = git_diff(&args, &ctx);
    assert!(
        !relative_path_outcome.error,
        "{}",
        relative_path_outcome.text
    );
    assert!(relative_path_outcome.text.contains("tracked.txt"));
    assert!(!relative_path_outcome.text.contains("staged.txt"));
    assert!(!relative_path_outcome.text.contains("new.txt"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn workspace_checkpoint_isolates_and_restores_changes_since_checkpoint() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }

    let _env_guard = crate::store::test_env_lock();
    let database_root = temp_root("workspace-checkpoint-store");
    std::env::set_var("NOLA_DB_PATH", database_root.join("nola.db"));
    let root = temp_root("workspace-checkpoint");
    let target = root.join("tracked.txt");
    fs::write(&target, "alpha\n").unwrap();
    assert!(Command::new("git")
        .arg("init")
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("git")
        .arg("add")
        .arg("tracked.txt")
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("git")
        .args([
            "-c",
            "user.name=Nola",
            "-c",
            "user.email=nola@example.com",
            "commit",
            "-m",
            "init",
        ])
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    fs::write(&target, "beta\n").unwrap();
    fs::write(root.join("baseline.txt"), "already here\n").unwrap();
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
    let mut create_args = Map::new();
    create_args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );
    create_args.insert(
        "label".to_string(),
        Value::String("before risky edit".to_string()),
    );

    let created = workspace_checkpoint(&create_args, &ctx);

    assert!(!created.error, "{}", created.text);
    assert!(created.text.contains("source: working tree snapshot"));
    assert!(created.text.contains("- baseline.txt"));
    let checkpoint_id = created
        .text
        .lines()
        .find_map(|line| line.strip_prefix("checkpoint_id: "))
        .unwrap()
        .to_string();
    fs::write(&target, "gamma\n").unwrap();
    fs::write(root.join("after.txt"), "new after checkpoint\n").unwrap();
    let mut diff_args = Map::new();
    diff_args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );
    diff_args.insert("action".to_string(), Value::String("diff".to_string()));
    diff_args.insert(
        "checkpoint_id".to_string(),
        Value::String(checkpoint_id.clone()),
    );

    let diffed = workspace_checkpoint(&diff_args, &ctx);

    assert!(!diffed.error, "{}", diffed.text);
    assert!(diffed.text.contains("-beta"));
    assert!(diffed.text.contains("+gamma"));
    assert!(!diffed.text.contains("-alpha"));
    assert!(diffed.text.contains("+ after.txt"));
    assert!(!diffed.text.contains("+ baseline.txt"));

    let mut preview_args = Map::new();
    preview_args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );
    preview_args.insert("action".to_string(), Value::String("restore".to_string()));
    preview_args.insert(
        "checkpoint_id".to_string(),
        Value::String(checkpoint_id.clone()),
    );
    let preview = workspace_checkpoint(&preview_args, &ctx);

    assert!(!preview.error, "{}", preview.text);
    assert!(preview.text.contains("restore preview"));
    assert_eq!(fs::read_to_string(&target).unwrap(), "gamma\n");
    assert!(root.join("after.txt").exists());

    let mut restore_args = Map::new();
    restore_args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );
    restore_args.insert("action".to_string(), Value::String("restore".to_string()));
    restore_args.insert("checkpoint_id".to_string(), Value::String(checkpoint_id));
    restore_args.insert("dry_run".to_string(), Value::Bool(false));
    restore_args.insert("remove_new_untracked".to_string(), Value::Bool(true));
    let restored = workspace_checkpoint(&restore_args, &ctx);

    assert!(!restored.error, "{}", restored.text);
    assert_eq!(fs::read_to_string(&target).unwrap(), "beta\n");
    assert!(root.join("baseline.txt").exists());
    assert!(!root.join("after.txt").exists());
    assert!(restored.text.contains("- after.txt"));
    std::env::remove_var("NOLA_DB_PATH");
    let _ = fs::remove_dir_all(database_root);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn git_history_scopes_recent_commits_to_path() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }

    let root = temp_root("git-history");
    let target = root.join("target.txt");
    let other = root.join("other.txt");
    fs::write(&target, "one\n").unwrap();
    fs::write(&other, "side\n").unwrap();
    assert!(Command::new("git")
        .arg("init")
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("git")
        .arg("add")
        .arg(".")
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("git")
        .args([
            "-c",
            "user.name=Nola",
            "-c",
            "user.email=nola@example.com",
            "commit",
            "-m",
            "target initial",
        ])
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    fs::write(&other, "side two\n").unwrap();
    assert!(Command::new("git")
        .arg("add")
        .arg("other.txt")
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("git")
        .args([
            "-c",
            "user.name=Nola",
            "-c",
            "user.email=nola@example.com",
            "commit",
            "-m",
            "other change",
        ])
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    fs::write(&target, "two\n").unwrap();
    assert!(Command::new("git")
        .arg("add")
        .arg("target.txt")
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("git")
        .args([
            "-c",
            "user.name=Nola",
            "-c",
            "user.email=nola@example.com",
            "commit",
            "-m",
            "target update",
        ])
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
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
    let mut args = Map::new();
    args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );
    args.insert(
        "path".to_string(),
        Value::String(target.display().to_string()),
    );
    args.insert("max_commits".to_string(), Value::from(5));

    let outcome = git_history(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("scope: target.txt"));
    assert!(outcome.text.contains("Subject: target update"));
    assert!(outcome.text.contains("Subject: target initial"));
    assert!(!outcome.text.contains("Subject: other change"));
    assert!(outcome.text.contains("M\ttarget.txt") || outcome.text.contains("A\ttarget.txt"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn git_blame_reports_line_summary_for_range() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }

    let root = temp_root("git-blame");
    let target = root.join("target.txt");
    fs::write(&target, "first\nsecond\nthird\n").unwrap();
    assert!(Command::new("git")
        .arg("init")
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("git")
        .arg("add")
        .arg("target.txt")
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("git")
        .args([
            "-c",
            "user.name=Nola",
            "-c",
            "user.email=nola@example.com",
            "commit",
            "-m",
            "initial lines",
        ])
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    fs::write(&target, "first\nsecond v2\nthird\n").unwrap();
    assert!(Command::new("git")
        .arg("add")
        .arg("target.txt")
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("git")
        .args([
            "-c",
            "user.name=Nola",
            "-c",
            "user.email=nola@example.com",
            "commit",
            "-m",
            "update second line",
        ])
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
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
    let mut args = Map::new();
    args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );
    args.insert(
        "path".to_string(),
        Value::String(target.display().to_string()),
    );
    args.insert("start_line".to_string(), Value::from(2));
    args.insert("line_count".to_string(), Value::from(1));

    let outcome = git_blame(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("range: 2-2"));
    assert!(outcome.text.contains("2:"));
    assert!(outcome
        .text
        .contains("Nola | update second line | second v2"));
    assert!(!outcome.text.contains("initial lines"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn git_show_reports_scoped_commit_patch() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }

    let root = temp_root("git-show");
    let target = root.join("target.txt");
    let other = root.join("other.txt");
    fs::write(&target, "old\n").unwrap();
    fs::write(&other, "other old\n").unwrap();
    assert!(Command::new("git")
        .arg("init")
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("git")
        .arg("add")
        .arg(".")
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("git")
        .args([
            "-c",
            "user.name=Nola",
            "-c",
            "user.email=nola@example.com",
            "commit",
            "-m",
            "initial",
        ])
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    fs::write(&target, "new\n").unwrap();
    fs::write(&other, "other new\n").unwrap();
    assert!(Command::new("git")
        .arg("add")
        .arg(".")
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("git")
        .args([
            "-c",
            "user.name=Nola",
            "-c",
            "user.email=nola@example.com",
            "commit",
            "-m",
            "update both",
        ])
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
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
    let mut args = Map::new();
    args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );
    args.insert("revision".to_string(), Value::String("HEAD".to_string()));
    args.insert(
        "path".to_string(),
        Value::String(target.display().to_string()),
    );

    let outcome = git_show(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("# Git show: HEAD"));
    assert!(outcome.text.contains("scope: target.txt"));
    assert!(outcome.text.contains("Subject: update both"));
    assert!(outcome
        .text
        .contains("diff --git a/target.txt b/target.txt"));
    assert!(outcome.text.contains("-old"));
    assert!(outcome.text.contains("+new"));
    assert!(!outcome.text.contains("other new"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn workspace_status_reports_allowed_git_changes() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }
    let root = temp_root("workspace-status");
    Command::new("git")
        .arg("init")
        .current_dir(&root)
        .output()
        .unwrap();
    let file = root.join("note.txt");
    fs::write(&file, "alpha\n").unwrap();
    Command::new("git")
        .arg("add")
        .arg("note.txt")
        .current_dir(&root)
        .output()
        .unwrap();
    fs::write(&file, "beta\n").unwrap();
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
    let mut args = Map::new();
    args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );
    args.insert("include_diff".to_string(), Value::Bool(true));

    let outcome = workspace_status(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("note.txt"));
    let outputs: Value = serde_json::from_str(outcome.text.lines()
        .find_map(|line| line.strip_prefix("diff_outputs: ")).unwrap()).unwrap();
    let unstaged = fs::read_to_string(outputs["unstaged"]["path"].as_str().unwrap()).unwrap();
    let staged = fs::read_to_string(outputs["staged"]["path"].as_str().unwrap()).unwrap();
    assert!(unstaged.contains("-alpha"));
    assert!(unstaged.contains("+beta"));
    assert!(staged.contains("+alpha"));
    assert!(outcome.text.contains("changed_entries: 1"));
    assert!(outcome.text.contains("repository_scope_complete: true"));
    assert!(outcome.text.contains("tracked_changed_entries: 1"));
    assert!(outcome.text.contains("untracked_entries: 0"));
    let inventory = &outcome.data.as_ref().unwrap()["change_inventory"];
    assert_eq!(inventory["entries"][0]["path"], "note.txt");
    assert_eq!(inventory["entries"][0]["tracking"], "tracked");
    assert_eq!(inventory["truncated"], false);
    assert_eq!(inventory["content_audited"], false);
    // Porcelain's leading space is a status column, not presentation padding.
    let commit = Command::new("git")
        .args(["-c", "user.name=Fixture", "-c", "user.email=fixture@localhost",
            "-c", "commit.gpgsign=false", "commit", "-m", "baseline"])
        .current_dir(&root).output().unwrap();
    assert!(commit.status.success());
    let unstaged_status = workspace_status(&args, &ctx);
    assert!(unstaged_status.text.lines().any(|line| line == " M note.txt"),
        "{}", unstaged_status.text);
    let nested = root.join("nested");
    fs::create_dir_all(&nested).unwrap();
    args.insert("root".to_string(), Value::String(nested.display().to_string()));
    let scoped = workspace_status(&args, &ctx);
    assert!(!scoped.error);
    assert!(scoped.text.contains("repository_scope_complete: false"));
    assert!(scoped.text.contains("not a repository-wide result"));
    assert_eq!(scoped.data.as_ref().unwrap()["change_inventory"]["repository_scope_complete"], false);
    assert!(outcome.display.contains("1 changed file"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn workspace_status_separates_complete_diff_from_metadata() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }
    let root = temp_root("workspace-status-complete-diff");
    Command::new("git")
        .arg("init")
        .current_dir(&root)
        .output()
        .unwrap();
    let file = root.join("large.txt");
    fs::write(&file, "baseline\n").unwrap();
    Command::new("git")
        .args(["add", "large.txt"])
        .current_dir(&root)
        .output()
        .unwrap();
    let tail_marker = "COMPLETE_DIFF_TAIL_MARKER";
    fs::write(
        &file,
        format!("{}\n{tail_marker}\n", "changed line\n".repeat(3_000)),
    )
    .unwrap();
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
    let mut args = Map::new();
    args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );
    args.insert("include_diff".to_string(), Value::Bool(true));

    let outcome = workspace_status(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.len() < 6_000);
    assert!(!outcome.text.contains(tail_marker));
    let activation = outcome.activation.as_ref().expect("saved diffs expose their readers");
    assert_eq!(activation.source, "full_output_ref");
    assert_eq!(activation.capability_ids, vec!["tool_output_read", "tool_output_search"]);
    let outputs: Value = serde_json::from_str(outcome.text.lines()
        .find_map(|line| line.strip_prefix("diff_outputs: ")).unwrap()).unwrap();
    assert_eq!(outcome.data.as_ref().unwrap()["diff_outputs"], outputs);
    let inventory = &outcome.data.as_ref().unwrap()["change_inventory"];
    assert_eq!(inventory["total_entries"], 1);
    assert_eq!(inventory["entries"][0]["path"], "large.txt");
    assert_eq!(inventory["repository_scope_complete"], true);
    assert_eq!(inventory["content_audited"], false);
    assert_eq!(inventory["task_completion_attested"], false);
    let path = outputs["unstaged"]["path"].as_str().unwrap();
    let diff = fs::read_to_string(path).unwrap();
    assert!(diff.contains(tail_marker));
    assert_eq!(outputs["unstaged"]["size_bytes"].as_u64().unwrap(), diff.len() as u64);
    let tail = ctx.output_store.read(std::path::Path::new(path),
        (diff.len() - tail_marker.len() - 2) as u64, 128, 6_000).unwrap();
    assert!(tail.content.contains(tail_marker));
    assert!(!outcome.text.contains("...[diff truncated]"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn workspace_status_accepts_a_regular_non_git_workspace() {
    let root = temp_root("workspace-status-regular-directory");
    fs::write(root.join("source.txt"), "source\n").unwrap();
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
    let mut args = Map::new();
    args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );

    let outcome = workspace_status(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("version_control: none"));
    assert!(outcome.text.contains("document tools remain available"));
    assert!(outcome.display.contains("Workspace ready"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn change_review_summarizes_risks_diagnostics_and_validation() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }
    let root = temp_root("change-review");
    fs::write(
        root.join("pyproject.toml"),
        "[tool.pytest.ini_options]\n\n[tool.ruff]\n",
    )
    .unwrap();
    let src = root.join("src");
    fs::create_dir_all(&src).unwrap();
    let target = src.join("user_service.py");
    let importer = src.join("app.py");
    fs::write(&target, "def load_user():\n    return None\n").unwrap();
    fs::write(
        &importer,
        "from src.user_service import load_user\n\nvalue = load_user()\n",
    )
    .unwrap();
    assert!(Command::new("git")
        .arg("init")
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("git")
        .arg("add")
        .arg(".")
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("git")
        .args([
            "-c",
            "user.name=Nola",
            "-c",
            "user.email=nola@example.com",
            "commit",
            "-m",
            "init",
        ])
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    fs::write(
        &target,
        "def load_user():\n    print('debug')\n    return {'id': 1}\n",
    )
    .unwrap();
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
    let mut args = Map::new();
    args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );

    let outcome = change_review(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("## Changed files"));
    assert!(outcome.text.contains("src/user_service.py"));
    assert!(outcome.text.contains("print call left in source"));
    assert!(outcome
        .text
        .contains("source files changed without tests changed"));
    assert!(outcome.text.contains("## Changed symbols"));
    assert!(outcome
        .text
        .contains("src/user_service.py:1: function load_user"));
    assert!(outcome.text.contains("changed_lines: 2-3"));
    assert!(outcome.text.contains("## Dependency impact"));
    assert!(outcome
        .text
        .contains("src/app.py:1: python_from src.user_service"));
    assert!(outcome.text.contains("## Suggested formatting"));
    assert!(outcome.text.contains("-m ruff format src/user_service.py"));
    assert!(outcome.text.contains("-m pytest"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn diff_review_reports_hunk_level_findings() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }
    let root = temp_root("diff-review");
    let src = root.join("src");
    let tests = root.join("tests");
    fs::create_dir_all(&src).unwrap();
    fs::create_dir_all(&tests).unwrap();
    let target = src.join("user_service.py");
    let test_file = tests.join("test_user_service.py");
    fs::write(
        &target,
        "def load_user(client, url):\n    return client.get(url)\n",
    )
    .unwrap();
    fs::write(
            &test_file,
            "from src.user_service import load_user\n\n\ndef test_load_user():\n    assert load_user(None, '/') is not None\n",
        )
        .unwrap();
    assert!(Command::new("git")
        .arg("init")
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("git")
        .arg("add")
        .arg(".")
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("git")
        .args([
            "-c",
            "user.name=Nola",
            "-c",
            "user.email=nola@example.com",
            "commit",
            "-m",
            "init",
        ])
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    fs::write(
            &target,
            "def load_user(client, url):\n    try:\n        return client.get(url, verify=False)\n    except Exception:\n        pass\n",
        )
        .unwrap();
    fs::write(
            &test_file,
            "from src.user_service import load_user\n\n\ndef test_load_user():\n    load_user(None, '/')\n",
        )
        .unwrap();
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
    let mut args = Map::new();
    args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );

    let outcome = diff_review(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("## Deterministic findings"));
    assert!(outcome.text.contains("src/user_service.py:3"));
    assert!(outcome
        .text
        .contains("disables TLS or certificate verification"));
    assert!(outcome.text.contains("src/user_service.py:4"));
    assert!(outcome.text.contains("adds broad exception handler"));
    assert!(outcome.text.contains("tests/test_user_service.py:5"));
    assert!(outcome
        .text
        .contains("deleted test assertion or expectation"));
    assert!(outcome.display.contains("3 findings"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn diff_review_scans_large_diff_tail_for_hard_coded_credentials() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }
    let root = temp_root("diff-review-large-tail");
    fs::create_dir_all(&root).unwrap();
    let target = root.join("main.py");
    fs::write(&target, "print('baseline')\n").unwrap();
    assert!(Command::new("git")
        .arg("init")
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("git")
        .arg("add")
        .arg(".")
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("git")
        .args([
            "-c",
            "user.name=Nola",
            "-c",
            "user.email=nola@example.com",
            "commit",
            "-m",
            "init",
        ])
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    let mut changed = String::new();
    for index in 0..3_000 {
        changed.push_str(&format!("value_{index} = {index}\n"));
    }
    changed.push_str("password = \"hardcoded-secret\" # COMPLETE_DIFF_TAIL_MARKER\n");
    fs::write(&target, changed).unwrap();
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
    let mut args = Map::new();
    args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );

    let outcome = diff_review(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("scanned_diff_lines: 3002"));
    assert!(outcome.text.contains("complete_diff_traversal: true"));
    assert!(outcome.text.contains("main.py:3001"));
    assert!(outcome.text.contains("hard-coded credential or secret"));
    assert!(outcome.text.contains("COMPLETE_DIFF_TAIL_MARKER"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn verification_review_reports_findings_first_verdict_and_test_gaps() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }
    let root = temp_root("verification-review");
    fs::write(
        root.join("pyproject.toml"),
        "[tool.pytest.ini_options]\n\n[tool.ruff]\n",
    )
    .unwrap();
    let src = root.join("src");
    fs::create_dir_all(&src).unwrap();
    let target = src.join("user_service.py");
    fs::write(&target, "def load_user():\n    return None\n").unwrap();
    assert!(Command::new("git")
        .arg("init")
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("git")
        .arg("add")
        .arg(".")
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("git")
        .args([
            "-c",
            "user.name=Nola",
            "-c",
            "user.email=nola@example.com",
            "commit",
            "-m",
            "init",
        ])
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    fs::write(
        &target,
        "def load_user():\n    print('debug')\n    return {'id': 1}\n",
    )
    .unwrap();
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
    let mut args = Map::new();
    args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );

    let outcome = verification_review(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("## Findings"));
    assert!(outcome.text.contains("verdict: attention_needed"));
    assert!(outcome.text.contains("medium: "));
    assert!(outcome.text.contains("print call left in source"));
    assert!(outcome
        .text
        .contains("source files changed without tests changed"));
    assert!(outcome.text.contains("## Test gaps"));
    assert!(outcome.text.contains("## Suggested validation"));
    assert!(outcome.text.contains("-m ruff format src/user_service.py"));
    assert!(outcome.display.contains("attention_needed"));
    fs::create_dir(root.join("tests-local")).unwrap();
    fs::write(root.join("tests-local/scratch.py"), "assert True\n").unwrap();
    args.insert("paths".into(), json!(["src/user_service.py"]));
    let scoped = verification_review(&args, &ctx);
    assert!(!scoped.error, "{}", scoped.text);
    assert!(scoped.text.contains("review_scope: selected_paths"));
    assert!(scoped.text.contains("## Reviewed paths\n- src/user_service.py"));
    assert!(scoped.text.contains("## Changed paths outside review"));
    assert!(scoped.text.lines().any(|line| line == "- tests-local/scratch.py"), "{}", scoped.text);
    assert!(scoped.text.contains("not an executed build or whole-delivery acceptance"));
    assert!(root.join("tests-local/scratch.py").exists());
    assert!(!src.join("__pycache__").exists(), "read-only review must not create bytecode in delivery");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn handoff_context_summarizes_resume_state_plans_and_validation() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }
    let root = temp_root("handoff-context");
    fs::write(
        root.join("pyproject.toml"),
        "[tool.pytest.ini_options]\n\n[tool.ruff]\n",
    )
    .unwrap();
    let plan_dir = root.join(".opencode").join("plans");
    fs::create_dir_all(&plan_dir).unwrap();
    fs::write(
        plan_dir.join("fix-parser.md"),
        "# Fix parser\n\n- Add state tracking\n",
    )
    .unwrap();
    let src = root.join("src");
    fs::create_dir_all(&src).unwrap();
    let target = src.join("parser.py");
    fs::write(&target, "def parse(value):\n    return value\n").unwrap();
    assert!(Command::new("git")
        .arg("init")
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("git")
        .arg("add")
        .arg(".")
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("git")
        .args([
            "-c",
            "user.name=Nola",
            "-c",
            "user.email=nola@example.com",
            "commit",
            "-m",
            "init",
        ])
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    fs::write(
        &target,
        "def parse(value):\n    print('debug')\n    return value.strip()\n",
    )
    .unwrap();
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
    let mut args = Map::new();
    args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );
    args.insert(
        "task".to_string(),
        Value::String("continue parser fix".to_string()),
    );

    let outcome = handoff_context(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("# Handoff context:"));
    assert!(outcome.text.contains("task: continue parser fix"));
    assert!(outcome.text.contains("## Persistent plans"));
    assert!(outcome
        .text
        .contains("fix-parser path=.opencode/plans/fix-parser.md sha256="));
    assert!(outcome.text.contains("## Changed symbols"));
    assert!(outcome.text.contains("src/parser.py:1: function parse"));
    assert!(outcome.text.contains("## Findings to preserve"));
    assert!(outcome.text.contains("print call left in source"));
    assert!(outcome.text.contains("## Suggested validation"));
    assert!(outcome.text.contains("-m ruff format src/parser.py"));
    assert!(outcome.text.contains("## Resume checklist"));
    assert!(outcome.display.contains("verdict=attention_needed"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn changed_symbols_reports_symbols_touched_by_diff() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }
    let root = temp_root("changed-symbols");
    let target = root.join("service.py");
    fs::write(
        &target,
        "def load_user():\n    return None\n\n\ndef untouched():\n    return None\n",
    )
    .unwrap();
    assert!(Command::new("git")
        .arg("init")
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("git")
        .arg("add")
        .arg("service.py")
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("git")
        .args([
            "-c",
            "user.name=Nola",
            "-c",
            "user.email=nola@example.com",
            "commit",
            "-m",
            "init",
        ])
        .current_dir(&root)
        .output()
        .unwrap()
        .status
        .success());
    fs::write(
        &target,
        "def load_user():\n    return {'id': 1}\n\n\ndef untouched():\n    return None\n",
    )
    .unwrap();
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
    let mut args = Map::new();
    args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );

    let outcome = changed_symbols(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("service.py:1: function load_user"));
    assert!(outcome.text.contains("changed_lines: 2"));
    assert!(!outcome.text.contains("function untouched"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn grep_supports_context_lines() {
    let root = temp_root("grep-context");
    let file = root.join("sample.rs");
    fs::write(&file, "before\nlet needle = 1;\nafter\n").unwrap();
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
    let mut args = Map::new();
    args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );
    args.insert("pattern".to_string(), Value::String("needle".to_string()));
    args.insert("glob".to_string(), Value::String("*.rs".to_string()));
    args.insert("context_lines".to_string(), Value::from(1));

    let outcome = grep_files(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome
        .text
        .contains(&format!("{}:1:  before", file.display())));
    assert!(outcome
        .text
        .contains(&format!("{}:2:> let needle = 1;", file.display())));
    assert!(outcome
        .text
        .contains(&format!("{}:3:  after", file.display())));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn grep_supports_output_modes_pagination_and_case_insensitive() {
    let root = temp_root("grep-output-modes");
    let first = root.join("first.py");
    let second = root.join("second.py");
    let other = root.join("other.txt");
    fs::write(&first, "Needle\nneedle again\n").unwrap();
    fs::write(&second, "needle here\n").unwrap();
    fs::write(&other, "needle ignored by glob\n").unwrap();
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
    let mut args = Map::new();
    args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );
    args.insert("pattern".to_string(), Value::String("needle".to_string()));
    args.insert("glob".to_string(), Value::String("*.py".to_string()));
    args.insert("case_insensitive".to_string(), Value::Bool(true));
    args.insert(
        "output_mode".to_string(),
        Value::String("files_with_matches".to_string()),
    );

    let files = grep_files(&args, &ctx);

    assert!(!files.error, "{}", files.text);
    assert!(files.text.contains(&first.display().to_string()));
    assert!(files.text.contains(&second.display().to_string()));
    assert!(!files.text.contains(&other.display().to_string()));
    assert!(files.display.contains("3"));
    assert!(files.display.contains("2 files"));

    args.insert(
        "output_mode".to_string(),
        Value::String("count".to_string()),
    );
    let counts = grep_files(&args, &ctx);

    assert!(!counts.error, "{}", counts.text);
    assert!(counts.text.contains(&format!("{}:2", first.display())));
    assert!(counts.text.contains(&format!("{}:1", second.display())));

    args.insert(
        "output_mode".to_string(),
        Value::String("content".to_string()),
    );
    args.insert("offset".to_string(), Value::from(1));
    args.insert("head_limit".to_string(), Value::from(1));
    let page = grep_files(&args, &ctx);

    assert!(!page.error, "{}", page.text);
    assert_eq!(
        page.text
            .lines()
            .filter(|line| !line.starts_with("...["))
            .count(),
        1
    );
    assert!(page.text.contains("showing 1 of 3 entries, offset 1"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn grep_skips_generated_dirs_and_binary_files() {
    let root = temp_root("grep-skips-generated");
    let src = root.join("src");
    let generated = root.join("node_modules").join("pkg");
    let git_dir = root.join(".git");
    fs::create_dir_all(&src).unwrap();
    fs::create_dir_all(&generated).unwrap();
    fs::create_dir_all(&git_dir).unwrap();
    let source_file = src.join("app.rs");
    fs::write(&source_file, "let needle = true;\n").unwrap();
    fs::write(generated.join("index.rs"), "let needle = false;\n").unwrap();
    fs::write(git_dir.join("config"), "needle hidden\n").unwrap();
    fs::write(src.join("data.bin"), b"needle\0ignored").unwrap();
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
    let mut args = Map::new();
    args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );
    args.insert("pattern".to_string(), Value::String("needle".to_string()));

    let outcome = grep_files(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains(&source_file.display().to_string()));
    assert!(!outcome.text.contains("node_modules"));
    assert!(!outcome.text.contains(".git"));
    assert!(!outcome.text.contains("data.bin"));
    assert!(outcome.display.contains("1"));
    assert!(outcome.display.contains("1 files"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn bash_background_task_status_and_stop() {
    let _guard = test_lock();
    let _env_guard = crate::store::test_env_lock();
    let root = temp_root("bash-background");
    std::env::set_var("NOLA_DB_PATH", root.join("nola.db"));
    if let Some(state) = BACKGROUND_BASH_TASKS.get() {
        state.lock().unwrap().clear();
    }
    let ctx = ToolContext {
        memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
        language_services: Default::default(),
        skills: Default::default(),
        allowed_dirs: vec![root.clone()],
        policy: crate::tools::ToolExecutionPolicy::new(vec!["sleep".to_string()], true, "allow".to_string(), Vec::new()),
        permission_scope: Default::default(),
        cancellation: Default::default(),
        plan_state: Default::default(),
        output_store: crate::tools::ToolOutputStore::for_test(),
        browser: BrowserRuntime::new(temp_root("browser-runtime")),
    };
    let mut args = Map::new();
    args.insert("command".to_string(), Value::String("sleep 60".to_string()));
    args.insert("cwd".to_string(), Value::String(root.display().to_string()));
    args.insert(
        "execution_mode".to_string(),
        Value::String("background".to_string()),
    );

    let started = run_bash(&args, &ctx);

    assert!(!started.error, "{}", started.text);
    let task_id = serde_json::from_str::<Value>(&started.text).unwrap()["task_id"]
        .as_str()
        .unwrap()
        .to_string();
    // Simulate a new CLI process: the Rust Child handle is gone, but the
    // persisted task metadata and wrapper-owned status/log files remain.
    BACKGROUND_BASH_TASKS
        .get()
        .expect("background state initialized")
        .lock()
        .unwrap()
        .clear();
    let mut status_args = Map::new();
    status_args.insert("task_id".to_string(), Value::String(task_id.clone()));
    let status = bash_background_status(&status_args, &ctx);

    assert!(!status.error, "{}", status.text);
    assert!(status.text.contains(&task_id));

    let mut wait_args = status_args.clone();
    wait_args.insert("timeout_sec".to_string(), Value::from(1));
    wait_args.insert("poll_interval_ms".to_string(), Value::from(100));
    let waiting = bash_background_wait(&wait_args, &ctx);
    assert!(!waiting.error, "{}", waiting.text);
    assert!(waiting.text.contains("wait_timed_out: true"));
    assert!(waiting.text.contains("next_action:"));

    let stopped = bash_background_stop(&status_args, &ctx);

    assert!(!stopped.error, "{}", stopped.text);
    assert!(stopped.text.contains(&task_id));
    assert!(
        !stopped.text.contains("cleanup_warning:"),
        "{}",
        stopped.text
    );
    assert!(NolaStore::open_default()
        .unwrap()
        .background_task(&task_id)
        .unwrap()
        .is_none());
    std::env::remove_var("NOLA_DB_PATH");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn bash_default_mode_keeps_slow_commands_in_the_foreground() {
    let _guard = test_lock();
    let _env_guard = crate::store::test_env_lock();
    let root = temp_root("bash-auto-background");
    std::env::set_var("NOLA_DB_PATH", root.join("nola.db"));
    std::env::set_var("NOLA_CLI_BASH_AUTO_BACKGROUND_AFTER_MS", "500");
    if let Some(state) = BACKGROUND_BASH_TASKS.get() {
        state.lock().unwrap().clear();
    }
    let ctx = ToolContext {
        memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
        language_services: Default::default(),
        skills: Default::default(),
        allowed_dirs: vec![root.clone()],
        policy: crate::tools::ToolExecutionPolicy::new(vec!["echo".to_string(), "sleep".to_string()], true, "allow".to_string(), Vec::new()),
        permission_scope: Default::default(),
        cancellation: Default::default(),
        plan_state: Default::default(),
        output_store: crate::tools::ToolOutputStore::for_test(),
        browser: BrowserRuntime::new(temp_root("browser-runtime")),
    };
    let mut args = Map::new();
    args.insert(
        "command".to_string(),
        Value::String("echo started && sleep 2 && echo finished".to_string()),
    );
    args.insert("cwd".to_string(), Value::String(root.display().to_string()));
    args.insert("_nola_approved_once".to_string(), Value::Bool(true));

    let forged = run_bash(&args, &ctx);
    assert!(forged.error, "model arguments must not authorize shell execution");
    let started = run_bash_with_permission(&args, &ctx, true);
    assert!(!started.error, "{}", started.text);
    assert!(started.text.contains("started"), "{}", started.text);
    assert!(started.text.contains("finished"), "{}", started.text);
    assert!(started.text.contains("exit=0"), "{}", started.text);
    assert!(!started.text.contains("Started background bash"));
    assert!(!started.text.contains("background_mode:"));
    std::env::remove_var("NOLA_CLI_BASH_AUTO_BACKGROUND_AFTER_MS");
    std::env::remove_var("NOLA_DB_PATH");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn bash_auto_mode_keeps_short_unknown_commands_in_the_foreground() {
    let _guard = test_lock();
    let _env_guard = crate::store::test_env_lock();
    let root = temp_root("bash-auto-foreground");
    std::env::set_var("NOLA_DB_PATH", root.join("nola.db"));
    std::env::set_var("NOLA_CLI_BASH_AUTO_BACKGROUND_AFTER_MS", "500");
    if let Some(state) = BACKGROUND_BASH_TASKS.get() {
        state.lock().unwrap().clear();
    }
    let ctx = ToolContext {
        memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
        language_services: Default::default(),
        skills: Default::default(),
        allowed_dirs: vec![root.clone()],
        policy: crate::tools::ToolExecutionPolicy::new(vec!["echo".to_string()], true, "allow".to_string(), Vec::new()),
        permission_scope: Default::default(),
        cancellation: Default::default(),
        plan_state: Default::default(),
        output_store: crate::tools::ToolOutputStore::for_test(),
        browser: BrowserRuntime::new(temp_root("browser-runtime")),
    };
    let mut args = Map::new();
    args.insert(
        "command".to_string(),
        Value::String("echo quick".to_string()),
    );
    args.insert("cwd".to_string(), Value::String(root.display().to_string()));

    let outcome = run_bash(&args, &ctx);
    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("quick"));
    assert!(!outcome.text.contains("Started background bash task"));
    assert!(NolaStore::open_default()
        .unwrap()
        .list_background_tasks()
        .unwrap()
        .is_empty());

    std::env::remove_var("NOLA_CLI_BASH_AUTO_BACKGROUND_AFTER_MS");
    std::env::remove_var("NOLA_DB_PATH");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn cancelling_a_background_wait_does_not_kill_the_task() {
    let _guard = test_lock();
    let _env_guard = crate::store::test_env_lock();
    let root = temp_root("bash-background-wait-cancel");
    std::env::set_var("NOLA_DB_PATH", root.join("nola.db"));
    if let Some(state) = BACKGROUND_BASH_TASKS.get() {
        state.lock().unwrap().clear();
    }
    let cancellation = RuntimeExecutionControl::default();
    let ctx = ToolContext {
        memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
        language_services: Default::default(),
        skills: Default::default(),
        allowed_dirs: vec![root.clone()],
        policy: crate::tools::ToolExecutionPolicy::new(vec!["sleep".to_string()], true, "allow".to_string(), Vec::new()),
        permission_scope: Default::default(),
        cancellation: cancellation.clone(),
        plan_state: Default::default(),
        output_store: crate::tools::ToolOutputStore::for_test(),
        browser: BrowserRuntime::new(temp_root("browser-runtime")),
    };
    let args = Map::from_iter([
        ("command".to_string(), Value::String("sleep 60".to_string())),
        ("cwd".to_string(), Value::String(root.display().to_string())),
        (
            "execution_mode".to_string(),
            Value::String("background".to_string()),
        ),
    ]);
    let started = run_bash(&args, &ctx);
    assert!(!started.error, "{}", started.text);
    let task_id = serde_json::from_str::<Value>(&started.text).unwrap()["task_id"]
        .as_str()
        .expect("background task id")
        .to_string();

    let cancel_for_thread = cancellation.clone();
    let cancel_thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        cancel_for_thread.request();
    });
    let wait_args = Map::from_iter([
        ("task_id".to_string(), Value::String(task_id.clone())),
        ("timeout_sec".to_string(), Value::from(5)),
        ("poll_interval_ms".to_string(), Value::from(100)),
    ]);
    let waited = bash_background_wait(&wait_args, &ctx);
    cancel_thread.join().unwrap();
    assert!(waited.error, "{}", waited.text);
    // Cancelling observation does not establish the process's current state.
    // Retain the task identity and explicitly report unknown, then observe it.
    let observation = waited.data.as_ref().expect("typed observation failure");
    assert_eq!(observation["task_id"], task_id);
    assert_eq!(observation["state"], "unknown");
    assert_eq!(observation["recovery"]["arguments"]["task_id"], task_id);
    assert_eq!(waited.execution.as_ref().unwrap().state,
        nola_host_protocol::tool_result::ExecutionState::Unknown);

    let fresh_ctx = ToolContext {
        memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
        cancellation: RuntimeExecutionControl::default(),
        ..ctx
    };
    let status_args = Map::from_iter([("task_id".to_string(), Value::String(task_id))]);
    let status = bash_background_status(&status_args, &fresh_ctx);
    assert!(!status.error, "{}", status.text);
    assert!(status.text.contains("status: running"), "{}", status.text);
    let stopped = bash_background_stop(&status_args, &fresh_ctx);
    assert!(!stopped.error, "{}", stopped.text);

    std::env::remove_var("NOLA_DB_PATH");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn bash_failed_command_includes_failure_location_hints() {
    let _guard = test_lock();
    if Command::new("python3").arg("--version").output().is_err() {
        return;
    }
    let root = temp_root("bash-failure-hints");
    let src = root.join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("app.py"), "value = 1\nraise AssertionError\n").unwrap();
    fs::write(
        root.join("fail.py"),
        "import sys\nprint('src/app.py:2: AssertionError')\nsys.exit(1)\n",
    )
    .unwrap();
    let ctx = ToolContext {
        memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
        language_services: Default::default(),
        skills: Default::default(),
        allowed_dirs: vec![root.clone()],
        policy: crate::tools::ToolExecutionPolicy::new(vec!["python3".to_string()], true, "allow".to_string(), Vec::new()),
        permission_scope: Default::default(),
        cancellation: Default::default(),
        plan_state: Default::default(),
        output_store: crate::tools::ToolOutputStore::for_test(),
        browser: BrowserRuntime::new(temp_root("browser-runtime")),
    };
    let mut args = Map::new();
    args.insert(
        "command".to_string(),
        Value::String("python3 fail.py".to_string()),
    );
    args.insert("cwd".to_string(), Value::String(root.display().to_string()));

    let outcome = run_bash(&args, &ctx);

    assert!(outcome.error);
    assert!(outcome.text.contains("--- parsed failure locations ---"));
    assert!(outcome
        .text
        .contains("- src/app.py:2 | src/app.py:2: AssertionError"));
    assert!(outcome.text.contains("call failure_context"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn bash_command_review_reports_prefix_and_destructive_risk() {
    let _guard = test_lock();
    let root = temp_root("bash-command-review");
    let ctx = ToolContext {
        memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
        language_services: Default::default(),
        skills: Default::default(),
        allowed_dirs: vec![root.clone()],
        policy: crate::tools::ToolExecutionPolicy::new(vec!["rm".to_string(), "git status".to_string()], true, "allow".to_string(), Vec::new()),
        permission_scope: Default::default(),
        cancellation: Default::default(),
        plan_state: Default::default(),
        output_store: crate::tools::ToolOutputStore::for_test(),
        browser: BrowserRuntime::new(temp_root("browser-runtime")),
    };
    let mut args = Map::new();
    args.insert(
        "command".to_string(),
        Value::String("rm -rf build".to_string()),
    );
    args.insert("cwd".to_string(), Value::String(root.display().to_string()));

    let destructive = bash_command_review(&args, &ctx);

    assert!(!destructive.error, "{}", destructive.text);
    assert!(destructive.text.contains("allowed_by_prefix: true"));
    assert!(destructive.text.contains("risk_level: high"));
    assert!(destructive.text.contains("destructive:"));

    args.insert(
        "command".to_string(),
        Value::String("git status | cat".to_string()),
    );
    let blocked = bash_command_review(&args, &ctx);

    assert!(!blocked.error, "{}", blocked.text);
    assert!(blocked.text.contains("approval_required: shell syntax"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn run_bash_allows_testing_commands_with_shell_chain() {
    let _guard = test_lock();
    let Some(python) = python_candidates()
        .into_iter()
        .find(|candidate| Command::new(candidate).arg("--version").output().is_ok())
    else {
        return;
    };
    let root = temp_root("bash-run-validation-chain");
    fs::write(root.join("pyproject.toml"), "").unwrap();
    let ctx = ToolContext {
        memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
        language_services: Default::default(),
        skills: Default::default(),
        allowed_dirs: vec![root.clone()],
        policy: crate::tools::ToolExecutionPolicy::new(vec![
            python.clone(),
            "pytest".to_string(),
            "cd".to_string(),
            "source".to_string(),
        ], true, "allow".to_string(), Vec::new()),
        permission_scope: Default::default(),
        cancellation: Default::default(),
        plan_state: Default::default(),
        output_store: crate::tools::ToolOutputStore::for_test(),
        browser: BrowserRuntime::new(temp_root("browser-runtime")),
    };
    let mut args = Map::new();
    args.insert(
        "command".to_string(),
        Value::String(format!(
            "{python} -m unittest -h && {python} -m unittest -h"
        )),
    );
    args.insert("cwd".to_string(), Value::String(root.display().to_string()));

    let outcome = run_bash(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome
        .display
        .starts_with(&format!("$ {python} -m unittest")));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn run_bash_allows_testing_commands_with_env_prefix_chain() {
    let _guard = test_lock();
    if Command::new("python3").arg("--version").output().is_err() {
        return;
    }
    let root = temp_root("bash-run-validation-env-chain");
    let ctx = ToolContext {
        memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
        language_services: Default::default(),
        skills: Default::default(),
        allowed_dirs: vec![root.clone()],
        policy: crate::tools::ToolExecutionPolicy::new(vec!["RUN_TESTS=1".to_string(), "python3".to_string()], true, "allow".to_string(), Vec::new()),
        permission_scope: Default::default(),
        cancellation: Default::default(),
        plan_state: Default::default(),
        output_store: crate::tools::ToolOutputStore::for_test(),
        browser: BrowserRuntime::new(temp_root("browser-runtime")),
    };
    let mut args = Map::new();
    args.insert(
        "command".to_string(),
        Value::String("RUN_TESTS=1 && python3 -m unittest -h".to_string()),
    );
    args.insert("cwd".to_string(), Value::String(root.display().to_string()));

    let outcome = run_bash(&args, &ctx);

    assert!(
        !outcome.error,
        "expected validation chain with env prefix to pass: {}",
        outcome.text
    );
    assert!(outcome.display.starts_with("$ RUN_TESTS=1"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn bash_timeout_max_defaults_to_long_swe_friendly_limit() {
    let _guard = test_lock();
    let _env_guard = crate::store::test_env_lock();
    env::remove_var("NOLA_CLI_BASH_TIMEOUT_MAX_SEC");
    assert_eq!(bash_timeout_max_sec(), 1_200);
    env::set_var("NOLA_CLI_BASH_TIMEOUT_MAX_SEC", "3600");
    assert_eq!(bash_timeout_max_sec(), 3_600);
    env::remove_var("NOLA_CLI_BASH_TIMEOUT_MAX_SEC");
}

#[test]
fn run_bash_rejects_non_validation_shell_operators() {
    let _guard = test_lock();
    let root = temp_root("bash-run-invalid-shell-chain");
    let ctx = ToolContext {
        memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
        language_services: Default::default(),
        skills: Default::default(),
        allowed_dirs: vec![root.clone()],
        policy: crate::tools::ToolExecutionPolicy::new(vec!["pytest".to_string()], true, "allow".to_string(), Vec::new()),
        permission_scope: Default::default(),
        cancellation: Default::default(),
        plan_state: Default::default(),
        output_store: crate::tools::ToolOutputStore::for_test(),
        browser: BrowserRuntime::new(temp_root("browser-runtime")),
    };
    let mut args = Map::new();
    args.insert(
        "command".to_string(),
        Value::String("git status | cat".to_string()),
    );
    args.insert("cwd".to_string(), Value::String(root.display().to_string()));

    let outcome = run_bash(&args, &ctx);

    assert!(outcome.error, "{}", outcome.text);
    assert_eq!(
        outcome.text,
        "shell metacharacters/substitutions are not allowed".to_string()
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn run_bash_sandbox_permissive_allows_readonly_git_and_blocks_mutating_git() {
    let _guard = test_lock();
    let _env_guard = crate::store::test_env_lock();
    let previous = env::var_os("NOLA_CLI_SANDBOX_BASH_PERMISSIVE");
    env::set_var("NOLA_CLI_SANDBOX_BASH_PERMISSIVE", "1");
    let root = temp_root("bash-run-sandbox-git");
    let init = Command::new("git").arg("init").arg(&root).output();
    if init.is_err() || !init.unwrap().status.success() {
        if let Some(value) = previous {
            env::set_var("NOLA_CLI_SANDBOX_BASH_PERMISSIVE", value);
        } else {
            env::remove_var("NOLA_CLI_SANDBOX_BASH_PERMISSIVE");
        }
        let _ = fs::remove_dir_all(root);
        return;
    }
    let ctx = ToolContext {
        memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
        language_services: Default::default(),
        skills: Default::default(),
        allowed_dirs: vec![root.clone()],
        policy: crate::tools::ToolExecutionPolicy::new(vec!["git".to_string()], true, "allow".to_string(), Vec::new()),
        permission_scope: Default::default(),
        cancellation: Default::default(),
        plan_state: Default::default(),
        output_store: crate::tools::ToolOutputStore::for_test(),
        browser: BrowserRuntime::new(temp_root("browser-runtime")),
    };
    let mut args = Map::new();
    args.insert("cwd".to_string(), Value::String(root.display().to_string()));
    args.insert(
        "command".to_string(),
        Value::String("git status | head -1".to_string()),
    );

    let readonly = run_bash(&args, &ctx);

    assert!(!readonly.error, "{}", readonly.text);
    args.insert(
        "command".to_string(),
        Value::String("git commit -m nope".to_string()),
    );

    let blocked = run_bash(&args, &ctx);

    assert!(blocked.error);
    assert!(blocked
        .text
        .contains("mutating or network git commands are disabled"));
    if let Some(value) = previous {
        env::set_var("NOLA_CLI_SANDBOX_BASH_PERMISSIVE", value);
    } else {
        env::remove_var("NOLA_CLI_SANDBOX_BASH_PERMISSIVE");
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn tool_search_dispatch_keeps_trusted_catalog_out_of_model_arguments() {
    let directory = tempfile::tempdir().unwrap();
    let ctx = ToolContext {
        memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
        language_services: Default::default(),
        skills: Default::default(), allowed_dirs: vec![directory.path().to_path_buf()],
        policy: ToolExecutionPolicy::new(Vec::new(), true, "deny".into(), Vec::new()),
        permission_scope: Default::default(), cancellation: Default::default(),
        plan_state: Default::default(), output_store: ToolOutputStore::for_test(),
        browser: BrowserRuntime::new(directory.path().join("browser")),
    };
    let catalog = ToolDiscoveryCatalog {
        available: ["read_file".to_string()].into_iter().collect(),
        ranked: vec!["read_file".into()], schemas: vec![json!({"name":"read_file",
            "description":"Frozen reader", "parameters":{"type":"object"}})],
    };
    for query in ["select:read_file", "继续查看", "search", "current"] {
        let mut request = ToolRequest { id: "discovery".into(), name: "tool_search".into(),
            args: serde_json::from_value(json!({"query": query, "max_results": 3})).unwrap() };
        let outcome = execute_authorized_tool_with_catalog(&request, &ctx, ExecutionAuthority::CoreApproved(nola_domain::PermissionProfile::Cautious), Some(&catalog));
        assert!(!outcome.error, "{}", outcome.text);
        assert_eq!(outcome.activation.unwrap().capability_ids, vec!["read_file"]);
        // Even trusted execution authority cannot authorize forged metadata.
        request.args.insert("_nola_available_tool_ids".into(), json!(["write_file"]));
        let rejected = execute_authorized_tool_with_catalog(&request, &ctx, ExecutionAuthority::CoreApproved(nola_domain::PermissionProfile::Cautious), Some(&catalog));
        assert!(rejected.error, "{}", rejected.text);
        assert!(rejected.activation.is_none());
    }
    let empty = ToolDiscoveryCatalog { available: HashSet::new(), ranked: vec![], schemas: vec![] };
    let request = ToolRequest { id: "empty".into(), name: "tool_search".into(),
        args: serde_json::from_value(json!({"query":"select:read_file"})).unwrap() };
    let result = execute_authorized_tool_with_catalog(&request, &ctx, ExecutionAuthority::CoreApproved(nola_domain::PermissionProfile::Cautious), Some(&empty));
    assert!(!result.error, "{}", result.text);
    assert!(result.activation.is_none());
}

#[test]
fn tool_search_returns_exact_tool_schema() {
    let mut args = Map::new();
    args.insert(
        "query".to_string(),
        Value::String("select:read_file".to_string()),
    );
    args.insert("max_results".to_string(), Value::from(3));

    let outcome = tool_search(&args, &crate::skill_runtime::SkillRuntimeContext::default());

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("mode: select"));
    assert!(outcome.text.contains("<functions>"));
    assert!(outcome.text.contains("tool_search_meta:"));
    assert!(outcome.text.contains(r#""reason":"ok""#));
    assert!(outcome
        .text
        .contains(r#""next_action":"proceed_with_matches""#));
    assert!(outcome
        .text
        .contains(r#""next_action_reason":"matches_available""#));
    assert!(outcome.text.contains(r#""name":"read_file""#));
    assert!(outcome.text.contains(r#""parameters""#));
    assert!(!outcome.text.contains(r#""name":"write_file""#));
    assert!(outcome.display.contains("1 tool"));
}

#[test]
fn tool_search_never_returns_tools_outside_the_active_host_catalog() {
    let mut args = Map::new();
    args.insert(
        "query".to_string(),
        Value::String("select:write_file,system_time".to_string()),
    );
    args.insert("max_results".to_string(), Value::from(5));
    args.insert(
        "_nola_available_tool_ids".to_string(),
        Value::Array(vec![
            Value::String("tool_search".to_string()),
            Value::String("system_time".to_string()),
        ]),
    );

    let outcome = tool_search(&args, &crate::skill_runtime::SkillRuntimeContext::default());

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains(r#""name":"system_time""#));
    assert!(!outcome.text.contains(r#""name":"write_file""#));
    assert!(outcome.text.contains("missing: write_file"));
    assert!(outcome.text.contains(r#""matches":["system_time"]"#));
}

#[test]
fn tool_search_select_combines_dynamic_and_static_catalog_entries_in_requested_order() {
    let mut args = Map::new();
    args.insert(
        "query".to_string(),
        Value::String("select:calendar_create,read_file".to_string()),
    );
    args.insert("max_results".to_string(), Value::from(1));
    args.insert(
        "_nola_available_tool_ids".to_string(),
        Value::Array(vec![
            Value::String("calendar_create".to_string()),
            Value::String("read_file".to_string()),
        ]),
    );
    args.insert(
        "_nola_dynamic_tool_schemas".to_string(),
        Value::Array(vec![json!({
            "type": "function",
            "function": {
                "name": "calendar_create",
                "description": "Create a calendar entry",
                "parameters": {"type": "object"}
            }
        })]),
    );

    let outcome = tool_search(&args, &crate::skill_runtime::SkillRuntimeContext::default());

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome
        .text
        .contains(r#""matches":["calendar_create","read_file"]"#));
    assert!(outcome.text.contains(r#""name":"calendar_create""#));
    assert!(outcome.text.contains(r#""name":"read_file""#));
    assert_eq!(
        outcome.activation.expect("activation").capability_ids,
        vec!["calendar_create", "read_file"]
    );
}

#[test]
fn tool_search_runtime_and_static_catalogs_share_missing_name_recovery() {
    let mut args = serde_json::from_value::<Map<String, Value>>(json!({
        "query": "select:git_diff,read_fiel",
        "_nola_available_tool_ids": ["git_diff", "read_file"],
    })).unwrap();
    let static_result = tool_search(&args, &crate::skill_runtime::SkillRuntimeContext::default());
    args.insert("_nola_dynamic_tool_schemas".into(), json!([{
        "name": "git_diff", "description": "Runtime diff schema",
        "parameters": {"type":"object", "properties":{"root":{"type":"string"}}}
    }]));
    let dynamic_result = tool_search(&args, &crate::skill_runtime::SkillRuntimeContext::default());
    let metadata = |outcome: &ToolOutcome| -> Value {
        serde_json::from_str(outcome.text.lines()
            .find_map(|line| line.strip_prefix("tool_search_meta: ")).unwrap()).unwrap()
    };
    assert_eq!(metadata(&static_result), metadata(&dynamic_result));
    assert_eq!(metadata(&dynamic_result)["reason"], "missing_tools");
    assert!(metadata(&dynamic_result)["suggestions"].as_array().unwrap().contains(&json!("read_file")));
    assert_eq!(dynamic_result.activation.unwrap().capability_ids, vec!["git_diff"]);

    // A typo in a runtime-only capability is recoverable as well, and a
    // disabled capability must never leak through the suggestion pool.
    args.insert("query".into(), json!("select:calendar_creat"));
    args.insert("_nola_available_tool_ids".into(), json!(["calendar_create"]));
    args.insert("_nola_dynamic_tool_schemas".into(), json!([
        {"name":"calendar_create", "description":"Create event", "parameters":{"type":"object"}},
        {"name":"calendar_creat_disabled", "description":"Disabled", "parameters":{"type":"object"}}
    ]));
    let result = tool_search(&args, &crate::skill_runtime::SkillRuntimeContext::default());
    assert_eq!(metadata(&result)["suggestions"], json!(["calendar_create"]));
    assert!(result.activation.is_none());
    assert!(!result.text.contains("calendar_creat_disabled"));
}

#[test]
fn tool_search_uses_core_semantic_rank_for_multilingual_natural_followups() {
    for query in ["那就执行刚才的操作", "Please continue with that", "search", "current"] {
        let args = serde_json::from_value::<Map<String, Value>>(json!({
            "query": query,
            "_nola_available_tool_ids": ["bash", "read_file", "device_action"],
            "_nola_core_tool_search_matches": ["device_action", "read_file"],
            "_nola_dynamic_tool_schemas": [{"name":"device_action", "description":"Opaque native operation", "parameters":{"type":"object"}}]
        })).unwrap();
        let result = tool_search(&args, &crate::skill_runtime::SkillRuntimeContext::default());
        assert!(!result.error);
        assert_eq!(result.activation.unwrap().capability_ids, vec!["device_action", "read_file"]);
    }
}

#[test]
fn tool_search_filters_do_not_get_bypassed_by_dynamic_catalog_entries() {
    let mut args = Map::new();
    args.insert(
        "query".to_string(),
        Value::String("read_only:true select:calendar_create".to_string()),
    );
    args.insert(
        "_nola_available_tool_ids".to_string(),
        Value::Array(vec![Value::String("calendar_create".to_string())]),
    );
    args.insert(
        "_nola_dynamic_tool_schemas".to_string(),
        Value::Array(vec![json!({
            "type": "function",
            "function": {
                "name": "calendar_create",
                "description": "Create a calendar entry",
                "parameters": {"type": "object"}
            }
        })]),
    );

    let outcome = tool_search(&args, &crate::skill_runtime::SkillRuntimeContext::default());

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("No tools matched query"));
    assert!(!outcome.text.contains(r#""name":"calendar_create""#));
}

#[test]
fn tool_search_select_does_not_truncate_with_max_results() {
    let mut args = Map::new();
    args.insert(
        "query".to_string(),
        Value::String("select:read_file,edit_file".to_string()),
    );
    args.insert("max_results".to_string(), Value::from(1));

    let outcome = tool_search(&args, &crate::skill_runtime::SkillRuntimeContext::default());

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains(r#""name":"read_file""#));
    assert!(outcome.text.contains(r#""name":"edit_file""#));
}

#[test]
fn tool_search_keyword_search_ranks_matching_tools() {
    let mut args = Map::new();
    args.insert(
        "query".to_string(),
        Value::String("notebook jupyter".to_string()),
    );
    args.insert("max_results".to_string(), Value::from(5));

    let outcome = tool_search(&args, &crate::skill_runtime::SkillRuntimeContext::default());

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("mode: keyword"));
    assert!(outcome.text.contains("notebook_read"));
    assert!(outcome.text.contains("<function>"));
}

#[test]
fn tool_search_discovers_file_inspection_from_declared_format_metadata() {
    for query in [
        "inspect audio",
        "audio transcription",
        "analyze an MP3 recording",
        "FLAC waveform frequencies",
        "WAV sample rate and channels",
    ] {
        let mut args = Map::new();
        args.insert("query".to_string(), Value::String(query.to_string()));
        args.insert("max_results".to_string(), Value::from(5));

        let outcome = tool_search(&args, &crate::skill_runtime::SkillRuntimeContext::default());

        assert!(!outcome.error, "{query}: {}", outcome.text);
        assert!(
            outcome.text.contains(r#""name":"read_file""#),
            "read_file missing for {query:?}: {}",
            outcome.text
        );
    }
}

#[test]
fn tool_search_rare_schema_metadata_outranks_common_name_substrings() {
    let names = (0..24).map(|index| format!("analysis_{index}")).collect::<Vec<_>>();
    let mut catalog = names.iter().map(|name| ToolSpec {
        name: name.as_str(),
        description: "Analyze an ordinary document.",
        parameters: json!({"type":"object"}),
        read_only: true,
    }).collect::<Vec<_>>();
    catalog.push(ToolSpec {
        name: "format_inspector",
        description: "Inspect ZX42 packets.",
        parameters: json!({"type":"object"}),
        read_only: true,
    });
    let (matches, mode, ..) = select_tool_specs(&catalog, "analyze an ZX42 packet", 1);
    assert_eq!(mode, "keyword");
    assert_eq!(matches.first().map(|spec| spec.name), Some("format_inspector"));
}

#[test]
fn memory_save_schema_distinguishes_zoned_timestamps_from_date_only_values() {
    let memory_save = specs()
        .into_iter()
        .find(|spec| spec.name == "memory_save")
        .expect("memory_save schema");
    let properties = memory_save.parameters["properties"]
        .as_object()
        .expect("memory_save properties");

    for field in [
        "mentioned_at",
        "event_time",
        "range_start",
        "range_end",
        "expires_at",
    ] {
        let description = properties[field]["description"]
            .as_str()
            .expect("temporal description");
        assert!(
            description.contains("explicit UTC offset"),
            "{field}: {description}"
        );
        assert!(description.contains("YYYY-MM-DD"), "{field}: {description}");
        assert!(
            description.contains("offsetless date-time"),
            "{field}: {description}"
        );
    }
}

#[test]
fn memory_save_schema_preserves_atomic_claim_and_update_semantics() {
    let memory_save = specs()
        .into_iter()
        .find(|spec| spec.name == "memory_save")
        .expect("memory_save schema");
    let properties = memory_save.parameters["properties"]
        .as_object()
        .expect("memory_save properties");

    let content = properties["content"]["description"]
        .as_str()
        .expect("content description");
    assert!(content.contains("atomic durable claim"));
    assert!(content.contains("decisive predicates"));

    let event_time = properties["event_time"]["description"]
        .as_str()
        .expect("event_time description");
    assert!(event_time.contains("relative time"));
    assert!(event_time.contains("unambiguous"));

    let supersedes = properties["supersedes"]["description"]
        .as_str()
        .expect("supersedes description");
    assert!(supersedes.contains("older claims"));
    assert!(supersedes.contains("Search existing memory"));
}

#[test]
fn tool_search_supports_search_hints() {
    let mut args = Map::new();
    args.insert("query".to_string(), Value::String("codeintel".to_string()));
    args.insert("max_results".to_string(), Value::from(5));

    let outcome = tool_search(&args, &crate::skill_runtime::SkillRuntimeContext::default());

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("mode: keyword"));
    assert!(
        outcome.text.contains("read_symbol")
            || outcome.text.contains("workspace_symbols")
            || outcome.text.contains("implementation_search")
            || outcome.text.contains("code_navigation")
    );
}

#[test]
fn tool_search_supports_exact_name_mode() {
    let mut args = Map::new();
    args.insert("query".to_string(), Value::String("read_file".to_string()));
    args.insert("max_results".to_string(), Value::from(5));

    let outcome = tool_search(&args, &crate::skill_runtime::SkillRuntimeContext::default());

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("mode: exact"));
    assert!(outcome.text.contains(r#""name":"read_file""#));
    assert!(!outcome.text.contains(r#""name":"write_file""#));
}

#[test]
fn tool_search_supports_prefix_mode() {
    let mut args = Map::new();
    args.insert(
        "query".to_string(),
        Value::String("tool_output_".to_string()),
    );
    args.insert("max_results".to_string(), Value::from(3));

    let outcome = tool_search(&args, &crate::skill_runtime::SkillRuntimeContext::default());

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("mode: prefix"));
    assert!(outcome.text.contains("tool_output_"));
}

#[test]
fn tool_search_supports_read_only_filters() {
    let mut args = Map::new();
    args.insert(
        "query".to_string(),
        Value::String("read_only=false select:read_file,edit_file".to_string()),
    );
    args.insert("max_results".to_string(), Value::from(5));

    let outcome = tool_search(&args, &crate::skill_runtime::SkillRuntimeContext::default());

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("filters: read_only=false"));
    assert!(outcome.text.contains(r#""name":"edit_file""#));
    assert!(!outcome.text.contains(r#""name":"read_file""#));
}

#[test]
fn tool_search_select_reports_missing_tools() {
    let mut args = Map::new();
    args.insert(
        "query".to_string(),
        Value::String("select:read_file,not_a_tool".to_string()),
    );
    args.insert("max_results".to_string(), Value::from(5));

    let outcome = tool_search(&args, &crate::skill_runtime::SkillRuntimeContext::default());

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("mode: select"));
    assert!(outcome.text.contains("missing: not_a_tool"));
    assert!(outcome.text.contains(r#""name":"read_file""#));
    assert!(outcome.text.contains(r#""retry_query":null"#));
    assert!(outcome.text.contains(r#""next_action":"proceed_with_matches""#));
    assert!(outcome.text.contains("read_file"));
}

#[test]
fn tool_search_select_no_match_reports_missing_tools() {
    let mut args = Map::new();
    args.insert(
        "query".to_string(),
        Value::String("select:definitely_missing_tool".to_string()),
    );
    args.insert("max_results".to_string(), Value::from(5));

    let outcome = tool_search(&args, &crate::skill_runtime::SkillRuntimeContext::default());

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("No tools matched query"));
    assert!(outcome.text.contains("missing: definitely_missing_tool"));
    assert!(outcome.text.contains("tool_search_meta:"));
    assert!(outcome.text.contains(r#""reason":"missing_tools""#));
    assert!(outcome.text.contains(r#""filtered_out_names":[]"#));
    assert!(outcome
        .text
        .contains(r#""missing_names":["definitely_missing_tool"]"#));
    assert!(outcome.text.contains(r#""active_filters":[]"#));
    assert!(outcome
        .text
        .contains(r#""requested_select":["definitely_missing_tool"]"#));
}

#[test]
fn tool_search_reports_suggestions_for_misspelled_select_name() {
    let mut args = Map::new();
    args.insert(
        "query".to_string(),
        Value::String("select:read_fiel".to_string()),
    );
    args.insert("max_results".to_string(), Value::from(5));

    let outcome = tool_search(&args, &crate::skill_runtime::SkillRuntimeContext::default());

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("No tools matched query"));
    assert!(outcome.text.contains("suggestions:"));
    assert!(outcome.text.contains("read_file"));
    assert!(outcome.text.contains(r#""retry_query":null"#));
    assert!(outcome
        .text
        .contains(r#""next_action":"consider_alternatives_or_report_limitation""#));
    assert!(outcome
        .text
        .contains(r#""next_action_reason":"exact_identifiers_absent_from_current_catalog""#));
}

#[test]
fn tool_search_missing_candidates_remain_suggestions_not_automatic_retries() {
    let mut args = Map::new();
    args.insert(
        "query".to_string(),
        Value::String("select:read_fiel,edti_file".to_string()),
    );
    args.insert("max_results".to_string(), Value::from(5));

    let outcome = tool_search(&args, &crate::skill_runtime::SkillRuntimeContext::default());

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains(r#""retry_query":null"#));
    assert!(outcome.text.contains("read_file"));
    assert!(outcome.text.contains("edit_file"));
    assert!(outcome.text.contains("suggestions:"));
    assert!(!outcome.text.contains("retry_query: select:"));
}

#[test]
fn tool_search_suggestions_respect_category_filter() {
    let mut args = Map::new();
    args.insert(
        "query".to_string(),
        Value::String("category:web_safe select:read_fiel".to_string()),
    );
    args.insert("max_results".to_string(), Value::from(5));

    let outcome = tool_search(&args, &crate::skill_runtime::SkillRuntimeContext::default());

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("No tools matched query"));
    assert!(!outcome.text.contains("read_file"));
    assert!(outcome.text.contains(r#""reason":"missing_tools""#));
    assert!(outcome
        .text
        .contains(r#""active_filters":["category=web_safe"]"#));
    assert!(outcome.text.contains(r#""requested_select":["read_fiel"]"#));
    if outcome.text.contains("retry_query: ") {
        assert!(outcome.text.contains("retry_query: category:web_safe "));
    }
}

#[test]
fn tool_search_reason_filtered_out_for_select_with_category() {
    let mut args = Map::new();
    args.insert(
        "query".to_string(),
        Value::String("category:web_safe select:read_file".to_string()),
    );
    args.insert("max_results".to_string(), Value::from(5));

    let outcome = tool_search(&args, &crate::skill_runtime::SkillRuntimeContext::default());

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("No tools matched query"));
    assert!(outcome.text.contains(r#""reason":"filtered_out""#));
    assert!(outcome
        .text
        .contains(r#""filtered_out_names":["read_file"]"#));
    assert!(outcome.text.contains(r#""missing_names":[]"#));
    assert!(outcome.text.contains(r#""requested_select":["read_file"]"#));
}

#[test]
fn tool_search_reason_no_keyword_match() {
    let mut args = Map::new();
    args.insert(
        "query".to_string(),
        Value::String("zzzz_not_a_real_tool_keyword".to_string()),
    );
    args.insert("max_results".to_string(), Value::from(5));

    let outcome = tool_search(&args, &crate::skill_runtime::SkillRuntimeContext::default());

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("No tools matched query"));
    assert!(outcome.text.contains(r#""reason":"no_keyword_match""#));
    assert!(outcome.text.contains(r#""filtered_out_names":[]"#));
    assert!(outcome.text.contains(r#""missing_names":[]"#));
    assert!(outcome.text.contains(r#""requested_select":[]"#));
}

#[test]
fn tool_search_next_action_relax_filters_when_filtered_out_without_retry() {
    let mut args = Map::new();
    args.insert(
        "query".to_string(),
        Value::String("category:web_safe has_param:nonexistent_param".to_string()),
    );

    let outcome = tool_search(&args, &crate::skill_runtime::SkillRuntimeContext::default());

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains(r#""reason":"filtered_out""#));
    assert!(outcome.text.contains(r#""retry_query":null"#));
    assert!(outcome.text.contains(r#""next_action":"relax_filters""#));
    assert!(outcome
        .text
        .contains(r#""next_action_reason":"active_filters_excluded_all_candidates""#));
}

#[test]
fn tool_search_next_action_broaden_keywords_without_retry_query() {
    let mut args = Map::new();
    args.insert("query".to_string(), Value::String("qzxjkvb".to_string()));
    args.insert("max_results".to_string(), Value::from(5));

    let outcome = tool_search(&args, &crate::skill_runtime::SkillRuntimeContext::default());

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains(r#""reason":"no_keyword_match""#));
    assert!(outcome.text.contains(r#""retry_query":null"#));
    assert!(outcome.text.contains(r#""next_action":"broaden_keywords""#));
    assert!(outcome
        .text
        .contains(r#""next_action_reason":"no_retry_query_available""#));
}

#[test]
fn tool_search_supports_exclude_filters() {
    let mut args = Map::new();
    args.insert(
        "query".to_string(),
        Value::String("exclude:edit_file select:read_file,edit_file".to_string()),
    );
    args.insert("max_results".to_string(), Value::from(5));

    let outcome = tool_search(&args, &crate::skill_runtime::SkillRuntimeContext::default());

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("mode: select"));
    assert!(outcome.text.contains("filters: exclude=edit_file"));
    assert!(outcome.text.contains(r#""name":"read_file""#));
    assert!(!outcome.text.contains(r#""name":"edit_file""#));
    assert!(outcome.text.contains("missing: edit_file"));
}

#[test]
fn tool_search_supports_category_filters() {
    let mut args = Map::new();
    args.insert(
        "query".to_string(),
        Value::String("category:web_safe select:web_search,read_file".to_string()),
    );
    args.insert("max_results".to_string(), Value::from(5));

    let outcome = tool_search(&args, &crate::skill_runtime::SkillRuntimeContext::default());

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("filters: category=web_safe"));
    assert!(outcome.text.contains(r#""name":"web_search""#));
    assert!(!outcome.text.contains(r#""name":"read_file""#));
    assert!(outcome.text.contains("missing: read_file"));
}

#[test]
fn tool_search_exclude_applies_to_keyword_mode() {
    let mut args = Map::new();
    args.insert(
        "query".to_string(),
        Value::String("tool_output exclude:tool_output_*".to_string()),
    );
    args.insert("max_results".to_string(), Value::from(5));

    let outcome = tool_search(&args, &crate::skill_runtime::SkillRuntimeContext::default());

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("No tools matched query"));
}

#[test]
fn tool_search_supports_has_param_filters() {
    let mut args = Map::new();
    args.insert(
        "query".to_string(),
        Value::String("has_param:dry_run select:read_file,edit_file".to_string()),
    );
    args.insert("max_results".to_string(), Value::from(5));

    let outcome = tool_search(&args, &crate::skill_runtime::SkillRuntimeContext::default());

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("filters: has_param=dry_run"));
    assert!(outcome.text.contains(r#""name":"edit_file""#));
    assert!(!outcome.text.contains(r#""name":"read_file""#));
}

#[test]
fn large_tool_result_is_persisted_and_readable() {
    let fixture = tempfile::tempdir().unwrap();
    let store = ToolOutputStore::new(fixture.path().to_path_buf());
    let original = format!("{}TAIL-MARKER", "A".repeat(MAX_TOOL_RESULT_CHARS + 200));
    let outcome = clamp_outcome_with_writer(ok(original, "large".to_string()), MAX_TOOL_RESULT_CHARS, |text| store.persist(text));

    assert!(!outcome.error, "{}", outcome.text);
    let preview = outcome.data.as_ref().expect("typed saved output preview");
    assert_eq!(preview["truncated"], true);
    assert_eq!(preview["full_output_ref"].as_str(), outcome.full_output_ref.as_deref());
    assert!(outcome.full_output_ref.is_some());
    assert!(outcome.text.contains("tool_output_read"));
    assert!(!outcome.text.contains("TAIL-MARKER"));

    let path = full_output_ref_from_outcome(&outcome).expect("saved output path");
    let mut args = Map::new();
    args.insert("path".to_string(), Value::String(path));
    args.insert(
        "offset".to_string(),
        Value::from((MAX_TOOL_RESULT_CHARS + 150) as i64),
    );
    args.insert("max_bytes".to_string(), Value::from(500));

    let readback = tool_output_read_in_store(&args, &store);

    assert!(!readback.error, "{}", readback.text);
    assert!(readback.text.contains("TAIL-MARKER"));
    assert!(serde_json::from_str::<Value>(&readback.text).unwrap()["has_more"].is_boolean());

    args.insert(
        "pattern".to_string(),
        Value::String("TAIL-MARKER".to_string()),
    );
    args.remove("offset");
    args.remove("max_bytes");
    let searched = tool_output_search_in_store(&args, &store);

    assert!(!searched.error, "{}", searched.text);
    assert!(searched.text.contains("TAIL-MARKER"));
    assert!(searched.text.contains("matches: 1"));

    let mut list_args = Map::new();
    list_args.insert("limit".to_string(), Value::from(5));
    let listed = tool_output_list_in_store(&list_args, &store);

    assert!(!listed.error, "{}", listed.text);
    assert!(listed.text.contains("size_bytes="));
    assert!(listed.text.contains(store.root().to_str().unwrap()));

    let mut find_args = Map::new();
    find_args.insert(
        "pattern".to_string(),
        Value::String("TAIL-MARKER".to_string()),
    );
    find_args.insert("max_outputs".to_string(), Value::from(5));
    let found = tool_output_find_in_store(&find_args, &store);

    assert!(!found.error, "{}", found.text);
    assert!(found.text.contains("Tool output find"));
    assert!(found.text.contains("TAIL-MARKER"));
    assert!(found.text.contains("matches:"));

    let unique = store.root().join("filename-only-match-cli.txt");
    fs::write(&unique, "unrelated content\n").unwrap();
    let mut filename_args = Map::new();
    filename_args.insert(
        "pattern".to_string(),
        Value::String("filename-only-match-cli".to_string()),
    );
    filename_args.insert("max_outputs".to_string(), Value::from(5));
    let filename_found = tool_output_find_in_store(&filename_args, &store);

    assert!(!filename_found.error, "{}", filename_found.text);
    assert!(filename_found
        .text
        .contains("path_match: filename-only-match-cli.txt"));
    assert!(filename_found
        .text
        .contains("matched saved output filename"));
}

#[test]
fn tool_output_search_merges_context_without_repeating_paths_or_lines() {
    let fixture = tempfile::tempdir().unwrap();
    let store = ToolOutputStore::new(fixture.path().to_path_buf());
    let source = (0..80).map(|index| format!("match-{index}\n")).collect::<String>();
    let path = store.persist(&source).unwrap();
    let args = serde_json::from_value::<Map<String, Value>>(json!({
        "path": path, "pattern": "match-", "context_lines": 5, "max_matches": 100,
    })).unwrap();
    let result = tool_output_search_in_store(&args, &store);
    assert!(!result.error);
    let body = result.text.split_once("\n\n").unwrap().1;
    assert_eq!(body.lines().count(), 80);
    assert_eq!(result.text.matches(path.canonicalize().unwrap().to_str().unwrap()).count(), 1);
    assert!(body.contains("match-79"));
    assert!(result.text.len() < 4_000);
    fs::remove_file(path).unwrap();
}

#[test]
fn tool_output_read_preserves_unicode_across_every_page_boundary() {
    let fixture = tempfile::tempdir().unwrap();
    let store = ToolOutputStore::new(fixture.path().to_path_buf());
    let original = "中文🙂\nlast line";
    let saved = store.persist(original).unwrap();
    for limit in 1..=8 {
        let mut offset = 0;
        let mut reconstructed = String::new();
        while offset < original.len() {
            let args = serde_json::from_value::<Map<String, Value>>(json!({
                "path": saved, "offset": offset, "max_bytes": limit,
            })).unwrap();
            let result = tool_output_read_in_store(&args, &store);
            assert!(!result.error, "{}", result.text);
            let page: Value = serde_json::from_str(&result.text).unwrap();
            let count = page["bytes_returned"].as_u64().unwrap() as usize;
            assert!(count > 0 && count <= limit + 3);
            let content = page["content"].as_str().unwrap();
            assert_eq!(count, content.len());
            reconstructed.push_str(content);
            offset += count;
            assert_eq!(page["next_offset"], offset);
        }
        assert_eq!(reconstructed, original);
    }
    let invalid = serde_json::from_value::<Map<String, Value>>(json!({"path": saved, "offset": 1})).unwrap();
    assert!(tool_output_read_in_store(&invalid, &store).error);
    fs::remove_file(saved).unwrap();
}

#[test]
fn tool_output_read_does_not_recursively_persist_a_large_readback() {
    let fixture = tempfile::tempdir().unwrap();
    let store = ToolOutputStore::new(fixture.path().to_path_buf());
    let original = format!("{}END-MARKER", "A".repeat(MAX_TOOL_RESULT_CHARS * 3));
    let persisted = clamp_outcome_with_writer(ok(original, "large".to_string()), MAX_TOOL_RESULT_CHARS, |text| store.persist(text));
    let path = full_output_ref_from_outcome(&persisted).expect("saved output path");
    let args = Map::from_iter([
        ("path".to_string(), Value::String(path)),
        ("offset".to_string(), Value::from(0)),
        // Reproduce the oversized request made by the real CLI review session.
        ("max_bytes".to_string(), Value::from(80_000)),
    ]);

    let readback = clamp_outcome_with_writer(tool_output_read_in_store(&args, &store), MAX_TOOL_RESULT_CHARS, |text| store.persist(text));

    assert!(!readback.error, "{}", readback.text);
    let page: Value = serde_json::from_str(&readback.text).unwrap();
    assert_eq!(page["bytes_returned"], MAX_TOOL_OUTPUT_SLICE_BYTES);
    assert_eq!(page["has_more"], true);
    assert_eq!(page["next_offset"], MAX_TOOL_OUTPUT_SLICE_BYTES);
    assert!(!readback.text.contains("full output saved to:"));
}

#[test]
fn public_web_url_validation_rejects_local_and_private_targets() {
    assert!(validate_public_web_url("https://example.com/path?q=1", "url").is_ok());
    assert!(validate_public_web_url("http://localhost:3000", "url").is_err());
    assert!(validate_public_web_url("http://127.0.0.1:3000", "url").is_err());
    assert!(validate_public_web_url("http://10.0.0.5", "url").is_err());
    assert!(validate_public_web_url("http://100.64.1.1", "url").is_err());
    assert!(validate_public_web_url("http://169.254.169.254/latest", "url").is_err());
    assert!(validate_public_web_url("http://192.0.2.10", "url").is_err());
    assert!(validate_public_web_url("http://[::1]:3000", "url").is_err());
    assert!(validate_public_web_url("http://[fd00::1]", "url").is_err());
    assert!(validate_public_web_url("http://[::ffff:127.0.0.1]", "url").is_err());
    assert!(validate_public_web_url("https://user:pass@example.com", "url").is_err());
    assert!(validate_public_web_url("file:///tmp/index.html", "url").is_err());
    assert!(validate_public_web_url("https://printer.local/status", "url").is_err());
    assert!(validate_public_web_url("https://service.internal/status", "url").is_err());
    assert!(validate_public_web_url("http://metadata", "url").is_err());
    assert!(validate_public_web_url("http://metadata.google.internal", "url").is_err());
}

#[test]
fn html_to_text_extracts_title_and_ignores_non_content_tags() {
    let html = r#"
            <html>
                <head>
                    <title>Nola &amp; CLI</title>
                    <style>.hidden { display: none; }</style>
                    <script>window.secret = "token";</script>
                </head>
                <body>
                    <h1>Hello&nbsp;world</h1>
                    <p>Use &lt;tools&gt; carefully.</p>
                </body>
            </html>
        "#;

    let (title, text) = html_to_text(html);

    assert_eq!(title, "Nola & CLI");
    assert!(text.contains("Hello world"));
    assert!(text.contains("Use <tools> carefully."));
    assert!(!text.contains("display"));
    assert!(!text.contains("secret"));
    assert!(!text.contains("Nola & CLI"));
}

#[test]
fn duckduckgo_result_parser_normalizes_redirects_and_strips_html() {
    let html = r#"
            <div class="result">
                <a href="/l/?kh=-1&amp;uddg=https%3A%2F%2Fexample.com%2Fdocs%3Fa%3D1" class="result__a">
                    <b>Nola</b> Docs
                </a>
                <div class="result__snippet">Read &amp; use <b>Nola</b>.</div>
            </div>
            <div class="result">
                <a class="result__a other" href="https://example.org/post">Second</a>
                <a class="result__snippet">Another result</a>
            </div>
        "#;

    let results = parse_duckduckgo_results(html, 10);

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].0, "Nola Docs");
    assert_eq!(results[0].1, "https://example.com/docs?a=1");
    assert_eq!(results[0].2, "Read & use Nola.");
    assert_eq!(results[1].0, "Second");
    assert_eq!(results[1].1, "https://example.org/post");
    assert_eq!(results[1].2, "Another result");
}

#[test]
fn google_result_parser_normalizes_redirects_filters_navigation_and_deduplicates() {
    let html = r#"
        <div class="g">
            <a href="/url?q=https%3A%2F%2Fexample.com%2Fdocs%3Fa%3D1&amp;sa=U">
                <h3><span>Nola</span> Docs</h3>
            </a>
            <div class="VwiC3b">Build &amp; run <em>Nola</em> locally.</div>
        </div>
        <div class="g">
            <a href="https://example.com/docs?a=1"><h3>Duplicate</h3></a>
            <div class="VwiC3b">Duplicate result.</div>
        </div>
        <div class="g">
            <a href="/search?q=nola"><h3>More Google results</h3></a>
        </div>
        <div class="g">
            <a href="https://example.org/guide"><h3>Second guide</h3></a>
            <span class="aCOpRe">A practical guide.</span>
        </div>
    "#;

    let results = parse_google_results(html, 10);

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].0, "Nola Docs");
    assert_eq!(results[0].1, "https://example.com/docs?a=1");
    assert_eq!(results[0].2, "Build & run Nola locally.");
    assert_eq!(results[1].0, "Second guide");
    assert_eq!(results[1].1, "https://example.org/guide");
    assert_eq!(results[1].2, "A practical guide.");
}

#[test]
fn bing_result_parser_extracts_organic_results_and_skips_tracking_links() {
    let html = r#"
        <ol id="b_results">
            <li class="b_algo">
                <h2><a href="https://example.com/reference"><strong>Nola</strong> Reference</a></h2>
                <div class="b_caption"><p>Read the &amp; complete reference.</p></div>
            </li>
            <li class="b_algo">
                <h2><a href="https://www.bing.com/ck/a?u=tracking">Advertisement</a></h2>
                <p>Not an organic target.</p>
            </li>
            <li class="b_algo">
                <h2><a href="//example.org/article">Article</a></h2>
                <p>Another useful result.</p>
            </li>
        </ol>
    "#;

    let results = parse_bing_results(html, 10);

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].0, "Nola Reference");
    assert_eq!(results[0].1, "https://example.com/reference");
    assert_eq!(results[0].2, "Read the & complete reference.");
    assert_eq!(results[1].0, "Article");
    assert_eq!(results[1].1, "https://example.org/article");
    assert_eq!(results[1].2, "Another useful result.");
}

#[test]
fn bing_rss_result_parser_extracts_keyless_search_results() {
    let xml = r#"
        <?xml version="1.0" encoding="utf-8"?>
        <rss version="2.0"><channel>
            <title>Bing: Nola</title>
            <item>
                <title>Nola &amp; Local AI</title>
                <link>https://example.com/nola</link>
                <description>Run &lt;b&gt;Nola&lt;/b&gt; locally &amp; privately.</description>
            </item>
            <item>
                <title>Second result</title>
                <link>https://example.org/second</link>
                <description>Another result.</description>
            </item>
        </channel></rss>
    "#;

    let results = parse_bing_results(xml, 1);

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, "Nola & Local AI");
    assert_eq!(results[0].1, "https://example.com/nola");
    assert_eq!(results[0].2, "Run Nola locally & privately.");
}

#[test]
fn web_search_schema_exposes_keyless_engine_selection() {
    let schema = tool_schemas()
        .into_iter()
        .find(|schema| schema["function"]["name"] == "web_search")
        .expect("web_search schema");
    assert_eq!(
        schema["function"]["parameters"]["properties"]["engine"]["enum"],
        json!([
            "auto",
            "bing",
            "bing_news",
            "google_news",
            "duckduckgo",
            "dashscope",
            "serper",
            "brave",
            "searxng",
            "google"
        ])
    );
    let query_description = schema["function"]["parameters"]["properties"]["query"]["description"]
        .as_str()
        .expect("query description");
    assert!(query_description.contains("passed unchanged"));
    assert!(query_description.contains("site:example.com"));
}

#[test]
fn system_time_is_exposed_as_a_read_only_clock_tool() {
    let schema = tool_schemas()
        .into_iter()
        .find(|schema| schema["function"]["name"] == "system_time")
        .expect("system_time schema");
    assert!(schema["function"]["description"]
        .as_str()
        .unwrap_or_default()
        .contains("before web_search"));
}

#[test]
fn query_encoding_matches_html_search_expectations() {
    assert_eq!(
        percent_encode_query("nola cli 中文"),
        "nola+cli+%E4%B8%AD%E6%96%87"
    );
    assert_eq!(percent_decode_form("nola+cli%2Fweb"), "nola cli/web");
}

#[test]
fn pinned_read_only_search_preserves_user_command_and_no_match_semantics() {
    let _guard = test_lock();
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("source.txt"), "hello\n").unwrap();
    let ctx = ToolContext {
        memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
        language_services: Default::default(), skills: Default::default(),
        allowed_dirs: vec![root.path().into()],
        policy: crate::tools::ToolExecutionPolicy::new(vec![], true, "allow".into(), vec![]),
        permission_scope: Default::default(), cancellation: Default::default(),
        plan_state: Default::default(), output_store: crate::tools::ToolOutputStore::for_test(),
        browser: BrowserRuntime::new(root.path().join("browser")),
    };
    for (command, matches) in [("grep hello source.txt | head -40", true),
        ("grep missing source.txt | head -40", false)] {
        let args = json!({"command":command,"cwd":root.path(),"operation_kind":"inspect"});
        let outcome = run_bash_with_permission(args.as_object().unwrap(), &ctx, true);
        assert!(!outcome.error, "{}", outcome.text);
        assert!(outcome.display.starts_with(&format!("$ {command}")), "{}", outcome.display);
        if !matches { assert!(outcome.display.contains("No matches found")); }
        let data = outcome.data.unwrap();
        assert!(data["command"].as_str().unwrap().contains("/usr/bin/env"), "execution must remain pinned");
        if !matches { assert_eq!(data["semantic_note"], "No matches found"); }
    }
}

#[test]
fn grep_reads_pages_beyond_two_hundred_and_reports_oversize() {
    let root = temp_root("grep-pages-budget");
    fs::create_dir_all(&root).unwrap();
    let file = root.join("source.txt");
    fs::write(&file, (0..450).map(|i| format!("needle {i}\n")).collect::<String>()).unwrap();
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
    let mut args = Map::new();
    args.insert("root".into(), Value::String(root.display().to_string()));
    args.insert("pattern".into(), Value::String("needle".into()));
    args.insert("output_mode".into(), Value::String("content".into()));
    args.insert("offset".into(), Value::from(200));
    let result = grep_files(&args, &ctx);
    assert!(!result.error, "{}", result.text);
    assert!(result.text.contains("needle 200"));
    assert!(result.text.contains("needle 399"));
    assert!(result.text.contains("scan incomplete"));
    fs::write(&file, vec![b'x';4_000_001]).unwrap();
    assert!(grep_files(&args,&ctx).error);
    let _ = fs::remove_dir_all(root);
}
#[test]
fn web_fetch_rejects_rebinding_and_private_redirect_targets_before_connecting() {
    let public = "1.1.1.1:443".parse().unwrap();
    let private = "127.0.0.1:80".parse().unwrap();
    assert!(validate_public_web_addresses(&[public]).is_ok());
    assert!(validate_public_web_addresses(&[public,private]).is_err());
    assert!(validate_public_web_addresses(&[]).is_err());
    let base = reqwest::Url::parse("https://example.com/page").unwrap();
    let target = base.join("http://169.254.169.254/latest").unwrap();
    assert!(validate_public_web_url(target.as_str(),"redirect_url").is_err());
    // A controlled local target must receive no connection, including when its
    // URL comes from a redirect Location resolved against a public origin.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let location = format!("http://{}/private", listener.local_addr().unwrap());
    let redirected = base.join(&location).unwrap();
    let origin = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let origin_url = format!("http://{}", origin.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let (mut stream, _) = origin.accept().unwrap();
        stream.set_read_timeout(Some(std::time::Duration::from_secs(2))).unwrap();
        let mut request = [0; 4096];
        std::io::Read::read(&mut stream, &mut request).unwrap();
        std::io::Write::write_all(&mut stream, format!(
            "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        ).as_bytes()).unwrap();
    });
    let client = reqwest::blocking::Client::builder().no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(2)).build().unwrap();
    let mut sends = 0;
    // Only the test transport maps the logical public origin to this fixture.
    // The production redirect gate must reject its Location before transport #2.
    let result = fetch_public_web_response(base.clone(), |_| {
        sends += 1;
        client.get(&origin_url).send().map_err(|e| error(e.to_string()))
    });
    server.join().unwrap();
    assert!(result.is_err());
    assert_eq!(sends, 1);
    let outcome = web_fetch(&serde_json::json!({"url": redirected.as_str()}).as_object().unwrap());
    assert!(outcome.error);
    assert!(matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock));
    assert!(validate_public_web_addresses(&["[::1]:443".parse().unwrap()]).is_err());
    assert!(validate_public_web_addresses(&["[::ffff:127.0.0.1]:443".parse().unwrap()]).is_err());
    assert!(validate_public_web_addresses(&["[2606:4700:4700::1111]:8443".parse().unwrap()]).is_ok());
}
