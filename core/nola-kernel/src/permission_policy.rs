// SPDX-License-Identifier: Apache-2.0
//! Pure ordered permission evaluation and scope matching. No filesystem access.
use nola_domain::{
    PermissionAuthorityFacts, PermissionEvaluation, PermissionEvaluationReason,
    PermissionPolicyDecision, PermissionRule, PermissionRuleSource, PermissionSubject,
};
use serde_json::{json, Value};

#[cfg(test)]
mod verified_resource_rule_tests {
    use super::*;
    use nola_domain::assistant_journal::AssistantDigest;
    use nola_domain::permission_scope::{ConcretePermissionSubject,PermissionResource};

    fn concrete(resources: Vec<PermissionResource>) -> ConcretePermissionSubject {
        ConcretePermissionSubject {execution_target:"trusted-host".into(),capability:"host.file".into(),
            operation:"read".into(),input_digest:AssistantDigest::parse_hex(&"a".repeat(64)).unwrap(),resources}
    }
    fn file(parts: &[&str]) -> PermissionResource {
        PermissionResource::File {filesystem_id:"volume".into(),components:parts.iter().map(|part|(*part).into()).collect(),access:"read".into()}
    }
    fn rule(domain: &str, scope: &str, decision: PermissionPolicyDecision) -> PermissionRule {
        PermissionRule {capability_pattern:domain.into(),resource_pattern:scope.into(),decision,source:PermissionRuleSource::UserGlobal}
    }
    fn evaluate(rules: &[PermissionRule], domain: &str, facts: &ConcretePermissionSubject, confirmation: bool) -> PermissionPolicyDecision {
        evaluate_permission_with_resolved_resources(rules,&PermissionSubject {capability:domain.into(),
            resource:facts.approval_resource_identity().unwrap()},PermissionAuthorityFacts {
                carries_live_user_authority:true,accepts_conversation_permission_grants:true,
            },confirmation,Some(facts)).decision
    }
    #[test]
    fn explicit_file_wildcard_ask_and_deny_apply_to_verified_resources() {
        let facts = concrete(vec![file(&["workspace","new.txt"])]);
        assert_eq!(evaluate(&[rule("edit","*",PermissionPolicyDecision::Ask)],"edit",&facts,false),PermissionPolicyDecision::Ask);
        let mut rules = vec![PermissionRule {capability_pattern:"*".into(),resource_pattern:"*".into(),
            decision:PermissionPolicyDecision::Allow,source:PermissionRuleSource::InvocationPolicy}];
        rules.push(rule("edit","*",PermissionPolicyDecision::Deny));
        assert_eq!(evaluate(&rules,"edit",&facts,false),PermissionPolicyDecision::Deny);
        let literal = PermissionRule {source:PermissionRuleSource::Conversation,..rule("edit","*",PermissionPolicyDecision::Allow)};
        assert_eq!(evaluate(&[literal],"edit",&facts,true),PermissionPolicyDecision::Ask,
            "a conversation literal star must not become a configured wildcard");
    }
    #[test]
    fn file_allow_covers_all_resources_while_ask_and_deny_match_any() {
        let inside = concrete(vec![file(&["workspace","first"]),file(&["workspace","nested","second"])]);
        let mixed = concrete(vec![file(&["workspace","first"]),file(&["workspace-other","second"])]);
        assert_eq!(evaluate(&[rule("read","/workspace",PermissionPolicyDecision::Allow)],"read",&inside,true),PermissionPolicyDecision::Allow);
        assert_eq!(evaluate(&[rule("read","/workspace",PermissionPolicyDecision::Allow)],"read",&mixed,true),PermissionPolicyDecision::Ask);
        for decision in [PermissionPolicyDecision::Ask,PermissionPolicyDecision::Deny] {
            assert_eq!(evaluate(&[rule("read","/workspace",decision)],"read",&mixed,false),decision);
        }
    }
    #[test]
    fn model_full_input_json_cannot_supply_verified_file_facts() {
        let facts = concrete(vec![file(&["workspace","file"])]);
        let rules = [rule("edit","/workspace",PermissionPolicyDecision::Allow)];
        let authority = PermissionAuthorityFacts {carries_live_user_authority:true,accepts_conversation_permission_grants:true};
        let fake = PermissionSubject {capability:"edit".into(),resource:facts.approval_resource_identity().unwrap()};
        assert_eq!(evaluate_permission(&rules,&fake,authority,true).decision,PermissionPolicyDecision::Ask);
        let current_raw = PermissionSubject {capability:"edit".into(),resource:"/workspace/current-native-path".into()};
        assert_eq!(evaluate_permission(&rules,&current_raw,authority,true).decision,PermissionPolicyDecision::Allow);
    }
    #[test]
    fn exact_process_scope_honors_user_grants_and_preserves_explicit_constraints() {
        let facts = concrete(vec![PermissionResource::ProcessInvocation {
            invocation_digest:AssistantDigest::parse_hex(&"b".repeat(64)).unwrap(),
            command:"sender --body @payload $DESTINATION".into(),cwd:"/workspace".into(),
        }]);
        let identity = facts.approval_resource_identity().unwrap();
        for source in [PermissionRuleSource::Conversation,PermissionRuleSource::Turn] {
            for decision in [PermissionPolicyDecision::Allow,PermissionPolicyDecision::Ask,PermissionPolicyDecision::Deny] {
                let rules = [PermissionRule {source,capability_pattern:"bash".into(),resource_pattern:identity.clone(),decision}];
                assert_eq!(evaluate(&rules,"bash",&facts,true),decision);
            }
        }
        let explicit = format!("{BASH_COMMAND_GLOB_PREFIX}{}",json!({"command_pattern":"sender *"}));
        assert_eq!(evaluate(&[rule("bash",&explicit,PermissionPolicyDecision::Allow)],"bash",&facts,true),PermissionPolicyDecision::Allow);
        let all = [PermissionRule {source:PermissionRuleSource::InvocationPolicy,
            capability_pattern:"*".into(),resource_pattern:"*".into(),decision:PermissionPolicyDecision::Allow}];
        assert_eq!(evaluate(&all,"bash",&facts,true),PermissionPolicyDecision::Allow);
        assert_eq!(evaluate(&[],"bash",&facts,false),PermissionPolicyDecision::Allow,
            "trusted ordinary read classification must not gain a new prompt");
    }
    #[test]
    fn process_configuration_matches_host_command_not_a_digest_or_operation_label() {
        let process = |command: &str| concrete(vec![PermissionResource::ProcessInvocation {
            invocation_digest:AssistantDigest::parse_hex(&"b".repeat(64)).unwrap(),command:command.into(),cwd:"/workspace".into(),
        }]);
        let pattern = format!("{BASH_COMMAND_GLOB_PREFIX}{}",json!({"command_pattern":"git status*"}));
        let rules = [rule("bash",&pattern,PermissionPolicyDecision::Allow)];
        assert_eq!(evaluate(&rules,"bash",&process("git status --short"),true),PermissionPolicyDecision::Allow);
        assert_eq!(evaluate(&rules,"bash",&process("rm -rf private"),true),PermissionPolicyDecision::Ask);
        let denied = format!("{BASH_COMMAND_GLOB_PREFIX}{}",json!({"command_pattern":"rm *"}));
        assert_eq!(evaluate(&[rule("bash",&denied,PermissionPolicyDecision::Deny)],"bash",&process("rm -rf private"),true),PermissionPolicyDecision::Deny);
    }
}

