// SPDX-License-Identifier: Apache-2.0

pub use nola_domain::{
    PermissionEvaluation, PermissionEvaluationReason, PermissionPolicyDecision, PermissionRule,
    PermissionRuleSource, PermissionSubject,
};
pub use nola_kernel::permission_contract_requires_confirmation;
#[cfg(test)]
use nola_kernel::permission_resource_matches;
use nola_kernel::{
    bash_conversation_scope, canonical_resource, BASH_COMMAND_GLOB_PREFIX,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub fn supported_shell_tokens(command: &str) -> Option<Vec<String>> {
    nola_kernel::safe_shell_tokens(command)
}

use super::protocol::{CallId, PermissionRequestId, ToolInvocationContext, TurnId};

pub use nola_domain::permission_settlement::{PermissionUserDecision, PermissionReceipt, PermissionReceiptOutcome};

/// User guidance supplied while a capability is suspended for permission.
/// This is not an authorization decision and can never create a grant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, tag = "type", rename_all = "snake_case")]
pub enum PermissionUserIntervention {
    ProvideInstruction { instruction: String },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionRuleSet {
    /// Rules are evaluated in order and the last matching rule wins.
    pub rules: Vec<PermissionRule>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PermissionContinuation {
    #[default]
    CoreTool,
    DelegatedAgent {
        occurrence: nola_host_protocol::DelegatedPermissionOccurrence,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        backend_permission_request_id: Option<PermissionRequestId>,
        target_effect_id: String,
        backend_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        backend_session_id: Option<String>,
    },
}

/// Durable, Host-independent description of one permission decision that is
/// waiting for the user. Hosts render this projection and answer by
/// `request_id`; they must not reconstruct requests from tool bubbles or
/// pending effects.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingPermissionRequest {
    /// Core-owned eligibility, never inferred from model input by a client.
    pub allow_conversation: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permission_instructions: Vec<nola_host_protocol::PermissionInstruction>,
    pub request_id: PermissionRequestId,
    pub turn_id: TurnId,
    pub call_id: CallId,
    pub capability_id: String,
    pub resource: String,
    pub arguments: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invocation_context: Option<ToolInvocationContext>,
    pub permission_domain: String,
    pub rationale: String,
    pub evaluation: PermissionEvaluation,
    #[serde(default)]
    pub continuation: PermissionContinuation,
}

/// Checkpointed permission state: pending challenges and resolved receipts.
/// Revision lets Hosts detect changes without deriving state from tool bubbles.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PermissionProjection {
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub pending: Vec<PendingPermissionRequest>,
    #[serde(default)]
    pub receipts: Vec<PermissionReceipt>,
}

impl PermissionProjection {
    pub fn validate_receipt_sources(&self) -> Result<(), String> {
        let mut sources = std::collections::HashMap::new();
        for receipt in &self.receipts {
            if sources.contains_key(&receipt.request_id) {
                return Err("duplicate permission receipt identity".into());
            }
            if let PermissionReceiptOutcome::PolicyApplied { policy } = &receipt.outcome {
                policy.validate()?;
                if policy.run_id != receipt.turn_id {
                    return Err("permission policy receipt belongs to another Run".into());
                }
            }
            if let PermissionReceiptOutcome::ConversationRuleApplied { source_request_id } = &receipt.outcome {
                if !sources.get(source_request_id).copied().unwrap_or(false) {
                    return Err("conversation permission reuse lacks its original authorization receipt".into());
                }
            }
            sources.insert(receipt.request_id.clone(), matches!(receipt.outcome,
                PermissionReceiptOutcome::Authorization {
                    decision: PermissionUserDecision::AllowConversation
                }));
        }
        Ok(())
    }

    pub fn resolve(
        &mut self,
        request_id: &PermissionRequestId,
        outcome: PermissionReceiptOutcome,
    ) -> bool {
        let Some(request) = self.pending.iter()
            .find(|request| &request.request_id == request_id).cloned() else {
            return false;
        };
        self.receipts.push(PermissionReceipt {
            request_id: request.request_id.as_str().into(),
            turn_id: request.turn_id.as_str().into(),
            call_id: request.call_id.as_str().into(),
            capability_id: request.capability_id,
            resource: request.resource,
            outcome,
        });
        self.remove(request_id)
    }

    pub fn interrupt_turn(&mut self, turn_id: &TurnId, reason: &str) {
        let calls = self.pending.iter()
            .filter(|request| &request.turn_id == turn_id)
            .map(|request| request.call_id.clone())
            .collect::<Vec<_>>();
        self.interrupt_calls(turn_id, &calls, reason);
    }

    /// Withdraw exact admitted calls, never every request sharing a tool name
    /// or permission domain. An interruption records no denial or new grant.
    pub fn interrupt_calls(&mut self, turn_id: &TurnId, calls: &[CallId], reason: &str) {
        let requests: Vec<_> = self.pending.iter()
            .filter(|request| &request.turn_id == turn_id && calls.contains(&request.call_id))
            .map(|request| request.request_id.clone())
            .collect();
        for request in requests {
            self.resolve(&request, PermissionReceiptOutcome::Interrupted {
                reason: reason.to_string(),
            });
        }
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub fn upsert(&mut self, request: PendingPermissionRequest) {
        if let Some(existing) = self
            .pending
            .iter_mut()
            .find(|existing| existing.request_id == request.request_id)
        {
            if existing != &request {
                *existing = request;
                self.revision = self.revision.saturating_add(1);
            }
            return;
        }
        self.pending.push(request);
        self.revision = self.revision.saturating_add(1);
    }

    pub fn remove(&mut self, request_id: &PermissionRequestId) -> bool {
        let previous_len = self.pending.len();
        self.pending
            .retain(|request| &request.request_id != request_id);
        let changed = self.pending.len() != previous_len;
        if changed {
            self.revision = self.revision.saturating_add(1);
        }
        changed
    }

    pub fn remove_turn(&mut self, turn_id: &TurnId) -> usize {
        let previous_len = self.pending.len();
        self.pending.retain(|request| &request.turn_id != turn_id);
        let removed = previous_len.saturating_sub(self.pending.len());
        if removed > 0 {
            self.revision = self.revision.saturating_add(1);
        }
        removed
    }
}

impl PermissionRuleSet {
    /// Only the explicit invocation-wide rule identifies allow_all. A scoped
    /// grant or a tool's default Allow never expands filesystem authority.
    pub fn profile(&self) -> nola_domain::PermissionProfile {
        if self.rules.iter().any(|rule| rule.source == PermissionRuleSource::InvocationPolicy
            && rule.capability_pattern == "*" && rule.resource_pattern == "*"
            && rule.decision == PermissionPolicyDecision::Allow) {
            nola_domain::PermissionProfile::AllowAll
        } else { nola_domain::PermissionProfile::Cautious }
    }

    /// Product mode is independent of tool defaults. Explicit hard denials are
    /// constraints; ask defaults cannot add approval prompts to allow_all.
    pub fn with_profile(&self, profile: nola_domain::PermissionProfile) -> Self {
        let mut rules = vec![PermissionRule {
            capability_pattern: "*".into(), resource_pattern: "*".into(),
            decision: PermissionPolicyDecision::Allow,
            source: if profile == nola_domain::PermissionProfile::AllowAll {
                PermissionRuleSource::InvocationPolicy
            } else { PermissionRuleSource::ProductDefault },
        }];
        rules.extend(self.rules.iter().filter(|rule| {
            rule.source != PermissionRuleSource::InvocationPolicy
                && !(rule.source == PermissionRuleSource::ProductDefault
                    && rule.capability_pattern == "*" && rule.resource_pattern == "*")
        }).cloned());
        Self { rules }
    }

    pub fn evaluate(&self, subject: &PermissionSubject) -> PermissionPolicyDecision {
        self.evaluate_with_explanation(subject).decision
    }

    pub fn evaluate_with_explanation(&self, subject: &PermissionSubject) -> PermissionEvaluation {
        self.evaluate_for_origin_with_explanation(
            subject,
            super::protocol::AgentInputOrigin::InteractiveUser,
        )
    }

    /// Evaluates permission policy for the provenance of the active Turn.
    /// Turn grants are evidence of a live interaction only. Conversation
    /// grants may also be used by a delegated child when the Host explicitly
    /// inherited the parent's structured conversation scope. Scheduled,
    /// replayed, and recovered input must rely on durable product, user-global,
    /// or Host policy instead.
    pub fn evaluate_for_origin(
        &self,
        subject: &PermissionSubject,
        origin: super::protocol::AgentInputOrigin,
    ) -> PermissionPolicyDecision {
        self.evaluate_for_origin_with_explanation(subject, origin)
            .decision
    }

    pub fn evaluate_for_origin_with_explanation(
        &self,
        subject: &PermissionSubject,
        origin: super::protocol::AgentInputOrigin,
    ) -> PermissionEvaluation {
        self.evaluate_resolved_for_origin_with_explanation(subject, origin,
            cautious_profile_requires_confirmation(subject))
    }

    pub fn evaluate_resolved_for_origin_with_explanation(
        &self, subject: &PermissionSubject, origin: super::protocol::AgentInputOrigin,
        confirmation_required: bool,
    ) -> PermissionEvaluation {
        self.evaluate_verified_resources_for_origin_with_explanation(subject,origin,confirmation_required,None)
    }

    pub fn evaluate_verified_resources_for_origin_with_explanation(
        &self, subject: &PermissionSubject, origin: super::protocol::AgentInputOrigin,
        confirmation_required: bool,
        resolved: Option<&nola_domain::permission_scope::ConcretePermissionSubject>,
    ) -> PermissionEvaluation {
        nola_kernel::evaluate_permission_with_resolved_resources(
            &self.rules,
            subject,
            nola_domain::PermissionAuthorityFacts {
                carries_live_user_authority: origin.carries_live_user_authority(),
                accepts_conversation_permission_grants: origin
                    .accepts_conversation_permission_grants(),
            },
            confirmation_required,
            resolved,
        )
    }

    pub fn append_scope(&mut self, rules: impl IntoIterator<Item = PermissionRule>) {
        self.rules.extend(rules);
    }

    /// Restores only conversation decisions from a checkpoint. Product, user,
    /// and Host policy come from the current configuration; Turn rules are
    /// ephemeral and must not survive a process boundary.
    pub fn merge_checkpoint_conversation_scope(&mut self, checkpoint: &Self) {
        for rule in checkpoint
            .rules
            .iter()
            .filter(|rule| rule.source == PermissionRuleSource::Conversation)
        {
            if !self.rules.contains(rule) {
                self.rules.push(rule.clone());
            }
        }
    }
}

/// A subject without its frozen operation contract carries no read-only fact.
/// Actual tool admission uses evaluate_resolved_for_origin_with_explanation.
pub fn cautious_profile_requires_confirmation(_subject: &PermissionSubject) -> bool {
    true
}

fn invocation_requests_irreversible_change(resource: &Value) -> bool {
    let Some(object) = resource.as_object() else {
        return false;
    };
    if [
        "overwrite",
        "force",
        "permanent",
        "remove_new_untracked",
        "discard_changes",
    ]
    .iter()
    .any(|key| object.get(*key).and_then(Value::as_bool) == Some(true))
    {
        return true;
    }
    ["action", "operation", "edit_mode"]
        .iter()
        .filter_map(|key| object.get(*key).and_then(Value::as_str))
        .map(|value| value.trim().to_ascii_lowercase())
        .any(|value| is_irreversible_operation(&value))
}

fn is_irreversible_operation(value: &str) -> bool {
    matches!(
        value,
        "delete"
            | "remove"
            | "erase"
            | "purge"
            | "destroy"
            | "wipe"
            | "truncate"
            | "drop"
            | "reset"
            | "revoke"
            | "clear"
            | "discard"
    )
}

/// Builds the stable, concrete resource used by Core permission policy.
///
/// A permission domain such as `filesystem.write` describes why approval is
/// required, but it is not an authorization boundary. The boundary is the
/// actual destination path, command, URL, query, or other model-produced
/// input. Keeping this derivation in Core prevents every Host from inventing
/// a subtly different rule language.
pub fn permission_resource_for_input(
    capability: &str,
    permission_domain: Option<&str>,
    input: &Value,
) -> String {
    let Some(object) = input.as_object() else {
        return canonical_resource(input);
    };
    let capability = capability.to_ascii_lowercase();
    let _permission_domain = permission_domain;

    if capability == "web_search" {
        return "web_search".to_string();
    }

    // Risk-sensitive tools need their operation flags at policy time. A path
    // alone cannot distinguish a harmless preview from an irreversible
    // delete/restore. Keep the full canonical invocation as the resource.
    if permission_resource_requires_full_input(&capability)
        || invocation_requests_irreversible_change(input)
    {
        return canonical_resource(input);
    }

    if matches!(
        capability.as_str(),
        "bash" | "bash_background_start" | "bash_command_review"
    ) {
        let command = ["command", "cmd", "script"]
            .iter()
            .find_map(|key| object.get(*key))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        let cwd = object
            .get("cwd")
            .and_then(Value::as_str)
            .unwrap_or(".")
            .trim();
        return json!({"command": command, "cwd": cwd}).to_string();
    }

    let keys = permission_resource_keys(capability.as_str());
    if keys.is_empty() {
        // Unknown and third-party capabilities are bound to the complete
        // invocation. Their names must never weaken or broaden a grant.
        return canonical_resource(input);
    }

    keys.iter()
        .find_map(|key| object.get(*key))
        .filter(|value| !value.is_null())
        .map(canonical_resource)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| canonical_resource(input))
}

/// Converts a user-facing permission pattern into Core's canonical resource
/// pattern. Shell rules are configured as command globs (for example
/// `git status*`), while the evaluated resource also carries the working
/// directory. Keep that translation in Core so every Host applies the same
/// semantics without matching serialized JSON by hand.
pub fn permission_resource_pattern_for_rule(capability: &str, pattern: &str) -> String {
    if matches!(
        capability.to_ascii_lowercase().as_str(),
        "bash" | "bash_background_start" | "bash_command_review"
    ) {
        return format!(
            "{BASH_COMMAND_GLOB_PREFIX}{}",
            serde_json::to_string(&json!({"command_pattern": pattern})).unwrap_or_default()
        );
    }
    pattern.to_string()
}

/// Selects resource fields only from the structured capability contract.
///
/// Domain is Host-declared catalog metadata. Capability IDs are matched only
/// for Nola's versioned built-ins; arbitrary names never acquire permission
/// semantics from words such as read, write, memory, or web.
pub fn permission_resource_keys(capability: &str) -> &'static [&'static str] {
    let capability = capability.trim().to_ascii_lowercase();
    if matches!(
        capability.as_str(),
        "write_file"
            | "append_file"
            | "create_file"
            | "delete_file"
            | "edit_file"
            | "edit_file_lines"
            | "apply_patch"
            | "multi_edit_file"
            | "replace_in_files"
            | "replace_symbol_body"
            | "rename_symbol"
            | "move_file"
            | "copy_file"
            | "create_directory"
            | "format_files"
    ) {
        &["destination", "path", "root", "cwd", "source"]
    } else if matches!(
        capability.as_str(),
        "read_file"
            | "read_many_files"
            | "list_directory"
            | "list_dir"
            | "file_outline"
            | "file_tree"
            | "find_files"
            | "find_up"
            | "glob"
            | "grep"
            | "search_files"
            | "code_search"
            | "project_overview"
            | "repo_map"
            | "read_symbol"
            | "symbol_search"
            | "stat_file"
    ) {
        &["path", "root", "cwd", "source", "paths"]
    } else if matches!(capability.as_str(), "web_search" | "web_fetch") {
        &["url", "query", "domain", "host", "target"]
    } else if matches!(
        capability.as_str(),
        "skill_context" | "skill_invoke" | "skill_list" | "skill_skip"
    ) {
        &["skill_id", "name"]
    } else if matches!(
        capability.as_str(),
        "memory_search"
    ) {
        &["id", "scope", "kind", "query"]
    } else {
        &[]
    }
}

