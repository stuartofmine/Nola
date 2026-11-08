// SPDX-License-Identifier: Apache-2.0
//! Permission lifecycle evidence, independent of the user's authorization decision.
//! Structural validation never verifies provenance or grants execution authority.
use crate::assistant_journal::AssistantDigest;
use crate::assistant_journal_operations::JournalNumber;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionRequestClosure {
    pub closure_id: String,
    pub request_id: String,
    pub task_id: String,
    pub child_session_id: String,
    pub closed_at_ms: i64,
    pub source: PermissionClosureSource,
}

/// Every bound source names the original permission occurrence and effect.
/// A disconnected Host, elapsed timeout, or UI dismissal is not a source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PermissionClosureSource {
    CoreSettled {
        permission: CorePermissionOccurrence,
        event_id: String,
        /// Canonical Run event sequence, not a Trace or Core cursor.
        event_cursor: JournalNumber,
    },
    RunSealed {
        permission: CorePermissionOccurrence,
        assistant_message_id: String,
        seal_digest: AssistantDigest,
    },
    EffectWithdrawn {
        permission: CorePermissionOccurrence,
        event_cursor: JournalNumber,
    },
    UnboundTaskTerminal {
        /// Closes this Task's permission surface, not an alleged Core effect.
        /// Store must prove absence of a Run binding and the exact terminal
        /// Task receipt; a request's optional Core ID proves neither.
        terminal_receipt_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorePermissionOccurrence {
    pub run_id: String,
    pub core_permission_request_id: String,
    pub effect_id: String,
}

/// Captured by the Store while the original pending effect and current Core
/// checkpoint witness are available. Retained with the immutable suspension;
/// delivery-queue cleanup must not erase this historical source reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorePermissionSourceProof {
    pub permission: CorePermissionOccurrence,
    pub host_request_id: String,
    pub payload_digest: AssistantDigest,
    pub checkpoint_revision: JournalNumber,
    pub event_cursor: JournalNumber,
}

/// Borrowed facts from the original request, not a second persistent projection.
pub struct PermissionClosureRequest<'a> {
    pub request_id: &'a str,
    pub task_id: &'a str,
    pub child_session_id: &'a str,
    pub core_permission_request_id: Option<&'a str>,
    pub created_at_ms: i64,
    pub resolved_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionClosureError {
    InvalidIdentity,
    IdentityMismatch,
    InvalidChronology,
    InvalidSource,
}

impl std::fmt::Display for PermissionClosureError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{self:?}")
    }
}
impl std::error::Error for PermissionClosureError {}

