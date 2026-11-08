// SPDX-License-Identifier: Apache-2.0
//! Trusted Host template selection, current resource authorization and private
//! capture provenance. These facts are not tool grants or renderer commands.
use nola_agent_engine::agent::resource_access::{
    evaluate_resource_access, AgentResourceAccessDecision, AgentResourceAccessGrant,
    AgentResourceAccessRequest, AgentResourceIdentity, AgentResourceKind, AgentResourcePermission,
    AgentResourcePrincipal, AgentResourcePrincipalKind,
};
use nola_domain::expert_message::{AdmitExpertConversationMessage, ExpertPromptDefinition};
use nola_store_port::product_setting::ProductSettingRepository;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

/// Aggregate native selection/catalog input budget, checked before admission
/// writes. Private persisted facts have their separate shared Domain budget.
pub const EXPERT_TEMPLATE_HOST_CONTEXT_MAX_BYTES: usize = 1_048_576;

/// Constructed by the authenticated native/server Host, never Application JSON.
/// Current time is deliberately absent: Application supplies its real clock.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostExpertTemplateCaptureContext {
    pub actor: AgentResourcePrincipal,
    pub request_host_id: Option<String>,
    pub selections: Vec<HostExpertTemplateSelection>,
    pub builtins: Vec<HostExpertBuiltinDefinition>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostExpertTemplateAuthority {
    pub actor: AgentResourcePrincipal,
    pub request_host_id: Option<String>,
}
impl HostExpertTemplateAuthority {
    pub fn validate(&self) -> Result<(), String> {
        if self.actor.kind == AgentResourcePrincipalKind::Anyone
            || !valid_identity(&self.actor.id)
            || self
                .request_host_id
                .as_ref()
                .is_some_and(|v| !valid_identity(v))
        {
            return Err("invalid_expert_template_host_authority".into());
        }
        Ok(())
    }
}
impl HostExpertTemplateCaptureContext {
    pub fn authority(&self) -> HostExpertTemplateAuthority {
        HostExpertTemplateAuthority {
            actor: self.actor.clone(),
            request_host_id: self.request_host_id.clone(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostExpertTemplateSelection {
    pub context_conversation_id: String,
    pub context_configuration_revision: i64,
    /// Exact current profile pin from Core's read-only preview, not execution authority.
    pub model_profile: nola_host_protocol::HostModelProfileReference,
    pub source: HostExpertTemplateSelectionSource,
    pub template_id: Option<String>,
    /// Exact user-defined instruction components from this native Host's model
    /// configuration producer. They are not managed-resource template claims.
    pub host_user_instructions: Vec<ExpertPromptDefinition>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostExpertTemplateSelectionSource {
    Explicit,
    HostDefault,
    NoTemplate,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpertTemplateCapturePreview {
    pub participants: Vec<ExpertTemplateCapturePreviewEntry>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpertTemplateCapturePreviewEntry {
    pub participant: nola_domain::expert_message::ExpertParticipantReference,
    pub context_configuration_revision: i64,
    pub defaults: nola_domain::persistent_agent::AgentDefaults,
    /// Current selected model's presentation block from the same context
    /// snapshot. This is not an AgentDefaults field or a resource permission.
    /// Hosts register any produced instruction bodies in the capture context.
    pub client_model_presentation: serde_json::Value,
    pub model_declaration: nola_domain::expert_message::ExpertModelDeclaration,
    /// Same-snapshot generation result; the declaration above remains raw.
    pub effective_parameters: crate::effective_model_parameters::EffectiveModelResolution,
}

/// Only a native code-defined catalog supplies builtin bodies. Custom bodies
/// must be read by Core from its own current transactional Store snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostExpertBuiltinDefinition {
    pub provider_id: String,
    pub template_id: String,
    pub content: String,
    pub platform: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpertTemplateCaptureFact {
    pub version: u32,
    pub conversation_id: String,
    pub submission_id: String,
    pub message_id: String,
    pub intent_sha256: String,
    pub actor: AgentResourcePrincipal,
    pub request_host_id: Option<String>,
    pub captured_at_ms: i64,
    pub entries: Vec<ExpertTemplateCaptureEntry>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpertTemplateCaptureEntry {
    pub participant_id: String,
    pub context_conversation_id: String,
    pub context_configuration_revision: i64,
    pub child_submission_id: String,
    /// Exact definition UTF-8 body digest, equal to expected.content_sha256.
    /// Component/resource identity is validated independently; platform is not a grant.
    pub definition_sha256: Option<String>,
    pub host_user_instructions: Vec<ExpertPromptDefinition>,
    pub origin: ExpertTemplateCaptureOrigin,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExpertTemplateCaptureOrigin {
    NoTemplate,
    Custom {
        catalog_revision: i64,
        resource: AgentResourceIdentity,
        decision: AgentResourceAccessDecision,
    },
    Builtin {
        provider_id: String,
        template_id: String,
    },
    /// Reuse of already captured conversation instruction bytes, not a new
    /// resource access decision or a grant carried into the new Run.
    Retained {
        source_conversation_id: String,
        source_submission_id: String,
        source_participant_id: String,
        source_capture_sha256: String,
    },
}

pub fn payload_sha256(value: &impl Serialize) -> Result<String, String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value).map_err(|e| e.to_string())?)
    ))
}
fn valid_identity(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 512 && !value.chars().any(char::is_control)
}
impl HostExpertTemplateCaptureContext {
    pub fn validate(&self) -> Result<(), String> {
        if serde_json::to_vec(self)
            .map_err(|_| "invalid_expert_template_host_context")?
            .len()
            > EXPERT_TEMPLATE_HOST_CONTEXT_MAX_BYTES
        {
            return Err("expert_template_host_context_exceeds_budget".into());
        }
        if self.actor.kind == AgentResourcePrincipalKind::Anyone
            || !valid_identity(&self.actor.id)
            || self
                .request_host_id
                .as_ref()
                .is_some_and(|v| !valid_identity(v))
            || self.selections.len() > 32
            || self.builtins.len() > 128
        {
            return Err("invalid_expert_template_host_context".into());
        }
        let mut contexts = BTreeSet::new();
        for selection in &self.selections {
            if !valid_identity(&selection.context_conversation_id)
                || selection.context_configuration_revision < 1
                || !valid_identity(&selection.model_profile.profile_id)
                || selection.model_profile.admitted_revision < 1
                || !contexts.insert(&selection.context_conversation_id)
                || matches!(
                    selection.source,
                    HostExpertTemplateSelectionSource::NoTemplate
                ) != selection.template_id.is_none()
                || selection
                    .template_id
                    .as_ref()
                    .is_some_and(|id| uuid::Uuid::parse_str(id).is_err())
            {
                return Err("invalid_expert_template_host_selection".into());
            }
            validate_host_user_instructions(&selection.host_user_instructions)?;
        }
        let mut builtins = BTreeSet::new();
        for definition in &self.builtins {
            let id = uuid::Uuid::parse_str(&definition.template_id)
                .map_err(|_| "invalid_expert_builtin_id")?;
            if !builtins.insert(id)
                || !valid_identity(&definition.provider_id)
                || definition.content.is_empty()
                || definition.content.len() > 262_144
                || definition.content.contains('\0')
            {
                return Err("invalid_expert_builtin_definition".into());
            }
        }
        Ok(())
    }
    pub fn validate_input(&self, input: &AdmitExpertConversationMessage) -> Result<(), String> {
        self.validate()?;
        let selected: BTreeSet<_> = input
            .selected_participants
            .iter()
            .map(|p| p.context_conversation_id.as_str())
            .collect();
        let contexts: BTreeSet<_> = self
            .selections
            .iter()
            .map(|p| p.context_conversation_id.as_str())
            .collect();
        if selected != contexts {
            return Err("expert_template_host_selection_set_changed".into());
        }
        Ok(())
    }
}

pub fn validate_host_user_instructions(
    definitions: &[ExpertPromptDefinition],
) -> Result<(), String> {
    if definitions.len() > 8 {
        return Err("expert_host_user_instructions_exceed_bound".into());
    }
    let mut components = BTreeSet::new();
    for definition in definitions {
        if !valid_identity(&definition.component_id)
            || !components.insert(&definition.component_id)
            || definition.definition_revision.is_some()
            || definition.content.trim().is_empty()
            || definition.content.len() > 262_144
            || definition.content.contains('\0')
            || definition.content_sha256
                != format!("{:x}", Sha256::digest(definition.content.as_bytes()))
        {
            return Err("invalid_expert_host_user_instruction".into());
        }
    }
    Ok(())
}

pub struct CurrentExpertCustomCatalog {
    pub revision: i64,
    pub templates: Vec<CurrentExpertCustomTemplate>,
}
pub struct CurrentExpertCustomTemplate {
    pub id: String,
    pub name: String,
    pub content: String,
    pub resource: Option<AgentResourceIdentity>,
    pub grants: Vec<AgentResourceAccessGrant>,
}
impl CurrentExpertCustomTemplate {
    pub fn decision(
        &self,
        host: &HostExpertTemplateCaptureContext,
        now_epoch_ms: i64,
    ) -> Result<AgentResourceAccessDecision, String> {
        if now_epoch_ms < 0 {
            return Err("invalid_expert_template_capture_time".into());
        }
        let resource = self
            .resource
            .clone()
            .ok_or("expert_template_ownership_facts_missing")?;
        Ok(evaluate_resource_access(&AgentResourceAccessRequest {
            schema_version: 1,
            resource,
            actor: host.actor.clone(),
            permission: AgentResourcePermission::ReadContent,
            grants: self.grants.clone(),
            now_epoch_ms,
            request_host_id: host.request_host_id.clone(),
        }))
    }
    pub fn definition(
        &self,
        catalog_revision: i64,
        selected_id: &str,
    ) -> Result<ExpertPromptDefinition, String> {
        let revision = u64::try_from(catalog_revision)
            .ok()
            .filter(|v| *v > 0)
            .ok_or("invalid_prompt_catalog_revision")?;
        Ok(ExpertPromptDefinition {
            component_id: format!("prompt_template:{selected_id}"),
            content: self.content.clone(),
            content_sha256: format!("{:x}", Sha256::digest(self.content.as_bytes())),
            definition_revision: Some(revision),
            platform: serde_json::Value::Null,
        })
    }
}

pub fn load_custom_catalog<R: ProductSettingRepository + ?Sized>(
    store: &R,
) -> Result<Option<CurrentExpertCustomCatalog>, String> {
    let Some(setting) = store
        .product_setting("prompt.templates")
        .map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };
    parse_custom_catalog(setting.revision, &setting.value_json).map(Some)
}

pub fn parse_custom_catalog(
    catalog_revision: i64,
    value_json: &str,
) -> Result<CurrentExpertCustomCatalog, String> {
    if catalog_revision < 1 {
        return Err("invalid_prompt_catalog_revision".into());
    }
    let catalog: serde_json::Value =
        serde_json::from_str(value_json).map_err(|_| "invalid_prompt_catalog")?;
    let fields = catalog.as_object().ok_or("invalid_prompt_catalog")?;
    if fields
        .keys()
        .any(|key| !matches!(key.as_str(), "customTemplates" | "selectedTemplateID"))
    {
        return Err("unknown_prompt_catalog_field".into());
    }
    if let Some(selected) = catalog.get("selectedTemplateID").filter(|v| !v.is_null()) {
        let id = selected
            .as_str()
            .ok_or("invalid_prompt_catalog_selection")?;
        uuid::Uuid::parse_str(id).map_err(|_| "invalid_prompt_catalog_selection")?;
    }
    let raw = catalog
        .get("customTemplates")
        .and_then(serde_json::Value::as_array)
        .ok_or("invalid_prompt_catalog")?;
    let mut ids = BTreeSet::new();
    let mut templates = Vec::with_capacity(raw.len());
    for template in raw {
        let fields = template
            .as_object()
            .ok_or("invalid_custom_prompt_definition")?;
        const FIELDS: &[&str] = &[
            "id",
            "name",
            "language",
            "content",
            "description",
            "isBuiltIn",
            "createdAt",
            "updatedAt",
            "accessOwnerID",
            "accessRevision",
            "accessGrants",
        ];
        if fields.keys().any(|key| !FIELDS.contains(&key.as_str())) {
            return Err("unknown_custom_prompt_field".into());
        }
        let id = template
            .get("id")
            .and_then(serde_json::Value::as_str)
            .ok_or("invalid_prompt_id")?;
        let parsed_id = uuid::Uuid::parse_str(id).map_err(|_| "invalid_prompt_id")?;
        let name = template
            .get("name")
            .and_then(serde_json::Value::as_str)
            .filter(|v| !v.trim().is_empty())
            .ok_or("invalid_prompt_name")?;
        let content = template
            .get("content")
            .and_then(serde_json::Value::as_str)
            .ok_or("invalid_prompt_content")?;
        template
            .get("description")
            .and_then(serde_json::Value::as_str)
            .ok_or("invalid_prompt_description")?;
        for key in ["createdAt", "updatedAt"] {
            if template
                .get(key)
                .and_then(serde_json::Value::as_f64)
                .filter(|v| v.is_finite())
                .is_none()
            {
                return Err("invalid_prompt_metadata_date".into());
            }
        }
        if template
            .get("language")
            .filter(|v| !v.is_null())
            .is_some_and(|v| !v.is_string())
        {
            return Err("invalid_prompt_language".into());
        }
        if !ids.insert(parsed_id)
            || content.len() > 262_144
            || content.contains('\0')
            || template.get("isBuiltIn") != Some(&serde_json::Value::Bool(false))
        {
            return Err("invalid_custom_prompt_definition".into());
        }
        let owner = template
            .get("accessOwnerID")
            .filter(|v| !v.is_null())
            .map(|v| {
                v.as_str()
                    .filter(|v| valid_identity(v))
                    .ok_or("invalid_prompt_owner")
            })
            .transpose()?;
        let revision = template
            .get("accessRevision")
            .filter(|v| !v.is_null())
            .map(|v| {
                v.as_u64()
                    .filter(|v| *v > 0)
                    .ok_or("invalid_prompt_access_revision")
            })
            .transpose()?;
        let grants: Vec<AgentResourceAccessGrant> =
            match template.get("accessGrants").filter(|v| !v.is_null()) {
                Some(value) => {
                    serde_json::from_value(value.clone()).map_err(|_| "invalid_prompt_grants")?
                }
                None => Vec::new(),
            };
        let resource = owner
            .zip(revision)
            .map(|(owner, revision)| AgentResourceIdentity {
                kind: AgentResourceKind::PromptTemplate,
                resource_id: parsed_id.to_string(),
                owner: AgentResourcePrincipal {
                    kind: AgentResourcePrincipalKind::LocalUser,
                    id: owner.into(),
                },
                host_id: None,
                revision,
                content_digest: Some(format!("sha256:{:x}", Sha256::digest(content.as_bytes()))),
            });
        templates.push(CurrentExpertCustomTemplate {
            id: id.into(),
            name: name.into(),
            content: content.into(),
            resource,
            grants,
        });
    }
    Ok(CurrentExpertCustomCatalog {
        revision: catalog_revision,
        templates,
    })
}

pub struct ResolvedExpertTemplate {
    pub template_id: Option<String>,
    pub definition: Option<ExpertPromptDefinition>,
    pub origin: ExpertTemplateCaptureOrigin,
}

/// Resolve one current selection inside the caller's complete-set transaction.
/// A missing custom member is never interpreted as a builtin declaration.
pub fn resolve_selection(
    host: &HostExpertTemplateCaptureContext,
    context_id: &str,
    configuration_revision: i64,
    configuration: &serde_json::Value,
    catalog: Option<&CurrentExpertCustomCatalog>,
    now_epoch_ms: i64,
) -> Result<ResolvedExpertTemplate, String> {
    let selection = host
        .selections
        .iter()
        .find(|s| s.context_conversation_id == context_id)
        .ok_or("expert_template_host_selection_missing")?;
    if selection.context_configuration_revision != configuration_revision {
        return Err("expert_template_context_configuration_changed".into());
    }
    let configured = match configuration.get("selectedPromptTemplateID") {
        None | Some(serde_json::Value::Null) => None,
        Some(value) => Some(
            value
                .as_str()
                .ok_or("expert_template_invalid_configured_selection")?,
        ),
    };
    match (
        configured,
        selection.source,
        selection.template_id.as_deref(),
    ) {
        (Some(configured), HostExpertTemplateSelectionSource::Explicit, Some(selected)) => {
            if uuid::Uuid::parse_str(configured)
                .map_err(|_| "expert_template_invalid_configured_selection")?
                != uuid::Uuid::parse_str(selected)
                    .map_err(|_| "invalid_expert_template_host_selection")?
            {
                return Err("expert_template_explicit_selection_changed".into());
            }
        }
        (None, HostExpertTemplateSelectionSource::HostDefault, Some(_)) => {}
        (None, HostExpertTemplateSelectionSource::NoTemplate, None) => {
            return Ok(ResolvedExpertTemplate {
                template_id: None,
                definition: None,
                origin: ExpertTemplateCaptureOrigin::NoTemplate,
            });
        }
        _ => return Err("expert_template_selection_source_changed".into()),
    }
    let selected = uuid::Uuid::parse_str(
        selection
            .template_id
            .as_deref()
            .ok_or("expert_template_selection_missing")?,
    )
    .map_err(|_| "invalid_expert_template_host_selection")?;
    let custom = catalog.and_then(|c| {
        c.templates
            .iter()
            .find(|t| uuid::Uuid::parse_str(&t.id).ok() == Some(selected))
    });
    let builtin = host
        .builtins
        .iter()
        .find(|t| uuid::Uuid::parse_str(&t.template_id).ok() == Some(selected));
    match (custom, builtin) {
        (Some(_), Some(_)) => Err("expert_template_definition_source_ambiguous".into()),
        (None, None) => Err("expert_template_definition_missing".into()),
        (Some(template), None) => {
            let catalog_revision = catalog.ok_or("expert_template_catalog_missing")?.revision;
            let decision = template.decision(host, now_epoch_ms)?;
            if !decision.allowed {
                return Err("expert_template_read_content_denied".into());
            }
            Ok(ResolvedExpertTemplate {
                template_id: Some(template.id.clone()),
                definition: Some(template.definition(catalog_revision, &template.id)?),
                origin: ExpertTemplateCaptureOrigin::Custom {
                    catalog_revision,
                    resource: template
                        .resource
                        .clone()
                        .ok_or("expert_template_ownership_facts_missing")?,
                    decision,
                },
            })
        }
        (None, Some(template)) => Ok(ResolvedExpertTemplate {
            template_id: Some(template.template_id.clone()),
            definition: Some(ExpertPromptDefinition {
                component_id: format!("prompt_template:{}", template.template_id),
                content: template.content.clone(),
                content_sha256: format!("{:x}", Sha256::digest(template.content.as_bytes())),
                definition_revision: None,
                platform: template.platform.clone(),
            }),
            origin: ExpertTemplateCaptureOrigin::Builtin {
                provider_id: template.provider_id.clone(),
                template_id: template.template_id.clone(),
            },
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn host(id: &str, selection: &str) -> HostExpertTemplateCaptureContext {
        HostExpertTemplateCaptureContext {
            actor: AgentResourcePrincipal {
                kind: AgentResourcePrincipalKind::LocalUser,
                id: id.into(),
            },
            request_host_id: None,
            selections: vec![HostExpertTemplateSelection {
                context_conversation_id: "context".into(),
                context_configuration_revision: 3,
                model_profile: nola_host_protocol::HostModelProfileReference {
                    profile_id: "profile".into(),
                    admitted_revision: 1,
                },
                source: HostExpertTemplateSelectionSource::Explicit,
                template_id: Some(selection.into()),
                host_user_instructions: vec![],
            }],
            builtins: vec![],
        }
    }
    fn template(id: &str) -> serde_json::Value {
        json!({"id":id,"name":"Definition","content":"Private body","isBuiltIn":false,
            "description":"Current template", "createdAt":0, "updatedAt":0,
            "accessOwnerID":"owner","accessRevision":4,"accessGrants":[]})
    }
    fn put(store: &mut nola_store::NolaStore, template: serde_json::Value, at: i64) {
        store
            .put_product_setting(
                "prompt.templates",
                &json!({"customTemplates":[template],"selectedTemplateID":null}).to_string(),
                at,
            )
            .unwrap();
    }
    fn resolve(
        store: &mut nola_store::NolaStore,
        host: &HostExpertTemplateCaptureContext,
        selected: &str,
    ) -> Result<ResolvedExpertTemplate, String> {
        host.validate()?;
        resolve_selection(
            host,
            "context",
            3,
            &json!({"selectedPromptTemplateID":selected}),
            load_custom_catalog(&store.product_setting_repository())?.as_ref(),
            100,
        )
    }
    #[test]
    fn actual_catalog_reauthorizes_current_body_and_never_claims_missing_ownership() {
        let mut store = nola_store::NolaStore::open(":memory:").unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        put(&mut store, template(&id), 1);
        let captured = resolve(&mut store, &host("owner", &id), &id).unwrap();
        assert_eq!(
            captured.definition.as_ref().unwrap().content,
            "Private body"
        );
        assert!(resolve(&mut store, &host("other", &id), &id).is_err());
        let mut unowned = template(&id);
        unowned.as_object_mut().unwrap().remove("accessOwnerID");
        put(&mut store, unowned, 2);
        let before = store.product_setting("prompt.templates").unwrap().unwrap();
        assert!(resolve(&mut store, &host("owner", &id), &id).is_err());
        let after = store.product_setting("prompt.templates").unwrap().unwrap();
        assert_eq!(before.value_json, after.value_json);
        assert_eq!(before.revision, after.revision);
    }
    #[test]
    fn actual_grants_are_content_revision_digest_time_and_host_scoped() {
        let mut store = nola_store::NolaStore::open(":memory:").unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let mut body = template(&id);
        let digest = format!("sha256:{:x}", Sha256::digest(b"Private body"));
        body["accessGrants"] = json!([{"grant_id":"grant","resource_kind":"prompt_template","resource_id":id,
            "principal":{"kind":"local_user","id":"other"},"permissions":["read_content"],
            "resource_revision":4,"content_digest":digest,"expires_at_epoch_ms":101}]);
        put(&mut store, body.clone(), 1);
        assert!(resolve(&mut store, &host("other", &id), &id).is_ok());
        for (field, value) in [
            ("permissions", json!(["read_metadata"])),
            ("resource_revision", json!(5)),
            ("content_digest", json!("sha256:wrong")),
            ("expires_at_epoch_ms", json!(99)),
            ("host_id", json!("other-host")),
        ] {
            let mut changed = body.clone();
            changed["accessGrants"][0][field] = value;
            put(&mut store, changed, 2);
            assert!(
                resolve(&mut store, &host("other", &id), &id).is_err(),
                "{field}"
            );
        }
        let mut revoked = body;
        revoked["accessGrants"] = json!([]);
        put(&mut store, revoked, 3);
        assert!(resolve(&mut store, &host("other", &id), &id).is_err());
    }
    #[test]
    fn host_default_is_frozen_but_explicit_missing_and_ambiguous_definitions_reject() {
        let mut store = nola_store::NolaStore::open(":memory:").unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        put(&mut store, template(&id), 1);
        let mut authority = host("owner", &id);
        authority.selections[0].source = HostExpertTemplateSelectionSource::HostDefault;
        let catalog = load_custom_catalog(&store.product_setting_repository()).unwrap();
        let resolved =
            resolve_selection(&authority, "context", 3, &json!({}), catalog.as_ref(), 100).unwrap();
        assert_eq!(resolved.template_id.as_deref(), Some(id.as_str()));
        assert!(
            resolve_selection(&authority, "context", 4, &json!({}), catalog.as_ref(), 100).is_err()
        );
        assert!(resolve_selection(
            &authority,
            "context",
            3,
            &json!({"selectedPromptTemplateID":id}),
            catalog.as_ref(),
            100
        )
        .is_err());
        authority.selections[0].source = HostExpertTemplateSelectionSource::Explicit;
        authority.builtins.push(HostExpertBuiltinDefinition {
            provider_id: "native-code".into(),
            template_id: id.clone(),
            content: "builtin".into(),
            platform: json!("ios"),
        });
        assert!(resolve_selection(
            &authority,
            "context",
            3,
            &json!({"selectedPromptTemplateID":id}),
            catalog.as_ref(),
            100
        )
        .is_err());
        authority.builtins.clear();
        assert!(resolve_selection(
            &authority,
            "context",
            3,
            &json!({"selectedPromptTemplateID":id}),
            None,
            100
        )
        .is_err());
        authority.selections[0].template_id = None;
        authority.selections[0].source = HostExpertTemplateSelectionSource::NoTemplate;
        authority.validate().unwrap();
        assert!(matches!(
            resolve_selection(&authority, "context", 3, &json!({}), catalog.as_ref(), 100)
                .unwrap()
                .origin,
            ExpertTemplateCaptureOrigin::NoTemplate
        ));
        assert!(resolve_selection(
            &authority,
            "context",
            3,
            &json!({"selectedPromptTemplateID":id}),
            catalog.as_ref(),
            100
        )
        .is_err());
    }
}
