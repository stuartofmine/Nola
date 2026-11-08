//! Application configuration wire entry delegates invariants to Domain.
pub use nola_store_port::conversation_profile::PatchConversationConfigurationInput;
pub use nola_domain::configuration_patch::{validate_patch, valid_model_identity};
use serde::{Deserialize, Serialize};

/// One selection intent; clients supply values, not mutually exclusive storage
/// fields. The resulting patch still uses the canonical configuration CAS.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectConversationExecutionInput {
    pub conversation_id: String,
    pub expected_revision: i64,
    pub selection: ConversationExecutionSelection,
    pub execution_host: Option<String>,
    pub execution_backend: Option<String>,
    pub external_execution_sessions: Option<serde_json::Map<String, serde_json::Value>>,
    /// Independent settings edited with a selection share its one CAS.
    /// Selection-owned fields cannot be smuggled back through this patch.
    pub additional_configuration: Option<nola_domain::configuration_patch::ConversationConfigurationPatch>,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConversationExecutionSelection {
    Inherit,
    Location {
        execution_host: String,
    },
    Model {
        model_id: String,
        parameters: serde_json::Map<String, serde_json::Value>,
        presentation: serde_json::Map<String, serde_json::Value>,
    },
    Host {
        binding: serde_json::Map<String, serde_json::Value>,
        parameters: serde_json::Map<String, serde_json::Value>,
        presentation: serde_json::Map<String, serde_json::Value>,
    },
}