impl PermissionRequestClosure {
    /// Check shape, original request identity and ordering. The caller must still
    /// verify the binding, effect ownership and committed source in the same
    /// transaction that closes the request and revokes its derived grants.
    pub fn validate_for(&self, request: &PermissionClosureRequest<'_>) -> Result<(), PermissionClosureError> {
        let nonempty = |value: &str| !value.trim().is_empty();
        if [&self.closure_id, &self.request_id, &self.task_id, &self.child_session_id]
            .into_iter().any(|id| !nonempty(id)) {
            return Err(PermissionClosureError::InvalidIdentity);
        }
        if self.request_id != request.request_id || self.task_id != request.task_id
            || self.child_session_id != request.child_session_id {
            return Err(PermissionClosureError::IdentityMismatch);
        }
        if request.created_at_ms < 0 || self.closed_at_ms < request.created_at_ms
            || request.resolved_at_ms.is_some_and(|at| at < request.created_at_ms || at > self.closed_at_ms) {
            return Err(PermissionClosureError::InvalidChronology);
        }
        let permission = match &self.source {
            PermissionClosureSource::CoreSettled { permission, event_id, event_cursor } => {
                if !nonempty(event_id) || event_cursor.0 == 0 {
                    return Err(PermissionClosureError::InvalidSource);
                }
                permission
            }
            PermissionClosureSource::RunSealed { permission, assistant_message_id, .. } => {
                if !nonempty(assistant_message_id) { return Err(PermissionClosureError::InvalidIdentity); }
                permission
            }
            PermissionClosureSource::EffectWithdrawn { permission, event_cursor } => {
                if event_cursor.0 == 0 { return Err(PermissionClosureError::InvalidSource); }
                permission
            }
            PermissionClosureSource::UnboundTaskTerminal { terminal_receipt_id } => {
                if !nonempty(terminal_receipt_id) { return Err(PermissionClosureError::InvalidIdentity); }
                // Optional Core IDs are unverified request metadata here.
                // Whether a Task is bound is a transactional Store fact, not
                // something structural validation may infer from that field.
                return Ok(());
            }
        };
        if [&permission.run_id, &permission.core_permission_request_id, &permission.effect_id]
            .into_iter().any(|id| !nonempty(id)) {
            return Err(PermissionClosureError::InvalidIdentity);
        }
        if request.core_permission_request_id != Some(permission.core_permission_request_id.as_str()) {
            return Err(PermissionClosureError::IdentityMismatch);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request() -> PermissionClosureRequest<'static> {
        PermissionClosureRequest { request_id: "request", task_id: "task", child_session_id: "child",
            core_permission_request_id: Some("core-permission"), created_at_ms: 10, resolved_at_ms: Some(20) }
    }
    fn closure() -> PermissionRequestClosure {
        PermissionRequestClosure { closure_id: "closure".into(), request_id: "request".into(),
            task_id: "task".into(), child_session_id: "child".into(), closed_at_ms: 30,
            source: PermissionClosureSource::RunSealed {
                permission: CorePermissionOccurrence { run_id: "run".into(), core_permission_request_id: "core-permission".into(), effect_id: "effect".into() },
                assistant_message_id: "assistant".into(), seal_digest: AssistantDigest::parse_hex(&"a".repeat(64)).unwrap(),
            } }
    }

    #[test]
    fn closure_preserves_decision_time_and_roundtrips_without_authorization() {
        let original = request();
        let value = closure();
        value.validate_for(&original).unwrap();
        let wire = serde_json::to_value(&value).unwrap();
        assert!(wire.get("decision").is_none());
        assert!(wire.get("resolved_at_ms").is_none());
        assert_eq!(serde_json::from_value::<PermissionRequestClosure>(wire).unwrap(), value);
        assert_eq!(original.resolved_at_ms, Some(20));
        let mut pending = request();
        pending.resolved_at_ms = None;
        value.validate_for(&pending).unwrap();
    }

    #[test]
    fn rejects_mismatched_request_and_impossible_chronology() {
        let value = closure();
        for field in 0..4 {
            let mut original = request();
            match field { 0 => original.request_id = "other", 1 => original.task_id = "other",
                2 => original.child_session_id = "other", _ => original.core_permission_request_id = Some("other") }
            assert_eq!(value.validate_for(&original), Err(PermissionClosureError::IdentityMismatch));
        }
        for (created, resolved) in [(-1, None), (31, None), (10, Some(9)), (10, Some(31))] {
            let mut original = request();
            original.created_at_ms = created;
            original.resolved_at_ms = resolved;
            assert_eq!(value.validate_for(&original), Err(PermissionClosureError::InvalidChronology));
        }
    }

    #[test]
    fn closure_source_does_not_infer_run_binding_from_optional_core_id() {
        let mut value = closure();
        let PermissionClosureSource::RunSealed { permission, .. } = value.source.clone() else { unreachable!() };
        value.source = PermissionClosureSource::EffectWithdrawn { permission, event_cursor: JournalNumber(0) };
        assert_eq!(value.validate_for(&request()), Err(PermissionClosureError::InvalidSource));
        value.source = PermissionClosureSource::UnboundTaskTerminal { terminal_receipt_id: "receipt".into() };
        value.validate_for(&request()).unwrap();
        let mut unbound = request();
        unbound.core_permission_request_id = None;
        value.validate_for(&unbound).unwrap();
        assert!(serde_json::from_value::<PermissionClosureSource>(json!({"kind":"host_disconnected"})).is_err());
    }

    #[test]
    fn wire_rejects_missing_evidence_unknown_fields_and_invalid_digest() {
        let wire = serde_json::to_value(closure()).unwrap();
        for field in ["closure_id", "request_id", "task_id", "child_session_id", "closed_at_ms", "source"] {
            let mut invalid = wire.clone();
            invalid.as_object_mut().unwrap().remove(field);
            assert!(serde_json::from_value::<PermissionRequestClosure>(invalid).is_err());
        }
        let mut invalid = wire.clone();
        invalid["decision"] = json!("deny");
        assert!(serde_json::from_value::<PermissionRequestClosure>(invalid).is_err());
        let mut invalid = wire;
        invalid["source"]["seal_digest"] = json!("not-evidence");
        assert!(serde_json::from_value::<PermissionRequestClosure>(invalid).is_err());
    }
}
