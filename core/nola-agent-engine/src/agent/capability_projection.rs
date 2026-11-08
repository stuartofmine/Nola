// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Freezes the exact Host-backed capability surface for one Agent Turn.
// - Boundary: Pure projection only; policy evaluation and Host execution remain separate.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

use super::host::{
    CapabilityAvailability, HostCapabilityCatalog, HostCapabilityDescriptor, HostCapabilityRoute,
    HostLease,
};

pub const TURN_CAPABILITY_PROJECTION_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentTurnCapabilityProjection {
    pub schema_version: u32,
    pub host_id: String,
    pub host_catalog_revision: u64,
    pub host_availability_epoch: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_lease: Option<HostLease>,
    #[serde(default)]
    pub projection_digest: String,
    #[serde(default)]
    pub model_schema_digest: String,
    /// Full available registry frozen for dispatch, validation, tool search,
    /// and explicit Skill activation. Only `enabled` is directly model-visible.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub available_capabilities: BTreeMap<String, HostCapabilityDescriptor>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enabled: Vec<AgentTurnCapability>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disabled: Vec<AgentTurnDisabledCapability>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentTurnCapability {
    pub capability_id: String,
    pub model_definition: Value,
    pub descriptor: HostCapabilityDescriptor,
    pub model_schema_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTurnDisabledCapability {
    pub capability_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentModelToolProjection {
    pub revision: u64,
    pub turn_projection_digest: String,
    pub schema_digest: String,
    /// Conservative token estimate compiled with the canonical schema digest.
    /// Model planning must not serialize the complete schema again merely to
    /// refresh context accounting.
    #[serde(default)]
    pub schema_token_estimate: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capability_ids: Vec<String>,
}

/// One compiled model surface. The exact ordered schemas and their witness
/// must travel together; hashing a sorted copy while sending another ordering
/// makes the cache identity attest a prefix that was never sent.
pub struct CompiledModelToolProjection {
    pub tools: Vec<Value>,
    pub projection: AgentModelToolProjection,
}

impl AgentTurnCapabilityProjection {
    pub fn resolve(configured_tools: &[Value], catalog: &HostCapabilityCatalog) -> Self {
        let available_capabilities = catalog
            .capabilities
            .iter()
            .filter(|(_, descriptor)| descriptor.availability == CapabilityAvailability::Available)
            .map(|(id, descriptor)| (id.clone(), descriptor.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut enabled = BTreeMap::<String, AgentTurnCapability>::new();
        let mut disabled = BTreeMap::<String, AgentTurnDisabledCapability>::new();
        let mut seen = BTreeSet::new();

        for configured in configured_tools {
            let Some(capability_id) = configured_tool_id(configured) else {
                let synthetic_id = format!("invalid-configured-tool-{}", disabled.len() + 1);
                disabled.insert(
                    synthetic_id.clone(),
                    AgentTurnDisabledCapability {
                        capability_id: synthetic_id,
                        reason: "invalid_configured_tool_definition".to_string(),
                    },
                );
                continue;
            };
            if !seen.insert(capability_id.clone()) {
                disabled.insert(
                    capability_id.clone(),
                    AgentTurnDisabledCapability {
                        capability_id,
                        reason: "duplicate_configured_capability".to_string(),
                    },
                );
                continue;
            }
            let Some(descriptor) = catalog.capabilities.get(&capability_id) else {
                disabled.insert(
                    capability_id.clone(),
                    AgentTurnDisabledCapability {
                        capability_id,
                        reason: "not_advertised_by_host".to_string(),
                    },
                );
                continue;
            };
            if descriptor.availability != CapabilityAvailability::Available {
                disabled.insert(
                    capability_id.clone(),
                    AgentTurnDisabledCapability {
                        capability_id,
                        reason: "host_capability_unavailable".to_string(),
                    },
                );
                continue;
            }
            if descriptor.route != HostCapabilityRoute::Tool
                || !descriptor.exposure.is_model_visible()
            {
                disabled.insert(
                    capability_id.clone(),
                    AgentTurnDisabledCapability {
                        capability_id,
                        reason: "not_directly_model_visible".to_string(),
                    },
                );
                continue;
            }

            let model_definition = descriptor.model_tool_definition(configured);
            let model_schema_digest = digest_json(&model_definition);
            enabled.insert(
                capability_id.clone(),
                AgentTurnCapability {
                    capability_id,
                    model_definition,
                    descriptor: descriptor.clone(),
                    model_schema_digest,
                },
            );
        }

        let enabled = enabled.into_values().collect::<Vec<_>>();
        let disabled = disabled.into_values().collect::<Vec<_>>();
        let model_schema_digest = digest_json(
            &enabled
                .iter()
                .map(|entry| &entry.model_definition)
                .collect::<Vec<_>>(),
        );
        let mut projection = Self {
            schema_version: TURN_CAPABILITY_PROJECTION_SCHEMA_VERSION,
            host_id: catalog.host_id.0.clone(),
            host_catalog_revision: catalog.revision,
            host_availability_epoch: catalog.availability_epoch,
            host_lease: catalog.lease.clone(),
            projection_digest: String::new(),
            model_schema_digest,
            available_capabilities,
            enabled,
            disabled,
        };
        projection.projection_digest = digest_json(&projection_without_digest(&projection));
        projection
    }

    pub fn enabled_ids(&self) -> Vec<String> {
        self.enabled
            .iter()
            .map(|entry| entry.capability_id.clone())
            .collect()
    }

    pub fn model_definitions(&self) -> Vec<Value> {
        self.enabled
            .iter()
            .map(|entry| entry.model_definition.clone())
            .collect()
    }

    pub fn model_definitions_for_ids(&self, capability_ids: &[String]) -> Vec<Value> {
        if capability_ids.is_empty() {
            return self.model_definitions();
        }
        let allowed = capability_ids
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        self.enabled
            .iter()
            .filter(|entry| allowed.contains(entry.capability_id.as_str()))
            .map(|entry| entry.model_definition.clone())
            .collect()
    }

    pub fn descriptor(&self, capability_id: &str) -> Option<&HostCapabilityDescriptor> {
        self.available_capabilities.get(capability_id)
    }

    /// Removes discovery-only capability payloads from a completed Turn.
    ///
    /// The projection and schema digests continue to identify the exact
    /// runtime catalog that the model saw, while every model-visible tool
    /// schema is removed. Concrete calls and results live in the Turn and
    /// transcript rather than in this discovery projection. Internal protocol
    /// capabilities remain available so a restored session can start its next
    /// Turn. Active Turns must keep the original projection because pending
    /// model/tool responses are validated against it.
    pub(crate) fn redact_tool_catalog_for_history(&mut self) {
        self.available_capabilities
            .retain(|_, descriptor| descriptor.route != HostCapabilityRoute::Tool);
        self.enabled.clear();
        self.disabled.clear();
    }

    /// Installs the exact subset selected by a typed Host activation response.
    /// Every selected descriptor must be present in this response. Validation
    /// is atomic: neither missing entries nor invalid later entries may reuse
    /// or partially replace the frozen catalog.
    pub(crate) fn install_activated_descriptors(
        &mut self,
        capability_ids: &[String],
        descriptors: Vec<Value>,
    ) -> Result<(), String> {
        let requested = capability_ids.iter().cloned().collect::<BTreeSet<_>>();
        if requested.len() != capability_ids.len() {
            return Err("activated capability IDs must be unique".into());
        }
        let mut received = BTreeMap::new();
        for value in descriptors {
            let descriptor: HostCapabilityDescriptor = serde_json::from_value(value)
                .map_err(|error| format!("invalid activated capability descriptor: {error}"))?;
            if !requested.contains(&descriptor.id) || received.contains_key(&descriptor.id) {
                return Err("activated capability descriptors do not match requested IDs".into());
            }
            if descriptor.route != HostCapabilityRoute::Tool
                || descriptor.availability != CapabilityAvailability::Available
            {
                return Err(format!(
                    "activated capability {} is not an available tool",
                    descriptor.id
                ));
            }
            received.insert(descriptor.id.clone(), descriptor);
        }
        if let Some(missing) = requested.iter().find(|id| !received.contains_key(*id)) {
            return Err(format!(
                "activation did not provide descriptor for capability {missing}"
            ));
        }
        self.available_capabilities.extend(received);
        Ok(())
    }

    pub fn frozen_host_catalog(&self) -> HostCapabilityCatalog {
        HostCapabilityCatalog {
            host_id: super::host::HostId(self.host_id.clone()),
            revision: self.host_catalog_revision,
            availability_epoch: self.host_availability_epoch,
            lease: self.host_lease.clone(),
            capabilities: self.available_capabilities.clone(),
        }
    }

    /// A correlated Host refresh advances the active Turn's route snapshot as
    /// well as the session catalog. Keep its selected tools; do not broaden the
    /// initial model surface simply because new descriptors were discovered.
    pub(crate) fn refresh_host_catalog(&mut self, catalog: &HostCapabilityCatalog) {
        *self = Self::resolve(&self.model_definitions(), catalog);
    }
}

impl AgentModelToolProjection {
    pub fn compile(
        revision: u64,
        turn_projection_digest: impl Into<String>,
        model_definitions: Vec<Value>,
    ) -> Result<CompiledModelToolProjection, String> {
        let mut ordered = BTreeMap::new();
        for definition in model_definitions {
            let id = exact_model_tool_id(&definition)
                .ok_or("model tool projection contains an invalid capability identity")?
                .to_string();
            if ordered.insert(id, canonical_json(&definition)).is_some() {
                return Err("model tool projection contains duplicate capability identities".into());
            }
        }
        let tools = ordered.into_values().collect::<Vec<_>>();
        let projection = Self::capture_ordered(revision, turn_projection_digest.into(), &tools);
        Ok(CompiledModelToolProjection { tools, projection })
    }

    fn capture_ordered(revision: u64, turn_projection_digest: String, definitions: &[Value]) -> Self {
        let schema_token_estimate = serde_json::to_vec(definitions)
            .expect("JSON model tool schemas serialize")
            .len().saturating_add(3) / 4;
        Self {
            revision,
            turn_projection_digest,
            schema_digest: digest_json(&definitions),
            schema_token_estimate,
            capability_ids: definitions.iter().filter_map(configured_tool_id).collect(),
        }
    }

    /// Binds the stable prompt prefix to the exact model-visible tool surface
    /// for this sampling step. Both inputs are Core-owned SHA-256 identities;
    /// Hosts must forward this value rather than hashing prompt or tool JSON.
    pub fn model_prefix_digest(&self, stable_prompt_digest: &str) -> String {
        let stable_prompt_digest = normalized_sha256(stable_prompt_digest);
        let tool_projection_digest = normalized_sha256(&self.schema_digest);
        let (Some(stable_prompt_digest), Some(tool_projection_digest)) =
            (stable_prompt_digest, tool_projection_digest)
        else {
            return String::new();
        };
        let material =
            format!("nola:model-prefix:v1\0{stable_prompt_digest}\0{tool_projection_digest}");
        format!("{:x}", Sha256::digest(material.as_bytes()))
    }

    /// Identifies the complete non-sensitive world state used by one model
    /// sampling step. This is broader than the provider cache identity: it
    /// includes the frozen Turn context, the exact Step tool projection, and
    /// Core's model invocation semantics.
    pub fn request_snapshot_digest(
        &self,
        turn_context: &super::protocol::AgentTurnContextSnapshot,
        invocation_policy_digest: &str,
    ) -> String {
        let Some(invocation_policy_digest) = normalized_sha256(invocation_policy_digest) else {
            return String::new();
        };
        let turn_context_digest = digest_json(turn_context);
        let tool_projection_digest = digest_json(self);
        let material = format!(
            "nola:request-snapshot:v1\0{turn_context_digest}\0{tool_projection_digest}\0{invocation_policy_digest}"
        );
        format!("{:x}", Sha256::digest(material.as_bytes()))
    }

    /// Binds cache identity to prompt, exact tool schema, and the Core-owned
    /// model invocation semantics. A reasoning/cache/capability policy change
    /// must invalidate a provider prefix even when prompt bytes are unchanged.
    pub fn model_prefix_digest_with_invocation_policy(
        &self,
        stable_prompt_digest: &str,
        invocation_policy_digest: &str,
    ) -> String {
        let stable_prompt_digest = normalized_sha256(stable_prompt_digest);
        let tool_projection_digest = normalized_sha256(&self.schema_digest);
        let invocation_policy_digest = normalized_sha256(invocation_policy_digest);
        let (
            Some(stable_prompt_digest),
            Some(tool_projection_digest),
            Some(invocation_policy_digest),
        ) = (
            stable_prompt_digest,
            tool_projection_digest,
            invocation_policy_digest,
        )
        else {
            return String::new();
        };
        let material = format!(
            "nola:model-prefix:v2\0{stable_prompt_digest}\0{tool_projection_digest}\0{invocation_policy_digest}"
        );
        format!("{:x}", Sha256::digest(material.as_bytes()))
    }

    pub fn delegated_context_identity_digest(
        model_prefix_digest: &str,
        model_id: Option<&str>,
        backend_id: &str,
        workspace: Option<&crate::agent::protocol::AgentWorkspaceIdentity>,
    ) -> String {
        let Some(model_prefix_digest) = normalized_sha256(model_prefix_digest) else {
            return String::new();
        };
        let material = format!(
            "nola:delegated-context:v2\0{model_prefix_digest}\0{}\0{}\0{}",
            model_id.unwrap_or_default(),
            backend_id,
            serde_json::json!(workspace.map(|value| (&value.workspace_id, &value.root_digest)))
        );
        format!("{:x}", Sha256::digest(material.as_bytes()))
    }

    /// Verifies a Host-visible model tool array using the exact canonical
    /// projection algorithm owned by Core. The array order is part of the
    /// physical prefix: additions, removals, reorderings and schema changes fail.
    pub fn matches_model_definitions(&self, model_definitions: &[Value]) -> bool {
        let mut seen = BTreeSet::new();
        if model_definitions.iter().any(|definition| {
            exact_model_tool_id(definition).is_none_or(|id| !seen.insert(id))
        }) {
            return false;
        }
        let observed = Self::capture_ordered(
            self.revision,
            self.turn_projection_digest.clone(),
            model_definitions,
        );
        self.capability_ids == observed.capability_ids
            && self.schema_digest == observed.schema_digest
            && self.schema_token_estimate == observed.schema_token_estimate
    }
}

fn exact_model_tool_id(value: &Value) -> Option<&str> {
    let nested = value.pointer("/function/name");
    let top = value.get("name");
    if nested.is_some() && top.is_some() && nested != top {
        return None;
    }
    nested.or(top)?.as_str().filter(|id| !id.is_empty() && id.trim() == *id)
}

fn normalized_sha256(value: &str) -> Option<String> {
    let value = value.trim().to_ascii_lowercase();
    (value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())).then_some(value)
}

fn configured_tool_id(value: &Value) -> Option<String> {
    value
        .pointer("/function/name")
        .or_else(|| value.get("name"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn projection_without_digest(projection: &AgentTurnCapabilityProjection) -> Value {
    let mut value = serde_json::to_value(projection).unwrap_or(Value::Null);
    if let Some(object) = value.as_object_mut() {
        object.remove("projection_digest");
    }
    value
}

fn canonical_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| (key.clone(), canonical_json(value)))
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.iter().map(canonical_json).collect()),
        _ => value.clone(),
    }
}

fn digest_json<T: Serialize>(value: &T) -> String {
    let value = serde_json::to_value(value).unwrap_or(Value::Null);
    let bytes = serde_json::to_vec(&canonical_json(&value)).unwrap_or_default();
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::host::{
        HostCancellationMode, HostCapabilityExposure, HostCapabilityParallelism,
        HostCapabilityRoute, HostOutputPolicy, HostReplaySafety,
    };
    use serde_json::json;

    #[test]
    fn delegated_resume_identity_changes_with_workspace_but_not_its_label() {
        use crate::agent::protocol::AgentWorkspaceIdentity;
        let mut workspace=AgentWorkspaceIdentity { workspace_id:"same-project".into(), root_digest:"b".repeat(64), summary:None };
        let digest=|workspace:Option<&AgentWorkspaceIdentity>|AgentModelToolProjection::delegated_context_identity_digest(
            &"a".repeat(64),Some("synthetic-model"),"codex",workspace);
        let original=digest(Some(&workspace));
        workspace.summary=Some("Changed display title".into());
        assert_eq!(original,digest(Some(&workspace)));
        workspace.root_digest="c".repeat(64);
        assert_ne!(original,digest(Some(&workspace)),"another root or frozen binding cannot resume the old harness context");
        assert_ne!(original,digest(None));
    }

    fn descriptor(id: &str, availability: CapabilityAvailability) -> HostCapabilityDescriptor {
        HostCapabilityDescriptor {
            id: id.to_string(),
            title: id.to_string(),
            route: HostCapabilityRoute::Tool,
            availability,
            exposure: HostCapabilityExposure::default(),
            permission_domain: None,
            input_schema: json!({"type":"object","properties":{"path":{"type":"string"}}}),
            input_normalization: Default::default(),
            output_policy: HostOutputPolicy {
                preview_char_limit: 1_000,
                persist_full_output: true,
                allow_artifacts: true,
            },
            supports_streaming: false,
            supports_argument_streaming: false,
            parallelism: HostCapabilityParallelism::default(),
            cancellation: HostCancellationMode::default(),
            replay_safety: HostReplaySafety::default(),
        }
    }

    #[test]
    fn freezes_only_available_advertised_tools_in_canonical_order() {
        let mut catalog = HostCapabilityCatalog::empty("host-1");
        catalog.revision = 4;
        catalog.availability_epoch = 9;
        catalog.capabilities.insert(
            "read_file".to_string(),
            descriptor("read_file", CapabilityAvailability::Available),
        );
        catalog.capabilities.insert(
            "write_file".to_string(),
            descriptor("write_file", CapabilityAvailability::Unavailable),
        );
        let projection = AgentTurnCapabilityProjection::resolve(
            &[
                json!({"type":"function","function":{"name":"write_file"}}),
                json!({"type":"function","function":{"name":"read_file"}}),
                json!({"type":"function","function":{"name":"missing"}}),
            ],
            &catalog,
        );

        assert_eq!(projection.enabled_ids(), vec!["read_file"]);
        assert_eq!(
            projection
                .disabled
                .iter()
                .map(|entry| (entry.capability_id.as_str(), entry.reason.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("missing", "not_advertised_by_host"),
                ("write_file", "host_capability_unavailable"),
            ]
        );
        assert!(!projection.projection_digest.is_empty());
        assert!(!projection.model_schema_digest.is_empty());
    }

    #[test]
    fn digest_is_independent_of_configured_tool_order() {
        let mut catalog = HostCapabilityCatalog::empty("host-1");
        catalog.capabilities.insert(
            "alpha".to_string(),
            descriptor("alpha", CapabilityAvailability::Available),
        );
        catalog.capabilities.insert(
            "beta".to_string(),
            descriptor("beta", CapabilityAvailability::Available),
        );
        let alpha = json!({"type":"function","function":{"name":"alpha"}});
        let beta = json!({"type":"function","function":{"name":"beta"}});
        let first =
            AgentTurnCapabilityProjection::resolve(&[alpha.clone(), beta.clone()], &catalog);
        let second = AgentTurnCapabilityProjection::resolve(&[beta, alpha], &catalog);
        assert_eq!(first.projection_digest, second.projection_digest);
        assert_eq!(first.model_schema_digest, second.model_schema_digest);
    }

    #[test]
    fn completed_history_discards_all_tool_schemas_and_keeps_original_digests() {
        let mut catalog = HostCapabilityCatalog::empty("host-1");
        for id in ["read_file", "write_file", "web_search"] {
            catalog.capabilities.insert(
                id.to_string(),
                descriptor(id, CapabilityAvailability::Available),
            );
        }
        let mut projection = AgentTurnCapabilityProjection::resolve(
            &[
                json!({"type":"function","function":{"name":"read_file"}}),
                json!({"type":"function","function":{"name":"write_file"}}),
            ],
            &catalog,
        );
        let projection_digest = projection.projection_digest.clone();
        let schema_digest = projection.model_schema_digest.clone();

        projection.redact_tool_catalog_for_history();

        assert!(projection.available_capabilities.is_empty());
        assert!(projection.enabled_ids().is_empty());
        assert!(projection.disabled.is_empty());
        assert_eq!(projection.projection_digest, projection_digest);
        assert_eq!(projection.model_schema_digest, schema_digest);
    }

    #[test]
    fn typed_activation_installs_only_the_selected_deferred_descriptor() {
        let catalog = HostCapabilityCatalog::empty("host-1");
        let mut projection = AgentTurnCapabilityProjection::resolve(&[], &catalog);
        let selected = descriptor("web_search", CapabilityAvailability::Available);

        projection
            .install_activated_descriptors(
                &["web_search".to_string()],
                vec![serde_json::to_value(&selected).unwrap()],
            )
            .unwrap();

        assert_eq!(projection.available_capabilities.len(), 1);
        assert_eq!(projection.descriptor("web_search"), Some(&selected));
        assert!(projection.descriptor("read_file").is_none());
    }

    #[test]
    fn activation_cannot_reuse_a_previously_advertised_descriptor() {
        let mut catalog = HostCapabilityCatalog::empty("host-1");
        catalog.capabilities.insert(
            "read_file".into(),
            descriptor("read_file", CapabilityAvailability::Available),
        );
        let mut projection = AgentTurnCapabilityProjection::resolve(&[], &catalog);
        let before = projection.clone();
        let error = projection
            .install_activated_descriptors(&["read_file".into()], vec![])
            .unwrap_err();
        assert!(error.contains("did not provide descriptor"));
        assert_eq!(projection, before);
    }

    #[test]
    fn activation_validates_the_whole_batch_before_mutating_the_catalog() {
        let alpha =
            serde_json::to_value(descriptor("alpha", CapabilityAvailability::Available)).unwrap();
        let beta =
            serde_json::to_value(descriptor("beta", CapabilityAvailability::Available)).unwrap();
        let mut unavailable = beta.clone();
        unavailable["route"] = serde_json::json!("model");
        for (ids, descriptors) in [
            (vec!["alpha", "beta"], vec![alpha.clone()]),
            (
                vec!["alpha", "beta"],
                vec![alpha.clone(), serde_json::json!({})],
            ),
            (vec!["alpha", "beta"], vec![alpha.clone(), unavailable]),
            (vec!["alpha"], vec![alpha.clone(), beta.clone()]),
            (vec!["alpha"], vec![alpha.clone(), alpha.clone()]),
            (vec!["alpha", "alpha"], vec![alpha.clone()]),
        ] {
            let catalog = HostCapabilityCatalog::empty("host-1");
            let mut projection = AgentTurnCapabilityProjection::resolve(&[], &catalog);
            let before = projection.clone();
            assert!(projection
                .install_activated_descriptors(
                    &ids.into_iter().map(str::to_owned).collect::<Vec<_>>(),
                    descriptors,
                )
                .is_err());
            assert_eq!(projection, before);
        }
        let mut projection =
            AgentTurnCapabilityProjection::resolve(&[], &HostCapabilityCatalog::empty("host-1"));
        projection
            .install_activated_descriptors(&["alpha".into(), "beta".into()], vec![beta, alpha])
            .unwrap();
        assert_eq!(projection.available_capabilities.len(), 2);
    }

    #[test]
    fn host_refresh_advances_turn_fence_without_implicitly_selecting_new_tools() {
        let mut catalog = HostCapabilityCatalog::empty("host-1");
        catalog.revision = 1;
        catalog.capabilities.insert("alpha".into(), descriptor("alpha", CapabilityAvailability::Available));
        let mut projection = AgentTurnCapabilityProjection::resolve(
            &[json!({"type":"function","function":{"name":"alpha"}})], &catalog);
        catalog.revision = 2;
        catalog.capabilities.insert("beta".into(), descriptor("beta", CapabilityAvailability::Available));
        projection.refresh_host_catalog(&catalog);
        assert_eq!(projection.frozen_host_catalog().revision, 2);
        assert_eq!(projection.enabled_ids(), vec!["alpha"]);
        assert!(projection.descriptor("beta").is_some());
    }

    #[test]
    fn model_step_compilation_canonicalizes_the_actual_schema_array_and_witness_together() {
        let alpha = json!({"type":"function","function":{"name":"alpha"}});
        let beta = json!({"type":"function","function":{"name":"beta"}});
        let first =
            AgentModelToolProjection::compile(2, "turn-digest", vec![alpha.clone(), beta.clone()]).unwrap();
        let second = AgentModelToolProjection::compile(2, "turn-digest", vec![beta, alpha]).unwrap();
        assert_eq!(first.tools, second.tools);
        assert_eq!(first.projection, second.projection);
        assert_eq!(first.projection.capability_ids, vec!["alpha", "beta"]);
        assert_eq!(first.projection.revision, 2);
        assert!(first.projection.matches_model_definitions(&first.tools));
        assert_eq!(first.projection.schema_digest, digest_json(&first.tools));
    }

    #[test]
    fn compilation_rejects_ambiguous_tool_identity_instead_of_attesting_a_partial_catalog() {
        let valid = json!({"type":"function","function":{"name":"read_file"}});
        for invalid in [
            json!({"type":"function","function":{}}),
            json!({"type":"function","function":{"name":""}}),
            json!({"type":"function","function":{"name":" read_file"}}),
            json!({"name":"other", "function":{"name":"read_file"}}),
            valid.clone(),
        ] {
            assert!(AgentModelToolProjection::compile(1, "turn", vec![valid.clone(), invalid]).is_err());
        }
        let empty = AgentModelToolProjection::compile(1, "turn", vec![]).unwrap();
        assert!(empty.projection.matches_model_definitions(&empty.tools));
    }

    #[test]
    fn restored_projection_rejects_changed_schema_cost_and_duplicate_identity() {
        let schema = json!({"type":"function","function":{"name":"read_file"}});
        let compiled = AgentModelToolProjection::compile(1, "turn", vec![schema.clone()]).unwrap();
        let encoded = serde_json::to_value(&compiled.projection).unwrap();
        let mut restored: AgentModelToolProjection = serde_json::from_value(encoded).unwrap();
        assert!(restored.matches_model_definitions(&compiled.tools));
        assert!(!restored.matches_model_definitions(&[schema.clone(), schema]));
        restored.schema_token_estimate += 1;
        assert!(!restored.matches_model_definitions(&compiled.tools));
    }

    #[test]
    fn model_prefix_digest_binds_stable_prompt_to_exact_step_tools() {
        let stable_prompt = "11".repeat(32);
        let other_stable_prompt = "22".repeat(32);
        let mut one_byte_changed_prompt = stable_prompt.clone();
        one_byte_changed_prompt.replace_range(62..64, "12");
        let alpha = json!({"type":"function","function":{"name":"alpha"}});
        let beta = json!({"type":"function","function":{"name":"beta"}});
        let alpha_projection = AgentModelToolProjection::compile(1, "turn", vec![alpha.clone()]).unwrap().projection;
        let reordered_projection =
            AgentModelToolProjection::compile(2, "other-turn", vec![beta.clone(), alpha.clone()]).unwrap().projection;
        let same_tools_reordered =
            AgentModelToolProjection::compile(3, "another-turn", vec![alpha.clone(), beta]).unwrap().projection;

        let first = alpha_projection.model_prefix_digest(&stable_prompt);
        assert_eq!(first.len(), 64);
        assert_ne!(
            first,
            alpha_projection.model_prefix_digest(&other_stable_prompt)
        );
        assert_ne!(
            first,
            alpha_projection.model_prefix_digest(&one_byte_changed_prompt),
            "a one-byte stable-policy change must invalidate the model prefix identity"
        );
        assert_ne!(
            first,
            reordered_projection.model_prefix_digest(&stable_prompt)
        );
        assert_eq!(
            reordered_projection.model_prefix_digest(&stable_prompt),
            same_tools_reordered.model_prefix_digest(&stable_prompt),
            "conversation forks, compaction, revision, and input order do not alter an unchanged model-visible prefix"
        );
        assert!(alpha_projection.model_prefix_digest("").is_empty());
        assert!(alpha_projection
            .model_prefix_digest("not-a-digest")
            .is_empty());
    }

    #[test]
    fn model_step_projection_verification_rejects_changed_physical_array_order() {
        let alpha = json!({
            "type":"function",
            "function":{"name":"alpha","parameters":{"type":"object"}}
        });
        let beta = json!({
            "type":"function",
            "function":{"name":"beta","parameters":{"type":"object"}}
        });
        let projection =
            AgentModelToolProjection::compile(1, "turn", vec![alpha.clone(), beta.clone()]).unwrap().projection;

        assert!(projection.matches_model_definitions(&[alpha.clone(), beta.clone()]));
        assert!(!projection.matches_model_definitions(&[beta.clone(), alpha.clone()]));
        assert!(!projection.matches_model_definitions(std::slice::from_ref(&alpha)));
        assert!(!projection.matches_model_definitions(&[
            alpha,
            beta,
            json!({"type":"function","function":{"name":"gamma"}}),
        ]));
        assert!(!projection.matches_model_definitions(&[
            json!({
                "type":"function",
                "function":{"name":"alpha","parameters":{"type":"object","required":["path"]}}
            }),
            json!({
                "type":"function",
                "function":{"name":"beta","parameters":{"type":"object"}}
            }),
        ]));
    }
}
