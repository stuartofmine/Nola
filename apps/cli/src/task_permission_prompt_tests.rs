// SPDX-License-Identifier: Apache-2.0
//! A delayed panel choice cannot be redirected to its Task's next request.
use super::*;

#[test]
fn stale_permission_panel_never_authorizes_the_next_request_of_the_same_task() {
    let _environment = crate::store::test_env_lock();
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("permissions.sqlite");
    let previous_database = std::env::var_os("NOLA_DB_PATH");
    std::env::set_var("NOLA_DB_PATH", &database);
    struct Restore(Option<std::ffi::OsString>);
    impl Drop for Restore {
        fn drop(&mut self) {
            if let Some(value) = &self.0 { std::env::set_var("NOLA_DB_PATH", value); }
            else { std::env::remove_var("NOLA_DB_PATH"); }
        }
    }
    let _restore = Restore(previous_database);
    let mut config = crate::host_invocation_request::HostInvocationRequest::for_test().unwrap();
    config.tool_settings.project_root = root.path().into();
    config.tool_settings.allowed_dirs = vec![root.path().into()];
    config.session.session_id = Some("owner".into());
    let store = NolaStore::open_default().unwrap();
    let mut task: DbRuntimeTask = serde_json::from_value(json!({
        "id":"task", "kind":"agent", "status":"queued", "title":"Two requests",
        "project_root":root.path().to_string_lossy(),"owner_session_id":"owner","child_session_id":"child",
        "unread":true,"cancel_requested":false,"retry_count":0,"created_at_ms":1,"updated_at_ms":1,"metadata_json":"{}"
    })).unwrap();
    task.set_scheduling(nola_application::RuntimeTaskScheduling {
        priority: nola_application::RuntimeTaskPriority::Normal,
        queued_at_ms: task.created_at_ms,
        resource_budget: nola_application::RuntimeTaskResourceBudget::default(),
        output_bytes: 0,
        resource_usage: nola_application::RuntimeTaskResourceUsage::default(),
        budget_exhaustion: None,
    }).unwrap();
    store.create_runtime_task(&task).unwrap();
    // Isolate the panel's read-only observation/choice contract. Task creation
    // still uses the mandatory queued admission; no worker or grant is issued.
    let inspect = rusqlite::Connection::open(&database).unwrap();
    inspect.execute("UPDATE durable_tasks SET status='waiting_permission' WHERE id='task'", []).unwrap();
    let make_request = |id: &str, path: &str| nola_application::NewPermissionRequest {
        request_id:id.into(),task_id:"task".into(),child_session_id:"child".into(),
        capability:"write_file".into(),input:json!({"path":root.path().join(path),"content":"fixture"}),
        scope_rule:format!("write_file:{}",root.path().join(path).display()),created_at_ms:2,
        core_permission_request_id:Some(format!("core-{id}")),permission_instructions:vec![],
    };
    let original = crate::permission_request_port::ensure_core_permission(make_request("first", "first.txt")).unwrap();
    let displayed = permission_prompt_for_task(&config, "task").expect("original request panel");
    assert_eq!(displayed.request_id(), original.request_id);
    // Another UI settled the first request and the Task now has a second
    // request. This fixture changes only observation state, never authorizes.
    inspect.execute("UPDATE permission_requests SET status='resolved',decision='deny',resolved_at_ms=3 WHERE request_id='first'", []).unwrap();
    let closed_original = crate::permission_request_port::permission_request(&original.request_id).unwrap().unwrap();
    let next = crate::permission_request_port::ensure_core_permission(make_request("second", "second.txt")).unwrap();
    assert_eq!(crate::permission_request_port::pending_permission_for_task("task").unwrap().unwrap().request_id, next.request_id);
    let receipt_count = || inspect.query_row::<i64,_,_>("SELECT COUNT(*) FROM command_receipts", [], |row| row.get(0)).unwrap();
    let before_choices = receipt_count();
    for choice in [PermissionChoice::Once, PermissionChoice::Session(original.scope_rule.clone()), PermissionChoice::Deny] {
        let outcome = resolve_task_permission_choice(&config, &displayed, choice);
        assert!(outcome.error, "A stale choice must be rejected, not retargeted: {}", outcome.text);
        let retained = crate::permission_request_port::permission_request(&next.request_id).unwrap().unwrap();
        assert_eq!(retained, next, "The new request and its decision must remain byte-for-byte unchanged");
        assert_eq!(receipt_count(), before_choices, "A stale panel must not submit any authorization command");
    }
    assert_eq!(crate::permission_request_port::permission_request(&original.request_id).unwrap().unwrap(), closed_original);
}
