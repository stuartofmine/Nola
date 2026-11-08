use super::*;
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();
static TEMP_ROOT_COUNTER: AtomicU64 = AtomicU64::new(0);

#[test]
fn skill_invocation_schema_exposes_sorted_canonical_ids() {
    let schema = skill_invocation_schema_from_ids([
        "word-document-authoring".to_string(),
        "analyze-data".to_string(),
        "word-document-authoring".to_string(),
    ]);
    assert_eq!(
        schema["enum"],
        serde_json::json!(["analyze-data", "word-document-authoring"])
    );
    assert_eq!(schema["type"], "string");
}

fn test_mutex() -> &'static Mutex<()> {
    TEST_MUTEX.get_or_init(|| Mutex::new(()))
}

fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    test_mutex()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[test]
fn session_permission_scopes_do_not_share_runtime_grants() {
    let _lock = test_lock();
    let first = SessionPermissionScope::default();
    let second = SessionPermissionScope::default();

    {
        let _scope = first.enter();
        configure_session_permission_config(None);
        configure_session_permission_scope(None);
        add_session_bash_allow_prefix("cargo check".to_string());
        add_session_agent_allow_rule("review".to_string());
        assert_eq!(
            session_bash_allow_prefix_for("cargo check --workspace").as_deref(),
            Some("cargo check")
        );
        assert_eq!(
            session_agent_allow_rule_for("review").as_deref(),
            Some("review")
        );
    }

    {
        let _scope = second.enter();
        configure_session_permission_config(None);
        configure_session_permission_scope(None);
        assert!(session_bash_allow_prefix_for("cargo check --workspace").is_none());
        assert!(session_agent_allow_rule_for("review").is_none());
        add_session_bash_allow_prefix("cargo test".to_string());
    }

    {
        let _scope = first.enter();
        assert_eq!(
            session_bash_allow_prefix_for("cargo check --workspace").as_deref(),
            Some("cargo check")
        );
        assert!(session_bash_allow_prefix_for("cargo test").is_none());
    }
}

#[test]
fn conversation_approval_reuses_only_the_matching_bash_scope() {
    let _lock = test_lock();
    let scope = SessionPermissionScope::default();
    let _scope = scope.enter();
    configure_session_permission_config(None);
    configure_session_permission_scope(None);

    let approved = ToolRequest {
        id: "bash-approved".to_string(),
        name: "bash".to_string(),
        args: object_args(json!({"command": "python3 build.py"})),
    };
    remember_session_approval(&approved);

    let later = ToolRequest {
        id: "bash-later".to_string(),
        name: "bash".to_string(),
        args: object_args(json!({"command": "python3 build.py --verbose"})),
    };
    assert!(
        session_tool_allow_rule_for(&later).is_some(),
        "the approved command scope should be reusable in this conversation"
    );

    let unrelated = ToolRequest {
        id: "bash-unrelated".to_string(),
        name: "bash".to_string(),
        args: object_args(json!({"command": "make test"})),
    };
    assert!(
        session_tool_allow_rule_for(&unrelated).is_none(),
        "approving one bash prefix must not approve every shell command"
    );
}

#[test]
fn conversation_approval_reuses_rm_within_the_same_working_directory_only() {
    let _lock = test_lock();
    let scope = SessionPermissionScope::default();
    let _scope = scope.enter();
    configure_session_permission_config(None);
    configure_session_permission_scope(None);

    let approved = ToolRequest {
        id: "rm-approved".to_string(),
        name: "bash".to_string(),
        args: object_args(json!({"command": "rm first.txt", "cwd": "/tmp/project"})),
    };
    remember_session_approval(&approved);

    let same_scope = ToolRequest {
        id: "rm-same-scope".to_string(),
        name: "bash".to_string(),
        args: object_args(json!({"command": "rm second.txt", "cwd": "/tmp/project"})),
    };
    assert!(session_tool_allow_rule_for(&same_scope).is_some());

    let different_directory = ToolRequest {
        id: "rm-other-directory".to_string(),
        name: "bash".to_string(),
        args: object_args(json!({"command": "rm second.txt", "cwd": "/tmp/other"})),
    };
    assert!(session_tool_allow_rule_for(&different_directory).is_none());

    let different_command = ToolRequest {
        id: "find-delete".to_string(),
        name: "bash".to_string(),
        args: object_args(json!({
            "command": "find . -maxdepth 1 -name second.txt -delete",
            "cwd": "/tmp/project"
        })),
    };
    assert!(session_tool_allow_rule_for(&different_command).is_none());
}

#[test]
fn memory_text_search_relaxes_a_model_selected_kind_that_hides_the_saved_fact() {
    let root = temp_root("memory-soft-kind");
    let store = NolaStore::open(&root.join("nola.db")).unwrap();
    store
        .save_memory(
            "global",
            "reference",
            "Android verification code: ANDROID-MEMORY-OLD",
            Some("Android verification code"),
            &["android".to_string(), "verification".to_string()],
            Some("test"),
        )
        .unwrap();
    store
        .save_memory(
            "global",
            "other",
            "User's iOS verification code: IOS-MEMORY-CURRENT",
            Some("iOS verification code"),
            &["ios".to_string(), "verification".to_string()],
            Some("test"),
        )
        .unwrap();

    let (memories, relaxed) = search_memories_with_soft_kind(
        &store,
        &nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
        "global",
        Some("iOS verification code"),
        Some("reference"),
        10,
    )
    .unwrap();

    assert!(relaxed);
    assert_eq!(memories.len(), 1);
    assert_eq!(
        memories[0]["content"],
        "User's iOS verification code: IOS-MEMORY-CURRENT"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn ordinary_external_read_does_not_need_directory_authority() {
    let _lock = test_lock();
    let allowed = temp_root("auto-approve-allowed");
    let outside = temp_root("auto-approve-outside");
    let outside_file = outside.join("outside.txt");
    fs::write(&outside_file, "ok").unwrap();

    for auto_approve in [false, true] {
        let ctx = ToolContext {
            memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
            language_services: Default::default(),
            skills: Default::default(), allowed_dirs: vec![allowed.clone()],
            policy: ToolExecutionPolicy::new(Vec::new(), auto_approve, "allow".into(), Vec::new()),
            permission_scope: Default::default(), cancellation: Default::default(),
            plan_state: Default::default(), output_store: ToolOutputStore::for_test(),
            browser: BrowserRuntime::new(allowed.join("browser")),
        };
        // Both profiles admit ordinary filesystem paths after Core admission.
        let outcome = execute_authorized_tool(&ToolRequest {
            id: "outside-read".into(), name: "read_file".into(),
            args: object_args(json!({"path":outside_file})),
        }, &ctx, ExecutionAuthority::CoreApproved(nola_domain::PermissionProfile::Cautious));
        assert!(!outcome.error, "{}", outcome.text);
        assert!(outcome.text.contains("ok"));
        let resolved = outside_file.canonicalize().unwrap();
        let _scope = ctx.permission_scope.enter();
        assert!(effective_allowed_dirs(std::slice::from_ref(&allowed))
            .iter()
            .any(|base| resolved.starts_with(base)));
    }

    let _ = fs::remove_dir_all(allowed);
    let _ = fs::remove_dir_all(outside);
}

#[test]
fn ordinary_resources_are_available_across_profiles_and_keep_working_directory() {
    let fixture = tempfile::tempdir().unwrap();
    let workspace = fixture.path().join("workspace");
    let outside = fixture.path().join("outside");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&outside).unwrap();
    let file = outside.join("evidence.txt");
    fs::write(&file, "fixture").unwrap();
    let roots = vec![workspace.canonicalize().unwrap()];
    let scope = SessionPermissionScope::default();
    let other_scope = SessionPermissionScope::default();
    let _scope = scope.enter();
    with_permission_cache_mut(|cache| cache.unrestricted_resources = true);
    assert_eq!(effective_allowed_dirs(&roots)[0], roots[0]);
    assert_eq!(resolve_allowed(file.to_str().unwrap(), &roots, true).unwrap(), file.canonicalize().unwrap());
    assert!(resolve_allowed_entry(outside.join("new.txt").to_str().unwrap(), &roots).is_ok());
    assert!(!git_process_preservation(&outside, &roots).unwrap().active());
    {
        let _other = other_scope.enter();
        assert!(resolve_allowed(file.to_str().unwrap(), &roots, true).is_ok());
    }
    with_permission_cache_mut(|cache| cache.unrestricted_resources = false);
    assert!(resolve_allowed(file.to_str().unwrap(), &roots, true).is_ok());
    assert_eq!(effective_allowed_dirs(&roots)[0], roots[0]);
}

#[test]
fn allow_all_does_not_install_a_hidden_read_only_git_sandbox() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    for args in [vec!["init", "--quiet"], vec!["config", "user.name", "Fixture"],
        vec!["config", "user.email", "fixture@example.invalid"]] {
        assert!(process::Command::new("git").args(args).current_dir(root).status().unwrap().success());
    }
    fs::write(root.join("staged.txt"), "fixture").unwrap();
    assert!(process::Command::new("git").args(["add", "staged.txt"]).current_dir(root).status().unwrap().success());
    let scope = SessionPermissionScope::default();
    let _scope = scope.enter();
    assert!(git_process_preservation(root, &[]).unwrap().active());
    with_permission_cache_mut(|cache| cache.unrestricted_resources = true);
    assert!(!git_process_preservation(root, &[]).unwrap().active());
    with_permission_cache_mut(|cache| cache.unrestricted_resources = false);
    assert!(git_process_preservation(root, &[]).unwrap().active());
}

#[test]
fn file_read_evidence_is_isolated_between_distinct_host_contexts() {
    let _lock = test_lock();
    let root = temp_root("file-access-scope");
    let path = root.join("shared.rs");
    fs::write(&path, "fn shared() {}\n").unwrap();
    let first = SessionPermissionScope::default();
    let second = SessionPermissionScope::default();

    {
        let _scope = first.enter();
        let observed = read_file_text(&path, &Map::new(), true, 16_000, FileReadPresentationBudget::direct()).unwrap();
        record_returned_file_observations(&observed.text, observed.observation);
        assert!(existing_file_access_snapshot(&path, &Map::new())
            .unwrap()
            .is_some());
    }

    {
        let _scope = second.enter();
        let outcome = match existing_file_access_snapshot(&path, &Map::new()) {
            Err(outcome) => outcome,
            Ok(_) => panic!("a separate run must read the file for itself"),
        };
        assert!(outcome.text.contains("has not been read yet"));
    }

    fs::remove_dir_all(root).unwrap();
}

fn object_args(value: Value) -> Map<String, Value> {
    value
        .as_object()
        .expect("test args must be an object")
        .clone()
}

#[test]
fn glob_limit_caps_matches_without_truncating_the_directory_scan() {
    let root = temp_root("glob-match-limit");
    fs::write(root.join("decoy.txt"), "not the target\n").unwrap();
    fs::create_dir_all(root.join("nested")).unwrap();
    let target = root.join("nested/target.json");
    fs::write(&target, "{}\n").unwrap();
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

    let outcome = glob_files(
        &object_args(json!({
            "pattern": "nested/target.json",
            "limit": 1
        })),
        &ctx,
    );

    assert!(!outcome.error, "{}", outcome.text);
    assert_eq!(outcome.text, target.display().to_string());
    let _ = fs::remove_dir_all(root);
}

fn assert_browser_boolean_arg_is_strict(action: &str, base: Value, key: &str) {
    let args = object_args(base);
    let mut invalid = args.clone();
    invalid.insert(key.to_string(), Value::String("false".to_string()));
    assert!(
        validate_browser_tool_args(action, &invalid).is_err(),
        "{action} should reject string {key}"
    );

    let mut valid = args;
    valid.insert(key.to_string(), Value::Bool(false));
    assert!(
        validate_browser_tool_args(action, &valid).is_ok(),
        "{action} should accept boolean {key}"
    );
}

fn assert_computer_boolean_arg_is_strict(action: &str, base: Value, key: &str) {
    let args = object_args(base);
    let mut invalid = args.clone();
    invalid.insert(key.to_string(), Value::String("false".to_string()));
    assert!(
        validate_computer_tool_args(action, &invalid).is_err(),
        "{action} should reject string {key}"
    );

    let mut valid = args;
    valid.insert(key.to_string(), Value::Bool(false));
    assert!(
        validate_computer_tool_args(action, &valid).is_ok(),
        "{action} should accept boolean {key}"
    );
}