#[cfg(test)]
mod operation_contract_tests {
    use super::*;
    use nola_domain::permission_resource_plan::*;
    use nola_domain::permission_scope::PermissionResource;

    #[test]
    fn unknown_contract_cannot_claim_a_read_from_model_operation_labels() {
        let subject = PermissionSubject {capability:"opaque".into(), resource:json!({"operation_kind":"inspect","dry_run":true}).to_string()};
        assert!(permission_contract_requires_confirmation(&PermissionOperationContract::default(),None));
        assert_eq!(evaluate_permission(&[],&subject,PermissionAuthorityFacts {
            carries_live_user_authority:true,accepts_conversation_permission_grants:true},true).decision, PermissionPolicyDecision::Ask);
    }

    #[test]
    fn ordinary_contracts_do_not_require_query_specific_approval() {
        let contract = PermissionOperationContract {risk:PermissionOperationRisk::OrdinaryRead,..Default::default()};
        assert!(!permission_contract_requires_confirmation(&contract,None));
        for resource in ["SELECT first", "SELECT second", "自然的后续问题", "current search"] {
            let evaluation = evaluate_permission(&[PermissionRule {capability_pattern:"*".into(), resource_pattern:"*".into(),
                decision:PermissionPolicyDecision::Ask,source:PermissionRuleSource::Host}],
                &PermissionSubject {capability:"configured_search".into(),resource:resource.into()}, PermissionAuthorityFacts {
                    carries_live_user_authority:true,accepts_conversation_permission_grants:true},false);
            assert_eq!(evaluation.decision,PermissionPolicyDecision::Allow);
        }
    }

    #[test]
    fn host_resource_facts_distinguish_creation_overwrite_and_sensitive_reads() {
        let plan = PermissionResourcePlan {capability:"file".into(), operation:"write".into(), requirements:vec![ResourceRequirement {
            requirement_id:"target".into(),locator_kind:ResourceLocatorKind::FilePath,access:"write".into(),
            selector:ResourceSelector::One {pointer:"/path".into()} }]};
        let input = json!({"path":"notes.txt","operation_kind":"inspect"});
        let request = PermissionResolutionRequest {resolution_id:"resolution".into(),execution_target:"host".into(),
            invocation_digest:PermissionResolutionRequest::input_digest(&input), capability:plan.capability.clone(),operation:plan.operation.clone(),
            demands:plan.extract(&input).unwrap()};
        let contract = PermissionOperationContract {resource_plan:Some(plan),..Default::default()};
        assert!(permission_contract_requires_confirmation(&contract,None));
        for (observation,expected) in [(PermissionOperationRisk::NewFile,false),(PermissionOperationRisk::Overwrite,true),
            (PermissionOperationRisk::SensitiveRead,true),(PermissionOperationRisk::Unknown,true)] {
            let facts = PermissionResolutionFacts {resolution_id:request.resolution_id.clone(),execution_target:request.execution_target.clone(),
                invocation_digest:request.invocation_digest,bindings:vec![ResolvedDemandBinding {demand:request.demands[0].clone(),
                    resource:PermissionResource::File {filesystem_id:"volume".into(),components:vec!["notes.txt".into()],access:"write".into()},
                    evidence_id:"host-evidence".into(),observation,browser:None,derived:None}]};
            assert_eq!(permission_contract_requires_confirmation(&contract,Some(&(request.clone(),facts))),expected);
        }
    }

