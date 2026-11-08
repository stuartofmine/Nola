#[path = "support/model_identity.rs"]
mod model_identity;
use std::collections::BTreeMap;

use nola_core::agent::host::{
    CapabilityAvailability, HostCancellationMode, HostCapabilityCatalog, HostCapabilityDescriptor,
    HostCapabilityExposure, HostCapabilityParallelism, HostCapabilityRoute, HostId,
    HostOutputPolicy, HostReplaySafety, HostRequest,
};
use nola_core::agent::recovery::CheckpointReason;
use nola_core::agent::{
    AgentCore, AgentCoreConfig, AgentInputDelivery, AgentInputOrigin, AgentItem, AgentSubmission,
    AgentTurnContextSnapshot, GoalRecordStatus, GoalRequirement, GoalRuntimeCommand,
    GoalRuntimeCommandEnvelope, ItemId, SessionId,
};
use serde_json::json;

fn capability(id: &str, route: HostCapabilityRoute) -> HostCapabilityDescriptor {
    HostCapabilityDescriptor {
        id: id.to_string(),
        title: id.to_string(),
        route,
        availability: CapabilityAvailability::Available,
        exposure: HostCapabilityExposure::Visible,
        permission_domain: None,
        input_schema: json!({"type": "object"}),
        input_normalization: Default::default(),
        output_policy: HostOutputPolicy {
            preview_char_limit: 4_000,
            persist_full_output: true,
            allow_artifacts: true,
        },
        supports_streaming: id == "model.stream",
        supports_argument_streaming: false,
        parallelism: HostCapabilityParallelism::Serial,
        cancellation: HostCancellationMode::Cooperative,
        replay_safety: HostReplaySafety::Recoverable,
    }
}

fn core(id: &str) -> AgentCore {
    core_with_items(id, Vec::new())
}

fn core_with_items(id: &str, initial_items: Vec<AgentItem>) -> AgentCore {
    let mut capabilities = BTreeMap::new();
    for (name, route) in [("model.stream", HostCapabilityRoute::Model)] {
        capabilities.insert(name.to_string(), capability(name, route));
    }
    if !initial_items.is_empty() {
        capabilities.insert("context.compact".into(),
            capability("context.compact", HostCapabilityRoute::Compaction));
    }
    AgentCore::try_new(AgentCoreConfig {
        turn_identity: model_identity::host_identity(),
        session_id: Some(SessionId::from_string(id.to_string())),
        initial_items,
        host_catalog: Some(HostCapabilityCatalog {
            host_id: HostId("origin-contract-host".to_string()),
            revision: 1,
            availability_epoch: 1,
            lease: None,
            capabilities,
        }),
        ..AgentCoreConfig::default()
    })
    .expect("create Core")
}

fn model_snapshot(transition: &nola_core::agent::AgentTransition) -> AgentTurnContextSnapshot {
    let request = transition
        .host_requests
        .iter()
        .find(|request| matches!(request.payload, HostRequest::ModelStream { .. }))
        .expect("model request");
    let HostRequest::ModelStream { settings, .. } = &request.payload else {
        unreachable!()
    };
    serde_json::from_value(settings["step_context"]["turn_context"].clone())
        .expect("typed turn context snapshot")
}

fn model_messages(transition: &nola_core::agent::AgentTransition) -> &[serde_json::Value] {
    let request = transition
        .host_requests
        .iter()
        .find(|request| matches!(request.payload, HostRequest::ModelStream { .. }))
        .expect("model request");
    let HostRequest::ModelStream { messages, .. } = &request.payload else {
        unreachable!()
    };
    messages
}

#[test]
fn interactive_and_scheduled_inputs_have_distinct_authority() {
    let mut interactive = core("interactive-origin-session");
    let transition = interactive
        .submit(AgentSubmission::UserInput {
            items: vec![AgentItem::user_message(
                ItemId::from_string("interactive-message".to_string()),
                "Run this now",
            )],
            turn_context_items: Vec::new(),
            turn_context_platform: Default::default(),
            turn_context_components: Vec::new(),
            submission_id: Some("interactive-submission".to_string()),
            submitted_at_epoch_ms: None,
            delivery: AgentInputDelivery::CurrentTurn,
        })
        .expect("submit interactive input");
    let interactive_origin = model_snapshot(&transition).input_origin;
    assert_eq!(interactive_origin, AgentInputOrigin::InteractiveUser);
    assert!(interactive_origin.carries_live_user_authority());

    let mut scheduled = core("scheduled-origin-session");
    let transition = scheduled
        .submit(AgentSubmission::AutomatedInput {
            origin: AgentInputOrigin::ScheduledAction,
            items: vec![AgentItem::user_message(
                ItemId::from_string("scheduled-message".to_string()),
                "Stored task prompt",
            )],
            turn_context_items: Vec::new(),
            turn_context_platform: Default::default(),
            turn_context_components: Vec::new(),
            submission_id: Some("scheduled-submission".to_string()),
            submitted_at_epoch_ms: None,
        })
        .expect("submit scheduled input");
    let scheduled_origin = model_snapshot(&transition).input_origin;
    assert_eq!(scheduled_origin, AgentInputOrigin::ScheduledAction);
    assert!(!scheduled_origin.carries_live_user_authority());
}

