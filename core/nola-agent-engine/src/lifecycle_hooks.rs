// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Plan trusted, observation-only lifecycle Hook invocations for platform Hosts.
// - Boundary: Core validates and matches configuration; Hosts perform all I/O and permission checks.
// - Security: This version cannot block, mutate tool input/output, or widen permissions.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const LIFECYCLE_HOOK_SCHEMA_VERSION: u32 = 2;
pub const LIFECYCLE_HOOK_ABI_VERSION: u32 = 4;
const MAX_RULES: usize = 128;
const MAX_HANDLER_BINDINGS: usize = 128;
const MAX_MATCHER_TOOL_NAMES: usize = 64;
const MAX_IDENTIFIER_CHARS: usize = 128;
const MIN_TIMEOUT_MS: u64 = 100;
const MAX_TIMEOUT_MS: u64 = 120_000;
const MAX_EXECUTION_DURATION_MS: u64 = 180_000;
const MAX_OUTPUT_PREVIEW_CHARS: usize = 4_096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleHookEventName {
    SessionStart,
    UserPromptSubmit,
    PreToolUse,
    PostToolUse,
    ToolError,
    TurnEnd,
    SessionEnd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleHookSource {
    Managed,
    Product,
    User,
    Workspace,
    Plugin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleHookTrustStatus {
    Managed,
    Trusted,
    Untrusted,
    Modified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleHookHandlerKind {
    Command,
    Http,
    Prompt,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleHookExecutionMode {
    Sync,
    Async,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleHookRequiredCapability {
    None,
    Process,
    Network,
    Model,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LifecycleHookMatcher {
    /// Exact canonical tool identifiers. `*` is the only wildcard. Display
    /// labels, translations, substrings, and alternate aliases never match.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LifecycleHookHandlerReference {
    pub id: String,
    pub kind: LifecycleHookHandlerKind,
    pub execution_mode: LifecycleHookExecutionMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LifecycleHookRule {
    pub id: String,
    pub event: LifecycleHookEventName,
    pub source: LifecycleHookSource,
    pub trust: LifecycleHookTrustStatus,
    pub handler: LifecycleHookHandlerReference,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub matcher: LifecycleHookMatcher,
    #[serde(default)]
    pub display_order: i64,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

/// A Host-owned executable binding advertised to Core. Execution details such
/// as commands, URLs, credentials, callbacks, and prompt text deliberately do
/// not cross this boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LifecycleHookHandlerBinding {
    pub handler_id: String,
    pub kind: LifecycleHookHandlerKind,
    pub source: LifecycleHookSource,
    pub trust: LifecycleHookTrustStatus,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LifecycleHookRegistryInput {
    pub schema_version: u32,
    pub rules: Vec<LifecycleHookRule>,
    pub handlers: Vec<LifecycleHookHandlerBinding>,
}

impl Default for LifecycleHookRegistryInput {
    fn default() -> Self {
        Self {
            schema_version: LIFECYCLE_HOOK_SCHEMA_VERSION,
            rules: Vec::new(),
            handlers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleHookRegistrationKind {
    Rule,
    Handler,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleHookRegistryRejectionReason {
    Invalid,
    Duplicate,
    Untrusted,
    Modified,
    Disabled,
    MissingHandler,
    HandlerKindMismatch,
    HandlerSourceMismatch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RejectedLifecycleHookRegistration {
    pub id: String,
    pub kind: LifecycleHookRegistrationKind,
    pub reason: LifecycleHookRegistryRejectionReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LifecycleHookRegistrySnapshot {
    pub schema_version: u32,
    pub revision_sha256: String,
    pub rules: Vec<LifecycleHookRule>,
    pub handlers: Vec<LifecycleHookHandlerBinding>,
    pub rejected: Vec<RejectedLifecycleHookRegistration>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleHookRegistryError {
    SchemaVersion,
    TooManyRules,
    TooManyHandlerBindings,
    Serialization,
}

fn default_enabled() -> bool {
    true
}

fn default_timeout_ms() -> u64 {
    10_000
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LifecycleHookEvent {
    pub id: String,
    pub name: LifecycleHookEventName,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LifecycleHookPlanInput {
    pub schema_version: u32,
    pub event: Option<LifecycleHookEvent>,
    pub rules: Vec<LifecycleHookRule>,
}

impl Default for LifecycleHookPlanInput {
    fn default() -> Self {
        Self {
            schema_version: LIFECYCLE_HOOK_SCHEMA_VERSION,
            event: None,
            rules: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleHookRejectionReason {
    Disabled,
    EventMismatch,
    MatcherMismatch,
    Untrusted,
    Modified,
    InvalidRule,
    DuplicateRule,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RejectedLifecycleHookRule {
    pub rule_id: String,
    pub reason: LifecycleHookRejectionReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LifecycleHookInvocation {
    pub id: String,
    pub event_id: String,
    pub rule_id: String,
    pub handler: LifecycleHookHandlerReference,
    pub source: LifecycleHookSource,
    pub display_order: i64,
    pub timeout_ms: u64,
    pub required_capability: LifecycleHookRequiredCapability,
    /// Reserved as a hard contract boundary. A future mutating Hook
    /// protocol must use a new schema version instead of changing this value.
    pub observation_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LifecycleHookPlan {
    pub schema_version: u32,
    pub event_id: String,
    pub invocations: Vec<LifecycleHookInvocation>,
    pub rejected_rules: Vec<RejectedLifecycleHookRule>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleHookExecutionStatus {
    Running,
    Succeeded,
    Failed,
    TimedOut,
    Cancelled,
}

/// Host-produced observation result. It deliberately has no fields for
/// mutated input, replacement output, permission decisions, or continuation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LifecycleHookExecutionInput {
    pub schema_version: u32,
    /// Exact immutable registry snapshot that authorized this invocation.
    pub registry_revision_sha256: String,
    pub invocation: LifecycleHookInvocation,
    pub status: LifecycleHookExecutionStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Bounded preview already redacted by the executing Host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_preview: Option<String>,
    #[serde(default)]
    pub output_truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LifecycleHookExecutionProjection {
    pub schema_version: u32,
    pub registry_revision_sha256: String,
    pub invocation_id: String,
    pub event_id: String,
    pub rule_id: String,
    pub handler_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub status: LifecycleHookExecutionStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_preview: Option<String>,
    pub output_truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    pub required_capability: LifecycleHookRequiredCapability,
    pub observation_only: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleHookExecutionError {
    SchemaVersion,
    InvalidRegistryRevision,
    InvalidInvocation,
    InvalidStatus,
    InvalidDuration,
    InvalidOutputPreview,
    InvalidErrorCode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifecycleHookPlanError {
    SchemaVersion,
    MissingEvent,
    InvalidEvent,
    TooManyRules,
}

/// Produces an immutable, fail-closed registry snapshot. Rules become
/// executable only when an exact trusted Host binding with the same handler
/// kind and source exists.
pub fn build_lifecycle_hook_registry(
    input: &LifecycleHookRegistryInput,
) -> Result<LifecycleHookRegistrySnapshot, LifecycleHookRegistryError> {
    if input.schema_version != LIFECYCLE_HOOK_SCHEMA_VERSION {
        return Err(LifecycleHookRegistryError::SchemaVersion);
    }
    if input.rules.len() > MAX_RULES {
        return Err(LifecycleHookRegistryError::TooManyRules);
    }
    if input.handlers.len() > MAX_HANDLER_BINDINGS {
        return Err(LifecycleHookRegistryError::TooManyHandlerBindings);
    }

    let mut rejected = Vec::new();
    let mut seen_handler_ids = BTreeSet::new();
    let mut accepted_handlers = BTreeMap::new();
    for binding in &input.handlers {
        let reason = if !seen_handler_ids.insert(binding.handler_id.clone()) {
            Some(LifecycleHookRegistryRejectionReason::Duplicate)
        } else if !valid_identifier(&binding.handler_id) {
            Some(LifecycleHookRegistryRejectionReason::Invalid)
        } else if !binding.enabled {
            Some(LifecycleHookRegistryRejectionReason::Disabled)
        } else {
            match binding.trust {
                LifecycleHookTrustStatus::Managed | LifecycleHookTrustStatus::Trusted => None,
                LifecycleHookTrustStatus::Untrusted => {
                    Some(LifecycleHookRegistryRejectionReason::Untrusted)
                }
                LifecycleHookTrustStatus::Modified => {
                    Some(LifecycleHookRegistryRejectionReason::Modified)
                }
            }
        };
        if let Some(reason) = reason {
            rejected.push(RejectedLifecycleHookRegistration {
                id: binding.handler_id.clone(),
                kind: LifecycleHookRegistrationKind::Handler,
                reason,
            });
        } else {
            accepted_handlers.insert(binding.handler_id.clone(), binding.clone());
        }
    }

    let mut seen_rule_ids = BTreeSet::new();
    let mut accepted_rules = Vec::new();
    for rule in &input.rules {
        let reason = if !seen_rule_ids.insert(rule.id.clone()) {
            Some(LifecycleHookRegistryRejectionReason::Duplicate)
        } else if !valid_rule(rule) {
            Some(LifecycleHookRegistryRejectionReason::Invalid)
        } else {
            match rule.trust {
                LifecycleHookTrustStatus::Untrusted => {
                    Some(LifecycleHookRegistryRejectionReason::Untrusted)
                }
                LifecycleHookTrustStatus::Modified => {
                    Some(LifecycleHookRegistryRejectionReason::Modified)
                }
                LifecycleHookTrustStatus::Managed | LifecycleHookTrustStatus::Trusted => {
                    match accepted_handlers.get(&rule.handler.id) {
                        None => Some(LifecycleHookRegistryRejectionReason::MissingHandler),
                        Some(binding) if binding.kind != rule.handler.kind => {
                            Some(LifecycleHookRegistryRejectionReason::HandlerKindMismatch)
                        }
                        Some(binding) if binding.source != rule.source => {
                            Some(LifecycleHookRegistryRejectionReason::HandlerSourceMismatch)
                        }
                        Some(_) => None,
                    }
                }
            }
        };
        if let Some(reason) = reason {
            rejected.push(RejectedLifecycleHookRegistration {
                id: rule.id.clone(),
                kind: LifecycleHookRegistrationKind::Rule,
                reason,
            });
        } else {
            accepted_rules.push(rule.clone());
        }
    }

    let mut handlers = accepted_handlers.into_values().collect::<Vec<_>>();
    handlers.sort_by(|left, right| {
        source_rank(left.source)
            .cmp(&source_rank(right.source))
            .then_with(|| left.handler_id.cmp(&right.handler_id))
    });
    accepted_rules.sort_by(|left, right| {
        source_rank(left.source)
            .cmp(&source_rank(right.source))
            .then_with(|| left.display_order.cmp(&right.display_order))
            .then_with(|| left.id.cmp(&right.id))
    });
    let digest_material = serde_json::to_vec(&(&accepted_rules, &handlers))
        .map_err(|_| LifecycleHookRegistryError::Serialization)?;
    let revision_sha256 = hex::encode(Sha256::digest(digest_material));

    Ok(LifecycleHookRegistrySnapshot {
        schema_version: LIFECYCLE_HOOK_SCHEMA_VERSION,
        revision_sha256,
        rules: accepted_rules,
        handlers,
        rejected,
    })
}

pub fn plan_lifecycle_hooks(
    input: &LifecycleHookPlanInput,
) -> Result<LifecycleHookPlan, LifecycleHookPlanError> {
    if input.schema_version != LIFECYCLE_HOOK_SCHEMA_VERSION {
        return Err(LifecycleHookPlanError::SchemaVersion);
    }
    let event = input
        .event
        .as_ref()
        .ok_or(LifecycleHookPlanError::MissingEvent)?;
    if !valid_identifier(&event.id) || !valid_identifier(&event.session_id) {
        return Err(LifecycleHookPlanError::InvalidEvent);
    }
    if event
        .turn_id
        .as_deref()
        .is_some_and(|value| !valid_identifier(value))
        || event
            .tool_call_id
            .as_deref()
            .is_some_and(|value| !valid_identifier(value))
        || event
            .tool_name
            .as_deref()
            .is_some_and(|value| !valid_tool_name(value))
    {
        return Err(LifecycleHookPlanError::InvalidEvent);
    }
    if input.rules.len() > MAX_RULES {
        return Err(LifecycleHookPlanError::TooManyRules);
    }

    let mut seen_rule_ids = BTreeSet::new();
    let mut invocations = Vec::new();
    let mut rejected_rules = Vec::new();
    for rule in &input.rules {
        let reason = if !valid_rule(rule) {
            Some(LifecycleHookRejectionReason::InvalidRule)
        } else if !seen_rule_ids.insert(rule.id.clone()) {
            Some(LifecycleHookRejectionReason::DuplicateRule)
        } else if !rule.enabled {
            Some(LifecycleHookRejectionReason::Disabled)
        } else if rule.event != event.name {
            Some(LifecycleHookRejectionReason::EventMismatch)
        } else if !matcher_matches(&rule.matcher, event.tool_name.as_deref()) {
            Some(LifecycleHookRejectionReason::MatcherMismatch)
        } else {
            match rule.trust {
                LifecycleHookTrustStatus::Managed | LifecycleHookTrustStatus::Trusted => None,
                LifecycleHookTrustStatus::Untrusted => {
                    Some(LifecycleHookRejectionReason::Untrusted)
                }
                LifecycleHookTrustStatus::Modified => Some(LifecycleHookRejectionReason::Modified),
            }
        };
        if let Some(reason) = reason {
            rejected_rules.push(RejectedLifecycleHookRule {
                rule_id: rule.id.clone(),
                reason,
            });
            continue;
        }

        invocations.push(LifecycleHookInvocation {
            id: invocation_id(event, rule),
            event_id: event.id.clone(),
            rule_id: rule.id.clone(),
            handler: rule.handler.clone(),
            source: rule.source,
            display_order: rule.display_order,
            timeout_ms: rule.timeout_ms,
            required_capability: required_capability(rule.handler.kind),
            observation_only: true,
        });
    }
    invocations.sort_by(|left, right| {
        source_rank(left.source)
            .cmp(&source_rank(right.source))
            .then_with(|| left.display_order.cmp(&right.display_order))
            .then_with(|| left.rule_id.cmp(&right.rule_id))
    });

    Ok(LifecycleHookPlan {
        schema_version: LIFECYCLE_HOOK_SCHEMA_VERSION,
        event_id: event.id.clone(),
        invocations,
        rejected_rules,
    })
}

pub fn project_lifecycle_hook_execution(
    input: &LifecycleHookExecutionInput,
) -> Result<LifecycleHookExecutionProjection, LifecycleHookExecutionError> {
    if input.schema_version != LIFECYCLE_HOOK_SCHEMA_VERSION {
        return Err(LifecycleHookExecutionError::SchemaVersion);
    }
    if input.registry_revision_sha256.len() != 64
        || !input
            .registry_revision_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(LifecycleHookExecutionError::InvalidRegistryRevision);
    }
    let invocation = &input.invocation;
    if !valid_identifier(&invocation.id)
        || !valid_identifier(&invocation.event_id)
        || !valid_identifier(&invocation.rule_id)
        || !valid_identifier(&invocation.handler.id)
        || !invocation.observation_only
        || invocation.timeout_ms < MIN_TIMEOUT_MS
        || invocation.timeout_ms > MAX_TIMEOUT_MS
        || invocation.required_capability != required_capability(invocation.handler.kind)
    {
        return Err(LifecycleHookExecutionError::InvalidInvocation);
    }
    let terminal = input.status != LifecycleHookExecutionStatus::Running;
    if terminal != input.duration_ms.is_some() {
        return Err(LifecycleHookExecutionError::InvalidStatus);
    }
    if input
        .duration_ms
        .is_some_and(|duration| duration > MAX_EXECUTION_DURATION_MS)
    {
        return Err(LifecycleHookExecutionError::InvalidDuration);
    }
    if input.duration_ms.is_some_and(|duration| {
        duration > invocation.timeout_ms && input.status != LifecycleHookExecutionStatus::TimedOut
    }) {
        return Err(LifecycleHookExecutionError::InvalidDuration);
    }
    if invocation
        .handler
        .display_name
        .as_deref()
        .is_some_and(|value| {
            value.trim().is_empty() || value.chars().count() > 256 || value.contains('\0')
        })
    {
        return Err(LifecycleHookExecutionError::InvalidInvocation);
    }
    if input.output_preview.as_deref().is_some_and(|preview| {
        preview.chars().count() > MAX_OUTPUT_PREVIEW_CHARS || preview.contains('\0')
    }) {
        return Err(LifecycleHookExecutionError::InvalidOutputPreview);
    }
    if input
        .error_code
        .as_deref()
        .is_some_and(|code| !valid_identifier(code))
    {
        return Err(LifecycleHookExecutionError::InvalidErrorCode);
    }

    Ok(LifecycleHookExecutionProjection {
        schema_version: LIFECYCLE_HOOK_SCHEMA_VERSION,
        registry_revision_sha256: input.registry_revision_sha256.to_ascii_lowercase(),
        invocation_id: invocation.id.clone(),
        event_id: invocation.event_id.clone(),
        rule_id: invocation.rule_id.clone(),
        handler_id: invocation.handler.id.clone(),
        display_name: invocation.handler.display_name.clone(),
        status: input.status,
        duration_ms: input.duration_ms,
        output_preview: input.output_preview.clone(),
        output_truncated: input.output_truncated,
        error_code: input.error_code.clone(),
        required_capability: invocation.required_capability,
        observation_only: true,
    })
}

fn valid_rule(rule: &LifecycleHookRule) -> bool {
    valid_identifier(&rule.id)
        && valid_identifier(&rule.handler.id)
        && rule.timeout_ms >= MIN_TIMEOUT_MS
        && rule.timeout_ms <= MAX_TIMEOUT_MS
        && rule.matcher.tool_names.len() <= MAX_MATCHER_TOOL_NAMES
        && rule
            .matcher
            .tool_names
            .iter()
            .all(|name| name == "*" || valid_tool_name(name))
        && rule.handler.display_name.as_deref().map_or(true, |value| {
            !value.trim().is_empty() && value.chars().count() <= 256
        })
}

fn valid_identifier(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && value.chars().count() <= MAX_IDENTIFIER_CHARS
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._:-".contains(character))
}

fn valid_tool_name(value: &str) -> bool {
    valid_identifier(value) && value == value.trim()
}

fn matcher_matches(matcher: &LifecycleHookMatcher, tool_name: Option<&str>) -> bool {
    if matcher.tool_names.is_empty() {
        return true;
    }
    let Some(tool_name) = tool_name else {
        return false;
    };
    matcher
        .tool_names
        .iter()
        .any(|candidate| candidate == "*" || candidate == tool_name)
}

fn required_capability(kind: LifecycleHookHandlerKind) -> LifecycleHookRequiredCapability {
    match kind {
        LifecycleHookHandlerKind::Command => LifecycleHookRequiredCapability::Process,
        LifecycleHookHandlerKind::Http => LifecycleHookRequiredCapability::Network,
        LifecycleHookHandlerKind::Prompt => LifecycleHookRequiredCapability::Model,
        LifecycleHookHandlerKind::Internal => LifecycleHookRequiredCapability::None,
    }
}

fn source_rank(source: LifecycleHookSource) -> u8 {
    match source {
        LifecycleHookSource::Managed => 0,
        LifecycleHookSource::Product => 1,
        LifecycleHookSource::User => 2,
        LifecycleHookSource::Workspace => 3,
        LifecycleHookSource::Plugin => 4,
    }
}

fn invocation_id(event: &LifecycleHookEvent, rule: &LifecycleHookRule) -> String {
    let material = format!("{}\0{}\0{}", event.id, rule.id, rule.handler.id);
    format!("hook_{}", hex::encode(Sha256::digest(material.as_bytes())))
}









#[cfg(test)]
mod tests {
    use super::*;

    fn rule(
        id: &str,
        source: LifecycleHookSource,
        trust: LifecycleHookTrustStatus,
        tool_names: &[&str],
    ) -> LifecycleHookRule {
        LifecycleHookRule {
            id: id.to_string(),
            event: LifecycleHookEventName::PreToolUse,
            source,
            trust,
            handler: LifecycleHookHandlerReference {
                id: format!("handler.{id}"),
                kind: LifecycleHookHandlerKind::Command,
                execution_mode: LifecycleHookExecutionMode::Sync,
                display_name: None,
            },
            enabled: true,
            matcher: LifecycleHookMatcher {
                tool_names: tool_names
                    .iter()
                    .map(|value| (*value).to_string())
                    .collect(),
            },
            display_order: 0,
            timeout_ms: 5_000,
        }
    }

    #[test]
    fn registry_requires_exact_trusted_host_binding_and_has_stable_revision() {
        let trusted_rule = rule(
            "audit",
            LifecycleHookSource::Workspace,
            LifecycleHookTrustStatus::Trusted,
            &["read_file"],
        );
        let binding = LifecycleHookHandlerBinding {
            handler_id: trusted_rule.handler.id.clone(),
            kind: trusted_rule.handler.kind,
            source: trusted_rule.source,
            trust: LifecycleHookTrustStatus::Trusted,
            enabled: true,
        };
        let first = build_lifecycle_hook_registry(&LifecycleHookRegistryInput {
            schema_version: LIFECYCLE_HOOK_SCHEMA_VERSION,
            rules: vec![trusted_rule.clone()],
            handlers: vec![binding.clone()],
        })
        .unwrap();
        let reordered = build_lifecycle_hook_registry(&LifecycleHookRegistryInput {
            schema_version: LIFECYCLE_HOOK_SCHEMA_VERSION,
            rules: vec![trusted_rule.clone()],
            handlers: vec![binding],
        })
        .unwrap();
        assert_eq!(first.revision_sha256, reordered.revision_sha256);
        assert_eq!(first.rules, vec![trusted_rule.clone()]);
        assert!(first.rejected.is_empty());

        let rejected = build_lifecycle_hook_registry(&LifecycleHookRegistryInput {
            schema_version: LIFECYCLE_HOOK_SCHEMA_VERSION,
            rules: vec![trusted_rule],
            handlers: vec![LifecycleHookHandlerBinding {
                handler_id: "handler.audit".to_string(),
                kind: LifecycleHookHandlerKind::Http,
                source: LifecycleHookSource::Workspace,
                trust: LifecycleHookTrustStatus::Trusted,
                enabled: true,
            }],
        })
        .unwrap();
        assert!(rejected.rules.is_empty());
        assert_eq!(
            rejected.rejected.last().map(|item| item.reason),
            Some(LifecycleHookRegistryRejectionReason::HandlerKindMismatch)
        );

        let input = serde_json::to_vec(&LifecycleHookRegistryInput::default()).unwrap();
        let decoded: LifecycleHookRegistryInput = serde_json::from_slice(&input).unwrap();
        let value = serde_json::to_value(build_lifecycle_hook_registry(&decoded).unwrap()).unwrap();
        assert_eq!(value["schemaVersion"], LIFECYCLE_HOOK_SCHEMA_VERSION);
        assert_eq!(value["rules"], serde_json::json!([]));
        assert_eq!(value["handlers"], serde_json::json!([]));
    }

    #[test]
    fn execution_projection_is_observation_only_and_bounded() {
        let event = LifecycleHookEvent {
            id: "event.1".to_string(),
            name: LifecycleHookEventName::PreToolUse,
            session_id: "session.1".to_string(),
            turn_id: Some("turn.1".to_string()),
            tool_name: Some("read_file".to_string()),
            tool_call_id: Some("call.1".to_string()),
        };
        let invocation = plan_lifecycle_hooks(&LifecycleHookPlanInput {
            schema_version: LIFECYCLE_HOOK_SCHEMA_VERSION,
            event: Some(event),
            rules: vec![rule(
                "audit",
                LifecycleHookSource::Managed,
                LifecycleHookTrustStatus::Managed,
                &["read_file"],
            )],
        })
        .unwrap()
        .invocations
        .remove(0);
        let projection = project_lifecycle_hook_execution(&LifecycleHookExecutionInput {
            schema_version: LIFECYCLE_HOOK_SCHEMA_VERSION,
            registry_revision_sha256: "a".repeat(64),
            invocation,
            status: LifecycleHookExecutionStatus::Succeeded,
            duration_ms: Some(12),
            output_preview: Some("observed read_file".to_string()),
            output_truncated: false,
            error_code: None,
        })
        .unwrap();
        assert!(projection.observation_only);
        assert_eq!(projection.registry_revision_sha256, "a".repeat(64));
        assert_eq!(projection.status, LifecycleHookExecutionStatus::Succeeded);
    }

    #[test]
    fn execution_projection_rejects_mutating_input() {
        let mut invocation = LifecycleHookInvocation {
            id: "hook.1".to_string(),
            event_id: "event.1".to_string(),
            rule_id: "rule.1".to_string(),
            handler: LifecycleHookHandlerReference {
                id: "handler.1".to_string(),
                kind: LifecycleHookHandlerKind::Internal,
                execution_mode: LifecycleHookExecutionMode::Sync,
                display_name: None,
            },
            source: LifecycleHookSource::Managed,
            display_order: 0,
            timeout_ms: 1_000,
            required_capability: LifecycleHookRequiredCapability::None,
            observation_only: true,
        };
        invocation.observation_only = false;
        assert_eq!(
            project_lifecycle_hook_execution(&LifecycleHookExecutionInput {
                schema_version: LIFECYCLE_HOOK_SCHEMA_VERSION,
                registry_revision_sha256: "b".repeat(64),
                invocation,
                status: LifecycleHookExecutionStatus::Running,
                duration_ms: None,
                output_preview: None,
                output_truncated: false,
                error_code: None,
            }),
            Err(LifecycleHookExecutionError::InvalidInvocation)
        );
    }

    fn event(tool_name: &str) -> LifecycleHookEvent {
        LifecycleHookEvent {
            id: "event.1".to_string(),
            name: LifecycleHookEventName::PreToolUse,
            session_id: "session.1".to_string(),
            turn_id: Some("turn.1".to_string()),
            tool_name: Some(tool_name.to_string()),
            tool_call_id: Some("call.1".to_string()),
        }
    }

    #[test]
    fn plans_trusted_exact_and_wildcard_rules_in_scope_order() {
        let plan = plan_lifecycle_hooks(&LifecycleHookPlanInput {
            event: Some(event("read_file")),
            rules: vec![
                rule(
                    "workspace",
                    LifecycleHookSource::Workspace,
                    LifecycleHookTrustStatus::Trusted,
                    &["*"],
                ),
                rule(
                    "managed",
                    LifecycleHookSource::Managed,
                    LifecycleHookTrustStatus::Managed,
                    &["read_file"],
                ),
            ],
            ..LifecycleHookPlanInput::default()
        })
        .expect("valid plan");

        assert_eq!(plan.invocations.len(), 2);
        assert_eq!(plan.invocations[0].rule_id, "managed");
        assert_eq!(plan.invocations[1].rule_id, "workspace");
        assert!(plan.invocations.iter().all(|item| item.observation_only));
        assert_eq!(
            plan.invocations[0].required_capability,
            LifecycleHookRequiredCapability::Process
        );
    }

    #[test]
    fn rejects_untrusted_modified_and_duplicate_rules() {
        let trusted = rule(
            "same",
            LifecycleHookSource::User,
            LifecycleHookTrustStatus::Trusted,
            &["read_file"],
        );
        let plan = plan_lifecycle_hooks(&LifecycleHookPlanInput {
            event: Some(event("read_file")),
            rules: vec![
                trusted.clone(),
                trusted,
                rule(
                    "untrusted",
                    LifecycleHookSource::Workspace,
                    LifecycleHookTrustStatus::Untrusted,
                    &["read_file"],
                ),
                rule(
                    "modified",
                    LifecycleHookSource::Workspace,
                    LifecycleHookTrustStatus::Modified,
                    &["read_file"],
                ),
            ],
            ..LifecycleHookPlanInput::default()
        })
        .expect("valid plan");

        assert_eq!(plan.invocations.len(), 1);
        assert_eq!(
            plan.rejected_rules
                .iter()
                .map(|rule| rule.reason)
                .collect::<Vec<_>>(),
            vec![
                LifecycleHookRejectionReason::DuplicateRule,
                LifecycleHookRejectionReason::Untrusted,
                LifecycleHookRejectionReason::Modified,
            ]
        );
    }

    #[test]
    fn tool_matcher_never_uses_display_labels_or_substrings() {
        let plan = plan_lifecycle_hooks(&LifecycleHookPlanInput {
            event: Some(event("read_file")),
            rules: vec![
                rule(
                    "display",
                    LifecycleHookSource::User,
                    LifecycleHookTrustStatus::Trusted,
                    &["读取文件"],
                ),
                rule(
                    "substring",
                    LifecycleHookSource::User,
                    LifecycleHookTrustStatus::Trusted,
                    &["read"],
                ),
            ],
            ..LifecycleHookPlanInput::default()
        })
        .expect("valid plan");

        assert!(plan.invocations.is_empty());
        assert!(plan.rejected_rules.iter().all(|rule| {
            rule.reason == LifecycleHookRejectionReason::MatcherMismatch
                || rule.reason == LifecycleHookRejectionReason::InvalidRule
        }));
    }
}
