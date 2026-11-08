// SPDX-License-Identifier: Apache-2.0
// Included by expert_message_transactions: every case uses the actual Store.
fn managed_catalog(
    store: &mut NolaStore,
    id: &str,
    owner: Option<&str>,
    grants: serde_json::Value,
    body: &str,
) {
    let mut template = json!({"id":id,"name":"Private rules","content":body,
        "description":"Current private definition", "isBuiltIn":false,
        "createdAt":0, "updatedAt":0, "accessRevision":1,"accessGrants":grants});
    if let Some(owner) = owner {
        template["accessOwnerID"] = json!(owner);
    }
    store
        .put_product_setting(
            "prompt.templates",
            &json!({"customTemplates":[template],"selectedTemplateID":null}).to_string(),
            100,
        )
        .unwrap();
}
fn select_managed(
    store: &mut NolaStore,
    input: &AdmitExpertConversationMessage,
    peer: usize,
    id: &str,
) {
    let context = &input.selected_participants[peer].context_conversation_id;
    let current = store.load_conversation(context).unwrap().unwrap();
    let result = store
        .patch_conversation_configuration(
            &nola_store_port::conversation_profile::PatchConversationConfigurationInput {
                conversation_id: context.clone(),
                expected_revision: current.configuration_revision,
                patch_json: json!({"selectedPromptTemplateID":id})
                    .to_string()
                    .parse()
                    .unwrap(),
                updated_at_ms: 100,
            },
        )
        .unwrap();
    assert!(matches!(
        result,
        nola_domain::conversation_configuration::ConfigurationCommitResult::Committed { .. }
    ));
}
fn read_grant(id: &str) -> serde_json::Value {
    json!({"grant_id":"current-read", "resource_kind":"prompt_template", "resource_id":uuid::Uuid::parse_str(id).unwrap().to_string(),
        "principal":{"kind":"local_user","id":"capture-user"}, "permissions":["read_content"], "resource_revision":1,
        "host_id":"capture-host", "expires_at_epoch_ms":2000})
}
fn assert_no_capture_parent(store: &mut NolaStore, input: &AdmitExpertConversationMessage) {
    assert!(store
        .load_conversation(&input.conversation_id)
        .unwrap()
        .unwrap()
        .messages
        .is_empty());
    assert!(store
        .expert_message_manifest(&input.conversation_id, &input.user_message_id)
        .unwrap()
        .is_none());
    assert!(store
        .expert_template_capture_fact(&input.conversation_id, &input.submission_id)
        .unwrap()
        .is_none());
    assert!(
        rooms::submissions(&store.product_setting_repository(), &input.room_id, 50, 0)
            .unwrap()
            .is_empty()
    );
    for peer in &input.selected_participants {
        assert!(store
            .load_conversation(&peer.context_conversation_id)
            .unwrap()
            .unwrap()
            .messages
            .is_empty());
    }
}
fn inline_user_instruction() -> ExpertPromptDefinition {
    use sha2::{Digest, Sha256};
    let content = "Preserve my precise, private model user instructions.";
    ExpertPromptDefinition {
        component_id: "native-user-definition".into(),
        content: content.into(),
        content_sha256: format!("{:x}", Sha256::digest(content.as_bytes())),
        definition_revision: None,
        platform: json!(null),
    }
}
fn components(
    definitions: &[ExpertPromptDefinition],
) -> Vec<nola_agent_engine::context_engine::ContextComponent> {
    definitions
        .iter()
        .map(|definition| {
            serde_json::from_value(json!({"kind":"prompt_template",
        "id":definition.component_id,"content":definition.content}))
            .unwrap()
        })
        .collect()
}
fn finish_actual_child(store: &mut NolaStore, accepted: &AcceptedAssistantTurn, context: &str) {
    use nola_application::{
        claim_committed_host_requests, reduce_and_accept_host_response, ClaimCommittedHostRequests,
        ReduceAndAcceptHostResponse,
    };
    let requests = claim_committed_host_requests(
        store,
        ClaimCommittedHostRequests {
            run_id: accepted.begin.run_id.clone(),
            worker_generation: "captured-worker".into(),
            lease_owner: "captured-owner".into(),
            claimed_at_ms: 11,
            lease_expires_at_ms: 1000,
            limit: 64,
        },
    )
    .unwrap();
    let request = requests
        .requests
        .iter()
        .find(|r| matches!(r.payload, HostRequest::ModelStream { .. }))
        .unwrap();
    for (index, payload) in [
        HostResponse::ExecutionStarted,
        HostResponse::ModelAttemptStarted {
            provider: "fixture".into(),
            model: "original-model".into(),
            reason: "initial".into(),
        },
        HostResponse::ModelCompleted {
            items: vec![serde_json::to_value(AgentItem::assistant_message(
                ItemId::from_string(format!("{}:answer", accepted.begin.run_id)),
                "Verified answer",
            ))
            .unwrap()],
            tool_calls: vec![],
            usage: json!({}),
        },
    ]
    .into_iter()
    .enumerate()
    {
        reduce_and_accept_host_response(
            store,
            ReduceAndAcceptHostResponse {
                conversation_id: context.into(),
                run_id: accepted.begin.run_id.clone(),
                invocation_config_digest: accepted.invocation_config_digest.clone(),
                received_at_ms: 12 + index as i64,
                response: HostResponseEnvelope {
                    request_id: request.request_id.clone(),
                    attempt_id: request.attempt_id.clone(),
                    delegated_invocation_id: request.delegated_invocation_id.clone(),
                    effect_id: request.effect_id.clone(),
                    host_id: request.host_id.clone(),
                    lease_id: request.lease_id.clone(),
                    session_id: request.session_id.clone(),
                    turn_id: request.turn_id.clone(),
                    catalog_revision: request.catalog_revision,
                    availability_epoch: request.availability_epoch,
                    delivery: request.delivery.clone(),
                    sequence: index as u64 + 1,
                    payload,
                },
            },
        )
        .unwrap();
    }
    assert!(store.active_run(context).unwrap().is_none());
}
fn complete_captured_source(store: &mut NolaStore, manifest: &ExpertMessageManifest) {
    for entry in &manifest.entries {
        let definitions = expert_message::load_child_instructions_authorized(
            store,
            &entry.participant.context_conversation_id,
            &entry.child_submission_id,
            &authority(),
        )
        .unwrap()
        .unwrap();
        let accepted = begin_real_child(
            store,
            &entry.participant.context_conversation_id,
            &entry.child_submission_id,
            &manifest.input.text,
            config(&entry.participant.context_conversation_id),
            components(&definitions),
        )
        .unwrap();
        finish_actual_child(store, &accepted, &entry.participant.context_conversation_id);
    }
}
fn edit_intent(store: &NolaStore, source: &AdmitExpertConversationMessage) -> PrepareEditedResend {
    let conversation = store
        .load_conversation(&source.conversation_id)
        .unwrap()
        .unwrap();
    let projection =
        nola_application::load_conversation_projection_v1(store, &source.conversation_id).unwrap();
    PrepareEditedResend {
        operation_id: uuid::Uuid::new_v4().to_string(),
        source_conversation_id: source.conversation_id.clone(),
        source_message_id: source.user_message_id.clone(),
        expected_source_revision: conversation.revision,
        expected_source_message_revision: projection
            .messages
            .iter()
            .find(|m| m.message_id.0 == source.user_message_id)
            .unwrap()
            .revision,
        edited_text: "Edited, with the original complete definitions".into(),
        target_conversation_id: uuid::Uuid::new_v4().to_string(),
        title: "Edited".into(),
        created_at_ms: 2000,
    }
}