/// Builds the Host-owned resource declaration for Nola's versioned built-in
/// catalog. Unknown capability IDs deliberately use their complete canonical
/// invocation; they never inherit semantics from a name prefix.
pub fn permission_resource_declaration_for_builtin(
    capability: &str,
) -> super::host::HostPermissionResourceDeclaration {
    use super::host::HostPermissionResourceDeclaration;

    match capability.trim().to_ascii_lowercase().as_str() {
        // These calls can affect multiple resources or carry a patch containing
        // paths. A FirstOf projection loses authority-relevant input. Until
        // typed Host resolution is wired, retain the entire invocation.
        capability if permission_resource_requires_full_input(capability) =>
            HostPermissionResourceDeclaration::FullInput,
        "web_search" => HostPermissionResourceDeclaration::Constant {
            value: "web_search".to_string(),
        },
        "memory_search" => HostPermissionResourceDeclaration::Constant { value:"personal-memory:read".into() },
        "search_calendar_events" => HostPermissionResourceDeclaration::Constant { value:"platform:calendar:read".into() },
        "search_reminders" => HostPermissionResourceDeclaration::Constant { value:"platform:reminders:read".into() },
        "search_contacts" => HostPermissionResourceDeclaration::Constant { value:"platform:contacts:read".into() },
        "bash" | "bash_background_start" | "bash_command_review" => {
            HostPermissionResourceDeclaration::Composite {
                keys: vec!["command".to_string(), "cwd".to_string()],
            }
        }
        capability => {
            let keys = permission_resource_keys(capability);
            if keys.is_empty() {
                HostPermissionResourceDeclaration::FullInput
            } else {
                HostPermissionResourceDeclaration::FirstOf {
                    keys: keys.iter().map(|key| (*key).to_string()).collect(),
                }
            }
        }
    }
}

