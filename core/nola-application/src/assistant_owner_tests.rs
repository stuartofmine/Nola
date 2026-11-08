// SPDX-License-Identifier: Apache-2.0
use super::*;
use nola_store_port::assistant_journal::AssistantJournalRepository;
use nola_store::NolaStore;
#[path = "../../nola-store/tests/support/terminal_sources.rs"]
mod terminal_sources;
use nola_store_port::assistant_ownership::{AssistantPartOwner, AssistantTurnOwner};

fn activity_birth_events(session: &str, facts: &[(&str, &str, &str, &str)]) -> Vec<nola_store_port::run_creation::InitialCoreRunEvent> {
    facts.iter().enumerate().map(|(index, (turn, message, kind, id))| nola_store_port::run_creation::InitialCoreRunEvent {
        core_session_id: session.into(), core_turn_id: Some((*turn).into()), core_cursor: index as u64 + 1,
        core_revision: 1, event_kind: "assistant_activity_bound".into(),
        payload_json: serde_json::json!({"type":"assistant_activity_bound","turn_id":turn,
            "assistant_message_id":message,"activity_kind":kind,"activity_id":id}).to_string(),
    }).collect()
}
fn activity_bindings(turn: &str, message: &str, task: &str, skill: &str) -> Vec<nola_domain::AssistantTurnBinding> {
    [("task", task), ("skill", skill)].into_iter().map(|(kind, id)| nola_domain::AssistantTurnBinding::Activity {
        turn_id: turn.into(), assistant_message_id: message.into(), activity_kind: kind.into(), activity_id: id.into(),
    }).collect()
}

fn owner(message: &str) -> AssistantOwner {
    AssistantOwner {
        conversation_id: "conversation".into(),
        run_id: "run".into(),
        assistant_message_id: message.into(),
    }
}
fn facts() -> AssistantOwnershipFacts {
    AssistantOwnershipFacts {
        activities: Vec::new(),
        admissions: vec![owner("first")],
        turns: vec![
            AssistantTurnOwner {
                turn_id: "run".into(),
                owner: owner("first"),
                admission: true,
            },
            AssistantTurnOwner {
                turn_id: "next-turn".into(),
                owner: owner("next"),
                admission: false,
            },
        ],
        parts: vec![
            AssistantPartOwner {
                part_id: "original-text".into(),
                turn_id: Some("run".into()),
                owner: owner("first"),
            },
            AssistantPartOwner {
                part_id: "next-text".into(),
                turn_id: Some("next-turn".into()),
                owner: owner("next"),
            },
        ],
    }
}
fn attempt(id: &str, predecessor: Option<&str>, text: &str) -> ModelAttemptRecord {
    ModelAttemptRecord {
        id: id.into(),
        run_id: "run".into(),
        invocation_id: "invocation".into(),
        effect_id: "effect".into(),
        text_part_id: text.into(),
        reasoning_part_id: format!("{id}-reasoning"),
        predecessor_id: predecessor.map(str::to_owned),
        state: nola_domain::ModelAttemptState::Failed,
        provider: Some("provider".into()),
        model: Some("model".into()),
        failure: Some(nola_domain::ModelAttemptFailure {
            code: "failed".into(),
            message: "Safe failure".into(),
            retryable: true,
            http_status: None,
        }),
        usage: serde_json::json!({}),
        created_at_ms: 1,
        started_at_ms: Some(1),
        completed_at_ms: Some(2),
        revision: 2,
    }
}

#[test]
fn explicit_admission_is_stable_without_the_admitted_message_in_current_window() {
    let resolver =
        AssistantOwnerResolver::new("conversation", Some("conversation"), &facts()).unwrap();
    assert_eq!(
        resolver
            .resolve_turn("conversation", "run")
            .unwrap()
            .unwrap()
            .assistant_message_id,
        "first"
    );
    assert_eq!(
        resolver
            .resolve_turn("conversation", "next-turn")
            .unwrap()
            .unwrap()
            .assistant_message_id,
        "next"
    );
    assert!(resolver
        .resolve_turn("conversation", "unknown")
        .unwrap()
        .is_none());
    assert_eq!(
        resolver.resolve_turn("foreign", "run"),
        Err(AssistantOwnerError::ForeignSession)
    );
}

