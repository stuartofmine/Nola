// SPDX-License-Identifier: Apache-2.0
//! Bounded conversation metadata, never a substitute for canonical content.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "MetadataFields", into = "MetadataFields")]
pub struct ConversationMetadataReplica {
    fields: MetadataFields,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MetadataFields {
    conversation_id: String,
    metadata_revision: u64,
    base_content_revision: u64,
    title: String,
    settings_json: Option<String>,
    updated_at_ms: i64,
}

impl ConversationMetadataReplica {
    pub fn new(
        conversation_id: String,
        metadata_revision: u64,
        base_content_revision: u64,
        title: String,
        settings_json: Option<String>,
        updated_at_ms: i64,
    ) -> Result<Self, String> {
        MetadataFields {
            conversation_id,
            metadata_revision,
            base_content_revision,
            title,
            settings_json,
            updated_at_ms,
        }
        .try_into()
    }
    pub fn conversation_id(&self) -> &str {
        &self.fields.conversation_id
    }
    pub fn metadata_revision(&self) -> u64 {
        self.fields.metadata_revision
    }
    pub fn base_content_revision(&self) -> u64 {
        self.fields.base_content_revision
    }
    pub fn title(&self) -> &str {
        &self.fields.title
    }
    pub fn settings_json(&self) -> Option<&str> {
        self.fields.settings_json.as_deref()
    }
    pub fn updated_at_ms(&self) -> i64 {
        self.fields.updated_at_ms
    }
}

impl TryFrom<MetadataFields> for ConversationMetadataReplica {
    type Error = String;
    fn try_from(fields: MetadataFields) -> Result<Self, Self::Error> {
        let id = &fields.conversation_id;
        if id.is_empty() || id.trim() != id || id.len() > 128 || id.chars().any(char::is_control) {
            return Err("invalid metadata conversation identity".into());
        }
        if fields.metadata_revision == 0
            || fields.base_content_revision == 0
            || fields.base_content_revision > fields.metadata_revision
        {
            return Err(
                "metadata must depend on a nonzero content revision no newer than itself".into(),
            );
        }
        if fields.title.len() > 4096 {
            return Err("conversation metadata title exceeds 4096 bytes".into());
        }
        if let Some(settings) = &fields.settings_json {
            if settings.len() > 64 * 1024 {
                return Err("conversation metadata settings exceed 65536 bytes".into());
            }
            if !matches!(
                serde_json::from_str::<serde_json::Value>(settings),
                Ok(serde_json::Value::Object(_))
            ) {
                return Err("conversation metadata settings must be a JSON object".into());
            }
        }
        Ok(Self { fields })
    }
}

impl From<ConversationMetadataReplica> for MetadataFields {
    fn from(value: ConversationMetadataReplica) -> Self {
        value.fields
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn value() -> serde_json::Value {
        serde_json::json!({"conversation_id":"c","metadata_revision":3,"base_content_revision":2,
            "title":"Title","settings_json":"{}","updated_at_ms":1})
    }
    #[test]
    fn metadata_round_trips_without_any_transcript_field() {
        let metadata: ConversationMetadataReplica = serde_json::from_value(value()).unwrap();
        assert_eq!(serde_json::to_value(metadata).unwrap(), value());
        let mut input = value();
        input["messages"] = serde_json::json!([]);
        assert!(serde_json::from_value::<ConversationMetadataReplica>(input).is_err());
    }
    #[test]
    fn decoding_enforces_dependency_and_identity_constraints() {
        for (key, invalid) in [
            ("base_content_revision", serde_json::json!(0)),
            ("base_content_revision", serde_json::json!(4)),
            ("metadata_revision", serde_json::json!(0)),
            ("conversation_id", serde_json::json!(" c ")),
        ] {
            let mut input = value();
            input[key] = invalid;
            assert!(serde_json::from_value::<ConversationMetadataReplica>(input).is_err());
        }
    }
    #[test]
    fn settings_are_bounded_objects_not_arbitrary_content() {
        for settings in [
            "[]".to_owned(),
            "null".to_owned(),
            "{".to_owned(),
            "x".repeat(65537),
        ] {
            assert!(ConversationMetadataReplica::new(
                "c".into(),
                2,
                1,
                "T".into(),
                Some(settings),
                1
            )
            .is_err());
        }
        assert!(
            ConversationMetadataReplica::new("c".into(), 2, 1, "x".repeat(4097), None, 1).is_err()
        );
    }
}