fn permission_resource_requires_full_input(capability: &str) -> bool {
    matches!(capability,
        "delete_file" | "workspace_checkpoint" | "notebook_edit"
            | "move_file" | "copy_file" | "read_many_files" | "format_files"
            | "replace_in_files" | "apply_patch")
}

/// Builds the durable conversation grant for the concrete permission subject.
///
/// Most capabilities preserve the exact Host-declared resource. Bash uses the
/// versioned Core scope format below so an explicit conversation decision can
/// cover the same command family in the same working directory without
/// silently covering a different executable or complex shell expression.
pub fn conversation_permission_rule(capability: &str, resource: String) -> (String, String) {
    let resource = if matches!(capability, "bash" | "shell.execute") {
        bash_conversation_scope(&resource).unwrap_or(resource)
    } else {
        resource
    };
    (capability.to_string(), resource)
}

#[cfg(test)]
mod recommended_profile_tests {
    use super::{
        permission_resource_for_input, PermissionPolicyDecision, PermissionRule, PermissionRuleSet,
        PermissionRuleSource, PermissionSubject,
    };
    use serde_json::json;

    fn recommended_rules() -> PermissionRuleSet {
        PermissionRuleSet {
            rules: vec![PermissionRule {
                capability_pattern: "*".to_string(),
                resource_pattern: "*".to_string(),
                decision: PermissionPolicyDecision::Allow,
                source: PermissionRuleSource::ProductDefault,
            }],
        }
    }

