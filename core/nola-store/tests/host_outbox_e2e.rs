use nola_store::host::replay_pending_effects;
use nola_store::{BeginTurnInput, NolaStore, OutboxEffectInput};
use tempfile::tempdir;

fn begin_input(effect_id: &str) -> BeginTurnInput {
    BeginTurnInput {
        user_part_kind: "text".into(),
        expected_configuration_revision: None,
        expected_parent_message_id: None,
        initial_events: Vec::new(),
        initial_assistant_part_mutations: vec![],
        memory_usage_entries: vec![],
        delegated_invocation_mutations: vec![],
        delegated_permission_mutations: vec![],
        model_attempt_mutations: vec![],
        conversation_id: "conversation-1".into(),
        conversation_title: "Outbox E2E".into(),
        user_message_id: "message-user".into(),
        user_part_id: "part-user".into(),
        user_text: "run the durable effect".into(),
        user_content_json: None,
        additional_user_parts: Vec::new(),
        assistant_message_id: "message-assistant".into(),
        assistant_part_id: "part-assistant".into(),
        assistant_turn_bindings: vec![],
        run_id: "run-1".into(),
        context_checkpoint_id: None,
        checkpoint_revision: 0,
        checkpoint_event_cursor: 0,
        checkpoint_json: "{\"state\":\"running\"}".into(),
        invocation_config_json: "{}".into(),
        invocation_config_digest: "conversation-config-digest".into(),
        outbox_effects: vec![OutboxEffectInput {
            delegated_invocation_id: None,
            model_attempt_id: None,
            effect_id: effect_id.into(),
            request_id: "request-1".into(),
            effect_kind: "tool.execute".into(),
            write: outbox_fixture::tool(effect_id, "request-1", serde_json::json!({"tool":"read_file"})),
        }],
        created_at_ms: 1_700_000_000_000,
    }
}

#[test]
fn failed_effect_survives_reopen_and_is_acknowledged_only_after_success() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("nola.db");
    {
        let mut store = NolaStore::open(&database).unwrap();
        store.begin_turn(&begin_input("effect-1")).unwrap();
        let report = replay_pending_effects(&mut store, "run-1", 10, 1_700_000_000_100, |_| {
            Err("host unavailable".into())
        })
        .unwrap();
        assert_eq!(report.dispatched, 1);
        assert_eq!(report.acknowledged, 0);
        assert!(report.stopped_after_failure);
        assert_eq!(store.pending_effects("run-1").unwrap().len(), 1);
    }

    let mut store = NolaStore::open(&database).unwrap();
    let mut observed = Vec::new();
    let report = replay_pending_effects(&mut store, "run-1", 10, 1_700_000_000_200, |effect| {
        observed.push((effect.effect_kind.clone(), effect.payload_json.clone()));
        Ok(())
    })
    .unwrap();
    assert_eq!(report.dispatched, 1);
    assert_eq!(report.acknowledged, 1);
    assert!(!report.stopped_after_failure);
    assert_eq!(
        observed,
        vec![("tool.execute".into(), match outbox_fixture::tool("effect-1", "request-1", serde_json::json!({"tool":"read_file"})) {
            nola_store_port::run_effect::OutboxEffectWrite::Declare { payload_json } => payload_json, _=>unreachable!(),
        })]
    );
    assert!(store.pending_effects("run-1").unwrap().is_empty());
}

#[test]
fn successful_prefix_is_not_replayed_when_a_later_effect_fails() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("nola.db");
    {
        let mut store = NolaStore::open(&database).unwrap();
        let mut input = begin_input("effect-1");
        input.outbox_effects.push(OutboxEffectInput {
            delegated_invocation_id: None,
            model_attempt_id: None,
            effect_id: "effect-2".into(),
            request_id: "request-2".into(),
            effect_kind: "artifact.publish".into(),
            write: outbox_fixture::tool("effect-2", "request-2", serde_json::json!({"artifact":"report.docx"})),
        });
        store.begin_turn(&input).unwrap();

        let mut first_pass = Vec::new();
        let report = replay_pending_effects(&mut store, "run-1", 10, 1_700_000_000_100, |effect| {
            first_pass.push(effect.effect_id.clone());
            if effect.effect_id == "effect-2" {
                Err("publisher unavailable".into())
            } else {
                Ok(())
            }
        })
        .unwrap();
        assert_eq!(first_pass, vec!["effect-1", "effect-2"]);
        assert_eq!(report.acknowledged, 1);
        assert!(report.stopped_after_failure);
    }

    let mut reopened = NolaStore::open(&database).unwrap();
    let mut replayed = Vec::new();
    let report = replay_pending_effects(&mut reopened, "run-1", 10, 1_700_000_000_200, |effect| {
        replayed.push(effect.effect_id.clone());
        Ok(())
    })
    .unwrap();
    assert_eq!(replayed, vec!["effect-2"]);
    assert_eq!(report.acknowledged, 1);
    assert!(reopened.pending_effects("run-1").unwrap().is_empty());
}
#[path = "support/outbox.rs"]
mod outbox_fixture;
