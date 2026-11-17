// SPDX-License-Identifier: Apache-2.0
//! Consolidated permission origin, delegation, evaluation, projection, path/bash/quoted-expansion
//! scoping, resource shape, and protected platform data omission contracts.

use chrono::{Duration, SecondsFormat, Utc};
use nola_core::agent::permissions::{
    conversation_permission_rule, permission_resource_for_input, permission_resource_keys,
};
use nola_core::agent::{
    AgentInputOrigin, CallId, PendingPermissionRequest, PermissionEvaluation,
    PermissionEvaluationReason, PermissionPolicyDecision, PermissionProjection, PermissionRequestId,
    PermissionRule, PermissionRuleSet, PermissionRuleSource, PermissionSubject, TurnId,
};
use nola_core::context_engine::{
    prepare_context, ContextComponent, ContextComponentKind, ContextComponentSource,
    ContextPrepareInput,
};
use serde_json::{json, Value};

fn assert_scope(candidate: PermissionRule, resource: &str, matched: bool) {
    let subject = PermissionSubject {
        capability: candidate.capability_pattern.clone(),
        resource: resource.into(),
    };
    let result = PermissionRuleSet {
        rules: vec![candidate.clone()],
    }
    .evaluate_for_origin_with_explanation(&subject, AgentInputOrigin::InteractiveUser);
    assert_eq!(
        result,
        PermissionEvaluation {
            decision: if matched {
                PermissionPolicyDecision::Allow
            } else {
                PermissionPolicyDecision::Ask
            },
            reason: if matched {
                PermissionEvaluationReason::MatchingRule
            } else {
                PermissionEvaluationReason::NoMatchingRule
            },
            rule_index: matched.then_some(0),
            rule: matched.then_some(candidate),
        },
        "resource={resource}"
    );
}