    #[test]
    fn recommended_profile_requires_explicit_workspace_access() {
        let subject = PermissionSubject {
            capability: "workspace_access".to_string(),
            resource: json!({"root": "/external/project"}).to_string(),
        };
        assert_eq!(recommended_rules().evaluate(&subject), PermissionPolicyDecision::Ask);
    }

    #[test]
    fn trusted_ordinary_operation_facts_allow_work_without_resource_string_heuristics() {
        let rules = recommended_rules();
        for subject in [
            PermissionSubject {
                capability: "web_search".to_string(),
                resource: "web_search".to_string(),
            },
            PermissionSubject {
                capability: "read_file".to_string(),
                resource: "/tmp/source.md".to_string(),
            },
            PermissionSubject {
                capability: "write_file".to_string(),
                resource: "/tmp/report.md".to_string(),
            },
            PermissionSubject {
                capability: "web_fetch".to_string(),
                resource: "https://example.com".to_string(),
            },
        ] {
            assert_eq!(
                rules.evaluate_resolved_for_origin_with_explanation(&subject,
                    crate::agent::AgentInputOrigin::InteractiveUser, false).decision,
                PermissionPolicyDecision::Allow,
                "{} is ordinary reversible work and should not interrupt native clients",
                subject.capability
            );
        }
    }

    #[test]
    fn recommended_profile_asks_before_delete_and_destructive_shell() {
        let rules = recommended_rules();
        assert_eq!(
            rules.evaluate(&PermissionSubject {
                capability: "delete_file".to_string(),
                resource: "/tmp/report.md".to_string(),
            }),
            PermissionPolicyDecision::Ask
        );
        assert_eq!(
            rules.evaluate(&PermissionSubject {
                capability: "bash".to_string(),
                resource: json!({"command": "rm -rf build", "cwd": "/tmp"}).to_string(),
            }),
            PermissionPolicyDecision::Ask
        );
    }

    #[test]
    fn recommended_profile_requires_process_authority_even_for_apparently_safe_commands() {
        let rules = recommended_rules();
        for command in [
            "osascript -e 'tell application \"Mail\" to get subject of first message'",
            "/usr/bin/python3 /Applications/Nola.app/Contents/Resources/word/scripts/create_document.py",
            "cargo test --quiet",
            "git diff -- README.md",
        ] {
            assert_eq!(
                rules.evaluate(&PermissionSubject {
                    capability: "bash".to_string(),
                    resource: json!({"command": command, "cwd": "/tmp"}).to_string(),
                }),
                PermissionPolicyDecision::Ask,
                "a process needs concrete authorization: {command}"
            );
        }
    }

    #[test]
    fn recommended_profile_still_stops_irreversible_native_automation() {
        let rules = recommended_rules();
        for command in [
            "osascript -e 'tell application \"Mail\" to delete first message'",
            "rm -rf build",
            "git reset --hard HEAD~1",
            "find . -name '*.tmp' -delete",
        ] {
            assert_eq!(
                rules.evaluate(&PermissionSubject {
                    capability: "bash".to_string(),
                    resource: json!({"command": command, "cwd": "/tmp"}).to_string(),
                }),
                PermissionPolicyDecision::Ask,
                "irreversible native command must wait for consent: {command}"
            );
        }
    }

