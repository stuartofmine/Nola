// SPDX-License-Identifier: Apache-2.0
//! Exercise publication consent through the real scheduler, not a policy copy.
use super::*;
use crate::agent::{
    generative_ui::capability_descriptor,
    permissions::{PermissionPolicyDecision, PermissionRule, PermissionRuleSet, PermissionRuleSource},
    protocol::ModelToolCall,
    tool_lifecycle::ToolCallState,
};

fn schedule_publication(
    rule: Option<(PermissionPolicyDecision, PermissionRuleSource)>,
    available: bool,
    exposed: bool,
) -> (ToolCallStatus, Vec<RuntimeEvent>, Vec<AgentEffect>) {
    let mut runtime = AgentRuntime::try_new(AgentCoreConfig {
        session_id: Some(SessionId::from_string("ui-consent-test".into())),
        turn_identity: crate::agent::model_capability::test_host_identity(),
        ..Default::default()
    }).unwrap();
    let mut descriptor = capability_descriptor();
    if !available { descriptor.availability = CapabilityAvailability::Unavailable; }
    let id = descriptor.id.clone();
    // Terminal denials ask the planner for the model's recoverable follow-up.
    // Admit that route as well; no model transport is executed by this test.
    let mut model = crate::agent::code_mode::capability_descriptors(&Default::default()).unwrap().remove(0);
    model.id = "model.stream".into();
    model.route = HostCapabilityRoute::Model;
    let subject = descriptor.permission_subject(&json!({})).unwrap();
    runtime.session.execution.permission_rules = PermissionRuleSet { rules: vec![PermissionRule {
        capability_pattern: "*".into(), resource_pattern: "*".into(),
        decision: PermissionPolicyDecision::Ask, source: PermissionRuleSource::ProductDefault,
    }] };
    if let Some((decision, source)) = rule {
        runtime.session.execution.permission_rules.rules.push(PermissionRule {
            capability_pattern: subject.capability, resource_pattern: "*".into(), decision, source,
        });
    }
    let turn_id = TurnId::from_string("publication-turn".into());
    let context = AgentTurnContext {
        input_origin: Default::default(), configuration: runtime.session.configuration.clone(),
        platform: Default::default(), components: vec![], component_metrics: vec![],
        component_payload_digest: String::new(), items: vec![],
        host_id: runtime.session.host.catalog.host_id.0.clone(),
        host_catalog_revision: runtime.session.host.catalog.revision,
        host_availability_epoch: runtime.session.host.catalog.availability_epoch,
        capability_projection: crate::agent::capability_projection::AgentTurnCapabilityProjection {
            host_id: runtime.session.host.catalog.host_id.0.clone(),
            host_catalog_revision: runtime.session.host.catalog.revision,
            host_availability_epoch: runtime.session.host.catalog.availability_epoch,
            available_capabilities: [(id.clone(), descriptor), (model.id.clone(), model)].into_iter().collect(),
            ..Default::default()
        }, runtime_snapshot: Default::default(),
    };
    let mut turn = AgentTurnState::new(turn_id.clone(), context);
    if exposed {
        turn.last_model_tool_projection = Some(crate::agent::capability_projection::AgentModelToolProjection {
            capability_ids: vec![id.clone()], ..Default::default()
        });
    }
    turn.tools.push(ToolCallState::from_model(ModelToolCall {
        call_id: CallId::from_string("publish".into()), name: id,
        arguments: nola_kernel::generative_ui::schema::minimal_form_example(),
        permission_scope: None, permission_rationale: None,
    }));
    runtime.session.execution.active_turn = Some(Box::new(turn));
    let (mut events, mut effects) = (vec![], vec![]);
    runtime.schedule_tools(&turn_id, &mut events, &mut effects).unwrap();
    let status = runtime.session.execution.active_turn.as_ref().unwrap().tools[0].status;
    (status, events, effects)
}

#[test]
fn publication_dispatches_once_without_a_default_consent_challenge() {
    let (status, events, effects) = schedule_publication(None, true, true);
    assert_eq!(status, ToolCallStatus::Running);
    assert!(!events.iter().any(|event| matches!(event, RuntimeEvent::PermissionRequested { .. })));
    assert!(!effects.iter().any(|effect| matches!(effect, AgentEffect::RequestPermission { .. })));
    assert_eq!(effects.iter().filter(|effect| matches!(effect, AgentEffect::ExecuteTool { tool_name, .. }
        if tool_name == "render_ui")).count(), 1);
}

#[test]
fn explicit_user_ask_and_deny_prevent_publication_dispatch() {
    for (decision, expected) in [
        (PermissionPolicyDecision::Ask, ToolCallStatus::AwaitingPermission),
        (PermissionPolicyDecision::Deny, ToolCallStatus::Denied),
    ] {
        let (status, events, effects) = schedule_publication(
            Some((decision, PermissionRuleSource::UserGlobal)), true, true);
        assert_eq!(status, expected);
        assert!(!effects.iter().any(|effect| matches!(effect, AgentEffect::ExecuteTool { .. })));
        assert_eq!(events.iter().filter(|event| matches!(event, RuntimeEvent::PermissionRequested { .. })).count(),
            usize::from(decision == PermissionPolicyDecision::Ask));
    }
}

#[test]
fn no_default_prompt_does_not_bypass_availability_or_model_exposure() {
    for (available, exposed) in [(false, true), (true, false)] {
        let (status, _, effects) = schedule_publication(None, available, exposed);
        assert_eq!(status, ToolCallStatus::Failed);
        assert!(!effects.iter().any(|effect| matches!(effect, AgentEffect::ExecuteTool { .. })));
    }
}