#[test]
fn automated_input_rejects_interactive_origin_and_active_turn_merging() {
    let mut invalid = core("invalid-origin-session");
    let error = invalid
        .submit(AgentSubmission::AutomatedInput {
            origin: AgentInputOrigin::InteractiveUser,
            items: vec![AgentItem::user_message(
                ItemId::from_string("invalid-message".to_string()),
                "Not actually automated",
            )],
            turn_context_items: Vec::new(),
            turn_context_platform: Default::default(),
            turn_context_components: Vec::new(),
            submission_id: None,
            submitted_at_epoch_ms: None,
        })
        .expect_err("interactive origin must not enter through automated input");
    assert!(error.contains("automated input requires"));

    let mut active = core("active-origin-session");
    active
        .submit(AgentSubmission::UserInput {
            items: vec![AgentItem::user_message(
                ItemId::from_string("active-message".to_string()),
                "Keep this turn active",
            )],
            turn_context_items: Vec::new(),
            turn_context_platform: Default::default(),
            turn_context_components: Vec::new(),
            submission_id: None,
            submitted_at_epoch_ms: None,
            delivery: AgentInputDelivery::CurrentTurn,
        })
        .expect("start active turn");
    let error = active
        .submit(AgentSubmission::AutomatedInput {
            origin: AgentInputOrigin::ScheduledAction,
            items: vec![AgentItem::user_message(
                ItemId::from_string("queued-scheduled-message".to_string()),
                "Do not merge this stored prompt",
            )],
            turn_context_items: Vec::new(),
            turn_context_platform: Default::default(),
            turn_context_components: Vec::new(),
            submission_id: None,
            submitted_at_epoch_ms: None,
        })
        .expect_err("scheduled input must not merge into active turn");
    assert!(error.contains("cannot be merged"));
}

#[test]
fn goal_continuation_uses_context_without_live_user_authority() {
    let mut goal = core("goal-continuation-origin-session");
    start_trigger_goal(&mut goal);
    let transition = goal
        .submit(AgentSubmission::AutomatedInput {
            origin: AgentInputOrigin::GoalContinuation,
            items: Vec::new(),
            turn_context_items: vec![AgentItem::user_message(
                ItemId::from_string("goal-runtime-context".to_string()),
                "Continue the active durable goal",
            )],
            turn_context_platform: Default::default(),
            turn_context_components: Vec::new(),
            submission_id: Some("goal-continuation-submission".to_string()),
            submitted_at_epoch_ms: None,
        })
        .expect("submit goal continuation");

    let origin = model_snapshot(&transition).input_origin;
    assert_eq!(origin, AgentInputOrigin::GoalContinuation);
    assert!(!origin.carries_live_user_authority());
}

fn start_trigger_goal(core: &mut AgentCore) {
    core.submit(AgentSubmission::GoalRuntime {
        envelope: GoalRuntimeCommandEnvelope {
            operation_id: "start-trigger-goal".into(),
            command: GoalRuntimeCommand::Start {
                id: "trigger-goal".into(), objective: "Continue the formally owned objective".into(),
                constraints: vec![],
                requirements: vec![GoalRequirement {
                    acceptance_policy: nola_core::agent::goal::GoalAcceptancePolicy::ModelJudgment,
                    semantic_revision: 1, id: "trigger-requirement".into(),
                    description: "Preserve the current Goal source".into(),
                    dependencies: vec![], status: GoalRecordStatus::Pending, evidence_refs: vec![],
                }],
                now_epoch_ms: 1,
            },
        },
    }).unwrap();
}