    #[test]
    fn unresolved_moves_require_confirmation_even_when_the_model_disables_overwrite() {
        let rules = recommended_rules();
        for (overwrite, expected) in [
            (false, PermissionPolicyDecision::Ask),
            (true, PermissionPolicyDecision::Ask),
        ] {
            let input = json!({
                "source": "/tmp/source.md",
                "destination": "/tmp/destination.md",
                "overwrite": overwrite,
                "dry_run": false
            });
            let resource =
                permission_resource_for_input("move_file", Some("filesystem.write"), &input);
            assert_eq!(
                rules.evaluate(&PermissionSubject {
                    capability: "move_file".to_string(),
                    resource,
                }),
                expected
            );
        }
    }

    #[test]
    fn recommended_profile_uses_concrete_action_instead_of_user_wording() {
        let rules = recommended_rules();
        for action in ["drop", "reset", "revoke", "clear"] {
            let input = json!({"action": action, "target": "saved-state"});
            let resource =
                permission_resource_for_input("manage_saved_state", Some("state.write"), &input);
            assert_eq!(
                rules.evaluate(&PermissionSubject {
                    capability: "manage_saved_state".to_string(),
                    resource,
                }),
                PermissionPolicyDecision::Ask,
                "action={action} should require confirmation"
            );
        }
        assert_eq!(
            rules.evaluate(&PermissionSubject {
                capability: "write_file".to_string(),
                resource: "/tmp/ordinary-output.md".to_string(),
            }),
            PermissionPolicyDecision::Ask,
            "a resource string does not prove the target is a new file"
        );
    }

    #[test]
    fn model_preview_flags_without_a_trusted_contract_do_not_grant_authority() {
        let rules = recommended_rules();
        assert_eq!(
            rules.evaluate(&PermissionSubject {
                capability: "delete_file".to_string(),
                resource: json!({"path":"report.md","dry_run":true}).to_string(),
            }),
            PermissionPolicyDecision::Ask
        );
        assert_eq!(
            rules.evaluate(&PermissionSubject {
                capability: "delete_file".to_string(),
                resource: json!({"path":"report.md","dry_run":false}).to_string(),
            }),
            PermissionPolicyDecision::Ask
        );
    }

    #[test]
    fn recommended_profile_only_asks_for_mutating_restore_and_notebook_delete() {
        let rules = recommended_rules();
        for subject in [
            PermissionSubject {
                capability: "workspace_checkpoint".to_string(),
                resource: json!({"action":"restore","dry_run":false}).to_string(),
            },
            PermissionSubject {
                capability: "notebook_edit".to_string(),
                resource: json!({"path":"analysis.ipynb","edit_mode":"delete"}).to_string(),
            },
        ] {
            assert_eq!(rules.evaluate(&subject), PermissionPolicyDecision::Ask);
        }
        assert_eq!(
            rules.evaluate(&PermissionSubject {
                capability: "workspace_checkpoint".to_string(),
                resource: json!({"action":"restore","dry_run":true}).to_string(),
            }),
            PermissionPolicyDecision::Ask
        );
        assert_eq!(
            rules.evaluate(&PermissionSubject {
                capability: "workspace_checkpoint".to_string(),
                resource: json!({"action":"restore"}).to_string(),
            }),
            PermissionPolicyDecision::Ask,
            "preview defaults must come from the tool contract"
        );
    }

    #[test]
    fn recommended_profile_requires_device_and_process_authority_for_lifecycle_actions() {
        let rules = recommended_rules();
        for capability in [
            "bash_background_stop",
            "terminal_input",
            "computer_quit_app",
            "computer_close_window",
        ] {
            assert_eq!(
                rules.evaluate(&PermissionSubject {
                    capability: crate::agent::policy::permission_name_for_tool(capability),
                    resource: "*".to_string(),
                }),
                PermissionPolicyDecision::Ask,
                "{capability} still controls a device or process"
            );
        }
    }

    #[test]
    fn recommended_profile_requires_authority_for_private_mail_read_and_delete() {
        let rules = recommended_rules();
        assert_eq!(
            rules.evaluate(&PermissionSubject {
                capability: "bash".to_string(),
                resource: json!({
                    "command": "osascript -e 'tell application \"Mail\" to get subject of every message'",
                    "cwd": "."
                })
                .to_string(),
            }),
            PermissionPolicyDecision::Ask
        );
        assert_eq!(
            rules.evaluate(&PermissionSubject {
                capability: "bash".to_string(),
                resource: json!({
                    "command": "osascript -e 'tell application \"Mail\" to delete every message of mailbox \"Trash\"'",
                    "cwd": "."
                })
                .to_string(),
            }),
            PermissionPolicyDecision::Ask
        );
    }

    #[test]
    fn recommended_profile_asks_before_any_ungranted_git_process() {
        let rules = recommended_rules();
        for command in ["git clean -n", "git reset --soft HEAD~1"] {
            assert_eq!(
                rules.evaluate(&PermissionSubject {
                    capability: "bash".to_string(),
                    resource: json!({"command": command, "cwd": "/tmp/project"}).to_string(),
                }),
                PermissionPolicyDecision::Ask,
                "{command} still starts an ungranted process"
            );
        }
        for command in ["git clean -fd", "git reset --hard HEAD~1"] {
            assert_eq!(
                rules.evaluate(&PermissionSubject {
                    capability: "bash".to_string(),
                    resource: json!({"command": command, "cwd": "/tmp/project"}).to_string(),
                }),
                PermissionPolicyDecision::Ask,
                "{command} can discard data"
            );
        }
    }

