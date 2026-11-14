// SPDX-License-Identifier: Apache-2.0
// Included by expert_message_transactions; uses isolated canonical Store/Run APIs.
fn patch_expert_context(store: &mut NolaStore, context: &str, patch: serde_json::Value) {
    let current = store.load_conversation(context).unwrap().unwrap();
    let receipt = store
        .patch_conversation_configuration(
            &nola_store_port::conversation_profile::PatchConversationConfigurationInput {
                conversation_id: context.into(),
                expected_revision: current.configuration_revision,
                patch_json: patch.to_string().parse().unwrap(),
                updated_at_ms: 50,
            },
        )
        .unwrap();
    assert!(matches!(
        receipt,
        nola_domain::conversation_configuration::ConfigurationCommitResult::Committed { .. }
    ));
}

#[test]
fn every_core_role_is_captured_without_client_builtin_claims_and_survives_replay() {
    for role in &nola_application::expert_roles::catalog().templates {
        let (mut store, input) = setup();
        select_managed(&mut store, &input, 0, &role.id);
        let manifest = admit(&mut store, input.clone()).unwrap();
        let first = &manifest.entries[0];
        assert_eq!(
            first.expected_prompt_definition.as_ref().unwrap().content,
            role.instructions()
        );
        let frozen = expert_message::load_child_instructions_authorized(
            &mut store,
            &first.participant.context_conversation_id,
            &first.child_submission_id,
            &authority(),
        )
        .unwrap()
        .unwrap();
        for definition in &frozen {
            let _: nola_agent_engine::context_engine::ContextComponent = serde_json::from_value(json!({
                "kind":"prompt_template", "id":definition.component_id,
                "content":definition.content, "platform":definition.platform,
            })).expect("Portable role definitions must pass the real Host component decoder");
        }
        assert_eq!(frozen[0].content, role.instructions());
        assert_eq!(admit(&mut store, input).unwrap(), manifest);
        complete_captured_source(&mut store, &manifest);
        assert!(store
            .canonical_run_terminal(
                &first.participant.context_conversation_id,
                &first.child_submission_id
            )
            .unwrap()
            .is_some());
    }
}

#[test]
fn role_uuid_collision_cannot_shadow_core_instructions_or_capture_private_content() {
    let (mut store, input) = setup();
    let role = &nola_application::expert_roles::catalog().templates[0];
    select_managed(&mut store, &input, 0, &role.id);
    managed_catalog(
        &mut store,
        &role.id,
        Some("capture-user"),
        json!([]),
        "shadowed private instructions",
    );
    let error = admit(&mut store, input.clone()).unwrap_err();
    assert!(error.contains("ambiguous"));
    assert_no_capture_parent(&mut store, &input);
}

#[test]
fn expert_exact_effort_resolves_against_the_declared_model_before_admission() {
    for effort in ["minimal", "low", "medium", "high", "xhigh"] {
        let (mut store, input) = setup();
        let mut profile = store.model_profile("model").unwrap().unwrap();
        profile.revision = 2;
        profile.source_json = json!({"baseURL":"https://fixture.invalid/v1", "wireProtocol":"responses", "capabilities":["chat"],
            "reasoning":{"control":"effort","efforts":["minimal","low","medium","high","xhigh"]},
            "executionCapabilities":{"toolCalls":true,"imageInput":false,"reasoning":true,"samplingParameters":true}}).to_string();
        store.upsert_model_profile(&profile).unwrap();
        let context = &input.selected_participants[0].context_conversation_id;
        patch_expert_context(&mut store, context, json!({"expertReasoningEffort":effort}));
        let preview = expert_message::preview_capture(&mut store, &input, &authority()).unwrap();
        let parameters =
            serde_json::to_value(&preview.participants[0].effective_parameters).unwrap();
        assert_eq!(
            parameters["reasoning_selection"],
            json!({"kind":"effort","value":effort})
        );
        let manifest = admit(&mut store, input).unwrap();
        let defaults = manifest.entries[0].defaults.conversation_configuration();
        assert_eq!(
            defaults["modelOverrides"]["configured:model"]["reasoningPreference"],
            effort
        );
    }
    for unsupported in ["none", "high"] {
        let (mut store, input) = setup(); // This model explicitly declares no reasoning support.
        patch_expert_context(
            &mut store,
            &input.selected_participants[0].context_conversation_id,
            json!({"expertReasoningEffort":unsupported}),
        );
        assert!(expert_message::preview_capture(&mut store, &input, &authority()).is_err());
        assert!(admit(&mut store, input.clone()).is_err());
        assert_no_capture_parent(&mut store, &input);
    }
}

fn update_first_expert(store: &mut NolaStore) {
    let directory = agents::load(&store.product_setting_repository()).unwrap();
    let first = directory
        .agents
        .iter()
        .find(|agent| agent.id().as_str() == "first")
        .unwrap();
    let command = agents::ConfigureAgent {
        id: first.id().clone(),
        expected_directory_revision: directory.revision,
        expected_agent_revision: Some(first.revision()),
        name: first.name().into(),
        description: "Updated code reviewer".into(),
        defaults: AgentDefaults::try_from(json!({"modelSelectionID":"configured:model",
            "selectedPromptTemplateID":nola_application::expert_roles::catalog().templates[1].id}))
        .unwrap(),
        availability: AgentAvailability::Active,
        updated_at_ms: 90,
    };
    fixture_command(store, |store| {
        agents::configure(&mut store.product_setting_repository(), command)
    })
    .unwrap();
}