#[test]
fn template_capture_rechecks_current_grants_and_rolls_back_the_whole_expert_set() {
    let variants = [
        "missing", "metadata", "expired", "revision", "digest", "host", "unowned",
    ];
    for variant in variants {
        let (mut store, input) = setup();
        let id = uuid::Uuid::new_v4().to_string();
        select_managed(&mut store, &input, 1, &id);
        let mut grant = read_grant(&id);
        match variant {
            "metadata" => grant["permissions"] = json!(["read_metadata"]),
            "expired" => grant["expires_at_epoch_ms"] = json!(999),
            "revision" => grant["resource_revision"] = json!(2),
            "digest" => grant["content_digest"] = json!(format!("sha256:{}", "f".repeat(64))),
            "host" => grant["host_id"] = json!("other-host"),
            _ => {}
        }
        managed_catalog(
            &mut store,
            &id,
            if variant == "unowned" {
                None
            } else {
                Some("another-principal")
            },
            if variant == "missing" {
                json!([])
            } else {
                json!([grant])
            },
            "Secret definition",
        );
        let before = store
            .product_setting("prompt.templates")
            .unwrap()
            .unwrap()
            .value_json;
        assert!(admit(&mut store, input.clone()).is_err(), "{variant}");
        assert_no_capture_parent(&mut store, &input);
        assert_eq!(
            store
                .product_setting("prompt.templates")
                .unwrap()
                .unwrap()
                .value_json,
            before,
            "No record is adopted or repaired"
        );
    }
    let (mut store, input) = setup();
    let id = uuid::Uuid::new_v4().to_string();
    select_managed(&mut store, &input, 0, &id);
    managed_catalog(
        &mut store,
        &id,
        Some("another-principal"),
        json!([read_grant(&id)]),
        "Allowed in the picker",
    );
    let selected = host_context(&store, &input); // selection grants no content authority
    managed_catalog(
        &mut store,
        &id,
        Some("another-principal"),
        json!([]),
        "Allowed in the picker",
    );
    assert!(admit_with_host(&mut store, input.clone(), &selected)
        .unwrap_err()
        .contains("read_content_denied"));
    assert_no_capture_parent(&mut store, &input);
}

