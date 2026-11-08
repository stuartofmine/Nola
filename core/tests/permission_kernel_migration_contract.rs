// SPDX-License-Identifier: Apache-2.0
//! Public Core permission API contracts retained across the Kernel cutover.

use nola_core::agent::permissions::{conversation_permission_rule, permission_resource_for_input};
use nola_core::agent::{
    AgentInputOrigin as Origin, PermissionEvaluation, PermissionEvaluationReason as Reason,
    PermissionPolicyDecision as Decision, PermissionRule, PermissionRuleSet,
    PermissionRuleSource as Source, PermissionSubject,
};
use serde_json::json;

fn rule(source: Source, decision: Decision, resource: &str) -> PermissionRule {
    PermissionRule {
        capability_pattern: "custom_capability".into(),
        resource_pattern: resource.into(),
        decision,
        source,
    }
}

#[test]
fn every_origin_and_rule_source_preserves_the_complete_explanation() {
    let subject = PermissionSubject {
        capability: "custom_capability".into(),
        resource: "resource".into(),
    };
    for (origin, conversation, turn) in [
        (Origin::InteractiveUser, true, true),
        (Origin::GoalInteractiveUser, true, true),
        (Origin::DelegatedAgent, true, false),
        (Origin::ScheduledAction, false, false),
        (Origin::Replay, false, false),
        (Origin::SystemRecovery, false, false),
        (Origin::Unknown, false, false),
        (Origin::GoalContinuation, false, false),
        (Origin::GoalAudit, false, false),
        (Origin::Maintenance, false, false),
    ] {
        for source in [
            Source::ProductDefault,
            Source::UserGlobal,
            Source::Host,
            Source::Conversation,
            Source::Turn,
        ] {
            for decision in [Decision::Allow, Decision::Ask, Decision::Deny] {
                let candidate = rule(source, decision, "resource");
                let rules = PermissionRuleSet {
                    rules: vec![
                        rule(Source::UserGlobal, Decision::Deny, "unrelated"),
                        candidate.clone(),
                    ],
                };
                let eligible = match source {
                    Source::Conversation => conversation,
                    Source::Turn => turn,
                    _ => true,
                };
                let guarded = eligible && matches!(source, Source::ProductDefault | Source::Host)
                    && decision == Decision::Allow;
                assert_eq!(
                    rules.evaluate_for_origin_with_explanation(&subject, origin),
                    PermissionEvaluation {
                        decision: if eligible && !guarded { decision } else { Decision::Ask },
                        reason: if guarded {
                            Reason::CautiousProfileRequiresConfirmation
                        } else if eligible {
                            Reason::MatchingRule
                        } else {
                            Reason::InteractiveGrantUnavailableForOrigin
                        },
                        rule_index: Some(1),
                        rule: Some(candidate),
                    },
                    "{origin:?}/{source:?}/{decision:?}"
                );
            }
        }
    }
}

#[test]
fn filtered_rules_keep_original_indices_and_do_not_override_durable_policy() {
    let subject = PermissionSubject {
        capability: "custom_capability".into(),
        resource: "resource".into(),
    };
    let rules = PermissionRuleSet {
        rules: vec![
            rule(Source::UserGlobal, Decision::Ask, "resource"),
            rule(Source::Host, Decision::Deny, "resource"),
            rule(Source::Conversation, Decision::Allow, "resource"),
            rule(Source::Turn, Decision::Ask, "resource"),
        ],
    };
    for (origin, index) in [
        (Origin::InteractiveUser, 3),
        (Origin::DelegatedAgent, 2),
        (Origin::ScheduledAction, 1),
        (Origin::Replay, 1),
        (Origin::SystemRecovery, 1),
    ] {
        assert_eq!(
            rules.evaluate_for_origin_with_explanation(&subject, origin),
            PermissionEvaluation {
                decision: rules.rules[index].decision,
                reason: Reason::MatchingRule,
                rule_index: Some(index),
                rule: Some(rules.rules[index].clone()),
            }
        );
    }
    let excluded = PermissionRuleSet {
        rules: rules.rules[2..].to_vec(),
    };
    assert_eq!(
        excluded.evaluate_for_origin_with_explanation(&subject, Origin::Replay),
        PermissionEvaluation {
            decision: Decision::Ask,
            reason: Reason::InteractiveGrantUnavailableForOrigin,
            rule_index: Some(1),
            rule: Some(excluded.rules[1].clone()),
        }
    );
    let absent = PermissionSubject {
        resource: "different".into(),
        ..subject
    };
    assert_eq!(
        rules.evaluate_for_origin_with_explanation(&absent, Origin::Replay),
        PermissionEvaluation {
            decision: Decision::Ask,
            reason: Reason::NoMatchingRule,
            rule_index: None,
            rule: None,
        }
    );
}

