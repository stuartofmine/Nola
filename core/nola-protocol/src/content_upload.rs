// SPDX-License-Identifier: Apache-2.0
//! Bounded content-upload contracts. Storage and authorization belong to Application.

use serde::{Deserialize, Serialize};

/// Worst-case JSON escaping expands one UTF-8 byte to six ASCII bytes.
/// Leave room below the control-command ceiling for identities and envelopes.
pub const CONTENT_CHUNK_MAX_BYTES: usize = 32 * 1024;
pub const CONTENT_ID_MAX_BYTES: usize = 128;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContentUploadOwner {
    pub conversation_id: String,
    pub import_id: String,
    pub part_id: String,
}

impl ContentUploadOwner {
    pub fn validate(&self) -> Result<(), ContentUploadError> {
        for id in [&self.conversation_id, &self.import_id, &self.part_id] {
            validate_id(id)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BeginContentUpload {
    pub upload_id: String,
    pub owner: ContentUploadOwner,
    pub expected_utf8_bytes: u64,
}

impl BeginContentUpload {
    /// Application supplies the configured quota; clients cannot raise it.
    pub fn validate(&self, maximum_bytes: u64) -> Result<(), ContentUploadError> {
        validate_id(&self.upload_id)?;
        self.owner.validate()?;
        if self.expected_utf8_bytes == 0 || self.expected_utf8_bytes > maximum_bytes {
            return Err(ContentUploadError::InvalidLength);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AppendContentChunk {
    pub upload_id: String,
    pub owner: ContentUploadOwner,
    pub offset_bytes: u64,
    pub text: String,
}

impl AppendContentChunk {
    /// Returns the exclusive byte boundary, not a character count. Application
    /// must additionally verify owner, durable offset, replay bytes and state.
    pub fn validate(&self, expected_utf8_bytes: u64) -> Result<u64, ContentUploadError> {
        validate_id(&self.upload_id)?;
        self.owner.validate()?;
        if self.text.is_empty() || self.text.len() > CONTENT_CHUNK_MAX_BYTES {
            return Err(ContentUploadError::InvalidChunk);
        }
        let end = self
            .offset_bytes
            .checked_add(self.text.len() as u64)
            .ok_or(ContentUploadError::InvalidOffset)?;
        if end > expected_utf8_bytes {
            return Err(ContentUploadError::InvalidOffset);
        }
        Ok(end)
    }
}

/// Used for progress, sealing and cancellation. Every operation carries owner
/// identity; an upload ID alone is never proof of access.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContentUploadRequest {
    pub upload_id: String,
    pub owner: ContentUploadOwner,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReadContentUploadChunk {
    pub upload_id: String,
    pub owner: ContentUploadOwner,
    pub offset_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContentUploadChunk {
    pub upload_id: String,
    pub owner: ContentUploadOwner,
    pub offset_bytes: u64,
    pub next_offset_bytes: u64,
    pub text: String,
    pub end_of_content: bool,
}

impl ContentUploadRequest {
    pub fn validate(&self) -> Result<(), ContentUploadError> {
        validate_id(&self.upload_id)?;
        self.owner.validate()
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContentUploadState {
    Receiving,
    Sealed,
    Cancelled,
    Expired,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContentUploadProgress {
    pub upload_id: String,
    pub owner: ContentUploadOwner,
    pub state: ContentUploadState,
    pub committed_utf8_bytes: u64,
    pub expected_utf8_bytes: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContentUploadError {
    InvalidIdentity,
    InvalidLength,
    InvalidChunk,
    InvalidOffset,
    OwnerMismatch,
    ReplayConflict,
    NotReceiving,
    Incomplete,
    NotFound,
    QuotaExceeded,
}

fn validate_id(id: &str) -> Result<(), ContentUploadError> {
    if id.trim().is_empty() || id.len() > CONTENT_ID_MAX_BYTES || id.chars().any(char::is_control) {
        return Err(ContentUploadError::InvalidIdentity);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(text: String, offset_bytes: u64) -> AppendContentChunk {
        AppendContentChunk {
            upload_id: "upload".into(),
            owner: ContentUploadOwner {
                conversation_id: "conversation".into(),
                import_id: "import".into(),
                part_id: "part".into(),
            },
            offset_bytes,
            text,
        }
    }

    #[test]
    fn chunk_budget_covers_worst_case_json_escaping() {
        let mut value = chunk("\0".repeat(CONTENT_CHUNK_MAX_BYTES), 0);
        value.upload_id = "x".repeat(CONTENT_ID_MAX_BYTES);
        value.owner.conversation_id = value.upload_id.clone();
        value.owner.import_id = value.upload_id.clone();
        value.owner.part_id = value.upload_id.clone();
        assert!(value.validate(CONTENT_CHUNK_MAX_BYTES as u64).is_ok());
        let wire =
            serde_json::to_vec(&serde_json::json!({"type":"append_content_chunk", "input":value}))
                .unwrap();
        assert!(wire.len() + 4096 < crate::COMMAND_MAX_CONTROL_BYTES);
    }

    #[test]
    fn unicode_offsets_are_bytes_and_overflow_is_rejected() {
        assert_eq!(chunk("中😀".into(), 3).validate(10), Ok(10));
        assert_eq!(
            chunk("中😀".into(), 3).validate(9),
            Err(ContentUploadError::InvalidOffset)
        );
        assert_eq!(
            chunk("x".into(), u64::MAX).validate(u64::MAX),
            Err(ContentUploadError::InvalidOffset)
        );
        assert_eq!(
            chunk(String::new(), 0).validate(1),
            Err(ContentUploadError::InvalidChunk)
        );
        assert_eq!(
            chunk("x".repeat(CONTENT_CHUNK_MAX_BYTES + 1), 0).validate(u64::MAX),
            Err(ContentUploadError::InvalidChunk)
        );
    }

    #[test]
    fn ten_million_bytes_fit_independently_bounded_commands() {
        let total = 10_000_000_u64;
        let mut offset = 0;
        while offset < total {
            let length = (total - offset).min(CONTENT_CHUNK_MAX_BYTES as u64) as usize;
            let value = chunk("x".repeat(length), offset);
            offset = value.validate(total).unwrap();
            assert!(serde_json::to_vec(&value).unwrap().len() < crate::COMMAND_MAX_CONTROL_BYTES);
        }
        assert_eq!(offset, total);
    }

    #[test]
    fn ownership_and_quotas_are_explicit() {
        let value = chunk("x".into(), 0);
        let mut begin = BeginContentUpload {
            upload_id: value.upload_id,
            owner: value.owner,
            expected_utf8_bytes: 10,
        };
        assert!(begin.validate(10).is_ok());
        assert_eq!(begin.validate(9), Err(ContentUploadError::InvalidLength));
        begin.owner.part_id.clear();
        assert_eq!(begin.validate(10), Err(ContentUploadError::InvalidIdentity));
        assert!(serde_json::from_str::<ContentUploadRequest>(r#"{"upload_id":"x"}"#).is_err());
    }
}