#[test]
fn context_only_automated_origins_apply_current_trigger_sources_without_user_authority() {
    use nola_core::agent::protocol::{AgentEvent, AgentRole, InternalTurnTrigger};
    for origin in [AgentInputOrigin::ScheduledAction, AgentInputOrigin::Replay,
        AgentInputOrigin::SystemRecovery, AgentInputOrigin::DelegatedAgent,
        AgentInputOrigin::GoalAudit, AgentInputOrigin::GoalContinuation] {
        let mut runtime = core(&format!("context-trigger-{origin:?}"));
        if origin == AgentInputOrigin::GoalContinuation { start_trigger_goal(&mut runtime); }
        let transition = runtime.submit(AgentSubmission::AutomatedInput {
            origin, items: vec![],
            turn_context_items: vec![AgentItem::Message {
                id: ItemId::from_string("actual-context".into()),
                role: AgentRole::Developer, content: "Recorded runtime context; no new user intent".into(),
            }],
            turn_context_platform: Default::default(), turn_context_components: vec![],
            submission_id: Some("context-trigger-run".into()), submitted_at_epoch_ms: None,
        }).unwrap();
        assert_eq!(model_snapshot(&transition).input_origin, origin);
        assert!(!origin.carries_live_user_authority());
        let applied = transition.event_envelopes.iter().find_map(|envelope| match &envelope.event {
            AgentEvent::InputApplied { applications, items, .. } => Some((applications, items)),
            _ => None,
        }).expect("the accepted context-only batch must actually apply");
        assert_eq!(applied.0.len(), 1);
        assert_eq!(applied.1.len(), 1);
        let trigger = InternalTurnTrigger::from_item(&applied.1[0]).unwrap();
        assert_eq!(trigger.origin, origin);
        assert_eq!(trigger.trigger_id, "context-trigger-run:internal-trigger");
        assert!(transition.event_envelopes.iter().any(|envelope| matches!(&envelope.event,
            AgentEvent::RequirementScopeChanged { scope, .. }
                if scope.source_input_refs == vec![trigger.trigger_id.clone()]
                && scope.applied_input_refs.len() == 1)));
        assert_eq!(runtime.control_snapshot().permission_projection_revision, 0,
            "an internal trigger is not a permission approval");
        let mut empty = core("empty-automated-context");
        let before = empty.control_snapshot();
        assert!(empty.submit(AgentSubmission::AutomatedInput {
            origin, items: vec![], turn_context_items: vec![],
            turn_context_platform: Default::default(), turn_context_components: vec![],
            submission_id: Some("empty-trigger".into()), submitted_at_epoch_ms: None,
        }).is_err(), "an origin alone cannot fabricate an input source");
        assert_eq!(empty.control_snapshot().revision, before.revision);
    }
}

#[test]
fn goal_trigger_rejects_missing_goal_and_foreign_or_substituted_current_facts() {
    use nola_core::agent::protocol::{internal_turn_context_digest, GoalTriggerBinding, InternalTurnTrigger};
    let context = vec![AgentItem::user_message(ItemId::from_string("context".into()), "Continue")];
    let input = |items| AgentSubmission::AutomatedInput {
        origin: AgentInputOrigin::GoalContinuation, items, turn_context_items: context.clone(),
        turn_context_platform: Default::default(), turn_context_components: vec![],
        submission_id: Some("trigger-run".into()), submitted_at_epoch_ms: None,
    };
    let mut missing = core("missing-goal");
    assert!(missing.submit(input(vec![])).unwrap_err().contains("current active Goal"));
    for mutation in 0..4 {
        let mut runtime = core("trigger-owner");
        start_trigger_goal(&mut runtime);
        let mut trigger = InternalTurnTrigger {
            trigger_id: "trigger-run:internal-trigger".into(),
            core_session_id: "trigger-owner".into(), conversation_id: Some("conversation".into()),
            origin: AgentInputOrigin::GoalContinuation,
            context_digest: internal_turn_context_digest(&context, &Default::default(), &[]),
            goal: Some(GoalTriggerBinding { goal_id: "trigger-goal".into(), goal_revision: 1 }),
        };
        match mutation {
            0 => trigger.core_session_id = "foreign-owner".into(),
            1 => trigger.goal.as_mut().unwrap().goal_id = "foreign-goal".into(),
            2 => trigger.goal.as_mut().unwrap().goal_revision = 2,
            _ => trigger.context_digest = "a".repeat(64),
        }
        let before = runtime.control_snapshot();
        assert!(runtime.submit(input(vec![trigger.item().unwrap()])).is_err());
        let after = runtime.control_snapshot();
        assert_eq!((after.revision, after.last_event_cursor, after.pending_host_effect_count),
            (before.revision, before.last_event_cursor, before.pending_host_effect_count));
    }
}

