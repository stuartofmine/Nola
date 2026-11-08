// SPDX-License-Identifier: Apache-2.0
//! Immutable canonical Part-field identity, independent of storage and rendering.
//! An upload ID is not a canonical content ID. Application validates ownership
//! against persisted records; possession of this descriptor grants no authority.

use serde::{Deserialize, Serialize};
use std::fmt;

pub const CANONICAL_BODY_CHUNK_MAX_BYTES: u64 = 32 * 1024;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, std::hash::Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum PartBodyField {
    ContentText,
    ContentJson,
}

const ID_MAX_BYTES: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    try_from = "RawCanonicalBodyReference",
    into = "RawCanonicalBodyReference"
)]
pub struct CanonicalBodyReference {
    content_id: String,
    conversation_id: String,
    message_id: String,
    part_id: String,
    field: PartBodyField,
    total_utf8_bytes: u64,
    chunk_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCanonicalBodyReference {
    content_id: String,
    conversation_id: String,
    message_id: String,
    part_id: String,
    field: PartBodyField,
    total_utf8_bytes: u64,
    chunk_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanonicalBodyReferenceError {
    InvalidIdentity { field: &'static str, utf8_bytes: usize },
    InvalidChunkLayout,
}

impl fmt::Display for CanonicalBodyReferenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidIdentity { field, utf8_bytes } => write!(formatter,
                "invalid canonical text identity: {field} ({utf8_bytes} UTF-8 bytes)"),
            Self::InvalidChunkLayout => formatter.write_str("invalid canonical UTF-8 chunk layout"),
        }
    }
}

impl std::error::Error for CanonicalBodyReferenceError {}

impl CanonicalBodyReference {
    pub fn for_field(
        content_id: String,
        conversation_id: String,
        message_id: String,
        part_id: String,
        field: PartBodyField,
        total_utf8_bytes: u64,
        chunk_count: u32,
    ) -> Result<Self, CanonicalBodyReferenceError> {
        RawCanonicalBodyReference {
            content_id,
            conversation_id,
            message_id,
            part_id,
            field,
            total_utf8_bytes,
            chunk_count,
        }
        .try_into()
    }

    pub fn field(&self) -> PartBodyField {
        self.field
    }
    pub fn content_id(&self) -> &str {
        &self.content_id
    }
    pub fn conversation_id(&self) -> &str {
        &self.conversation_id
    }
    pub fn message_id(&self) -> &str {
        &self.message_id
    }
    pub fn part_id(&self) -> &str {
        &self.part_id
    }
    pub fn total_utf8_bytes(&self) -> u64 {
        self.total_utf8_bytes
    }
    pub fn chunk_count(&self) -> u32 {
        self.chunk_count
    }
}

impl TryFrom<RawCanonicalBodyReference> for CanonicalBodyReference {
    type Error = CanonicalBodyReferenceError;

    fn try_from(raw: RawCanonicalBodyReference) -> Result<Self, Self::Error> {
        for (field, id) in [
            ("content_id", &raw.content_id),
            ("conversation_id", &raw.conversation_id),
            ("message_id", &raw.message_id),
            ("part_id", &raw.part_id),
        ] {
            if id.is_empty()
                || id.trim() != id
                || id.len() > ID_MAX_BYTES
                || id.chars().any(char::is_control)
            {
                return Err(Self::Error::InvalidIdentity { field, utf8_bytes: id.len() });
            }
        }
        // Chunks are nonempty and bounded, but need not be equal-sized. Empty
        // text remains an inline Part rather than an externally stored object.
        let chunks = u64::from(raw.chunk_count);
        if chunks == 0
            || raw.total_utf8_bytes < chunks
            || raw.total_utf8_bytes > chunks * CANONICAL_BODY_CHUNK_MAX_BYTES
        {
            return Err(Self::Error::InvalidChunkLayout);
        }
        Ok(Self {
            content_id: raw.content_id,
            conversation_id: raw.conversation_id,
            message_id: raw.message_id,
            part_id: raw.part_id,
            field: raw.field,
            total_utf8_bytes: raw.total_utf8_bytes,
            chunk_count: raw.chunk_count,
        })
    }
}

impl From<CanonicalBodyReference> for RawCanonicalBodyReference {
    fn from(value: CanonicalBodyReference) -> Self {
        Self {
            content_id: value.content_id,
            conversation_id: value.conversation_id,
            message_id: value.message_id,
            part_id: value.part_id,
            field: value.field,
            total_utf8_bytes: value.total_utf8_bytes,
            chunk_count: value.chunk_count,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> CanonicalBodyReference {
        CanonicalBodyReference::for_field(
            "content".into(),
            "conversation".into(),
            "message".into(),
            "part".into(),
            crate::PartBodyField::ContentText,
            10_000_000,
            306,
        )
        .unwrap()
    }

    #[test]
    fn ten_million_byte_content_has_a_small_round_trippable_reference() {
        let reference = valid();
        let wire = serde_json::to_vec(&reference).unwrap();
        assert!(wire.len() < 1024);
        assert_eq!(
            serde_json::from_slice::<CanonicalBodyReference>(&wire).unwrap(),
            reference
        );
    }

    #[test]
    fn deserialization_cannot_bypass_identity_validation() {
        for key in ["content_id", "conversation_id", "message_id", "part_id"] {
            for invalid in [
                "".to_string(),
                " padded".into(),
                "x\n".into(),
                "x".repeat(129),
            ] {
                let mut wire = serde_json::to_value(valid()).unwrap();
                wire[key] = invalid.into();
                let error = serde_json::from_value::<CanonicalBodyReference>(wire).unwrap_err().to_string();
                assert!(error.contains(key));
                assert!(!error.contains("padded"));
            }
        }
    }

    #[test]
    fn impossible_layouts_and_unknown_fields_are_rejected() {
        for (bytes, count) in [(0_u64, 0_u32), (3, 4), (32769, 1), (u64::MAX, u32::MAX)] {
            assert!(CanonicalBodyReference::for_field(
                "c".into(),
                "s".into(),
                "m".into(),
                "p".into(),
                crate::PartBodyField::ContentText,
                bytes,
                count
            )
            .is_err());
        }
        let mut wire = serde_json::to_value(valid()).unwrap();
        wire["upload_id"] = "not-a-canonical-reference".into();
        assert!(serde_json::from_value::<CanonicalBodyReference>(wire).is_err());
    }
    #[test]
    fn field_is_required_and_has_one_canonical_descriptor() {
        let text: CanonicalBodyReference = valid();
        assert_eq!(text.field(), PartBodyField::ContentText);
        let mut wire = serde_json::to_value(&text).unwrap();
        assert_eq!(wire["field"], "content_text");
        wire.as_object_mut().unwrap().remove("field");
        assert!(serde_json::from_value::<CanonicalBodyReference>(wire.clone()).is_err());
        wire["field"] = "content_json".into();
        let json: CanonicalBodyReference = serde_json::from_value(wire.clone()).unwrap();
        assert_ne!(json, text);
        assert_eq!(json.field(), PartBodyField::ContentJson);
        wire["field"] = "unknown".into();
        assert!(serde_json::from_value::<CanonicalBodyReference>(wire).is_err());
    }
}
