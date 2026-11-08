// SPDX-License-Identifier: Apache-2.0
//! Authority-scoped generative UI projections. These are internal product
//! contracts, not A2UI wire messages and not renderer-owned mutable state.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const A2UI_WIRE_VERSION: &str = "v0.9.1";
pub const UI_MAX_MESSAGE_BYTES: usize = 256 * 1024;
pub const UI_MAX_SURFACE_BYTES: usize = 1024 * 1024;
pub const UI_MAX_NODES: usize = 1000;
pub const UI_MAX_DEPTH: usize = 32;
pub const UI_MAX_ACTIVE_SURFACES: usize = 8;
pub const UI_MAX_ACTION_BYTES: usize = 64 * 1024;
/// Bounded semantic transcript content; component Text props retain full values.
// Canonical Part and journal carry the same summary; full component text remains in the Surface.
pub const UI_MAX_SUMMARY_BYTES: usize = crate::assistant_journal_operations::MAX_METADATA_STRING_BYTES;
/// Leaves room for derived message/receipt identities at every Host boundary.
pub const UI_MAX_SUBMISSION_ID_BYTES: usize = 128;

pub fn validate_ui_submission_id(id: &str) -> Result<(), UiError> {
    if id.is_empty() || id.len() > UI_MAX_SUBMISSION_ID_BYTES
        || id.trim() != id || id.chars().any(char::is_control) {
        return Err(UiError::InvalidIdentity);
    }
    Ok(())
}


/// Derived from an admitted execution, never accepted from model arguments.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiSurfaceOwner {
    pub authority_id: String,
    pub conversation_id: String,
    pub participant_id: String,
    pub context_id: String,
    pub originating_run_id: String,
    pub message_id: String,
    pub part_id: String,
}

