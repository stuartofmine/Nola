// SPDX-License-Identifier: Apache-2.0
//! Embedded transport for durable Application command admission.

use super::{encode, utf8_input, NolaCoreApplication};
use nola_application::{ApplicationCommand, ExecuteReceivedCommand, ModelCommand};
use nola_protocol::{CommandEnvelope, APPLICATION_SERVICE_MAX_FRAME_BYTES};
use serde_json::{json, Value};
use std::os::raw::c_char;

fn failure(code: &str, message: impl ToString) -> *mut c_char {
    encode(
        json!({"ok": false, "error": {"code": code, "message": message.to_string(), "recoverable": false}}),
    )
}

fn query_result(envelope: &CommandEnvelope, result: Value) -> *mut c_char {
    let now = chrono::Utc::now().to_rfc3339();
    encode(json!({"ok": true, "result": nola_protocol::CommandReceipt {
        protocol_version: envelope.protocol_version,
        client_id: envelope.client_id.clone(), request_id: envelope.request_id.clone(),
        received_at: now.clone(),
        outcome: nola_protocol::CommandReceiptOutcome::Committed {
            completed_at: now, committed_revision: None, result,
        },
    }}))
}

#[no_mangle]
pub extern "C" fn nola_core_application_submit(
    application: *mut NolaCoreApplication,
    envelope_ptr: *const u8,
    envelope_len: usize,
) -> *mut c_char {
    if application.is_null() {
        return failure("application_unavailable", "Nola Application is unavailable");
    }
    if envelope_len > APPLICATION_SERVICE_MAX_FRAME_BYTES {
        return failure("invalid_envelope", "command exceeds the frame limit");
    }
    let Some(input) = utf8_input(envelope_ptr, envelope_len) else {
        return failure("invalid_utf8", "command envelope is not UTF-8");
    };
    let envelope: CommandEnvelope = match serde_json::from_str(&input) {
        Ok(value) => value,
        Err(error) => return failure("invalid_envelope", error),
    };
    if let Err(error) = envelope.validate() {
        return failure("invalid_envelope", error);
    }
    if envelope.command.command_type == "command_receipt" {
        if envelope.expected_revision.is_some() {
            return failure(
                "invalid_query_revision",
                "receipt lookup cannot carry a write revision",
            );
        }
        let request: nola_protocol::CommandReceiptLookupRequest =
            match serde_json::from_value(envelope.command.arguments.clone()) {
                Ok(value) => value,
                Err(error) => return failure("invalid_receipt_lookup", error),
            };
        if let Err(error) = request.validate() {
            return failure("invalid_receipt_lookup", error);
        }
        let runtime_ref = unsafe { &*application };
        let mut runtime = match runtime_ref.application.lock() {
            Ok(value) => value,
            Err(_) => {
                return failure(
                    "runtime_reopen_required",
                    "The canonical owner is unavailable; retain the original request for reconciliation",
                )
            }
        };
        return match runtime.command_receipt(&envelope.client_id, &request.request_id) {
            Ok(receipt) => query_result(
                &envelope,
                json!(receipt.map(|value| value.into_protocol_receipt())),
            ),
            Err(error) => {
                if error.requires_owner_reopen() {
                    let _ = runtime_ref.application.reopen_locked(&mut runtime);
                }
                encode(json!({"ok": false, "error": {"code": error.code, "message": error.message, "recoverable": error.recoverable}}))
            },
        };
    }
    let command_json = envelope.command.arguments.to_string();
    let mut arguments = envelope.command.arguments.clone();
    arguments.as_object_mut().expect("validated object").insert(
        "type".into(),
        Value::String(envelope.command.command_type.clone()),
    );
    let command: ApplicationCommand = match serde_json::from_value(arguments) {
        Ok(value) => value,
        Err(error) => return failure("invalid_command", error),
    };
    let model_edit = matches!(
        &command,
        ApplicationCommand::Model {
            command: ModelCommand::ConfigureParameters { .. }
        }
    );
    if model_edit && envelope.expected_revision.is_none() {
        return failure(
            "missing_revision",
            "model parameter editing requires a revision precondition",
        );
    }
    let runtime_ref = unsafe { &*application };
    if command.is_read_only_query() {
        if envelope.expected_revision.is_some() {
            return failure(
                "invalid_query_revision",
                "queries cannot carry a write revision",
            );
        }
        let command = match runtime_ref.application.try_execute_on_read_replica(command) {
            Ok(Ok(result)) => return query_result(&envelope, result),
            Ok(Err(error)) => return failure("application_rejected", error),
            Err(command) => command,
        };
        let mut runtime = match runtime_ref.application.lock() {
            Ok(value) => value,
            Err(_) => {
                return failure(
                    "runtime_reopen_required",
                    "The canonical owner is unavailable; retain the original request for reconciliation",
                )
            }
        };
        return match runtime.execute(command) {
            Ok(result) => query_result(&envelope, result),
            Err(error) => failure("application_rejected", error),
        };
    }
    let mut runtime = match runtime_ref.application.lock() {
        Ok(value) => value,
        Err(_) => {
            return failure(
                "runtime_reopen_required",
                "The canonical owner is unavailable; retain the original request for reconciliation",
            )
        }
    };
    match runtime.execute_received_command(
        ExecuteReceivedCommand {
            client_id: envelope.client_id,
            request_id: envelope.request_id,
            command_type: envelope.command.command_type,
            command_json,
            expected_revision: envelope.expected_revision,
            received_at: chrono::Utc::now().to_rfc3339(),
        },
        command,
    ) {
        Ok(receipt) => encode(json!({"ok": true, "result": receipt.into_protocol_receipt()})),
        Err(error) => {
            if error.requires_owner_reopen() {
                let _ = runtime_ref.application.reopen_locked(&mut runtime);
            }
            encode(json!({"ok": false, "error": {"code": error.code, "message": error.message, "recoverable": error.recoverable}}))
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{nola_core_application_free, nola_core_application_open};
    struct Handle(*mut NolaCoreApplication);
    impl Drop for Handle {
        fn drop(&mut self) {
            nola_core_application_free(self.0);
        }
    }
    fn open(path: &str) -> Handle {
        let handle = Handle(nola_core_application_open(path.as_ptr(), path.len()));
        assert!(!handle.0.is_null());
        handle
    }
    fn submit(handle: &Handle, value: &Value) -> Value {
        let input = value.to_string();
        let output = nola_core_application_submit(handle.0, input.as_ptr(), input.len());
        assert!(!output.is_null());
        let value =
            serde_json::from_slice(unsafe { std::ffi::CStr::from_ptr(output) }.to_bytes()).unwrap();
        nola_core::nola_core_free_c_string(output);
        value
    }
    fn edit(id: &str, revision: u64, temperature: f64) -> Value {
        json!({"protocol_version": nola_protocol::APPLICATION_SERVICE_PROTOCOL_VERSION,
        "request_id": id, "client_id": "mobile-fixture", "expected_revision": revision,
        "command": {"type": "model", "arguments": {"command": {
            "type": "configure_parameters", "input": {
                "id": "fixture", "display_name": "Fixture", "provider": "local",
                "model_name": "fixture", "runtime": "fixture", "source_json": "{}",
                "parameters_json": json!({"temperature": temperature}).to_string(),
                "selected": false, "enabled": true, "revision": 0,
                "created_at_ms": 1, "updated_at_ms": 2, "deleted_at_ms": null
            }
        }}}})
    }
    fn resource_command(resource: &str, id: &str, command: Value) -> Value {
        json!({"protocol_version":nola_protocol::APPLICATION_SERVICE_PROTOCOL_VERSION,
            "client_id":"resource-editors", "request_id":id,
            "command":{"type":resource,"arguments":{"command":command}}})
    }
    #[test]
    fn another_process_cannot_open_the_owned_database() {
        const KEY: &str = "NOLA_MOBILE_OWNER_CHILD_DATABASE";
        if let Ok(path) = std::env::var(KEY) {
            let handle = nola_core_application_open(path.as_ptr(), path.len());
            assert!(handle.is_null(), "second process must not acquire a writer");
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory
            .path()
            .join("owned.db")
            .to_string_lossy()
            .into_owned();
        let owner = open(&path);
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "submission::tests::another_process_cannot_open_the_owned_database",
                "--nocapture",
            ])
            .env(KEY, &path)
            .status()
            .unwrap();
        assert!(status.success());
        drop(owner);
        let _reopened = open(&path);
    }

    #[test]
    fn generic_submission_recovers_after_process_restart() {
        const KEY: &str = "NOLA_MOBILE_RECEIPT_CHILD_DATABASE";
        let request = json!({"protocol_version": nola_protocol::APPLICATION_SERVICE_PROTOCOL_VERSION,
        "client_id":"restart", "request_id":"original",
        "command":{"type":"create_conversation","arguments":{"input":{
            "id":"mobile-restart", "title":"Original", "created_at_ms":1, "updated_at_ms":1
        }}}});
        if let Ok(path) = std::env::var(KEY) {
            let handle = open(&path);
            let receipt = submit(&handle, &request);
            assert_eq!(receipt["result"]["status"], "committed", "{receipt}");
            std::fs::write(format!("{path}.receipt"), receipt.to_string()).unwrap();
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory
            .path()
            .join("restart.db")
            .to_string_lossy()
            .into_owned();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "submission::tests::generic_submission_recovers_after_process_restart",
                "--nocapture",
            ])
            .env(KEY, &path)
            .status()
            .unwrap();
        assert!(status.success());
        let original: Value =
            serde_json::from_str(&std::fs::read_to_string(format!("{path}.receipt")).unwrap())
                .unwrap();
        let handle = open(&path);
        assert_eq!(submit(&handle, &request), original);
        let lookup = json!({"protocol_version": nola_protocol::APPLICATION_SERVICE_PROTOCOL_VERSION,
            "client_id":"restart", "request_id":"lookup",
            "command":{"type":"command_receipt","arguments":{"request_id":"original"}}});
        assert_eq!(
            submit(&handle, &lookup)["result"]["result"],
            original["result"]
        );
        let query = json!({"protocol_version":nola_protocol::APPLICATION_SERVICE_PROTOCOL_VERSION,
            "client_id":"restart","request_id":"fresh",
            "command":{"type":"list_conversations","arguments":{"query":{"project_root":null,"non_empty":false,"limit":1024,"offset":0}}}});
        assert_eq!(
            submit(&handle, &query)["result"]["result"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let mut lookup_query = lookup;
        lookup_query["command"]["arguments"]["request_id"] = json!("fresh");
        assert!(submit(&handle, &lookup_query)["result"]["result"].is_null());
    }

    #[test]
    fn canonical_path_handles_share_runtime_and_raw_mutations_are_refused() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("new-parent").join("shared.db");
        let first = open(path.to_str().unwrap());
        let second = open(
            directory
                .path()
                .join("new-parent")
                .join(".")
                .join("shared.db")
                .to_str()
                .unwrap(),
        );
        assert!(std::sync::Arc::ptr_eq(
            unsafe { &(*first.0).application },
            unsafe { &(*second.0).application }
        ));
        let alias = directory.path().join("alias.db");
        std::os::unix::fs::symlink(&path, &alias).unwrap();
        let alias_handle = open(alias.to_str().unwrap());
        let hardlink = directory.path().join("hardlink.db");
        std::fs::hard_link(&path, &hardlink).unwrap();
        let hardlink_handle = open(hardlink.to_str().unwrap());
        assert!(std::sync::Arc::ptr_eq(
            unsafe { &(*first.0).application },
            unsafe { &(*alias_handle.0).application }
        ));
        assert!(std::sync::Arc::ptr_eq(
            unsafe { &(*first.0).application },
            unsafe { &(*hardlink_handle.0).application }
        ));
        let memory1 = open(":memory:");
        let memory2 = open(":memory:");
        assert!(!std::sync::Arc::ptr_eq(
            unsafe { &(*memory1.0).application },
            unsafe { &(*memory2.0).application }
        ));
        let raw = br#"{"type":"put_product_setting","key":"search","value_json":"true","updated_at_ms":1}"#;
        let output = crate::nola_core_application_execute(first.0, raw.as_ptr(), raw.len());
        let value: Value =
            serde_json::from_slice(unsafe { std::ffi::CStr::from_ptr(output) }.to_bytes()).unwrap();
        nola_core::nola_core_free_c_string(output);
        assert_eq!(value["error"]["code"], "receipt_required");
    }

    #[test]
    fn skill_editor_commands_cross_ffi_with_revision_and_replay_guards() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("skills.db").to_string_lossy().into_owned();
        let handle = open(&path);
        let mut input = json!({"id":"review","source":"user","version":"v1",
            "display_name":"Review","summary":"Review text","category":"custom",
            "instructions_ref":"skill://review/SKILL.md",
            "instructions_text":"Review the supplied text.",
            "instructions_digest":"3b7d3ff3c0cb68998d557a16e00de20ea6c57b4072cc9e1d6005fc0cf37e1d9f",
            "required_tools":[],"default_mode":null,"enabled":true,
            "created_at_ms":1,"updated_at_ms":2});
        let create = resource_command(
            "skill",
            "create",
            json!({"type":"create_user","input":input}),
        );
        let created = submit(&handle, &create);
        assert_eq!(created["result"]["status"], "committed", "{created}");
        let revision = created["result"]["result"]["revision"].as_i64().unwrap();
        input["version"] = json!("v2");
        let edited = submit(
            &handle,
            &resource_command(
                "skill",
                "edit",
                json!({
            "type":"configure_user","input":input,"expected_revision":revision}),
            ),
        );
        assert_eq!(edited["result"]["status"], "committed", "{edited}");
        let stale = resource_command(
            "skill",
            "stale",
            json!({"type":"set_enabled_checked",
            "id":"review","enabled":false,"expected_revision":revision,"updated_at_ms":3}),
        );
        let rejected = submit(&handle, &stale);
        assert_eq!(rejected["result"]["status"], "rejected", "{rejected}");
        assert!(rejected["result"]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("skill_revision_conflict"));
        drop(handle);
        let reopened = open(&path);
        assert_eq!(submit(&reopened, &create), created);
        assert_eq!(submit(&reopened, &stale), rejected);
        let disabled = submit(
            &reopened,
            &resource_command(
                "skill",
                "disable",
                json!({
            "type":"set_enabled_checked","id":"review","enabled":false,
            "expected_revision":edited["result"]["result"]["revision"],"updated_at_ms":4}),
            ),
        );
        assert_eq!(disabled["result"]["status"], "committed", "{disabled}");
        assert_eq!(disabled["result"]["result"]["record"]["enabled"], false);
        let deleted = submit(
            &reopened,
            &resource_command(
                "skill",
                "delete",
                json!({
            "type":"delete_user_checked","id":"review",
            "expected_revision":disabled["result"]["result"]["revision"],"deleted_at_ms":5}),
            ),
        );
        assert_eq!(deleted["result"]["status"], "committed", "{deleted}");
        let query = submit(
            &reopened,
            &resource_command("skill", "query", json!({"type":"get_editor","id":"review"})),
        );
        assert_eq!(query["result"]["status"], "committed");
    }
    #[test]
    fn mcp_batch_rollback_and_replay_cross_ffi() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("mcp.db").to_string_lossy().into_owned();
        let handle = open(&path);
        let create = json!({"type":"create","id":"server","display_name":"Server",
            "enabled":false,"configuration_json":"{}","created_at_ms":1});
        let batch = resource_command(
            "mcp",
            "failed-batch",
            json!({"type":"apply_changes","commands":[
            create, {"type":"delete_checked","id":"server","expected_revision":999,"deleted_at_ms":2}]}),
        );
        let rejected = submit(&handle, &batch);
        assert_eq!(rejected["result"]["status"], "rejected", "{rejected}");
        drop(handle);
        let reopened = open(&path);
        assert_eq!(submit(&reopened, &batch), rejected);
        // The first batch operation must have rolled back, including its identity.
        let saved = submit(&reopened, &resource_command("mcp", "create", create));
        assert_eq!(saved["result"]["status"], "committed", "{saved}");
    }
    #[test]
    fn memory_replacement_rejects_stale_snapshot_without_removing_new_memory() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("memory.db").to_string_lossy().into_owned();
        let handle = open(&path);
        let candidate = json!({"record": {
            "id":"concurrent", "owner":{"kind":"profile","profile_id":"local"}, "namespace":{"kind":"personal"}, "scope":"global",
            "kind":"preference", "content":"Keep concise answers", "confidence":0.8,
            "provenance":{"kind":"explicit_user","source":"test://editor"}
        }, "quality":null, "proposed_at_ms":1});
        let envelope = |id: &str, command: Value| {
            json!({
                "protocol_version":nola_protocol::APPLICATION_SERVICE_PROTOCOL_VERSION,
                "client_id":"memory-editors", "request_id":id,
                "command":{"type":"memory", "arguments":{"command":command}}
            })
        };
        let created = submit(
            &handle,
            &envelope(
                "create",
                json!({
                    "type":"commit", "intent":"explicit_user", "input":candidate
                }),
            ),
        );
        assert_eq!(created["result"]["status"], "committed");
        let stale = envelope(
            "stale",
            json!({"type":"replace", "expected":[],
            "candidates":[],"intent":"explicit_user","deleted_at_ms":2}),
        );
        let rejected = submit(&handle, &stale);
        assert_eq!(rejected["result"]["status"], "rejected");
        assert!(rejected["result"]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("memory_snapshot_conflict"));
        drop(handle);
        let reopened = open(&path);
        assert_eq!(submit(&reopened, &stale), rejected);
        let cleared = submit(
            &reopened,
            &envelope(
                "explicit-clear",
                json!({
                    "type":"replace","expected":[created["result"]["result"]],
                    "candidates":[],"intent":"explicit_user","deleted_at_ms":3
                }),
            ),
        );
        assert_eq!(cleared["result"]["status"], "committed", "{cleared}");
        assert_eq!(cleared["result"]["result"], json!([]));
    }
    #[test]
    fn uncertain_commit_reopens_the_shared_owner_before_exact_replay_from_another_live_handle() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("shared-owner.db").to_string_lossy().into_owned();
        let first = open(&path);
        let second = open(&path);
        let fixture = rusqlite::Connection::open(&path).unwrap();
        fixture.execute_batch("CREATE TABLE owner_commit_parent(id INTEGER PRIMARY KEY);
            CREATE TABLE owner_commit_child(parent_id INTEGER REFERENCES owner_commit_parent(id) DEFERRABLE INITIALLY DEFERRED);
            CREATE TRIGGER fail_owner_receipt_commit AFTER UPDATE OF status ON command_receipts
            WHEN NEW.status='committed' BEGIN INSERT INTO owner_commit_child(parent_id) VALUES (1); END;").unwrap();
        let request = json!({"protocol_version": nola_protocol::APPLICATION_SERVICE_PROTOCOL_VERSION,
            "client_id":"owner-reopen-fixture", "request_id":"exact-original-request",
            "command":{"type":"put_product_setting","arguments":{
                "key":"owner-reopen-result","value_json":"{\"once\":true}","updated_at_ms":1
            }}});
        let unknown = submit(&first, &request);
        assert_eq!(unknown["ok"], false, "{unknown}");
        assert!(nola_protocol::command_outcome_requires_owner_reopen(
            unknown["error"]["code"].as_str().unwrap()
        ), "{unknown}");
        fixture.execute_batch("DROP TRIGGER fail_owner_receipt_commit;").unwrap();
        // Both handles remain alive: free/open cannot explain this recovery.
        let committed = submit(&second, &request);
        assert_eq!(committed["result"]["status"], "committed", "{committed}");
        assert_eq!(submit(&first, &request), committed);
        let count: i64 = fixture.query_row(
            "SELECT COUNT(*) FROM command_receipts WHERE client_id='owner-reopen-fixture' AND request_id='exact-original-request' AND status='committed'",
            [], |row| row.get(0),
        ).unwrap();
        assert_eq!(count, 1);
        let value: String = fixture.query_row(
            "SELECT value_json FROM product_settings WHERE key='owner-reopen-result'", [], |row| row.get(0),
        ).unwrap();
        assert_eq!(value, "{\"once\":true}");
    }

    #[test]
    fn product_mutations_replay_after_reopen_and_queries_are_not_admitted() {
        let root = tempfile::tempdir().unwrap();
        let path = root
            .path()
            .join("product.db")
            .to_string_lossy()
            .into_owned();
        let handle = open(&path);
        let command = json!({"protocol_version": nola_protocol::APPLICATION_SERVICE_PROTOCOL_VERSION,
        "client_id":"mutation-fixture", "request_id":"setting-1",
        "command":{"type":"put_product_setting","arguments":{
            "key":"fixture","value_json":"true","updated_at_ms":1
        }}});
        let saved = submit(&handle, &command);
        assert_eq!(saved["result"]["status"], "committed", "{saved}");
        assert_eq!(submit(&handle, &command), saved);
        let mut changed = command.clone();
        changed["command"]["arguments"]["value_json"] = json!("false");
        assert_eq!(submit(&handle, &changed)["ok"], false);
        let query = json!({"protocol_version": nola_protocol::APPLICATION_SERVICE_PROTOCOL_VERSION,
            "client_id":"mutation-fixture", "request_id":"query-1",
            "command":{"type":"get_product_setting","arguments":{"key":"fixture"}}});
        assert_eq!(submit(&handle, &query)["result"]["status"], "committed");
        drop(handle);
        let reopened = open(&path);
        assert_eq!(submit(&reopened, &command), saved);
    }
    #[test]
    fn ffi_submission_replays_conflicts_and_commits_after_reopen() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("state.db").to_string_lossy().into_owned();
        let handle = open(&path);
        let command = edit("create", 0, 0.2);
        let created = submit(&handle, &command);
        assert_eq!(created["result"]["status"], "committed");
        let revision = created["result"]["committed_revision"].as_u64().unwrap();
        let stale = edit("stale", 0, 0.9);
        let conflict = submit(&handle, &stale);
        assert_eq!(conflict["result"]["status"], "conflict");
        assert_eq!(conflict["result"]["conflict"]["actual_revision"], revision);
        assert_eq!(submit(&handle, &command), created);
        assert_eq!(submit(&handle, &edit("create", 0, 0.8))["ok"], false);
        let mut invalid = edit("no-revision", 0, 0.8);
        invalid["expected_revision"] = Value::Null;
        assert_eq!(
            submit(&handle, &invalid)["error"]["code"],
            "missing_revision"
        );
        drop(handle);
        let reopened = open(&path);
        assert_eq!(submit(&reopened, &stale), conflict);
        assert_eq!(submit(&reopened, &command), created);
        let updated = submit(&reopened, &edit("update", revision, 0.3));
        assert_eq!(updated["result"]["status"], "committed");
        assert!(updated["result"]["committed_revision"].as_u64().unwrap() > revision);
    }
}