#[test]
fn permission_origins_delegation_profiles_and_rule_filtering_preserve_explanations() {
    let custom_rule =
        |source: PermissionRuleSource, decision: PermissionPolicyDecision, resource: &str| {
            PermissionRule {
                capability_pattern: "custom_capability".into(),
                resource_pattern: resource.into(),
                decision,
                source,
            }
        };
    let custom_subject = PermissionSubject {
        capability: "custom_capability".into(),
        resource: "resource".into(),
    };

    // 1. Exhaustive 10-origin x 5-source x 3-decision explanation matrix.
    for (origin, conversation, turn) in [
        (AgentInputOrigin::InteractiveUser, true, true),
        (AgentInputOrigin::GoalInteractiveUser, true, true),
        (AgentInputOrigin::DelegatedAgent, true, false),
        (AgentInputOrigin::ScheduledAction, false, false),
        (AgentInputOrigin::Replay, false, false),
        (AgentInputOrigin::SystemRecovery, false, false),
        (AgentInputOrigin::Unknown, false, false),
        (AgentInputOrigin::GoalContinuation, false, false),
        (AgentInputOrigin::GoalAudit, false, false),
        (AgentInputOrigin::Maintenance, false, false),
    ] {
        for source in [
            PermissionRuleSource::ProductDefault,
            PermissionRuleSource::UserGlobal,
            PermissionRuleSource::Host,
            PermissionRuleSource::Conversation,
            PermissionRuleSource::Turn,
        ] {
            for decision in [
                PermissionPolicyDecision::Allow,
                PermissionPolicyDecision::Ask,
                PermissionPolicyDecision::Deny,
            ] {
                let candidate = custom_rule(source, decision, "resource");
                let rules = PermissionRuleSet {
                    rules: vec![
                        custom_rule(
                            PermissionRuleSource::UserGlobal,
                            PermissionPolicyDecision::Deny,
                            "unrelated",
                        ),
                        candidate.clone(),
                    ],
                };
                let eligible = match source {
                    PermissionRuleSource::Conversation => conversation,
                    PermissionRuleSource::Turn => turn,
                    _ => true,
                };
                let guarded = eligible
                    && matches!(
                        source,
                        PermissionRuleSource::ProductDefault | PermissionRuleSource::Host
                    )
                    && decision == PermissionPolicyDecision::Allow;
                assert_eq!(
                    rules.evaluate_for_origin_with_explanation(&custom_subject, origin),
                    PermissionEvaluation {
                        decision: if eligible && !guarded {
                            decision
                        } else {
                            PermissionPolicyDecision::Ask
                        },
                        reason: if guarded {
                            PermissionEvaluationReason::CautiousProfileRequiresConfirmation
                        } else if eligible {
                            PermissionEvaluationReason::MatchingRule
                        } else {
                            PermissionEvaluationReason::InteractiveGrantUnavailableForOrigin
                        },
                        rule_index: Some(1),
                        rule: Some(candidate),
                    }
                );
            }
        }
    }

    // 2. Filtered rule index preservation + absent vs explicit Ask rule.
    let layered = PermissionRuleSet {
        rules: vec![
            custom_rule(
                PermissionRuleSource::UserGlobal,
                PermissionPolicyDecision::Ask,
                "resource",
            ),
            custom_rule(
                PermissionRuleSource::Host,
                PermissionPolicyDecision::Deny,
                "resource",
            ),
            custom_rule(
                PermissionRuleSource::Conversation,
                PermissionPolicyDecision::Allow,
                "resource",
            ),
            custom_rule(
                PermissionRuleSource::Turn,
                PermissionPolicyDecision::Ask,
                "resource",
            ),
        ],
    };
    for (origin, index) in [
        (AgentInputOrigin::InteractiveUser, 3),
        (AgentInputOrigin::DelegatedAgent, 2),
        (AgentInputOrigin::ScheduledAction, 1),
        (AgentInputOrigin::Replay, 1),
        (AgentInputOrigin::SystemRecovery, 1),
    ] {
        assert_eq!(
            layered.evaluate_for_origin_with_explanation(&custom_subject, origin),
            PermissionEvaluation {
                decision: layered.rules[index].decision,
                reason: PermissionEvaluationReason::MatchingRule,
                rule_index: Some(index),
                rule: Some(layered.rules[index].clone()),
            }
        );
    }
    let excluded = PermissionRuleSet {
        rules: layered.rules[2..].to_vec(),
    };
    assert_eq!(
        excluded.evaluate_for_origin_with_explanation(&custom_subject, AgentInputOrigin::Replay),
        PermissionEvaluation {
            decision: PermissionPolicyDecision::Ask,
            reason: PermissionEvaluationReason::InteractiveGrantUnavailableForOrigin,
            rule_index: Some(1),
            rule: Some(excluded.rules[1].clone()),
        }
    );
    let empty_eval = PermissionRuleSet::default().evaluate_with_explanation(&custom_subject);
    assert_eq!(empty_eval.decision, PermissionPolicyDecision::Ask);
    assert_eq!(empty_eval.reason, PermissionEvaluationReason::NoMatchingRule);

    // 3. Delegated inheritance, scheduled durable policy vs trusted ordinary risk, AllowAll vs Cautious downgrade.
    let write_subject = PermissionSubject {
        capability: "write_file".into(),
        resource: r#"{"path":"report.md"}"#.into(),
    };
    let conv_grant = PermissionRuleSet {
        rules: vec![PermissionRule {
            capability_pattern: "write_file".into(),
            resource_pattern: write_subject.resource.clone(),
            decision: PermissionPolicyDecision::Allow,
            source: PermissionRuleSource::Conversation,
        }],
    };
    assert_eq!(
        conv_grant
            .evaluate_for_origin_with_explanation(&write_subject, AgentInputOrigin::DelegatedAgent)
            .decision,
        PermissionPolicyDecision::Allow
    );
    for source in [
        PermissionRuleSource::ProductDefault,
        PermissionRuleSource::UserGlobal,
        PermissionRuleSource::Host,
    ] {
        let rules = PermissionRuleSet {
            rules: vec![PermissionRule {
                capability_pattern: "write_file".into(),
                resource_pattern: "*".into(),
                decision: PermissionPolicyDecision::Allow,
                source,
            }],
        };
        assert_eq!(
            rules.evaluate_for_origin(&write_subject, AgentInputOrigin::ScheduledAction),
            if source == PermissionRuleSource::UserGlobal {
                PermissionPolicyDecision::Allow
            } else {
                PermissionPolicyDecision::Ask
            }
        );
        assert_eq!(
            rules
                .evaluate_resolved_for_origin_with_explanation(
                    &write_subject,
                    AgentInputOrigin::ScheduledAction,
                    false
                )
                .decision,
            PermissionPolicyDecision::Allow
        );
    }

    let delete_subject = PermissionSubject {
        capability: "delete_file".into(),
        resource: json!({"path":"notes.md","dry_run":false}).to_string(),
    };
    let delete_rule = |decision, source| PermissionRule {
        capability_pattern: "delete_file".into(),
        resource_pattern: if source == PermissionRuleSource::Conversation {
            delete_subject.resource.clone()
        } else {
            "*".into()
        },
        decision,
        source,
    };
    let mut allow_all =
        PermissionRuleSet::default().with_profile(nola_domain::PermissionProfile::AllowAll);
    allow_all.rules.push(delete_rule(
        PermissionPolicyDecision::Ask,
        PermissionRuleSource::Host,
    ));
    for origin in [
        AgentInputOrigin::InteractiveUser,
        AgentInputOrigin::ScheduledAction,
    ] {
        assert_eq!(
            allow_all.evaluate_for_origin(&delete_subject, origin),
            PermissionPolicyDecision::Allow
        );
    }
    let downgraded = allow_all
        .clone()
        .with_profile(nola_domain::PermissionProfile::Cautious);
    assert_eq!(
        downgraded.evaluate(&delete_subject),
        PermissionPolicyDecision::Ask
    );
    allow_all.rules.push(delete_rule(
        PermissionPolicyDecision::Deny,
        PermissionRuleSource::Host,
    ));
    assert_eq!(
        allow_all.evaluate(&delete_subject),
        PermissionPolicyDecision::Deny
    );

    let mut os_only =
        PermissionRuleSet::default().with_profile(nola_domain::PermissionProfile::Cautious);
    os_only.rules.push(delete_rule(
        PermissionPolicyDecision::Allow,
        PermissionRuleSource::Host,
    ));
    assert_eq!(
        os_only.evaluate(&delete_subject),
        PermissionPolicyDecision::Ask
    );
    os_only.rules.push(delete_rule(
        PermissionPolicyDecision::Allow,
        PermissionRuleSource::Conversation,
    ));
    assert_eq!(
        os_only.evaluate(&delete_subject),
        PermissionPolicyDecision::Allow
    );

    // 4. PermissionProjection idempotent upsert, serialization roundtrip, and turn removal.
    let make_req = |id: &str, turn: &str| PendingPermissionRequest {
        allow_conversation: true,
        permission_instructions: Vec::new(),
        request_id: PermissionRequestId::from_string(id.to_string()),
        turn_id: TurnId::from_string(turn.to_string()),
        call_id: CallId::from_string(format!("call-{id}")),
        capability_id: "write_file".into(),
        resource: json!({"path":"/tmp/report.md"}).to_string(),
        arguments: json!({"path":"/tmp/report.md","content":"report"}),
        invocation_context: None,
        permission_domain: "workspace.write".into(),
        rationale: "Write the requested report".into(),
        evaluation: PermissionEvaluation {
            decision: PermissionPolicyDecision::Ask,
            reason: PermissionEvaluationReason::NoMatchingRule,
            rule_index: None,
            rule: None,
        },
        continuation: Default::default(),
    };
    let mut projection = PermissionProjection::default();
    projection.upsert(make_req("permission-1", "turn-1"));
    projection.upsert(make_req("permission-1", "turn-1"));
    assert_eq!(projection.revision, 1);
    projection.upsert(make_req("permission-2", "turn-1"));
    projection.upsert(make_req("permission-3", "turn-2"));
    assert_eq!(projection.revision, 3);
    let mut restored: PermissionProjection =
        serde_json::from_slice(&serde_json::to_vec(&projection).unwrap()).unwrap();
    assert_eq!(restored, projection);
    assert_eq!(
        restored.remove_turn(&TurnId::from_string("turn-1".into())),
        2
    );
    assert_eq!(restored.pending.len(), 1);
    assert_eq!(restored.pending[0].request_id.as_str(), "permission-3");
    assert_eq!(restored.revision, 4);
}