#[test]
fn maintenance_keeps_history_without_inventing_an_ordinary_requirement_scope() {
    let mut runtime = core_with_items("maintenance-context", (0..20).map(|index|
        AgentItem::user_message(ItemId::from_string(format!("historical-{index}")),
            "Historical context that is not newly accepted input. ".repeat(100))).collect());
    let transition = runtime.submit(AgentSubmission::Compact {
        operation_id: "maintenance-operation".into(),
    }).unwrap();
    assert!(transition.host_requests.iter().any(|request|
        matches!(request.payload, HostRequest::Compact { .. })),
        "the regression must enter actual maintenance, not merely reject an empty request");
    assert!(!transition.event_envelopes.iter().any(|envelope| matches!(&envelope.event,
        nola_core::agent::protocol::AgentEvent::RequirementScopeChanged { .. })));
}

#[test]
fn explicit_goal_user_input_is_visible_and_carries_live_authority() {
    let mut goal = core("goal-interactive-origin-session");
    let transition = goal
        .submit(AgentSubmission::GoalUserInput {
            items: vec![AgentItem::user_message(
                ItemId::from_string("goal-interactive-message".to_string()),
                "Create the requested durable result",
            )],
            turn_context_items: Vec::new(),
            turn_context_platform: Default::default(),
            turn_context_components: Vec::new(),
            submission_id: Some("goal-interactive-submission".to_string()),
            submitted_at_epoch_ms: None,
            delivery: AgentInputDelivery::CurrentTurn,
        })
        .expect("submit explicit goal user input");

    let origin = model_snapshot(&transition).input_origin;
    assert_eq!(origin, AgentInputOrigin::GoalInteractiveUser);
    assert!(origin.carries_live_user_authority());
    assert!(origin.accepts_conversation_permission_grants());
    assert!(!origin.is_automated());
}

#[test]
fn cold_restored_goal_continuation_reprojects_authoritative_objective() {
    let config = {
        let mut capabilities = BTreeMap::new();
        capabilities.insert(
            "model.stream".to_string(),
            capability("model.stream", HostCapabilityRoute::Model),
        );
        AgentCoreConfig {
            turn_identity: model_identity::host_identity(),
            session_id: Some(SessionId::from_string(
                "goal-restored-context-session".to_string(),
            )),
            host_catalog: Some(HostCapabilityCatalog {
                host_id: HostId("origin-contract-host".to_string()),
                revision: 1,
                availability_epoch: 1,
                lease: None,
                capabilities,
            }),
            ..AgentCoreConfig::default()
        }
    };
    let mut original = AgentCore::try_new(config.clone()).expect("create Core");
    original
        .submit(AgentSubmission::GoalRuntime {
            envelope: GoalRuntimeCommandEnvelope {
                operation_id: "start-restorable-goal".to_string(),
                command: GoalRuntimeCommand::Start {
                    id: "goal-restored".to_string(),
                    objective: "Preserve the exact durable objective after compaction".to_string(),
                    constraints: vec!["Never treat a wait timeout as completion".to_string()],
                    requirements: vec![GoalRequirement {
                        acceptance_policy: nola_core::agent::goal::GoalAcceptancePolicy::ModelJudgment,
                        semantic_revision: 1, id: "verified-terminal-state".to_string(),
                        description: "Observe the authoritative terminal state".to_string(),
                        dependencies: Vec::new(),
                        status: GoalRecordStatus::Pending,
                        evidence_refs: Vec::new(),
                    }],
                    now_epoch_ms: 1,
                },
            },
        })
        .expect("persist Goal");
    let checkpoint = original.checkpoint(CheckpointReason::GoalUpdated);
    let mut restored = AgentCore::restore(config, checkpoint).expect("restore Goal checkpoint");

    let transition = restored
        .submit(AgentSubmission::AutomatedInput {
            origin: AgentInputOrigin::GoalContinuation,
            items: Vec::new(),
            turn_context_items: vec![AgentItem::user_message(
                ItemId::from_string("ephemeral-continuation-context".to_string()),
                "Continue",
            )],
            turn_context_platform: Default::default(),
            turn_context_components: Vec::new(),
            submission_id: Some("restored-goal-continuation".to_string()),
            submitted_at_epoch_ms: None,
        })
        .expect("continue restored Goal");

    let goal_context_message = model_messages(&transition)
        .iter()
        .find(|message| {
            message
                .get("content")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|content| content.contains("<durable_goal_state"))
        })
        .expect("authoritative durable Goal context");
    assert_eq!(goal_context_message["role"], "user");
    let goal_context = goal_context_message["content"]
        .as_str()
        .expect("Goal context text");
    let goal_json = goal_context.lines().find(|line| line.starts_with('{'))
        .expect("Goal JSON remains a complete record");
    let _: serde_json::Value = serde_json::from_str(goal_json).expect("valid Goal JSON");
    assert!(goal_context.contains("Preserve the exact durable objective after compaction"));
    assert!(goal_context.contains("Never treat a wait timeout as completion"));
    assert!(goal_context.contains("verified-terminal-state"));
}
