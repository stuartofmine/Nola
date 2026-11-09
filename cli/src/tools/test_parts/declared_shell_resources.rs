// SPDX-License-Identifier: Apache-2.0
// Exact shell output resources are independent of the process working directory.

#[test]
fn environment_probe_rejects_commands_and_reports_unknown_authority() {
    let (fixture, cwd, _) = resource_fixture();
    let _environment = ResourceTestEnvironment::new(fixture.path());
    let ctx = resource_context(vec![cwd]);
    for name in ["../python", "python;touch marker", "/bin/sh", "$(id)", "-x"] {
        let outcome = environment_probe(json!({"executables":[name]}).as_object().unwrap(), &ctx);
        assert!(outcome.error, "{name}");
    }
    let outcome = environment_probe(json!({"executables":[]}).as_object().unwrap(), &ctx);
    assert!(!outcome.error);
    let data = outcome.data.unwrap();
    assert_eq!(data["network"], "not_checked");
    assert_eq!(data["installation_permission"], "not_established");
    assert_eq!(data["writable_locations"], "not_checked");
    assert!(outcome.artifacts.is_empty());
}

#[test]
fn failed_dependency_setup_retains_actual_exit_and_stderr() {
    let (fixture, cwd, _) = resource_fixture();
    let _environment = ResourceTestEnvironment::new(fixture.path());
    let ctx = resource_context(vec![cwd.clone()]);
    let _scope = ctx.permission_scope.enter();
    ctx.permission_scope.configure_profile(crate::permissions::PermissionProfile::AllowAll);
    for (index, diagnostic) in ["network unavailable", "permission denied", "package unavailable"].iter().enumerate() {
        let request = ToolRequest { id: format!("dependency-failure-{index}"), name: "bash".into(),
            args: json!({"command":format!("printf '{diagnostic}' >&2; exit 7"),
                "cwd":cwd, "operation_kind":"execute"}).as_object().unwrap().clone() };
        let outcome = execute_authorized_tool(&request, &ctx, ExecutionAuthority::CoreApproved(nola_domain::PermissionProfile::AllowAll));
        assert!(outcome.error);
        let data = outcome.data.as_ref().expect(&outcome.text);
        assert_eq!(data["exit_code"], 7);
        assert!(data["stderr"].as_str().unwrap().contains(diagnostic));
    }
}

#[test]
fn shell_contract_failure_preserves_installation_diagnostics() {
    let (fixture, cwd, _) = resource_fixture();
    let _environment = ResourceTestEnvironment::new(fixture.path());
    let ctx = resource_context(vec![cwd.clone()]);
    let _scope = ctx.permission_scope.enter();
    ctx.permission_scope.configure_profile(crate::permissions::PermissionProfile::AllowAll);
    let target = cwd.join("dependency.marker");
    fs::write(&target, "original").unwrap();
    let request = ToolRequest { id: "dependency-no-change".into(), name: "bash".into(),
        args: json!({"command":"printf 'installer diagnostic'; printf 'network unavailable' >&2",
            "cwd":cwd, "operation_kind":"mutate",
            "output_artifacts":[{"path":target,"operation":"edit"}]}).as_object().unwrap().clone() };
    let outcome = execute_authorized_tool(&request, &ctx, ExecutionAuthority::CoreApproved(nola_domain::PermissionProfile::AllowAll));
    assert!(outcome.error, "{}", outcome.text);
    let data = outcome.data.as_ref().expect(&outcome.text);
    assert_eq!(data["exit_code"], 0);
    assert!(data["stdout"].as_str().unwrap().contains("installer diagnostic"));
    assert!(data["stderr"].as_str().unwrap().contains("network unavailable"));
    assert_eq!(data["output_contract_failure"]["observed"], "unchanged");
    assert_eq!(fs::read_to_string(target).unwrap(), "original");
}

#[test]
#[cfg(unix)]
fn output_observation_includes_permission_changes_without_content_changes() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("script.sh");
    fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    let before = capture_file_artifact_states([&path]).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    let after = capture_file_artifact_states([&path]).unwrap();
    assert_eq!(observed_file_artifact_operation(&path, &before, &after), Some("edit"));
    assert_eq!(fs::read_to_string(&path).unwrap(), "#!/bin/sh\nexit 0\n");
    let unchanged = capture_file_artifact_states([&path]).unwrap();
    assert_eq!(observed_file_artifact_operation(&path, &after, &unchanged), None);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    let restored = capture_file_artifact_states([&path]).unwrap();
    assert_eq!(before, restored);
}

