// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Typed MCP descriptors and deterministic Skill execution semantics.
// - Security: Concrete catalog references are resolved; natural-language text is never inspected.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const MCP_RESOURCE_READ_CAPABILITY_ID: &str = "mcp_resources_read";
pub const MCP_RESOURCE_TEMPLATE_READ_CAPABILITY_ID: &str = "mcp_resource_templates_read";
pub const MCP_PROMPT_GET_CAPABILITY_ID: &str = "mcp_prompts_get";
pub const MCP_SERVER_INSTRUCTIONS_MAX_CHARS: usize = 8_000;

/// Normalizes untrusted usage notes returned by MCP `initialize` without
/// splitting Unicode scalar values. Transport adapters must use this common
/// boundary so HTTP and stdio servers cannot project different prompt text.
pub fn bounded_mcp_server_instructions(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.is_empty() {
        return None;
    }
    let mut chars = value.chars();
    let bounded = chars
        .by_ref()
        .take(MCP_SERVER_INSTRUCTIONS_MAX_CHARS)
        .collect::<String>();
    Some(if chars.next().is_some() {
        format!("{bounded}\n[truncated]")
    } else {
        bounded
    })
}

#[cfg(test)]
mod mcp_instruction_contract_tests {
    use super::*;

    #[test]
    fn server_instructions_are_trimmed_and_empty_values_are_omitted() {
        assert_eq!(bounded_mcp_server_instructions(None), None);
        assert_eq!(bounded_mcp_server_instructions(Some("  \n\t")), None);
        assert_eq!(
            bounded_mcp_server_instructions(Some("  use exact resource ids  ")),
            Some("use exact resource ids".to_string())
        );
    }

    #[test]
    fn server_instructions_are_bounded_by_unicode_characters() {
        let input = "界".repeat(MCP_SERVER_INSTRUCTIONS_MAX_CHARS + 1);
        let bounded = bounded_mcp_server_instructions(Some(&input)).unwrap();
        let body = bounded.strip_suffix("\n[truncated]").unwrap();

        assert_eq!(body.chars().count(), MCP_SERVER_INSTRUCTIONS_MAX_CHARS);
        assert!(body.chars().all(|character| character == '界'));
    }