    #[test]
    fn explicit_conversation_allow_overrides_destructive_default() {
        let mut rules = recommended_rules();
        rules.rules.push(PermissionRule {
            capability_pattern: "delete_file".to_string(),
            resource_pattern: "/tmp/report.md".to_string(),
            decision: PermissionPolicyDecision::Allow,
            source: PermissionRuleSource::Conversation,
        });
        assert_eq!(
            rules.evaluate(&PermissionSubject {
                capability: "delete_file".to_string(),
                resource: "/tmp/report.md".to_string(),
            }),
            PermissionPolicyDecision::Allow
        );
        assert_eq!(rules.evaluate(&PermissionSubject {
            capability: "delete_file".to_string(),
            resource: "/tmp/other.md".to_string(),
        }), PermissionPolicyDecision::Ask);
    }

    #[test]
    fn private_domains_require_consent_even_with_a_dry_run_flag() {
        for capability in ["camera", "microphone", "photos", "location", "calendar",
            "reminders", "contacts", "health", "clipboard", "computer", "browser", "notifications", "bash",
            "memory_search", "memory_save", "memory_update", "memory_delete"] {
            assert_eq!(recommended_rules().evaluate(&PermissionSubject {
                capability: capability.into(), resource: json!({"dry_run":true}).to_string(),
            }), PermissionPolicyDecision::Ask, "{capability}");
        }
    }

    #[test]
    fn private_consent_is_bound_to_the_actual_resource_and_can_be_denied() {
        let mut rules = recommended_rules();
        let subject = PermissionSubject { capability: "contacts".into(), resource: "alice".into() };
        rules.rules.push(PermissionRule { capability_pattern: "contacts".into(),
            resource_pattern: "alice".into(), decision: PermissionPolicyDecision::Allow,
            source: PermissionRuleSource::Conversation });
        assert_eq!(rules.evaluate(&subject), PermissionPolicyDecision::Allow);
        assert_eq!(rules.evaluate(&PermissionSubject { resource: "bob".into(), ..subject.clone() }),
            PermissionPolicyDecision::Ask);
        rules.rules.push(PermissionRule { capability_pattern: "contacts".into(),
            resource_pattern: "alice".into(), decision: PermissionPolicyDecision::Deny,
            source: PermissionRuleSource::UserGlobal });
        assert_eq!(rules.evaluate(&subject), PermissionPolicyDecision::Deny);
    }

    #[test]
    fn memory_write_consent_cannot_cover_different_content_in_the_same_scope() {
        let first = json!({"scope":"global", "content":"first explicit fact"});
        let second = json!({"scope":"global", "content":"different private fact"});
        let resource = permission_resource_for_input("memory_save", Some("memory_save"), &first);
        assert_ne!(resource, permission_resource_for_input("memory_save", Some("memory_save"), &second));
        assert!(matches!(super::permission_resource_declaration_for_builtin("memory_save"),
            crate::agent::host::HostPermissionResourceDeclaration::FullInput));
    }
}

#[cfg(test)]
mod permission_receipt_source_tests {
    use super::*;

    fn receipt(id: &str, outcome: PermissionReceiptOutcome) -> PermissionReceipt {
        PermissionReceipt { request_id: id.into(), turn_id: "turn".into(), call_id: id.into(),
            capability_id: "read_file".into(), resource: "/project".into(), outcome }
    }

    #[test]
    fn reuse_requires_unique_prior_original_authorization() {
        let original = receipt("original", PermissionReceiptOutcome::Authorization {
            decision: PermissionUserDecision::AllowConversation });
        let reused = receipt("reused", PermissionReceiptOutcome::ConversationRuleApplied {
            source_request_id: "original".into() });
        let projection = |receipts| PermissionProjection { receipts, ..Default::default() };
        assert!(projection(vec![original.clone(), reused.clone()]).validate_receipt_sources().is_ok());
        assert!(projection(vec![reused.clone(), original.clone()]).validate_receipt_sources().is_err());
        assert!(projection(vec![original.clone(), reused.clone(), original.clone()]).validate_receipt_sources().is_err());
        let chained = receipt("chained", PermissionReceiptOutcome::ConversationRuleApplied {
            source_request_id: "reused".into() });
        assert!(projection(vec![original, reused, chained]).validate_receipt_sources().is_err());
    }
}

#[cfg(test)]
mod permission_path_scope_tests {
    use super::{permission_resource_matches, PermissionRule, PermissionRuleSource};
    use crate::agent::permissions::{PermissionPolicyDecision, PermissionSubject};

    #[test]
    fn conversation_edit_directory_covers_descendant_files_only() {
        let rule = PermissionRule {
            capability_pattern: "edit".to_string(),
            resource_pattern: "/workspace/project".to_string(),
            decision: PermissionPolicyDecision::Allow,
            source: PermissionRuleSource::Conversation,
        };
        assert!(permission_resource_matches(
            &rule,
            &PermissionSubject {
                capability: "edit".to_string(),
                resource: "/workspace/project/src/main.rs".to_string(),
            },
        ));
        assert!(!permission_resource_matches(
            &rule,
            &PermissionSubject {
                capability: "edit".to_string(),
                resource: "/workspace/project-other/main.rs".to_string(),
            },
        ));
    }

    #[test]
    fn conversation_file_url_scope_covers_the_same_local_resource() {
        let rule = PermissionRule {
            capability_pattern: "files_read".to_string(),
            resource_pattern: "file:///tmp/reference.txt".to_string(),
            decision: PermissionPolicyDecision::Allow,
            source: PermissionRuleSource::Conversation,
        };
        assert!(permission_resource_matches(
            &rule,
            &PermissionSubject {
                capability: "files_read".to_string(),
                resource: "file:///tmp/reference.txt".to_string(),
            },
        ));
        assert!(!permission_resource_matches(
            &rule,
            &PermissionSubject {
                capability: "files_read".to_string(),
                resource: "file://remote.example/tmp/reference.txt".to_string(),
            },
        ));
    }

