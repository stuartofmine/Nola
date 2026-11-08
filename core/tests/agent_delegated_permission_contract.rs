// SPDX-License-Identifier: Apache-2.0

use nola_core::agent::{
    AgentInputOrigin, PermissionEvaluationReason, PermissionPolicyDecision, PermissionRule,
    PermissionRuleSet, PermissionRuleSource, PermissionSubject,
};

fn subject() -> PermissionSubject {
    PermissionSubject {
        capability: "write_file".to_string(),
        resource: r#"{"path":"report.md"}"#.to_string(),
    }
}

fn rules(source: PermissionRuleSource) -> PermissionRuleSet {
    PermissionRuleSet {
        rules: vec![PermissionRule {
            capability_pattern: "write_file".to_string(),
            resource_pattern: if source == PermissionRuleSource::Conversation {
                subject().resource
            } else { "*".into() },
            decision: PermissionPolicyDecision::Allow,
            source,
        }],
    }
}

#[test]
fn delegated_agent_inherits_explicit_conversation_grants() {
    let evaluation = rules(PermissionRuleSource::Conversation)
        .evaluate_for_origin_with_explanation(&subject(), AgentInputOrigin::DelegatedAgent);
    assert_eq!(evaluation.decision, PermissionPolicyDecision::Allow);
    assert_eq!(evaluation.reason, PermissionEvaluationReason::MatchingRule);
}

#[test]
fn delegated_agent_does_not_inherit_parent_turn_grants() {
    let evaluation = rules(PermissionRuleSource::Turn)
        .evaluate_for_origin_with_explanation(&subject(), AgentInputOrigin::DelegatedAgent);
    assert_eq!(evaluation.decision, PermissionPolicyDecision::Ask);
    assert_eq!(
        evaluation.reason,
        PermissionEvaluationReason::InteractiveGrantUnavailableForOrigin
    );
}

#[test]
fn automated_work_does_not_inherit_conversation_grants() {
    for origin in [
        AgentInputOrigin::ScheduledAction,
        AgentInputOrigin::Replay,
        AgentInputOrigin::SystemRecovery,
    ] {
        let evaluation = rules(PermissionRuleSource::Conversation)
            .evaluate_for_origin_with_explanation(&subject(), origin);
        assert_eq!(evaluation.decision, PermissionPolicyDecision::Ask);
        assert_eq!(
            evaluation.reason,
            PermissionEvaluationReason::InteractiveGrantUnavailableForOrigin
        );
    }
}