    #[test]
    fn derived_members_have_individual_risk_and_cannot_hide_an_explicit_file_deny() {
        let plan = PermissionResourcePlan {capability:"compound-host-operation".into(),operation:"filesystem".into(),
            requirements:vec![ResourceRequirement {requirement_id:"imports".into(),locator_kind:ResourceLocatorKind::DerivedFileCollection,
                access:"derive_move_imports".into(),selector:ResourceSelector::One {pointer:"/root".into()}}]};
        let input = json!({"root":"/project"});
        let request = PermissionResolutionRequest {resolution_id:"derived".into(),execution_target:"host".into(),
            invocation_digest:PermissionResolutionRequest::input_digest(&input),capability:plan.capability.clone(),
            operation:plan.operation.clone(),demands:plan.extract(&input).unwrap()};
        let members = vec![HostDerivedResourceMember {resource:PermissionResource::File {
            filesystem_id:"volume".into(),components:vec!["project".into(),"app.ts".into()],access:"overwrite".into()},
            before_version:format!("sha256:{}","a".repeat(64)),planned_after_version:format!("sha256:{}","b".repeat(64)),
            observation:PermissionOperationRisk::Overwrite,evidence_id:"actual-importer".into()}];
        let derived = HostDerivedResourceCollection {collection_id:"imports".into(),discovery_complete:true,
            plan_digest:HostDerivedResourceCollection::digest(&request,&request.demands[0],&members),members};
        let facts = PermissionResolutionFacts {resolution_id:request.resolution_id.clone(),execution_target:request.execution_target.clone(),
            invocation_digest:request.invocation_digest,bindings:vec![ResolvedDemandBinding {demand:request.demands[0].clone(),
                resource:PermissionResource::File {filesystem_id:"volume".into(),components:vec!["project".into()],access:"derive_move_imports".into()},
                evidence_id:"complete-collection".into(),observation:PermissionOperationRisk::OrdinaryRead,browser:None,derived:Some(derived)}]};
        let contract = PermissionOperationContract {risk:PermissionOperationRisk::OrdinaryRead,resource_plan:Some(plan),..Default::default()};
        assert!(permission_contract_requires_confirmation(&contract,Some(&(request.clone(),facts.clone()))),
            "an ordinary compound operation cannot mask an overwrite member");
        let concrete = facts.subject_for(&request).unwrap();
        let rules = [
            PermissionRule {capability_pattern:"*".into(),resource_pattern:"*".into(),decision:PermissionPolicyDecision::Allow,
                source:PermissionRuleSource::InvocationPolicy},
            PermissionRule {capability_pattern:"edit".into(),resource_pattern:"/project/app.ts".into(),decision:PermissionPolicyDecision::Deny,
                source:PermissionRuleSource::UserGlobal},
        ];
        let decision = evaluate_permission_with_resolved_resources(&rules,&PermissionSubject {
            capability:"edit".into(),resource:concrete.approval_resource_identity().unwrap()},PermissionAuthorityFacts {
                carries_live_user_authority:true,accepts_conversation_permission_grants:true},true,Some(&concrete));
        assert_eq!(decision.decision,PermissionPolicyDecision::Deny);
    }
}

/// Pure classification over a trusted catalog contract and correlated Host
/// observations. Missing facts never turn an opaque execution into a read.
pub fn permission_contract_requires_confirmation(
    contract: &nola_domain::permission_resource_plan::PermissionOperationContract,
    resolution: Option<&(nola_domain::permission_resource_plan::PermissionResolutionRequest,
        nola_domain::permission_resource_plan::PermissionResolutionFacts)>,
) -> bool {
    use nola_domain::permission_resource_plan::PermissionOperationRisk;
    if contract.resource_plan.is_none() { return contract.risk.requires_confirmation(); }
    let Some((request, facts)) = resolution else { return true; };
    if facts.subject_for(request).is_err() { return true; }
    if !matches!(contract.risk, PermissionOperationRisk::OrdinaryRead | PermissionOperationRisk::NewFile
        | PermissionOperationRisk::Unknown) { return true; }
    facts.bindings.iter().any(|binding| binding.observation.requires_confirmation()
        || binding.derived.as_ref().is_some_and(|collection|
            collection.members.iter().any(|member| member.observation.requires_confirmation())))
}

