// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Resolve a platform-aware, policy-constrained built-in tool catalog.
// - Security: Runtime manifests report only handler facts; Core owns policy metadata.

use crate::tool_catalog::{
    builtin_tool_metadata, capability_supported, default_route, normalize_tool_id, permission_for,
    requires_result_egress, route_supported,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};

pub const TOOL_POLICY_SCHEMA_VERSION: u32 = 1;
pub const TOOL_POLICY_ABI_VERSION: u32 = 1;
const MAX_MANIFEST_TTL_MS: u64 = 300_000;
const MAX_CLOCK_SKEW_MS: u64 = 30_000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolPlatform {
    Ios,
    Android,
    Macos,
    Cli,
    Web,
    #[default]
    AgentHost,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolRoute {
    Device,
    #[default]
    AgentHost,
    Cloud,
    WebClient,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolPermission {
    None,
    ScopedResource,
    SystemPermission,
    PlatformCapability,
    PlatformLifecycle,
    #[default]
    AgentApproval,
    Unsupported,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityAuthorization {
    Unavailable,
    Denied,
    #[default]
    Ask,
    Granted,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolRisk {
    ReadOnly,
    #[default]
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RuntimeToolManifest {
    pub tool_id: String,
    pub handler_id: Option<String>,
    pub schema: Value,
    pub route: Option<ToolRoute>,
    pub available: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct RuntimeResultEgressReceipt {
    pub tool_id: String,
    pub route: ToolRoute,
    pub manifest_id: String,
    pub host_id: String,
    pub client_id: String,
    pub session_id: String,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
    pub allow_result_egress: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RuntimeCapabilityManifest {
    pub manifest_id: String,
    pub manifest_version: u64,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
    pub host_id: Option<String>,
    pub client_id: Option<String>,
    pub session_id: Option<String>,
    pub routes: Vec<ToolRoute>,
    pub capabilities: Vec<String>,
    pub authorizations: HashMap<String, CapabilityAuthorization>,
    pub result_egress_receipts: Vec<RuntimeResultEgressReceipt>,
    pub tools: Vec<RuntimeToolManifest>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RuntimeIdentity {
    pub host_id: Option<String>,
    pub client_id: Option<String>,
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ScopedCapabilityGrant {
    pub capability: String,
    pub route: Option<ToolRoute>,
    pub scope: Option<String>,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ResultEgressGrant {
    pub tool_id: String,
    pub route: ToolRoute,
    pub manifest_id: String,
    pub host_id: String,
    pub client_id: String,
    pub session_id: String,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ConversationToolPolicy {
    pub mode: String,
    pub allow_tools: Vec<String>,
    pub deny_tools: Vec<String>,
}

fn default_max_risk() -> ToolRisk {
    ToolRisk::High
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UserToolPolicy {
    pub allow_tools: Vec<String>,
    pub deny_tools: Vec<String>,
    pub allow_capabilities: Vec<String>,
    pub deny_capabilities: Vec<String>,
    pub max_risk: ToolRisk,
    pub allow_result_egress_routes: Vec<ToolRoute>,
}

impl Default for UserToolPolicy {
    fn default() -> Self {
        Self {
            allow_tools: Vec::new(),
            deny_tools: Vec::new(),
            allow_capabilities: Vec::new(),
            deny_capabilities: Vec::new(),
            max_risk: default_max_risk(),
            allow_result_egress_routes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolPolicyInput {
    pub schema_version: u32,
    pub platform: ToolPlatform,
    pub now_ms: u64,
    pub expected_identity: RuntimeIdentity,
    pub runtime: RuntimeCapabilityManifest,
    pub conversation: ConversationToolPolicy,
    pub user_policy: UserToolPolicy,
    pub scoped_grants: Vec<ScopedCapabilityGrant>,
    pub result_egress_grants: Vec<ResultEgressGrant>,
}

impl Default for ToolPolicyInput {
    fn default() -> Self {
        Self {
            schema_version: TOOL_POLICY_SCHEMA_VERSION,
            platform: ToolPlatform::default(),
            now_ms: 0,
            expected_identity: RuntimeIdentity::default(),
            runtime: RuntimeCapabilityManifest::default(),
            conversation: ConversationToolPolicy::default(),
            user_policy: UserToolPolicy::default(),
            scoped_grants: Vec::new(),
            result_egress_grants: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResolvedTool {
    pub tool_id: String,
    pub name: String,
    pub enabled: bool,
    pub route: Option<ToolRoute>,
    pub capability: String,
    pub permission: ToolPermission,
    pub authorization: CapabilityAuthorization,
    pub risk: ToolRisk,
    pub requires_result_egress: bool,
    pub reason: String,
    pub schema: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolPolicyOutput {
    pub schema_version: u32,
    pub platform: ToolPlatform,
    pub policy_digest: String,
    pub manifest_digest: String,
    pub manifest_valid_until_ms: u64,
    pub enabled_tools: Vec<ResolvedTool>,
    pub disabled_tools: Vec<ResolvedTool>,
}

/// Returns interactive capabilities already declared by the Host.
///
/// User wording is deliberately ignored here. Text is not an authorization boundary and must
/// never add, remove, or select tools. The model chooses among exposed capabilities from the
/// request's meaning; Core policy evaluates an actual call when it is proposed.
pub fn select_interactive_host_tool_ids(
    _turn_text: Option<&str>,
    available_tool_ids: &[String],
) -> Vec<String> {
    let mut seen = HashSet::new();
    available_tool_ids
        .iter()
        .map(|tool_id| tool_id.trim().to_string())
        .filter(|tool_id| !tool_id.is_empty() && seen.insert(tool_id.clone()))
        .collect()
}

/// Keeps the Host-declared tool catalog stable for the whole turn.
///
/// This contract boundary trims and de-duplicates declared capabilities, but preserves the
/// Host's opaque tool identities and never interprets the user's text, language, or phrasing.
pub fn select_turn_tool_ids(
    _platform: ToolPlatform,
    _conversation_mode: &str,
    _turn_text: Option<&str>,
    available_tool_ids: &[String],
) -> Vec<String> {
    let mut seen = HashSet::new();
    available_tool_ids
        .iter()
        .map(|tool_id| tool_id.trim().to_string())
        .filter(|tool_id| !tool_id.is_empty() && seen.insert(tool_id.clone()))
        .collect()
}

pub fn resolve_tool_policy(input: &ToolPolicyInput) -> ToolPolicyOutput {
    let manifest_reason = validate_manifest(input).err();
    let available_routes = input.runtime.routes.iter().copied().collect::<HashSet<_>>();
    let available_capabilities = normalized_set(&input.runtime.capabilities);
    let conversation_allow = normalized_set(&input.conversation.allow_tools);
    let conversation_deny = normalized_set(&input.conversation.deny_tools);
    let user_allow = normalized_set(&input.user_policy.allow_tools);
    let user_deny = normalized_set(&input.user_policy.deny_tools);
    let capability_allow = normalized_set(&input.user_policy.allow_capabilities);
    let capability_deny = normalized_set(&input.user_policy.deny_capabilities);
    let mut enabled_tools = Vec::new();
    let mut disabled_tools = Vec::new();

    for advertised in &input.runtime.tools {
        let tool_id = normalize_tool_id(&advertised.tool_id);
        let schema_name = advertised
            .schema
            .pointer("/function/name")
            .and_then(Value::as_str)
            .map(normalize_tool_id)
            .unwrap_or_default();
        let metadata = builtin_tool_metadata(&tool_id);
        let capability = metadata
            .as_ref()
            .map(|value| value.capability.clone())
            .unwrap_or_default();
        let route = advertised.route.or_else(|| {
            metadata
                .as_ref()
                .map(|_| default_route(input.platform, &tool_id))
        });
        let risk = metadata
            .as_ref()
            .map(|value| value.risk)
            .unwrap_or(ToolRisk::Critical);
        let permission = route
            .map(|value| permission_for(input.platform, value, &capability))
            .unwrap_or(ToolPermission::Unsupported);
        let mut authorization = authorization_for(input, &capability, route, permission);
        if authorization == CapabilityAuthorization::Ask
            && has_valid_scoped_grant(input, &capability, route)
        {
            authorization = CapabilityAuthorization::Granted;
        }
        let requires_egress = route
            .map(|value| requires_result_egress(value, &capability))
            .unwrap_or(false);
        let reason = manifest_reason.clone().unwrap_or_else(|| {
            first_disabled_reason(PolicyFacts {
                input,
                advertised,
                metadata_exists: metadata.is_some(),
                tool_id: &tool_id,
                schema_name: &schema_name,
                capability: &capability,
                route,
                permission,
                risk,
                authorization,
                requires_egress,
                available_routes: &available_routes,
                available_capabilities: &available_capabilities,
                conversation_allow: &conversation_allow,
                conversation_deny: &conversation_deny,
                user_allow: &user_allow,
                user_deny: &user_deny,
                capability_allow: &capability_allow,
                capability_deny: &capability_deny,
            })
        });
        let enabled = reason == "enabled";
        let resolved = ResolvedTool {
            tool_id: tool_id.clone(),
            name: schema_name,
            enabled,
            route,
            capability,
            permission,
            authorization,
            risk,
            requires_result_egress: requires_egress,
            reason,
            schema: advertised.schema.clone(),
        };
        if enabled {
            enabled_tools.push(resolved);
        } else {
            disabled_tools.push(resolved);
        }
    }

    ToolPolicyOutput {
        schema_version: TOOL_POLICY_SCHEMA_VERSION,
        platform: input.platform,
        policy_digest: digest_json(input),
        manifest_digest: digest_json(&input.runtime),
        manifest_valid_until_ms: input.runtime.expires_at_ms,
        enabled_tools,
        disabled_tools,
    }
}

#[cfg(test)]
mod turn_selection_tests {
    use super::*;

    fn available() -> Vec<String> {
        [
            "tool_search_meta",
            "web_search",
            "web_fetch",
            "read_file",
            "chrome_native_open",
            "computer_screenshot",
            "memory_search",
            "memory_save",
        ]
        .into_iter()
        .map(str::to_string)
        .collect()
    }

    #[test]
    fn turn_catalog_does_not_depend_on_wording_language_or_platform() {
        let available = available();
        let chinese = select_turn_tool_ids(
            ToolPlatform::Macos,
            "chat",
            Some("搜索最新资料并读取本地文件"),
            &available,
        );
        let english = select_turn_tool_ids(
            ToolPlatform::Web,
            "agent",
            Some("please inspect current evidence"),
            &available,
        );
        let neutral = select_turn_tool_ids(ToolPlatform::Cli, "chat", None, &available);

        assert_eq!(chinese, available);
        assert_eq!(english, available);
        assert_eq!(neutral, available);
    }

    #[test]
    fn interactive_catalog_uses_the_complete_declared_catalog_not_names_or_text() {
        let mut available = available();
        available.extend([
            "vendor.calendar.read.v2".to_string(),
            "opaque-capability-17".to_string(),
        ]);
        let first = select_interactive_host_tool_ids(Some("hello"), &available);
        let second = select_interactive_host_tool_ids(
            Some("截图、记忆、浏览器、browser and screenshot"),
            &available,
        );

        assert_eq!(first, second);
        assert_eq!(first, available);
    }
}

fn validate_manifest(input: &ToolPolicyInput) -> Result<(), String> {
    if input.schema_version != TOOL_POLICY_SCHEMA_VERSION {
        return Err("unsupported_schema_version".into());
    }
    if input.now_ms == 0 {
        return Err("manifest_clock_missing".into());
    }
    if input.runtime.manifest_id.trim().is_empty() || input.runtime.manifest_id.len() > 256 {
        return Err("manifest_identity_invalid".into());
    }
    if input.runtime.manifest_version == 0 {
        return Err("manifest_version_invalid".into());
    }
    if input.runtime.issued_at_ms == 0
        || input.runtime.expires_at_ms <= input.runtime.issued_at_ms
        || input.runtime.expires_at_ms - input.runtime.issued_at_ms > MAX_MANIFEST_TTL_MS
    {
        return Err("manifest_freshness_invalid".into());
    }
    if input.runtime.issued_at_ms > input.now_ms.saturating_add(MAX_CLOCK_SKEW_MS) {
        return Err("manifest_not_yet_valid".into());
    }
    if input.runtime.expires_at_ms <= input.now_ms {
        return Err("manifest_expired".into());
    }
    identity_matches(
        "host",
        &input.expected_identity.host_id,
        &input.runtime.host_id,
    )?;
    identity_matches(
        "client",
        &input.expected_identity.client_id,
        &input.runtime.client_id,
    )?;
    identity_matches(
        "session",
        &input.expected_identity.session_id,
        &input.runtime.session_id,
    )?;
    Ok(())
}

fn identity_matches(
    kind: &str,
    expected: &Option<String>,
    actual: &Option<String>,
) -> Result<(), String> {
    if let Some(expected) = expected.as_ref().filter(|value| !value.trim().is_empty()) {
        if actual.as_ref().map(|value| value.trim()) != Some(expected.trim()) {
            return Err(format!("manifest_{kind}_identity_mismatch"));
        }
    }
    Ok(())
}

struct PolicyFacts<'a> {
    input: &'a ToolPolicyInput,
    advertised: &'a RuntimeToolManifest,
    metadata_exists: bool,
    tool_id: &'a str,
    schema_name: &'a str,
    capability: &'a str,
    route: Option<ToolRoute>,
    permission: ToolPermission,
    risk: ToolRisk,
    authorization: CapabilityAuthorization,
    requires_egress: bool,
    available_routes: &'a HashSet<ToolRoute>,
    available_capabilities: &'a HashSet<String>,
    conversation_allow: &'a HashSet<String>,
    conversation_deny: &'a HashSet<String>,
    user_allow: &'a HashSet<String>,
    user_deny: &'a HashSet<String>,
    capability_allow: &'a HashSet<String>,
    capability_deny: &'a HashSet<String>,
}

fn first_disabled_reason(f: PolicyFacts<'_>) -> String {
    if !f.metadata_exists {
        return "unknown_tool_definition".into();
    }
    if !f.advertised.available {
        return "runtime_handler_unavailable".into();
    }
    if f.advertised
        .handler_id
        .as_deref()
        .unwrap_or("")
        .trim()
        .is_empty()
    {
        return "runtime_handler_invalid".into();
    }
    if f.tool_id.is_empty() || f.schema_name.is_empty() || f.tool_id != f.schema_name {
        return "invalid_tool_schema".into();
    }
    let Some(route) = f.route else {
        return "runtime_route_unavailable".into();
    };
    if !route_supported(f.input.platform, f.tool_id, route)
        || !capability_supported(f.input.platform, route, f.capability)
        || f.permission == ToolPermission::Unsupported
    {
        return "platform_not_supported".into();
    }
    if !f.conversation_allow.is_empty() && !f.conversation_allow.contains(f.tool_id) {
        return "tool_not_allowed_for_conversation".into();
    }
    if f.conversation_deny.contains(f.tool_id) {
        return "tool_denied_for_conversation".into();
    }
    if !f.user_allow.is_empty() && !f.user_allow.contains(f.tool_id) {
        return "tool_not_allowed_by_user".into();
    }
    if f.user_deny.contains(f.tool_id) {
        return "tool_denied_by_user".into();
    }
    if !f.capability_allow.is_empty() && !f.capability_allow.contains(f.capability) {
        return "capability_not_allowed_by_user".into();
    }
    if f.capability_deny.contains(f.capability) {
        return "capability_denied_by_user".into();
    }
    if f.risk > f.input.user_policy.max_risk {
        return "risk_exceeds_user_policy".into();
    }
    if !f.available_routes.contains(&route) {
        return "runtime_route_unavailable".into();
    }
    if f.capability != "none" && !f.available_capabilities.contains(f.capability) {
        return "runtime_capability_unavailable".into();
    }
    match f.authorization {
        CapabilityAuthorization::Unavailable => return "authorization_unavailable".into(),
        CapabilityAuthorization::Denied => return "authorization_denied".into(),
        CapabilityAuthorization::Ask | CapabilityAuthorization::Granted => {}
    }
    if f.requires_egress {
        let Some(receipt) = valid_result_egress_receipt(f.input, f.tool_id, route) else {
            return "result_egress_receipt_missing".into();
        };
        if !f
            .input
            .user_policy
            .allow_result_egress_routes
            .contains(&route)
        {
            return "result_egress_route_not_allowed".into();
        }
        if !has_valid_result_egress_grant(f.input, receipt) {
            return "result_egress_session_grant_missing".into();
        }
    }
    "enabled".into()
}

fn valid_result_egress_receipt<'a>(
    input: &'a ToolPolicyInput,
    tool_id: &str,
    route: ToolRoute,
) -> Option<&'a RuntimeResultEgressReceipt> {
    let expected = RuntimeIdentity {
        host_id: input.runtime.host_id.clone(),
        client_id: input.runtime.client_id.clone(),
        session_id: input.runtime.session_id.clone(),
    };
    input.runtime.result_egress_receipts.iter().find(|receipt| {
        valid_egress_receipt(receipt, &expected, tool_id, route, input.now_ms)
            && receipt.manifest_id.trim() == input.runtime.manifest_id.trim()
            && receipt.expires_at_ms <= input.runtime.expires_at_ms
    })
}

fn has_valid_result_egress_grant(
    input: &ToolPolicyInput,
    receipt: &RuntimeResultEgressReceipt,
) -> bool {
    input.result_egress_grants.iter().any(|grant| valid_egress_grant(grant, receipt, input.now_ms))
}

fn valid_egress_receipt(receipt: &RuntimeResultEgressReceipt, expected: &RuntimeIdentity,
    tool_id: &str, route: ToolRoute, now_ms: u64) -> bool {
    let (Some(host), Some(client), Some(session)) = (
        nonempty_identity(&expected.host_id), nonempty_identity(&expected.client_id),
        nonempty_identity(&expected.session_id),
    ) else { return false; };
    receipt.allow_result_egress
        && !receipt.manifest_id.trim().is_empty()
        && !normalize_tool_id(tool_id).is_empty()
        && normalize_tool_id(&receipt.tool_id) == normalize_tool_id(tool_id)
        && receipt.route == route
        && receipt.host_id.trim() == host && receipt.client_id.trim() == client
        && receipt.session_id.trim() == session
        && receipt.issued_at_ms > 0 && receipt.expires_at_ms > receipt.issued_at_ms
        && receipt.expires_at_ms - receipt.issued_at_ms <= MAX_MANIFEST_TTL_MS
        && receipt.issued_at_ms <= now_ms.saturating_add(MAX_CLOCK_SKEW_MS)
        && receipt.expires_at_ms > now_ms
}

fn valid_egress_grant(grant: &ResultEgressGrant, receipt: &RuntimeResultEgressReceipt, now_ms: u64) -> bool {
    normalize_tool_id(&grant.tool_id) == normalize_tool_id(&receipt.tool_id)
            && grant.route == receipt.route
            && grant.manifest_id.trim() == receipt.manifest_id.trim()
            && grant.host_id.trim() == receipt.host_id.trim()
            && grant.client_id.trim() == receipt.client_id.trim()
            && grant.session_id.trim() == receipt.session_id.trim()
            && grant.expires_at_ms > now_ms
            && grant.expires_at_ms <= receipt.expires_at_ms
}

/// Recheck short-lived proof at dispatch and result acceptance. The expected
/// identity must come from the authenticated execution owner, not the receipt.
/// This validates the supplied proof; it is not a revocation registry.
pub fn validate_result_egress_evidence(
    receipt: &RuntimeResultEgressReceipt, grant: &ResultEgressGrant,
    expected: &RuntimeIdentity, tool_id: &str, route: ToolRoute, now_ms: u64,
) -> Result<u64, &'static str> {
    if !valid_egress_receipt(receipt, expected, tool_id, route, now_ms) {
        return Err("result egress receipt is invalid or expired");
    }
    if !valid_egress_grant(grant, receipt, now_ms) {
        return Err("result egress grant is invalid or expired");
    }
    Ok(grant.expires_at_ms)
}

fn nonempty_identity(value: &Option<String>) -> Option<&str> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn authorization_for(
    input: &ToolPolicyInput,
    capability: &str,
    _route: Option<ToolRoute>,
    permission: ToolPermission,
) -> CapabilityAuthorization {
    if permission == ToolPermission::None || capability == "none" {
        return CapabilityAuthorization::Granted;
    }
    if permission == ToolPermission::Unsupported {
        return CapabilityAuthorization::Unavailable;
    }
    input
        .runtime
        .authorizations
        .iter()
        .find(|(key, _)| normalize_tool_id(key) == capability)
        .map(|(_, value)| *value)
        .unwrap_or(CapabilityAuthorization::Ask)
}

fn has_valid_scoped_grant(
    input: &ToolPolicyInput,
    capability: &str,
    route: Option<ToolRoute>,
) -> bool {
    input.scoped_grants.iter().any(|grant| {
        normalize_tool_id(&grant.capability) == capability
            && grant
                .route
                .map(|value| Some(value) == route)
                .unwrap_or(true)
            && grant.expires_at_ms > input.now_ms
            && grant.scope.as_deref().map(str::trim).unwrap_or("") != ""
    })
}

fn normalized_set(values: &[String]) -> HashSet<String> {
    values
        .iter()
        .map(|value| normalize_tool_id(value))
        .collect()
}

fn canonical_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let sorted = map
                .iter()
                .map(|(key, value)| (key.clone(), canonical_json(value)))
                .collect::<BTreeMap<_, _>>();
            Value::Object(sorted.into_iter().collect())
        }
        Value::Array(values) => Value::Array(values.iter().map(canonical_json).collect()),
        _ => value.clone(),
    }
}

fn digest_json<T: Serialize>(value: &T) -> String {
    let value = serde_json::to_value(value).unwrap_or(Value::Null);
    let canonical = serde_json::to_vec(&canonical_json(&value)).unwrap_or_default();
    hex::encode(Sha256::digest(canonical))
}





#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn live_egress_proof_uses_exact_owner_and_millisecond_deadline() {
        let expected = RuntimeIdentity { host_id: Some("h".into()), client_id: Some("c".into()), session_id: Some("s".into()) };
        let receipt = RuntimeResultEgressReceipt { tool_id: "device_write_file".into(), route: ToolRoute::Device,
            manifest_id: "m".into(), host_id: "h".into(), client_id: "c".into(), session_id: "s".into(),
            issued_at_ms: 1_000, expires_at_ms: 2_001, allow_result_egress: true };
        let grant = ResultEgressGrant { tool_id: receipt.tool_id.clone(), route: receipt.route,
            manifest_id: "m".into(), host_id: "h".into(), client_id: "c".into(), session_id: "s".into(), expires_at_ms: 2_000 };
        let validate = |r: &RuntimeResultEgressReceipt, g: &ResultEgressGrant, identity: &RuntimeIdentity, now| {
            validate_result_egress_evidence(r, g, identity, "device_write_file", ToolRoute::Device, now)
        };
        assert_eq!(validate(&receipt, &grant, &expected, 1_999), Ok(2_000));
        assert!(validate(&receipt, &grant, &expected, 2_000).is_err());
        for field in ["host", "client", "session"] {
            let mut wrong = expected.clone();
            match field { "host" => wrong.host_id = Some("other".into()), "client" => wrong.client_id = Some("other".into()), _ => wrong.session_id = Some("other".into()) }
            assert!(validate(&receipt, &grant, &wrong, 1_500).is_err());
        }
        let mut wrong = grant.clone(); wrong.manifest_id = "other".into();
        assert!(validate(&receipt, &wrong, &expected, 1_500).is_err());
        let mut wrong = receipt.clone(); wrong.allow_result_egress = false;
        assert!(validate(&wrong, &grant, &expected, 1_500).is_err());
        wrong = receipt.clone(); wrong.expires_at_ms = wrong.issued_at_ms + MAX_MANIFEST_TTL_MS + 1;
        assert!(validate(&wrong, &grant, &expected, 1_500).is_err());
        wrong = receipt.clone(); wrong.issued_at_ms = 40_000; wrong.expires_at_ms = 41_000;
        assert!(validate(&wrong, &grant, &expected, 1_500).is_err());
    }

    fn tool(tool_id: &str, route: ToolRoute) -> RuntimeToolManifest {
        RuntimeToolManifest {
            tool_id: tool_id.into(),
            handler_id: Some(tool_id.into()),
            schema: json!({
                "type": "function",
                "function": {"name": tool_id, "parameters": {"type": "object"}}
            }),
            route: Some(route),
            available: true,
        }
    }

    fn input(platform: ToolPlatform) -> ToolPolicyInput {
        let now_ms = 1_000_000;
        ToolPolicyInput {
            platform,
            now_ms,
            runtime: RuntimeCapabilityManifest {
                manifest_id: "test-manifest".into(),
                manifest_version: 1,
                issued_at_ms: now_ms - 1_000,
                expires_at_ms: now_ms + 60_000,
                routes: vec![ToolRoute::Device, ToolRoute::AgentHost, ToolRoute::Cloud],
                capabilities: vec![
                    "health".into(),
                    "process".into(),
                    "network".into(),
                    "files_read".into(),
                    "files_write".into(),
                    "calendar".into(),
                    "clipboard".into(),
                    "location".into(),
                ],
                authorizations: HashMap::from([
                    ("health".into(), CapabilityAuthorization::Granted),
                    ("process".into(), CapabilityAuthorization::Ask),
                    ("network".into(), CapabilityAuthorization::Granted),
                    ("files_read".into(), CapabilityAuthorization::Granted),
                    ("files_write".into(), CapabilityAuthorization::Granted),
                    ("calendar".into(), CapabilityAuthorization::Granted),
                    ("clipboard".into(), CapabilityAuthorization::Granted),
                    ("location".into(), CapabilityAuthorization::Granted),
                ]),
                ..RuntimeCapabilityManifest::default()
            },
            user_policy: UserToolPolicy {
                max_risk: ToolRisk::Critical,
                ..UserToolPolicy::default()
            },
            ..ToolPolicyInput::default()
        }
    }

    fn authorize_device_egress(input: &mut ToolPolicyInput, tool_ids: &[&str]) {
        let host_id = "device-host-1";
        let client_id = "controller-client-1";
        let session_id = "conversation-session-1";
        input.runtime.host_id = Some(host_id.into());
        input.runtime.client_id = Some(client_id.into());
        input.runtime.session_id = Some(session_id.into());
        input.expected_identity = RuntimeIdentity {
            host_id: Some(host_id.into()),
            client_id: Some(client_id.into()),
            session_id: Some(session_id.into()),
        };
        input.user_policy.allow_result_egress_routes = vec![ToolRoute::Device];
        input.runtime.result_egress_receipts = tool_ids
            .iter()
            .map(|tool_id| RuntimeResultEgressReceipt {
                tool_id: (*tool_id).into(),
                route: ToolRoute::Device,
                manifest_id: input.runtime.manifest_id.clone(),
                host_id: host_id.into(),
                client_id: client_id.into(),
                session_id: session_id.into(),
                issued_at_ms: input.now_ms - 1_000,
                expires_at_ms: input.now_ms + 30_000,
                allow_result_egress: true,
            })
            .collect();
        input.result_egress_grants = input
            .runtime
            .result_egress_receipts
            .iter()
            .map(|receipt| ResultEgressGrant {
                tool_id: receipt.tool_id.clone(),
                route: receipt.route,
                manifest_id: receipt.manifest_id.clone(),
                host_id: receipt.host_id.clone(),
                client_id: receipt.client_id.clone(),
                session_id: receipt.session_id.clone(),
                expires_at_ms: receipt.expires_at_ms,
            })
            .collect();
    }

    #[test]
    fn runtime_cannot_downgrade_core_metadata_with_untrusted_fields() {
        let mut value = serde_json::to_value(input(ToolPlatform::Cli)).unwrap();
        value["runtime"]["tools"] = json!([{
            "tool_id": "bash",
            "handler_id": "bash",
            "schema": {"type":"function","function":{"name":"bash","parameters":{"type":"object"}}},
            "route": "agent_host",
            "available": true,
            "capability": "none",
            "risk": "read_only",
            "permission": "none",
            "requires_result_egress": false
        }]);
        let parsed: ToolPolicyInput = serde_json::from_value(value).unwrap();
        let output = resolve_tool_policy(&parsed);
        assert_eq!(output.enabled_tools[0].capability, "process");
        assert_eq!(output.enabled_tools[0].risk, ToolRisk::High);
        assert_eq!(
            output.enabled_tools[0].permission,
            ToolPermission::AgentApproval
        );
    }

    #[test]
    fn unknown_tool_fails_closed() {
        let mut input = input(ToolPlatform::Cli);
        input.runtime.tools = vec![tool("invented_super_tool", ToolRoute::AgentHost)];
        let output = resolve_tool_policy(&input);
        assert_eq!(output.disabled_tools[0].reason, "unknown_tool_definition");
    }

    #[test]
    fn apple_device_ids_resolve_on_controller() {
        let mut input = input(ToolPlatform::Web);
        let tool_ids = [
            "device_read_file",
            "device_write_file",
            "device_clipboard_read",
            "device_clipboard_write",
            "device_get_location",
            "device_create_calendar_event",
            "device_search_calendar_events",
        ];
        input.runtime.tools = tool_ids
            .into_iter()
            .map(|name| tool(name, ToolRoute::Device))
            .collect();
        authorize_device_egress(&mut input, &tool_ids);
        let output = resolve_tool_policy(&input);
        assert_eq!(output.enabled_tools.len(), 7);
    }

    #[test]
    fn device_egress_requires_receipt_route_and_exact_session_grant() {
        let mut input = input(ToolPlatform::Web);
        input.runtime.tools = vec![tool("device_read_file", ToolRoute::Device)];

        let output = resolve_tool_policy(&input);
        assert_eq!(
            output.disabled_tools[0].reason,
            "result_egress_receipt_missing"
        );

        authorize_device_egress(&mut input, &["device_read_file"]);
        input.user_policy.allow_result_egress_routes.clear();
        let output = resolve_tool_policy(&input);
        assert_eq!(
            output.disabled_tools[0].reason,
            "result_egress_route_not_allowed"
        );

        input.user_policy.allow_result_egress_routes = vec![ToolRoute::Device];
        input.result_egress_grants.clear();
        let output = resolve_tool_policy(&input);
        assert_eq!(
            output.disabled_tools[0].reason,
            "result_egress_session_grant_missing"
        );

        input.result_egress_grants = vec![ResultEgressGrant {
            tool_id: "device_read_file".into(),
            route: ToolRoute::Device,
            manifest_id: input.runtime.manifest_id.clone(),
            host_id: "device-host-1".into(),
            client_id: "controller-client-1".into(),
            session_id: "wrong-session".into(),
            expires_at_ms: input.now_ms + 30_000,
        }];
        let output = resolve_tool_policy(&input);
        assert_eq!(
            output.disabled_tools[0].reason,
            "result_egress_session_grant_missing"
        );

        authorize_device_egress(&mut input, &["device_read_file"]);
        let output = resolve_tool_policy(&input);
        assert_eq!(output.enabled_tools[0].tool_id, "device_read_file");
    }

    #[test]
    fn device_internal_tools_do_not_require_result_egress_receipts() {
        let mut input = input(ToolPlatform::Ios);
        input.runtime.tools = ["skill_invoke", "memory_search", "memory_save"]
            .into_iter()
            .map(|name| tool(name, ToolRoute::Device))
            .collect();

        let output = resolve_tool_policy(&input);
        assert_eq!(output.enabled_tools.len(), 3, "{output:?}");
        assert!(output
            .enabled_tools
            .iter()
            .all(|tool| tool.capability == "none"
                && tool.permission == ToolPermission::None
                && !tool.requires_result_egress));
    }

    #[test]
    fn serialized_policy_contract_has_no_global_egress_fields() {
        let value = serde_json::to_value(input(ToolPlatform::Web)).unwrap();
        assert!(value["runtime"].get("allow_result_egress").is_none());
        assert!(value["user_policy"].get("allow_result_egress").is_none());
    }

    #[test]
    fn device_egress_receipt_must_match_identity_and_be_fresh() {
        let mut input = input(ToolPlatform::Web);
        input.runtime.tools = vec![tool("device_read_file", ToolRoute::Device)];
        authorize_device_egress(&mut input, &["device_read_file"]);

        input.runtime.result_egress_receipts[0].session_id = "wrong-session".into();
        assert_eq!(
            resolve_tool_policy(&input).disabled_tools[0].reason,
            "result_egress_receipt_missing"
        );

        authorize_device_egress(&mut input, &["device_read_file"]);
        input.runtime.result_egress_receipts[0].expires_at_ms = input.now_ms;
        assert_eq!(
            resolve_tool_policy(&input).disabled_tools[0].reason,
            "result_egress_receipt_missing"
        );
    }

    #[test]
    fn agent_host_without_egress_requirement_ignores_device_egress_grants() {
        let mut input = input(ToolPlatform::Cli);
        input.runtime.tools = vec![tool("read_file", ToolRoute::AgentHost)];
        let output = resolve_tool_policy(&input);
        assert_eq!(output.enabled_tools[0].tool_id, "read_file");
    }

    #[test]
    fn mainline_host_policy_recognizes_environment_recovery_tools() {
        for platform in [ToolPlatform::Cli, ToolPlatform::AgentHost, ToolPlatform::Macos] {
            let mut input = input(platform);
            input.runtime.tools = vec![tool("environment_probe", ToolRoute::AgentHost),
                tool("read_file", ToolRoute::AgentHost)];
            let output = resolve_tool_policy(&input);
            assert!(output.disabled_tools.is_empty(), "{platform:?}: {:?}", output.disabled_tools);
            assert_eq!(output.enabled_tools.len(), 2);
            assert!(output.enabled_tools.iter().all(|tool| tool.capability == "files_read"));
            // The environment probe does not acquire process/installation authority.
            assert!(!output.enabled_tools.iter().any(|tool| tool.capability == "process"));
        }
    }

    #[test]
    fn expired_and_wrong_identity_manifests_fail_closed() {
        let mut input = input(ToolPlatform::Cli);
        input.runtime.tools = vec![tool("read_file", ToolRoute::AgentHost)];
        input.runtime.expires_at_ms = input.now_ms;
        assert_eq!(
            resolve_tool_policy(&input).disabled_tools[0].reason,
            "manifest_expired"
        );
        input.runtime.expires_at_ms = input.now_ms + 1_000;
        input.expected_identity.host_id = Some("expected".into());
        input.runtime.host_id = Some("other".into());
        assert_eq!(
            resolve_tool_policy(&input).disabled_tools[0].reason,
            "manifest_host_identity_mismatch"
        );
    }

    #[test]
    fn scoped_grant_only_upgrades_ask() {
        let mut input = input(ToolPlatform::Cli);
        input.runtime.tools = vec![tool("bash", ToolRoute::AgentHost)];
        input.scoped_grants = vec![ScopedCapabilityGrant {
            capability: "process".into(),
            route: Some(ToolRoute::AgentHost),
            scope: Some("cwd:/tmp".into()),
            expires_at_ms: input.now_ms + 10_000,
        }];
        assert_eq!(
            resolve_tool_policy(&input).enabled_tools[0].authorization,
            CapabilityAuthorization::Granted
        );
        input
            .runtime
            .authorizations
            .insert("process".into(), CapabilityAuthorization::Denied);
        assert_eq!(
            resolve_tool_policy(&input).disabled_tools[0].authorization,
            CapabilityAuthorization::Denied
        );
    }

    #[test]
    fn digests_are_deterministic_across_map_order() {
        let mut first = input(ToolPlatform::Cli);
        first.runtime.tools = vec![tool("read_file", ToolRoute::AgentHost)];
        let mut second = first.clone();
        let entries = first
            .runtime
            .authorizations
            .iter()
            .map(|(key, value)| (key.clone(), *value))
            .collect::<Vec<_>>();
        second.runtime.authorizations = entries.into_iter().rev().collect();
        let first = resolve_tool_policy(&first);
        let second = resolve_tool_policy(&second);
        assert_eq!(first.policy_digest, second.policy_digest);
        assert_eq!(first.manifest_digest, second.manifest_digest);
    }

    #[test]
    fn pure_policy_returns_v1_contract() {
        let mut input = input(ToolPlatform::Cli);
        input.runtime.tools = vec![tool("read_file", ToolRoute::AgentHost)];
        let bytes = serde_json::to_vec(&input).unwrap();
        let decoded: ToolPolicyInput = serde_json::from_slice(&bytes).unwrap();
        let output = resolve_tool_policy(&decoded);
        assert_eq!(output.schema_version, 1);
        assert_eq!(output.enabled_tools[0].tool_id, "read_file");
    }

    #[test]
    fn shared_v1_golden_fixture_matches_core_receipt() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../fixtures/tool_policy_v1_golden.json")).unwrap();
        let input: ToolPolicyInput = serde_json::from_value(fixture["input"].clone()).unwrap();
        let output = resolve_tool_policy(&input);
        let expected = &fixture["expected"];
        let enabled_ids = output
            .enabled_tools
            .iter()
            .map(|tool| tool.tool_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(enabled_ids, vec!["device_read_file"]);
        assert_eq!(
            expected["enabled_tool_ids"],
            serde_json::to_value(enabled_ids).unwrap()
        );
        assert_eq!(output.disabled_tools.len(), 1);
        assert_eq!(output.disabled_tools[0].tool_id, "device_write_file");
        assert_eq!(
            output.disabled_tools[0].reason,
            expected["disabled_tools"][0]["reason"].as_str().unwrap()
        );
        let receipt = &expected["receipt"];
        assert_eq!(
            output.schema_version as u64,
            receipt["schema_version"].as_u64().unwrap()
        );
        assert_eq!(
            serde_json::to_value(output.platform).unwrap(),
            receipt["platform"]
        );
        assert_eq!(
            output.manifest_valid_until_ms,
            receipt["manifest_valid_until_ms"].as_u64().unwrap()
        );
        let actual_tool = serde_json::to_value(&output.enabled_tools[0]).unwrap();
        for key in [
            "tool_id",
            "name",
            "route",
            "capability",
            "permission",
            "authorization",
            "risk",
            "requires_result_egress",
            "reason",
        ] {
            assert_eq!(
                actual_tool[key], receipt["enabled_tool"][key],
                "receipt key {key}"
            );
        }
        let digest_len = receipt["digest_hex_length"].as_u64().unwrap() as usize;
        assert_eq!(output.policy_digest.len(), digest_len);
        assert_eq!(output.manifest_digest.len(), digest_len);
    }
    #[test]
    fn mcp_extensions_preserve_original_limits_without_borrowing_device_grants() {
        for platform in [ToolPlatform::Ios, ToolPlatform::Macos] {
        let mut policy=input(platform);
        if platform == ToolPlatform::Macos { policy.runtime.routes=vec![ToolRoute::AgentHost]; }
        assert!(mcp_host_extension_allowed(&policy,"mcp_connect","mcp_connection",ToolRisk::Medium));
        policy.user_policy.deny_capabilities.push("network".into());
        assert!(!mcp_host_extension_allowed(&policy,"mcp_connect","mcp_connection",ToolRisk::Medium));
        policy.user_policy.deny_capabilities.clear();policy.conversation.allow_tools.push("skill_invoke".into());
        assert!(!mcp_host_extension_allowed(&policy,"mcp_connect","mcp_connection",ToolRisk::Medium));
        policy.conversation.allow_tools.clear();policy.user_policy.max_risk=ToolRisk::ReadOnly;
        assert!(!mcp_host_extension_allowed(&policy,"mcp_connect","mcp_connection",ToolRisk::Medium));
        assert!(!mcp_host_extension_allowed(&policy,"device_read_file","files_read",ToolRisk::ReadOnly));
        }
    }

}

/// Host-owned extensions retain the caller's original allow/deny/risk intent.
/// This only admits a descriptor to the catalog; Core still asks permission
/// for the concrete MCP connection/call and its result disclosure. This is
/// the same external-Host lane as CLI MCP, not an OS device-read bridge; it
/// creates no device-result receipt or standing disclosure grant.
pub fn mcp_host_extension_allowed(input:&ToolPolicyInput,id:&str,capability:&str,risk:ToolRisk)->bool {
    let id=normalize_tool_id(id);
    let allowed=|values:&[String],candidate:&str| {let set=normalized_set(values);set.is_empty()||set.contains(candidate)};
    let denied=|values:&[String],candidate:&str| normalized_set(values).contains(candidate);
    validate_manifest(input).is_ok()
        && (input.runtime.routes.contains(&ToolRoute::Device)
            || (input.runtime.routes.contains(&ToolRoute::AgentHost)
                && normalized_set(&input.runtime.capabilities).contains("network")
                && matches!(input.runtime.authorizations.get("network"), Some(CapabilityAuthorization::Granted | CapabilityAuthorization::Ask))))
        && matches!(capability,"mcp_connection"|"mcp"|"mcp_destructive")
        && allowed(&input.conversation.allow_tools,&id) && !denied(&input.conversation.deny_tools,&id)
        && allowed(&input.user_policy.allow_tools,&id) && !denied(&input.user_policy.deny_tools,&id)
        && !denied(&input.user_policy.deny_capabilities,"network")
        && !denied(&input.user_policy.deny_capabilities,capability)
        && (input.user_policy.allow_capabilities.is_empty()
            || normalized_set(&input.user_policy.allow_capabilities).contains("network")
            || normalized_set(&input.user_policy.allow_capabilities).contains(capability))
        && risk<=input.user_policy.max_risk
}

pub fn mcp_host_descriptor_allowed(input: &ToolPolicyInput, descriptor: &crate::agent::host::HostCapabilityDescriptor) -> bool {
    use crate::agent::host::{HostCapabilityOriginKind, HostCapabilityEffect, HostCapabilityRoute};
    if descriptor.route != HostCapabilityRoute::Tool
        || descriptor.input_normalization.origin.kind != HostCapabilityOriginKind::Mcp { return false; }
    let risk = if descriptor.id == "mcp_connect" { ToolRisk::Medium }
        else if descriptor.effect_for(&serde_json::json!({})) == HostCapabilityEffect::Investigate { ToolRisk::ReadOnly }
        else { ToolRisk::High };
    mcp_host_extension_allowed(input, &descriptor.id, descriptor.permission_domain.as_deref().unwrap_or(""), risk)
}
