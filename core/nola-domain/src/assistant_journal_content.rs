// SPDX-License-Identifier: Apache-2.0
//! Versioned dynamic body content, separate from canonical journal operations.
//! Digests cover the exact immutable UTF-8 chunk bytes. Consumers must retain
//! those bytes; parsing/re-encoding JSON is never a digest verification method.
use crate::assistant_journal_operations::JournalAffectedResource;
use crate::{ArtifactReference, SourceEvidence, StructuredValue};
use serde::{Deserialize, Serialize};

pub const ASSISTANT_STRUCTURED_CONTENT_VERSION: u16 = 1;
const MAX_STRUCTURED_DEPTH: usize = 64;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantStructuredContentV1 {
    pub protocol_version: u16,
    pub value: StructuredValue,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantResourceContentV1 {
    pub protocol_version: u16,
    pub resources: Vec<JournalAffectedResource>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantArtifactContentV1 {
    pub protocol_version: u16,
    pub artifacts: Vec<ArtifactReference>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantSourceEvidenceContentV1 {
    pub protocol_version: u16,
    pub evidence: SourceEvidence,
}

/// Serialize the borrowed canonical JSON only at a complete-field boundary.
/// No intermediate Value clone or hydrated MessagePartView is required.
pub fn encode_assistant_json_content(value: &serde_json::Value) -> Result<String, String> {
    let mut pending = vec![(value, 0usize)];
    while let Some((value, depth)) = pending.pop() {
        if depth > MAX_STRUCTURED_DEPTH {
            return Err("structured content exceeds maximum depth".into());
        }
        match value {
            serde_json::Value::Array(values) => {
                pending.extend(values.iter().map(|value| (value, depth + 1)))
            }
            serde_json::Value::Object(values) => {
                pending.extend(values.values().map(|value| (value, depth + 1)))
            }
            _ => {}
        }
    }
    #[derive(Serialize)]
    struct Borrowed<'a> {
        protocol_version: u16,
        value: &'a serde_json::Value,
    }
    serde_json::to_string(&Borrowed {
        protocol_version: 1,
        value,
    })
    .map_err(|error| error.to_string())
}
pub fn encode_assistant_artifact_content(
    artifacts: &[ArtifactReference],
) -> Result<String, String> {
    #[derive(Serialize)]
    struct Borrowed<'a> {
        protocol_version: u16,
        artifacts: &'a [ArtifactReference],
    }
    serde_json::to_string(&Borrowed {
        protocol_version: 1,
        artifacts,
    })
    .map_err(|error| error.to_string())
}
pub fn decode_assistant_artifact_content(text: &str) -> Result<Vec<ArtifactReference>, String> {
    let content: AssistantArtifactContentV1 =
        serde_json::from_str(text).map_err(|error| error.to_string())?;
    if content.protocol_version != 1 {
        return Err("unsupported artifact content version".into());
    }
    Ok(content.artifacts)
}
pub fn encode_assistant_source_evidence_content(
    evidence: &SourceEvidence,
) -> Result<String, String> {
    #[derive(Serialize)]
    struct Borrowed<'a> {
        protocol_version: u16,
        evidence: &'a SourceEvidence,
    }
    serde_json::to_string(&Borrowed {
        protocol_version: 1,
        evidence,
    })
    .map_err(|error| error.to_string())
}
pub fn decode_assistant_source_evidence_content(text: &str) -> Result<SourceEvidence, String> {
    let content: AssistantSourceEvidenceContentV1 =
        serde_json::from_str(text).map_err(|error| error.to_string())?;
    if content.protocol_version != 1 {
        return Err("unsupported source evidence content version".into());
    }
    Ok(content.evidence)
}

fn validate_value(value: &StructuredValue) -> Result<(), String> {
    let mut pending = vec![(value, 0usize)];
    while let Some((value, depth)) = pending.pop() {
        if depth > MAX_STRUCTURED_DEPTH {
            return Err("structured content exceeds maximum depth".into());
        }
        match value {
            StructuredValue::Array(values) => pending.extend(values.iter().map(|v| (v, depth + 1))),
            StructuredValue::Object(values) => {
                pending.extend(values.values().map(|v| (v, depth + 1)))
            }
            _ => {}
        }
    }
    Ok(())
}