pub fn evaluate_permission(
    rules: &[PermissionRule],
    subject: &PermissionSubject,
    authority: PermissionAuthorityFacts,
    profile_confirmation_required: bool,
) -> PermissionEvaluation {
    evaluate_permission_with_resolved_resources(rules,subject,authority,profile_confirmation_required,None)
}

/// `resolved` must come from a correlated, validated Host resolution. Never
/// recover it by interpreting model-controlled FullInput/resource JSON.
pub fn evaluate_permission_with_resolved_resources(
    rules: &[PermissionRule], subject: &PermissionSubject, authority: PermissionAuthorityFacts,
    profile_confirmation_required: bool,
    resolved: Option<&nola_domain::permission_scope::ConcretePermissionSubject>,
) -> PermissionEvaluation {
    let matched = rules
        .iter()
        .enumerate()
        .filter(|rule| {
            authority.carries_live_user_authority
                || (authority.accepts_conversation_permission_grants
                    && rule.1.source == PermissionRuleSource::Conversation)
                || !matches!(
                    rule.1.source,
                    PermissionRuleSource::Conversation | PermissionRuleSource::Turn
                )
        })
        .filter(|rule| {
            capability_matches(rule.1, &subject.capability)
                && permission_resource_matches_verified(rule.1, subject, resolved)
        })
        .last();

    // A declared invocation policy is not an OS permission or a temporary
    // live-user grant. Host Ask defaults cannot reintroduce application UI in
    // allow_all. A matching explicit Deny remains an execution constraint.
    if !matched.is_some_and(|(_, rule)| rule.decision == PermissionPolicyDecision::Deny) {
        if let Some((rule_index, rule)) = rules.iter().enumerate().rev().find(|(_, rule)|
            rule.source == PermissionRuleSource::InvocationPolicy
                && rule.capability_pattern == "*" && rule.resource_pattern == "*"
                && rule.decision == PermissionPolicyDecision::Allow) {
            return PermissionEvaluation { decision: PermissionPolicyDecision::Allow,
                reason: PermissionEvaluationReason::MatchingRule, rule_index: Some(rule_index), rule: Some(rule.clone()) };
        }
    }

    match matched {
        Some((rule_index, rule)) if !profile_confirmation_required
            && rule.decision == PermissionPolicyDecision::Ask
            && matches!(rule.source, PermissionRuleSource::ProductDefault | PermissionRuleSource::Host) => PermissionEvaluation {
                decision: PermissionPolicyDecision::Allow, reason: PermissionEvaluationReason::OrdinaryOperationAllowed,
                rule_index: Some(rule_index), rule: Some(rule.clone()),
            },
        // The cautious product profile is deliberately permissive for
        // ordinary work, but it must still stop before a sensitive or
        // irreversible invocation. Explicit user/conversation/turn rules remain the
        // final authority and therefore bypass this product-default guard.
        Some((rule_index, rule))
            if matches!(rule.source, PermissionRuleSource::ProductDefault | PermissionRuleSource::Host)
                && rule.decision == PermissionPolicyDecision::Allow
                && profile_confirmation_required =>
        {
            PermissionEvaluation {
                decision: PermissionPolicyDecision::Ask,
                reason: PermissionEvaluationReason::CautiousProfileRequiresConfirmation,
                rule_index: Some(rule_index),
                rule: Some(rule.clone()),
            }
        }
        Some((rule_index, rule)) => PermissionEvaluation {
            decision: rule.decision,
            reason: PermissionEvaluationReason::MatchingRule,
            rule_index: Some(rule_index),
            rule: Some(rule.clone()),
        },
        None if !profile_confirmation_required => PermissionEvaluation {
            decision: PermissionPolicyDecision::Allow, reason: PermissionEvaluationReason::OrdinaryOperationAllowed,
            rule_index: None, rule: None,
        },
        None => {
            let excluded_interactive_rule = (!authority.carries_live_user_authority)
                .then(|| {
                    rules
                        .iter()
                        .enumerate()
                        .filter(|(_, rule)| {
                            matches!(
                                rule.source,
                                PermissionRuleSource::Conversation | PermissionRuleSource::Turn
                            ) && capability_matches(rule, &subject.capability)
                                && permission_resource_matches(rule, subject)
                        })
                        .last()
                })
                .flatten();
            if let Some((rule_index, rule)) = excluded_interactive_rule {
                PermissionEvaluation {
                    decision: PermissionPolicyDecision::Ask,
                    reason: PermissionEvaluationReason::InteractiveGrantUnavailableForOrigin,
                    rule_index: Some(rule_index),
                    rule: Some(rule.clone()),
                }
            } else {
                PermissionEvaluation {
                    decision: PermissionPolicyDecision::Ask,
                    reason: PermissionEvaluationReason::NoMatchingRule,
                    rule_index: None,
                    rule: None,
                }
            }
        }
    }
}

pub const BASH_COMMAND_GLOB_PREFIX: &str = "nola:bash_command_glob:v1:";

pub const BASH_SCOPE_PREFIX: &str = "nola:bash_scope:v1:";