impl UiSurfaceOwner {
    pub fn validate(&self) -> Result<(), UiError> {
        for identity in [
            &self.authority_id,
            &self.conversation_id,
            &self.participant_id,
            &self.context_id,
            &self.originating_run_id,
            &self.message_id,
            &self.part_id,
        ] {
            if identity.trim().is_empty() || identity.len() > 512 {
                return Err(UiError::InvalidIdentity);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiSurfaceIdentity {
    pub owner: UiSurfaceOwner,
    pub surface_id: String,
    pub generation: u64,
}

impl UiSurfaceIdentity {
    pub fn validate(&self) -> Result<(), UiError> {
        self.owner.validate()?;
        if self.surface_id.trim().is_empty()
            || self.surface_id.len() > 512
            || self.generation == 0
        {
            return Err(UiError::InvalidIdentity);
        }
        Ok(())
    }
}

/// Exact durable reference; the component tree stays in the Surface repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiSurfaceRevisionRef {
    pub identity: UiSurfaceIdentity,
    pub revision: u64,
}

impl UiSurfaceRevisionRef {
    pub fn validate(&self) -> Result<(), UiError> {
        self.identity.validate()?;
        if self.revision == 0 || self.revision > i64::MAX as u64 { return Err(UiError::InvalidIdentity); }
        Ok(())
    }
}

/// A verified user interaction retains model context separately from presentation.
/// This value is constructed by Core after action validation, never inferred from prose.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiActionContent {
    pub surface: UiSurfaceRevisionRef,
    pub envelope: serde_json::Value,
    pub summary: String,
}
impl UiActionContent {
    /// Read-only model projection of the admitted action. The committed edit
    /// revision lives outside the A2UI wire envelope and must not be lost when
    /// adapting that envelope into a user message. Never mutate the canonical
    /// input, or trust a same-named value supplied inside the envelope.
    pub fn model_envelope(&self) -> serde_json::Value {
        let mut projected = self.envelope.as_object().cloned().unwrap_or_default();
        projected.insert("surface_revision".into(), serde_json::json!(self.surface.revision));
        serde_json::Value::Object(projected)
    }

    pub fn validate(&self) -> Result<(), UiError> {
        self.surface.validate()?;
        if self.summary.is_empty() || self.summary.len() > UI_MAX_SUMMARY_BYTES
            || self.summary.chars().any(char::is_control)
            || self.envelope.get("type").and_then(serde_json::Value::as_str) != Some("ui_action")
            || self.envelope.get("surface") != Some(&serde_json::to_value(&self.surface.identity).map_err(|_| UiError::InvalidIdentity)?)
            || self.envelope.pointer("/message/version").and_then(serde_json::Value::as_str) != Some(A2UI_WIRE_VERSION)
            || self.envelope.pointer("/message/action/surfaceId").and_then(serde_json::Value::as_str) != Some(self.surface.identity.surface_id.as_str())
            || ["name", "sourceComponentId", "timestamp"].iter().any(|field|
                self.envelope.pointer(&format!("/message/action/{field}")).and_then(serde_json::Value::as_str).is_none_or(|value| value.trim().is_empty()))
            || self.envelope.to_string().len() > UI_MAX_ACTION_BYTES {
            return Err(UiError::InvalidIdentity);
        }
        Ok(())
    }
}

/// Typed journal payload, shared by model publications and user interactions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum UiSurfaceJournalEvent {
    UiSurfaceChanged { identity: UiSurfaceIdentity, revision: u64, phase: UiSurfacePhase },
}

impl UiSurfaceJournalEvent {
    pub fn changed(state: &UiSurfaceState) -> Result<Self, UiError> {
        UiSurfaceRevisionRef { identity: state.identity.clone(), revision: state.revision }.validate()?;
        Ok(Self::UiSurfaceChanged { identity: state.identity.clone(), revision: state.revision, phase: state.phase })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiSurfacePhase {
    Pending,
    Ready,
    ReadOnly,
    Deleted,
}

/// Clients echo this reference, never a supplied tool name or executable action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiActionRef {
    pub instance_id: String,
    pub action_id: String,
}

/// Values have already been resolved by Core. Binding paths remain opaque to
/// renderers and are only echoed when committing user edits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiNodeSnapshot {
    pub instance_id: String,
    pub component_id: String,
    pub component: String,
    pub props: BTreeMap<String, Value>,
    pub slots: BTreeMap<String, Vec<String>>,
    pub bindings: BTreeMap<String, String>,
    pub actions: BTreeMap<String, UiActionRef>,
    pub validation_errors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiSnapshot {
    pub identity: UiSurfaceIdentity,
    pub revision: u64,
    pub catalog_id: String,
    pub catalog_digest: String,
    pub phase: UiSurfacePhase,
    pub root_instance_id: Option<String>,
    pub nodes: BTreeMap<String, UiNodeSnapshot>,
    pub summary: String,
}

/// Delivered through the existing journal, whose envelope supplies the cursor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiChangeSet {
    pub identity: UiSurfaceIdentity,
    pub base_revision: u64,
    pub revision: u64,
    pub catalog_digest: String,
    pub phase: UiSurfacePhase,
    pub root_instance_id: Option<String>,
    pub upserted_nodes: Vec<UiNodeSnapshot>,
    pub removed_instance_ids: Vec<String>,
    pub summary: String,
}

/// A read response on the existing Application bridge. Snapshot explicitly
/// replaces a missing/incompatible baseline; Changes never guesses a baseline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum UiSurfaceUpdate {
    Snapshot { snapshot: UiSnapshot },
    Changes { changes: UiChangeSet },
    Unchanged { identity: UiSurfaceIdentity, revision: u64 },
    Missing { identity: UiSurfaceIdentity },
}

/// Null is a value. Deletion is a separate operation, including across FFI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum UiEdit {
    Set { instance_id: String, binding: String, value: Value },
    Delete { instance_id: String, binding: String },
}

/// Ephemeral draft projection. Never an action submission or a persisted edit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiDraftPreviewInput {
    pub identity: UiSurfaceIdentity,
    pub expected_surface_revision: u64,
    pub edits: Vec<UiEdit>,
    pub draft_generation: u64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiDraftPreview {
    pub identity: UiSurfaceIdentity,
    pub base_revision: u64,
    pub draft_generation: u64,
    pub snapshot: UiSnapshot,
}