impl SelectConversationExecutionInput {
    pub fn into_patch(self) -> Result<PatchConversationConfigurationInput, String> {
        use serde_json::json;
        let mut patch = match self.selection {
            ConversationExecutionSelection::Inherit => json!({
                "modelSelectionID": null, "executionHostBinding": null, "clientHostParameters": null,
            }),
            ConversationExecutionSelection::Location { execution_host } => {
                json!({"executionHostRawValue":execution_host})
            }
            ConversationExecutionSelection::Model { model_id, parameters, presentation } => {
                if !valid_model_identity(&model_id) {
                    return Err("invalid selected model identity".into());
                }
                json!({
                    "modelSelectionID": model_id,
                    "executionHostBinding": null,
                    "clientHostParameters": null,
                    "modelOverrides": {model_id.clone(): parameters},
                    "clientModelPresentation": {model_id: presentation},
                })
            }
            ConversationExecutionSelection::Host { binding, parameters, presentation } => json!({
                "modelSelectionID": null,
                "executionHostBinding": binding,
                "clientHostParameters": parameters,
                "clientModelPresentation": {"execution-host": presentation},
            }),
        };
        if let Some(host) = self.execution_host {
            if patch.get("executionHostRawValue").is_some() {
                return Err("execution location must have one source".into());
            }
            patch["executionHostRawValue"] = json!(host);
        }
        if let Some(backend) = self.execution_backend {
            patch["executionBackend"] = json!(backend);
        }
        if let Some(sessions) = self.external_execution_sessions {
            patch["externalExecutionSessions"] = json!(sessions);
        }
        if let Some(additional) = self.additional_configuration {
            for (key, value) in additional.fields() {
                if matches!(key.as_str(), "modelSelectionID" | "executionHostBinding" |
                    "clientHostParameters" | "clientModelPresentation" | "modelOverrides" | "executionHostRawValue")
                    || patch.get(key).is_some() {
                    return Err("additional configuration overlaps execution selection".into());
                }
                patch[key] = value.clone();
            }
        }
        Ok(PatchConversationConfigurationInput {
            conversation_id: self.conversation_id,
            expected_revision: self.expected_revision,
            patch_json: patch.to_string().parse()?,
            updated_at_ms: self.updated_at_ms,
        })
    }
}
#[cfg(test)]
use serde_json::Value;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn model_selection_and_presentation_share_one_strict_patch_contract() {
        assert!(validate_patch(&json!({"modelSelectionID":"configured:model", "clientModelPresentation":{
            "configured:model":{"showThinking":true,"streamOutput":false,"customSystemPrompt":"", "responseStyle":"concise"}
        }}).to_string()).is_ok());
        assert!(validate_patch(&json!({"clientModelPresentation":{"local:model":null}}).to_string()).is_ok());
        for patch in [
            json!({"modelSelectionID":"model"}),
            json!({"clientModelPresentation":{"configured:model":{"apiKey":"secret"}}}),
            json!({"clientModelPresentation":{"configured:model":{"showThinking":"true"}}}),
            json!({"clientModelPresentation":{"unscoped":{"responseStyle":"concise"}}}),
        ] { assert!(validate_patch(&patch.to_string()).is_err(), "{patch}"); }
    }
    #[test]
    fn host_binding_is_credential_free_and_parameters_use_shared_contract() {
        for host in [json!("local"), json!("remote"), Value::Null] {
            assert!(validate_patch(&json!({"executionHostRawValue":host}).to_string()).is_ok());
        }
        for host in [json!("auto"), json!(false), json!({"host":"local"})] {
            assert!(validate_patch(&json!({"executionHostRawValue":host}).to_string()).is_err());
        }
        assert!(validate_patch(&json!({"modelSelectionID":null,"executionHostBinding":{"baseURL":"https://host.example"},"clientHostParameters":{"temperature":0.7,"seed":{"kind":"clear"}},"clientModelPresentation":{"execution-host":{"showThinking":true}}}).to_string()).is_ok());
        for url in ["file:///tmp", "https://user:password@host", "https://host?token=secret", "https://host#secret"] {
            assert!(validate_patch(&json!({"executionHostBinding":{"baseURL":url}}).to_string()).is_err());
        }
        for patch in [json!({"executionHostBinding":{"baseURL":"https://host","apiKey":"secret"}}),
            json!({"executionHostBinding":{"baseURL":"https://host","tlsCertificateSHA256":"bad"}}),
            json!({"executionHostBinding":{"baseURL":"https://host"},"modelSelectionID":"local:model"}),
            json!({"clientHostParameters":{"seed":42}}), json!({"clientHostParameters":{"apiKey":"secret"}})] {
            assert!(validate_patch(&patch.to_string()).is_err(), "{patch}");
        }
    }
    use serde_json::json;
    #[test]
    fn seed_override_accepts_only_tagged_intent_and_patch_reset() {
        for seed in [
            Value::Null,
            json!({"kind":"clear"}),
            json!({"kind":"value","value":"0"}),
            json!({"kind":"value","value":i64::MIN.to_string()}),
            json!({"kind":"value","value":i64::MAX.to_string()}),
        ] {
            assert!(validate_patch(
                &json!({"modelOverrides":{"configured:m":{"seed":seed}}}).to_string()
            )
            .is_ok());
        }
        for seed in [
            json!(0),
            json!("0"),
            json!({"kind":"value","value":0}),
            json!({"kind":"value","value":"01"}),
            json!({"kind":"clear","value":"0"}),
        ] {
            assert!(validate_patch(
                &json!({"modelOverrides":{"configured:m":{"seed":seed}}}).to_string()
            )
            .is_err());
        }
    }
    #[test]
    fn connected_host_identity_is_scoped_and_canonical() {
        let key = "connectedHost:aG9zdA:cHJvZmlsZQ";
        assert!(validate_patch(
            &json!({"modelOverrides":{key:{"temperature":0,"seed":{"kind":"value","value":"0"}}}})
                .to_string()
        )
        .is_ok());
        for invalid in [
            "connectedHost:host",
            "connectedHost::cHJvZmlsZQ",
            "connectedHost:aG9zdA==:cHJvZmlsZQ",
            "connectedHost:a:cHJvZmlsZQ",
            "connectedHost:aG9zdA:cHJvZmlsZQ:extra",
        ] {
            assert!(!valid_model_identity(invalid), "{invalid}");
        }
        assert!(validate_patch(
            &json!({"modelOverrides":{key:{"wireProtocol":"responses"}}}).to_string()
        )
        .is_err());
    }
    #[test]
    fn knowledge_patch_uses_native_fields_without_coercion_or_client_timestamps() {
        for knowledge in [json!({"keyPointsMarkdown":"  fact\n"}), json!({"wikiMarkdown":""}),
            json!({"keyPointsMarkdown":"事实", "wikiMarkdown":"# Wiki\n"})] {
            assert!(validate_patch(&json!({"knowledge":knowledge}).to_string()).is_ok());
        }
        for knowledge in [Value::Null, json!({}), json!("text"), json!({"wikiMarkdown":null}),
            json!({"keyPointsMarkdown":1}), json!({"knowledgePoints":"alias"}),
            json!({"wikiMarkdown":"ok", "updatedAt":1}), json!({"wikiMarkdown":"ok", "secret":"no"})] {
            assert!(validate_patch(&json!({"knowledge":knowledge}).to_string()).is_err());
        }
    }

    #[test]
    fn sparse_configuration_contract_rejects_unknown_fields_without_coercion() {
        for value in [json!(true), json!(false), Value::Null] {
            assert!(validate_patch(&json!({"memoryEnabledOverride":value}).to_string()).is_ok());
            assert!(validate_patch(&json!({"retrievalEnabledOverride":value}).to_string()).is_ok());
        }
        assert!(validate_patch(&json!({"isPinned":true}).to_string()).is_ok());
        assert!(validate_patch(&json!({"isPinned":false}).to_string()).is_ok());
        assert!(validate_patch(&json!({"modelOverrides":{"configured:m":{"temperature":0,"seed":{"kind":"value","value":"0"},"topP":null}},"permissionProfile":null}).to_string()).is_ok());
        for patch in [
            json!({"memoryEnabledOverride":"true"}),
            json!({"memoryEnabledOverride":1}),
            json!({"retrievalEnabledOverride":"false"}),
            json!({"retrievalEnabledOverride":0}),
            json!({"isPinned":"true"}),
            json!({"isPinned":null}),
            json!({"permissionProfile":"moderate"}),
            json!({"permissionProfile":true}),
            json!({"credentials":"secret"}),
            json!({"modelOverrides":{"remote:m":{}}}),
            json!({"modelOverrides":{"configured:m":{"temperature":3}}}),
            json!({"modelOverrides":{"configured:m":{"topK":0}}}),
            json!({"modelOverrides":{"configured:m":{"topK":1.5}}}),
            json!({"modelOverrides":{"configured:m":{"presencePenalty":3}}}),
            json!({"modelOverrides":{"configured:m":{"reasoning":{"control":"automatic","budgets":[-1],"efforts":[]}}}}),
        ] {
            assert!(validate_patch(&patch.to_string()).is_err(), "{patch}");
        }
    }
}