#[test]
fn core_session_is_distinct_from_canonical_owner_identity() {
    assert_eq!(
        validate_checkpoint_identity(
            "conversation",
            "conversation",
            "core-session",
            "core-session"
        ),
        Ok("core-session")
    );
    for (stored, root, state) in [
        ("foreign-conversation", "core-session", "core-session"),
        ("conversation", "foreign-core", "core-session"),
        ("conversation", "core-session", "foreign-core"),
        ("conversation", "", ""),
    ] {
        assert_eq!(
            validate_checkpoint_identity("conversation", stored, root, state),
            Err(AssistantOwnerError::ForeignSession)
        );
    }
    let resolver =
        AssistantOwnerResolver::new("conversation", Some("core-session"), &facts()).unwrap();
    assert_eq!(
        resolver
            .resolve_turn("core-session", "run")
            .unwrap()
            .unwrap()
            .conversation_id,
        "conversation"
    );
    assert_eq!(
        resolver.resolve_turn("conversation", "run"),
        Err(AssistantOwnerError::ForeignSession)
    );
    let no_core = AssistantOwnerResolver::new("conversation", None, &facts()).unwrap();
    assert_eq!(
        no_core.resolve_turn("core-session", "run"),
        Err(AssistantOwnerError::ForeignSession)
    );
}

#[test]
fn historical_part_membership_is_keyed_by_run_without_cross_run_lineage() {
    let mut input = facts();
    let mut historical_owner = owner("historical-assistant");
    historical_owner.run_id = "historical-run".into();
    input.parts.push(AssistantPartOwner {
        part_id: "original-text".into(),
        turn_id: Some("historical-turn".into()),
        owner: historical_owner,
    });
    let resolver =
        AssistantOwnerResolver::new("conversation", Some("core-session"), &input).unwrap();
    let current = attempt("current", None, "original-text");
    let mut historical = attempt("historical", None, "original-text");
    historical.run_id = "historical-run".into();
    assert_eq!(
        resolver
            .resolve_attempt(&current, &BTreeMap::new())
            .unwrap()
            .unwrap()
            .assistant_message_id,
        "first"
    );
    assert_eq!(
        resolver
            .resolve_attempt(&historical, &BTreeMap::new())
            .unwrap()
            .unwrap()
            .assistant_message_id,
        "historical-assistant"
    );
    historical.predecessor_id = Some("current".into());
    assert!(matches!(
        resolver.resolve_attempt(&historical, &BTreeMap::from([("current", &current)])),
        Err(AssistantOwnerError::AttemptLineage(_))
    ));
    // A successor-only Part is not evidence for the historical Run, even
    // when the two Runs share the same physical assistant message.
    let mut old_with_new_text = attempt("old-new-text", None, "next-text");
    old_with_new_text.run_id = "historical-run".into();
    assert_eq!(
        resolver.resolve_attempt(&old_with_new_text, &BTreeMap::new()),
        Err(AssistantOwnerError::ForeignRun)
    );
    let mut old_with_new_reasoning = historical.clone();
    old_with_new_reasoning.predecessor_id = None;
    old_with_new_reasoning.reasoning_part_id = "next-text".into();
    assert_eq!(
        resolver.resolve_attempt(&old_with_new_reasoning, &BTreeMap::new()),
        Err(AssistantOwnerError::ForeignRun)
    );
}