    #[test]
    fn conversation_file_url_scope_rejects_encoded_parent_traversal() {
        let rule = PermissionRule {
            capability_pattern: "files_read".to_string(),
            resource_pattern: "file:///tmp/workspace".to_string(),
            decision: PermissionPolicyDecision::Allow,
            source: PermissionRuleSource::Conversation,
        };
        assert!(!permission_resource_matches(
            &rule,
            &PermissionSubject {
                capability: "files_read".to_string(),
                resource: "file:///tmp/workspace/%2E%2E/secret.txt".to_string(),
            },
        ));
    }
}

/// Parses one simple shell command for permission scoping. Operators,
/// substitutions, newlines, and unbalanced quotes reject prefix reuse.

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn call_interruption_preserves_sibling_and_other_turn_challenges() {
        let request = |id: &str, turn: &str, call: &str| PendingPermissionRequest {
            allow_conversation:true, permission_instructions:Vec::new(),
            request_id:PermissionRequestId::from_string(id.into()),
            turn_id:TurnId::from_string(turn.into()), call_id:CallId::from_string(call.into()),
            capability_id:"write_file".into(), resource:"same-path".into(),
            arguments:json!({"path":"same-path"}), invocation_context:None,
            permission_domain:"filesystem.write".into(), rationale:"test".into(),
            evaluation:PermissionEvaluation { decision:PermissionPolicyDecision::Ask,
                reason:PermissionEvaluationReason::NoMatchingRule, rule_index:None, rule:None },
            continuation:PermissionContinuation::CoreTool,
        };
        let mut projection = PermissionProjection::default();
        projection.upsert(request("target", "turn", "child"));
        projection.upsert(request("sibling", "turn", "sibling"));
        projection.upsert(request("other-turn", "other", "child"));
        let turn = TurnId::from_string("turn".into());
        let calls = [CallId::from_string("child".into())];
        projection.interrupt_calls(&turn, &calls, "cell stopped");
        assert_eq!(projection.pending.len(), 2);
        assert_eq!(projection.receipts.len(), 1);
        assert_eq!(projection.receipts[0].request_id, "target");
        assert!(matches!(&projection.receipts[0].outcome,
            PermissionReceiptOutcome::Interrupted { reason } if reason == "cell stopped"));
        let revision = projection.revision;
        projection.interrupt_calls(&turn, &calls, "cell stopped");
        assert_eq!(projection.revision, revision);
        assert_eq!(projection.receipts.len(), 1);
        assert!(projection.pending.iter().any(|request| request.request_id.as_str() == "sibling"));
        assert!(projection.pending.iter().any(|request| request.request_id.as_str() == "other-turn"));
    }

    #[test]
    fn multi_resource_invocation_preserves_source_and_destination() {
        assert_eq!(
            permission_resource_for_input(
                "move_file",
                Some("filesystem.write"),
                &json!({"source": "draft.txt", "destination": "out/final.txt"}),
            ),
            canonical_resource(&json!({"source": "draft.txt", "destination": "out/final.txt"}))
        );
    }

    #[test]
    fn web_search_permission_does_not_depend_on_query_wording() {
        assert_eq!(
            permission_resource_for_input(
                "web_search",
                Some("network.read"),
                &json!({"query": "Iran July 2026"}),
            ),
            "web_search"
        );
        assert_eq!(
            permission_resource_for_input(
                "web_search",
                Some("network.read"),
                &json!({"query": "检索长期记忆"}),
            ),
            "web_search"
        );
    }

    #[test]
    fn conversation_grant_preserves_the_frozen_permission_subject() {
        assert_eq!(
            conversation_permission_rule("web_search", "web_search".to_string()),
            ("web_search".to_string(), "web_search".to_string())
        );
        assert_eq!(
            conversation_permission_rule("filesystem.write", "notes/a.md".to_string()),
            ("filesystem.write".to_string(), "notes/a.md".to_string())
        );
    }

    #[test]
    fn conversation_rule_covers_only_the_approved_capability_and_resource() {
        let rules = PermissionRuleSet {
            rules: vec![PermissionRule {
                capability_pattern: "write_file".to_string(),
                resource_pattern: "notes/today.md".to_string(),
                decision: PermissionPolicyDecision::Allow,
                source: PermissionRuleSource::Conversation,
            }],
        };
        assert_eq!(
            rules.evaluate(&PermissionSubject {
                capability: "write_file".to_string(),
                resource: "notes/today.md".to_string(),
            }),
            PermissionPolicyDecision::Allow
        );
        assert_eq!(
            rules.evaluate(&PermissionSubject {
                capability: "write_file".to_string(),
                resource: "notes/tomorrow.md".to_string(),
            }),
            PermissionPolicyDecision::Ask
        );
        assert_eq!(
            rules.evaluate(&PermissionSubject {
                capability: "bash".to_string(),
                resource: "notes/today.md".to_string(),
            }),
            PermissionPolicyDecision::Ask
        );
    }

    #[test]
    fn checkpoint_merge_keeps_only_conversation_rules_after_current_policy() {
        let conversation = PermissionRule {
            capability_pattern: "write_file".to_string(),
            resource_pattern: "*".to_string(),
            decision: PermissionPolicyDecision::Allow,
            source: PermissionRuleSource::Conversation,
        };
        let mut current = PermissionRuleSet {
            rules: vec![PermissionRule {
                capability_pattern: "*".to_string(),
                resource_pattern: "*".to_string(),
                decision: PermissionPolicyDecision::Ask,
                source: PermissionRuleSource::ProductDefault,
            }],
        };
        current.merge_checkpoint_conversation_scope(&PermissionRuleSet {
            rules: vec![
                conversation.clone(),
                PermissionRule {
                    source: PermissionRuleSource::Turn,
                    ..conversation.clone()
                },
            ],
        });
        assert_eq!(current.rules.len(), 2);
        assert_eq!(current.rules.last(), Some(&conversation));
    }

    #[test]
    fn conversation_bash_grant_reuses_only_the_same_safe_script_entrypoint() {
        let first = permission_resource_for_input(
            "bash",
            Some("bash"),
            &json!({"command": "/usr/bin/python3 write_fixture.py first.txt first", "cwd": "/workspace"}),
        );
        let (capability_pattern, resource_pattern) = conversation_permission_rule("bash", first);
        let rules = PermissionRuleSet {
            rules: vec![PermissionRule {
                capability_pattern,
                resource_pattern,
                decision: PermissionPolicyDecision::Allow,
                source: PermissionRuleSource::Conversation,
            }],
        };
        let subject = |command: &str| PermissionSubject {
            capability: "bash".to_string(),
            resource: permission_resource_for_input(
                "bash",
                Some("bash"),
                &json!({"command": command, "cwd": "/workspace"}),
            ),
        };
        assert_eq!(
            rules.evaluate(&subject(
                "/usr/bin/python3 write_fixture.py second.txt second"
            )),
            PermissionPolicyDecision::Allow
        );
        assert_eq!(
            rules.evaluate(&subject("/usr/bin/python3 other_writer.py other.txt other")),
            PermissionPolicyDecision::Ask
        );
        assert_eq!(
            rules.evaluate(&subject(
                "/usr/bin/python3 write_fixture.py third.txt third; rm -rf build"
            )),
            PermissionPolicyDecision::Ask
        );
    }

    #[test]
    fn conversation_bash_grant_reuses_one_destructive_command_family_only() {
        let first = permission_resource_for_input(
            "bash",
            Some("shell.execute"),
            &json!({"command": "rm first.txt"}),
        );
        let (capability_pattern, resource_pattern) = conversation_permission_rule("bash", first);
        let rules = PermissionRuleSet {
            rules: vec![PermissionRule {
                capability_pattern,
                resource_pattern,
                decision: PermissionPolicyDecision::Allow,
                source: PermissionRuleSource::Conversation,
            }],
        };
        let subject = |command: &str| PermissionSubject {
            capability: "bash".to_string(),
            resource: permission_resource_for_input(
                "bash",
                Some("shell.execute"),
                &json!({"command": command}),
            ),
        };

        assert_eq!(
            rules.evaluate(&subject("rm second.txt")),
            PermissionPolicyDecision::Allow
        );
        assert_eq!(
            rules.evaluate(&subject("find . -name other.txt -delete")),
            PermissionPolicyDecision::Ask
        );
    }

    #[test]
    fn shell_execute_domain_uses_the_versioned_bash_conversation_scope() {
        let first = permission_resource_for_input(
            "bash",
            Some("shell.execute"),
            &json!({"command": "rm first.txt", "cwd": "/workspace"}),
        );
        let (capability_pattern, resource_pattern) =
            conversation_permission_rule("shell.execute", first);
        let rules = PermissionRuleSet {
            rules: vec![PermissionRule {
                capability_pattern,
                resource_pattern,
                decision: PermissionPolicyDecision::Allow,
                source: PermissionRuleSource::Conversation,
            }],
        };
        let subject = |command: &str, cwd: &str| PermissionSubject {
            capability: "shell.execute".to_string(),
            resource: permission_resource_for_input(
                "bash",
                Some("shell.execute"),
                &json!({"command": command, "cwd": cwd}),
            ),
        };

        assert_eq!(
            rules.evaluate(&subject("rm second.txt", "/workspace")),
            PermissionPolicyDecision::Allow
        );
        assert_eq!(
            rules.evaluate(&subject("rm second.txt", "/other")),
            PermissionPolicyDecision::Ask
        );
        assert_eq!(
            rules.evaluate(&subject(
                "find . -maxdepth 1 -name second.txt -delete",
                "/workspace"
            )),
            PermissionPolicyDecision::Ask
        );

        let implicit_first = permission_resource_for_input(
            "bash",
            Some("shell.execute"),
            &json!({"command": "rm first.txt"}),
        );
        let (capability_pattern, resource_pattern) =
            conversation_permission_rule("shell.execute", implicit_first);
        let implicit_rules = PermissionRuleSet {
            rules: vec![PermissionRule {
                capability_pattern,
                resource_pattern,
                decision: PermissionPolicyDecision::Allow,
                source: PermissionRuleSource::Conversation,
            }],
        };
        assert_eq!(
            implicit_rules.evaluate(&PermissionSubject {
                capability: "shell.execute".to_string(),
                resource: permission_resource_for_input(
                    "bash",
                    Some("shell.execute"),
                    &json!({"command": "rm second.txt"}),
                ),
            }),
            PermissionPolicyDecision::Allow
        );
    }

    #[test]
    fn user_bash_command_glob_matches_canonical_resources_across_working_directories() {
        let rules = PermissionRuleSet {
            rules: vec![PermissionRule {
                capability_pattern: "bash".to_string(),
                resource_pattern: permission_resource_pattern_for_rule("bash", "git status*"),
                decision: PermissionPolicyDecision::Allow,
                source: PermissionRuleSource::UserGlobal,
            }],
        };
        let subject = |command: &str, cwd: &str| PermissionSubject {
            capability: "bash".to_string(),
            resource: permission_resource_for_input(
                "bash",
                Some("bash"),
                &json!({"command": command, "cwd": cwd}),
            ),
        };

        assert_eq!(
            rules.evaluate(&subject("git status --short", "/workspace/one")),
            PermissionPolicyDecision::Allow
        );
        assert_eq!(
            rules.evaluate(&subject("git status", "/workspace/two")),
            PermissionPolicyDecision::Allow
        );
        assert_eq!(
            rules.evaluate(&subject("git diff", "/workspace/one")),
            PermissionPolicyDecision::Ask
        );
    }
}