#[test]
fn allow_all_shell_executes_outside_workspace_without_directory_reapproval() {
    let (fixture, cwd, outside) = resource_fixture();
    let _environment = ResourceTestEnvironment::new(fixture.path());
    let ctx = resource_context(vec![cwd]);
    let _scope = ctx.permission_scope.enter();
    ctx.permission_scope.configure_profile(crate::permissions::PermissionProfile::AllowAll);
    let request = ToolRequest { id: "allow-all-outside".into(), name: "bash".into(),
        args: json!({"command":"pwd && printf approved", "cwd":outside,
            "operation_kind":"inspect"}).as_object().unwrap().clone() };
    let result = execute_authorized_tool(&request, &ctx, ExecutionAuthority::CoreApproved(nola_domain::PermissionProfile::AllowAll));
    assert!(!result.error, "{}", result.text);
    assert!(result.data.as_ref().unwrap()["stdout"].as_str().unwrap().contains("approved"));
    ctx.permission_scope.configure_profile(crate::permissions::PermissionProfile::Cautious);
    let admitted = execute_authorized_tool(&request, &ctx, ExecutionAuthority::CoreApproved(nola_domain::PermissionProfile::AllowAll));
    assert!(!admitted.error,"the effect's allow_all policy overrides a stale cautious Host scope: {}",admitted.text);
    let restricted = execute_authorized_tool(&request, &ctx, ExecutionAuthority::CoreApproved(nola_domain::PermissionProfile::Cautious));
    assert!(!restricted.error, "{}", restricted.text);
}

struct ResourceTestEnvironment {
    previous: Vec<(&'static str, Option<std::ffi::OsString>)>,
    // Hold the same process-wide lock as every other database fixture until
    // Drop restores the environment, including on assertion panic.
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl ResourceTestEnvironment {
    fn new(base: &Path) -> Self {
        let lock = crate::store::test_env_lock();
        let values = [
            ("NOLA_HOME", base.join("home")),
            ("NOLA_DB_PATH", base.join("state/fixture.db")),
            ("NOLA_MAC_APP_SUPPORT", base.join("mac")),
        ];
        let previous = values
            .iter()
            .map(|(name, _)| (*name, std::env::var_os(name)))
            .collect();
        for (name, value) in values {
            std::env::set_var(name, value);
        }
        Self { previous, _lock: lock }
    }
}

impl Drop for ResourceTestEnvironment {
    fn drop(&mut self) {
        for (name, value) in &self.previous {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
    }
}

fn resource_context(roots: Vec<PathBuf>) -> ToolContext {
    ToolContext {
        memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
        language_services: Default::default(),
        skills: Default::default(),
        browser: BrowserRuntime::new(roots[0].join("browser")),
        allowed_dirs: roots,
        policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), true, "allow".into(), Vec::new()),
        permission_scope: Default::default(),
        cancellation: Default::default(),
        plan_state: Default::default(),
        output_store: ToolOutputStore::for_test(),
    }
}

fn resource_fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let fixture = tempfile::tempdir().unwrap();
    let cwd = fixture.path().join("source");
    let other = fixture.path().join("destination");
    fs::create_dir_all(&cwd).unwrap();
    fs::create_dir_all(&other).unwrap();
    (
        fixture,
        cwd.canonicalize().unwrap(),
        other.canonicalize().unwrap(),
    )
}

fn resource_request(id: &str, cwd: &Path, command: String, outputs: Value) -> ToolRequest {
    ToolRequest {
        id: id.into(),
        name: "bash".into(),
        args: json!({"command":command,"cwd":cwd,"operation_kind":"mutate",
            "execution_mode":"foreground","output_artifacts":outputs})
        .as_object()
        .unwrap()
        .clone(),
    }
}

fn checkpoint_for(request: &ToolRequest, ctx: &ToolContext) -> WorkspaceCheckpoint {
    let work = format!("workspace:{}", ctx.allowed_dirs[0].display());
    let transaction =
        crate::work_step_port::work_step_transaction(&format!("work-step:{work}:{}", request.id))
            .unwrap()
            .unwrap();
    let checkpoint_id = transaction.workspace_checkpoint_id.unwrap();
    let store = NolaStore::open_default().unwrap();
    workspace_checkpoint_from_record(
        store
            .list_workspace_checkpoints(&ctx.allowed_dirs[0].to_string_lossy())
            .unwrap()
            .into_iter()
            .find(|record| record.id == checkpoint_id)
            .unwrap(),
    )
}