#[test]
fn permission_resource_shapes_local_paths_and_bash_quoted_expansion_scopes_hold() {
    // 1. Resource shape invariants: arbitrary names & domain labels never invent resource keys.
    for capability in [
        "vendor_read_everything",
        "writeish-calendar",
        "memory_like_extension",
        "chrome_clone",
        "搜索所有文件",
    ] {
        assert!(permission_resource_keys(capability).is_empty());
    }
    let opaque_input = json!({"path": "/tmp/a", "query": "current"});
    for (cap, domain) in [
        ("opaque-capability-17", "filesystem.read"),
        ("vendor.calendar.v2", "network"),
    ] {
        assert_eq!(
            permission_resource_for_input(cap, Some(domain), &opaque_input),
            opaque_input.to_string()
        );
    }
    assert_eq!(
        permission_resource_keys("read_file"),
        &["path", "root", "cwd", "source", "paths"]
    );
    assert!(permission_resource_keys("memory_save").is_empty());
    let mem_input = json!({"scope":"global", "kind":"preference", "content":"first"});
    assert_eq!(
        permission_resource_for_input("memory_save", Some("memory_save"), &mem_input),
        mem_input.to_string()
    );
    assert_ne!(
        permission_resource_for_input("memory_save", Some("memory_save"), &mem_input),
        permission_resource_for_input(
            "memory_save",
            Some("memory_save"),
            &json!({"scope":"global", "kind":"preference", "content":"changed"})
        )
    );

    // 2. Local path component matching & single percent-decoding.
    for capability in ["read", "edit", "files_read", "files_write"] {
        let candidate = PermissionRule {
            capability_pattern: capability.into(),
            resource_pattern: "file://localhost/work/project%20one".into(),
            decision: PermissionPolicyDecision::Allow,
            source: PermissionRuleSource::Conversation,
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
            ("/work/project one/%252e%252e/file.txt", true),
        ] {
            assert_scope(candidate.clone(), resource, matched);
        }
    }

    // 3. Bash conversation cwd binding, compound prefix rejection, and quoted expansion blocking.
    let bash_res = |command: &str, cwd: &str| {
        permission_resource_for_input("bash", Some("bash"), &json!({"command": command, "cwd": cwd}))
    };
    let (cap_pat, res_pat) = conversation_permission_rule(
        "bash",
        bash_res("python3 script.py first", "/work/project"),
    );
    let bash_candidate = PermissionRule {
        capability_pattern: cap_pat,
        resource_pattern: res_pat,
        decision: PermissionPolicyDecision::Allow,
        source: PermissionRuleSource::Conversation,
    };
    for (command, cwd, matched) in [
        ("python3 script.py second", "/work/project", true),
        ("python3 'script.py' 'second file'", "/work/project", true),
        ("python3 other.py second", "/work/project", false),
        ("python3 script.py second", "/work/project-other", false),
        ("python3 script.py second", ".", false),
        ("python3 script.py second; echo next", "/work/project", false),
        ("python3 script.py second && echo next", "/work/project", false),
        ("python3 script.py second | cat", "/work/project", false),
        ("python3 script.py second > output", "/work/project", false),
        ("python3 script.py $(echo second)", "/work/project", false),
        ("python3 script.py `echo second`", "/work/project", false),
        ("python3 script.py second\necho next", "/work/project", false),
        ("python3 script.py 'unfinished", "/work/project", false),
    ] {
        assert_scope(bash_candidate.clone(), &bash_res(command, cwd), matched);
    }

    let (cap, scope) = conversation_permission_rule(
        "bash",
        json!({"command":"python3 review.py --input first.txt", "cwd":"/workspace"}).to_string(),
    );
    let expansion_rules = PermissionRuleSet {
        rules: vec![PermissionRule {
            capability_pattern: cap,
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
        let cmd = format!("python3 review.py --input {argument}");
        let decision = expansion_rules.evaluate(&PermissionSubject {
            capability: "bash".into(),
            resource: json!({"command":cmd,"cwd":"/workspace"}).to_string(),
        });
        assert_eq!(
            decision,
            if allowed {
                PermissionPolicyDecision::Allow
            } else {
                PermissionPolicyDecision::Ask
            }
        );
    }

    for cmd in [
        r#"python3 review.py "$(other_command)""#,
        r#"python3 review.py "`other_command`""#,
    ] {
        let original = json!({"command":cmd,"cwd":"/workspace"}).to_string();
        let (_, exact_scope) = conversation_permission_rule("bash", original.clone());
        assert_eq!(exact_scope, original);
    }

    let (_, backslash_scope) = conversation_permission_rule(
        "bash",
        json!({"command":"python3 review.py", "cwd":"/workspace"}).to_string(),
    );
    let backslash_rules = PermissionRuleSet {
        rules: vec![PermissionRule {
            capability_pattern: "bash".into(),
            resource_pattern: backslash_scope,
            decision: PermissionPolicyDecision::Allow,
            source: PermissionRuleSource::Conversation,
        }],
    };
    for (cmd, allowed) in [
        (r#"python3 "review.py""#, true),
        (r#"python3 review\.py"#, true),
        (r#"python3 "review\.py""#, false),
        (r#"python3 'review\.py'"#, false),
    ] {
        assert_eq!(
            backslash_rules.evaluate(&PermissionSubject {
                capability: "bash".into(),
                resource: json!({"command":cmd,"cwd":"/workspace"}).to_string(),
            }),
            if allowed {
                PermissionPolicyDecision::Allow
            } else {
                PermissionPolicyDecision::Ask
            }
        );
    }
}

#[test]
fn protected_platform_data_omits_unauthorized_stale_and_unattributed_sources() {
    let prepared_payload = |authorization: Option<&str>, observed_at: &str| {
        let input = ContextPrepareInput {
            components: vec![ContextComponent {
                kind: ContextComponentKind::PlatformData,
                id: "health".into(),
                content: Value::String("private-health-payload".into()),
                source: Some(ContextComponentSource {
                    authorization: authorization.map(str::to_string),
                    observed_at: Some(observed_at.to_string()),
                    maximum_age_minutes: Some(60),
                    ..ContextComponentSource::default()
                }),
            }],
            ..ContextPrepareInput::default()
        };
        serde_json::to_string(&prepare_context(&input)).unwrap()
    };

    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
    let stale = (Utc::now() - Duration::minutes(61)).to_rfc3339_opts(SecondsFormat::Secs, true);

    let denied = prepared_payload(Some("denied"), &now);
    assert!(!denied.contains("private-health-payload") && denied.contains("authorization is denied"));

    let old = prepared_payload(Some("granted"), &stale);
    assert!(!old.contains("private-health-payload") && old.contains("older than 60 minutes"));

    assert!(prepared_payload(Some("granted"), &now).contains("private-health-payload"));
    assert!(prepared_payload(Some("not_required"), &now).contains("private-health-payload"));

    let unknown = prepared_payload(None, &now);
    assert!(!unknown.contains("private-health-payload") && unknown.contains("authorization is unknown"));

    for authorization in [None, Some("denied"), Some("not_determined")] {
        let input = ContextPrepareInput {
            components: vec![ContextComponent {
                kind: ContextComponentKind::PlatformData,
                id: "protected-source".into(),
                content: Value::String("private-source-body".into()),
                source: Some(ContextComponentSource {
                    authorization: authorization.map(str::to_string),
                    observed_at: Some(now.clone()),
                    maximum_age_minutes: Some(60),
                    ..ContextComponentSource::default()
                }),
            }],
            ..ContextPrepareInput::default()
        };
        let prepared = prepare_context(&input);
        let turn =
            nola_core::context_engine::prepare_context_components(input.platform, &input.components, 4096);
        for metrics in [&prepared.component_metrics, &turn.component_metrics] {
            let metric = metrics
                .iter()
                .find(|m| m.kind == ContextComponentKind::PlatformData)
                .unwrap();
            assert!(metric.used_ids.is_empty());
            assert!(metric.truncated_ids.is_empty());
            assert_eq!(metric.dropped_ids, vec!["protected-source"]);
            assert!(metric.used_tokens > 0);
        }
        assert!(!serde_json::to_string(&prepared.messages)
            .unwrap()
            .contains("private-source-body"));
        assert!(!serde_json::to_string(&turn.messages)
            .unwrap()
            .contains("private-source-body"));
    }

    for source in [None, Some(ContextComponentSource::default())] {
        let input = ContextPrepareInput {
            components: vec![ContextComponent {
                kind: ContextComponentKind::PlatformData,
                id: "unattributed-source".into(),
                content: Value::String("must-not-be-disclosed".into()),
                source,
            }],
            ..ContextPrepareInput::default()
        };
        let prepared = prepare_context(&input);
        let turn =
            nola_core::context_engine::prepare_context_components(input.platform, &input.components, 4096);
        for messages in [&prepared.messages, &turn.messages] {
            let encoded = serde_json::to_string(messages).unwrap();
            assert!(!encoded.contains("must-not-be-disclosed"));
            assert!(encoded.contains("authorization is unknown"));
        }
        for metrics in [&prepared.component_metrics, &turn.component_metrics] {
            let metric = metrics
                .iter()
                .find(|m| m.kind == ContextComponentKind::PlatformData)
                .unwrap();
            assert!(metric.used_ids.is_empty());
            assert_eq!(metric.dropped_ids, vec!["unattributed-source"]);
        }
    }
}
