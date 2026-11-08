// SPDX-License-Identifier: Apache-2.0
//! No shell command is executed: these are concrete scope-admission facts.
use nola_core::agent::permissions::{
    conversation_permission_rule, PermissionPolicyDecision, PermissionRule, PermissionRuleSet,
    PermissionRuleSource, PermissionSubject,
};
use serde_json::json;

#[test]
fn script_scope_does_not_authorize_double_quoted_expansion() {
    let (capability, scope) = conversation_permission_rule(
        "bash",
        json!({
            "command":"python3 review.py --input first.txt", "cwd":"/workspace"
        })
        .to_string(),
    );
    let rules = PermissionRuleSet {
        rules: vec![PermissionRule {
            capability_pattern: capability,
            resource_pattern: scope,
            decision: PermissionPolicyDecision::Allow,
            source: PermissionRuleSource::Conversation,
        }],
    };
    for (argument, allowed) in [
        (r#""plain text""#, true),
        (r#"'$(other_command)'"#, true),
        (r#"'`other_command`'"#, true),
        (r#""\$(literal)""#, true),
        (r#""\`literal\`""#, true),
        (r#""$(other_command)""#, false),
        (r#""`other_command`""#, false),
        (r#""${expanded_variable}""#, false),
        (r#""prefix$(other_command)suffix""#, false),
        (r#""\\$(other_command)""#, false),
        (r#"'safe'"$(other_command)""#, false),
    ] {
        let command = format!("python3 review.py --input {argument}");
        let decision = rules.evaluate(&PermissionSubject {
            capability: "bash".into(),
            resource: json!({"command":command,"cwd":"/workspace"}).to_string(),
        });
        assert_eq!(
            decision,
            if allowed {
                PermissionPolicyDecision::Allow
            } else {
                PermissionPolicyDecision::Ask
            },
            "{argument}"
        );
    }
}

#[test]
fn expansion_cannot_create_a_reusable_script_scope() {
    for command in [
        r#"python3 review.py "$(other_command)""#,
        r#"python3 review.py "`other_command`""#,
    ] {
        let original = json!({"command":command,"cwd":"/workspace"}).to_string();
        let (_, scope) = conversation_permission_rule("bash", original.clone());
        assert_eq!(
            scope, original,
            "unparsed invocation must remain exact, never broaden to script prefix"
        );
    }
}

#[test]
fn quoted_backslash_cannot_change_the_authorized_script_identity() {
    let (_, scope) = conversation_permission_rule(
        "bash",
        json!({
            "command":"python3 review.py", "cwd":"/workspace"
        })
        .to_string(),
    );
    let rules = PermissionRuleSet {
        rules: vec![PermissionRule {
            capability_pattern: "bash".into(),
            resource_pattern: scope,
            decision: PermissionPolicyDecision::Allow,
            source: PermissionRuleSource::Conversation,
        }],
    };
    for (command, allowed) in [
        (r#"python3 "review.py""#, true),
        (r#"python3 review\.py"#, true),
        (r#"python3 "review\.py""#, false),
        (r#"python3 'review\.py'"#, false),
    ] {
        let decision = rules.evaluate(&PermissionSubject {
            capability: "bash".into(),
            resource: json!({"command":command,"cwd":"/workspace"}).to_string(),
        });
        assert_eq!(
            decision,
            if allowed {
                PermissionPolicyDecision::Allow
            } else {
                PermissionPolicyDecision::Ask
            },
            "{command}"
        );
    }
}