    #[test]
    fn typed_tool_references_resolve_to_model_capability_ids() {
        assert_eq!(
            CapabilityToolReference::Builtin {
                tool_name: "write_file".to_string(),
            }
            .model_capability_id(),
            "write_file"
        );
        let mcp = CapabilityToolReference::Mcp {
            server_id: "documents".to_string(),
            tool_name: "create".to_string(),
        };
        assert_eq!(
            mcp.model_capability_id(),
            mcp_model_tool_id("documents", "create")
        );
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpTransportKind {
    Stdio,
    StreamableHttp,
    ServerSentEvents,
    InProcess,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpServerAvailability {
    Available,
    Unavailable,
    AuthorizationRequired,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpToolDescriptor {
    pub name: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub input_schema: Value,
    #[serde(default)]
    pub output_schema: Option<Value>,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub destructive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpResourceDescriptor {
    pub resource_id: String,
    pub uri: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub mime_type: Option<String>,
    #[serde(default)]
    pub annotations: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpResourceTemplateDescriptor {
    pub template_id: String,
    pub uri_template: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpPromptArgumentDescriptor {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpPromptDescriptor {
    pub name: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub arguments: Vec<McpPromptArgumentDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpServerDescriptor {
    pub server_id: String,
    pub display_name: String,
    pub transport: McpTransportKind,
    /// Opaque Host-owned reference. Credentials must remain outside Core state.
    pub connection_ref: String,
    pub availability: McpServerAvailability,
    #[serde(default)]
    pub protocol_version: Option<String>,
    /// Bounded usage notes returned by MCP initialize. External reference only.
    #[serde(default)]
    pub instructions: Option<String>,
    #[serde(default)]
    pub tools: BTreeMap<String, McpToolDescriptor>,
    #[serde(default)]
    pub resources: BTreeMap<String, McpResourceDescriptor>,
    #[serde(default)]
    pub resource_templates: BTreeMap<String, McpResourceTemplateDescriptor>,
    #[serde(default)]
    pub prompts: BTreeMap<String, McpPromptDescriptor>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct McpCapabilityCatalog {
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub servers: BTreeMap<String, McpServerDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct McpToolReference {
    pub server_id: String,
    pub tool_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct McpResourceReference {
    pub server_id: String,
    pub resource_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct McpResourceTemplateReference {
    pub server_id: String,
    pub template_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct McpPromptReference {
    pub server_id: String,
    pub prompt_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct McpResourceReadRequest {
    pub server_id: String,
    pub resource_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct McpResourceTemplateReadRequest {
    pub server_id: String,
    pub template_id: String,
    #[serde(default)]
    pub variables: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct McpPromptGetRequest {
    pub server_id: String,
    pub prompt_name: String,
    #[serde(default)]
    pub arguments: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(tag = "provider", rename_all = "snake_case")]
pub enum CapabilityToolReference {
    Builtin {
        tool_name: String,
    },
    Mcp {
        server_id: String,
        tool_name: String,
    },
}

impl CapabilityToolReference {
    pub fn display_id(&self) -> String {
        match self {
            Self::Builtin { tool_name } => format!("builtin:{tool_name}"),
            Self::Mcp {
                server_id,
                tool_name,
            } => format!("mcp:{server_id}:{tool_name}"),
        }
    }

    /// Stable model-visible capability id resolved from this typed reference.
    /// Skill activation uses this identity directly; tool-result JSON is not
    /// reparsed into a second, weaker capability representation.
    pub fn model_capability_id(&self) -> String {
        match self {
            Self::Builtin { tool_name } => tool_name.clone(),
            Self::Mcp {
                server_id,
                tool_name,
            } => mcp_model_tool_id(server_id, tool_name),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityLookupKind {
    Server,
    Tool,
    Resource,
    ResourceTemplate,
    Prompt,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityRecovery {
    RefreshCatalogThenRetry,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecoverableCapabilityError {
    pub code: String,
    pub kind: CapabilityLookupKind,
    pub requested_id: String,
    #[serde(default)]
    pub server_id: Option<String>,
    pub catalog_revision: u64,
    /// Stable exact IDs, bounded to avoid putting the full catalog in an error.
    #[serde(default)]
    pub available_ids: Vec<String>,
    pub recovery: CapabilityRecovery,
}

impl McpCapabilityCatalog {
    /// Projects the exact connected MCP tool surface into provider-safe model
    /// function definitions. The opaque function name is stable and bounded;
    /// Hosts retain the structural route back to `(server_id, tool_name)`.
    pub fn model_tool_definitions(&self) -> Vec<Value> {
        let mut definitions = self
            .servers
            .values()
            .filter(|server| server.availability == McpServerAvailability::Available)
            .flat_map(|server| {
                server.tools.values().map(move |tool| {
                    let description = tool
                        .description
                        .clone()
                        .or_else(|| tool.title.clone())
                        .unwrap_or_else(|| {
                            format!("{} tool from {}", tool.name, server.display_name)
                        });
                    serde_json::json!({
                        "type": "function",
                        "function": {
                            "name": mcp_model_tool_id(&server.server_id, &tool.name),
                            "description": description,
                            "parameters": tool.input_schema,
                        }
                    })
                })
            })
            .collect::<Vec<_>>();
        if self.servers.values().any(|server| {
            server.availability == McpServerAvailability::Available && !server.resources.is_empty()
        }) {
            definitions.push(mcp_resource_read_model_definition());
        }
        if self.servers.values().any(|server| {
            server.availability == McpServerAvailability::Available
                && !server.resource_templates.is_empty()
        }) {
            definitions.push(mcp_resource_template_read_model_definition());
        }
        if self.servers.values().any(|server| {
            server.availability == McpServerAvailability::Available && !server.prompts.is_empty()
        }) {
            definitions.push(mcp_prompt_get_model_definition());
        }
        definitions
    }

    pub fn resolve_tool(
        &self,
        reference: &McpToolReference,
    ) -> Result<&McpToolDescriptor, RecoverableCapabilityError> {
        let server = self.server(&reference.server_id, CapabilityLookupKind::Tool)?;
        server.tools.get(&reference.tool_name).ok_or_else(|| {
            self.missing(
                "unknown_mcp_tool",
                CapabilityLookupKind::Tool,
                &reference.tool_name,
                Some(&reference.server_id),
                bounded_keys(&server.tools),
            )
        })
    }

    pub fn resolve_resource(
        &self,
        reference: &McpResourceReference,
    ) -> Result<&McpResourceDescriptor, RecoverableCapabilityError> {
        let server = self.server(&reference.server_id, CapabilityLookupKind::Resource)?;
        server.resources.get(&reference.resource_id).ok_or_else(|| {
            self.missing(
                "unknown_mcp_resource",
                CapabilityLookupKind::Resource,
                &reference.resource_id,
                Some(&reference.server_id),
                bounded_keys(&server.resources),
            )
        })
    }

    pub fn resolve_resource_template(
        &self,
        reference: &McpResourceTemplateReference,
    ) -> Result<&McpResourceTemplateDescriptor, RecoverableCapabilityError> {
        let server = self.server(&reference.server_id, CapabilityLookupKind::ResourceTemplate)?;
        server
            .resource_templates
            .get(&reference.template_id)
            .ok_or_else(|| {
                self.missing(
                    "unknown_mcp_resource_template",
                    CapabilityLookupKind::ResourceTemplate,
                    &reference.template_id,
                    Some(&reference.server_id),
                    bounded_keys(&server.resource_templates),
                )
            })
    }

    pub fn resolve_prompt(
        &self,
        reference: &McpPromptReference,
    ) -> Result<&McpPromptDescriptor, RecoverableCapabilityError> {
        let server = self.server(&reference.server_id, CapabilityLookupKind::Prompt)?;
        server.prompts.get(&reference.prompt_name).ok_or_else(|| {
            self.missing(
                "unknown_mcp_prompt",
                CapabilityLookupKind::Prompt,
                &reference.prompt_name,
                Some(&reference.server_id),
                bounded_keys(&server.prompts),
            )
        })
    }

    fn server(
        &self,
        server_id: &str,
        kind: CapabilityLookupKind,
    ) -> Result<&McpServerDescriptor, RecoverableCapabilityError> {
        if self.revision == 0 {
            return Err(self.missing(
                "unversioned_mcp_catalog",
                kind,
                server_id,
                Some(server_id),
                bounded_keys(&self.servers),
            ));
        }
        let server = self.servers.get(server_id).ok_or_else(|| {
            self.missing(
                "unknown_mcp_server",
                kind,
                server_id,
                Some(server_id),
                bounded_keys(&self.servers),
            )
        })?;
        if server
            .protocol_version
            .as_deref()
            .is_none_or(|version| version.trim().is_empty())
        {
            return Err(self.missing(
                "unversioned_mcp_server",
                kind,
                server_id,
                Some(server_id),
                Vec::new(),
            ));
        }
        Ok(server)
    }

    fn missing(
        &self,
        code: &str,
        kind: CapabilityLookupKind,
        requested_id: &str,
        server_id: Option<&str>,
        available_ids: Vec<String>,
    ) -> RecoverableCapabilityError {
        RecoverableCapabilityError {
            code: code.to_string(),
            kind,
            requested_id: requested_id.to_string(),
            server_id: server_id.map(str::to_string),
            catalog_revision: self.revision,
            available_ids,
            recovery: CapabilityRecovery::RefreshCatalogThenRetry,
        }
    }
}

pub fn mcp_resource_read_model_definition() -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": MCP_RESOURCE_READ_CAPABILITY_ID,
            "description": "Read one resource advertised by the frozen MCP catalog using its exact server and resource identifiers.",
            "parameters": {
                "type": "object",
                "properties": {
                    "server_id": {"type": "string", "minLength": 1, "maxLength": 512},
                    "resource_id": {"type": "string", "minLength": 1, "maxLength": 2048}
                },
                "required": ["server_id", "resource_id"],
                "additionalProperties": false
            }
        }
    })
}

pub fn mcp_resource_template_read_model_definition() -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": MCP_RESOURCE_TEMPLATE_READ_CAPABILITY_ID,
            "description": "Expand and read one parameterized resource advertised by the frozen MCP catalog using its exact server and template identifiers.",
            "parameters": {
                "type": "object",
                "properties": {
                    "server_id": {"type": "string", "minLength": 1, "maxLength": 512},
                    "template_id": {"type": "string", "minLength": 1, "maxLength": 4096},
                    "variables": {
                        "type": "object",
                        "maxProperties": 64,
                        "additionalProperties": {"type": "string", "maxLength": 16384}
                    }
                },
                "required": ["server_id", "template_id", "variables"],
                "additionalProperties": false
            }
        }
    })
}

pub fn mcp_prompt_get_model_definition() -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": MCP_PROMPT_GET_CAPABILITY_ID,
            "description": "Get one prompt advertised by the frozen MCP catalog using its exact server and prompt identifiers.",
            "parameters": {
                "type": "object",
                "properties": {
                    "server_id": {"type": "string", "minLength": 1, "maxLength": 512},
                    "prompt_name": {"type": "string", "minLength": 1, "maxLength": 512},
                    "arguments": {
                        "type": "object",
                        "maxProperties": 64,
                        "additionalProperties": {"type": "string", "maxLength": 16384}
                    }
                },
                "required": ["server_id", "prompt_name"],
                "additionalProperties": false
            }
        }
    })
}

/// Stable provider-safe identity for one concrete MCP tool. This is an opaque
/// transport identifier, not a user-facing label and not a routing heuristic.
/// 28 SHA-256 bytes keep the full name below common 64-character limits.
pub fn mcp_model_tool_id(server_id: &str, tool_name: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"nola.mcp.model-tool.v1\0");
    hasher.update(server_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(tool_name.as_bytes());
    let digest = hasher.finalize();
    let encoded = digest[..28]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("mcp_{encoded}")
}

fn bounded_keys<T>(values: &BTreeMap<String, T>) -> Vec<String> {
    values.keys().take(20).cloned().collect()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct CapabilityCatalog {
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub builtin_tools: BTreeSet<String>,
    #[serde(default)]
    pub mcp: McpCapabilityCatalog,
}

/// One atomic Host refresh: Skill dependency resolution and executable routes
/// must advance together, under the correlation of the original request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityCatalogRefresh {
    pub catalog: CapabilityCatalog,
    pub host_catalog: super::host::HostCapabilityCatalog,
}

impl CapabilityCatalog {
    pub fn resolve_tool(
        &self,
        reference: &CapabilityToolReference,
    ) -> Result<(), RecoverableCapabilityError> {
        if self.revision == 0 {
            return Err(catalog_lookup_error(
                "unversioned_capability_catalog",
                reference,
                self.revision,
                Vec::new(),
            ));
        }
        match reference {
            CapabilityToolReference::Builtin { tool_name }
                if self.builtin_tools.contains(tool_name) =>
            {
                Ok(())
            }
            CapabilityToolReference::Builtin { tool_name } => Err(RecoverableCapabilityError {
                code: "unknown_builtin_tool".to_string(),
                kind: CapabilityLookupKind::Tool,
                requested_id: tool_name.clone(),
                server_id: None,
                catalog_revision: self.revision,
                available_ids: self.builtin_tools.iter().take(20).cloned().collect(),
                recovery: CapabilityRecovery::RefreshCatalogThenRetry,
            }),
            CapabilityToolReference::Mcp {
                server_id,
                tool_name,
            } => {
                if self.mcp.revision != self.revision {
                    return Err(catalog_lookup_error(
                        "mcp_catalog_revision_mismatch",
                        reference,
                        self.revision,
                        self.mcp.servers.keys().take(20).cloned().collect(),
                    ));
                }
                self.mcp
                    .resolve_tool(&McpToolReference {
                        server_id: server_id.clone(),
                        tool_name: tool_name.clone(),
                    })
                    .map(|_| ())
            }
        }
    }
}

fn catalog_lookup_error(
    code: &str,
    reference: &CapabilityToolReference,
    catalog_revision: u64,
    available_ids: Vec<String>,
) -> RecoverableCapabilityError {
    let (requested_id, server_id) = match reference {
        CapabilityToolReference::Builtin { tool_name } => (tool_name.clone(), None),
        CapabilityToolReference::Mcp {
            server_id,
            tool_name,
        } => (tool_name.clone(), Some(server_id.clone())),
    };
    RecoverableCapabilityError {
        code: code.to_string(),
        kind: CapabilityLookupKind::Tool,
        requested_id,
        server_id,
        catalog_revision,
        available_ids,
        recovery: CapabilityRecovery::RefreshCatalogThenRetry,
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillExecutionMode {
    Inline,
    Fork,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillDescriptor {
    pub skill_id: String,
    pub version: String,
    pub display_name: String,
    /// Opaque immutable content reference materialized by the Host.
    pub instructions_ref: String,
    /// Lowercase SHA-256 of the exact instructions bytes materialized by the Host.
    ///
    /// Core validates the required content identity without learning how a
    /// platform stores the Skill. Missing identity is an invalid current format.
    pub instructions_digest: String,
    #[serde(default)]
    pub required_tools: Vec<CapabilityToolReference>,
    #[serde(default)]
    pub default_mode: Option<SkillExecutionMode>,
    #[serde(default)]
    pub execution_contract: SkillExecutionContract,
}

pub const SKILL_EXECUTION_CONTRACT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkillExecutionContract {
    #[serde(default = "default_skill_execution_contract_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub progress_steps: Vec<SkillProgressStep>,
    #[serde(default)]
    pub artifacts: Vec<SkillArtifactContract>,
    #[serde(default)]
    pub verifiers: Vec<SkillVerifierContract>,
    #[serde(default)]
    pub resume_points: Vec<String>,
}

impl Default for SkillExecutionContract {
    fn default() -> Self {
        Self {
            schema_version: SKILL_EXECUTION_CONTRACT_SCHEMA_VERSION,
            progress_steps: Vec::new(),
            artifacts: Vec::new(),
            verifiers: Vec::new(),
            resume_points: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkillProgressStep {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkillArtifactContract {
    pub id: String,
    pub media_type: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkillVerifierContract {
    pub id: String,
    pub description: String,
    pub method: SkillVerificationMethod,
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillVerificationMethod { ModelJudgment, HostCheck }
impl SkillVerificationMethod {
    pub fn as_str(self) -> &'static str {
        match self { Self::ModelJudgment => "model_judgment", Self::HostCheck => "host_check" }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SkillEvidenceReference {
    ExecutionEvidence { evidence_id: String },
    ToolCall { call_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SkillVerificationEvidence {
    ModelJudgment { sources: Vec<SkillEvidenceReference> },
    HostCheck {
        source: SkillEvidenceReference,
        resource_uri: String,
        version: String,
        check_kind: nola_domain::execution_evidence::ExecutionCheckKind,
        predicate: String,
    },
}
impl SkillVerificationEvidence {
    pub fn method(&self) -> SkillVerificationMethod {
        match self { Self::ModelJudgment { .. } => SkillVerificationMethod::ModelJudgment, Self::HostCheck { .. } => SkillVerificationMethod::HostCheck }
    }
    pub fn sources(&self) -> Vec<&SkillEvidenceReference> {
        match self { Self::ModelJudgment { sources } => sources.iter().collect(), Self::HostCheck { source, .. } => vec![source] }
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.sources().len() > 16 { return Err("Skill verification has too many evidence sources".into()); }
        for source in self.sources() {
            let id = match source { SkillEvidenceReference::ExecutionEvidence { evidence_id } => evidence_id,
                SkillEvidenceReference::ToolCall { call_id } => call_id };
            if id.trim().is_empty() || id.trim() != id || id.len() > 256 { return Err("Skill verification requires exact source identities".into()); }
        }
        if let Self::HostCheck { resource_uri, version, predicate, .. } = self {
            if resource_uri.trim().is_empty() || version.trim().is_empty() || predicate.trim().is_empty() {
                return Err("Host verification requires a concrete resource version and check predicate".into());
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkillVerificationResult {
    pub verifier_id: String,
    pub passed: bool,
    pub evidence: SkillVerificationEvidence,
    pub replaces_evidence: Option<SkillVerificationEvidence>,
    #[serde(default)]
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillRunBudget {
    pub max_model_turns: u32,
    pub max_tool_calls: u32,
    #[serde(default)]
    pub max_elapsed_millis: Option<u64>,
}

/// Immutable Skill materialization returned by a Host after executing the
/// model-visible Skill capability. Core, not the Host, owns the resulting run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MaterializedSkillInvocation {
    pub skill_id: String,
    pub version: String,
    #[serde(default)]
    pub display_name: String,
    pub instructions_ref: String,
    pub instructions_digest: String,
    #[serde(default)]
    pub required_tools: Vec<CapabilityToolReference>,
    #[serde(default)]
    pub mode: Option<SkillExecutionMode>,
    #[serde(default)]
    pub budget: Option<SkillRunBudget>,
    #[serde(default)]
    pub execution_contract: SkillExecutionContract,
}

impl MaterializedSkillInvocation {
    pub fn descriptor(&self) -> SkillDescriptor {
        SkillDescriptor {
            skill_id: self.skill_id.clone(),
            version: self.version.clone(),
            display_name: if self.display_name.trim().is_empty() {
                self.skill_id.clone()
            } else {
                self.display_name.clone()
            },
            instructions_ref: self.instructions_ref.clone(),
            instructions_digest: self.instructions_digest.clone(),
            required_tools: self.required_tools.clone(),
            default_mode: self.mode,
            execution_contract: self.execution_contract.clone(),
        }
    }
}

/// Typed, Host-materialized update emitted by the internal `skill_update`
/// capability. Every update names its Core-owned SkillRun so nested or
/// sequential Skills in one turn cannot mutate each other's completion state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum MaterializedSkillRuntimeUpdate {
    Progress {
        run_id: String,
        step_id: String,
        completed: bool,
    },
    Artifact {
        run_id: String,
        artifact_id: String,
        artifact_ref: String,
    },
    Verification {
        run_id: String,
        verifier_id: String,
        passed: bool,
        evidence: SkillVerificationEvidence,
        replaces_evidence: Option<SkillVerificationEvidence>,
        #[serde(default)]
        summary: String,
    },
    ResumePoint {
        run_id: String,
        resume_point: String,
    },
}

impl Default for SkillRunBudget {
    fn default() -> Self {
        Self {
            // A Skill is a completion contract, not a fixed-round batch job.
            // Ordinary cancellation, capability policy and provider timeouts
            // remain the safety boundaries. A small implicit budget could
            // terminate a valid document workflow after validation but before
            // the final user-facing handoff.
            max_model_turns: u32::MAX,
            max_tool_calls: u32::MAX,
            max_elapsed_millis: None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillRunUsage {
    pub model_turns: u32,
    pub tool_calls: u32,
    pub elapsed_millis: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillRunOwner {
    pub session_id: String,
    pub parent_turn_id: String,
    #[serde(default)]
    pub child_turn_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillRunStatus {
    Running,
    WaitingForCatalog,
    Completed,
    Failed,
    Cancelled,
}

impl SkillRunStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillRun {
    pub run_id: String,
    pub owner: SkillRunOwner,
    pub descriptor: SkillDescriptor,
    pub mode: SkillExecutionMode,
    pub status: SkillRunStatus,
    pub budget: SkillRunBudget,
    pub usage: SkillRunUsage,
    #[serde(default)]
    pub missing_tools: Vec<CapabilityToolReference>,
    pub catalog_revision: u64,
    #[serde(default)]
    pub result_ref: Option<String>,
    #[serde(default)]
    pub failure: Option<String>,
    #[serde(default)]
    pub current_progress_step: Option<String>,
    #[serde(default)]
    pub completed_progress_steps: BTreeSet<String>,
    #[serde(default)]
    pub artifacts: BTreeMap<String, String>,
    #[serde(default)]
    pub verifications: BTreeMap<String, SkillVerificationResult>,
    #[serde(default)]
    pub resume_point: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SkillRuntimeState {
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub next_event_sequence: u64,
    #[serde(default)]
    pub accepted_operation_ids: BTreeSet<String>,
    #[serde(default)]
    pub catalog: CapabilityCatalog,
    #[serde(default)]
    pub runs: BTreeMap<String, SkillRun>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillRuntimeCommandEnvelope {
    pub operation_id: String,
    pub command: SkillRuntimeCommand,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SkillRuntimeCommand {
    ReplaceCatalog {
        catalog: CapabilityCatalog,
    },
    Start {
        run_id: String,
        owner: SkillRunOwner,
        descriptor: SkillDescriptor,
        mode: SkillExecutionMode,
        budget: SkillRunBudget,
    },
    RecordModelTurn {
        run_id: String,
    },
    RecordToolCall {
        run_id: String,
        tool: CapabilityToolReference,
    },
    RecordElapsed {
        run_id: String,
        elapsed_millis: u64,
    },
    RecordProgress {
        run_id: String,
        step_id: String,
        completed: bool,
    },
    RegisterArtifact {
        run_id: String,
        artifact_id: String,
        artifact_ref: String,
    },
    RecordVerification {
        run_id: String,
        result: SkillVerificationResult,
    },
    SetResumePoint {
        run_id: String,
        resume_point: String,
    },
    Complete {
        run_id: String,
        result_ref: Option<String>,
    },
    Fail {
        run_id: String,
        error: String,
    },
    Cancel {
        run_id: String,
        reason: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillRuntimeEventEnvelope {
    pub sequence: u64,
    pub revision: u64,
    pub event: SkillRuntimeEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SkillRuntimeEvent {
    CatalogReplaced {
        revision: u64,
    },
    Started {
        run: SkillRun,
    },
    CapabilityRefreshRequested {
        run_id: String,
        errors: Vec<RecoverableCapabilityError>,
    },
    Resumed {
        run_id: String,
        catalog_revision: u64,
    },
    UsageUpdated {
        run_id: String,
        usage: SkillRunUsage,
    },
    ProgressUpdated {
        run_id: String,
        step_id: String,
        completed: bool,
    },
    ArtifactRegistered {
        run_id: String,
        artifact_id: String,
        artifact_ref: String,
    },
    VerificationRecorded {
        run_id: String,
        result: SkillVerificationResult,
    },
    ResumePointUpdated {
        run_id: String,
        resume_point: String,
    },
    BudgetExceeded {
        run_id: String,
        dimension: SkillBudgetDimension,
    },
    Completed {
        run_id: String,
        result_ref: Option<String>,
    },
    Failed {
        run_id: String,
        error: String,
    },
    Cancelled {
        run_id: String,
        reason: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillBudgetDimension {
    ModelTurns,
    ToolCalls,
    ElapsedMillis,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillRuntimeTransition {
    pub revision: u64,
    pub duplicate: bool,
    pub events: Vec<SkillRuntimeEventEnvelope>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillRuntimeError {
    pub code: String,
    pub message: String,
}

impl std::fmt::Display for SkillRuntimeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for SkillRuntimeError {}

impl SkillRuntimeState {
    /// Removes live capability discovery state before a completed Turn becomes
    /// durable history. Skill runs already pin the exact descriptor, version,
    /// content identity, and tool references that they used, so retaining the
    /// catalog would persist unrelated optional capabilities. Catalog replace
    /// operation ids are removed with the snapshot so a later Turn can hydrate
    /// the same Host revision again instead of treating it as a duplicate.
    pub fn redact_catalog_for_history(&mut self) {
        self.catalog = CapabilityCatalog::default();
        self.accepted_operation_ids
            .retain(|operation_id| !operation_id.starts_with("host_catalog:"));
    }

    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
            && self.catalog.revision == 0
            && self.revision == 0
            && self.next_event_sequence == 0
            && self.accepted_operation_ids.is_empty()
    }

    pub fn apply(
        &mut self,
        envelope: SkillRuntimeCommandEnvelope,
    ) -> Result<SkillRuntimeTransition, SkillRuntimeError> {
        if envelope.operation_id.trim().is_empty() {
            return Err(runtime_error(
                "invalid_operation_id",
                "operation_id must not be empty",
            ));
        }
        if self.accepted_operation_ids.contains(&envelope.operation_id) {
            return Ok(SkillRuntimeTransition {
                revision: self.revision,
                duplicate: true,
                events: Vec::new(),
            });
        }
        let events = self.apply_command(envelope.command)?;
        self.accepted_operation_ids.insert(envelope.operation_id);
        self.revision = self.revision.saturating_add(1);
        let revision = self.revision;
        let events = events
            .into_iter()
            .map(|event| {
                self.next_event_sequence = self.next_event_sequence.saturating_add(1);
                SkillRuntimeEventEnvelope {
                    sequence: self.next_event_sequence,
                    revision,
                    event,
                }
            })
            .collect();
        Ok(SkillRuntimeTransition {
            revision,
            duplicate: false,
            events,
        })
    }

    fn apply_command(
        &mut self,
        command: SkillRuntimeCommand,
    ) -> Result<Vec<SkillRuntimeEvent>, SkillRuntimeError> {
        match command {
            SkillRuntimeCommand::ReplaceCatalog { catalog } => {
                validate_catalog(&catalog)?;
                if catalog.revision < self.catalog.revision {
                    return Err(runtime_error(
                        "stale_catalog",
                        "catalog revision moved backwards",
                    ));
                }
                if catalog.revision == self.catalog.revision && catalog != self.catalog {
                    return Err(runtime_error(
                        "catalog_revision_conflict",
                        "catalog content changed without a revision increase",
                    ));
                }
                self.catalog = catalog;
                let revision = self.catalog.revision;
                let catalog = self.catalog.clone();
                let mut resumed = Vec::new();
                for run in self.runs.values_mut() {
                    if run.status != SkillRunStatus::WaitingForCatalog {
                        continue;
                    }
                    run.missing_tools = missing_tools(&catalog, &run.missing_tools);
                    run.catalog_revision = revision;
                    if run.missing_tools.is_empty() {
                        run.status = SkillRunStatus::Running;
                        resumed.push(SkillRuntimeEvent::Resumed {
                            run_id: run.run_id.clone(),
                            catalog_revision: revision,
                        });
                    }
                }
                let mut events = vec![SkillRuntimeEvent::CatalogReplaced { revision }];
                events.extend(resumed);
                Ok(events)
            }
            SkillRuntimeCommand::Start {
                run_id,
                owner,
                descriptor,
                mode,
                budget,
            } => {
                validate_start(&run_id, &owner, &descriptor, mode, &budget)?;
                if self.runs.contains_key(&run_id) {
                    return Err(runtime_error("duplicate_run_id", "run_id already exists"));
                }
                let missing = missing_tools(&self.catalog, &descriptor.required_tools);
                let status = if missing.is_empty() {
                    SkillRunStatus::Running
                } else {
                    SkillRunStatus::WaitingForCatalog
                };
                let current_progress_step = descriptor
                    .execution_contract
                    .progress_steps
                    .first()
                    .map(|step| step.id.clone());
                let resume_point = descriptor.execution_contract.resume_points.first().cloned();
                let run = SkillRun {
                    run_id: run_id.clone(),
                    owner,
                    descriptor,
                    mode,
                    status,
                    budget,
                    usage: SkillRunUsage::default(),
                    missing_tools: missing.clone(),
                    catalog_revision: self.catalog.revision,
                    result_ref: None,
                    failure: None,
                    current_progress_step,
                    completed_progress_steps: BTreeSet::new(),
                    artifacts: BTreeMap::new(),
                    verifications: BTreeMap::new(),
                    resume_point,
                };
                self.runs.insert(run_id.clone(), run.clone());
                let mut events = vec![SkillRuntimeEvent::Started { run }];
                if !missing.is_empty() {
                    events.push(SkillRuntimeEvent::CapabilityRefreshRequested {
                        run_id,
                        errors: missing
                            .iter()
                            .filter_map(|tool| self.catalog.resolve_tool(tool).err())
                            .collect(),
                    });
                }
                Ok(events)
            }
            SkillRuntimeCommand::RecordModelTurn { run_id } => {
                self.update_usage(run_id, UsageIncrement::ModelTurn)
            }
            SkillRuntimeCommand::RecordToolCall { run_id, tool } => {
                if let Err(error) = self.catalog.resolve_tool(&tool) {
                    let revision = self.catalog.revision;
                    let run = active_run_mut(&mut self.runs, &run_id)?;
                    run.status = SkillRunStatus::WaitingForCatalog;
                    run.missing_tools = vec![tool];
                    run.catalog_revision = revision;
                    return Ok(vec![SkillRuntimeEvent::CapabilityRefreshRequested {
                        run_id,
                        errors: vec![error],
                    }]);
                }
                self.update_usage(run_id, UsageIncrement::ToolCall)
            }
            SkillRuntimeCommand::RecordElapsed {
                run_id,
                elapsed_millis,
            } => self.update_usage(run_id, UsageIncrement::Elapsed(elapsed_millis)),
            SkillRuntimeCommand::RecordProgress {
                run_id,
                step_id,
                completed,
            } => {
                let run = active_run_mut(&mut self.runs, &run_id)?;
                if !run
                    .descriptor
                    .execution_contract
                    .progress_steps
                    .iter()
                    .any(|step| step.id == step_id)
                {
                    return Err(runtime_error(
                        "unknown_skill_progress_step",
                        "Skill progress step is not declared by the execution contract",
                    ));
                }
                if completed {
                    run.completed_progress_steps.insert(step_id.clone());
                } else {
                    run.completed_progress_steps.remove(&step_id);
                }
                run.current_progress_step = (!completed).then_some(step_id.clone()).or_else(|| {
                    run.descriptor
                        .execution_contract
                        .progress_steps
                        .iter()
                        .find(|step| !run.completed_progress_steps.contains(&step.id))
                        .map(|step| step.id.clone())
                });
                Ok(vec![SkillRuntimeEvent::ProgressUpdated {
                    run_id,
                    step_id,
                    completed,
                }])
            }
            SkillRuntimeCommand::RegisterArtifact {
                run_id,
                artifact_id,
                artifact_ref,
            } => {
                let run = active_run_mut(&mut self.runs, &run_id)?;
                if artifact_ref.trim().is_empty()
                    || !run
                        .descriptor
                        .execution_contract
                        .artifacts
                        .iter()
                        .any(|artifact| artifact.id == artifact_id)
                {
                    return Err(runtime_error(
                        "invalid_skill_artifact",
                        "Skill artifact must be declared and have a non-empty reference",
                    ));
                }
                if let Some(existing) = run.artifacts.get(&artifact_id) {
                    if existing != &artifact_ref {
                        return Err(runtime_error(
                            "skill_artifact_conflict",
                            "Skill artifact id is already bound to a different reference",
                        ));
                    }
                    return Ok(Vec::new());
                }
                run.artifacts
                    .insert(artifact_id.clone(), artifact_ref.clone());
                Ok(vec![SkillRuntimeEvent::ArtifactRegistered {
                    run_id,
                    artifact_id,
                    artifact_ref,
                }])
            }
            SkillRuntimeCommand::RecordVerification { run_id, result } => {
                let run = active_run_mut(&mut self.runs, &run_id)?;
                if result.evidence.validate().is_err()
                    || !run
                        .descriptor
                        .execution_contract
                        .verifiers
                        .iter()
                        .any(|verifier| verifier.id == result.verifier_id && verifier.method == result.evidence.method())
                {
                    return Err(runtime_error(
                        "invalid_skill_verification",
                        "Skill verification must use its declared method and structured evidence",
                    ));
                }
                if let Some(existing) = run.verifications.get(&result.verifier_id) {
                    if existing == &result { return Ok(Vec::new()); }
                    if result.replaces_evidence.as_ref() != Some(&existing.evidence) {
                        return Err(runtime_error(
                            "skill_verification_conflict",
                            "Skill verifier replacement must name the exact previous evidence",
                        ));
                    }
                } else if result.replaces_evidence.is_some() {
                    return Err(runtime_error("skill_verification_replacement_missing", "Skill verifier has no current evidence to replace"));
                }
                run.verifications
                    .insert(result.verifier_id.clone(), result.clone());
                Ok(vec![SkillRuntimeEvent::VerificationRecorded {
                    run_id,
                    result,
                }])
            }
            SkillRuntimeCommand::SetResumePoint {
                run_id,
                resume_point,
            } => {
                let run = active_run_mut(&mut self.runs, &run_id)?;
                if !run
                    .descriptor
                    .execution_contract
                    .resume_points
                    .contains(&resume_point)
                {
                    return Err(runtime_error(
                        "unknown_skill_resume_point",
                        "Skill resume point is not declared by the execution contract",
                    ));
                }
                run.resume_point = Some(resume_point.clone());
                Ok(vec![SkillRuntimeEvent::ResumePointUpdated {
                    run_id,
                    resume_point,
                }])
            }
            SkillRuntimeCommand::Complete { run_id, result_ref } => {
                let run = run_mut(&mut self.runs, &run_id)?;
                if run.status.is_terminal() {
                    return Ok(Vec::new());
                }
                if run.status != SkillRunStatus::Running {
                    return Err(runtime_error("run_not_running", "Skill run is not running"));
                }
                validate_skill_completion(run)?;
                run.status = SkillRunStatus::Completed;
                run.result_ref = result_ref.clone();
                Ok(vec![SkillRuntimeEvent::Completed { run_id, result_ref }])
            }
            SkillRuntimeCommand::Fail { run_id, error } => {
                let run = run_mut(&mut self.runs, &run_id)?;
                if run.status.is_terminal() {
                    return Ok(Vec::new());
                }
                run.status = SkillRunStatus::Failed;
                run.failure = Some(error.clone());
                Ok(vec![SkillRuntimeEvent::Failed { run_id, error }])
            }
            SkillRuntimeCommand::Cancel { run_id, reason } => {
                let run = run_mut(&mut self.runs, &run_id)?;
                if run.status.is_terminal() {
                    return Ok(Vec::new());
                }
                run.status = SkillRunStatus::Cancelled;
                Ok(vec![SkillRuntimeEvent::Cancelled { run_id, reason }])
            }
        }
    }

    fn update_usage(
        &mut self,
        run_id: String,
        increment: UsageIncrement,
    ) -> Result<Vec<SkillRuntimeEvent>, SkillRuntimeError> {
        let run = active_run_mut(&mut self.runs, &run_id)?;
        match increment {
            UsageIncrement::ModelTurn if run.usage.model_turns >= run.budget.max_model_turns => {
                return Ok(exhaust(run, SkillBudgetDimension::ModelTurns))
            }
            UsageIncrement::ToolCall if run.usage.tool_calls >= run.budget.max_tool_calls => {
                return Ok(exhaust(run, SkillBudgetDimension::ToolCalls))
            }
            UsageIncrement::Elapsed(value)
                if run.budget.max_elapsed_millis.is_some_and(|maximum| {
                    run.usage.elapsed_millis.saturating_add(value) > maximum
                }) =>
            {
                return Ok(exhaust(run, SkillBudgetDimension::ElapsedMillis))
            }
            UsageIncrement::ModelTurn => {
                run.usage.model_turns = run.usage.model_turns.saturating_add(1)
            }
            UsageIncrement::ToolCall => {
                run.usage.tool_calls = run.usage.tool_calls.saturating_add(1)
            }
            UsageIncrement::Elapsed(value) => {
                run.usage.elapsed_millis = run.usage.elapsed_millis.saturating_add(value)
            }
        }
        Ok(vec![SkillRuntimeEvent::UsageUpdated {
            run_id,
            usage: run.usage.clone(),
        }])
    }
}

enum UsageIncrement {
    ModelTurn,
    ToolCall,
    Elapsed(u64),
}

fn missing_tools(
    catalog: &CapabilityCatalog,
    tools: &[CapabilityToolReference],
) -> Vec<CapabilityToolReference> {
    tools
        .iter()
        .filter(|tool| catalog.resolve_tool(tool).is_err())
        .cloned()
        .collect()
}

fn validate_catalog(catalog: &CapabilityCatalog) -> Result<(), SkillRuntimeError> {
    if catalog.revision == 0 {
        return Err(runtime_error(
            "unversioned_capability_catalog",
            "capability catalog revision must be greater than zero",
        ));
    }
    if !catalog.mcp.servers.is_empty() && catalog.mcp.revision != catalog.revision {
        return Err(runtime_error(
            "mcp_catalog_revision_mismatch",
            "MCP catalog revision must match the enclosing capability catalog",
        ));
    }
    for (server_id, server) in &catalog.mcp.servers {
        if server_id != &server.server_id
            || server
                .protocol_version
                .as_deref()
                .is_none_or(|version| version.trim().is_empty())
        {
            return Err(runtime_error(
                "invalid_mcp_server_identity",
                "MCP server identity and protocol version must be explicit",
            ));
        }
        if server.tools.iter().any(|(id, tool)| id != &tool.name)
            || server
                .resources
                .iter()
                .any(|(id, resource)| id != &resource.resource_id)
            || server
                .resource_templates
                .iter()
                .any(|(id, template)| id != &template.template_id)
            || server.prompts.iter().any(|(id, prompt)| id != &prompt.name)
        {
            return Err(runtime_error(
                "invalid_mcp_resource_identity",
                "MCP catalog keys must exactly match their descriptor identities",
            ));
        }
    }
    Ok(())
}

fn validate_start(
    run_id: &str,
    owner: &SkillRunOwner,
    descriptor: &SkillDescriptor,
    mode: SkillExecutionMode,
    budget: &SkillRunBudget,
) -> Result<(), SkillRuntimeError> {
    if run_id.trim().is_empty()
        || owner.session_id.trim().is_empty()
        || owner.parent_turn_id.trim().is_empty()
        || descriptor.skill_id.trim().is_empty()
        || descriptor.version.trim().is_empty()
        || descriptor.instructions_ref.trim().is_empty()
    {
        return Err(runtime_error(
            "invalid_skill_run",
            "Skill run identifiers must not be empty",
        ));
    }
    if !is_lowercase_sha256(&descriptor.instructions_digest)
        || descriptor
            .instructions_ref
            .rsplit_once("#sha256=")
            .is_none_or(|(_, digest)| digest != descriptor.instructions_digest)
    {
        return Err(runtime_error(
            "invalid_skill_content_identity",
            "Skill instructions_ref must identify the exact materialized SHA-256",
        ));
    }
    match (mode, owner.child_turn_id.as_deref()) {
        (SkillExecutionMode::Inline, None) => {}
        (SkillExecutionMode::Fork, Some(child))
            if !child.trim().is_empty() && child != owner.parent_turn_id => {}
        (SkillExecutionMode::Inline, Some(_)) => {
            return Err(runtime_error(
                "invalid_inline_owner",
                "inline Skill cannot own a child Turn",
            ))
        }
        (SkillExecutionMode::Fork, _) => {
            return Err(runtime_error(
                "invalid_fork_owner",
                "forked Skill requires a distinct child Turn",
            ))
        }
    }
    if budget.max_model_turns == 0 || budget.max_tool_calls == 0 {
        return Err(runtime_error(
            "invalid_skill_budget",
            "Skill budget must permit work",
        ));
    }
    validate_execution_contract(&descriptor.execution_contract)?;
    Ok(())
}

fn validate_execution_contract(contract: &SkillExecutionContract) -> Result<(), SkillRuntimeError> {
    if contract.schema_version != SKILL_EXECUTION_CONTRACT_SCHEMA_VERSION {
        return Err(runtime_error(
            "unsupported_skill_execution_contract",
            "Skill execution contract schema version is not supported",
        ));
    }
    let mut progress = BTreeSet::new();
    for step in &contract.progress_steps {
        if step.id.trim().is_empty()
            || step.title.trim().is_empty()
            || !progress.insert(step.id.clone())
        {
            return Err(runtime_error(
                "invalid_skill_progress_contract",
                "Skill progress steps require unique non-empty ids and titles",
            ));
        }
    }
    let mut artifacts = BTreeSet::new();
    for artifact in &contract.artifacts {
        if artifact.id.trim().is_empty()
            || artifact.media_type.trim().is_empty()
            || !artifacts.insert(artifact.id.clone())
        {
            return Err(runtime_error(
                "invalid_skill_artifact_contract",
                "Skill artifacts require unique ids and media types",
            ));
        }
    }
    let mut verifiers = BTreeSet::new();
    for verifier in &contract.verifiers {
        if verifier.id.trim().is_empty()
            || verifier.description.trim().is_empty()
            || !verifiers.insert(verifier.id.clone())
        {
            return Err(runtime_error(
                "invalid_skill_verifier_contract",
                "Skill verifiers require unique non-empty ids and descriptions",
            ));
        }
    }
    let mut resume_points = BTreeSet::new();
    if contract
        .resume_points
        .iter()
        .any(|point| point.trim().is_empty() || !resume_points.insert(point.clone()))
    {
        return Err(runtime_error(
            "invalid_skill_resume_contract",
            "Skill resume points must be unique and non-empty",
        ));
    }
    Ok(())
}

pub(crate) fn validate_skill_completion(run: &SkillRun) -> Result<(), SkillRuntimeError> {
    let contract = &run.descriptor.execution_contract;
    let missing_progress = contract
        .progress_steps
        .iter()
        .any(|step| step.required && !run.completed_progress_steps.contains(&step.id));
    let missing_artifacts = contract
        .artifacts
        .iter()
        .any(|artifact| artifact.required && !run.artifacts.contains_key(&artifact.id));
    let failed_or_missing_verification = contract.verifiers.iter().any(|verifier| {
        verifier.required
            && !run
                .verifications
                .get(&verifier.id)
                .is_some_and(|result| result.passed && result.evidence.method() == verifier.method && result.evidence.validate().is_ok())
    });
    if missing_progress || missing_artifacts || failed_or_missing_verification {
        return Err(runtime_error(
            "skill_completion_contract_unsatisfied",
            "Skill cannot complete until required progress, artifacts, and verifiers pass",
        ));
    }
    Ok(())
}

fn default_skill_execution_contract_schema_version() -> u32 {
    SKILL_EXECUTION_CONTRACT_SCHEMA_VERSION
}

fn is_lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn active_run_mut<'a>(
    runs: &'a mut BTreeMap<String, SkillRun>,
    run_id: &str,
) -> Result<&'a mut SkillRun, SkillRuntimeError> {
    let run = run_mut(runs, run_id)?;
    if run.status != SkillRunStatus::Running {
        return Err(runtime_error("run_not_running", "Skill run is not running"));
    }
    Ok(run)
}

fn run_mut<'a>(
    runs: &'a mut BTreeMap<String, SkillRun>,
    run_id: &str,
) -> Result<&'a mut SkillRun, SkillRuntimeError> {
    runs.get_mut(run_id)
        .ok_or_else(|| runtime_error("unknown_skill_run", "Skill run does not exist"))
}

fn exhaust(run: &mut SkillRun, dimension: SkillBudgetDimension) -> Vec<SkillRuntimeEvent> {
    run.status = SkillRunStatus::Failed;
    run.failure = Some(format!("budget_exceeded:{dimension:?}"));
    vec![SkillRuntimeEvent::BudgetExceeded {
        run_id: run.run_id.clone(),
        dimension,
    }]
}

fn runtime_error(code: &str, message: &str) -> SkillRuntimeError {
    SkillRuntimeError {
        code: code.to_string(),
        message: message.to_string(),
    }
}