fn interactive_grant(rule: &PermissionRule) -> bool {
    // Concrete conversation approvals are literal resource data. Invocation
    // policy is separate and does not depend on a live window's authority.
    rule.source == PermissionRuleSource::Conversation
}

fn capability_matches(rule: &PermissionRule, capability: &str) -> bool {
    if interactive_grant(rule) {
        rule.capability_pattern == capability
    } else {
        wildcard_matches(&rule.capability_pattern, capability)
    }
}

pub fn permission_resource_matches(rule: &PermissionRule, subject: &PermissionSubject) -> bool {
    // Concrete user approvals are data, not configuration pattern programs.
    // In particular '*' in a filename, query or JSON input cannot authorize
    // other calls. Explicit command/directory scopes retain their own parsers.
    if interactive_grant(rule) {
        if rule.capability_pattern != subject.capability { return false; }
        if rule.resource_pattern == subject.resource { return true; }
    }
    if !interactive_grant(rule) && rule.resource_pattern == "*" { return true; }
    let shell_capability = matches!(subject.capability.as_str(),
        "bash" | "shell.execute" | "bash_background_start" | "bash_command_review");
    if shell_capability && rule.capability_pattern == subject.capability
        && rule.resource_pattern.starts_with(BASH_SCOPE_PREFIX)
    {
        return bash_scope_matches(&rule.resource_pattern, &subject.resource);
    }
    if shell_capability && rule.capability_pattern == subject.capability
        && rule.resource_pattern.starts_with(BASH_COMMAND_GLOB_PREFIX)
    {
        return bash_command_glob_matches(&rule.resource_pattern, &subject.resource);
    }
    if rule.capability_pattern == subject.capability
        && matches!(
            subject.capability.as_str(),
            "read" | "edit" | "files_read" | "files_write"
        )
    {
        return permission_path_scope_matches(&rule.resource_pattern, &subject.resource);
    }
    !interactive_grant(rule) && wildcard_matches(&rule.resource_pattern, &subject.resource)
}

fn permission_resource_matches_verified(rule: &PermissionRule, subject: &PermissionSubject,
    resolved: Option<&nola_domain::permission_scope::ConcretePermissionSubject>) -> bool {
    use nola_domain::permission_scope::PermissionResource;
    let Some(resolved) = resolved else { return permission_resource_matches(rule,subject); };
    if resolved.validate().is_err() { return false; }
    if !resolved.supports_reusable_approval() && rule.decision == PermissionPolicyDecision::Allow
        && matches!(rule.source, PermissionRuleSource::Conversation | PermissionRuleSource::Turn) {
        return false;
    }
    // Conversation approvals remain literal grants of the frozen exact scope.
    if interactive_grant(rule) {
        return rule.capability_pattern == subject.capability && rule.resource_pattern == subject.resource;
    }
    if rule.resource_pattern == "*" { return true; }
    let matched = if matches!(subject.capability.as_str(),"read"|"edit"|"files_read"|"files_write") {
        let Some(path) = normalized_local_permission_path(&rule.resource_pattern) else { return false; };
        if !path.is_absolute() { return false; }
        let mut components = Vec::new();
        for component in path.components() {
            match component {
                std::path::Component::RootDir => {},
                std::path::Component::Normal(part) => {
                    let Some(part) = part.to_str() else { return false; };
                    components.push(part.to_owned());
                }
                _ => return false,
            }
        }
        resolved.resources.iter().map(|resource| match resource {
            PermissionResource::File {components:requested,..} => requested.starts_with(&components),
            _ => false,
        }).collect::<Vec<_>>()
    } else if matches!(subject.capability.as_str(),"bash"|"shell.execute"|"bash_background_start"|"bash_command_review")
        && (rule.resource_pattern.starts_with(BASH_SCOPE_PREFIX) || rule.resource_pattern.starts_with(BASH_COMMAND_GLOB_PREFIX)) {
        resolved.resources.iter().map(|resource| match resource {
            PermissionResource::ProcessInvocation {command,cwd,..} => permission_resource_matches(rule,&PermissionSubject {
                capability:subject.capability.clone(),resource:json!({"command":command,"cwd":cwd}).to_string(),
            }),
            _ => false,
        }).collect::<Vec<_>>()
    } else { return permission_resource_matches(rule,subject); };
    if rule.decision == PermissionPolicyDecision::Allow { matched.into_iter().all(|matched|matched) }
    else { matched.into_iter().any(|matched|matched) }
}

fn permission_path_scope_matches(granted: &str, requested: &str) -> bool {
    let Some(granted) = normalized_local_permission_path(granted) else {
        return false;
    };
    let Some(requested) = normalized_local_permission_path(requested) else {
        return false;
    };
    if !granted.is_absolute()
        || !requested.is_absolute()
        || granted
            .components()
            .chain(requested.components())
            .any(|component| component == std::path::Component::ParentDir)
    {
        return false;
    }
    requested == granted || requested.starts_with(granted)
}