/// Wrapped by the public Application command envelope for receipt idempotency.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiInteraction {
    pub identity: UiSurfaceIdentity,
    pub expected_surface_revision: u64,
    pub edits: Vec<UiEdit>,
    pub action: Option<UiActionRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiError {
    UnsupportedProtocol,
    UnsupportedCatalog,
    UnsupportedComponent,
    UnsupportedFunction,
    InvalidIdentity,
    InvalidMessage,
    InvalidBinding,
    InvalidListIdentity,
    ImmutableListIdentity,
    ValidationFailed { fields: Vec<UiFieldValidationError> },
    RevisionConflict,
    StaleOwner,
    SurfaceClosed,
    ResourceLimit,
    PermissionDenied,
    CommitUnknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiFieldValidationError {
    pub instance_id: String,
    pub label: String,
    pub messages: Vec<String>,
}

impl std::fmt::Display for UiError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if matches!(self, Self::InvalidListIdentity) {
            return formatter.write_str("InvalidListIdentity: editable template lists require every item to have a unique, non-empty string or numeric id; add stable ids and retry the unchanged surface revision");
        }
        if matches!(self, Self::ImmutableListIdentity) {
            return formatter.write_str("ImmutableListIdentity: input bindings must not modify a template item's stable id or its parent data; bind editable fields separately from id and retry the unchanged surface revision");
        }
        if let Self::ValidationFailed { fields } = self {
            for (index, field) in fields.iter().enumerate() {
                if index > 0 { writeln!(formatter)?; }
                if !field.label.is_empty() { write!(formatter, "{}: ", field.label)?; }
                write!(formatter, "{}", field.messages.join("; "))?;
            }
            Ok(())
        } else {
            write!(formatter, "{self:?}")
        }
    }
}

impl std::error::Error for UiError {}

/// Durable authority state. Renderers receive UiSnapshot, never this structure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiSurfaceState {
    pub identity: UiSurfaceIdentity,
    pub revision: u64,
    pub catalog_id: String,
    pub catalog_digest: String,
    pub phase: UiSurfacePhase,
    pub components: BTreeMap<String, Value>,
    pub data: Value,
    pub theme: Option<Value>,
    pub send_data_model: bool,
}

#[cfg(test)]
mod submission_identity_tests {
    use super::*;

    #[test]
    fn model_action_carries_committed_revision_without_mutating_or_trusting_wire_data() {
        let identity = serde_json::json!({"owner":{"authority_id":"host","conversation_id":"conversation",
            "participant_id":"agent","context_id":"conversation","originating_run_id":"run",
            "message_id":"assistant","part_id":"tool"},"surface_id":"form","generation":1});
        for supplied_revision in [None, Some(999)] {
            let mut action: UiActionContent = serde_json::from_value(serde_json::json!({
                "surface":{"identity":identity,"revision":2},"summary":"Submitted form",
                "envelope":{"type":"ui_action","surface":identity,"message":{"version":"v0.9.1",
                    "action":{"name":"submit","sourceComponentId":"submit","surfaceId":"form",
                        "timestamp":"2026-11-01T00:00:00Z","context":{"name":"ORCHID 用户"}}}}
            })).unwrap();
            if let Some(value) = supplied_revision { action.envelope["surface_revision"] = serde_json::json!(value); }
            action.validate().unwrap();
            let canonical = action.envelope.clone();
            let projected = action.model_envelope();
            assert_eq!(projected["surface_revision"], 2);
            assert_eq!(projected["surface"], canonical["surface"]);
            assert_eq!(projected["message"], canonical["message"]);
            assert_eq!(action.envelope, canonical);
        }
    }

    #[test]
    fn queued_identity_fits_every_host_without_normalization() {
        for id in ["".to_owned(), " action".to_owned(), "action ".to_owned(),
            "a\nb".to_owned(), "a\0b".to_owned(), "x".repeat(129), "界".repeat(43)] {
            assert!(validate_ui_submission_id(&id).is_err(), "accepted invalid identity: {id:?}");
        }
        for id in ["action-1".to_owned(), "x".repeat(128), "界".repeat(42)] {
            assert!(validate_ui_submission_id(&id).is_ok());
            assert!(format!("ui:{id}:user").len() <= 256);
            assert!(format!("{id}:assistant").len() <= 256);
        }
    }
}