#[test]
fn committed_capture_replay_uses_original_bytes_and_all_body_reads_require_the_capture_actor() {
    let (mut store, input) = setup();
    let id = uuid::Uuid::new_v4().to_string();
    select_managed(&mut store, &input, 0, &id);
    managed_catalog(
        &mut store,
        &id,
        Some("capture-user"),
        json!([]),
        "Original captured rules",
    );
    let host = host_context(&store, &input);
    let manifest = admit_with_host(&mut store, input.clone(), &host).unwrap();
    let original_fact = store
        .expert_template_capture_fact(&input.conversation_id, &input.submission_id)
        .unwrap()
        .unwrap();
    managed_catalog(
        &mut store,
        &id,
        Some("another-principal"),
        json!([]),
        "Changed forbidden rules",
    );
    assert_eq!(
        admit_with_host(&mut store, input.clone(), &host).unwrap(),
        manifest
    );
    let mut unavailable_current_provider = host.clone();
    unavailable_current_provider.selections.clear();
    unavailable_current_provider.builtins.clear();
    assert_eq!(
        admit_with_host(&mut store, input.clone(), &unavailable_current_provider).unwrap(),
        manifest,
        "An already captured intent needs its original authority, not a new provider selection"
    );
    assert_eq!(
        store
            .expert_template_capture_fact(&input.conversation_id, &input.submission_id)
            .unwrap()
            .unwrap(),
        original_fact
    );
    assert_eq!(
        store
            .load_conversation(&input.conversation_id)
            .unwrap()
            .unwrap()
            .messages
            .len(),
        1
    );
    let mut other = authority();
    other.actor.id = "other-reader".into();
    let mut foreign_host = authority();
    foreign_host.request_host_id = Some("another-host".into());
    assert!(expert_message::load_manifest_authorized(
        &mut store,
        &input.conversation_id,
        &input.user_message_id,
        &foreign_host
    )
    .is_err());
    assert!(expert_message::load_manifest_authorized(
        &mut store,
        &input.conversation_id,
        &input.user_message_id,
        &other
    )
    .is_err());
    let peer = &manifest.entries[0];
    assert!(expert_message::load_child_instructions_authorized(
        &mut store,
        &peer.participant.context_conversation_id,
        &peer.child_submission_id,
        &other
    )
    .is_err());
    assert!(expert_message::load_child_instructions_authorized(
        &mut store,
        &peer.participant.context_conversation_id,
        "another-run",
        &authority()
    )
    .is_err());
    let definitions = expert_message::load_child_instructions_authorized(
        &mut store,
        &peer.participant.context_conversation_id,
        &peer.child_submission_id,
        &authority(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(definitions[0].content, "Original captured rules");
}

#[test]
fn no_managed_template_preserves_registered_native_user_definitions_and_exact_begin_set() {
    let (mut store, input) = setup();
    let mut host = host_context(&store, &input);
    let user = inline_user_instruction();
    host.selections[0].host_user_instructions = vec![user.clone()];
    let manifest = admit_with_host(&mut store, input.clone(), &host).unwrap();
    let peer = &manifest.entries[0];
    assert!(peer.expected_prompt_definition.is_none());
    let context = &peer.participant.context_conversation_id;
    let run = &peer.child_submission_id;
    assert!(
        begin_real_child(
            &mut store,
            context,
            run,
            &input.text,
            config(context),
            vec![]
        )
        .is_err(),
        "Cannot silently drop native user configuration"
    );
    assert!(store.active_run(context).unwrap().is_none());
    assert!(store
        .load_conversation(context)
        .unwrap()
        .unwrap()
        .messages
        .is_empty());
    let mut foreign = user.clone();
    foreign.component_id = "another-explicit-definition".into();
    assert!(
        begin_real_child(
            &mut store,
            context,
            run,
            &input.text,
            config(context),
            components(&[user.clone(), foreign])
        )
        .is_err(),
        "No prefix or extra typed definition exemption"
    );
    let actual = begin_real_child(
        &mut store,
        context,
        run,
        &input.text,
        config(context),
        components(&[user]),
    )
    .unwrap();
    assert_eq!(actual.begin.run_id, *run);
    let observed = store
        .expert_message_manifest(&input.conversation_id, &input.user_message_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        observed.entries[0]
            .instruction_witness
            .as_ref()
            .unwrap()
            .prompt_definitions
            .len(),
        1
    );
}

#[test]
fn selected_definition_and_registered_user_ids_cannot_overlap_or_impersonate_builtins() {
    let (mut store, input) = setup();
    let id = uuid::Uuid::new_v4().to_string();
    select_managed(&mut store, &input, 0, &id);
    managed_catalog(
        &mut store,
        &id,
        Some("capture-user"),
        json!([]),
        "Managed definition",
    );
    let mut host = host_context(&store, &input);
    let mut user = inline_user_instruction();
    user.component_id = format!("prompt_template:{id}");
    host.selections[0].host_user_instructions = vec![user];
    assert!(admit_with_host(&mut store, input.clone(), &host)
        .unwrap_err()
        .contains("overlapping"));
    assert_no_capture_parent(&mut store, &input);
    host.selections[0].host_user_instructions.clear();
    host.builtins.push(
        nola_application::expert_template_capture::HostExpertBuiltinDefinition {
            provider_id: "native-code".into(),
            template_id: id,
            content: "Claimed builtin".into(),
            platform: json!(null),
        },
    );
    assert!(admit_with_host(&mut store, input.clone(), &host)
        .unwrap_err()
        .contains("ambiguous"));
    assert_no_capture_parent(&mut store, &input);
}

#[test]
fn missing_custom_id_is_not_a_builtin_and_host_default_selection_is_frozen_explicitly() {
    let (mut store, input) = setup();
    let id = uuid::Uuid::new_v4().to_string();
    select_managed(&mut store, &input, 0, &id);
    assert!(admit(&mut store, input.clone())
        .unwrap_err()
        .contains("definition_missing"));
    assert_no_capture_parent(&mut store, &input);
    let (mut store, input) = setup();
    let id = uuid::Uuid::new_v4().to_string();
    managed_catalog(
        &mut store,
        &id,
        Some("capture-user"),
        json!([]),
        "Actual global default",
    );
    let mut host = host_context(&store, &input);
    host.selections[0].source = HostExpertTemplateSelectionSource::HostDefault;
    host.selections[0].template_id = Some(id.clone());
    let manifest = admit_with_host(&mut store, input.clone(), &host).unwrap();
    assert_eq!(
        manifest.entries[0].defaults.conversation_configuration()["selectedPromptTemplateID"],
        json!(id)
    );
    assert_eq!(
        manifest.entries[0]
            .expected_prompt_definition
            .as_ref()
            .unwrap()
            .content,
        "Actual global default"
    );
}

#[test]
fn edited_source_reuses_authorized_definition_bytes_but_no_old_resource_grant_and_checks_every_read(
) {
    let (mut store, input) = setup();
    let id = uuid::Uuid::new_v4().to_string();
    select_managed(&mut store, &input, 0, &id);
    managed_catalog(
        &mut store,
        &id,
        Some("another-principal"),
        json!([read_grant(&id)]),
        "Original authorized definition",
    );
    let mut host = host_context(&store, &input);
    host.selections[0].host_user_instructions = vec![inline_user_instruction()];
    let manifest = admit_with_host(&mut store, input.clone(), &host).unwrap();
    complete_captured_source(&mut store, &manifest);
    managed_catalog(
        &mut store,
        &id,
        Some("another-principal"),
        json!([]),
        "Changed and revoked",
    );
    let intent = edit_intent(&store, &input);
    let mut other = authority();
    other.actor.id = "foreign-principal".into();
    assert!(
        fixture_command(&mut store, |store| expert_message::prepare_resend(
            store,
            intent.clone(),
            &other,
            3000
        ))
        .is_err()
    );
    assert!(store
        .load_conversation(&intent.target_conversation_id)
        .unwrap()
        .is_none());
    let prepared = prepare(&mut store, intent.clone()).unwrap();
    assert_eq!(
        prepared.entries[0]
            .original
            .expected_prompt_definition
            .as_ref()
            .unwrap()
            .content,
        "Original authorized definition"
    );
    assert_eq!(prepare(&mut store, intent.clone()).unwrap(), prepared);
    assert!(expert_message::load_prepared_authorized(
        &mut store,
        &intent.target_conversation_id,
        &other
    )
    .is_err());
    assert!(expert_message::load_manifest_authorized(
        &mut store,
        &intent.target_conversation_id,
        &prepared.parent_message_id,
        &other
    )
    .is_err());
    let child = &prepared.entries[0];
    let definitions = expert_message::load_child_instructions_authorized(
        &mut store,
        &child.participant.context_conversation_id,
        &child.prepared.run_id,
        &authority(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(definitions.len(), 2);
    assert!(definitions
        .iter()
        .any(|definition| definition.content == inline_user_instruction().content));
    assert!(definitions
        .iter()
        .any(|definition| definition.content == "Original authorized definition"));
    assert!(expert_message::load_child_instructions_authorized(
        &mut store,
        &child.participant.context_conversation_id,
        &child.prepared.run_id,
        &other
    )
    .is_err());
    let fact: serde_json::Value = serde_json::from_str(
        &store
            .expert_template_capture_fact(&intent.target_conversation_id, &intent.operation_id)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(fact["entries"][0]["origin"]["kind"], "retained");
    assert!(
        fact["entries"][0]["origin"].get("decision").is_none(),
        "Retained snapshot is not a freshly granted current read"
    );
    let context = store
        .load_conversation(&child.participant.context_conversation_id)
        .unwrap()
        .unwrap();
    assert!(context.messages.is_empty());
    assert!(store.active_run(&context.id).unwrap().is_none());
    let settings: serde_json::Value =
        serde_json::from_str(context.settings_json.as_deref().unwrap()).unwrap();
    assert!(settings.get("allowed_dirs").is_none());
    assert!(settings.get("permission_rules").is_none());
}

#[test]
fn trusted_builtin_definition_is_exact_at_begin_and_capture_facts_are_immutable() {
    let (mut store, input) = setup();
    let id = uuid::Uuid::new_v4().to_string();
    select_managed(&mut store, &input, 0, &id);
    let mut host = host_context(&store, &input);
    host.builtins.push(
        nola_application::expert_template_capture::HostExpertBuiltinDefinition {
            provider_id: "native-test-provider".into(),
            template_id: id,
            content: "The actual trusted builtin body".into(),
            platform: json!(null),
        },
    );
    let manifest = admit_with_host(&mut store, input.clone(), &host).unwrap();
    let original = store
        .expert_template_capture_fact(&input.conversation_id, &input.submission_id)
        .unwrap()
        .unwrap();
    let mut changed: serde_json::Value = serde_json::from_str(&original).unwrap();
    changed["entries"][0]["definition_sha256"] = json!("0".repeat(64));
    assert!(fixture_command(&mut store, |store| {
        store
            .put_expert_template_capture_fact(
                &input.conversation_id,
                &input.submission_id,
                &changed.to_string(),
                1000,
            )
            .map_err(|e| e.to_string())
    })
    .is_err());
    assert_eq!(
        store
            .expert_template_capture_fact(&input.conversation_id, &input.submission_id)
            .unwrap()
            .unwrap(),
        original
    );
    let entry = &manifest.entries[0];
    let context = &entry.participant.context_conversation_id;
    let mut wrong = entry.expected_prompt_definition.clone().unwrap();
    wrong.content = "Renderer replacement body".into();
    assert!(begin_real_child(
        &mut store,
        context,
        &entry.child_submission_id,
        &input.text,
        config(context),
        components(&[wrong])
    )
    .is_err());
    assert!(store.active_run(context).unwrap().is_none());
    let actual = expert_message::load_child_instructions_authorized(
        &mut store,
        context,
        &entry.child_submission_id,
        &authority(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(actual[0].content, "The actual trusted builtin body");
    begin_real_child(
        &mut store,
        context,
        &entry.child_submission_id,
        &input.text,
        config(context),
        components(&actual),
    )
    .unwrap();
}

#[test]
fn explicit_selection_cannot_be_replaced_by_a_different_host_default() {
    let (mut store, input) = setup();
    let id = uuid::Uuid::new_v4().to_string();
    select_managed(&mut store, &input, 0, &id);
    let mut host = host_context(&store, &input);
    host.selections[0].source = HostExpertTemplateSelectionSource::HostDefault;
    host.selections[0].template_id = Some(uuid::Uuid::new_v4().to_string());
    assert!(admit_with_host(&mut store, input.clone(), &host).is_err());
    assert_no_capture_parent(&mut store, &input);
}