fn assert_scope(candidate: PermissionRule, resource: &str, matched: bool) {
    let subject = PermissionSubject {
        capability: candidate.capability_pattern.clone(),
        resource: resource.into(),
    };
    let result = PermissionRuleSet {
        rules: vec![candidate.clone()],
    }
    .evaluate_for_origin_with_explanation(&subject, Origin::InteractiveUser);
    assert_eq!(
        result,
        PermissionEvaluation {
            decision: if matched {
                Decision::Allow
            } else {
                Decision::Ask
            },
            reason: if matched {
                Reason::MatchingRule
            } else {
                Reason::NoMatchingRule
            },
            rule_index: matched.then_some(0),
            rule: matched.then_some(candidate),
        },
        "resource={resource}"
    );
}

#[test]
fn local_path_scope_uses_components_and_decodes_percent_encoding_once() {
    for capability in ["read", "edit", "files_read", "files_write"] {
        let candidate = PermissionRule {
            capability_pattern: capability.into(),
            resource_pattern: "file://localhost/work/project%20one".into(),
            decision: Decision::Allow,
            source: Source::Conversation,
        };
        for (resource, matched) in [
            ("/work/project one", true),
            ("file:///work/project%20one/子目录/file.txt", true),
            ("/work/project one-other/file.txt", false),
            ("/work/project one/../secret", false),
            ("file:///work/project%20one/%2e%2E/secret", false),
            ("file:///work/project%20one%2f..%2fsecret", false),
            ("file://remote.example/work/project%20one/file.txt", false),
            ("work/project one/file.txt", false),
            ("/work/project one/%", false),
            ("/work/project one/%GG", false),
            ("/work/project one/%ff", false),
            // %252e decodes to the literal filename %2e, not a second traversal.
            ("/work/project one/%252e%252e/file.txt", true),
        ] {
            assert_scope(candidate.clone(), resource, matched);
        }
    }
}

#[test]
fn bash_conversation_scope_binds_cwd_and_rejects_compound_prefix_reuse() {
    let resource = |command: &str, cwd: &str| {
        permission_resource_for_input(
            "bash",
            Some("bash"),
            &json!({"command": command, "cwd": cwd}),
        )
    };
    let (capability_pattern, resource_pattern) =
        conversation_permission_rule("bash", resource("python3 script.py first", "/work/project"));
    let candidate = PermissionRule {
        capability_pattern,
        resource_pattern,
        decision: Decision::Allow,
        source: Source::Conversation,
    };
    for (command, cwd, matched) in [
        ("python3 script.py second", "/work/project", true),
        ("python3 'script.py' 'second file'", "/work/project", true),
        ("python3 other.py second", "/work/project", false),
        ("python3 script.py second", "/work/project-other", false),
        ("python3 script.py second", ".", false),
        (
            "python3 script.py second; echo next",
            "/work/project",
            false,
        ),
        (
            "python3 script.py second && echo next",
            "/work/project",
            false,
        ),
        ("python3 script.py second | cat", "/work/project", false),
        ("python3 script.py second > output", "/work/project", false),
        ("python3 script.py $(echo second)", "/work/project", false),
        ("python3 script.py `echo second`", "/work/project", false),
        (
            "python3 script.py second\necho next",
            "/work/project",
            false,
        ),
        ("python3 script.py 'unfinished", "/work/project", false),
    ] {
        assert_scope(candidate.clone(), &resource(command, cwd), matched);
    }
}