/// Objects use StructuredValue's ordered map. Numbers retain serde_json's
/// i64/u64/finite-f64 value and emitted decimal token, including u64::MAX.
/// Web decoders must use lossless JSON numeric tokens, not Number rounding.
pub fn encode_assistant_structured_content(value: &StructuredValue) -> Result<String, String> {
    validate_value(value)?;
    #[derive(Serialize)]
    struct Borrowed<'a> {
        protocol_version: u16,
        value: &'a StructuredValue,
    }
    serde_json::to_string(&Borrowed {
        protocol_version: 1,
        value,
    })
    .map_err(|e| e.to_string())
}
pub fn decode_assistant_structured_content(text: &str) -> Result<StructuredValue, String> {
    let content: AssistantStructuredContentV1 =
        serde_json::from_str(text).map_err(|e| e.to_string())?;
    if content.protocol_version != ASSISTANT_STRUCTURED_CONTENT_VERSION {
        return Err("unsupported structured content version".into());
    }
    validate_value(&content.value)?;
    Ok(content.value)
}
pub fn encode_assistant_resource_content(
    resources: &[JournalAffectedResource],
) -> Result<String, String> {
    #[derive(Serialize)]
    struct Borrowed<'a> {
        protocol_version: u16,
        resources: &'a [JournalAffectedResource],
    }
    serde_json::to_string(&Borrowed {
        protocol_version: 1,
        resources,
    })
    .map_err(|e| e.to_string())
}
pub fn decode_assistant_resource_content(
    text: &str,
) -> Result<Vec<JournalAffectedResource>, String> {
    let content: AssistantResourceContentV1 =
        serde_json::from_str(text).map_err(|e| e.to_string())?;
    if content.protocol_version != ASSISTANT_STRUCTURED_CONTENT_VERSION {
        return Err("unsupported resource content version".into());
    }
    Ok(content.resources)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn structured_scalars_objects_arrays_and_large_numbers_roundtrip_without_stringification() {
        for json in [
            "null",
            "true",
            "18446744073709551615",
            "-9223372036854775808",
            "1.25",
            "\"汉字\\n\"",
            "[null,true,1,{\"z\":2,\"a\":3}]",
        ] {
            let value: StructuredValue = serde_json::from_str(json).unwrap();
            let wire = encode_assistant_structured_content(&value).unwrap();
            assert_eq!(decode_assistant_structured_content(&wire).unwrap(), value);
        }
        let value: StructuredValue =
            serde_json::from_str("{\"z\":18446744073709551615,\"a\":true}").unwrap();
        assert_eq!(
            encode_assistant_structured_content(&value).unwrap(),
            "{\"protocol_version\":1,\"value\":{\"a\":true,\"z\":18446744073709551615}}"
        );
        assert!(
            decode_assistant_structured_content("{\"protocol_version\":2,\"value\":null}").is_err()
        );
        assert!(decode_assistant_structured_content(
            "{\"protocol_version\":1,\"value\":null,\"extra\":0}"
        )
        .is_err());
    }
    #[test]
    fn dynamic_resource_names_are_not_metadata_truncated() {
        let resources = vec![
            JournalAffectedResource {
                resource_type: "file".into(),
                uri: "u".repeat(10_000),
                operation: "write".into()
            };
            200
        ];
        assert_eq!(
            decode_assistant_resource_content(
                &encode_assistant_resource_content(&resources).unwrap()
            )
            .unwrap(),
            resources
        );
    }

    #[test]
    fn borrowed_json_encoder_retains_null_empty_and_numeric_values_without_value_clone() {
        for value in [
            serde_json::json!(null),
            serde_json::json!(""),
            serde_json::json!([]),
            serde_json::json!({}),
            serde_json::json!({"max": u64::MAX, "min": i64::MIN}),
        ] {
            let wire = encode_assistant_json_content(&value).unwrap();
            let decoded = decode_assistant_structured_content(&wire).unwrap();
            assert_eq!(serde_json::to_value(decoded).unwrap(), value);
        }
    }

    #[test]
    fn typed_artifacts_retain_optional_empty_fields_and_reject_imported_authority() {
        let artifacts = vec![
            ArtifactReference {
                uri: "artifact-ref".into(),
                name: "原始名字🙂".repeat(600),
                mime_type: Some(String::new()),
                operation: None,
            },
            ArtifactReference {
                uri: "artifact-2".into(),
                name: String::new(),
                mime_type: None,
                operation: Some(String::new()),
            },
        ];
        let wire = encode_assistant_artifact_content(&artifacts).unwrap();
        assert_eq!(decode_assistant_artifact_content(&wire).unwrap(), artifacts);
        let original: serde_json::Value = serde_json::from_str(&wire).unwrap();
        for field in ["grant", "lease", "token", "raw_input", "full_output_ref"] {
            let mut invalid = original.clone();
            invalid["artifacts"][0][field] = "forged".into();
            assert!(decode_assistant_artifact_content(&invalid.to_string()).is_err());
        }
    }

    #[test]
    fn typed_memory_evidence_retains_redacted_facts_and_rejects_body_or_credential() {
        let evidence = SourceEvidence::Memory {
            usage: crate::MemoryUsage {
                record_id: "record".into(),
                component_id: "component".into(),
                usage_count: u64::MAX,
                last_used_at_epoch_ms: Some(i64::MIN),
                title: Some("脱敏证据🙂".repeat(600)),
                source: Some(String::new()),
                truncated: true,
                requires_current_state_verification: true,
                ..Default::default()
            },
        };
        let wire = encode_assistant_source_evidence_content(&evidence).unwrap();
        assert_eq!(
            decode_assistant_source_evidence_content(&wire).unwrap(),
            evidence
        );
        let original: serde_json::Value = serde_json::from_str(&wire).unwrap();
        for field in ["content", "summary", "credential", "grant", "lease"] {
            let mut invalid = original.clone();
            invalid["evidence"]["usage"][field] = "private".into();
            assert!(decode_assistant_source_evidence_content(&invalid.to_string()).is_err());
        }
    }
}