#[test]
fn python_bool_helpers_ignore_invalid_numeric_values() {
    let Some(python) = python_candidates().into_iter().find(|candidate| {
        process::Command::new(candidate)
            .arg("--version")
            .output()
            .is_ok()
    }) else {
        return;
    };

    let root = temp_root("python-bool-helpers");
    fs::create_dir_all(&root).unwrap();
    for (index, (helper, split_marker)) in [(COMPUTER_HELPER, "\ntry:\n    payload = json.loads")]
        .into_iter()
        .enumerate()
    {
        let helper_defs = helper.split(split_marker).next().unwrap();
        let script = format!(
            r#"{helper_defs}
print(json.dumps([
    bool_arg({{"flag": 1}}, "flag", False),
    bool_arg({{"flag": 0}}, "flag", True),
    bool_arg({{"flag": 2}}, "flag", False),
    bool_arg({{"flag": 2}}, "flag", True),
    bool_arg({{"flag": []}}, "flag", False),
    bool_arg({{"flag": []}}, "flag", True),
]))
"#
        );
        let script_path = root.join(format!("helper-{index}.py"));
        fs::write(&script_path, script).unwrap();
        let output = process::Command::new(&python)
            .arg(&script_path)
            .output()
            .expect("python helper should run");
        assert!(
            output.status.success(),
            "helper bool_arg check failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "[true, false, false, true, false, true]"
        );
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn bash_execution_mode_does_not_classify_command_keywords() {
    for command in ["npm run dev", "cargo test", "unknown-task --work"] {
        let mut args = Map::new();
        args.insert("command".to_string(), Value::String(command.to_string()));
        assert_eq!(
            bash_execution_mode(&args).unwrap(),
            BashExecutionMode::Foreground,
            "{command}"
        );
    }
}

#[test]
fn glob_regex_matches_recursive_rust_files() {
    let regex = glob_regex("**/*.rs").unwrap();
    assert!(regex.is_match("src/main.rs"));
    assert!(regex.is_match("a/b/c/lib.rs"));
    assert!(!regex.is_match("src/main.py"));
}

#[test]
fn resolve_missing_path_preserves_nested_components() {
    let root = temp_root("missing-path");
    let target = root.join("new").join("nested").join("file.txt");
    let resolved = resolve_missing_path(&target).unwrap();

    assert!(resolved.ends_with(Path::new("new").join("nested").join("file.txt")));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn resolve_allowed_accepts_ordinary_paths_outside_workspace() {
    let root = temp_root("allowed");
    let outside = temp_root("outside").join("file.txt");
    fs::write(&outside, "nope").unwrap();

    let resolved = resolve_allowed(outside.to_str().unwrap(), &[root.clone()], true).unwrap();
    assert_eq!(resolved, outside.canonicalize().unwrap());
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(outside.parent().unwrap());
}

#[test]
fn resolve_allowed_anchors_relative_paths_to_the_workspace() {
    let root = temp_root("relative-workspace");
    let target = root.join("nested").join("note.txt");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, "workspace-relative\n").unwrap();

    let resolved = resolve_allowed("nested/note.txt", std::slice::from_ref(&root), true)
        .expect("relative tool paths should resolve inside the workspace");

    assert_eq!(resolved, target.canonicalize().unwrap());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn sha256_hex_matches_known_vectors() {
    assert_eq!(
        sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn notebook_tools_read_and_edit_cells() {
    let root = temp_root("notebook-tools");
    let target = root.join("analysis.ipynb");
    fs::write(
        &target,
        serde_json::to_string(&json!({
            "cells": [
                {
                    "cell_type": "markdown",
                    "id": "intro",
                    "metadata": {},
                    "source": ["# Analysis\n"]
                },
                {
                    "cell_type": "code",
                    "id": "load",
                    "metadata": {},
                    "execution_count": 3,
                    "outputs": [{"output_type": "stream", "name": "stdout", "text": ["old\n"]}],
                    "source": ["x = 1\n", "print(x)\n"]
                }
            ],
            "metadata": {"language_info": {"name": "python"}},
            "nbformat": 4,
            "nbformat_minor": 5
        }))
        .unwrap(),
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
    let mut read_args = Map::new();
    read_args.insert(
        "path".to_string(),
        Value::String(target.display().to_string()),
    );
    read_args.insert("include_outputs".to_string(), Value::Bool(true));

    let read = notebook_read(&read_args, &ctx);

    assert!(!read.error, "{}", read.text);
    assert!(read
        .text
        .contains(&format!("# Notebook: {}", target.display())));
    assert!(read.text.contains("## Cell 0 [markdown] id=intro"));
    assert!(read.text.contains("## Cell 1 [code] id=load"));
    assert!(read.text.contains("   1 | x = 1"));
    assert!(read.text.contains("Outputs:"));
    assert!(read.text.contains("old"));

    let original_notebook_text = fs::read_to_string(&target).unwrap();
    let mut preview_args = Map::new();
    preview_args.insert(
        "path".to_string(),
        Value::String(target.display().to_string()),
    );
    preview_args.insert("cell_index".to_string(), Value::Number(1.into()));
    preview_args.insert(
        "edit_mode".to_string(),
        Value::String("replace".to_string()),
    );
    preview_args.insert("cell_type".to_string(), Value::String("code".to_string()));
    preview_args.insert(
        "source".to_string(),
        Value::String("x = 2\nprint(x)\n".to_string()),
    );
    preview_args.insert("dry_run".to_string(), Value::Bool(true));

    let preview = notebook_edit(&preview_args, &ctx);

    assert!(!preview.error, "{}", preview.text);
    assert!(preview.text.contains("dry_run: true"));
    assert!(preview.text.contains("## Diff"));
    assert!(preview.text.contains("x = 2\\n"));
    assert_eq!(fs::read_to_string(&target).unwrap(), original_notebook_text);

    let mut replace_args = Map::new();
    replace_args.insert(
        "path".to_string(),
        Value::String(target.display().to_string()),
    );
    replace_args.insert("cell_index".to_string(), Value::Number(1.into()));
    replace_args.insert(
        "edit_mode".to_string(),
        Value::String("replace".to_string()),
    );
    replace_args.insert("cell_type".to_string(), Value::String("code".to_string()));
    replace_args.insert(
        "source".to_string(),
        Value::String("x = 2\nprint(x)\n".to_string()),
    );

    let replaced = notebook_edit(&replace_args, &ctx);

    assert!(!replaced.error, "{}", replaced.text);
    let notebook: Value = serde_json::from_str(&fs::read_to_string(&target).unwrap()).unwrap();
    assert_eq!(
        notebook["cells"][1]["source"],
        json!(["x = 2\n", "print(x)\n"])
    );
    assert_eq!(notebook["cells"][1]["execution_count"], Value::Null);
    assert_eq!(notebook["cells"][1]["outputs"], json!([]));

    let mut insert_args = Map::new();
    insert_args.insert(
        "path".to_string(),
        Value::String(target.display().to_string()),
    );
    insert_args.insert("cell_index".to_string(), Value::Number(2.into()));
    insert_args.insert("edit_mode".to_string(), Value::String("insert".to_string()));
    insert_args.insert(
        "cell_type".to_string(),
        Value::String("markdown".to_string()),
    );
    insert_args.insert(
        "source".to_string(),
        Value::String("## Result\nDone\n".to_string()),
    );

    let inserted = notebook_edit(&insert_args, &ctx);

    assert!(!inserted.error, "{}", inserted.text);
    let notebook: Value = serde_json::from_str(&fs::read_to_string(&target).unwrap()).unwrap();
    assert_eq!(notebook["cells"].as_array().unwrap().len(), 3);
    assert_eq!(notebook["cells"][2]["cell_type"], "markdown");
    assert_eq!(
        notebook["cells"][2]["source"],
        json!(["## Result\n", "Done\n"])
    );
    assert!(notebook["cells"][2]["id"]
        .as_str()
        .unwrap()
        .starts_with("nola-"));

    let mut delete_args = Map::new();
    delete_args.insert(
        "path".to_string(),
        Value::String(target.display().to_string()),
    );
    delete_args.insert("cell_index".to_string(), Value::Number(0.into()));
    delete_args.insert("edit_mode".to_string(), Value::String("delete".to_string()));

    let deleted = notebook_edit(&delete_args, &ctx);

    assert!(!deleted.error, "{}", deleted.text);
    let notebook: Value = serde_json::from_str(&fs::read_to_string(&target).unwrap()).unwrap();
    assert_eq!(
        notebook["cells"]
            .as_array()
            .unwrap()
            .iter()
            .map(|cell| cell["cell_type"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["code", "markdown"]
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn move_and_delete_file_tools_operate_on_single_files() {
    let root = temp_root("move-delete");
    let source = root.join("src.txt");
    let destination = root.join("nested").join("dst.txt");
    fs::write(&source, "hello\n").unwrap();
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
    let mut move_args = Map::new();
    move_args.insert(
        "source".to_string(),
        Value::String(source.display().to_string()),
    );
    move_args.insert(
        "destination".to_string(),
        Value::String(destination.display().to_string()),
    );

    let moved = move_file(&move_args, &ctx);

    assert!(!moved.error, "{}", moved.text);
    assert!(!source.exists());
    assert_eq!(fs::read_to_string(&destination).unwrap(), "hello\n");

    let mut delete_args = Map::new();
    delete_args.insert(
        "path".to_string(),
        Value::String(destination.display().to_string()),
    );
    let deleted = delete_file(&delete_args, &ctx);

    assert!(!deleted.error, "{}", deleted.text);
    assert!(!destination.exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn write_file_supports_dry_run_diff() {
    let root = temp_root("write-dry-run");
    let target = root.join("config.txt");
    fs::write(&target, "old\n").unwrap();
    let digest = file_sha256_hex(&target).unwrap();
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
    args.insert("content".to_string(), Value::String("new\n".to_string()));
    args.insert("expected_sha256".to_string(), Value::String(digest));
    args.insert("dry_run".to_string(), Value::Bool(true));

    let preview = write_file(&args, &ctx);

    assert!(!preview.error, "{}", preview.text);
    assert!(preview.text.contains("dry_run: true"));
    assert!(preview.text.contains("## Diff"));
    assert!(preview.text.contains("-old"));
    assert!(preview.text.contains("+new"));
    assert_eq!(fs::read_to_string(&target).unwrap(), "old\n");

    let new_target = root.join("nested").join("created.txt");
    let mut new_args = Map::new();
    new_args.insert(
        "path".to_string(),
        Value::String(new_target.display().to_string()),
    );
    new_args.insert(
        "content".to_string(),
        Value::String("created\n".to_string()),
    );
    new_args.insert("dry_run".to_string(), Value::Bool(true));
    let new_preview = write_file(&new_args, &ctx);

    assert!(!new_preview.error, "{}", new_preview.text);
    assert!(new_preview.text.contains("+created"));
    assert!(!new_target.exists());
    assert!(!new_target.parent().unwrap().exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn write_file_auto_formats_rust_files() {
    let root = temp_root("write-auto-format");
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    let target = root.join("src").join("main.rs");
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
        "content".to_string(),
        Value::String("fn main(){println!(\"hi\");}\n".to_string()),
    );

    let outcome = write_file(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("## Formatting"));
    assert!(outcome.text.contains("cargo fmt"));
    assert_eq!(
        fs::read_to_string(&target).unwrap(),
        "fn main() {\n    println!(\"hi\");\n}\n"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn move_file_updates_js_ts_relative_imports() {
    let root = temp_root("move-imports");
    let src = root.join("src");
    let lib = root.join("lib");
    fs::create_dir_all(&src).unwrap();
    fs::create_dir_all(&lib).unwrap();
    let util = src.join("util.ts");
    let other = src.join("other.ts");
    let app = src.join("app.ts");
    let destination = lib.join("util.ts");
    fs::write(&util, "export const util = 1\n").unwrap();
    fs::write(&other, "export const other = 2\n").unwrap();
    fs::write(
        &app,
        "import { util } from './util'\nimport { other } from './other'\n",
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
    let mut move_args = Map::new();
    move_args.insert(
        "source".to_string(),
        Value::String(util.display().to_string()),
    );
    move_args.insert(
        "destination".to_string(),
        Value::String(destination.display().to_string()),
    );
    move_args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );
    move_args.insert("update_imports".to_string(), Value::Bool(true));

    // Function-level mutation fixture: supply an actual Host-resolved and
    // revalidated collection, rather than treating root access as approval.
    let request = ToolRequest { id:"move-import-function-fixture".into(), name:"move_file".into(), args:move_args.clone() };
    let spec = specs_with_skills(&ctx.skills).into_iter().find(|spec| spec.name == "move_file").unwrap();
    let resources = permission_resolution::LocalPermissionResources::prepare(&ctx,&spec,&request).unwrap();
    let expected = resources.revalidate(&ctx,&Value::Object(move_args.clone())).unwrap();
    let _resources = enter_permission_resource_expectations(expected);
    let moved = move_file(&move_args, &ctx);

    assert!(!moved.error, "{}", moved.text);
    assert!(moved.text.contains("src/app.ts:1: ./util -> ../lib/util"));
    assert_eq!(
        fs::read_to_string(&app).unwrap(),
        "import { util } from '../lib/util'\nimport { other } from './other'\n"
    );

    let mut diagnostic_args = Map::new();
    diagnostic_args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );
    diagnostic_args.insert(
        "paths".to_string(),
        json!([
            app.display().to_string(),
            destination.display().to_string(),
            other.display().to_string()
        ]),
    );
    let diagnostics = code_diagnostics(&diagnostic_args, &ctx);

    assert!(!diagnostics.error, "{}", diagnostics.text);
    assert!(!diagnostics.text.contains("unresolved relative import"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn move_file_rejects_overwrite_without_opt_in() {
    let root = temp_root("move-no-overwrite");
    let source = root.join("src.txt");
    let destination = root.join("dst.txt");
    fs::write(&source, "new\n").unwrap();
    fs::write(&destination, "old\n").unwrap();
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
        "source".to_string(),
        Value::String(source.display().to_string()),
    );
    args.insert(
        "destination".to_string(),
        Value::String(destination.display().to_string()),
    );

    let outcome = move_file(&args, &ctx);

    assert!(outcome.error);
    assert!(outcome.text.contains("Destination already exists"));
    assert!(source.exists());
    assert_eq!(fs::read_to_string(&destination).unwrap(), "old\n");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn move_and_delete_file_reject_stale_expected_sha256() {
    let root = temp_root("move-delete-stale");
    let source = root.join("src.txt");
    let destination = root.join("dst.txt");
    fs::write(&source, "original\n").unwrap();
    let original_digest = file_sha256_hex(&source).unwrap();
    fs::write(&source, "user edit\n").unwrap();
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
    let mut move_args = Map::new();
    move_args.insert(
        "source".to_string(),
        Value::String(source.display().to_string()),
    );
    move_args.insert(
        "destination".to_string(),
        Value::String(destination.display().to_string()),
    );
    move_args.insert(
        "expected_sha256".to_string(),
        Value::String(original_digest),
    );

    let moved = move_file(&move_args, &ctx);

    assert!(moved.error);
    assert!(moved.text.contains("expected_sha256 mismatch"));
    assert!(source.exists());
    assert!(!destination.exists());

    let delete_target = root.join("delete.txt");
    fs::write(&delete_target, "before\n").unwrap();
    let delete_digest = file_sha256_hex(&delete_target).unwrap();
    fs::write(&delete_target, "after\n").unwrap();
    let mut delete_args = Map::new();
    delete_args.insert(
        "path".to_string(),
        Value::String(delete_target.display().to_string()),
    );
    delete_args.insert("expected_sha256".to_string(), Value::String(delete_digest));

    let deleted = delete_file(&delete_args, &ctx);

    assert!(deleted.error);
    assert!(deleted.text.contains("expected_sha256 mismatch"));
    assert_eq!(fs::read_to_string(&delete_target).unwrap(), "after\n");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn move_and_delete_file_support_dry_run() {
    let root = temp_root("move-delete-dry-run");
    let src = root.join("src");
    let lib = root.join("lib");
    fs::create_dir_all(&src).unwrap();
    fs::create_dir_all(&lib).unwrap();
    let util = src.join("util.ts");
    let app = src.join("app.ts");
    let destination = lib.join("util.ts");
    fs::write(&util, "export const util = 1\n").unwrap();
    fs::write(&app, "import { util } from './util'\n").unwrap();
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
    let mut move_args = Map::new();
    move_args.insert(
        "source".to_string(),
        Value::String(util.display().to_string()),
    );
    move_args.insert(
        "destination".to_string(),
        Value::String(destination.display().to_string()),
    );
    move_args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );
    move_args.insert("update_imports".to_string(), Value::Bool(true));
    move_args.insert("dry_run".to_string(), Value::Bool(true));

    let moved = move_file(&move_args, &ctx);

    assert!(!moved.error, "{}", moved.text);
    assert!(moved.text.contains("dry_run: true"));
    assert!(moved.text.contains("src/app.ts:1: ./util -> ../lib/util"));
    assert!(util.exists());
    assert!(!destination.exists());
    assert_eq!(
        fs::read_to_string(&app).unwrap(),
        "import { util } from './util'\n"
    );

    let mut delete_args = Map::new();
    delete_args.insert(
        "path".to_string(),
        Value::String(util.display().to_string()),
    );
    delete_args.insert("dry_run".to_string(), Value::Bool(true));
    let deleted = delete_file(&delete_args, &ctx);

    assert!(!deleted.error, "{}", deleted.text);
    assert!(deleted.text.contains("Would delete"));
    assert!(util.exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn delete_file_rejects_directories() {
    let root = temp_root("delete-dir");
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
        Value::String(root.display().to_string()),
    );

    let outcome = delete_file(&args, &ctx);

    assert!(outcome.error);
    assert!(outcome.text.contains("Not a file"));
    assert!(root.exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn replace_symbol_body_previews_then_replaces_symbol() {
    let root = temp_root("replace-symbol-body");
    let target = root.join("service.py");
    let original = "def load_user():\n    return None\n\n\ndef untouched():\n    return None\n";
    let replacement = "def load_user():\n    user = {'id': 1}\n    return user\n";
    fs::write(&target, original).unwrap();
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
    let mut preview_args = Map::new();
    preview_args.insert(
        "path".to_string(),
        Value::String(target.display().to_string()),
    );
    preview_args.insert("symbol".to_string(), Value::String("load_user".to_string()));
    preview_args.insert(
        "new_text".to_string(),
        Value::String(replacement.to_string()),
    );

    let preview = replace_symbol_body(&preview_args, &ctx);

    assert!(!preview.error, "{}", preview.text);
    assert!(preview.text.contains("dry_run: true"));
    assert!(preview.text.contains("## Current"));
    assert!(preview.text.contains("## Replacement"));
    assert_eq!(fs::read_to_string(&target).unwrap(), original);

    let mut apply_args = preview_args;
    apply_args.insert("dry_run".to_string(), Value::Bool(false));
    let applied = replace_symbol_body(&apply_args, &ctx);

    assert!(!applied.error, "{}", applied.text);
    assert!(applied.text.contains("Applied."));
    assert!(!applied.text.contains("## Formatting"));
    assert_eq!(
            fs::read_to_string(&target).unwrap(),
            "def load_user():\n    user = {'id': 1}\n    return user\n\n\ndef untouched():\n    return None\n"
        );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn replace_in_files_previews_then_applies_exact_replacements() {
    let root = temp_root("replace-in-files");
    let first = root.join("first.txt");
    let second = root.join("second.txt");
    let ignored = root.join("ignored.md");
    fs::write(&first, "alpha beta alpha\n").unwrap();
    fs::write(&second, "beta alpha\n").unwrap();
    fs::write(&ignored, "alpha\n").unwrap();
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
    let mut preview_args = Map::new();
    preview_args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );
    preview_args.insert("glob".to_string(), Value::String("*.txt".to_string()));
    preview_args.insert("old_text".to_string(), Value::String("alpha".to_string()));
    preview_args.insert("new_text".to_string(), Value::String("Nola".to_string()));

    let preview = replace_in_files(&preview_args, &ctx);

    assert!(!preview.error, "{}", preview.text);
    assert!(preview.text.contains("dry_run: true"));
    assert!(preview.text.contains(&format!("- {}: 2", first.display())));
    assert!(preview.text.contains(&format!("- {}: 1", second.display())));
    assert!(preview.text.contains("sha256="));
    assert_eq!(fs::read_to_string(&first).unwrap(), "alpha beta alpha\n");

    let mut apply_args = preview_args;
    apply_args.insert("dry_run".to_string(), Value::Bool(false));
    let mut expected = Map::new();
    expected.insert(
        "first.txt".to_string(),
        Value::String(file_sha256_hex(&first).unwrap()),
    );
    expected.insert(
        "second.txt".to_string(),
        Value::String(file_sha256_hex(&second).unwrap()),
    );
    fs::write(&second, "beta user alpha\n").unwrap();
    apply_args.insert(
        "expected_sha256_by_path".to_string(),
        Value::Object(expected.clone()),
    );
    let stale = replace_in_files(&apply_args, &ctx);

    assert!(stale.error);
    assert!(stale.text.contains("expected_sha256_by_path mismatch"));
    assert_eq!(fs::read_to_string(&first).unwrap(), "alpha beta alpha\n");
    assert_eq!(fs::read_to_string(&second).unwrap(), "beta user alpha\n");

    fs::write(&second, "beta alpha\n").unwrap();
    expected.insert(
        "second.txt".to_string(),
        Value::String(file_sha256_hex(&second).unwrap()),
    );
    apply_args.insert(
        "expected_sha256_by_path".to_string(),
        Value::Object(expected),
    );
    let applied = replace_in_files(&apply_args, &ctx);

    assert!(!applied.error, "{}", applied.text);
    assert_eq!(fs::read_to_string(&first).unwrap(), "Nola beta Nola\n");
    assert_eq!(fs::read_to_string(&second).unwrap(), "beta Nola\n");
    assert_eq!(fs::read_to_string(&ignored).unwrap(), "alpha\n");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn rename_symbol_previews_then_applies_identifier_boundary_rename() {
    let root = temp_root("rename-symbol");
    let target = root.join("app.ts");
    let docs = root.join("README.md");
    fs::write(
        &target,
        "export function loadUser() { return 1 }\nconst preloadUser = loadUser()\n",
    )
    .unwrap();
    fs::write(&docs, "loadUser documentation\n").unwrap();
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
    let mut preview_args = Map::new();
    preview_args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );
    preview_args.insert(
        "old_symbol".to_string(),
        Value::String("loadUser".to_string()),
    );
    preview_args.insert(
        "new_symbol".to_string(),
        Value::String("fetchUser".to_string()),
    );

    let preview = rename_symbol(&preview_args, &ctx);

    assert!(!preview.error, "{}", preview.text);
    assert!(preview.text.contains("dry_run: true"));
    assert!(preview.text.contains(&format!("- {}: 2", target.display())));
    assert!(preview.text.contains("sha256="));
    assert!(!preview.text.contains("README.md"));
    assert!(fs::read_to_string(&target)
        .unwrap()
        .starts_with("export function loadUser"));

    let mut apply_args = preview_args;
    apply_args.insert("dry_run".to_string(), Value::Bool(false));
    apply_args.insert(
        "expected_sha256_by_path".to_string(),
        json!({"app.ts": file_sha256_hex(&target).unwrap()}),
    );
    let applied = rename_symbol(&apply_args, &ctx);

    assert!(!applied.error, "{}", applied.text);
    assert_eq!(
        fs::read_to_string(&target).unwrap(),
        "export function fetchUser() { return 1 }\nconst preloadUser = fetchUser()\n"
    );
    assert_eq!(
        fs::read_to_string(&docs).unwrap(),
        "loadUser documentation\n"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn edit_file_replaces_exact_text_once() {
    let root = temp_root("edit");
    let file = root.join("note.txt");
    fs::write(&file, "hello world").unwrap();
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
    args.insert("old_text".to_string(), Value::String("world".to_string()));
    args.insert("new_text".to_string(), Value::String("Nola".to_string()));

    let outcome = edit_file(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert_eq!(fs::read_to_string(&file).unwrap(), "hello Nola");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn edit_tools_reject_no_effect_replacements_without_touching_the_file() {
    let root = temp_root("edit-no-effect");
    let file = root.join("note.txt");
    fs::write(&file, "return 0;\n").unwrap();
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

    let mut exact_args = Map::new();
    exact_args.insert(
        "path".to_string(),
        Value::String(file.display().to_string()),
    );
    exact_args.insert(
        "old_text".to_string(),
        Value::String("return 0;".to_string()),
    );
    exact_args.insert(
        "new_text".to_string(),
        Value::String("return 0;".to_string()),
    );
    let exact = edit_file(&exact_args, &ctx);
    assert!(exact.error);
    assert!(exact.text.contains("edit_no_effect: true"));

    let mut line_args = Map::new();
    line_args.insert(
        "path".to_string(),
        Value::String(file.display().to_string()),
    );
    line_args.insert("start_line".to_string(), Value::from(1));
    line_args.insert("line_count".to_string(), Value::from(1));
    line_args.insert(
        "new_text".to_string(),
        Value::String("return 0;\n".to_string()),
    );
    let line = edit_file_lines(&line_args, &ctx);
    assert!(line.error);
    assert!(line.text.contains("edit_no_effect: true"));
    assert_eq!(fs::read_to_string(&file).unwrap(), "return 0;\n");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn edit_file_lines_replaces_and_inserts_ranges() {
    let root = temp_root("edit-lines");
    let file = root.join("note.txt");
    fs::write(&file, "one\ntwo\nthree\n").unwrap();
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
    let mut replace_args = Map::new();
    replace_args.insert(
        "path".to_string(),
        Value::String(file.display().to_string()),
    );
    replace_args.insert("start_line".to_string(), Value::from(2));
    replace_args.insert("line_count".to_string(), Value::from(1));
    replace_args.insert("new_text".to_string(), Value::String("TWO".to_string()));
    replace_args.insert("dry_run".to_string(), Value::Bool(true));

    let preview = edit_file_lines(&replace_args, &ctx);

    assert!(!preview.error, "{}", preview.text);
    assert!(preview.text.contains("dry_run: true"));
    assert!(preview.text.contains("-two"));
    assert!(preview.text.contains("+TWO"));
    assert_eq!(fs::read_to_string(&file).unwrap(), "one\ntwo\nthree\n");

    replace_args.insert("dry_run".to_string(), Value::Bool(false));
    let replaced = edit_file_lines(&replace_args, &ctx);
    assert!(!replaced.error, "{}", replaced.text);
    assert_eq!(fs::read_to_string(&file).unwrap(), "one\nTWO\nthree\n");

    let mut insert_args = Map::new();
    insert_args.insert(
        "path".to_string(),
        Value::String(file.display().to_string()),
    );
    insert_args.insert("start_line".to_string(), Value::from(3));
    insert_args.insert("line_count".to_string(), Value::from(0));
    insert_args.insert(
        "new_text".to_string(),
        Value::String("inserted\n".to_string()),
    );

    let inserted = edit_file_lines(&insert_args, &ctx);

    assert!(!inserted.error, "{}", inserted.text);
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        "one\nTWO\ninserted\nthree\n"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn edit_file_lines_preserves_crlf() {
    let root = temp_root("edit-lines-crlf");
    let file = root.join("note.txt");
    fs::write(&file, b"one\r\ntwo\r\n").unwrap();
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
    args.insert("start_line".to_string(), Value::from(2));
    args.insert("line_count".to_string(), Value::from(1));
    args.insert("new_text".to_string(), Value::String("TWO\n".to_string()));

    let outcome = edit_file_lines(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert_eq!(fs::read(&file).unwrap(), b"one\r\nTWO\r\n");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn read_file_supports_line_ranges_and_numbers() {
    let root = temp_root("read-lines");
    let file = root.join("note.txt");
    fs::write(&file, "one\ntwo\nthree\nfour\n").unwrap();
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
    args.insert("start_line".to_string(), Value::from(2));
    args.insert("line_count".to_string(), Value::from(2));

    let outcome = read_file(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(!outcome.text.contains("sha256:"));
    assert!(outcome.text.contains("lines: 2-3 of 4"));
    assert!(outcome.text.contains("   2 | two"));
    assert!(outcome.text.contains("   3 | three"));
    assert!(!outcome.text.contains("one"));
    assert!(!outcome.text.contains("four"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn read_file_large_path_only_returns_preview() {
    let root = temp_root("read-large-preview");
    let file = root.join("large.py");
    let mut content = String::new();
    for index in 1..=10_000 {
        content.push_str(&format!("line_{index}\n"));
    }
    fs::write(&file, content).unwrap();
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

    let preview = read_file(&args, &ctx);

    assert!(!preview.error, "{}", preview.text);
    assert!(preview.text.contains("large file preview"));
    assert!(preview.text.contains("of 10000"), "{}", preview.text);
    assert!(preview.text.contains("line_1"));
    assert!(!preview.text.contains("line_10000"));

    args.insert("start_line".to_string(), Value::from(9_999));
    args.insert("line_count".to_string(), Value::from(2));
    let focused = read_file(&args, &ctx);

    assert!(!focused.error, "{}", focused.text);
    assert!(focused.text.contains("line_9999"));
    assert!(focused.text.contains("line_10000"));
    assert!(!focused.text.contains("large file preview"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn read_file_reports_an_unsupported_binary_without_lossy_text() {
    let root = temp_root("read-binary");
    let file = root.join("workbook.xls");
    fs::write(&file, [0xd0, 0xcf, 0x11, 0xe0, 0, 1, 2, 3]).unwrap();
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

    let outcome = read_file(&args, &ctx);

    assert!(outcome.error);
    assert!(outcome.text.contains("unsupported binary format"));
    assert!(outcome.text.contains("recognized formats"));
    assert!(!outcome.text.contains('\u{fffd}'));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn read_file_snapshots_images_and_explicit_visual_pdf_without_mutation_credit() {
    let root = tempfile::tempdir().unwrap();
    let outputs = tempfile::tempdir().unwrap();
    let ctx = ToolContext {
        memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
        language_services: Default::default(),
        skills: Default::default(), allowed_dirs: vec![root.path().to_path_buf()],
        policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), true, "allow".into(), Vec::new()),
        permission_scope: Default::default(), cancellation: Default::default(), plan_state: Default::default(),
        output_store: crate::tools::ToolOutputStore::new(outputs.path().to_path_buf()),
        browser: BrowserRuntime::new(root.path().join("browser")),
    };
    for (name, bytes, mime) in [("image.png", b"\x89PNG\r\n\x1a\nfixture".as_slice(), "image/png"),
        ("doc.pdf", b"%PDF-1.4 fixture".as_slice(), "application/pdf")] {
        let source = root.path().join(name);
        fs::write(&source, bytes).unwrap();
        let args = json!({"path":source,"visual":true});
        let outcome = read_file(args.as_object().unwrap(), &ctx);
        assert!(!outcome.error, "{}", outcome.text);
        assert!(outcome.execution.is_none());
        assert_eq!(outcome.artifacts.len(), 1);
        let artifact = &outcome.artifacts[0];
        assert_eq!(artifact.mime_type.as_deref(), Some(mime));
        let snapshot = url::Url::parse(&artifact.uri).unwrap().to_file_path().unwrap();
        fs::write(&source, b"changed source").unwrap();
        assert_eq!(fs::read(snapshot).unwrap(), bytes);
    }
    let outside = outputs.path().join("outside.png");
    fs::write(&outside, b"\x89PNG\r\n\x1a\nsecret").unwrap();
    assert!(read_file(json!({"path":outside}).as_object().unwrap(), &ctx).error);
}

#[test]
fn read_file_pdf_page_is_bounded_and_preserves_authorization() {
    let root = tempfile::tempdir().unwrap();
    let outputs = tempfile::tempdir().unwrap();
    let ctx = ToolContext {
        memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
        language_services: Default::default(),
        skills: Default::default(), allowed_dirs: vec![root.path().to_path_buf()],
        policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), true, "allow".into(), Vec::new()),
        permission_scope: Default::default(), cancellation: Default::default(), plan_state: Default::default(),
        output_store: crate::tools::ToolOutputStore::new(outputs.path().to_path_buf()),
        browser: BrowserRuntime::new(root.path().join("browser")),
    };
    use lopdf::{dictionary, Object};
    let mut document = lopdf::Document::with_version("1.5");
    let pages = document.new_object_id();
    let page = document.add_object(dictionary! {"Type"=>"Page", "Parent"=>pages,
        "MediaBox"=>vec![0.into(),0.into(),200.into(),100.into()]});
    document.objects.insert(pages, Object::Dictionary(dictionary! {
        "Type"=>"Pages", "Kids"=>vec![Object::Reference(page)], "Count"=>1}));
    let catalog = document.add_object(dictionary! {"Type"=>"Catalog", "Pages"=>pages});
    document.trailer.set("Root", catalog);
    let source = root.path().join("input with spaces.pdf");
    document.save(&source).unwrap();
    for page in [0, 501] {
        assert!(read_file(json!({"path":source,"pdf_page":page}).as_object().unwrap(), &ctx).error);
    }
    let outside = outputs.path().join("outside.pdf");
    fs::copy(&source, &outside).unwrap();
    assert!(read_file(json!({"path":outside,"pdf_page":1}).as_object().unwrap(), &ctx).error);
    let outcome = read_file(json!({"path":source,"pdf_page":1}).as_object().unwrap(), &ctx);
    if Command::new("pdftoppm").arg("-v").output().is_err() {
        assert!(outcome.error && outcome.text.contains("missing_dependency"));
        return;
    }
    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.execution.is_none());
    assert_eq!(outcome.artifacts[0].mime_type.as_deref(), Some("image/png"));
    let snapshot = url::Url::parse(&outcome.artifacts[0].uri).unwrap().to_file_path().unwrap();
    assert!(fs::read(snapshot).unwrap().starts_with(b"\x89PNG\r\n\x1a\n"));
    assert!(read_file(json!({"path":source,"pdf_page":2}).as_object().unwrap(), &ctx).error);
}

#[test]
fn read_many_files_batches_ranges_and_rejects_outside_paths() {
    let root = temp_root("read-many");
    let first = root.join("first.rs");
    let second = root.join("second.rs");
    let outside = root.parent().unwrap().join(format!(
        "{}_outside.rs",
        root.file_name().unwrap().to_string_lossy()
    ));
    fs::write(&first, "one\ntwo\nthree\n").unwrap();
    fs::write(&second, "alpha\nbeta\ngamma\n").unwrap();
    fs::write(&outside, "secret\n").unwrap();
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
        "files".to_string(),
        json!([
            {"path": first.display().to_string(), "start_line": 2, "line_count": 1},
            {"path": second.display().to_string(), "line_count": 2}
        ]),
    );

    let outcome = read_many_files(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains(&first.display().to_string()));
    assert!(outcome.text.contains(&second.display().to_string()));
    assert!(outcome.text.contains("   2 | two"));
    assert!(outcome.text.contains("   1 | alpha"));
    assert!(outcome.text.contains("   2 | beta"));
    assert!(!outcome.text.contains("three"));

    args.insert(
        "files".to_string(),
        json!([{"path": outside.display().to_string()}]),
    );
    let denied = read_many_files(&args, &ctx);

    assert!(!denied.error, "{}", denied.text);
    let _ = fs::remove_file(outside);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn file_outline_extracts_common_source_symbols() {
    let root = temp_root("file-outline");
    let file = root.join("sample.ts");
    fs::write(
        &file,
        [
            "import React from 'react'",
            "import { helper } from './helper'",
            "export interface User { id: string }",
            "export class UserStore {",
            "  load() { return [] }",
            "}",
            "export const makeUser = (id: string) => ({ id })",
            "type UserId = string",
            "pub fn save_user() {}",
            "public func refreshUser() {}",
            "private fun syncUser() {}",
        ]
        .join("\n"),
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
        "path".to_string(),
        Value::String(file.display().to_string()),
    );
    args.insert("include_imports".to_string(), Value::Bool(true));

    let outcome = file_outline(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("## Symbols"));
    assert!(outcome.text.contains("## Imports"));
    assert!(outcome.text.contains("1: js_import react"));
    assert!(outcome.text.contains("2: js_import ./helper"));
    assert!(outcome.text.contains("3: interface User"));
    assert!(outcome.text.contains("4: class UserStore"));
    assert!(outcome.text.contains("5: method load"));
    assert!(outcome.text.contains("7: function makeUser"));
    assert!(outcome.text.contains("8: type UserId"));
    assert!(outcome.text.contains("9: function save_user"));
    assert!(outcome.text.contains("10: function refreshUser"));
    assert!(outcome.text.contains("11: function syncUser"));
    assert!(outcome.display.contains("8 symbols"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn read_symbol_reads_function_body_without_next_symbol() {
    let root = temp_root("read-symbol");
    let file = root.join("sample.py");
    fs::write(
            &file,
            "class Service:\n    def load_user(self, user_id):\n        row = query_db(user_id)\n        return normalize(row)\n\n    def other(self):\n        return None\n\ndef render():\n    return Service().load_user('1')\n",
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
        "path".to_string(),
        Value::String(file.display().to_string()),
    );
    args.insert("symbol".to_string(), Value::String("load_user".to_string()));

    let outcome = read_symbol(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome
        .text
        .contains(&format!("## {}:2: function load_user", file.display())));
    assert!(outcome
        .text
        .contains("   2 |     def load_user(self, user_id):"));
    assert!(outcome
        .text
        .contains("   3 |         row = query_db(user_id)"));
    assert!(outcome
        .text
        .contains("   4 |         return normalize(row)"));
    assert!(!outcome.text.contains("def other"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn repo_map_extracts_multi_file_symbols() {
    let root = temp_root("repo-map");
    let src = root.join("src");
    fs::create_dir_all(&src).unwrap();
    let first = src.join("users.py");
    let second = src.join("orders.py");
    let ignored = src.join("README.md");
    fs::write(
        &first,
        "class UserStore:\n    pass\n\ndef load_user():\n    pass\n",
    )
    .unwrap();
    fs::write(&second, "def load_order():\n    pass\n").unwrap();
    fs::write(&ignored, "# notes\n").unwrap();
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
    args.insert("glob".to_string(), Value::String("src/*.py".to_string()));
    args.insert("max_files".to_string(), Value::from(10));

    let outcome = repo_map(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("## src/users.py"));
    assert!(outcome.text.contains("- 1: class UserStore"));
    assert!(outcome.text.contains("- 4: function load_user"));
    assert!(outcome.text.contains("## src/orders.py"));
    assert!(outcome.text.contains("- 1: function load_order"));
    assert!(!outcome.text.contains("README.md"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn workspace_symbols_finds_partial_symbol_names() {
    let root = temp_root("workspace-symbols");
    let src = root.join("src");
    fs::create_dir_all(&src).unwrap();
    let file = src.join("service.py");
    let ignored = src.join("notes.txt");
    fs::write(
            &file,
            "class UserStore:\n    pass\n\ndef load_user():\n    pass\n\ndef save_user():\n    pass\n\ndef render_order():\n    pass\n",
        )
        .unwrap();
    fs::write(&ignored, "def fake_user():\n    pass\n").unwrap();
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
    args.insert("query".to_string(), Value::String("user".to_string()));
    args.insert("glob".to_string(), Value::String("src/*.py".to_string()));
    args.insert(
        "kinds".to_string(),
        Value::Array(vec![
            Value::String("class".to_string()),
            Value::String("function".to_string()),
        ]),
    );
    args.insert("max_results".to_string(), Value::from(10));

    let outcome = workspace_symbols(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("- src/service.py:1: class UserStore"));
    assert!(outcome
        .text
        .contains("- src/service.py:4: function load_user"));
    assert!(outcome
        .text
        .contains("- src/service.py:7: function save_user"));
    assert!(!outcome.text.contains("render_order"));
    assert!(!outcome.text.contains("notes.txt"));
    assert!(outcome.display.contains("3 matches"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn code_search_finds_structural_definition_and_call_matches() {
    let root = temp_root("code-search");
    let src = root.join("src");
    fs::create_dir_all(&src).unwrap();
    let app = src.join("app.ts");
    let service = src.join("service.py");
    fs::write(
            &app,
            "function loadUser() {\n  return 1\n}\n\nexport function render() {\n  return loadUser()\n}\n\nexport function hydrate() {\n  return loadUser()\n}\n",
        )
        .unwrap();
    fs::write(&service, "def load_user():\n    return 1\n").unwrap();
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
    let mut definition_args = Map::new();
    definition_args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );
    definition_args.insert("query".to_string(), Value::String("load".to_string()));
    definition_args.insert("mode".to_string(), Value::String("definition".to_string()));
    definition_args.insert("language".to_string(), Value::String("ts".to_string()));

    let definitions = code_search(&definition_args, &ctx);

    assert!(!definitions.error, "{}", definitions.text);
    assert!(definitions
        .text
        .contains("- src/app.ts:1: definition function loadUser"));
    assert!(!definitions.text.contains("src/service.py"));

    let mut call_args = Map::new();
    call_args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );
    call_args.insert("query".to_string(), Value::String("loadUser".to_string()));
    call_args.insert("mode".to_string(), Value::String("call".to_string()));
    call_args.insert("exact".to_string(), Value::Bool(true));
    call_args.insert("context_lines".to_string(), Value::from(1));
    call_args.insert(
        "enclosing_symbol".to_string(),
        Value::String("render".to_string()),
    );

    let calls = code_search(&call_args, &ctx);

    assert!(!calls.error, "{}", calls.text);
    assert!(calls.text.contains("- src/app.ts:6: call loadUser"));
    assert!(calls.text.contains("in: function render"));
    assert!(calls.text.contains("5 | export function render() {"));
    assert!(!calls.text.contains("hydrate"));
    assert!(calls.display.contains("1 matches"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn symbol_search_finds_definitions_and_references() {
    let root = temp_root("symbol-search");
    let file = root.join("sample.rs");
    let other = root.join("other.txt");
    fs::write(
            &file,
            "fn load_user(user_id: &str) -> &str {\n    user_id\n}\n\nfn render() {\n    load_user(\"1\");\n}\n",
        )
        .unwrap();
    fs::write(&other, "load_user should be ignored by glob\n").unwrap();
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
    args.insert("symbol".to_string(), Value::String("load_user".to_string()));
    args.insert("glob".to_string(), Value::String("*.rs".to_string()));

    let outcome = symbol_search(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains(&format!(
        "{}:1:def: fn load_user(user_id: &str) -> &str {{",
        file.display()
    )));
    assert!(outcome
        .text
        .contains(&format!("{}:6:ref: load_user(\"1\");", file.display())));
    assert!(!outcome.text.contains("other.txt"));
    assert!(outcome.display.contains("1 definitions, 1 references"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn implementation_search_finds_type_and_method_implementations() {
    let root = temp_root("implementation-search");
    let src = root.join("src");
    fs::create_dir_all(&src).unwrap();
    let py_file = src.join("stores.py");
    let ts_file = src.join("stores.ts");
    fs::write(
            &py_file,
            "class BaseStore:\n    pass\n\nclass UserStore(BaseStore):\n    def load(self):\n        return None\n",
        )
        .unwrap();
    fs::write(
        &ts_file,
        "class ApiStore implements BaseStore {\n  load() { return null }\n}\n",
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
    let mut type_args = Map::new();
    type_args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );
    type_args.insert("symbol".to_string(), Value::String("BaseStore".to_string()));
    type_args.insert("glob".to_string(), Value::String("src/*".to_string()));
    type_args.insert("max_matches".to_string(), Value::from(20));

    let type_outcome = implementation_search(&type_args, &ctx);

    assert!(!type_outcome.error, "{}", type_outcome.text);
    assert!(type_outcome
        .text
        .contains(&format!("{}:4: class UserStore", py_file.display())));
    assert!(type_outcome
        .text
        .contains(&format!("{}:1: class ApiStore", ts_file.display())));
    assert!(!type_outcome
        .text
        .contains(&format!("{}:1: class BaseStore", py_file.display())));
    assert!(type_outcome.display.contains("2 type implementations"));

    let mut method_args = Map::new();
    method_args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );
    method_args.insert("symbol".to_string(), Value::String("load".to_string()));
    method_args.insert("glob".to_string(), Value::String("src/*".to_string()));
    method_args.insert("max_matches".to_string(), Value::from(20));

    let method_outcome = implementation_search(&method_args, &ctx);

    assert!(!method_outcome.error, "{}", method_outcome.text);
    assert!(method_outcome.text.contains("## Same-name definitions"));
    assert!(method_outcome
        .text
        .contains(&format!("{}:5: function load", py_file.display())));
    assert!(method_outcome
        .text
        .contains(&format!("{}:2: method load", ts_file.display())));
    assert!(method_outcome.display.contains("2 same-name definitions"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn code_navigation_finds_symbol_at_position() {
    let root = temp_root("code-navigation");
    let file = root.join("sample.py");
    fs::write(
            &file,
            "# Fetch a normalized user by id.\ndef load_user(user_id):\n    return user_id\n\ndef render():\n    return load_user('1')\n",
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
        "path".to_string(),
        Value::String(file.display().to_string()),
    );
    args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );
    args.insert("line".to_string(), Value::from(6));
    args.insert("character".to_string(), Value::from(13));
    args.insert("glob".to_string(), Value::String("*.py".to_string()));

    let outcome = code_navigation(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("# Code navigation: load_user"));
    assert!(outcome.text.contains(&format!(
        "{}:2:def: def load_user(user_id):",
        file.display()
    )));
    assert!(outcome.text.contains("## Symbol docs"));
    assert!(outcome
        .text
        .contains("doc: # Fetch a normalized user by id."));
    assert!(outcome
        .text
        .contains(&format!("{}:6:ref: return load_user('1')", file.display())));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn code_navigation_finds_implementation_candidates() {
    let root = temp_root("code-navigation-implementations");
    let file = root.join("stores.py");
    fs::write(
            &file,
            "class BaseStore:\n    pass\n\nclass UserStore(BaseStore):\n    def load(self):\n        return None\n",
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
        "path".to_string(),
        Value::String(file.display().to_string()),
    );
    args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );
    args.insert("line".to_string(), Value::from(1));
    args.insert("character".to_string(), Value::from(8));
    args.insert(
        "operation".to_string(),
        Value::String("implementations".to_string()),
    );
    args.insert("glob".to_string(), Value::String("*.py".to_string()));

    let outcome = code_navigation(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("# Code navigation: BaseStore"));
    assert!(outcome.text.contains("## Type implementations"));
    assert!(outcome
        .text
        .contains(&format!("{}:4: class UserStore", file.display())));
    assert!(!outcome
        .text
        .contains(&format!("{}:1: class BaseStore", file.display())));
    assert!(outcome.display.contains("1 type implementations"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn call_graph_reports_incoming_and_outgoing_calls() {
    let root = temp_root("call-graph");
    let file = root.join("sample.py");
    fs::write(
            &file,
            "def fetch_user(user_id):\n    row = query_db(user_id)\n    return normalize(row)\n\ndef render():\n    return fetch_user('1')\n",
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
        "symbol".to_string(),
        Value::String("fetch_user".to_string()),
    );
    args.insert("glob".to_string(), Value::String("*.py".to_string()));

    let outcome = call_graph(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains(&format!(
        "{}:1:def: def fetch_user(user_id):",
        file.display()
    )));
    assert!(outcome.text.contains(&format!(
        "{}:6:in: function render at line 5 -> fetch_user",
        file.display()
    )));
    assert!(outcome
        .text
        .contains(&format!("{}:2:out: fetch_user -> query_db", file.display())));
    assert!(outcome.text.contains(&format!(
        "{}:3:out: fetch_user -> normalize",
        file.display()
    )));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn code_diagnostics_reports_conflicts_json_and_debug_leftovers() {
    let root = temp_root("code-diagnostics");
    let conflict = root.join("conflict.py");
    let syntax = root.join("syntax.py");
    let config = root.join("config.json");
    let toml_config = root.join("broken.toml");
    let yaml_config = root.join("broken.yaml");
    let script = root.join("app.ts");
    let complex_ts = root.join("complex.ts");
    let secret_file = root.join("secret.py");
    let password_file = root.join("password.ts");
    fs::write(
        &conflict,
        "<<<<<<< HEAD\nprint('debug')\n=======\nprint('other')\n>>>>>>> branch\n",
    )
    .unwrap();
    fs::write(&syntax, "def broken(:\n    pass\n").unwrap();
    fs::write(&config, "{\"broken\": true,,}\n").unwrap();
    fs::write(&toml_config, "name = [\n").unwrap();
    fs::write(&yaml_config, "items: [one, two\n").unwrap();
    fs::write(&script, "console.log('debug')\n").unwrap();
    fs::write(
        &complex_ts,
        "const template = `value ${items.map((item) => ({ id: item.id }))}`\nconst authorization = 'notDetermined'\n",
    )
    .unwrap();
    fs::write(
        &secret_file,
        "api_key = 'sk-abcdefghijklmnopqrstuvwxyz123456'\n",
    )
    .unwrap();
    fs::write(&password_file, "const password = 'supersecretvalue'\n").unwrap();
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
    args.insert("glob".to_string(), Value::String("*".to_string()));

    let outcome = code_diagnostics(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains(&format!(
        "{}:1:error: unresolved merge conflict marker",
        conflict.display()
    )));
    if Command::new("python3").arg("--version").output().is_ok()
        || Command::new("python").arg("--version").output().is_ok()
    {
        assert!(outcome.text.contains(&format!(
            "{}:1:error: Python syntax error",
            syntax.display()
        )));
    }
    assert!(outcome
        .text
        .contains(&format!("{}:1:error: invalid JSON", config.display())));
    if toml_syntax_diagnostic(&toml_config).is_some() {
        assert!(outcome
            .text
            .contains(&format!("{}:1:error: invalid TOML", toml_config.display())));
    }
    assert!(outcome.text.contains(&format!(
        "{}:1:error: invalid YAML: unclosed [",
        yaml_config.display()
    )));
    assert!(outcome.text.contains(&format!(
        "{}:1:warning: console.log left in source",
        script.display()
    )));
    assert!(outcome.text.contains(&format!(
        "{}:1:warning: possible hard-coded secret (OpenAI-style API key)",
        secret_file.display()
    )));
    assert!(!outcome.text.contains("sk-abcdefghijklmnopqrstuvwxyz123456"));
    assert!(!outcome.text.contains("api_key = [REDACTED]"));
    assert!(outcome.text.contains(&format!(
        "{}:1:warning: possible hard-coded secret (password assignment)",
        password_file.display()
    )));
    assert!(!outcome.text.contains("supersecretvalue"));
    assert!(outcome.text.contains("password = [REDACTED]"));
    assert!(!outcome.text.contains("structural syntax issue"));
    assert!(!outcome.text.contains("authorization assignment"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn code_diagnostics_prefers_git_changed_files_when_paths_missing() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }

    let root = temp_root("code-diagnostics-git");
    let clean_but_noisy = root.join("clean_but_noisy.py");
    let changed = root.join("changed.py");
    fs::write(&clean_but_noisy, "print('committed debug')\n").unwrap();
    fs::write(&changed, "value = 1\n").unwrap();
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
    fs::write(&changed, "print('new debug')\n").unwrap();
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

    let outcome = code_diagnostics(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("scan_scope: git_changed_files"));
    assert!(outcome.text.contains(&format!(
        "{}:1:warning: print call left in source",
        changed.display()
    )));
    assert!(!outcome
        .text
        .contains(&clean_but_noisy.display().to_string()));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn code_diagnostics_reports_missing_relative_imports() {
    let root = temp_root("code-diagnostics-imports");
    let src = root.join("src");
    let pkg = root.join("pkg");
    fs::create_dir_all(&src).unwrap();
    fs::create_dir_all(&pkg).unwrap();
    fs::write(src.join("present.ts"), "export const value = 1\n").unwrap();
    let ts_file = src.join("app.ts");
    fs::write(
        &ts_file,
        "import { value } from './present'\nimport missing from './missing'\n",
    )
    .unwrap();
    fs::write(pkg.join("__init__.py"), "").unwrap();
    fs::write(pkg.join("present.py"), "value = 1\n").unwrap();
    let py_file = pkg.join("module.py");
    fs::write(
        &py_file,
        "from .present import value\nfrom .missing import value\n",
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
        json!([ts_file.display().to_string(), py_file.display().to_string()]),
    );

    let outcome = code_diagnostics(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains(&format!(
        "{}:2:error: unresolved relative import \"./missing\"",
        ts_file.display()
    )));
    assert!(outcome.text.contains(&format!(
        "{}:2:error: unresolved Python relative import \".missing\"",
        py_file.display()
    )));
    let unresolved_lines = outcome
        .text
        .lines()
        .filter(|line| line.contains("unresolved"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!unresolved_lines.contains("present"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn failure_context_maps_output_to_source_snippets() {
    let root = temp_root("failure-context");
    let src = root.join("src");
    fs::create_dir_all(&src).unwrap();
    let target = src.join("service.py");
    fs::write(
        &target,
        "def load_user():\n    value = 1\n    return value\n",
    )
    .unwrap();
    let ts_target = src.join("view.ts");
    fs::write(
        &ts_target,
        "export function render() {\n  return missing\n}\n",
    )
    .unwrap();
    let lint_target = src.join("lint.ts");
    fs::write(
        &lint_target,
        "export const linted = () => {\n  return window.any\n}\n",
    )
    .unwrap();
    let output = concat!(
        "E   AssertionError\n",
        "src/service.py:2: AssertionError\n",
        "src/view.ts(2,10): error TS2304: Cannot find name 'missing'.\n",
        "\n",
        "src/lint.ts\n",
        "  2:10  error  Unexpected any  @typescript-eslint/no-explicit-any\n"
    );
    let store = ToolOutputStore::new(root.join("tool-outputs"));
    fs::create_dir_all(store.root()).unwrap();
    let output_path = store.root().join("failure-context-test.txt");
    fs::write(&output_path, output).unwrap();
    let ctx = ToolContext {
        memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
        language_services: Default::default(),
        skills: Default::default(),
        allowed_dirs: vec![root.clone()],
        policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), true, "allow".to_string(), Vec::new()),
        permission_scope: Default::default(),
        cancellation: Default::default(),
        plan_state: Default::default(),
        output_store: store,
        browser: BrowserRuntime::new(temp_root("browser-runtime")),
    };
    let mut args = Map::new();
    args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );
    args.insert(
        "output_path".to_string(),
        Value::String(output_path.display().to_string()),
    );
    args.insert("context_lines".to_string(), Value::from(1));

    let outcome = failure_context(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("source: output_path:"));
    assert!(outcome.text.contains("src/service.py:2"));
    assert!(outcome.text.contains("src/view.ts:2"));
    assert!(outcome.text.contains("src/lint.ts:2"));
    assert!(outcome
        .text
        .contains("nearest_symbol: function load_user at line 1"));
    assert!(outcome.text.contains("2 |     value = 1"));
    assert!(outcome.text.contains("2 |   return missing"));
    assert!(outcome.text.contains("2 |   return window.any"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn failure_triage_prioritizes_failure_output() {
    let root = temp_root("failure-triage");
    fs::write(
        root.join("pyproject.toml"),
        "[tool.pytest.ini_options]\n\n[tool.ruff]\n",
    )
    .unwrap();
    let src = root.join("src");
    fs::create_dir_all(&src).unwrap();
    let target = src.join("service.py");
    fs::write(
        &target,
        "def load_user():\n    value = 1\n    return value\n",
    )
    .unwrap();
    let ts_target = src.join("view.ts");
    fs::write(
        &ts_target,
        "export function render() {\n  return missing\n}\n",
    )
    .unwrap();
    let output = concat!(
        "FAILED tests/test_service.py::test_load_user - AssertionError\n",
        "src/service.py:2: AssertionError: expected 2 got 1\n",
        "src/view.ts(2,10): error TS2304: Cannot find name 'missing'.\n",
        "ModuleNotFoundError: No module named 'requests'\n"
    );
    let store = ToolOutputStore::new(root.join("tool-outputs"));
    fs::create_dir_all(store.root()).unwrap();
    let output_path = store.root().join("failure-triage-test.txt");
    fs::write(&output_path, output).unwrap();
    let ctx = ToolContext {
        memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
        language_services: Default::default(),
        skills: Default::default(),
        allowed_dirs: vec![root.clone()],
        policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), true, "allow".to_string(), Vec::new()),
        permission_scope: Default::default(),
        cancellation: Default::default(),
        plan_state: Default::default(),
        output_store: store,
        browser: BrowserRuntime::new(temp_root("browser-runtime")),
    };
    let mut args = Map::new();
    args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );
    args.insert(
        "output_path".to_string(),
        Value::String(output_path.display().to_string()),
    );

    let outcome = failure_triage(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("# Failure triage:"));
    assert!(outcome.text.contains("primary_category: test"));
    assert!(outcome.text.contains("environment/dependency"));
    assert!(outcome.text.contains("typecheck"));
    assert!(outcome.text.contains("## Fix queue"));
    assert!(outcome.text.contains("src/service.py:2"));
    assert!(outcome.text.contains("src/view.ts:2"));
    assert!(outcome.text.contains("function load_user at line 1"));
    assert!(outcome
        .text
        .contains("failure_context root=<root> output_path=<saved-output>"));
    assert!(outcome.text.contains("-m pytest"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn file_dependencies_resolves_local_imports() {
    let root = temp_root("file-dependencies");
    let src = root.join("src");
    let components = src.join("components");
    let utils = src.join("utils");
    fs::create_dir_all(&components).unwrap();
    fs::create_dir_all(&utils).unwrap();
    let entry = src.join("entry.ts");
    let app = src.join("app.ts");
    let button = components.join("button.ts");
    let math = utils.join("math.ts");
    fs::write(
            &entry,
            "import { Button } from './components/button'\nconst math = require('./utils/math')\nimport React from 'react'\n",
        )
        .unwrap();
    fs::write(&app, "import './entry'\n").unwrap();
    fs::write(&button, "export const Button = () => null\n").unwrap();
    fs::write(
        &math,
        "export const add = (a: number, b: number) => a + b\n",
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
        "path".to_string(),
        Value::String(entry.display().to_string()),
    );
    args.insert("root".to_string(), Value::String(src.display().to_string()));

    let outcome = file_dependencies(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains(&format!(
        "js_import ./components/button -> {}",
        button.display()
    )));
    assert!(outcome
        .text
        .contains(&format!("js_require ./utils/math -> {}", math.display())));
    assert!(outcome.text.contains("js_import react"));
    assert!(outcome.text.contains("## Local importers"));
    assert!(outcome.text.contains("app.ts:1: js_import ./entry"));
    assert!(outcome
        .display
        .contains("3 imports, 2 local files, 1 importers"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn project_overview_reports_manifests_languages_and_commands() {
    let root = temp_root("project-overview");
    fs::write(
        root.join("package.json"),
        r#"{"scripts":{"test":"vitest","build":"vite build"}}"#,
    )
    .unwrap();
    fs::write(root.join("pyproject.toml"), "[tool.pytest.ini_options]\n").unwrap();
    fs::write(root.join("README.md"), "# Demo\n").unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src").join("app.ts"), "export const app = 1\n").unwrap();
    fs::write(root.join("src").join("main.py"), "print('hi')\n").unwrap();
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

    let outcome = project_overview(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("## Top-level entries"));
    assert!(outcome.text.contains("- src/"));
    assert!(outcome.text.contains("- package.json"));
    assert!(outcome.text.contains("- pyproject.toml"));
    assert!(outcome.text.contains("- README.md"));
    assert!(outcome.text.contains("- TypeScript: 1"));
    assert!(outcome.text.contains("- Python: 1"));
    assert!(outcome.text.contains("- cd . && npm run test"));
    assert!(outcome.text.contains("- cd . && npm run build"));
    assert!(outcome.text.contains("-m pytest"));
    assert!(outcome.text.contains("truncated: false"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn project_overview_marks_large_results_as_a_sample_with_a_continuation_hint() {
    let root = temp_root("project-overview-truncated");
    for index in 0..101 {
        fs::write(
            root.join(format!("source-{index:03}.rs")),
            "fn value() {}\n",
        )
        .unwrap();
    }
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
    let args = object_args(json!({
        "root": root.display().to_string(),
        "max_files": 100
    }));

    let outcome = project_overview(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("scanned_files: 100"));
    assert!(outcome.text.contains("truncated: true"));
    assert!(outcome
        .text
        .contains("continue with narrower paths or patterns"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn instruction_context_loads_applicable_instruction_hierarchy() {
    let root = temp_root("instruction-context");
    fs::write(root.join("AGENTS.md"), "root rules\n").unwrap();
    let rules_dir = root.join(".nola").join("rules");
    fs::create_dir_all(&rules_dir).unwrap();
    fs::write(rules_dir.join("always.md"), "always-on rule\n").unwrap();
    fs::write(
        rules_dir.join("typescript.md"),
        "---\npaths:\n  - packages/app/src/*.ts\n---\ntypescript scoped rule\n",
    )
    .unwrap();
    fs::write(
        rules_dir.join("python.md"),
        "---\npaths: scripts/*.py\n---\npython scoped rule\n",
    )
    .unwrap();
    let package = root.join("packages").join("app");
    let src = package.join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(package.join("NOLA.md"), "package rules\n").unwrap();
    let target = src.join("main.ts");
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
        "root".to_string(),
        Value::String(root.display().to_string()),
    );
    args.insert(
        "paths".to_string(),
        Value::Array(vec![Value::String(target.display().to_string())]),
    );

    let outcome = instruction_context(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("## AGENTS.md"));
    assert!(outcome.text.contains("root rules"));
    assert!(outcome.text.contains("always-on rule"));
    assert!(outcome.text.contains("typescript scoped rule"));
    assert!(!outcome.text.contains("python scoped rule"));
    assert!(!outcome.text.contains("paths:"));
    assert!(outcome.text.contains("package rules"));
    assert!(outcome.text.find("root rules").unwrap() < outcome.text.find("package rules").unwrap());
    assert!(outcome.display.contains("4 files"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn skill_context_discovers_and_loads_local_skills() {
    let root = temp_root("skill-context");
    let skill_dir = root.join("skills").join("review");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
            skill_dir.join("SKILL.md"),
            "# Skill: Review\ndescription: Review changes for correctness.\n\nUse this when reviewing a patch.\n",
        )
        .unwrap();
    fs::write(skill_dir.join("checklist.md"), "- correctness\n").unwrap();
    let ctx = ToolContext {
        memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
        language_services: Default::default(),
        skills: crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(Some(vec![root.clone()])),
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

    let listing = skill_context(&args, &ctx);

    assert!(!listing.error, "{}", listing.text);
    assert!(listing.text.contains("## Available skills"));
    assert!(listing.text.contains("Review (skills/review/SKILL.md)"));

    args.insert("name".to_string(), Value::String("Review".to_string()));
    let loaded = skill_context(&args, &ctx);

    assert!(!loaded.error, "{}", loaded.text);
    assert!(loaded.text.contains("## Skill: Review"));
    assert!(loaded
        .text
        .contains("description: Review changes for correctness."));
    assert!(loaded.text.contains("Use this when reviewing a patch."));
    assert!(loaded.text.contains("base_directory:"));
    assert!(loaded
        .text
        .contains("relative paths in this skill are resolved from base_directory."));
    assert!(loaded.text.contains("- checklist.md"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn skill_context_resolves_exact_name_across_configured_skill_directories() {
    let workspace = temp_root("skill-context-workspace");
    let skill_dir = temp_root("skill-context-trusted").join("word");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: word-document-authoring\ndescription: Create Word documents.\n---\n\n# Create Word Document\n\nUse the bundled generator.\n",
    )
    .unwrap();
    let mut args = Map::new();
    args.insert(
        "root".to_string(),
        Value::String(workspace.display().to_string()),
    );
    args.insert(
        "name".to_string(),
        Value::String("word-document-authoring".to_string()),
    );
    args.insert("max_skills".to_string(), Value::Number(1.into()));
    args.insert("include_files".to_string(), Value::Bool(false));

    let (discovered, _) = discover_skill_files(&skill_dir, 100);
    assert_eq!(discovered, vec![skill_dir.join("SKILL.md")]);
    let (content, _, _) = read_skill_file(&discovered[0], 40_000).unwrap();
    assert_eq!(
        skill_title(&discovered[0], &content),
        "word-document-authoring"
    );

    let loaded = skill_context_with_dirs(&args, &[workspace.clone(), skill_dir.clone()]);

    assert!(!loaded.error, "{}", loaded.text);
    assert!(loaded.text.contains("## Skill: word-document-authoring"));
    assert!(loaded.text.contains("Use the bundled generator."));
    assert!(loaded
        .text
        .contains(&format!("base_directory: {}", skill_dir.display())));
    let _ = fs::remove_dir_all(workspace);
    let _ = fs::remove_dir_all(skill_dir.parent().unwrap());
}

#[test]
fn skill_context_resolves_stable_id_query_across_configured_directories() {
    let workspace = temp_root("skill-context-query-workspace");
    let skill_dir = temp_root("skill-context-query-trusted").join("ocr");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: extract-image-text\ndescription: Extract image text.\n---\n\nUse the bundled OCR generator.\n",
    )
    .unwrap();
    let mut args = Map::new();
    args.insert(
        "root".to_string(),
        Value::String(workspace.display().to_string()),
    );
    args.insert(
        "query".to_string(),
        Value::String("extract-image-text".to_string()),
    );
    args.insert("max_skills".to_string(), Value::Number(1.into()));
    args.insert("include_files".to_string(), Value::Bool(false));

    let loaded = skill_context_with_dirs(&args, &[workspace.clone(), skill_dir.clone()]);

    assert!(!loaded.error, "{}", loaded.text);
    assert!(loaded.text.contains("## Skill: extract-image-text"));
    assert!(loaded.text.contains("Use the bundled OCR generator."));
    let _ = fs::remove_dir_all(workspace);
    let _ = fs::remove_dir_all(skill_dir.parent().unwrap());
}

#[test]
fn skill_execution_context_precedes_long_instructions_and_preserves_workspace() {
    let workspace = temp_root("skill-execution-workspace");
    let skill_root = temp_root("skill-execution-trusted");
    let skill_dir = skill_root.join("word-document-authoring");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: word-document-authoring\ndescription: Create Word documents.\n---\n\nFollow the bundled workflow.\n",
    )
    .unwrap();
    fs::write(
        skill_dir.join("NOLA_MANIFEST.json"),
        r#"{"version":"1.0.0","required_tools":[]}"#,
    )
    .unwrap();
    let ctx = ToolContext {
        memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
        language_services: Default::default(),
        skills: crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(Some(vec![skill_root.clone()])),
        allowed_dirs: vec![workspace.clone()],
        policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), true, "allow".to_string(), Vec::new()),
        permission_scope: Default::default(),
        cancellation: Default::default(),
        plan_state: Default::default(),
        output_store: crate::tools::ToolOutputStore::for_test(),
        browser: BrowserRuntime::new(temp_root("skill-execution-browser")),
    };
    let mut args = Map::new();
    args.insert(
        "skill".to_string(),
        Value::String("word-document-authoring".to_string()),
    );

    let invoked = skill_invoke(&args, &ctx);
    assert!(!invoked.error, "{}", invoked.text);
    let payload: Value = serde_json::from_str(&invoked.text).unwrap();
    let execution = payload
        .get("_nola_execution_context")
        .expect("compact execution context");
    assert_eq!(
        execution.get("workspace_root").and_then(Value::as_str),
        Some(workspace.to_string_lossy().as_ref())
    );
    assert_eq!(
        execution.get("base_directory").and_then(Value::as_str),
        Some(skill_dir.to_string_lossy().as_ref())
    );
    let compact_position = invoked.text.find("_nola_execution_context").unwrap();
    let instructions_position = invoked.text.find("\"instructions\"").unwrap();
    assert!(compact_position < instructions_position);

    let _ = fs::remove_dir_all(workspace);
    let _ = fs::remove_dir_all(skill_root);
}

#[test]
fn skill_context_rejects_unknown_exact_name_instead_of_silent_empty_success() {
    let workspace = temp_root("skill-context-missing");
    fs::create_dir_all(&workspace).unwrap();
    let mut args = Map::new();
    args.insert(
        "root".to_string(),
        Value::String(workspace.display().to_string()),
    );
    args.insert(
        "name".to_string(),
        Value::String("missing-skill".to_string()),
    );

    let outcome = skill_context_with_dirs(&args, std::slice::from_ref(&workspace));

    assert!(outcome.error);
    assert!(outcome.text.contains("No enabled Skill matched"));
    assert!(outcome.text.contains("instead of improvising"));
    let _ = fs::remove_dir_all(workspace);
}

#[test]
fn nola_orchestrate_validates_and_canonicalizes_native_automation() {
    let _env_guard = crate::store::test_env_lock();
    let database_root = temp_root("orchestrate-store");
    std::env::set_var("NOLA_DB_PATH", database_root.join("nola.db"));
    let skill_root = database_root.join("review-fixture");
    fs::create_dir(&skill_root).unwrap();
    fs::write(skill_root.join("SKILL.md"), "---\nname: review-fixture\ndescription: isolated review\n---\nReview data.\n").unwrap();
    let skills = SkillRuntimeContext::from_configured_dirs(Some(vec![skill_root]));
    let skill_id = skills.index().unwrap()[0]["id"].as_str().unwrap().to_string();
    let mut args = Map::new();
    args.insert(
        "name".to_string(),
        Value::String("Daily market brief".to_string()),
    );
    args.insert(
        "prompt".to_string(),
        Value::String("  Analyze the latest market data and create a brief.\n".to_string()),
    );
    args.insert(
        "skill_id".to_string(),
        Value::String(skill_id.clone()),
    );
    args.insert("frequency".to_string(), Value::String("daily".to_string()));
    args.insert("execution_spec".to_string(), json!({
        "kind": "agent", "spec": {
            "prompt": "  Analyze the latest market data and create a brief.\n",
            "execution_scope": {"kind": "local_workspace", "root": database_root},
            "permission_profile": "cautious",
            "model_selection": "use_current_default_at_admission",
            "context_refs": [
                {"kind":"personal_data","id":"platform:reminders"},
                {"kind":"web_source","id":"a16c0016-0016-4016-a016-000000000016"}
            ], "skill_refs": [skill_id]
        }
    }));
    args.insert("hour".to_string(), Value::Number(8.into()));
    args.insert("minute".to_string(), Value::Number(30.into()));
    args.insert(
        "context_sources".to_string(),
        json!(["personal_data", "web_sources"]),
    );
    args.insert(
        "web_sources".to_string(),
        json!([{
            "id": "a16c0016-0016-4016-a016-000000000016",
            "name": "Market feed",
            "url": "https://example.com/feed.json",
            "format": "json",
            "refresh_frequency": "daily"
        }]),
    );

    let outcome = nola_orchestrate(&args, &skills);
    assert!(!outcome.error, "{}", outcome.text);
    let value: Value = serde_json::from_str(&outcome.text).unwrap();
    assert_eq!(value["status"], "persisted");
    assert_eq!(value["automation"]["skillId"], skill_id);
    assert_eq!(value["automation"]["executionSpec"]["spec"]["skill_refs"], json!([skill_id]));
    assert_eq!(value["automation"]["executionSpec"]["spec"]["prompt"], args["prompt"]);
    assert_eq!(value["automation"]["userPrompt"], args["prompt"]);
    assert_eq!(
        value["automation"]["webSources"][0]["url"],
        "https://example.com/feed.json"
    );
    assert_eq!(value["persisted"], true);
    assert_eq!(value["read_back_verified"], true);
    assert_eq!(value["native_commit_required"], false);
    assert_eq!(outcome.artifacts.len(), 1);
    let artifact = &outcome.artifacts[0];
    assert!(artifact.uri.starts_with("nola://automation/act-agent-"));
    assert_eq!(artifact.mime_type.as_deref(), Some("application/json"));
    assert_eq!(artifact.operation.as_deref(), Some("upsert"));
    artifact.validate().unwrap();

    let disabled = SkillRuntimeContext::from_configured_dirs(Some(vec![]));
    assert!(nola_orchestrate(&args, &disabled).error);
    let mut mismatched = args.clone();
    mismatched.get_mut("execution_spec").unwrap()["spec"]["skill_refs"] = json!([]);
    let rejected = nola_orchestrate(&mismatched, &skills);
    assert!(rejected.error);
    assert!(rejected.text.contains("must match execution_spec"));
    args.insert(
        "skill_id".to_string(),
        Value::String("unknown-skill".to_string()),
    );
    let rejected = nola_orchestrate(&args, &skills);
    assert!(rejected.error);
    assert!(rejected.text.contains("not available in the current Host catalog"));
    std::env::remove_var("NOLA_DB_PATH");
    let _ = fs::remove_dir_all(database_root);
}

#[test]
fn agent_context_discovers_and_loads_custom_agents() {
    let root = temp_root("agent-context");
    let claude_agents = root.join(".claude").join("agents");
    let opencode_agents = root.join(".opencode").join("agents");
    fs::create_dir_all(&claude_agents).unwrap();
    fs::create_dir_all(&opencode_agents).unwrap();
    fs::write(
            claude_agents.join("reviewer.md"),
            "---\nname: reviewer\ndescription: Reviews code changes for correctness.\ntools:\n  - read_file\n  - git_diff\n---\nInspect the diff, identify regressions, and request focused tests.\n",
        )
        .unwrap();
    fs::write(
            opencode_agents.join("explore.md"),
            "---\ndescription: Maps unfamiliar code before edits.\nmode: subagent\nmodel: haiku\n---\nBuild a concise repository map and list likely files to read.\n",
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

    let listing = agent_context(&args, &ctx);

    assert!(!listing.error, "{}", listing.text);
    assert!(listing.text.contains("## Available custom agents"));
    assert!(listing
        .text
        .contains("reviewer (.claude/agents/reviewer.md)"));
    assert!(listing
        .text
        .contains("explore (.opencode/agents/explore.md) [subagent, haiku]"));
    assert!(!listing.text.contains("Inspect the diff"));

    args.insert("name".to_string(), Value::String("reviewer".to_string()));
    let loaded = agent_context(&args, &ctx);

    assert!(!loaded.error, "{}", loaded.text);
    assert!(loaded.text.contains("## Agent: reviewer"));
    assert!(loaded
        .text
        .contains("description: Reviews code changes for correctness."));
    assert!(loaded.text.contains("- tools: read_file, git_diff"));
    assert!(loaded
        .text
        .contains("Inspect the diff, identify regressions"));
    assert!(!loaded.text.contains("name: reviewer"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn command_context_discovers_and_loads_local_commands() {
    let root = temp_root("command-context");
    let claude_commands = root.join(".claude").join("commands");
    let opencode_commands = root.join(".opencode").join("commands");
    fs::create_dir_all(&claude_commands).unwrap();
    fs::create_dir_all(&opencode_commands).unwrap();
    fs::write(
            claude_commands.join("review.md"),
            "---\ndescription: Review current patch for regressions.\nargument-hint: [optional path]\nallowed-tools:\n  - git_diff\n  - read_file\n---\nInspect the current diff and produce findings first.\n",
        )
        .unwrap();
    fs::write(
            opencode_commands.join("explore.md"),
            "---\ndescription: Explore unfamiliar code before edits.\nagent: explorer\nmodel: haiku\nsubtask: true\n---\nMap the codebase and list likely files.\n",
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

    let listing = command_context(&args, &ctx);

    assert!(!listing.error, "{}", listing.text);
    assert!(listing.text.contains("## Available custom commands"));
    assert!(listing
        .text
        .contains("/review (.claude/commands/review.md)"));
    assert!(listing
        .text
        .contains("/explore (.opencode/commands/explore.md) [explorer, haiku, true]"));
    assert!(!listing.text.contains("Inspect the current diff"));

    args.insert("name".to_string(), Value::String("review".to_string()));
    let loaded = command_context(&args, &ctx);

    assert!(!loaded.error, "{}", loaded.text);
    assert!(loaded.text.contains("## Command: /review"));
    assert!(loaded
        .text
        .contains("description: Review current patch for regressions."));
    assert!(loaded.text.contains("- allowed-tools: git_diff, read_file"));
    assert!(loaded.text.contains("- argument-hint: [optional path]"));
    assert!(loaded
        .text
        .contains("Inspect the current diff and produce findings first."));
    assert!(!loaded.text.contains("---"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn agent_config_context_discovers_redacts_and_loads_project_agent_config() {
    let root = temp_root("agent-config-context");
    let claude_dir = root.join(".claude");
    let opencode_dir = root.join(".opencode");
    let tool_dir = opencode_dir.join("tool");
    fs::create_dir_all(&claude_dir).unwrap();
    fs::create_dir_all(&tool_dir).unwrap();
    fs::write(
            claude_dir.join("settings.json"),
            r#"{"permissions":{"defaultMode":"acceptEdits","allow":["Bash(npm test)","Edit"],"deny":["Bash(rm -rf *)"]},"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"command":"npm run lint"}]}]},"apiKey":"sk-secret"}"#,
        )
        .unwrap();
    fs::write(
        root.join(".mcp.json"),
        r#"{"mcpServers":{"github":{"command":"gh","token":"ghp_secret"}}}"#,
    )
    .unwrap();
    fs::write(
            opencode_dir.join("opencode.json"),
            r#"{"model":"anthropic/claude-sonnet-4-5","provider":{"anthropic":{"options":{"apiKey":"sk-provider"}}},"plugin":["./plugins/smoke.ts"],"agent":{"reviewer":{"model":"haiku"}}}"#,
        )
        .unwrap();
    fs::write(
            tool_dir.join("github-triage.ts"),
            "import { tool } from '@opencode-ai/plugin'\nexport default tool({ description: 'triage issues' })\n",
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

    let listing = agent_config_context(&args, &ctx);

    assert!(!listing.error, "{}", listing.text);
    assert!(listing.text.contains("## Available agent config files"));
    assert!(listing
        .text
        .contains(".claude/settings.json [Claude settings]"));
    assert!(listing.text.contains(".mcp.json [MCP config]"));
    assert!(listing
        .text
        .contains(".opencode/opencode.json [OpenCode config]"));
    assert!(listing
        .text
        .contains(".opencode/tool/github-triage.ts [OpenCode custom tool]"));
    assert!(listing
        .text
        .contains("permissions.defaultMode: acceptEdits"));
    assert!(listing.text.contains("mcpServers: github"));
    assert!(!listing.text.contains("sk-secret"));

    args.insert("query".to_string(), Value::String("github".to_string()));
    args.insert("include_content".to_string(), Value::Bool(true));
    let loaded = agent_config_context(&args, &ctx);

    assert!(!loaded.error, "{}", loaded.text);
    assert!(loaded.text.contains("## .mcp.json"));
    assert!(loaded.text.contains("## .opencode/tool/github-triage.ts"));
    assert!(loaded.text.contains("<redacted>"));
    assert!(!loaded.text.contains("ghp_secret"));
    assert!(loaded.text.contains("triage issues"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn agent_config_context_bounds_plugin_scanning() {
    let root = temp_root("agent-config-context-bounded");
    let plugin_dir = root.join(".claude/plugins/generated");
    fs::create_dir_all(&plugin_dir).unwrap();
    for index in 0..1_500 {
        fs::write(plugin_dir.join(format!("noise-{index}.bin")), b"noise").unwrap();
    }
    fs::write(
        root.join(".claude/settings.json"),
        r#"{"permissions":{"defaultMode":"acceptEdits"}}"#,
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
    let args = serde_json::json!({"root": root})
        .as_object()
        .unwrap()
        .clone();

    let listing = agent_config_context(&args, &ctx);

    assert!(!listing.error, "{}", listing.text);
    assert!(listing.text.contains(".claude/settings.json"));
    let scanned = listing
        .text
        .lines()
        .find_map(|line| line.strip_prefix("scanned_files: "))
        .and_then(|value| value.parse::<usize>().ok())
        .expect("scanned file count");
    assert!(scanned <= 6_000, "scan budget was not enforced: {scanned}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn ci_context_discovers_workflows_and_task_runners() {
    let root = temp_root("ci-context");
    let workflows = root.join(".github").join("workflows");
    fs::create_dir_all(&workflows).unwrap();
    fs::write(
            workflows.join("ci.yml"),
            "name: CI\non: [push]\njobs:\n  test:\n    runs-on: ubuntu-latest\n    steps:\n      - run: npm ci\n      - run: npm test\n  lint:\n    runs-on: ubuntu-latest\n    steps:\n      - run: npm run lint\n",
        )
        .unwrap();
    fs::write(
        root.join("Makefile"),
        "test:\n\tpytest\n\nlint:\n\truff check .\n",
    )
    .unwrap();
    fs::write(
        root.join("justfile"),
        "fmt:\n    cargo fmt\n\ncheck:\n    cargo check\n",
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

    let listing = ci_context(&args, &ctx);

    assert!(!listing.error, "{}", listing.text);
    assert!(listing
        .text
        .contains(".github/workflows/ci.yml [GitHub Actions]"));
    assert!(listing.text.contains("jobs: test, lint"));
    assert!(listing
        .text
        .contains("run: npm ci | npm test | npm run lint"));
    assert!(listing.text.contains("Makefile [Makefile]"));
    assert!(listing.text.contains("targets: test, lint"));
    assert!(listing.text.contains("justfile [justfile]"));
    assert!(listing.text.contains("recipes: fmt, check"));
    assert!(listing.text.contains("- make test"));
    assert!(listing.text.contains("- make lint"));
    assert!(listing.text.contains("- just check"));

    args.insert("include_content".to_string(), Value::Bool(true));
    let loaded = ci_context(&args, &ctx);

    assert!(!loaded.error, "{}", loaded.text);
    assert!(loaded.text.contains("### .github/workflows/ci.yml"));
    assert!(loaded.text.contains("npm run lint"));

    let mut plan_args = Map::new();
    plan_args.insert(
        "root".to_string(),
        Value::String(root.display().to_string()),
    );
    let plan = validation_plan(&plan_args, &ctx);

    assert!(!plan.error, "{}", plan.text);
    assert!(plan.text.contains("- make test"));
    assert!(plan.text.contains("- make lint"));
    assert!(plan.text.contains("- just check"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn coding_conventions_reports_style_signals() {
    let root = temp_root("coding-conventions");
    let src = root.join("src");
    let tests = root.join("tests");
    fs::create_dir_all(&src).unwrap();
    fs::create_dir_all(&tests).unwrap();
    fs::write(root.join(".editorconfig"), "indent_size = 4\n").unwrap();
    let service = src.join("service.py");
    let test_file = tests.join("test_service.py");
    fs::write(
        &service,
        "# SPDX-License-Identifier: Apache-2.0\ndef load_user():\n    return {'id': 1}\n",
    )
    .unwrap();
    fs::write(
        &test_file,
        "# SPDX-License-Identifier: Apache-2.0\ndef test_load_user():\n    assert load_user()\n",
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
    args.insert("glob".to_string(), Value::String("**/*.py".to_string()));
    args.insert("max_files".to_string(), Value::from(10));

    let outcome = coding_conventions(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("## Config files"));
    assert!(outcome.text.contains("- .editorconfig"));
    assert!(outcome.text.contains("src/service.py"));
    assert!(outcome.text.contains("tests/test_service.py"));
    assert!(outcome.text.contains("- Python: 2"));
    assert!(outcome.text.contains("- spaces: 4"));
    assert!(outcome
        .text
        .contains("# SPDX-License-Identifier: Apache-2.0"));
    assert!(outcome.text.contains("- pytest: 1"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn file_tree_reports_compact_tree_and_skips_common_dirs() {
    let root = temp_root("file-tree");
    let src = root.join("src");
    let nested = src.join("nested");
    let node_modules = root.join("node_modules");
    fs::create_dir_all(&nested).unwrap();
    fs::create_dir_all(&node_modules).unwrap();
    fs::write(src.join("app.py"), "print('hi')\n").unwrap();
    fs::write(nested.join("deep.py"), "print('deep')\n").unwrap();
    fs::write(node_modules.join("ignored.js"), "ignored\n").unwrap();
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
    args.insert("max_depth".to_string(), Value::from(3));

    let outcome = file_tree(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("# File tree:"));
    assert!(outcome.text.contains("- src/"));
    assert!(outcome.text.contains("- nested/"));
    assert!(outcome.text.contains("- app.py"));
    assert!(outcome.text.contains("- deep.py"));
    assert!(!outcome.text.contains("node_modules"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn find_files_finds_name_fragments_and_skips_common_dirs() {
    let root = temp_root("find-files");
    let src = root.join("src");
    let node_modules = root.join("node_modules");
    fs::create_dir_all(&src).unwrap();
    fs::create_dir_all(&node_modules).unwrap();
    fs::write(src.join("ModelsScreen.kt"), "class ModelsScreen\n").unwrap();
    fs::write(node_modules.join("ModelsIgnored.kt"), "ignored\n").unwrap();
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
    args.insert("query".to_string(), Value::String("models".to_string()));

    let outcome = find_files(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("file src/ModelsScreen.kt"));
    assert!(!outcome.text.contains("ModelsIgnored"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn discovery_roots_resolve_dot_and_slash_to_the_allowed_workspace() {
    let root = temp_root("discovery-workspace-root");
    fs::write(root.join("note.txt"), "workspace fixture\n").unwrap();
    let ctx = ToolContext {
        memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
        language_services: Default::default(),
        skills: Default::default(),
        allowed_dirs: vec![root.clone()],
        policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), false, "allow".to_string(), Vec::new()),
        permission_scope: Default::default(),
        cancellation: Default::default(),
        plan_state: Default::default(),
        output_store: crate::tools::ToolOutputStore::for_test(),
        browser: BrowserRuntime::new(temp_root("browser-runtime")),
    };

    for requested_root in [".", "/"] {
        let args = object_args(json!({
            "root": requested_root,
            "query": "note"
        }));
        let outcome = find_files(&args, &ctx);

        assert!(
            !outcome.error,
            "root {requested_root} should resolve to the workspace: {}",
            outcome.text
        );
        assert!(outcome.text.contains("file note.txt"));
        assert!(outcome.text.contains(&root.display().to_string()));
    }

    assert_eq!(resolve_allowed("/", &ctx.allowed_dirs, true).unwrap(), PathBuf::from("/"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn find_files_stops_at_the_scan_budget_and_skips_symlink_escape() {
    let root = temp_root("find-files-budget");
    let outside = temp_root("find-files-outside");
    fs::create_dir_all(root.join("nested")).unwrap();
    for index in 0..20 {
        fs::write(
            root.join("nested").join(format!("entry-{index}.txt")),
            "fixture\n",
        )
        .unwrap();
    }
    fs::write(outside.join("secret-match.txt"), "outside\n").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, root.join("outside-link")).unwrap();
    let ctx = ToolContext {
        memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
        language_services: Default::default(),
        skills: Default::default(),
        allowed_dirs: vec![root.clone()],
        policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), false, "allow".to_string(), Vec::new()),
        permission_scope: Default::default(),
        cancellation: Default::default(),
        plan_state: Default::default(),
        output_store: crate::tools::ToolOutputStore::for_test(),
        browser: BrowserRuntime::new(temp_root("browser-runtime")),
    };
    let args = object_args(json!({
        "root": root.display().to_string(),
        "query": "match",
        "max_scanned_entries": 5
    }));

    let outcome = find_files(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("scan limit reached"));
    assert!(!outcome.text.contains("secret-match"));
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(outside);
}

#[test]
fn find_files_skips_build_outputs_and_obeys_wall_clock_budget() {
    let root = temp_root("find-files-duration");
    fs::create_dir_all(root.join(".build").join("products")).unwrap();
    fs::create_dir_all(root.join("source")).unwrap();
    fs::write(root.join(".build/products/create_document.py"), "ignored\n").unwrap();
    fs::write(root.join("source/create_document.py"), "visible\n").unwrap();
    let ctx = ToolContext {
        memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
        language_services: Default::default(),
        skills: Default::default(),
        allowed_dirs: vec![root.clone()],
        policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), false, "allow".to_string(), Vec::new()),
        permission_scope: Default::default(),
        cancellation: Default::default(),
        plan_state: Default::default(),
        output_store: crate::tools::ToolOutputStore::for_test(),
        browser: BrowserRuntime::new(temp_root("browser-runtime")),
    };
    let args = object_args(json!({
        "root": root.display().to_string(),
        "query": "create_document.py",
        "max_duration_ms": 3000
    }));

    let started = std::time::Instant::now();
    let outcome = find_files(&args, &ctx);

    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("file source/create_document.py"));
    assert!(!outcome.text.contains(".build/products"));
    assert!(started.elapsed() < std::time::Duration::from_secs(4));
    let _ = fs::remove_dir_all(root);
}