#[test]
fn declared_outputs_resolve_relative_paths_but_authorize_each_resource_not_cwd() {
    let (_fixture, cwd, other) = resource_fixture();
    let scope = SessionPermissionScope::default();
    let _scope = scope.enter();
    let target = other.join("new/report.txt");
    let args = json!({"output_artifacts":[{"path":target,"operation":"create"}]});
    let outputs = declared_bash_outputs(
        args.as_object().unwrap(),
        &cwd,
        &[cwd.clone(), other.clone()],
    )
    .unwrap();
    assert_eq!(outputs[0].path, target);
    assert!(!target.parent().unwrap().exists());
    assert!(
        capture_file_artifact_states(outputs.iter().map(|value| &value.path))
            .unwrap()
            .is_empty()
    );
    let relative =
        json!({"output_artifacts":[{"path":"../destination/new/report.txt","operation":"create"}]});
    assert_eq!(
        declared_bash_outputs(relative.as_object().unwrap(), &cwd, &[cwd.clone(), other]).unwrap(),
        outputs
    );
}

#[test]
fn output_declarations_allow_external_paths_but_reject_duplicate_resources() {
    let (_fixture, cwd, other) = resource_fixture();
    let scope = SessionPermissionScope::default();
    let _scope = scope.enter();
    let target = other.join("report.txt");
    let args = json!({"output_artifacts":[{"path":target,"operation":"create"}]});
    let outputs = declared_bash_outputs(args.as_object().unwrap(), &cwd, std::slice::from_ref(&cwd)).unwrap();
    assert_eq!(outputs.len(), 1);
    assert!(!target.exists());
    let duplicates = json!({"output_artifacts":[
        {"path":target,"operation":"create"},
        {"path":"../destination/report.txt","operation":"create"}]});
    assert!(declared_bash_outputs(
        duplicates.as_object().unwrap(),
        &cwd,
        &[cwd.clone(), other.clone()]
    )
    .unwrap_err()
    .text
    .contains("Duplicate"));
    let excess = json!({"output_artifacts":(0..101).map(|index| json!({
        "path":other.join(format!("output-{index}")),"operation":"create"})).collect::<Vec<_>>()});
    assert!(
        declared_bash_outputs(excess.as_object().unwrap(), &cwd, &[cwd.clone(), other])
            .unwrap_err()
            .text
            .contains("100 resources")
    );
}

#[test]
fn shell_creates_new_directory_outside_cwd_and_replays_exact_observed_outputs() {
    let (fixture, cwd, other) = resource_fixture();
    let _environment = ResourceTestEnvironment::new(fixture.path());
    let ctx = resource_context(vec![cwd.clone(), other.clone()]);
    fs::write(cwd.join("input.txt"), "generated\n").unwrap();
    fs::write(other.join("unrelated.txt"), "untouched\n").unwrap();
    let target = other.join("new/report.txt");
    let request = resource_request(
        "new-dir-output",
        &cwd,
        format!(
            "mkdir -p '{}' && cp input.txt '{}'",
            crate::platform_paths::shell_path(target.parent().unwrap()),
            crate::platform_paths::shell_path(&target)
        ),
        json!([{"path":target,"operation":"create"}]),
    );
    let first = execute_authorized_tool(&request, &ctx, ExecutionAuthority::CoreApproved(nola_domain::PermissionProfile::Cautious));
    assert!(!first.error, "{}", first.text);
    assert_eq!(fs::read_to_string(&target).unwrap(), "generated\n");
    assert_eq!(first.artifacts.len(), 1);
    assert_eq!(first.artifacts[0].operation.as_deref(), Some("create"));
    first.artifacts[0].validate().unwrap();
    let checkpoint = checkpoint_for(&request, &ctx);
    let manifest: Value = serde_json::from_str(&checkpoint.status).unwrap();
    assert_eq!(manifest["version"], 1);
    assert_eq!(manifest["resource_scoped"], true);
    assert_eq!(manifest["entries"].as_array().unwrap().len(), 1);
    assert_eq!(
        manifest["entries"][0]["resource_root"],
        other.to_string_lossy().as_ref()
    );
    assert_eq!(manifest["entries"][0]["relative_path"], "new/report.txt");
    assert_eq!(manifest["entries"][0]["existed"], false);
    fs::write(&target, "changed after commit\n").unwrap();
    let replay = execute_authorized_tool(&request, &ctx, ExecutionAuthority::CoreApproved(nola_domain::PermissionProfile::Cautious));
    assert!(!replay.error, "{}", replay.text);
    assert_eq!(replay.artifacts, first.artifacts);
    assert_eq!(
        fs::read_to_string(&target).unwrap(),
        "changed after commit\n"
    );
    let restored =
        file_preimage_checkpoint_action(&checkpoint, &cwd, &ctx, "restore", 40_000, false);
    assert!(!restored.error, "{}", restored.text);
    assert!(!target.exists());
    assert_eq!(
        fs::read_to_string(other.join("unrelated.txt")).unwrap(),
        "untouched\n"
    );
}

