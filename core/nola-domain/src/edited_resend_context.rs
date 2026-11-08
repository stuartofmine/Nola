// SPDX-License-Identifier: Apache-2.0
//! One frozen-context contract shared by trusted Host attestation and Store consumption.
//! Callers project the existing typed ContextComponent; no parallel runtime model.
use crate::{
    edited_resend::{canonical_resend_value_sha256, PreparedEditedResend},
    expert_message::ExpertPromptDefinition,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub fn validate_items(
    prepared: &PreparedEditedResend,
    items: &Value,
    context_items: &Value,
) -> Result<(), String> {
    let values = items
        .as_array()
        .ok_or("edited_resend_user_items_required")?;
    if values.len() != 1 || !context_items.as_array().is_some_and(Vec::is_empty) {
        return Err("edited_resend_only_edited_user_message_allowed".into());
    }
    let message = &values[0];
    if message.get("type").and_then(Value::as_str) != Some("message")
        || message.get("role").and_then(Value::as_str) != Some("user")
        || message.get("id").and_then(Value::as_str) != Some(prepared.user_message_id.as_str())
        || message.get("content").and_then(Value::as_str)
            != Some(prepared.input.edited_text.as_str())
    {
        return Err("edited_resend_user_message_differs_from_prepared_target".into());
    }
    Ok(())
}

pub fn validate_components(
    expected: &[ExpertPromptDefinition],
    components: &Value,
) -> Result<String, String> {
    let values = components
        .as_array()
        .ok_or("edited_resend_typed_components_required")?;
    let mut ids = std::collections::BTreeSet::new();
    let mut templates = std::collections::BTreeSet::new();
    for component in values {
        let object = component
            .as_object()
            .ok_or("edited_resend_invalid_component")?;
        if object
            .keys()
            .any(|key| !matches!(key.as_str(), "kind" | "id" | "content" | "source"))
        {
            return Err("edited_resend_unknown_component_field".into());
        }
        let id = component
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty())
            .ok_or("edited_resend_component_identity_required")?;
        if !ids.insert(id) {
            return Err("edited_resend_duplicate_component_identity".into());
        }
        let kind = component
            .get("kind")
            .and_then(Value::as_str)
            .ok_or("edited_resend_component_kind_required")?;
        if kind == "prompt_template" {
            let definition = expected
                .iter()
                .find(|item| item.component_id == id)
                .ok_or("edited_resend_unknown_original_prompt_definition")?;
            if component.get("content").and_then(Value::as_str) != Some(definition.content.as_str())
                || format!("{:x}", Sha256::digest(definition.content.as_bytes()))
                    != definition.content_sha256
            {
                return Err("edited_resend_original_prompt_body_changed".into());
            }
            templates.insert(id);
        } else if !matches!(kind, "system" | "skill" | "ui_client_data") {
            return Err("edited_resend_resource_or_history_context_forbidden".into());
        }
        if component
            .get("source")
            .filter(|value| !value.is_null())
            .is_some_and(|source| {
                source.as_object().is_none_or(|fields| {
                    fields
                        .keys()
                        .any(|key| !matches!(key.as_str(), "platform" | "title" | "mime_type"))
                })
            })
        {
            return Err("edited_resend_context_must_not_restore_resource_evidence".into());
        }
    }
    if templates.len() != expected.len() {
        return Err("edited_resend_original_prompt_set_incomplete".into());
    }
    canonical_resend_value_sha256(components)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn fresh_guidance_is_bound_but_resource_and_missing_definition_context_are_rejected() {
        let guidance = json!([{"kind":"system","id":"fresh-host","content":"Current guidance","source":{"platform":"shared","title":"Host"}}]);
        let digest = validate_components(&[], &guidance).unwrap();
        let mut changed = guidance.clone();
        changed[0]["content"] = json!("Changed after attest");
        assert_ne!(digest, validate_components(&[], &changed).unwrap());
        for kind in [
            "memory",
            "session_memory",
            "document",
            "attachment",
            "platform_data",
            "web_retrieval",
            "project_instruction",
        ] {
            let mut resource = guidance.clone();
            resource[0]["kind"] = json!(kind);
            assert!(validate_components(&[], &resource).is_err());
        }
        for field in ["uri", "authorization", "observed_at", "maximum_age_minutes"] {
            let mut evidence = guidance.clone();
            evidence[0]["source"][field] = json!("old-owner");
            assert!(validate_components(&[], &evidence).is_err());
        }
        let original = ExpertPromptDefinition {
            component_id: "original-template".into(),
            content: "Original".into(),
            content_sha256: format!("{:x}", Sha256::digest(b"Original")),
            definition_revision: Some(3),
            platform: Value::Null,
        };
        assert!(validate_components(std::slice::from_ref(&original), &guidance).is_err());
        let mut complete = guidance;
        complete.as_array_mut().unwrap().push(
            json!({"kind":"prompt_template","id":original.component_id,"content":original.content}),
        );
        assert!(validate_components(&[original], &complete).is_ok());
    }
}
