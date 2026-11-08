use nola_store::{BeginTurnInput, CommitTransitionInput, NolaStore, PartMutation};
#[path = "terminal_sources.rs"]
mod terminal_sources;

/// Exercise the actual local transition transaction before deleting a Run's
/// conversation. This is Store integration coverage, not a Core cancellation E2E.
pub fn cancel_before_delete(store: &mut NolaStore, opened: &BeginTurnInput) {
    assert!(store.active_run(&opened.conversation_id).unwrap().is_some());
    let before: Vec<_> = store
        .pending_sync_changes(20)
        .unwrap()
        .into_iter()
        .map(|change| change.operation_id)
        .collect();
    assert!(store
        .delete_conversation(&opened.conversation_id)
        .unwrap_err()
        .to_string()
        .contains("Run is active"));
    let after: Vec<_> = store
        .pending_sync_changes(20)
        .unwrap()
        .into_iter()
        .map(|change| change.operation_id)
        .collect();
    assert_eq!(before, after);

    let mut cancelled = CommitTransitionInput {
            memory_usage_entries: vec![],
            delegated_invocation_mutations: vec![],
            delegated_permission_mutations: vec![],
            effect_delivery_attempt_mutations: vec![],
            model_attempt_mutations: vec![],
            transition_id: format!("{}:cancel-before-delete", opened.run_id),
            run_id: opened.run_id.clone(),
            expected_checkpoint_revision: 1,
            checkpoint_revision: 2,
            checkpoint_event_cursor: 0,
            checkpoint: nola_store_port::run_effect::CheckpointWrite::Baseline { checkpoint_json: r#"{"revision":2}"#.into() },
            submission_reduction_json: None, host_event: None,
            events: Vec::new(),
            ensured_messages: Vec::new(),
            assistant_turn_bindings: vec![],
            part_message_owners: Default::default(),
            part_mutations: vec![PartMutation::Close {
                part_id: opened.assistant_part_id.clone(),
                state: "cancelled".into(),
                content_json: None,
                content_digest: None,
                updated_at_ms: 2_500,
            }],
            outbox_effects: Vec::new(),
            acknowledged_effect_ids: opened
                .outbox_effects
                .iter()
                .map(|effect| effect.effect_id.clone())
                .collect(),
            run_status: None,
            assistant_status: Some("cancelled".into()),
            assistant_status_message_id: None,
            committed_at_ms: 2_500,
        };
    let turn=opened.assistant_turn_bindings.iter().find_map(|binding|match binding {
        nola_domain::AssistantTurnBinding::Admission {turn_id,..}=>Some(turn_id.as_str()),
        _=>None,
    }).unwrap_or(&opened.run_id);
    terminal_sources::add_sources(store,&mut cancelled,&opened.conversation_id,turn,&opened.user_message_id,true);
    store.begin_write_unit_of_work().unwrap();
    let result=store.commit_transition(&cancelled).unwrap();
    terminal_sources::publish_and_seal(store,&cancelled,&opened.conversation_id);
    store.commit_write_unit_of_work().unwrap();
    assert!(result.applied);
    assert!(store.active_run(&opened.conversation_id).unwrap().is_none());
    let conversation = store
        .load_conversation(&opened.conversation_id)
        .unwrap()
        .unwrap();
    let assistant = conversation
        .messages
        .iter()
        .find(|message| message.id == opened.assistant_message_id)
        .unwrap();
    assert_eq!(assistant.status, "cancelled");
    assert_eq!(assistant.parts[0].state, "cancelled");
}
