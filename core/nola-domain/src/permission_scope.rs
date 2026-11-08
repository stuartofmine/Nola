// SPDX-License-Identifier: Apache-2.0
//! Concrete authorization scopes. Pure matching does not prove provenance,
//! resolve filesystem identities, or authorize execution by itself.
use crate::assistant_journal::AssistantDigest;
use serde::{Deserialize, Serialize};
use crate::permission_closure::CorePermissionOccurrence;
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PermissionOfferScope {
    Once,
    Reusable { scope: ReusablePermissionScope },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionScopeOffer {
    pub offer_id: AssistantDigest,
    pub scope: PermissionOfferScope,
}

/// Canonical bound offers: Core owns the subject/scope decision; Application
/// binds it to the original committed Core effect and canonical Run occurrence.
/// Read from that stored source, not a client's copy. Core must not invent the
/// database Run identity. These consistency checks prove neither provenance nor
/// that the caller's reusable scopes were actually decided by Core.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenPermissionOffers {
    pub occurrence: CorePermissionOccurrence,
    pub subject: ConcretePermissionSubject,
    pub offers: Vec<PermissionScopeOffer>,
}

/// Selection deliberately has no client-supplied scope/resource/pattern.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionOfferSelection {
    pub occurrence: CorePermissionOccurrence,
    pub offer_id: AssistantDigest,
}

impl FrozenPermissionOffers {
    pub fn freeze(occurrence: CorePermissionOccurrence, subject: ConcretePermissionSubject,
        reusable: Vec<ReusablePermissionScope>) -> Result<Self, ScopeError> {
        let mut frozen = Self { occurrence, subject, offers: Vec::new() };
        for scope in std::iter::once(PermissionOfferScope::Once)
            .chain(reusable.into_iter().map(|scope| PermissionOfferScope::Reusable { scope })) {
            frozen.offers.push(PermissionScopeOffer { offer_id: frozen.offer_id(&scope)?, scope });
        }
        frozen.validate()?;
        Ok(frozen)
    }

    fn offer_id(&self, scope: &PermissionOfferScope) -> Result<AssistantDigest, ScopeError> {
        // Typed structs fix field order; vector order is retained as part of
        // the frozen fact, not guessed or re-rendered from UI descriptions.
        let bytes = serde_json::to_vec(&("nola.permission-offer.v1", &self.occurrence, &self.subject, scope))
            .map_err(|_| ScopeError::InvalidOffer)?;
        AssistantDigest::parse_hex(&format!("{:x}", Sha256::digest(bytes))).map_err(|_| ScopeError::InvalidOffer)
    }

    pub fn validate(&self) -> Result<(), ScopeError> {
        identity(&self.occurrence.run_id)?;
        identity(&self.occurrence.core_permission_request_id)?;
        identity(&self.occurrence.effect_id)?;
        self.subject.validate()?;
        if self.offers.iter().filter(|offer| matches!(offer.scope, PermissionOfferScope::Once)).count() != 1 {
            return Err(ScopeError::InvalidOffer);
        }
        let ids: Vec<_> = self.offers.iter().map(|offer| offer.offer_id).collect();
        unique(&ids)?;
        for offer in &self.offers {
            if offer.offer_id != self.offer_id(&offer.scope)? { return Err(ScopeError::InvalidOffer); }
            if let PermissionOfferScope::Reusable { scope } = &offer.scope {
                if scope.evaluate(&self.subject)? != ScopeMatch::Covered { return Err(ScopeError::OutsideOfferedScope); }
            }
        }
        Ok(())
    }

