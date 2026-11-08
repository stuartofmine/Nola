// SPDX-License-Identifier: Apache-2.0
// Actual Store preview/model-CAS coverage; no attempted admission rollback as preview.
#[test]
fn capture_preview_returns_only_selected_models_instruction_presentation_at_the_pinned_revision() {
    let (mut store, input) = setup();
    let old_host = host_context(&store, &input);
    let peer = &input.selected_participants[0];
    let context = store.load_conversation(&peer.context_conversation_id).unwrap().unwrap();
    let presentation = json!({"customSystemPrompt":"Keep this exact user instruction", "responseStyle":"formal"});
    let patched = store.patch_conversation_configuration(
        &nola_store_port::conversation_profile::PatchConversationConfigurationInput {
            conversation_id: context.id.clone(),
            expected_revision: context.configuration_revision,
            patch_json: json!({"clientModelPresentation":{
                "configured:model":presentation.clone(),
                "configured:other":{"customSystemPrompt":"Not selected"}
            }}).to_string().parse().unwrap(),
            updated_at_ms: 100,
        },
    ).unwrap();
    assert!(patched.is_committed());
    let preview = expert_message::preview_capture(&mut store, &input, &authority()).unwrap();
    let entry = preview.participants.iter().find(|entry| entry.participant == *peer).unwrap();
    assert_eq!(entry.client_model_presentation, presentation);
    assert!(entry.context_configuration_revision > context.configuration_revision);
    assert!(entry.defaults.conversation_configuration().get("clientModelPresentation").is_none());
    assert!(admit_with_host(&mut store, input.clone(), &old_host).is_err());
    assert_no_capture_parent(&mut store, &input);
}

