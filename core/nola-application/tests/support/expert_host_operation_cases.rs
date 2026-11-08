// SPDX-License-Identifier: Apache-2.0
// Included by expert_message_transactions; drives the production Runtime API.
use nola_application::host_expert_template_operation::{
    HostExpertOperationError, HostExpertTemplateOperation,
};

fn host_runtime(store: NolaStore, path: &std::path::Path) -> support::ApplicationRuntime {
    nola_application::ApplicationRuntime::from_store(
        store,
        Box::new(
            nola_store::content_upload_port::LazyContentUploadStore::for_application(
                path,
                nola_application::MAXIMUM_CONTENT_UPLOAD_BYTES,
            ),
        ),
    )
}
fn admit_operation(
    input: &AdmitExpertConversationMessage,
    context: &HostExpertTemplateCaptureContext,
) -> HostExpertTemplateOperation {
    HostExpertTemplateOperation::Admit {
        input: input.clone(),
        context: context.clone(),
    }
}

#[test]
fn private_runtime_rejection_requires_new_transport_request_not_new_business_intent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host-operation.db");
    let (mut store, input) = setup_at(&path);
    let id = uuid::Uuid::new_v4().to_string();
    select_managed(&mut store, &input, 0, &id);
    managed_catalog(
        &mut store,
        &id,
        Some("not-the-capture-user"),
        json!([]),
        "Original body",
    );
    let context = host_context(&store, &input);
    let mut runtime = host_runtime(store, &path);
    let first = runtime.execute_host_expert_template_operation(
        "trusted-host",
        "rejected-request",
        admit_operation(&input, &context),
    );
    assert!(matches!(
        first,
        Err(HostExpertOperationError::Rejected { .. })
    ));
    let mut external = NolaStore::open(&path).unwrap();
    assert_no_capture_parent(&mut external, &input);
    managed_catalog(
        &mut external,
        &id,
        Some("capture-user"),
        json!([]),
        "Authorized later",
    );
    assert!(
        matches!(
            runtime.execute_host_expert_template_operation(
                "trusted-host",
                "rejected-request",
                admit_operation(&input, &context)
            ),
            Err(HostExpertOperationError::Rejected { .. })
        ),
        "A terminal rejected receipt is not re-evaluated"
    );
    let committed = runtime
        .execute_host_expert_template_operation(
            "trusted-host",
            "new-request",
            admit_operation(&input, &context),
        )
        .unwrap();
    let manifest: ExpertMessageManifest = serde_json::from_value(committed.clone()).unwrap();
    assert_eq!(manifest.input, input);
    assert_eq!(
        manifest.entries[0]
            .expected_prompt_definition
            .as_ref()
            .unwrap()
            .content,
        "Authorized later"
    );
    let mut unavailable_provider = context;
    unavailable_provider.selections.clear();
    unavailable_provider.builtins.clear();
    assert_eq!(runtime.execute_host_expert_template_operation("trusted-host", "new-request", admit_operation(&input, &unavailable_provider)).unwrap(), committed,
        "Committed transport identity is bound to business intent/authority, not current provider context");
    assert_eq!(
        external
            .load_conversation(&input.conversation_id)
            .unwrap()
            .unwrap()
            .messages
            .len(),
        1
    );
}

#[test]
fn private_runtime_commit_uncertainty_is_pending_and_requires_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host-unknown-commit.db");
    let (store, input) = setup_at(&path);
    let context = host_context(&store, &input);
    let mut runtime = host_runtime(store, &path);
    let fixture = rusqlite::Connection::open(&path).unwrap();
    fixture.execute_batch("CREATE TABLE host_commit_parent(id INTEGER PRIMARY KEY);
        CREATE TABLE host_commit_child(parent_id INTEGER REFERENCES host_commit_parent(id) DEFERRABLE INITIALLY DEFERRED);
        CREATE TRIGGER fail_host_receipt_commit AFTER UPDATE OF status ON command_receipts
        WHEN NEW.status='committed' BEGIN INSERT INTO host_commit_child(parent_id) VALUES (1); END;").unwrap();
    let error = runtime
        .execute_host_expert_template_operation(
            "trusted-host",
            "uncertain-request",
            admit_operation(&input, &context),
        )
        .unwrap_err();
    assert!(
        matches!(error, HostExpertOperationError::OutcomePending { ref request_id, reopen_required: true, .. } if request_id == "uncertain-request")
    );
    assert!(matches!(
        runtime.execute_host_expert_template_operation(
            "trusted-host",
            "uncertain-request",
            admit_operation(&input, &context)
        ),
        Err(HostExpertOperationError::OutcomePending {
            reopen_required: true,
            ..
        })
    ));
    fixture
        .execute_batch("DROP TRIGGER fail_host_receipt_commit;")
        .unwrap();
    drop(runtime);
    let mut observer = NolaStore::open(&path).unwrap();
    assert_no_capture_parent(&mut observer, &input);
    drop(observer);
    let mut reopened = support::open_runtime(&path).unwrap();
    let result = reopened
        .execute_host_expert_template_operation(
            "trusted-host",
            "uncertain-request",
            admit_operation(&input, &context),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_value::<ExpertMessageManifest>(result)
            .unwrap()
            .input,
        input
    );
}

#[test]
fn private_runtime_lost_savepoint_is_pending_not_business_rejection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host-lost-savepoint.db");
    let (store, input) = setup_at(&path);
    let context = host_context(&store, &input);
    let mut runtime = host_runtime(store, &path);
    let fixture = rusqlite::Connection::open(&path).unwrap();
    fixture.execute_batch("CREATE TRIGGER abort_expert_parent BEFORE INSERT ON messages
        WHEN NEW.conversation_id='parent' BEGIN SELECT RAISE(ROLLBACK, 'actual transaction aborted'); END;").unwrap();
    let error = runtime
        .execute_host_expert_template_operation(
            "trusted-host",
            "savepoint-request",
            admit_operation(&input, &context),
        )
        .unwrap_err();
    assert!(
        matches!(error, HostExpertOperationError::OutcomePending { ref request_id, reopen_required: true, .. } if request_id == "savepoint-request")
    );
    fixture
        .execute_batch("DROP TRIGGER abort_expert_parent;")
        .unwrap();
    drop(runtime);
    let mut observer = NolaStore::open(&path).unwrap();
    assert_no_capture_parent(&mut observer, &input);
    drop(observer);
    let mut reopened = support::open_runtime(&path).unwrap();
    reopened
        .execute_host_expert_template_operation(
            "trusted-host",
            "savepoint-request",
            admit_operation(&input, &context),
        )
        .unwrap();
}

#[test]
fn renderer_application_json_cannot_spoof_private_expert_capture_authority() {
    let (store, input) = setup();
    let context = host_context(&store, &input);
    for command in [
        nola_application::ApplicationCommand::AdmitExpertConversationMessage {
            input,
            context,
            captured_at_ms: 1000,
        },
        nola_application::ApplicationCommand::LoadExpertMessageManifest {
            conversation_id: "parent".into(),
            message_id: "first-parent-user".into(),
            authority: authority(),
        },
        nola_application::ApplicationCommand::LoadExpertChildInstructions {
            conversation_id: "child".into(),
            expected_run_id: "run".into(),
            authority: authority(),
        },
    ] {
        let value = serde_json::to_value(command).unwrap();
        assert!(serde_json::from_value::<nola_application::ApplicationCommand>(value).is_err());
    }
}