fn normalized_local_permission_path(value: &str) -> Option<std::path::PathBuf> {
    let trimmed = value.trim();
    let encoded_path = if let Some(rest) = trimmed.strip_prefix("file://") {
        if rest.starts_with('/') {
            rest
        } else if let Some(local_path) = rest.strip_prefix("localhost") {
            if local_path.starts_with('/') {
                local_path
            } else {
                return None;
            }
        } else {
            return None;
        }
    } else {
        trimmed
    };
    let bytes = encoded_path.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = *bytes.get(index + 1)?;
            let low = *bytes.get(index + 2)?;
            decoded.push((hex_value(high)? << 4) | hex_value(low)?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    let path = std::path::PathBuf::from(String::from_utf8(decoded).ok()?);
    (!path
        .components()
        .any(|component| component == std::path::Component::ParentDir))
    .then_some(path)
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn bash_command_glob_matches(pattern: &str, resource: &str) -> bool {
    let Some(encoded) = pattern.strip_prefix(BASH_COMMAND_GLOB_PREFIX) else {
        return false;
    };
    let Ok(pattern) = serde_json::from_str::<Value>(encoded) else {
        return false;
    };
    let Some(command_pattern) = pattern.get("command_pattern").and_then(Value::as_str) else {
        return false;
    };
    let Ok(resource) = serde_json::from_str::<Value>(resource) else {
        return false;
    };
    let Some(command) = resource.get("command").and_then(Value::as_str) else {
        return false;
    };
    wildcard_matches(command_pattern, command)
}

pub fn bash_conversation_scope(resource: &str) -> Option<String> {
    let resource = serde_json::from_str::<Value>(resource).ok()?;
    let command = resource.get("command")?.as_str()?;
    let cwd = resource.get("cwd").and_then(Value::as_str).unwrap_or(".");
    let tokens = safe_shell_tokens(command)?;
    let executable = tokens.first()?.rsplit('/').next().unwrap_or_default();
    let keep = match executable {
        // Choosing "allow for this conversation" must mean more than one
        // exact argv string. Keep destructive executable families scoped to
        // the same working directory, while still requiring a fresh decision
        // when the agent switches to a different destructive mechanism.
        "rm" | "rmdir" | "unlink" | "shred" | "truncate" | "kill" | "killall" | "pkill" => 1,
        "git" | "cargo" | "xcodebuild" => 2,
        "npm" | "pnpm" | "yarn" if tokens.get(1).map(String::as_str) == Some("run") => 3,
        "npm" | "pnpm" | "yarn" => 2,
        "python" | "python3" if tokens.get(1).map(String::as_str) == Some("-m") => 3,
        "python" | "python3" | "swift" | "ruby" | "perl"
            if tokens.get(1).is_some_and(|value| !value.starts_with('-')) =>
        {
            2
        }
        "bash" | "sh" | "zsh"
            if tokens.get(1).is_some_and(|value| {
                !value.starts_with('-')
                    && ["sh", "bash", "zsh"]
                        .iter()
                        .any(|suffix| value.ends_with(&format!(".{suffix}")))
            }) =>
        {
            2
        }
        executable if executable.ends_with("gradlew") => 2,
        _ => tokens.len(),
    };
    let scope = json!({
        "cwd": cwd,
        "tokens": tokens.into_iter().take(keep).collect::<Vec<_>>(),
    });
    Some(format!(
        "{BASH_SCOPE_PREFIX}{}",
        serde_json::to_string(&scope).ok()?
    ))
}

fn bash_scope_matches(pattern: &str, resource: &str) -> bool {
    let Some(encoded) = pattern.strip_prefix(BASH_SCOPE_PREFIX) else {
        return false;
    };
    let Ok(scope) = serde_json::from_str::<Value>(encoded) else {
        return false;
    };
    let Ok(resource) = serde_json::from_str::<Value>(resource) else {
        return false;
    };
    let granted_cwd = scope.get("cwd").and_then(Value::as_str).unwrap_or(".");
    let requested_cwd = resource.get("cwd").and_then(Value::as_str).unwrap_or(".");
    if granted_cwd != requested_cwd {
        return false;
    }
    let Some(command) = resource.get("command").and_then(Value::as_str) else {
        return false;
    };
    let Some(command_tokens) = safe_shell_tokens(command) else {
        return false;
    };
    let Some(scope_tokens) = scope.get("tokens").and_then(Value::as_array) else {
        return false;
    };
    let scope_tokens = scope_tokens
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    !scope_tokens.is_empty()
        && command_tokens.len() >= scope_tokens.len()
        && command_tokens
            .iter()
            .zip(scope_tokens)
            .all(|(actual, expected)| actual == expected)
}

pub fn safe_shell_tokens(command: &str) -> Option<Vec<String>> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut quote = None;
    let mut escaped = false;
    for character in command.chars() {
        if escaped {
            // In double quotes, backslash only removes the special meaning
            // of these shell characters. Preserve it before ordinary letters
            // so a different script filename cannot match an existing scope.
            if quote == Some('"') && !matches!(character, '$' | '`' | '"' | '\\' | '\n') {
                token.push('\\');
            }
            if character != '\n' {
                token.push(character);
            }
            escaped = false;
            continue;
        }
        if character == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if let Some(active_quote) = quote {
            // Double quotes preserve shell expansion. A static script-prefix
            // grant cannot authorize commands introduced by that expansion.
            // Single quotes and backslash-escaped metacharacters stay literal.
            if active_quote == '"' && matches!(character, '$' | '`') {
                return None;
            }
            if character == active_quote {
                quote = None;
            } else {
                token.push(character);
            }
            continue;
        }
        match character {
            '\'' | '"' => quote = Some(character),
            ' ' | '\t' => {
                if !token.is_empty() {
                    tokens.push(std::mem::take(&mut token));
                }
            }
            ';' | '|' | '&' | '>' | '<' | '`' | '\n' | '\r' | '$' => return None,
            _ => token.push(character),
        }
    }
    if escaped || quote.is_some() {
        return None;
    }
    if !token.is_empty() {
        tokens.push(token);
    }
    (!tokens.is_empty()).then_some(tokens)
}

pub fn canonical_resource(value: &Value) -> String {
    match value {
        Value::String(value) => value.trim().to_string(),
        value => serde_json::to_string(value).unwrap_or_else(|_| value.to_string()),
    }
}

fn wildcard_matches(pattern: &str, value: &str) -> bool {
    let pattern = pattern.as_bytes();
    let value = value.as_bytes();
    let mut pattern_index = 0;
    let mut value_index = 0;
    let mut star_index = None;
    let mut retry_value_index = 0;

    while value_index < value.len() {
        if pattern_index < pattern.len()
            && (pattern[pattern_index] == b'?' || pattern[pattern_index] == value[value_index])
        {
            pattern_index += 1;
            value_index += 1;
        } else if pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
            star_index = Some(pattern_index);
            pattern_index += 1;
            retry_value_index = value_index;
        } else if let Some(star_index) = star_index {
            pattern_index = star_index + 1;
            retry_value_index += 1;
            value_index = retry_value_index;
        } else {
            return false;
        }
    }

    while pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
        pattern_index += 1;
    }
    pattern_index == pattern.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rule(source: PermissionRuleSource, decision: PermissionPolicyDecision) -> PermissionRule {
        PermissionRule {
            capability_pattern: if source == PermissionRuleSource::Conversation {
                "tool.read"
            } else { "tool.*" }.into(),
            resource_pattern: if source == PermissionRuleSource::Conversation {
                "resource"
            } else { "*" }.into(),
            decision,
            source,
        }
    }
    fn subject() -> PermissionSubject {
        PermissionSubject {
            capability: "tool.read".into(),
            resource: "resource".into(),
        }
    }
    #[test]
    fn interactive_permission_literals_never_become_configuration_patterns() {
        let authority = PermissionAuthorityFacts { carries_live_user_authority: true,
            accepts_conversation_permission_grants: true };
        for source in [PermissionRuleSource::Conversation] {
            for resource in ["notes/*.md".to_string(),
                json!({"source":"*", "destination":"out/a"}).to_string()] {
                let rule = PermissionRule { capability_pattern: "custom.*".into(),
                    resource_pattern: resource.clone(), decision: PermissionPolicyDecision::Allow, source };
                let original = PermissionSubject { capability: "custom.*".into(), resource: resource.clone() };
                assert_eq!(evaluate_permission(&[rule.clone()], &original, authority, true).decision,
                    PermissionPolicyDecision::Allow);
                let changed = PermissionSubject { capability: original.capability.clone(),
                    resource: resource.replace('*', "private") };
                assert_eq!(evaluate_permission(&[rule.clone()], &changed, authority, true).decision,
                    PermissionPolicyDecision::Ask);
                let other_capability = PermissionSubject { capability: "custom.read".into(), resource };
                assert_eq!(evaluate_permission(&[rule], &other_capability, authority, true).decision,
                    PermissionPolicyDecision::Ask);
            }
        }
        // Explicit configured patterns keep their documented semantics.
        let configured = rule(PermissionRuleSource::UserGlobal, PermissionPolicyDecision::Allow);
        assert_eq!(evaluate_permission(&[configured], &subject(), authority, false).decision,
            PermissionPolicyDecision::Allow);
        let command = json!({"command":"cargo test", "cwd":"/workspace"}).to_string();
        let encoded = bash_conversation_scope(&command).unwrap();
        let not_shell = PermissionRule { capability_pattern: "custom.tool".into(),
            resource_pattern: encoded, decision: PermissionPolicyDecision::Allow,
            source: PermissionRuleSource::Conversation };
        assert!(!permission_resource_matches(&not_shell,
            &PermissionSubject { capability: "custom.tool".into(), resource: command }));
    }
    #[test]
    fn authority_source_decision_and_profile_matrix_retains_explanation() {
        for live in [false, true] {
            for conversation in [false, true] {
                let authority = PermissionAuthorityFacts {
                    carries_live_user_authority: live,
                    accepts_conversation_permission_grants: conversation,
                };
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
                        for profile in [false, true] {
                            let rule = rule(source, decision);
                            let evaluation = evaluate_permission(
                                &[rule.clone()],
                                &subject(),
                                authority,
                                profile,
                            );
                            let admitted = live
                                || (conversation && source == PermissionRuleSource::Conversation)
                                || !matches!(
                                    source,
                                    PermissionRuleSource::Conversation | PermissionRuleSource::Turn
                                );
                            let guarded = admitted
                                && matches!(source, PermissionRuleSource::ProductDefault | PermissionRuleSource::Host)
                                && decision == PermissionPolicyDecision::Allow
                                && profile;
                            assert_eq!(
                                evaluation.decision,
                                if !profile && (!admitted || (decision == PermissionPolicyDecision::Ask
                                    && matches!(source, PermissionRuleSource::ProductDefault | PermissionRuleSource::Host))) {
                                    PermissionPolicyDecision::Allow
                                } else if admitted && !guarded {
                                    decision
                                } else {
                                    PermissionPolicyDecision::Ask
                                }
                            );
                            assert_eq!(
                                evaluation.reason,
                                if !profile && (!admitted || (decision == PermissionPolicyDecision::Ask
                                    && matches!(source, PermissionRuleSource::ProductDefault | PermissionRuleSource::Host))) {
                                    PermissionEvaluationReason::OrdinaryOperationAllowed
                                } else if !admitted {
                                    PermissionEvaluationReason::InteractiveGrantUnavailableForOrigin
                                } else if guarded {
                                    PermissionEvaluationReason::CautiousProfileRequiresConfirmation
                                } else {
                                    PermissionEvaluationReason::MatchingRule
                                }
                            );
                            assert_eq!(evaluation.rule_index, if !admitted && !profile {None} else {Some(0)});
                            assert_eq!(evaluation.rule, if !admitted && !profile {None} else {Some(rule)});
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn last_matching_rule_preserves_original_index_and_excluded_grant_reason() {
        let authority = PermissionAuthorityFacts {
            carries_live_user_authority: false,
            accepts_conversation_permission_grants: false,
        };
        let mut unrelated = rule(PermissionRuleSource::Host, PermissionPolicyDecision::Allow);
        unrelated.capability_pattern = "other".into();
        let rules = vec![
            rule(PermissionRuleSource::Host, PermissionPolicyDecision::Deny),
            unrelated.clone(),
            rule(
                PermissionRuleSource::Conversation,
                PermissionPolicyDecision::Allow,
            ),
        ];
        let result = evaluate_permission(&rules, &subject(), authority, true);
        assert_eq!(result.rule_index, Some(0));
        assert_eq!(result.decision, PermissionPolicyDecision::Deny);
        let result = evaluate_permission(&rules[1..], &subject(), authority, true);
        assert_eq!(result.rule_index, Some(1));
        assert_eq!(
            result.reason,
            PermissionEvaluationReason::InteractiveGrantUnavailableForOrigin
        );
        let result = evaluate_permission(&[unrelated], &subject(), authority, true);
        assert_eq!(result.rule_index, None);
        assert_eq!(result.rule, None);
        assert_eq!(result.reason, PermissionEvaluationReason::NoMatchingRule);
    }
    #[test]
    fn wildcard_scope_and_bash_reuse_are_separate_exact_capability_contracts() {
        let resource = json!({"cwd":"/workspace","command":"cargo test unit"}).to_string();
        let pattern = bash_conversation_scope(&resource).unwrap();
        let mut rule = PermissionRule {
            capability_pattern: "bash".into(),
            resource_pattern: pattern,
            decision: PermissionPolicyDecision::Allow,
            source: PermissionRuleSource::Conversation,
        };
        for (command, cwd, expected) in [
            ("cargo test all", "/workspace", true),
            ("cargo build", "/workspace", false),
            ("cargo test all", "/other", false),
            ("cargo test; echo bad", "/workspace", false),
        ] {
            assert_eq!(
                permission_resource_matches(
                    &rule,
                    &PermissionSubject {
                        capability: "bash".into(),
                        resource: json!({"command":command,"cwd":cwd}).to_string()
                    }
                ),
                expected
            );
        }
        rule.capability_pattern = "ba*".into();
        assert!(!permission_resource_matches(
            &rule,
            &PermissionSubject {
                capability: "bash".into(),
                resource
            }
        ));
        assert!(wildcard_matches("cap.*", "cap.读取"));
        assert!(
            !wildcard_matches("?", "读"),
            "retain byte-oriented wildcard semantics"
        );
    }
    #[test]
    fn path_scope_keeps_component_boundaries_and_one_percent_decode() {
        for (requested, expected) in [
            ("/workspace/a", true),
            ("/workspace-other/a", false),
            ("file://localhost/workspace/a", true),
            ("file://remote/workspace/a", false),
            ("/workspace/%2e%2e/secret", false),
            ("/workspace/%GG", false),
            ("/workspace/%252e%252e/a", true),
        ] {
            assert_eq!(
                permission_path_scope_matches("/workspace", requested),
                expected,
                "{requested}"
            );
        }
    }
}
