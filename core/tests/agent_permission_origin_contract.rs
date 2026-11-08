use nola_core::agent::permissions::{
    PermissionPolicyDecision, PermissionRule, PermissionRuleSet, PermissionRuleSource,
    PermissionSubject,
};
use nola_core::agent::AgentInputOrigin;

fn rule(source: PermissionRuleSource, decision: PermissionPolicyDecision) -> PermissionRule {
    PermissionRule {
        capability_pattern: "write_file".to_string(),
        resource_pattern: if source == PermissionRuleSource::Conversation {
            subject().resource
        } else { "*".into() },
        decision,
        source,
    }
}

fn subject() -> PermissionSubject {
    PermissionSubject {
        capability: "write_file".to_string(),
        resource: "/tmp/report.md".to_string(),
    }
}

#[test]
fn scheduled_input_does_not_inherit_conversation_or_turn_grants() {
    let rules = PermissionRuleSet {
        rules: vec![
            rule(
                PermissionRuleSource::Conversation,
                PermissionPolicyDecision::Allow,
            ),
            rule(PermissionRuleSource::Turn, PermissionPolicyDecision::Allow),
        ],
    };

    assert_eq!(
        rules.evaluate_for_origin(&subject(), AgentInputOrigin::InteractiveUser),
        PermissionPolicyDecision::Allow
    );
    assert_eq!(
        rules.evaluate_for_origin(&subject(), AgentInputOrigin::ScheduledAction),
        PermissionPolicyDecision::Ask
    );
}

#[test]
fn scheduled_input_respects_explicit_durable_policy_and_trusted_operation_risk() {
    for source in [
        PermissionRuleSource::ProductDefault,
        PermissionRuleSource::UserGlobal,
        PermissionRuleSource::Host,
    ] {
        let rules = PermissionRuleSet {
            rules: vec![rule(source, PermissionPolicyDecision::Allow)],
        };
        assert_eq!(
            rules.evaluate_for_origin(&subject(), AgentInputOrigin::ScheduledAction),
            if source == PermissionRuleSource::UserGlobal {PermissionPolicyDecision::Allow}
                else {PermissionPolicyDecision::Ask},
            "opaque operations cannot use a product/Host Allow as application consent"
        );
        assert_eq!(rules.evaluate_resolved_for_origin_with_explanation(&subject(),
            AgentInputOrigin::ScheduledAction, false).decision, PermissionPolicyDecision::Allow,
            "a trusted ordinary operation does not need an interactive window");
    }
}