    /// Borrow the stored scope rather than constructing one from user input.
    /// Store must still arbitrate closed/revoked/stop state in its write UoW.
    pub fn select(&self, selection: &PermissionOfferSelection) -> Result<&PermissionOfferScope, ScopeError> {
        self.validate()?;
        if selection.occurrence != self.occurrence { return Err(ScopeError::OccurrenceMismatch); }
        self.offers.iter().find(|offer| offer.offer_id == selection.offer_id)
            .map(|offer| &offer.scope).ok_or(ScopeError::UnknownOffer)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConcretePermissionSubject {
    pub execution_target: String,
    pub capability: String,
    pub operation: String,
    pub input_digest: AssistantDigest,
    pub resources: Vec<PermissionResource>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PermissionResource {
    /// Host resolves symlinks and volume identity before producing components.
    /// Empty components designate the filesystem root, not an unknown path.
    File { filesystem_id: String, components: Vec<String>, access: String },
    NetworkOrigin { origin: String, access: String },
    /// Host-confirmed invocation text and resolved cwd support explicit command
    /// rules; the digest independently binds the complete original input.
    ProcessInvocation { invocation_digest: AssistantDigest, command: String, cwd: String },
    Platform { namespace: String, resource_id: String, access: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResourceScope {
    Exact { resource: PermissionResource },
    DirectoryTree { filesystem_id: String, components: Vec<String>, access: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CapabilityScope {
    Exact { capability: String },
    /// Explicit set supplied by the contract, never inferred from a name prefix.
    Set { capabilities: Vec<String> },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReusablePermissionScope {
    pub execution_target: String,
    pub capabilities: CapabilityScope,
    pub operation: String,
    pub resources: Vec<ResourceScope>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScopeMatch { Covered, OutsideScope }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScopeError {
    InvalidIdentity, InvalidPath, InvalidOrigin, EmptyResources, DuplicateEntry,
    InvalidOffer, OutsideOfferedScope, OccurrenceMismatch, UnknownOffer,
}

impl std::fmt::Display for ScopeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{self:?}") }
}
impl std::error::Error for ScopeError {}

fn identity(value: &str) -> Result<(), ScopeError> {
    if value.trim().is_empty() || value.chars().any(char::is_control) {
        Err(ScopeError::InvalidIdentity)
    } else { Ok(()) }
}
fn path(filesystem: &str, components: &[String], access: &str) -> Result<(), ScopeError> {
    identity(filesystem)?;
    identity(access)?;
    if components.iter().any(|part| part.is_empty() || part == "." || part == ".."
        || part.contains('/') || part.contains('\0')) {
        return Err(ScopeError::InvalidPath);
    }
    Ok(())
}
fn unique<T: PartialEq>(items: &[T]) -> Result<(), ScopeError> {
    if items.iter().enumerate().any(|(i, value)| items[..i].contains(value)) {
        Err(ScopeError::DuplicateEntry)
    } else { Ok(()) }
}
impl PermissionResource {
    pub fn validate(&self) -> Result<(), ScopeError> {
        match self {
            Self::File { filesystem_id, components, access } => path(filesystem_id, components, access),
            Self::NetworkOrigin { origin, access } => {
                identity(access)?;
                let parsed = url::Url::parse(origin).map_err(|_| ScopeError::InvalidOrigin)?;
                if !matches!(parsed.scheme(), "http" | "https") || parsed.origin().ascii_serialization() != *origin {
                    return Err(ScopeError::InvalidOrigin);
                }
                Ok(())
            }
            Self::ProcessInvocation { command, cwd, .. } => {
                if command.trim().is_empty() || command.contains('\0') || cwd.trim().is_empty() || cwd.contains('\0') {
                    return Err(ScopeError::InvalidIdentity);
                }
                Ok(())
            },
            Self::Platform { namespace, resource_id, access } => {
                identity(namespace)?; identity(resource_id)?; identity(access)
            }
        }
    }
}
impl ConcretePermissionSubject {
    /// A reusable approval authorizes an operation within a declared scope;
    /// it does not promise immutable process inputs. ProcessInvocation carries
    /// the exact command, cwd and invocation parameters, and the subject binds
    /// the Host/capability/operation. Fresh resolution, risk checks and explicit
    /// deny/revocation still apply on every execution.
    pub fn supports_reusable_approval(&self) -> bool {
        self.validate().is_ok()
    }

    /// Stable approval boundary excludes incidental query/argv bytes, while
    /// retaining the exact Host, operation and resolved resource identities.
    pub fn approval_resource_identity(&self) -> Result<String, ScopeError> {
        self.validate()?;
        let mut resources = self.resources.iter().map(|resource|serde_json::to_value(resource)
            .expect("permission resource is serializable")).collect::<Vec<_>>();
        resources.sort_by_key(serde_json::Value::to_string);
        Ok(serde_json::json!({"execution_target":self.execution_target,"capability":self.capability,
            "operation":self.operation,"resources":resources}).to_string())
    }

    pub fn validate(&self) -> Result<(), ScopeError> {
        identity(&self.execution_target)?; identity(&self.capability)?; identity(&self.operation)?;
        if self.resources.is_empty() { return Err(ScopeError::EmptyResources); }
        unique(&self.resources)?;
        for resource in &self.resources { resource.validate()?; }
        Ok(())
    }
}
impl ReusablePermissionScope {
    /// Caller must separately establish selected-offer provenance, live grant
    /// ownership and resource identity freshness. Covered is not execution authority.
    pub fn evaluate(&self, subject: &ConcretePermissionSubject) -> Result<ScopeMatch, ScopeError> {
        subject.validate()?;
        if !subject.supports_reusable_approval() { return Ok(ScopeMatch::OutsideScope); }
        identity(&self.execution_target)?; identity(&self.operation)?;
        let capability_matches = match &self.capabilities {
            CapabilityScope::Exact { capability } => { identity(capability)?; capability == &subject.capability }
            CapabilityScope::Set { capabilities } => {
                if capabilities.is_empty() { return Err(ScopeError::InvalidIdentity); }
                unique(capabilities)?;
                for capability in capabilities { identity(capability)?; }
                capabilities.contains(&subject.capability)
            }
        };
        if self.resources.is_empty() { return Err(ScopeError::EmptyResources); }
        unique(&self.resources)?;
        for resource in &self.resources {
            match resource {
                ResourceScope::Exact { resource } => resource.validate()?,
                ResourceScope::DirectoryTree { filesystem_id, components, access } => path(filesystem_id, components, access)?,
            }
        }
        let covered = self.execution_target == subject.execution_target && capability_matches
            && self.operation == subject.operation && subject.resources.iter().all(|requested| {
                self.resources.iter().any(|scope| match (scope, requested) {
                    (ResourceScope::Exact { resource }, requested) => resource == requested,
                    (ResourceScope::DirectoryTree { filesystem_id, components, access },
                     PermissionResource::File { filesystem_id: fs, components: parts, access: requested_access }) =>
                        filesystem_id == fs && access == requested_access && parts.starts_with(components),
                    _ => false,
                })
            });
        Ok(if covered { ScopeMatch::Covered } else { ScopeMatch::OutsideScope })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn approval_identity_tracks_resolved_targets_without_binding_query_bytes() {
        let original = subject();
        let mut changed_input = original.clone();
        changed_input.input_digest = AssistantDigest::parse_hex(&"b".repeat(64)).unwrap();
        assert_eq!(original.approval_resource_identity().unwrap(),changed_input.approval_resource_identity().unwrap());
        changed_input.execution_target = "different-host".into();
        assert_ne!(original.approval_resource_identity().unwrap(),changed_input.approval_resource_identity().unwrap());
        changed_input = original.clone();
        changed_input.resources = vec![file(&["other","a.rs"],"write")];
        assert_ne!(original.approval_resource_identity().unwrap(),changed_input.approval_resource_identity().unwrap());
    }
    fn file(parts: &[&str], access: &str) -> PermissionResource {
        PermissionResource::File { filesystem_id: "volume".into(),
            components: parts.iter().map(|part| (*part).into()).collect(), access: access.into() }
    }
    fn subject() -> ConcretePermissionSubject {
        ConcretePermissionSubject { execution_target: "mac-host".into(), capability: "file.write".into(),
            operation: "write".into(), input_digest: AssistantDigest::parse_hex(&"a".repeat(64)).unwrap(),
            resources: vec![file(&["project", "src", "a.rs"], "write")] }
    }
    fn scope() -> ReusablePermissionScope {
        ReusablePermissionScope { execution_target: "mac-host".into(),
            capabilities: CapabilityScope::Exact { capability: "file.write".into() }, operation: "write".into(),
            resources: vec![ResourceScope::DirectoryTree { filesystem_id: "volume".into(),
                components: vec!["project".into()], access: "write".into() }] }
    }
    fn occurrence() -> CorePermissionOccurrence {
        CorePermissionOccurrence { run_id: "run".into(), core_permission_request_id: "permission".into(), effect_id: "effect".into() }
    }
    #[test]
    fn frozen_selection_roundtrips_and_borrows_exact_original_offer() {
        let original = FrozenPermissionOffers::freeze(occurrence(), subject(), vec![scope()]).unwrap();
        let restored: FrozenPermissionOffers = serde_json::from_str(&serde_json::to_string(&original).unwrap()).unwrap();
        for offer in &restored.offers {
            let selection = PermissionOfferSelection { occurrence: occurrence(), offer_id: offer.offer_id };
            assert!(std::ptr::eq(restored.select(&selection).unwrap(), &offer.scope));
            let mut wire = serde_json::to_value(selection).unwrap(); wire["scope"] = serde_json::json!({"allow_all":true});
            assert!(serde_json::from_value::<PermissionOfferSelection>(wire).is_err());
        }
        assert_eq!(restored, original);
    }
    #[test]
    fn frozen_offers_reject_missing_once_duplicates_and_unrelated_scope() {
        let mut outside = scope(); outside.execution_target = "other".into();
        assert_eq!(FrozenPermissionOffers::freeze(occurrence(), subject(), vec![outside]), Err(ScopeError::OutsideOfferedScope));
        assert_eq!(FrozenPermissionOffers::freeze(occurrence(), subject(), vec![scope(), scope()]), Err(ScopeError::DuplicateEntry));
        let mut original = FrozenPermissionOffers::freeze(occurrence(), subject(), vec![scope()]).unwrap();
        original.offers.remove(0);
        assert_eq!(original.validate(), Err(ScopeError::InvalidOffer));
    }
    #[test]
    fn selection_cannot_transfer_to_changed_input_effect_or_expanded_scope() {
        let original = FrozenPermissionOffers::freeze(occurrence(), subject(), vec![scope()]).unwrap();
        let selection = PermissionOfferSelection { occurrence: occurrence(), offer_id: original.offers[1].offer_id };
        let mut changed_input = subject();
        changed_input.input_digest = AssistantDigest::parse_hex(&"b".repeat(64)).unwrap();
        let next = FrozenPermissionOffers::freeze(occurrence(), changed_input, vec![scope()]).unwrap();
        assert_eq!(next.select(&selection), Err(ScopeError::UnknownOffer));
        let mut other = occurrence(); other.effect_id = "next-effect".into();
        let next = FrozenPermissionOffers::freeze(other, subject(), vec![scope()]).unwrap();
        assert_eq!(next.select(&selection), Err(ScopeError::OccurrenceMismatch));
        let mut corrupted = original;
        if let PermissionOfferScope::Reusable { scope } = &mut corrupted.offers[1].scope {
            if let ResourceScope::DirectoryTree { components, .. } = &mut scope.resources[0] { components.clear(); }
        }
        assert_eq!(corrupted.select(&selection), Err(ScopeError::InvalidOffer));
    }
    #[test]
    fn directory_coverage_never_crosses_target_capability_operation_or_component_boundary() {
        assert_eq!(scope().evaluate(&subject()).unwrap(), ScopeMatch::Covered);
        for field in 0..6 {
            let mut requested = subject();
            match field {
                0 => requested.execution_target = "remote-host".into(),
                1 => requested.capability = "file.delete".into(),
                2 => requested.operation = "delete".into(),
                3 => requested.resources = vec![file(&["project-backup", "a.rs"], "write")],
                4 => requested.resources = vec![file(&["project", "a.rs"], "read")],
                _ => if let PermissionResource::File { filesystem_id, .. } = &mut requested.resources[0] { *filesystem_id = "other-volume".into(); },
            }
            assert_eq!(scope().evaluate(&requested).unwrap(), ScopeMatch::OutsideScope, "{field}");
        }
    }
    #[test]
    fn multi_resource_operation_requires_every_resource_and_explicit_capability_set() {
        let mut requested = subject();
        requested.resources.push(file(&["elsewhere", "a.rs"], "write"));
        assert_eq!(scope().evaluate(&requested).unwrap(), ScopeMatch::OutsideScope);
        let mut granted = scope();
        granted.resources.push(ResourceScope::Exact { resource: requested.resources[1].clone() });
        assert_eq!(granted.evaluate(&requested).unwrap(), ScopeMatch::Covered);
        requested.capability = "file.patch".into();
        assert_eq!(granted.evaluate(&requested).unwrap(), ScopeMatch::OutsideScope);
        granted.capabilities = CapabilityScope::Set { capabilities: vec!["file.write".into(), "file.patch".into()] };
        assert_eq!(granted.evaluate(&requested).unwrap(), ScopeMatch::Covered);
    }
    #[test]
    fn malformed_path_empty_and_duplicate_resources_are_errors_not_authorization() {
        for component in ["..", ".", "", "a/b", "a\0b"] {
            let mut requested = subject(); requested.resources = vec![file(&["project", component], "write")];
            assert_eq!(scope().evaluate(&requested), Err(ScopeError::InvalidPath));
        }
        let mut requested = subject(); requested.resources.clear();
        assert_eq!(scope().evaluate(&requested), Err(ScopeError::EmptyResources));
        requested = subject(); requested.resources.push(requested.resources[0].clone());
        assert_eq!(scope().evaluate(&requested), Err(ScopeError::DuplicateEntry));
        // Components are produced by the target resolver. Backslash/newline
        // can be filename bytes on POSIX, not universal path separators.
        requested.resources = vec![file(&["project", "a\\b\nc"], "write")];
        assert_eq!(scope().evaluate(&requested).unwrap(), ScopeMatch::Covered);
    }
    #[test]
    fn network_scope_requires_canonical_origin_and_exact_port() {
        for origin in ["https://example.com", "https://example.com:8443"] {
            PermissionResource::NetworkOrigin { origin: origin.into(), access: "connect".into() }.validate().unwrap();
        }
        for origin in ["https://example.com/path", "https://example.com:443", "https://user@example.com", "null", "file:///tmp/a"] {
            assert_eq!(PermissionResource::NetworkOrigin { origin: origin.into(), access: "connect".into() }.validate(), Err(ScopeError::InvalidOrigin));
        }
        let mut requested = subject();
        requested.resources = vec![PermissionResource::NetworkOrigin { origin: "https://example.com:8443".into(), access: "connect".into() }];
        let mut granted = scope();
        granted.resources = vec![ResourceScope::Exact { resource: PermissionResource::NetworkOrigin { origin: "https://example.com".into(), access: "connect".into() } }];
        assert_eq!(granted.evaluate(&requested).unwrap(), ScopeMatch::OutsideScope);
    }
    #[test]
    fn process_identity_and_platform_access_do_not_broaden() {
        let mut requested = subject();
        requested.resources = vec![PermissionResource::ProcessInvocation { invocation_digest: requested.input_digest,
            command: "cargo test".into(), cwd: "/project".into() }];
        let mut granted = scope();
        granted.resources = vec![ResourceScope::Exact { resource: requested.resources[0].clone() }];
        assert!(requested.supports_reusable_approval());
        assert_eq!(granted.evaluate(&requested).unwrap(), ScopeMatch::Covered);
        if let PermissionResource::ProcessInvocation {cwd,..} = &mut requested.resources[0] {
            *cwd = "/other-project".into();
        }
        assert_eq!(granted.evaluate(&requested).unwrap(), ScopeMatch::OutsideScope);
        requested.resources = vec![PermissionResource::ProcessInvocation {
            invocation_digest: AssistantDigest::parse_hex(&"b".repeat(64)).unwrap(),
            command: "cargo test".into(), cwd: "/project".into() }];
        assert_eq!(granted.evaluate(&requested).unwrap(), ScopeMatch::OutsideScope);
        requested.resources = vec![PermissionResource::Platform {
            namespace: "clipboard".into(), resource_id: "primary".into(), access: "read".into() }];
        granted.resources = vec![ResourceScope::Exact { resource: requested.resources[0].clone() }];
        assert_eq!(granted.evaluate(&requested).unwrap(), ScopeMatch::Covered);
        if let PermissionResource::Platform { access, .. } = &mut requested.resources[0] { *access = "write".into(); }
        assert_eq!(granted.evaluate(&requested).unwrap(), ScopeMatch::OutsideScope);
    }
    #[test]
    fn process_execution_can_offer_an_explicit_exact_scope_without_freezing_dependencies() {
        for command in ["curl --data-binary @payload https://example.test", "tool $PAYLOAD", "./script"] {
            let mut requested = subject();
            requested.resources.push(PermissionResource::ProcessInvocation {
                invocation_digest: requested.input_digest, command: command.into(), cwd: "/project".into(),
            });
            let exact = ReusablePermissionScope { execution_target:requested.execution_target.clone(),
                capabilities:CapabilityScope::Exact {capability:requested.capability.clone()},
                operation:requested.operation.clone(),resources:requested.resources.iter().cloned()
                    .map(|resource|ResourceScope::Exact {resource}).collect() };
            assert_eq!(exact.evaluate(&requested).unwrap(),ScopeMatch::Covered);
            let offers = FrozenPermissionOffers::freeze(occurrence(),requested.clone(),vec![exact]).unwrap();
            assert_eq!(offers.offers.len(), 2);
            let once = FrozenPermissionOffers::freeze(occurrence(),requested,vec![]).unwrap();
            assert_eq!(once.offers.len(),1);
            assert!(matches!(once.offers[0].scope,PermissionOfferScope::Once));
        }
    }
    #[test]
    fn wire_roundtrip_rejects_unknown_fields_and_missing_target() {
        let wire = serde_json::to_value(scope()).unwrap();
        assert_eq!(serde_json::from_value::<ReusablePermissionScope>(wire.clone()).unwrap(), scope());
        let mut invalid = wire.clone(); invalid["allow_all"] = serde_json::json!(true);
        assert!(serde_json::from_value::<ReusablePermissionScope>(invalid).is_err());
        let mut invalid = wire; invalid.as_object_mut().unwrap().remove("execution_target");
        assert!(serde_json::from_value::<ReusablePermissionScope>(invalid).is_err());
    }
}