#[test]
fn retry_without_canonical_physical_parts_follows_exact_same_effect_predecessor() {
    let resolver =
        AssistantOwnerResolver::new("conversation", Some("conversation"), &facts()).unwrap();
    let original = attempt("a", None, "original-text");
    let retry = attempt("b", Some("a"), "not-materialized-physical-part");
    let index = BTreeMap::from([("a", &original), ("b", &retry)]);
    assert_eq!(
        resolver
            .resolve_attempt(&retry, &index)
            .unwrap()
            .unwrap()
            .assistant_message_id,
        "first"
    );
    let mut forged = retry.clone();
    forged.run_id = "foreign-run".into();
    assert!(matches!(
        resolver.resolve_attempt(&forged, &index),
        Err(AssistantOwnerError::AttemptLineage(_))
    ));
    let mut forged = retry.clone();
    forged.effect_id = "foreign-effect".into();
    assert!(resolver.resolve_attempt(&forged, &index).is_err());
    let mut direct = original.clone();
    direct.run_id = "foreign-run".into();
    assert_eq!(
        resolver.resolve_attempt(&direct, &index),
        Err(AssistantOwnerError::ForeignRun)
    );
}

#[test]
fn ambiguous_turn_is_an_error_only_for_synthetic_resolution_not_ordinary_part_reads() {
    let mut input = facts();
    input.turns.push(AssistantTurnOwner {
        turn_id: "next-turn".into(),
        owner: owner("other"),
        admission: false,
    });
    input.turns.push(AssistantTurnOwner {
        turn_id: "run".into(),
        owner: owner("next"),
        admission: false,
    });
    input.parts.push(AssistantPartOwner {
        part_id: "other".into(),
        turn_id: Some("next-turn".into()),
        owner: owner("other"),
    });
    // Also preserve direct canonical Part binding for an original-turn Part
    // moved into a successor message: admission still owns original-turn cards.
    input.parts.push(AssistantPartOwner {
        part_id: "bound".into(),
        turn_id: Some("run".into()),
        owner: owner("next"),
    });
    let resolver =
        AssistantOwnerResolver::new("conversation", Some("conversation"), &input).unwrap();
    assert!(matches!(
        resolver.resolve_turn("conversation", "next-turn"),
        Err(AssistantOwnerError::AmbiguousTurn(_))
    ));
    assert_eq!(
        resolver
            .resolve_turn("conversation", "run")
            .unwrap()
            .unwrap()
            .assistant_message_id,
        "first"
    );
    let next_attempt = attempt("next", None, "bound");
    assert_eq!(
        resolver
            .resolve_attempt(&next_attempt, &BTreeMap::new())
            .unwrap()
            .unwrap()
            .assistant_message_id,
        "next"
    );
}

#[test]
fn admission_turn_identity_is_not_guessed_from_the_run() {
    let mut input = facts();
    input.turns[0].turn_id = "actual-admitted-core-turn".into();
    let resolver =
        AssistantOwnerResolver::new("conversation", Some("conversation"), &input).unwrap();
    assert_eq!(
        resolver
            .resolve_turn("conversation", "actual-admitted-core-turn")
            .unwrap()
            .unwrap()
            .assistant_message_id,
        "first"
    );
    // Even a canonical Part's former JSON turn spelling is not a persisted
    // admission. No run-id or text metadata fallback may synthesize it.
    assert!(resolver
        .resolve_turn("conversation", "run")
        .unwrap()
        .is_none());
    input.turns.clear();
    let unresolved =
        AssistantOwnerResolver::new("conversation", Some("conversation"), &input).unwrap();
    assert!(unresolved
        .resolve_turn("conversation", "next-turn")
        .unwrap()
        .is_none());
}

#[test]
fn an_admission_turn_requires_the_exact_canonical_admission_owner() {
    let mut input = facts();
    input.turns[0].owner = owner("forged-message");
    assert!(matches!(
        AssistantOwnerResolver::new("conversation", Some("conversation"), &input),
        Err(AssistantOwnerError::InvalidFact)
    ));
}