#[test]
fn capture_preview_is_read_only_and_matches_the_actual_admission_model_resolution() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("capture-preview.db");
    let (mut store, input) = setup_at(&path);
    let observer = rusqlite::Connection::open(&path).unwrap();
    // Opening the real Store starts its derived-archive publisher. Wait for
    // that existing setup work to commit before attributing another
    // connection's data_version change to this preview. Keep the actual
    // no-commit assertion below; do not disable the production worker.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let drained: bool = observer.query_row(
            "SELECT published_generation=target_generation FROM work_archive_publication WHERE singleton=1",
            [], |row| row.get(0),
        ).unwrap();
        if drained { break; }
        assert!(std::time::Instant::now() < deadline, "The real setup archive publication did not settle");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let before: i64 = observer
        .query_row("PRAGMA data_version", [], |row| row.get(0))
        .unwrap();
    let preview = expert_message::preview_capture(&mut store, &input, &authority()).unwrap();
    let after: i64 = observer
        .query_row("PRAGMA data_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        before, after,
        "A private preview must not commit any writes"
    );
    assert_no_capture_parent(&mut store, &input);
    assert_eq!(
        observer
            .query_row("SELECT COUNT(*) FROM command_receipts", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        observer
            .query_row("SELECT COUNT(*) FROM runs", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        preview.participants.len(),
        input.selected_participants.len()
    );
    let context = host_context(&store, &input);
    let manifest = admit_with_host(&mut store, input.clone(), &context).unwrap();
    for (actual, planned) in manifest.entries.iter().zip(&preview.participants) {
        assert_eq!(actual.participant, planned.participant);
        assert_eq!(
            actual.context_configuration_revision,
            planned.context_configuration_revision
        );
        assert_eq!(actual.model_declaration, planned.model_declaration);
        assert_eq!(actual.defaults, planned.defaults);
    }
}

#[test]
fn capture_admission_rejects_profile_revision_drift_without_context_revision_change() {
    let (mut store, input) = setup();
    let preview = expert_message::preview_capture(&mut store, &input, &authority()).unwrap();
    let context = host_context(&store, &input);
    let mut profile = store.model_profile("model").unwrap().unwrap();
    profile.revision += 1;
    profile.display_name = "Changed profile".into();
    store.upsert_model_profile(&profile).unwrap();
    assert_eq!(
        store
            .load_conversation(&input.selected_participants[0].context_conversation_id)
            .unwrap()
            .unwrap()
            .configuration_revision,
        preview.participants[0].context_configuration_revision
    );
    assert!(admit_with_host(&mut store, input.clone(), &context)
        .unwrap_err()
        .contains("model_profile_changed"));
    assert_no_capture_parent(&mut store, &input);
    let refreshed = host_context(&store, &input);
    assert_eq!(
        admit_with_host(&mut store, input, &refreshed)
            .unwrap()
            .entries[0]
            .model_declaration
            .profile_revision,
        profile.revision
    );
}

#[test]
fn capture_preview_rejects_foreign_refs_and_stale_parent_without_mutation() {
    let (mut store, input) = setup();
    let mut stale = input.clone();
    stale.expected_revision += 1;
    assert!(expert_message::preview_capture(&mut store, &stale, &authority()).is_err());
    let mut room = input.clone();
    room.expected_room_revision += 1;
    assert!(expert_message::preview_capture(&mut store, &room, &authority()).is_err());
    let mut foreign = input.clone();
    foreign.selected_participants[0].agent_revision += 1;
    assert!(expert_message::preview_capture(&mut store, &foreign, &authority()).is_err());
    let mut context = input.clone();
    context.selected_participants[0].context_conversation_id = context.selected_participants[1]
        .context_conversation_id
        .clone();
    assert!(expert_message::preview_capture(&mut store, &context, &authority()).is_err());
    assert_no_capture_parent(&mut store, &input);
}

#[test]
fn capture_admission_rejects_inherited_default_drift_even_when_context_revision_is_unchanged() {
    let (mut store, input) = setup();
    for peer in &input.selected_participants {
        let context = store
            .load_conversation(&peer.context_conversation_id)
            .unwrap()
            .unwrap();
        let patched = store
            .patch_conversation_configuration(
                &nola_store_port::conversation_profile::PatchConversationConfigurationInput {
                    conversation_id: context.id,
                    expected_revision: context.configuration_revision,
                    patch_json: json!({"modelSelectionID":null})
                        .to_string()
                        .parse()
                        .unwrap(),
                    updated_at_ms: 100,
                },
            )
            .unwrap();
        assert!(patched.is_committed());
    }
    let preview = expert_message::preview_capture(&mut store, &input, &authority()).unwrap();
    let mut host = host_context(&store, &input);
    let mut other = store.model_profile("model").unwrap().unwrap();
    other.id = "other-model".into();
    other.selected = false;
    store.upsert_model_profile(&other).unwrap();
    store
        .put_product_setting(
            nola_application::model_selection::DEFAULT_CHAT_MODEL_KEY,
            &json!({"version":1,"selection":{"kind":"profile","profile_id":"other-model"}})
                .to_string(),
            101,
        )
        .unwrap();
    assert!(admit_with_host(&mut store, input.clone(), &host)
        .unwrap_err()
        .contains("model_profile_changed"));
    assert_no_capture_parent(&mut store, &input);
    let refreshed = expert_message::preview_capture(&mut store, &input, &authority()).unwrap();
    for (old, current) in preview.participants.iter().zip(&refreshed.participants) {
        assert_eq!(
            old.context_configuration_revision,
            current.context_configuration_revision
        );
        assert_eq!(current.model_declaration.profile_id, "other-model");
    }
    for selection in &mut host.selections {
        let model = &refreshed
            .participants
            .iter()
            .find(|entry| {
                entry.participant.context_conversation_id == selection.context_conversation_id
            })
            .unwrap()
            .model_declaration;
        selection.model_profile = nola_host_protocol::HostModelProfileReference {
            profile_id: model.profile_id.clone(),
            admitted_revision: model.profile_revision,
        };
    }
    assert_eq!(
        admit_with_host(&mut store, input, &host).unwrap().entries[0]
            .model_declaration
            .profile_id,
        "other-model"
    );
}

#[test]
fn capture_preview_effective_sparse_parameters_share_pinned_raw_declaration_without_rewriting_it() {
    let (mut store,input)=setup();
    let peer=&input.selected_participants[0];
    let context=store.load_conversation(&peer.context_conversation_id).unwrap().unwrap();
    let original=store.model_profile("model").unwrap().unwrap();
    let outcome=store.patch_conversation_configuration(&nola_store_port::conversation_profile::PatchConversationConfigurationInput {
        conversation_id:context.id.clone(),expected_revision:context.configuration_revision,
        patch_json:json!({"modelOverrides":{"configured:model":{"temperature":0.0,"topP":null,"seed":{"kind":"clear"}}}}).to_string().parse().unwrap(),updated_at_ms:20,
    }).unwrap();
    assert!(outcome.is_committed());
    let preview=expert_message::preview_capture(&mut store,&input,&authority()).unwrap();
    let actual=preview.participants.iter().find(|value|value.participant==*peer).unwrap();
    let effective=serde_json::to_value(&actual.effective_parameters).unwrap();
    assert_eq!(effective["status"],"available");
    assert_eq!(effective["temperature"],0.0);
    assert_eq!(effective["top_p"],0.9);
    assert_eq!(actual.model_declaration.parameters,serde_json::from_str::<serde_json::Value>(&original.parameters_json).unwrap());
    assert_eq!(store.model_profile("model").unwrap().unwrap(),original);
    assert!(store.expert_message_by_submission(&input.conversation_id,&input.submission_id).unwrap().is_none());
}