#[test]
fn explicit_rejoin_is_atomic_preserves_history_and_cannot_duplicate_on_retry() {
    let (mut store, input) = setup();
    let manifest = admit(&mut store, input.clone()).unwrap();
    complete_captured_source(&mut store, &manifest);
    let old = manifest.entries[0].participant.clone();
    let history = store
        .load_conversation(&old.context_conversation_id)
        .unwrap()
        .unwrap();
    update_first_expert(&mut store);
    let snapshot = rooms::load(&store.product_setting_repository(), &input.room_id)
        .unwrap()
        .unwrap();
    let command = rooms::JoinAgent {
        room_id: input.room_id.clone(),
        expected_room_revision: snapshot.revision,
        agent_id: old.agent_id.clone(),
        expected_agent_revision: 2,
        at_ms: 100,
    };
    // A stale directory snapshot cannot replace the current roster.
    let stale = rooms::JoinAgent {
        expected_agent_revision: 999,
        ..command.clone()
    };
    assert!(fixture_command(&mut store, |store| rooms::rejoin(store, stale)).is_err());
    assert_eq!(
        rooms::load(&store.product_setting_repository(), &input.room_id)
            .unwrap()
            .unwrap(),
        snapshot
    );
    let replaced =
        fixture_command(&mut store, |store| rooms::rejoin(store, command.clone())).unwrap();
    let member = replaced
        .room
        .participants()
        .iter()
        .find(|member| member.agent_id == old.agent_id)
        .unwrap();
    assert_eq!(member.agent_revision, 2);
    assert_ne!(member.context_conversation_id, old.context_conversation_id);
    assert_ne!(member.id, old.participant_id);
    assert_eq!(
        store
            .load_conversation(&old.context_conversation_id)
            .unwrap()
            .unwrap(),
        history
    );
    assert_eq!(
        expert_message::load_child_instructions_authorized(
            &mut store,
            &old.context_conversation_id,
            &manifest.entries[0].child_submission_id,
            &authority()
        )
        .unwrap()
        .unwrap()
        .len(),
        0
    );
    assert!(fixture_command(&mut store, |store| rooms::rejoin(store, command)).is_err());
    assert_eq!(
        rooms::load(&store.product_setting_repository(), &input.room_id)
            .unwrap()
            .unwrap(),
        replaced
    );
}

#[test]
fn rejoin_rejects_pending_work_without_mutating_its_owner() {
    let (mut store, input) = setup();
    let manifest = admit(&mut store, input.clone()).unwrap();
    update_first_expert(&mut store);
    let snapshot = rooms::load(&store.product_setting_repository(), &input.room_id)
        .unwrap()
        .unwrap();
    let command = rooms::JoinAgent {
        room_id: input.room_id.clone(),
        expected_room_revision: snapshot.revision,
        agent_id: manifest.entries[0].participant.agent_id.clone(),
        expected_agent_revision: 2,
        at_ms: 100,
    };
    assert!(
        fixture_command(&mut store, |store| rooms::rejoin(store, command))
            .unwrap_err()
            .contains("pending_submission")
    );
    assert_eq!(
        rooms::load(&store.product_setting_repository(), &input.room_id)
            .unwrap()
            .unwrap(),
        snapshot
    );
    assert_eq!(
        expert_message::load_child_instructions_authorized(
            &mut store,
            &manifest.entries[0].participant.context_conversation_id,
            &manifest.entries[0].child_submission_id,
            &authority()
        )
        .unwrap()
        .unwrap()
        .len(),
        0
    );
}

#[test]
fn rejoin_final_write_failure_rolls_back_the_new_child_and_all_receipts() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("application.db");
    let (mut store, input) = setup_at(&path);
    update_first_expert(&mut store);
    let snapshot = rooms::load(&store.product_setting_repository(), &input.room_id)
        .unwrap()
        .unwrap();
    let sql = rusqlite::Connection::open(&path).unwrap();
    let before: i64 = sql
        .query_row("SELECT COUNT(*) FROM conversations", [], |row| row.get(0))
        .unwrap();
    sql.execute_batch(&format!("CREATE TRIGGER reject_role_rejoin BEFORE UPDATE ON product_settings WHEN NEW.key='{}{}' BEGIN SELECT RAISE(ABORT,'fixture roster write failure'); END;",
        nola_domain::agent_room::ROOM_KEY_PREFIX, input.room_id)).unwrap();
    let command = rooms::JoinAgent {
        room_id: input.room_id.clone(),
        expected_room_revision: snapshot.revision,
        agent_id: input.selected_participants[0].agent_id.clone(),
        expected_agent_revision: 2,
        at_ms: 100,
    };
    assert!(
        fixture_command(&mut store, |store| rooms::rejoin(store, command))
            .unwrap_err()
            .contains("fixture roster write failure")
    );
    assert_eq!(
        rooms::load(&store.product_setting_repository(), &input.room_id)
            .unwrap()
            .unwrap(),
        snapshot
    );
    let after: i64 = sql
        .query_row("SELECT COUNT(*) FROM conversations", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        after, before,
        "The failed rejoin must not leave an orphan child"
    );
}