#[test]
fn adding_successor_messages_does_not_rehome_failed_attempt_cards() {
    let mut input = facts();
    input.parts.push(AssistantPartOwner {
        part_id: "third".into(),
        turn_id: Some("third-turn".into()),
        owner: owner("third"),
    });
    let resolver =
        AssistantOwnerResolver::new("conversation", Some("conversation"), &input).unwrap();
    let original = attempt("a", None, "original-text");
    assert_eq!(
        resolver
            .resolve_attempt(&original, &BTreeMap::new())
            .unwrap()
            .unwrap()
            .assistant_message_id,
        "first"
    );
    let unresolved = attempt("unknown", None, "no-evidence");
    assert!(resolver
        .resolve_attempt(&unresolved, &BTreeMap::new())
        .unwrap()
        .is_none());
}

#[test]
fn real_store_window_keeps_admission_facts_and_does_not_broadcast_task_cards() {
    use nola_agent_engine::agent::context_window::ContextWindowConfig;
    use nola_agent_engine::agent::recovery::{CheckpointReason, CoreCheckpoint};
use nola_domain::core_event::{EventCursor};
    use nola_agent_engine::agent::{
        AgentSessionConfiguration, AgentThreadState, SessionId,
    };
    use nola_store::{
        BeginTurnInput, CommitTransitionInput, MessagePartInput, NolaStore, TransitionMessageInput,
    };
    let directory = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(directory.path().join("owner.db")).unwrap();
    let mut state = AgentThreadState::new(
        SessionId::from_string("core-owner-session".into()),
        AgentSessionConfiguration {
            model: None,
            max_model_iterations: 0,
            max_strategy_recovery_attempts: 3,
            tools: vec![],
            context_window: ContextWindowConfig::default(),
            memory_policy: Default::default(),
            turn_identity: crate::model_fixture::identity(),
        },
    );
    state.conversation.projection = serde_json::from_value(serde_json::json!({"active_message_segments":{"run":"first","next-turn":"next"},"activity_message_owners":{"task:original-task":"first","skill:original-skill":"first","task:next-task":"next","skill:next-skill":"next"},"revision":1,"order":["text","next-text"],"parts":{
        "text":{"kind":"text","id":"text","message_id":"first","turn_id":"run","text":"first","is_complete":false},
        "next-text":{"kind":"text","id":"next-text","message_id":"next","turn_id":"next-turn","text":"next","is_complete":false}
    }})).unwrap();
    state.task_projection = serde_json::from_value(serde_json::json!({"revision":1,"order":["original-task","next-task"],"tasks":{
        "original-task":{"id":"original-task","owner_session_id":"core-owner-session","owner_turn_id":"run","kind":"tool","state":"running"},
        "next-task":{"id":"next-task","owner_session_id":"core-owner-session","owner_turn_id":"next-turn","kind":"tool","state":"running"}
    }})).unwrap();
    for (id, turn) in [("original-skill", "run"), ("next-skill", "next-turn")] {
        let skill = serde_json::from_value(serde_json::json!({"run_id":id,"owner":{"session_id":"core-owner-session","parent_turn_id":turn},
            "descriptor":{"skill_id":"skill","version":"1","display_name":"Skill","instructions_ref":format!("skill#sha256={}","a".repeat(64)),"instructions_digest":"a".repeat(64)},
            "mode":"inline","status":"running","budget":{"max_model_turns":100,"max_tool_calls":100,"max_elapsed_millis":null},
            "usage":{"model_turns":0,"tool_calls":0,"elapsed_millis":0},"catalog_revision":1})).unwrap();
        state.skill_runtime.runs.insert(id.into(), skill);
    }
    let checkpoint = CoreCheckpoint {
        schema_version: nola_agent_engine::agent::recovery::AGENT_CHECKPOINT_SCHEMA_VERSION,
        session_id: "core-owner-session".into(),
        revision: 1,
        last_event_cursor: EventCursor { sequence: 4 },
        capability_catalog_revision: 0,
        created_at_ms: 1,
        reason: CheckpointReason::Manual,
        state,
    };
    store
        .begin_turn(&BeginTurnInput {
            user_part_kind: "text".into(),
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            initial_events: activity_birth_events("core-owner-session", &[("run","first","task","original-task"),("run","first","skill","original-skill"),("next-turn","next","task","next-task"),("next-turn","next","skill","next-skill")]),
            initial_assistant_part_mutations: vec![],
            assistant_turn_bindings: vec![
                nola_domain::AssistantTurnBinding::Admission {
                    turn_id: "run".into(),
                    assistant_message_id: "first".into(),
                },
                nola_domain::AssistantTurnBinding::Part {
                    turn_id: "run".into(),
                    assistant_message_id: "first".into(),
                    part_id: "text".into(),
                },
            ].into_iter().chain(activity_bindings("run", "first", "original-task", "original-skill")).collect(),
            conversation_id: "conversation".into(),
            conversation_title: "owners".into(),
            user_message_id: "user".into(),
            user_part_id: "user-text".into(),
            user_text: "hi".into(),
            user_content_json: None,
            additional_user_parts: vec![],
            assistant_message_id: "first".into(),
            assistant_part_id: "text".into(),
            run_id: "run".into(),
            context_checkpoint_id: None,
            checkpoint_revision: 1,
            checkpoint_event_cursor: 4,
            checkpoint_json: serde_json::to_string(&checkpoint).unwrap(),
            invocation_config_json: "{}".into(),
            invocation_config_digest: "test".into(),
            outbox_effects: vec![],
            model_attempt_mutations: vec![],
            delegated_invocation_mutations: vec![],
            delegated_permission_mutations: vec![],
            memory_usage_entries: vec![],
            created_at_ms: 1,
        })
        .unwrap();
    store
        .commit_transition(&CommitTransitionInput {
            assistant_turn_bindings: vec![nola_domain::AssistantTurnBinding::Part {
                turn_id: "next-turn".into(),
                assistant_message_id: "next".into(),
                part_id: "next-text".into(),
            }].into_iter().chain(activity_bindings("next-turn", "next", "next-task", "next-skill")).collect(),
            transition_id: "successor".into(),
            run_id: "run".into(),
            expected_checkpoint_revision: 1,
            checkpoint_revision: 2,
            checkpoint_event_cursor: 4,
            checkpoint: nola_store_port::run_effect::CheckpointWrite::Baseline {
                checkpoint_json: serde_json::to_string(&checkpoint).unwrap(),
            },
            submission_reduction_json: None, host_event: None,
            events: vec![],
            ensured_messages: vec![TransitionMessageInput {
                id: "next".into(),
                run_id: Some("run".into()),
                role: "assistant".into(),
                status: "streaming".into(),
                created_at_ms: 2,
                completed_at_ms: None,
                initial_parts: vec![MessagePartInput {
                    id: "next-text".into(),
                    kind: "text".into(),
                    ordinal: 1,
                    state: "streaming".into(),
                    content_text: "next".into(),
                    content_json: None,
                    content_digest: None,
                    created_at_ms: 2,
                    updated_at_ms: 2,
                }],
            }],
            part_message_owners: Default::default(),
            part_mutations: vec![],
            outbox_effects: vec![],
            model_attempt_mutations: vec![],
            delegated_invocation_mutations: vec![],
            delegated_permission_mutations: vec![],
            effect_delivery_attempt_mutations: vec![],
            acknowledged_effect_ids: vec![],
            memory_usage_entries: vec![],
            run_status: None,
            assistant_status: None,
            assistant_status_message_id: None,
            committed_at_ms: 2,
        })
        .unwrap();
    let raw: serde_json::Value = serde_json::from_str(
        &store
            .load_local_presentation_inputs("conversation", r#"["next"]"#, None)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    let ownership: AssistantOwnershipFacts =
        serde_json::from_value(raw["ownership"].clone()).unwrap();
    assert_eq!(ownership.admissions[0].assistant_message_id, "first");
    assert!(raw["tasks"]["tasks"].get("original-task").is_none());
    assert!(raw["tasks"]["tasks"].get("next-task").is_some());
    let window =
        crate::load_conversation_projection_window_v1(&store, "conversation", None, 1, 32_768)
            .unwrap();
    assert_eq!(window.projection.messages.len(), 1);
    let ids: Vec<_> = window.projection.messages[0]
        .parts
        .iter()
        .filter_map(|p| {
            if let nola_domain::MessagePartView::Task { task_id, .. } = p {
                Some(task_id.as_str())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(ids, vec!["next-task"]);
    let skill_ids: Vec<_> = window.projection.messages[0]
        .parts
        .iter()
        .filter_map(|p| {
            if let nola_domain::MessagePartView::Skill { run_id, .. } = p {
                Some(run_id.as_str())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(skill_ids, vec!["next-skill"]);
    let all = crate::load_conversation_projection_v1(&store, "conversation").unwrap();
    let task_messages: Vec<_> = all
        .messages
        .iter()
        .flat_map(|m| {
            m.parts.iter().filter_map(move |p| {
                if let nola_domain::MessagePartView::Task { task_id, .. } = p {
                    Some((m.message_id.0.as_str(), task_id.as_str()))
                } else {
                    None
                }
            })
        })
        .collect();
    assert_eq!(
        task_messages,
        vec![("first", "original-task"), ("next", "next-task")]
    );
    let skill_messages: Vec<_> = all
        .messages
        .iter()
        .flat_map(|m| {
            m.parts.iter().filter_map(move |p| {
                if let nola_domain::MessagePartView::Skill { run_id, .. } = p {
                    Some((m.message_id.0.as_str(), run_id.as_str()))
                } else {
                    None
                }
            })
        })
        .collect();
    assert_eq!(
        skill_messages,
        vec![("first", "original-skill"), ("next", "next-skill")]
    );
    // Canonical reads retain the exact owner's cards without a remote snapshot adapter.
    let owner_message = all
        .messages
        .iter()
        .find(|m| m.message_id.0 == "first")
        .unwrap();
    let remote_tasks: Vec<_> = owner_message
        .parts
        .iter()
        .filter_map(|p| {
            if let nola_domain::MessagePartView::Task { task_id, .. } = p {
                Some(task_id.as_str())
            } else {
                None
            }
        })
        .collect();
    let remote_skills: Vec<_> = owner_message
        .parts
        .iter()
        .filter_map(|p| {
            if let nola_domain::MessagePartView::Skill { run_id, .. } = p {
                Some(run_id.as_str())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(remote_tasks, vec!["original-task"]);
    assert_eq!(remote_skills, vec!["original-skill"]);
}

#[test]
fn real_internal_handoff_keeps_historical_cards_after_cold_reopen() {
    use nola_domain::{AssistantTurnBinding, MessagePartView, ModelAttemptMutation};
    use nola_agent_engine::agent::context_window::ContextWindowConfig;
    use nola_agent_engine::agent::recovery::{CheckpointReason, CoreCheckpoint};
use nola_domain::core_event::{EventCursor};
    use nola_agent_engine::agent::{
        AgentSessionConfiguration, AgentThreadState, SessionId,
    };
    use nola_store::{BeginInternalTurnInput, BeginTurnInput, NolaStore};

    const CORE_SESSION: &str = "different-core-session";
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("historical-owner.db");
    let mut store = NolaStore::open(&database).unwrap();
    let mut state = AgentThreadState::new(
        SessionId::from_string(CORE_SESSION.into()),
        AgentSessionConfiguration {
            model: None,
            max_model_iterations: 0,
            max_strategy_recovery_attempts: 3,
            tools: vec![],
            context_window: ContextWindowConfig::default(),
            memory_policy: Default::default(),
            turn_identity: crate::model_fixture::identity(),
        },
    );
    state.conversation.projection = serde_json::from_value(serde_json::json!({
        "active_message_segments":{"old-core-turn":"visible-assistant"},
        "activity_message_owners":{"task:old-task":"visible-assistant","skill:old-skill":"visible-assistant"},
        "revision":1,"order":["old-text"],"parts":{
            "old-text":{"kind":"text","id":"old-text","message_id":"visible-assistant",
                "turn_id":"old-core-turn","text":"Before resumption","is_complete":true}
        }
    }))
    .unwrap();
    state.task_projection = serde_json::from_value(serde_json::json!({
        "revision":1,"order":["old-task"],"tasks":{
            "old-task":{"id":"old-task","owner_session_id":CORE_SESSION,
                "owner_turn_id":"old-core-turn","kind":"tool","state":"failed"}
        }
    }))
    .unwrap();
    let skill = serde_json::from_value(serde_json::json!({
        "run_id":"old-skill","owner":{"session_id":CORE_SESSION,"parent_turn_id":"old-core-turn"},
        "descriptor":{"skill_id":"skill","version":"1","display_name":"Historical skill",
            "instructions_ref":format!("skill#sha256={}","a".repeat(64)),"instructions_digest":"a".repeat(64)},
        "mode":"inline","status":"failed","budget":{"max_model_turns":100,"max_tool_calls":100,"max_elapsed_millis":null},
        "usage":{"model_turns":0,"tool_calls":0,"elapsed_millis":0},"catalog_revision":1
    })).unwrap();
    state.skill_runtime.runs.insert("old-skill".into(), skill);
    let mut checkpoint = CoreCheckpoint {
        schema_version: nola_agent_engine::agent::recovery::AGENT_CHECKPOINT_SCHEMA_VERSION,
        session_id: CORE_SESSION.into(),
        revision: 1,
        last_event_cursor: EventCursor { sequence: 2 },
        capability_catalog_revision: 0,
        created_at_ms: 1,
        reason: CheckpointReason::Manual,
        state,
    };
    let mut failed_attempt = attempt("old-attempt", None, "old-text");
    failed_attempt.run_id = "old-run".into();
    failed_attempt.revision = 1;
    store
        .begin_turn(&BeginTurnInput {
            user_part_kind: "text".into(),
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            initial_events: activity_birth_events(CORE_SESSION, &[("old-core-turn","visible-assistant","task","old-task"),("old-core-turn","visible-assistant","skill","old-skill")]),
            conversation_id: "conversation".into(),
            conversation_title: "Historical owners".into(),
            user_message_id: "user".into(),
            user_part_id: "user:body".into(),
            user_text: "Continue".into(),
            user_content_json: None,
            additional_user_parts: vec![],
            assistant_message_id: "visible-assistant".into(),
            assistant_part_id: "old-text".into(),
            initial_assistant_part_mutations: vec![],
            assistant_turn_bindings: vec![
                AssistantTurnBinding::Admission {
                    turn_id: "old-core-turn".into(),
                    assistant_message_id: "visible-assistant".into(),
                },
                AssistantTurnBinding::Part {
                    turn_id: "old-core-turn".into(),
                    assistant_message_id: "visible-assistant".into(),
                    part_id: "old-text".into(),
                },
            ].into_iter().chain(activity_bindings("old-core-turn", "visible-assistant", "old-task", "old-skill")).collect(),
            run_id: "old-run".into(),
            context_checkpoint_id: None,
            checkpoint_revision: 1,
            checkpoint_event_cursor: 2,
            checkpoint_json: serde_json::to_string(&checkpoint).unwrap(),
            invocation_config_json: "{}".into(),
            invocation_config_digest: "fixture".into(),
            outbox_effects: vec![],
            model_attempt_mutations: vec![ModelAttemptMutation {
                expected_revision: None,
                record: failed_attempt,
            }],
            delegated_invocation_mutations: vec![],
            delegated_permission_mutations: vec![],
            memory_usage_entries: vec![],
            created_at_ms: 1,
        })
        .unwrap();
    // A successor requires a real terminal result, publication and seal. Keep
    // the historical Core identity and projections while committing the
    // original input/scope/error/result facts through the normal Store UoW.
    let mut terminal = nola_store_port::run_effect::CommitTransitionInput {
        transition_id: "old-run-terminal".into(), run_id: "old-run".into(),
        expected_checkpoint_revision: 1, checkpoint_revision: 2,
        checkpoint_event_cursor: 0,
        checkpoint: nola_store_port::run_effect::CheckpointWrite::Baseline { checkpoint_json: String::new() },
        submission_reduction_json: None, host_event: None, events: vec![], ensured_messages: vec![],
        assistant_turn_bindings: vec![], part_message_owners: Default::default(), part_mutations: vec![],
        outbox_effects: vec![], model_attempt_mutations: vec![], delegated_invocation_mutations: vec![],
        delegated_permission_mutations: vec![], effect_delivery_attempt_mutations: vec![],
        acknowledged_effect_ids: vec![], memory_usage_entries: vec![], run_status: None,
        assistant_status: Some("completed".into()), assistant_status_message_id: None, committed_at_ms: 2,
    };
    terminal_sources::add_sources(&store, &mut terminal, "conversation", "old-core-turn", "user", false);
    for event in &mut terminal.events {
        let mut payload: serde_json::Value = serde_json::from_str(&event.payload_json).unwrap();
        payload["core_envelope"]["session_id"] = serde_json::json!(CORE_SESSION);
        event.payload_json = payload.to_string();
    }
    checkpoint.revision = 2;
    checkpoint.created_at_ms = 2;
    checkpoint.last_event_cursor.sequence = terminal.checkpoint_event_cursor as u64;
    terminal.checkpoint = nola_store_port::run_effect::CheckpointWrite::Baseline {
        checkpoint_json: serde_json::to_string(&checkpoint).unwrap(),
    };
    store.begin_write_unit_of_work().unwrap();
    assert!(store.commit_transition(&terminal).unwrap().applied);
    terminal_sources::publish_and_seal(&mut store, &terminal, "conversation");
    store.commit_write_unit_of_work().unwrap();
    assert!(store.local_assistant_run_terminal_seal("conversation", "old-run").unwrap().is_some());
    let target = store
        .internal_assistant_target("conversation")
        .unwrap()
        .unwrap();
    checkpoint.revision = 3;
    checkpoint.created_at_ms = 3;
    store
        .begin_internal_turn(&BeginInternalTurnInput {
            initial_events: Vec::new(),
            conversation_id: "conversation".into(),
            assistant_message_id: target.assistant_message_id,
            expected_previous_run_id: target.run_id,
            expected_checkpoint_revision: Some(2),
            assistant_part_id: "not-yet-materialized".into(),
            initial_assistant_part_mutations: vec![],
            assistant_turn_bindings: vec![AssistantTurnBinding::Admission {
                turn_id: "resumed-core-turn".into(),
                assistant_message_id: "visible-assistant".into(),
            }],
            run_id: "resumed-run".into(),
            context_checkpoint_id: None,
            checkpoint_revision: 3,
            checkpoint_event_cursor: checkpoint.last_event_cursor.sequence as i64,
            checkpoint_json: serde_json::to_string(&checkpoint).unwrap(),
            invocation_config_json: "{}".into(),
            invocation_config_digest: "fixture".into(),
            outbox_effects: vec![],
            model_attempt_mutations: vec![],
            delegated_invocation_mutations: vec![],
            delegated_permission_mutations: vec![],
            memory_usage_entries: vec![],
            created_at_ms: 3,
        })
        .unwrap();
    drop(store);
    let store = NolaStore::open(&database).unwrap();
    let projection = crate::load_conversation_projection_v1(&store, "conversation").unwrap();
    assert_eq!(projection.messages.len(), 2);
    assert_eq!(projection.messages[1].message_id.0, "visible-assistant");
    let parts = &projection.messages[1].parts;
    assert!(parts.iter().any(
        |part| matches!(part, MessagePartView::Task { task_id, .. } if task_id == "old-task")
    ));
    assert!(parts.iter().any(
        |part| matches!(part, MessagePartView::Skill { run_id, .. } if run_id == "old-skill")
    ));
    assert!(parts.iter().any(
        |part| matches!(part, MessagePartView::ModelAttempt { attempt_id, failure, .. }
        if attempt_id.0 == "old-attempt" && failure.message == "Safe failure")
    ));
}
