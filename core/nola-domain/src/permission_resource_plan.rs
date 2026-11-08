// SPDX-License-Identifier: Apache-2.0
//! Extract resource demands from normalized tool input. These are unresolved
//! locators, not filesystem identities, permission subjects or authorization.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use crate::assistant_journal::AssistantDigest;
use crate::permission_scope::{ConcretePermissionSubject, PermissionResource};

/// Declared by the trusted capability catalog, never by model arguments.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionOperationRisk {
    OrdinaryRead, NewFile, Overwrite, Delete, SensitiveRead, ExternalSend, NewConnection,
    #[default]
    Unknown,
}

impl PermissionOperationRisk {
    pub fn requires_confirmation(self) -> bool {
        !matches!(self, Self::OrdinaryRead | Self::NewFile)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionOperationContract {
    pub risk: PermissionOperationRisk,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_plan: Option<PermissionResourcePlan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview_when_true: Option<String>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub risk_when_nonempty: std::collections::BTreeMap<String, PermissionOperationRisk>,
}

impl PermissionOperationContract {
    pub fn for_input(&self, input: &Value) -> Self {
        let mut contract = self.clone();
        if self.preview_when_true.as_ref().is_some_and(|key|input.get(key).and_then(Value::as_bool) == Some(true)) {
            contract.risk = PermissionOperationRisk::OrdinaryRead;
        }
        for (pointer, risk) in &self.risk_when_nonempty {
            if input.pointer(pointer).is_some_and(|value| match value {
                Value::Array(values) => !values.is_empty(), Value::String(value) => !value.is_empty(),
                Value::Object(value) => !value.is_empty(), Value::Null => false, _ => true,
            }) { contract.risk = *risk; }
        }
        contract
    }

    pub fn resource_plan_for(&self, input: &Value) -> Option<PermissionResourcePlan> {
        let mut plan = self.resource_plan.clone()?;
        if self.preview_when_true.as_ref().is_some_and(|key| input.get(key).and_then(Value::as_bool) == Some(true)) {
            plan.operation = format!("preview:{}", plan.operation);
            for requirement in &mut plan.requirements { requirement.access = "read".into(); }
        }
        Some(plan)
    }
}

/// Stored with the Core resolution effect. `invocation_digest` is established
/// by the owner of normalized input, not accepted from model/approval text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionResolutionRequest {
    pub resolution_id: String,
    pub execution_target: String,
    pub invocation_digest: AssistantDigest,
    pub capability: String,
    pub operation: String,
    pub demands: Vec<UnresolvedResourceDemand>,
}

impl PermissionResolutionRequest {
    pub fn input_digest(input: &Value) -> AssistantDigest {
        use sha2::{Digest, Sha256};
        fn canonical(value: &Value) -> Value {
            match value {
                Value::Object(object) => Value::Object(object.iter().map(|(key,value)|(key.clone(),canonical(value)))
                    .collect::<std::collections::BTreeMap<_,_>>().into_iter().collect()),
                Value::Array(values) => Value::Array(values.iter().map(canonical).collect()),
                value => value.clone(),
            }
        }
        let bytes = serde_json::to_vec(&canonical(input)).expect("JSON input is serializable");
        let mut hash = Sha256::new();
        hash.update(b"nola.permission-input.v1\0");
        hash.update(bytes);
        let digest = format!("{:x}", hash.finalize());
        AssistantDigest::parse_hex(&digest).expect("SHA-256 is a canonical digest")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserPermissionResourceFacts {
    pub ok: bool,
    pub target: String,
    pub owner_id: String,
    pub connection_state: String,
    pub connection_generation: Option<String>,
    pub page_id: Option<String>,
    pub page_origin: Option<String>,
    pub requested_origin: Option<String>,
    /// Unknown is not evidence of a public or unauthenticated page.
    pub authentication_state: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedDemandBinding {
    pub demand: UnresolvedResourceDemand,
    pub resource: PermissionResource,
    /// Reference to Host-owned resolution evidence, not a portable file handle.
    pub evidence_id: String,
    /// Actual target state established by the Host's read-only resolver.
    pub observation: PermissionOperationRisk,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser: Option<BrowserPermissionResourceFacts>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derived: Option<HostDerivedResourceCollection>,
}

pub const MAX_DERIVED_RESOURCE_MEMBERS: usize = 500;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostDerivedResourceMember {
    pub resource: PermissionResource,
    pub before_version: String,
    pub planned_after_version: String,
    pub observation: PermissionOperationRisk,
    pub evidence_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostDerivedResourceCollection {
    pub collection_id: String,
    pub discovery_complete: bool,
    pub members: Vec<HostDerivedResourceMember>,
    pub plan_digest: AssistantDigest,
}

impl HostDerivedResourceCollection {
    pub fn digest(request: &PermissionResolutionRequest, demand: &UnresolvedResourceDemand,
        members: &[HostDerivedResourceMember]) -> AssistantDigest {
        // Canonical object ordering is shared with normalized input digests;
        // the domain tag prevents confusing this plan with a tool invocation.
        PermissionResolutionRequest::input_digest(&serde_json::json!({
            "domain":"nola.host-derived-resource-collection.v1", "resolution_id":request.resolution_id,
            "execution_target":request.execution_target,"invocation_digest":request.invocation_digest,
            "capability":request.capability,"operation":request.operation,"demand":demand,"members":members,
        }))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionResolutionFacts {
    pub resolution_id: String,
    pub execution_target: String,
    pub invocation_digest: AssistantDigest,
    pub bindings: Vec<ResolvedDemandBinding>,
}

impl PermissionResolutionFacts {
    /// Stable grant scope and fresh Host observations are separate from the
    /// one-shot execution evidence. A change of observed process risk must not
    /// silently inherit an earlier conversation grant.
    pub fn process_approval_risks(&self) -> Vec<serde_json::Value> {
        let mut risks = self.bindings.iter()
            .filter(|binding| matches!(binding.resource, PermissionResource::ProcessInvocation { .. }))
            .map(|binding| serde_json::json!({"requirement":binding.demand.requirement_id,
                "risk":binding.observation,
                "sensitive_evidence": if binding.observation == PermissionOperationRisk::SensitiveRead {
                    Some(&binding.evidence_id)
                } else { None }})).collect::<Vec<_>>();
        risks.sort_by_key(serde_json::Value::to_string);
        risks
    }

    /// Check completeness and correlation, not Host authenticity or freshness.
    /// The Core response reducer must verify original effect delivery authority;
    /// the executing Host must separately revalidate each evidence reference.
    pub fn subject_for(&self, request: &PermissionResolutionRequest) -> Result<ConcretePermissionSubject, ResourcePlanError> {
        if !valid_identity(&request.resolution_id) || !valid_identity(&request.execution_target)
            || !valid_identity(&request.capability) || !valid_identity(&request.operation)
            || request.demands.is_empty() {
            return Err(ResourcePlanError::InvalidDeclaration);
        }
        if self.resolution_id != request.resolution_id || self.execution_target != request.execution_target
            || self.invocation_digest != request.invocation_digest {
            return Err(ResourcePlanError::ResolutionMismatch);
        }
        let mut keys = BTreeSet::new();
        for demand in &request.demands {
            if !valid_identity(&demand.requirement_id) || !valid_identity(&demand.access)
                || demand.locator.is_empty() || demand.locator.contains('\0') {
                return Err(ResourcePlanError::InvalidDeclaration);
            }
            if !keys.insert((&demand.requirement_id, demand.item_index)) {
                return Err(ResourcePlanError::DuplicateRequirement);
            }
        }
        if self.bindings.len() != request.demands.len() { return Err(ResourcePlanError::IncompleteResolution); }
        let mut resources = Vec::new();
        for demand in &request.demands {
            let matching: Vec<_> = self.bindings.iter().filter(|binding|
                binding.demand.requirement_id == demand.requirement_id && binding.demand.item_index == demand.item_index).collect();
            if matching.len() != 1 { return Err(ResourcePlanError::IncompleteResolution); }
            let binding = matching[0];
            if &binding.demand != demand || !valid_identity(&binding.evidence_id) {
                return Err(ResourcePlanError::ResolutionMismatch);
            }
            let access = match (&demand.locator_kind, &binding.resource) {
                (ResourceLocatorKind::FilePath | ResourceLocatorKind::DerivedFileCollection, PermissionResource::File { access, .. })
                | (ResourceLocatorKind::NetworkUrl, PermissionResource::NetworkOrigin { access, .. })
                | (ResourceLocatorKind::PlatformResource, PermissionResource::Platform { access, .. }) => access,
                (ResourceLocatorKind::ProcessCommand | ResourceLocatorKind::ProcessHandle, PermissionResource::ProcessInvocation { invocation_digest, command, .. })
                    if invocation_digest == &request.invocation_digest && command == &demand.locator => &demand.access,
                _ => return Err(ResourcePlanError::ResolutionMismatch),
            };
            if access != &demand.access || binding.resource.validate().is_err() {
                return Err(ResourcePlanError::ResolutionMismatch);
            }
            if demand.locator_kind == ResourceLocatorKind::DerivedFileCollection {
                let collection = binding.derived.as_ref().ok_or(ResourcePlanError::IncompleteResolution)?;
                if collection.collection_id != demand.requirement_id || !collection.discovery_complete
                    || collection.members.len() > MAX_DERIVED_RESOURCE_MEMBERS
                    || collection.plan_digest != HostDerivedResourceCollection::digest(request, demand, &collection.members)
                    || binding.browser.is_some() {
                    return Err(ResourcePlanError::ResolutionMismatch);
                }
                let PermissionResource::File { filesystem_id, components, .. } = &binding.resource else {
                    return Err(ResourcePlanError::ResolutionMismatch);
                };
                let member_access = match demand.access.as_str() {
                    "derive_move_imports" => "overwrite", "read" => "read",
                    _ => return Err(ResourcePlanError::InvalidDeclaration),
                };
                let mut previous = None;
                for member in &collection.members {
                    let PermissionResource::File { filesystem_id: fs, components: parts, access } = &member.resource else {
                        return Err(ResourcePlanError::ResolutionMismatch);
                    };
                    let valid_version = |version: &str| version.strip_prefix("sha256:")
                        .is_some_and(|digest| AssistantDigest::parse_hex(digest).is_ok());
                    if fs != filesystem_id || !parts.starts_with(components) || parts.len() <= components.len()
                        || access != member_access || member.resource.validate().is_err()
                        || !valid_identity(&member.evidence_id) || !valid_version(&member.before_version)
                        || !valid_version(&member.planned_after_version) || member.before_version == member.planned_after_version
                        || (member_access == "overwrite" && member.observation != PermissionOperationRisk::Overwrite)
                        || (member_access == "read" && !matches!(member.observation,
                            PermissionOperationRisk::OrdinaryRead | PermissionOperationRisk::SensitiveRead))
                        || previous.as_ref().is_some_and(|last: &Vec<String>| last >= parts) {
                        return Err(ResourcePlanError::ResolutionMismatch);
                    }
                    previous = Some(parts.clone());
                    if !resources.contains(&member.resource) { resources.push(member.resource.clone()); }
                }
                // The root locates a collection; it grants no directory write.
                continue;
            } else if binding.derived.is_some() {
                return Err(ResourcePlanError::ResolutionMismatch);
            }
            if let Some(browser) = &binding.browser {
                let PermissionResource::Platform {namespace,..} = &binding.resource else {
                    return Err(ResourcePlanError::ResolutionMismatch);
                };
                if demand.locator_kind != ResourceLocatorKind::PlatformResource
                    || browser.target != demand.locator || namespace != &format!("browser.{}",browser.target)
                    || !browser.ok || !valid_identity(&browser.owner_id) || browser.authentication_state != "unknown"
                    || !matches!(browser.connection_state.as_str(),"existing"|"absent") {
                    return Err(ResourcePlanError::ResolutionMismatch);
                }
            }
            // Preserve every binding as evidence; coverage only needs one copy
            // of an identical resolved resource+access. Never deduplicate demands.
            if !resources.contains(&binding.resource) { resources.push(binding.resource.clone()); }
        }
        let subject = ConcretePermissionSubject {
            execution_target: request.execution_target.clone(), capability: request.capability.clone(),
            operation: request.operation.clone(), input_digest: request.invocation_digest, resources,
        };
        subject.validate().map_err(|_| ResourcePlanError::ResolutionMismatch)?;
        Ok(subject)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceLocatorKind {
    FilePath, NetworkUrl, PlatformResource, ProcessCommand,
    /// Host-owned process/task or configured service identity, never shell text.
    /// Resolution must bind this exact identity and the normalized invocation digest.
    ProcessHandle,
    DerivedFileCollection,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResourceSelector {
    One { pointer: String },
    FirstAvailable { pointers: Vec<String>, default_locator: Option<String> },
    /// Empty item_pointer means each array value itself is a locator.
    Each { array_pointer: String, item_pointer: String },
    /// Omitted/empty arrays create no demands; malformed arrays/items reject.
    OptionalEach { array_pointer: String, item_pointer: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceRequirement {
    pub requirement_id: String,
    pub locator_kind: ResourceLocatorKind,
    pub access: String,
    pub selector: ResourceSelector,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionResourcePlan {
    pub capability: String,
    pub operation: String,
    pub requirements: Vec<ResourceRequirement>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnresolvedResourceDemand {
    pub requirement_id: String,
    /// Preserves the exact input occurrence even if two locators are equal.
    pub item_index: usize,
    pub locator_kind: ResourceLocatorKind,
    pub access: String,
    pub locator: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResourcePlanError {
    InvalidDeclaration,
    DuplicateRequirement,
    InvalidPointer,
    MissingResource { requirement_id: String, item_index: usize },
    InvalidLocator { requirement_id: String, item_index: usize },
    InvalidCollection { requirement_id: String },
    ResolutionMismatch,
    IncompleteResolution,
}
impl std::fmt::Display for ResourcePlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{self:?}") }
}
impl std::error::Error for ResourcePlanError {}

fn valid_identity(value: &str) -> bool {
    !value.trim().is_empty() && !value.chars().any(char::is_control)
}
fn validate_pointer(pointer: &str) -> Result<(), ResourcePlanError> {
    if !pointer.is_empty() && !pointer.starts_with('/') { return Err(ResourcePlanError::InvalidPointer); }
    let mut chars = pointer.chars();
    while let Some(character) = chars.next() {
        if character == '~' && !matches!(chars.next(), Some('0' | '1')) {
            return Err(ResourcePlanError::InvalidPointer);
        }
    }
    Ok(())
}

impl PermissionResourcePlan {
    /// No partial success: every declared requirement and every array element
    /// must be present and valid. Normalize aliases/defaults before this call.
    pub fn extract(&self, normalized_input: &Value) -> Result<Vec<UnresolvedResourceDemand>, ResourcePlanError> {
        if !valid_identity(&self.capability) || !valid_identity(&self.operation) || self.requirements.is_empty() {
            return Err(ResourcePlanError::InvalidDeclaration);
        }
        let mut seen = BTreeSet::new();
        for requirement in &self.requirements {
            if !valid_identity(&requirement.requirement_id) || !valid_identity(&requirement.access) {
                return Err(ResourcePlanError::InvalidDeclaration);
            }
            if !seen.insert(&requirement.requirement_id) { return Err(ResourcePlanError::DuplicateRequirement); }
            match &requirement.selector {
                ResourceSelector::One { pointer } => validate_pointer(pointer)?,
                ResourceSelector::FirstAvailable { pointers, default_locator } => {
                    if pointers.is_empty() || default_locator.as_ref().is_some_and(|value|value.is_empty()||value.contains('\0')) {
                        return Err(ResourcePlanError::InvalidDeclaration);
                    }
                    for pointer in pointers { validate_pointer(pointer)?; }
                }
                ResourceSelector::Each { array_pointer, item_pointer }
                | ResourceSelector::OptionalEach { array_pointer, item_pointer } => {
                    validate_pointer(array_pointer)?; validate_pointer(item_pointer)?;
                }
            }
        }
        let mut demands = Vec::new();
        for requirement in &self.requirements {
            let mut append = |value: Option<&Value>, item_index| -> Result<(), ResourcePlanError> {
                let value = value.ok_or_else(|| ResourcePlanError::MissingResource {
                    requirement_id: requirement.requirement_id.clone(), item_index,
                })?;
                let locator = value.as_str().filter(|value| !value.is_empty() && !value.contains('\0'))
                    .ok_or_else(|| ResourcePlanError::InvalidLocator {
                        requirement_id: requirement.requirement_id.clone(), item_index,
                    })?;
                // Never trim a path or expand ~/cwd/project roots here. Those
                // operations require the actual target Host's resource resolver.
                demands.push(UnresolvedResourceDemand {
                    requirement_id: requirement.requirement_id.clone(), item_index,
                    locator_kind: requirement.locator_kind, access: requirement.access.clone(), locator: locator.into(),
                });
                Ok(())
            };
            match &requirement.selector {
                ResourceSelector::One { pointer } => append(normalized_input.pointer(pointer), 0)?,
                ResourceSelector::FirstAvailable { pointers, default_locator } => {
                    let default = default_locator.as_ref().map(|value|Value::String(value.clone()));
                    append(pointers.iter().find_map(|pointer|normalized_input.pointer(pointer).filter(|value|!value.is_null()))
                        .or(default.as_ref()),0)?;
                }
                ResourceSelector::Each { array_pointer, item_pointer } => {
                    let items = normalized_input.pointer(array_pointer).and_then(Value::as_array)
                        .filter(|items| !items.is_empty()).ok_or_else(|| ResourcePlanError::InvalidCollection {
                            requirement_id: requirement.requirement_id.clone(),
                        })?;
                    for (index, item) in items.iter().enumerate() { append(item.pointer(item_pointer), index)?; }
                }
                ResourceSelector::OptionalEach { array_pointer, item_pointer } => {
                    if let Some(value) = normalized_input.pointer(array_pointer) {
                        let items = value.as_array().ok_or_else(|| ResourcePlanError::InvalidCollection {
                            requirement_id: requirement.requirement_id.clone(),
                        })?;
                        for (index, item) in items.iter().enumerate() { append(item.pointer(item_pointer), index)?; }
                    }
                }
            }
        }
        Ok(demands)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn operation_labels_are_distinct_from_declared_preview_and_supersession_semantics() {
        let contract = PermissionOperationContract {risk:PermissionOperationRisk::NewFile,
            risk_when_nonempty:[("/supersedes".into(),PermissionOperationRisk::Overwrite)].into(),..Default::default()};
        assert_eq!(contract.for_input(&json!({"operation_kind":"inspect"})).risk,PermissionOperationRisk::NewFile);
        assert_eq!(contract.for_input(&json!({"operation_kind":"inspect","supersedes":["existing"]})).risk,
            PermissionOperationRisk::Overwrite);
        let preview = PermissionOperationContract {resource_plan:Some(move_plan()),preview_when_true:Some("dry_run".into()),..Default::default()};
        assert!(preview.resource_plan_for(&json!({"dry_run":true})).unwrap().requirements.iter().all(|requirement|requirement.access == "read"));
        assert_eq!(preview.resource_plan_for(&json!({"dry_run":false})).unwrap(),move_plan());
        assert_eq!(PermissionResolutionRequest::input_digest(&json!({"a":1,"b":{"z":2,"c":3}})),
            PermissionResolutionRequest::input_digest(&json!({"b":{"c":3,"z":2},"a":1})));
    }
    fn requirement(id: &str, access: &str, pointer: &str) -> ResourceRequirement {
        ResourceRequirement { requirement_id: id.into(), access: access.into(), locator_kind: ResourceLocatorKind::FilePath,
            selector: ResourceSelector::One { pointer: pointer.into() } }
    }
    fn move_plan() -> PermissionResourcePlan {
        PermissionResourcePlan { capability: "move_file".into(), operation: "move".into(),
            requirements: vec![requirement("source", "remove", "/source"), requirement("destination", "write", "/destination")] }
    }
    #[test]
    fn process_facts_bind_the_original_command_and_require_host_cwd() {
        let input = json!({"command":"git status","operation_kind":"inspect"});
        let plan = PermissionResourcePlan {capability:"bash".into(),operation:"process".into(),requirements:vec![ResourceRequirement {
            requirement_id:"command".into(),locator_kind:ResourceLocatorKind::ProcessCommand,access:"execute".into(),
            selector:ResourceSelector::One {pointer:"/command".into()},
        }]};
        let request = PermissionResolutionRequest {resolution_id:"process".into(),execution_target:"trusted-host".into(),
            invocation_digest:PermissionResolutionRequest::input_digest(&input),capability:plan.capability.clone(),operation:plan.operation.clone(),
            demands:plan.extract(&input).unwrap()};
        let mut facts = PermissionResolutionFacts {resolution_id:request.resolution_id.clone(),execution_target:request.execution_target.clone(),
            invocation_digest:request.invocation_digest,bindings:vec![ResolvedDemandBinding {
                demand:request.demands[0].clone(),resource:PermissionResource::ProcessInvocation {invocation_digest:request.invocation_digest,
                    command:"git status".into(),cwd:"/workspace".into()},observation:PermissionOperationRisk::Unknown,
                evidence_id:"host-process".into(),browser:None,derived:None,
            }]};
        facts.subject_for(&request).unwrap();
        let mut wire = serde_json::to_value(&facts).unwrap();
        wire["bindings"][0]["resource"].as_object_mut().unwrap().remove("cwd");
        assert!(serde_json::from_value::<PermissionResolutionFacts>(wire).is_err(),"missing Host cwd is not a supported process fact");
        if let PermissionResource::ProcessInvocation {command,..} = &mut facts.bindings[0].resource {
            *command = "rm -rf private".into();
        }
        assert!(facts.subject_for(&request).is_err(),"a valid input digest must not authorize a substituted command description");
    }

    #[test]
    fn process_handle_requires_exact_identity_digest_and_resource_kind() {
        let input = json!({"task_id":"task-1","text":"hello\n"});
        let plan = PermissionResourcePlan { capability:"terminal_input".into(), operation:"terminal.input".into(),
            requirements:vec![ResourceRequirement { requirement_id:"terminal-task".into(),
                locator_kind:ResourceLocatorKind::ProcessHandle,access:"input".into(),
                selector:ResourceSelector::One {pointer:"/task_id".into()} }] };
        let request = PermissionResolutionRequest { resolution_id:"resolution".into(),execution_target:"host".into(),
            invocation_digest:PermissionResolutionRequest::input_digest(&input),capability:plan.capability.clone(),
            operation:plan.operation.clone(),demands:plan.extract(&input).unwrap() };
        let facts = PermissionResolutionFacts { resolution_id:request.resolution_id.clone(),execution_target:request.execution_target.clone(),
            invocation_digest:request.invocation_digest,bindings:vec![ResolvedDemandBinding {
                demand:request.demands[0].clone(),resource:PermissionResource::ProcessInvocation {
                    invocation_digest:request.invocation_digest,command:"task-1".into(),cwd:"/workspace".into()},
                observation:PermissionOperationRisk::Unknown,evidence_id:"host-task".into(),browser:None,derived:None }] };
        facts.subject_for(&request).unwrap();
        let mut substituted = facts.clone();
        if let PermissionResource::ProcessInvocation {command,..} = &mut substituted.bindings[0].resource { *command="task-2".into(); }
        assert!(substituted.subject_for(&request).is_err());
        let mut substituted = facts.clone();
        if let PermissionResource::ProcessInvocation {invocation_digest,..} = &mut substituted.bindings[0].resource {
            *invocation_digest=PermissionResolutionRequest::input_digest(&json!({"text":"different"}));
        }
        assert!(substituted.subject_for(&request).is_err());
        let mut substituted = facts;
        substituted.bindings[0].resource=PermissionResource::Platform {namespace:"terminal".into(),resource_id:"task-1".into(),access:"input".into()};
        assert!(substituted.subject_for(&request).is_err());
    }

    fn resolved_move() -> (PermissionResolutionRequest, PermissionResolutionFacts) {
        let plan = move_plan();
        let request = PermissionResolutionRequest {
            resolution_id: "resolution".into(), execution_target: "host".into(),
            invocation_digest: AssistantDigest::parse_hex(&"a".repeat(64)).unwrap(),
            capability: plan.capability.clone(), operation: plan.operation.clone(),
            demands: plan.extract(&json!({"source":"source","destination":"destination"})).unwrap(),
        };
        let facts = PermissionResolutionFacts { resolution_id: request.resolution_id.clone(),
            execution_target: request.execution_target.clone(), invocation_digest: request.invocation_digest,
            bindings: request.demands.iter().map(|demand| ResolvedDemandBinding {
                demand: demand.clone(), evidence_id: format!("evidence:{}", demand.requirement_id),
                observation: PermissionOperationRisk::Unknown, browser: None, derived: None,
                resource: PermissionResource::File { filesystem_id: "volume".into(),
                    components: vec![demand.locator.clone()], access: demand.access.clone() },
            }).collect() };
        (request, facts)
    }

    fn resolved_collection() -> (PermissionResolutionRequest, PermissionResolutionFacts) {
        let (mut request, mut facts) = resolved_move();
        let demand = UnresolvedResourceDemand { requirement_id:"imports".into(),item_index:0,
            locator_kind:ResourceLocatorKind::DerivedFileCollection,access:"derive_move_imports".into(),locator:"/project".into() };
        request.demands.push(demand.clone());
        let members = vec![HostDerivedResourceMember {resource:PermissionResource::File {
            filesystem_id:"volume".into(),components:vec!["project".into(),"app.ts".into()],access:"overwrite".into()},
            before_version:format!("sha256:{}","a".repeat(64)),planned_after_version:format!("sha256:{}","b".repeat(64)),
            observation:PermissionOperationRisk::Overwrite,evidence_id:"host-observed-importer".into() }];
        let derived = HostDerivedResourceCollection { collection_id:"imports".into(),discovery_complete:true,
            plan_digest:HostDerivedResourceCollection::digest(&request,&demand,&members),members };
        facts.bindings.push(ResolvedDemandBinding {demand,resource:PermissionResource::File {
            filesystem_id:"volume".into(),components:vec!["project".into()],access:"derive_move_imports".into()},
            evidence_id:"host-collection".into(),observation:PermissionOperationRisk::OrdinaryRead,browser:None,derived:Some(derived)});
        (request,facts)
    }

    #[test]
    fn declared_collection_expands_exact_members_not_a_root_write_grant() {
        let (request,facts) = resolved_collection();
        let subject = facts.subject_for(&request).unwrap();
        assert_eq!(subject.resources.len(),3);
        assert!(subject.resources.iter().any(|resource| matches!(resource,PermissionResource::File {components,access,..}
            if components.iter().map(String::as_str).eq(["project","app.ts"]) && access == "overwrite")));
        assert!(!subject.resources.contains(&facts.bindings[2].resource));
        let mut another = request.clone(); another.invocation_digest = AssistantDigest::parse_hex(&"c".repeat(64)).unwrap();
        assert_ne!(facts.bindings[2].derived.as_ref().unwrap().plan_digest,
            HostDerivedResourceCollection::digest(&another,&another.demands[2],&facts.bindings[2].derived.as_ref().unwrap().members));
    }

    #[test]
    fn collection_rejects_undeclared_duplicate_outside_incomplete_or_understated_facts() {
        let (request,original) = resolved_collection();
        for mutation in 0..8 {
            let mut facts = original.clone();
            let collection = facts.bindings[2].derived.as_mut().unwrap();
            match mutation {
                0 => collection.members.push(collection.members[0].clone()),
                1 => if let PermissionResource::File {components,..} = &mut collection.members[0].resource {
                    *components = vec!["outside".into(),"app.ts".into()];
                },
                2 => collection.discovery_complete = false,
                3 => collection.members[0].before_version = "unknown".into(),
                4 => collection.members[0].observation = PermissionOperationRisk::OrdinaryRead,
                5 => collection.collection_id = "not-declared".into(),
                6 => collection.members[0].evidence_id.clear(),
                7 => collection.members[0].planned_after_version = collection.members[0].before_version.clone(),
                _ => unreachable!(),
            }
            collection.plan_digest = HostDerivedResourceCollection::digest(&request,&request.demands[2],&collection.members);
            assert!(facts.subject_for(&request).is_err(),"mutation {mutation}");
        }
        let mut facts = original.clone();
        facts.bindings[0].derived = facts.bindings[2].derived.clone();
        assert!(facts.subject_for(&request).is_err(),"a regular file demand cannot smuggle a collection");
        let mut facts = original;
        facts.bindings[2].derived.as_mut().unwrap().plan_digest = AssistantDigest::parse_hex(&"f".repeat(64)).unwrap();
        assert!(facts.subject_for(&request).is_err(),"the complete member plan must match its digest");
    }

    #[test]
    fn complete_empty_collection_is_not_truncation_and_oversized_collections_fail() {
        let (request,mut facts) = resolved_collection();
        let collection = facts.bindings[2].derived.as_mut().unwrap();
        collection.members.clear();
        collection.plan_digest = HostDerivedResourceCollection::digest(&request,&request.demands[2],&collection.members);
        assert_eq!(facts.subject_for(&request).unwrap().resources.len(),2);
        let (_,mut facts) = resolved_collection();
        let collection = facts.bindings[2].derived.as_mut().unwrap();
        let template = collection.members[0].clone();
        collection.members = (0..=MAX_DERIVED_RESOURCE_MEMBERS).map(|index| {
            let mut member = template.clone();
            if let PermissionResource::File {components,..} = &mut member.resource { components[1] = format!("app-{index:04}.ts"); }
            member
        }).collect();
        collection.plan_digest = HostDerivedResourceCollection::digest(&request,&request.demands[2],&collection.members);
        assert!(facts.subject_for(&request).is_err());
    }
    #[test]
    fn resolved_bindings_require_all_occurrences_even_when_reordered() {
        let (request, mut facts) = resolved_move();
        let expected = facts.subject_for(&request).unwrap();
        facts.bindings.reverse();
        assert_eq!(facts.subject_for(&request).unwrap(), expected);
        facts.bindings[0] = facts.bindings[1].clone();
        assert_eq!(facts.subject_for(&request), Err(ResourcePlanError::IncompleteResolution));
        facts.bindings.pop();
        assert_eq!(facts.subject_for(&request), Err(ResourcePlanError::IncompleteResolution));
    }
    #[test]
    fn resolution_cannot_change_target_input_demand_kind_or_access() {
        let (request, original) = resolved_move();
        for field in 0..7 {
            let mut facts = original.clone();
            match field {
                0 => facts.execution_target = "other-host".into(),
                1 => facts.invocation_digest = AssistantDigest::parse_hex(&"b".repeat(64)).unwrap(),
                2 => facts.resolution_id = "other-resolution".into(),
                3 => facts.bindings[0].demand.locator = "other-path".into(),
                4 => facts.bindings[0].evidence_id.clear(),
                5 => if let PermissionResource::File { access, .. } = &mut facts.bindings[0].resource { *access = "read".into(); },
                _ => facts.bindings[0].resource = PermissionResource::Platform {
                    namespace: "files".into(), resource_id: "source".into(), access: "remove".into() },
            }
            assert_eq!(facts.subject_for(&request), Err(ResourcePlanError::ResolutionMismatch), "{field}");
        }
    }
    #[test]
    fn wire_reopen_keeps_resolution_evidence_and_does_not_accept_extra_authority() {
        let (request, facts) = resolved_move();
        let wire = serde_json::to_value(&facts).unwrap();
        let restored: PermissionResolutionFacts = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(restored.subject_for(&request), facts.subject_for(&request));
        let mut forged = wire; forged["allowed"] = json!(true);
        assert!(serde_json::from_value::<PermissionResolutionFacts>(forged).is_err());
        let mut duplicate_request = request.clone(); duplicate_request.demands.push(request.demands[0].clone());
        assert_eq!(facts.subject_for(&duplicate_request), Err(ResourcePlanError::DuplicateRequirement));
    }
    #[test]
    fn preserves_both_move_resources_access_and_literal_paths() {
        let result = move_plan().extract(&json!({"source":" ../源文件 ","destination":"~/目标"})).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!((&*result[0].locator, &*result[0].access), (" ../源文件 ", "remove"));
        assert_eq!((&*result[1].locator, &*result[1].access), ("~/目标", "write"));
        assert!(move_plan().extract(&json!({"destination":"/same-target"})).is_err());
        assert!(move_plan().extract(&json!({"source":"/source"})).is_err());
        let literal = move_plan().extract(&json!({"source":" ", "destination":"a\\b\nc"})).unwrap();
        assert_eq!(literal[0].locator, " ");
        assert_eq!(literal[1].locator, "a\\b\nc");
    }
    #[test]
    fn optional_collections_skip_only_absent_or_empty_and_authorize_every_member() {
        let plan = PermissionResourcePlan { capability:"semantic_query".into(), operation:"query".into(), requirements:vec![
            ResourceRequirement { requirement_id:"support".into(), locator_kind:ResourceLocatorKind::FilePath, access:"read".into(),
                selector:ResourceSelector::OptionalEach {array_pointer:"/documents".into(),item_pointer:"/path".into()} }] };
        assert!(plan.extract(&json!({})).unwrap().is_empty());
        assert!(plan.extract(&json!({"documents":[]})).unwrap().is_empty());
        let demands = plan.extract(&json!({"documents":[{"path":"/a"},{"path":"/b"}]})).unwrap();
        assert_eq!(demands.len(),2);
        assert_eq!(demands[1].locator,"/b");
        for input in [json!({"documents":null}),json!({"documents":"/a"}),json!({"documents":[{}]})] {
            assert!(plan.extract(&input).is_err());
        }
    }

    #[test]
    fn every_array_item_is_required_and_duplicate_paths_retain_occurrence() {
        let plan = PermissionResourcePlan { capability: "multi_edit".into(), operation: "edit".into(), requirements: vec![
            ResourceRequirement { requirement_id: "edits".into(), locator_kind: ResourceLocatorKind::FilePath, access: "write".into(),
                selector: ResourceSelector::Each { array_pointer: "/edits".into(), item_pointer: "/path".into() } }] };
        let result = plan.extract(&json!({"edits":[{"path":"a"},{"path":"a"}]})).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!((result[0].item_index, result[1].item_index), (0, 1));
        assert_eq!(plan.extract(&json!({"edits":[{"path":"a"},{}]})),
            Err(ResourcePlanError::MissingResource { requirement_id: "edits".into(), item_index: 1 }));
        for input in [json!({}), json!({"edits":[]}), json!({"edits":"a"}), json!({"edits":[{"path":null}]})] {
            assert!(plan.extract(&input).is_err());
        }
    }
    #[test]
    fn pointers_are_explicit_and_no_tool_name_or_prompt_changes_selection() {
        let mut plan = move_plan();
        plan.capability = "陌生工具.search_current".into();
        plan.requirements = vec![requirement("literal", "read", "/a~1b/~0key")];
        let input = json!({"a/b":{"~key":"chosen"},"prompt":"read everything","path":"not-selected"});
        assert_eq!(plan.extract(&input).unwrap()[0].locator, "chosen");
        plan.requirements[0].selector = ResourceSelector::One { pointer: "/bad~2pointer".into() };
        assert_eq!(plan.extract(&input), Err(ResourcePlanError::InvalidPointer));
    }
    #[test]
    fn invalid_declarations_and_locator_types_fail_without_partial_results() {
        let mut plan = move_plan(); plan.requirements.push(plan.requirements[0].clone());
        assert_eq!(plan.extract(&json!({})), Err(ResourcePlanError::DuplicateRequirement));
        for value in [json!(null), json!(false), json!([]), json!({}), json!(""), json!("a\u{0000}b")] {
            assert!(move_plan().extract(&json!({"source":"valid", "destination":value})).is_err());
        }
        let mut wire = serde_json::to_value(move_plan()).unwrap(); wire["fallback"] = json!("allow");
        assert!(serde_json::from_value::<PermissionResourcePlan>(wire).is_err());
        let roundtrip: PermissionResourcePlan = serde_json::from_value(serde_json::to_value(move_plan()).unwrap()).unwrap();
        assert_eq!(roundtrip, move_plan());
    }
}