#[test]
fn admitted_resource_checkpoint_restores_external_files_after_cwd_scope_changes() {
    let (fixture, cwd, other) = resource_fixture();
    let _environment = ResourceTestEnvironment::new(fixture.path());
    let ctx = resource_context(vec![cwd.clone(), other.clone()]);
    let local = cwd.join("local.txt");
    let remote = other.join("remote.txt");
    fs::write(cwd.join("input.txt"), "updated\n").unwrap();
    fs::write(&local, "local baseline\n").unwrap();
    fs::write(&remote, "remote baseline\n").unwrap();
    let request = resource_request(
        "two-root-output",
        &cwd,
        format!(
            "cp input.txt local.txt && cp input.txt '{}'",
            crate::platform_paths::shell_path(&remote)
        ),
        json!([{"path":local,"operation":"edit"},{"path":remote,"operation":"edit"}]),
    );
    let changed = execute_authorized_tool(&request, &ctx, ExecutionAuthority::CoreApproved(nola_domain::PermissionProfile::Cautious));
    assert!(!changed.error, "{}", changed.text);
    assert_eq!(changed.artifacts.len(), 2);
    let checkpoint = checkpoint_for(&request, &ctx);
    let manifest: Value = serde_json::from_str(&checkpoint.status).unwrap();
    assert_eq!(manifest["entries"].as_array().unwrap().len(), 2);
    let revoked = resource_context(vec![cwd.clone()]);
    let scope = revoked.permission_scope.enter();
    let denied =
        file_preimage_checkpoint_action(&checkpoint, &cwd, &revoked, "restore", 40_000, false);
    assert!(!denied.error, "{}", denied.text);
    assert_eq!(fs::read_to_string(&local).unwrap(), "local baseline\n");
    assert_eq!(fs::read_to_string(&remote).unwrap(), "remote baseline\n");
    drop(scope);
}

#[test]
fn resource_checkpoint_rejects_changed_symlink_identity_before_any_restore() {
    #[cfg(unix)]
    {
        let (fixture, cwd, other) = resource_fixture();
        let _environment = ResourceTestEnvironment::new(fixture.path());
        let ctx = resource_context(vec![cwd.clone(), other.clone()]);
        let local = cwd.join("local.txt");
        let remote = other.join("remote.txt");
        fs::write(&local, "local baseline\n").unwrap();
        fs::write(&remote, "remote baseline\n").unwrap();
        let request = resource_request(
            "symlink-output",
            &cwd,
            "unused".into(),
            json!([{"path":local,"operation":"edit"},{"path":remote,"operation":"edit"}]),
        );
        let id = automatic_file_preimage_checkpoint(&request, &ctx, &cwd)
            .unwrap()
            .unwrap();
        let record = NolaStore::open_default()
            .unwrap()
            .list_workspace_checkpoints(&cwd.to_string_lossy())
            .unwrap()
            .into_iter()
            .find(|record| record.id == id)
            .unwrap();
        let checkpoint = workspace_checkpoint_from_record(record);
        fs::write(&local, "local changed\n").unwrap();
        let redirected = other.join("redirected.txt");
        fs::write(&redirected, "do not touch\n").unwrap();
        fs::remove_file(&remote).unwrap();
        std::os::unix::fs::symlink(&redirected, &remote).unwrap();
        let denied =
            file_preimage_checkpoint_action(&checkpoint, &cwd, &ctx, "restore", 40_000, false);
        assert!(denied.error);
        assert_eq!(fs::read_to_string(&local).unwrap(), "local changed\n");
        assert_eq!(fs::read_to_string(&redirected).unwrap(), "do not touch\n");
    }
}

#[test]
fn approved_operation_can_create_outputs_outside_workspace() {
    let (fixture, cwd, other) = resource_fixture();
    let _environment = ResourceTestEnvironment::new(fixture.path());
    let ctx = resource_context(vec![cwd.clone()]);
    fs::write(cwd.join("input.txt"), "payload\n").unwrap();
    let target = other.join("not-authorized.txt");
    let request = resource_request(
        "outside-grant-output",
        &cwd,
        format!("cp input.txt '{}'", crate::platform_paths::shell_path(&target)),
        json!([{"path":target,"operation":"create"}]),
    );
    let denied = execute_authorized_tool(&request, &ctx, ExecutionAuthority::CoreApproved(nola_domain::PermissionProfile::Cautious));
    assert!(!denied.error, "{}", denied.text);
    assert_eq!(fs::read_to_string(&target).unwrap(), "payload\n");
}

