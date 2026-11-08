// SPDX-License-Identifier: Apache-2.0

use nola_core::agent::{
    AgentInputOrigin, PermissionEvaluationReason, PermissionPolicyDecision, PermissionRule,
    PermissionRuleSet, PermissionRuleSource, PermissionSubject,
};
use serde_json::json;

fn rule(decision: PermissionPolicyDecision, source: PermissionRuleSource) -> PermissionRule {
    PermissionRule {
        capability_pattern: "delete_file".to_string(),
        resource_pattern: if source == PermissionRuleSource::Conversation {
            delete_subject().resource
        } else { "*".into() },
        decision,
        source,
    }
}

fn delete_subject() -> PermissionSubject {
    PermissionSubject {
        capability: "delete_file".to_string(),
        resource: json!({"path":"notes.md","dry_run":false}).to_string(),
    }
}

#[test]
fn evaluation_explains_product_confirmation_and_preserves_the_matched_rule() {
    let product_rule = rule(
        PermissionPolicyDecision::Allow,
        PermissionRuleSource::ProductDefault,
    );
    let evaluation = PermissionRuleSet {
        rules: vec![product_rule.clone()],
    }
    .evaluate_with_explanation(&delete_subject());

    assert_eq!(evaluation.decision, PermissionPolicyDecision::Ask);
    assert_eq!(
        evaluation.reason,
        PermissionEvaluationReason::CautiousProfileRequiresConfirmation
    );
    assert_eq!(evaluation.rule_index, Some(0));
    assert_eq!(evaluation.rule, Some(product_rule));
}

#[test]
fn scheduled_work_cannot_reuse_an_interactive_conversation_grant() {
    let conversation_rule = rule(
        PermissionPolicyDecision::Allow,
        PermissionRuleSource::Conversation,
    );
    let rules = PermissionRuleSet {
        rules: vec![conversation_rule.clone()],
    };

    let interactive = rules
        .evaluate_for_origin_with_explanation(&delete_subject(), AgentInputOrigin::InteractiveUser);
    assert_eq!(interactive.decision, PermissionPolicyDecision::Allow);
    assert_eq!(interactive.reason, PermissionEvaluationReason::MatchingRule);

    let scheduled = rules
        .evaluate_for_origin_with_explanation(&delete_subject(), AgentInputOrigin::ScheduledAction);
    assert_eq!(scheduled.decision, PermissionPolicyDecision::Ask);
    assert_eq!(
        scheduled.reason,
        PermissionEvaluationReason::InteractiveGrantUnavailableForOrigin
    );
    assert_eq!(scheduled.rule, Some(conversation_rule));
}

#[test]
fn evaluation_distinguishes_absent_policy_from_an_explicit_ask_rule() {
    let subject = PermissionSubject {
        capability: "custom_tool".to_string(),
        resource: "resource-1".to_string(),
    };
    let empty = PermissionRuleSet::default().evaluate_with_explanation(&subject);
    assert_eq!(empty.decision, PermissionPolicyDecision::Ask);
    assert_eq!(empty.reason, PermissionEvaluationReason::NoMatchingRule);
    assert!(empty.rule.is_none());

    let explicit = PermissionRule {
        capability_pattern: "custom_*".to_string(),
        resource_pattern: "resource-*".to_string(),
        decision: PermissionPolicyDecision::Ask,
        source: PermissionRuleSource::UserGlobal,
    };
    let evaluated = PermissionRuleSet {
        rules: vec![explicit.clone()],
    }
    .evaluate_with_explanation(&subject);
    assert_eq!(evaluated.reason, PermissionEvaluationReason::MatchingRule);
    assert_eq!(evaluated.rule, Some(explicit));
}

#[test]
fn admitted_allow_all_overrides_host_asks_for_every_input_origin_but_not_denials() {
    let mut rules = PermissionRuleSet::default().with_profile(nola_domain::PermissionProfile::AllowAll);
    rules.rules.push(rule(PermissionPolicyDecision::Ask, PermissionRuleSource::Host));
    for origin in [AgentInputOrigin::InteractiveUser, AgentInputOrigin::ScheduledAction] {
        assert_eq!(rules.evaluate_for_origin(&delete_subject(), origin), PermissionPolicyDecision::Allow);
    }
    assert_eq!(rules.profile(), nola_domain::PermissionProfile::AllowAll);
    rules.rules.push(rule(PermissionPolicyDecision::Deny, PermissionRuleSource::Host));
    assert_eq!(rules.evaluate(&delete_subject()), PermissionPolicyDecision::Deny);
}

#[test]
fn os_granted_sensitive_capability_is_not_application_consent() {
    let mut rules = PermissionRuleSet::default().with_profile(nola_domain::PermissionProfile::Cautious);
    rules.rules.push(rule(PermissionPolicyDecision::Allow, PermissionRuleSource::Host));
    assert_eq!(rules.evaluate(&delete_subject()), PermissionPolicyDecision::Ask);
    assert_eq!(rules.profile(), nola_domain::PermissionProfile::Cautious);
    rules.rules.push(rule(PermissionPolicyDecision::Allow, PermissionRuleSource::Conversation));
    assert_eq!(rules.evaluate(&delete_subject()), PermissionPolicyDecision::Allow);
}

#[test]
fn policy_downgrade_retains_unknown_capability_confirmation_rules() {
    let subject = PermissionSubject { capability:"third_party_action".into(),resource:"resource".into() };
    let rules = PermissionRuleSet { rules:vec![PermissionRule {
        capability_pattern: subject.capability.clone(),resource_pattern:"*".into(),
        source:PermissionRuleSource::Host,decision:PermissionPolicyDecision::Ask,
    }] }.with_profile(nola_domain::PermissionProfile::AllowAll);
    assert_eq!(rules.evaluate(&subject),PermissionPolicyDecision::Allow);
    let cautious = rules.with_profile(nola_domain::PermissionProfile::Cautious);
    assert_eq!(cautious.evaluate(&subject),PermissionPolicyDecision::Ask);
    assert_eq!(cautious.profile(),nola_domain::PermissionProfile::Cautious);
}
