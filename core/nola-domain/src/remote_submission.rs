// SPDX-License-Identifier: Apache-2.0
//! Remote submission intent and source admission are distinct durable facts.
use crate::run_message_admission::RunMessageAdmissionCursor;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Read-only observation used before freezing a new remote submission.
/// A conversation with execution history requires an admitted source owner.
/// An explicitly configured, unstarted local conversation has no Run owner yet.
/// This never rebases a bound request or changes its expected preconditions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmissionFrontierRequest {
    pub expected_host_id: String,
    pub client_id: String,
    pub conversation_id: String,
    pub expected_run_id: Option<String>,
    pub assistant_message_id: Option<String>,
}
impl SubmissionFrontierRequest {
    pub fn validate(&self, source_host_id: &str) -> Result<(), String> {
        if self.expected_host_id != source_host_id
            || self.expected_run_id.is_some() != self.assistant_message_id.is_some()
            || [&self.expected_host_id, &self.client_id, &self.conversation_id].into_iter()
                .chain(self.expected_run_id.iter()).chain(self.assistant_message_id.iter())
                .any(|id| id.is_empty() || id.trim() != id || id.len() > 512 || id.chars().any(char::is_control)) {
            return Err("invalid submission frontier owner".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmissionFrontier {
    pub protocol_version: u32,
    pub source_host_id: String,
    pub client_id: String,
    pub conversation_id: String,
    pub configuration_revision: Option<i64>,
    pub parent_message_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteSubmissionIdentity {
    pub source_host_id: String,
    pub client_id: String,
    pub conversation_id: String,
    pub submission_id: String,
    pub user_message_id: String,
    pub assistant_message_id: String,
}
impl RemoteSubmissionIdentity {
    pub fn validate(&self) -> Result<(), String> {
        for value in [
            &self.source_host_id,
            &self.client_id,
            &self.conversation_id,
            &self.submission_id,
            &self.user_message_id,
            &self.assistant_message_id,
        ] {
            if value.trim().is_empty() || value.len() > 512 {
                return Err("invalid remote submission identity".into());
            }
        }
        if self.user_message_id == self.assistant_message_id {
            return Err("submission message identities collide".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteSubmissionPhase {
    Prepared,
    Bound,
    Admitted,
    Rejected,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteSubmissionIntentView {
    pub identity: RemoteSubmissionIdentity,
    pub request_digest: Option<String>,
    pub phase: RemoteSubmissionPhase,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceSubmissionReceipt {
    pub protocol_version: u16,
    pub identity: RemoteSubmissionIdentity,
    pub request_digest: String,
    pub phase: RemoteSubmissionPhase,
    pub admission: Option<RunMessageAdmissionCursor>,
    pub rejection_code: Option<String>,
}
impl SourceSubmissionReceipt {
    pub fn validate(&self) -> Result<(), String> {
        self.identity.validate()?;
        validate_request_digest(&self.request_digest)?;
        if self.protocol_version != 1 || self.phase == RemoteSubmissionPhase::Bound {
            return Err("invalid source submission receipt version or phase".into());
        }
        match (&self.admission, self.phase) {
            (Some(cursor), RemoteSubmissionPhase::Admitted)
                if cursor.sequence.0 > 0
                    && cursor.identity.conversation_id == self.identity.conversation_id
                    && cursor.identity.run_id == self.identity.submission_id
                    && cursor.identity.assistant_message_id
                        == self.identity.assistant_message_id => {}
            (None, phase) if phase != RemoteSubmissionPhase::Admitted => {}
            _ => return Err("source receipt admission owner mismatch".into()),
        }
        if let Some(cursor) = &self.admission {
            cursor.identity.validate().map_err(|e| e.to_string())?;
        }
        if self.rejection_code.is_some() && self.phase != RemoteSubmissionPhase::Rejected {
            return Err("rejection code is only valid for rejected submissions".into());
        }
        if self.rejection_code.as_ref().is_some_and(|code| {
            code.is_empty()
                || code.len() > 128
                || !code
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        }) {
            return Err("invalid source rejection code".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisteredSourceSubmission {
    pub receipt: SourceSubmissionReceipt,
    pub created: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrepareRemoteSubmissionInput {
    pub source_host_id: String,
    pub client_id: String,
    pub conversation_id: String,
    pub conversation_title: String,
    pub submission_id: String,
    pub user_message_id: String,
    pub user_part_id: String,
    pub user_text: String,
    pub user_content_json: Option<String>,
    pub assistant_message_id: String,
    pub assistant_part_id: String,
    pub context_checkpoint_id: Option<String>,
    pub created_at_ms: i64,
}
impl PrepareRemoteSubmissionInput {
    pub fn identity(&self) -> RemoteSubmissionIdentity {
        RemoteSubmissionIdentity {
            source_host_id: self.source_host_id.clone(),
            client_id: self.client_id.clone(),
            conversation_id: self.conversation_id.clone(),
            submission_id: self.submission_id.clone(),
            user_message_id: self.user_message_id.clone(),
            assistant_message_id: self.assistant_message_id.clone(),
        }
    }
}
pub fn validate_request_digest(digest: &str) -> Result<(), String> {
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("invalid submission request digest".into());
    }
    Ok(())
}
/// Canonical UTF-8 JSON uses recursively sorted object keys, preserving arrays
/// and JSON number values. The transport's digest field is never semantic input.
pub fn canonical_submission_request(request: &Value) -> Result<String, String> {
    if !request.is_object() || request.get("request_digest").is_some() {
        return Err("submission request must be an object without request_digest".into());
    }
    fn sorted(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let ordered: std::collections::BTreeMap<_, _> = map.iter().collect();
                Value::Object(
                    ordered
                        .into_iter()
                        .map(|(key, value)| (key.clone(), sorted(value)))
                        .collect(),
                )
            }
            Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
            other => other.clone(),
        }
    }
    let json = serde_json::to_string(&sorted(request)).map_err(|e| e.to_string())?;
    if json.len() > 16 * 1024 * 1024 {
        return Err("submission request exceeds frozen payload budget".into());
    }
    Ok(json)
}
pub fn submission_request_digest(request: &Value) -> Result<String, String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(canonical_submission_request(request)?.as_bytes())
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn digest_is_recursive_semantic_object_order_but_preserves_array_order_and_unicode() {
        let a: Value =
            serde_json::from_str(r#"{"z":["中文🙂",1],"a":{"second":null,"first":0.25}}"#).unwrap();
        let b: Value =
            serde_json::from_str(r#"{"a":{"first":0.25,"second":null},"z":["中文🙂",1]}"#).unwrap();
        assert_eq!(
            submission_request_digest(&a).unwrap(),
            submission_request_digest(&b).unwrap()
        );
        let mut changed = b.clone();
        changed["z"] = serde_json::json!([1, "中文🙂"]);
        assert_ne!(
            submission_request_digest(&a).unwrap(),
            submission_request_digest(&changed).unwrap()
        );
        changed["request_digest"] = serde_json::json!("must not hash transport metadata");
        assert!(submission_request_digest(&changed).is_err());
        assert!(submission_request_digest(&Value::Null).is_err());
    }
}