#[test]
fn resource_checkpoint_backup_budget_rejects_before_shell_changes_any_target() {
    let (fixture, cwd, other) = resource_fixture();
    let _environment = ResourceTestEnvironment::new(fixture.path());
    let ctx = resource_context(vec![cwd.clone(), other.clone()]);
    let target = other.join("large.txt");
    File::create(&target)
        .unwrap()
        .set_len(MAX_PREIMAGE_FILE_BYTES + 1)
        .unwrap();
    fs::write(cwd.join("input.txt"), "payload\n").unwrap();
    let marker = cwd.join("executed.txt");
    let request = resource_request(
        "large-output-budget",
        &cwd,
        format!(
            "cp input.txt executed.txt && cp input.txt '{}'",
            crate::platform_paths::shell_path(&target)
        ),
        json!([{"path":marker,"operation":"create"},{"path":target,"operation":"edit"}]),
    );
    let denied = execute_authorized_tool(&request, &ctx, ExecutionAuthority::CoreApproved(nola_domain::PermissionProfile::Cautious));
    assert!(denied.error);
    assert!(denied.text.contains("bounded recovery budget"));
    assert!(!marker.exists());
    assert_eq!(
        fs::metadata(&target).unwrap().len(),
        MAX_PREIMAGE_FILE_BYTES + 1
    );
}

#[test]
fn cancelled_resource_operation_does_not_spawn_a_command_for_approved_outputs() {
    let (fixture, cwd, other) = resource_fixture();
    let _environment = ResourceTestEnvironment::new(fixture.path());
    let ctx = resource_context(vec![cwd.clone(), other.clone()]);
    fs::write(cwd.join("input.txt"), "payload\n").unwrap();
    let target = other.join("cancelled.txt");
    let request = resource_request(
        "cancelled-output",
        &cwd,
        format!("cp input.txt '{}'", crate::platform_paths::shell_path(&target)),
        json!([{"path":target,"operation":"create"}]),
    );
    ctx.cancellation.request();
    let denied = execute_authorized_tool(&request, &ctx, ExecutionAuthority::CoreApproved(nola_domain::PermissionProfile::Cautious));
    assert!(denied.error);
    assert!(denied.text.contains("tool_execution_skipped"));
    assert!(!target.exists());
}

#[test]
fn sensitive_external_shell_read_executes_only_after_invocation_approval() {
    let (fixture, cwd, outside) = resource_fixture();
    let _environment = ResourceTestEnvironment::new(fixture.path());
    let ctx = resource_context(vec![cwd.clone()]);
    let secret = outside.join(".env");
    fs::write(&secret, "SYNTHETIC_PERMISSION_FIXTURE=approved\n").unwrap();
    let args = json!({"command":format!("cat '{}'", crate::platform_paths::shell_path(&secret)),
        "cwd":cwd,"operation_kind":"inspect", "sensitive_read_reason":"Read test fixture credentials"});
    let blocked = run_bash_execution(args.as_object().unwrap(), &ctx, false);
    assert!(blocked.error);
    assert!(!blocked.text.contains("SYNTHETIC_PERMISSION_FIXTURE=approved"));
    let request = ToolRequest {id:"sensitive-external-read".into(),name:"bash".into(),
        args:args.as_object().unwrap().clone()};
    let admitted = execute_authorized_tool(&request,&ctx,
        ExecutionAuthority::CoreApproved(nola_domain::PermissionProfile::Cautious));
    assert!(!admitted.error, "{}", admitted.text);
    assert!(admitted.data.as_ref().unwrap()["stdout"].as_str().unwrap().contains("SYNTHETIC_PERMISSION_FIXTURE=approved"));
}

#[test]
fn approved_shell_redirect_to_external_temporary_directory_is_not_blocked() {
    let (fixture, cwd, outside) = resource_fixture();
    let _environment = ResourceTestEnvironment::new(fixture.path());
    let ctx = resource_context(vec![cwd.clone()]);
    let target = outside.join("summary.txt");
    let request = ToolRequest {id:"external-redirect".into(),name:"bash".into(),
        args:json!({"command":format!("printf summary > '{}'",crate::platform_paths::shell_path(&target)),
            "cwd":cwd,"operation_kind":"execute"}).as_object().unwrap().clone()};
    let admitted = execute_authorized_tool(&request,&ctx,
        ExecutionAuthority::CoreApproved(nola_domain::PermissionProfile::Cautious));
    assert!(!admitted.error, "{}", admitted.text);
    assert_eq!(fs::read_to_string(target).unwrap(), "summary");
}
