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
    admit_with_host(&mut external,input.clone(),&context).unwrap(); // Seed a persisted pre-Main receipt; new Host admissions cannot fan out.
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
    let (mut store, input) = setup_at(&path);
    let context = host_context(&store, &input);
    admit_with_host(&mut store,input.clone(),&context).unwrap();
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
    let observer = NolaStore::open(&path).unwrap();
    assert_eq!(observer.load_conversation(&input.conversation_id).unwrap().unwrap().messages.len(),1);
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
    let (mut store, input) = setup_at(&path);
    let run=uuid::Uuid::new_v4().to_string();
    begin_real_child(&mut store,"parent",&run,"Main dispatch",config("parent"),vec![]).unwrap();
    let request=nola_application::expert_peer::PeerExchangeInput{source_context_id:"parent".into(),source_run_id:run,call_id:"saved-main-call".into(),
        target_participant_id:input.selected_participants[0].participant_id.clone(),kind:"request".into(),text:"Check one part".into(),in_reply_to:None};
    let preview=nola_application::expert_peer::preview(&mut store,&request,&authority()).unwrap();
    let a=authority();
    let context=HostExpertTemplateCaptureContext{actor:a.actor,request_host_id:a.request_host_id,builtins:vec![],selections:preview.participants.iter().map(|p|HostExpertTemplateSelection{
        context_conversation_id:p.participant.context_conversation_id.clone(),context_configuration_revision:p.context_configuration_revision,
        model_profile:nola_host_protocol::HostModelProfileReference{profile_id:p.model_declaration.profile_id.clone(),admitted_revision:p.model_declaration.profile_revision},
        source:HostExpertTemplateSelectionSource::NoTemplate,template_id:None,host_user_instructions:vec![]}).collect()};
    let operation=HostExpertTemplateOperation::AdmitPeer{input:request,context};
    let mut runtime = host_runtime(store, &path);
    let fixture = rusqlite::Connection::open(&path).unwrap();
    fixture.execute_batch("CREATE TRIGGER abort_expert_parent BEFORE INSERT ON product_settings
        WHEN NEW.key LIKE 'expert_peer_private/v1/exchange/%' BEGIN SELECT RAISE(ROLLBACK, 'actual transaction aborted'); END;").unwrap();
    let error = runtime
        .execute_host_expert_template_operation(
            "trusted-host",
            "savepoint-request",
            operation.clone(),
        )
        .unwrap_err();
    assert!(
        matches!(error, HostExpertOperationError::OutcomePending { ref request_id, reopen_required: true, .. } if request_id == "savepoint-request"),"{error:?}"
    );
    fixture
        .execute_batch("DROP TRIGGER abort_expert_parent;")
        .unwrap();
    drop(runtime);
    let observer = NolaStore::open(&path).unwrap();
    assert_eq!(observer.load_conversation(&input.conversation_id).unwrap().unwrap().messages.iter().filter(|message|message.role=="user").count(),1);
    drop(observer);
    let mut reopened = support::open_runtime(&path).unwrap();
    reopened
        .execute_host_expert_template_operation(
            "trusted-host",
            "savepoint-request",
            operation.clone(),
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

#[test]
fn private_host_parent_export_preserves_hidden_command_boundary_and_canonical_message() {
    use nola_application::ApplicationCommand;
    let dir=tempfile::tempdir().unwrap();
    let path=dir.path().join("parent-export.db");
    let (mut store,input)=setup_at(&path);
    let context=host_context(&store,&input);
    admit_with_host(&mut store,input.clone(),&context).unwrap();
    let mut runtime=host_runtime(store,&path);
    runtime.execute_host_expert_template_operation("trusted-host","admit-for-export",admit_operation(&input,&context)).unwrap();
    let epoch:String=serde_json::from_value(runtime.execute(ApplicationCommand::ReadUiAuthority).unwrap()).unwrap();
    let op=HostExpertTemplateOperation::ExportRemoteParentAdmission {
        conversation_id:input.conversation_id.clone(),message_id:input.user_message_id.clone(),
        source_store_epoch:epoch.clone(),authority:authority(),
    };
    assert!(!op.mutates_state());
    // Production Service decodes the private operation, never the forbidden
    // generic Application command. Preserve both sides of that wire boundary.
    let wire=serde_json::to_value(&op).unwrap();
    let decoded:HostExpertTemplateOperation=serde_json::from_value(wire.clone()).unwrap();
    let mut forged=wire;
    forged["source_host_id"]=json!("renderer-selected-host");
    assert!(serde_json::from_value::<HostExpertTemplateOperation>(forged).is_err());
    assert!(serde_json::from_value::<ApplicationCommand>(json!({
        "type":"export_remote_expert_parent_admission","conversation_id":input.conversation_id,
        "message_id":input.user_message_id,"source_host_id":"capture-host","source_store_epoch":epoch,
    })).is_err());
    let exported=runtime.execute_host_expert_template_operation("trusted-host","parent-export",decoded).unwrap();
    assert_eq!(exported["source_host_id"],"capture-host");
    assert_eq!(exported["source_store_epoch"],epoch);
    assert_eq!(exported["conversation_id"],input.conversation_id);
    assert_eq!(exported["input"],json!(input));
    let mut wrong_epoch=op.clone();
    if let HostExpertTemplateOperation::ExportRemoteParentAdmission{source_store_epoch,..}=&mut wrong_epoch {*source_store_epoch="retired".into();}
    assert!(runtime.execute_host_expert_template_operation("trusted-host","wrong-epoch",wrong_epoch).is_err());
    let db=rusqlite::Connection::open(&path).unwrap();
    assert_eq!(db.query_row("SELECT COUNT(*) FROM runs",[],|row|row.get::<_,i64>(0)).unwrap(),0);
}
