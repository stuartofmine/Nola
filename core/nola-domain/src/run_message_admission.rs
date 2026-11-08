// SPDX-License-Identifier: Apache-2.0
//! Bounded immutable message declarations, separate from mutable assistant content.
use crate::assistant_journal::{AssistantDigest, AssistantJournalIdentity};
use crate::assistant_journal_operations::JournalNumber;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const ADMISSION_CHUNK_BYTES: usize = 16384;
pub fn admission_digest(bytes: &[u8]) -> AssistantDigest {
    AssistantDigest::parse_hex(&format!("{:x}", Sha256::digest(bytes))).expect("SHA-256 hex")
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunMessageAdmissionCursor {
    pub identity: AssistantJournalIdentity,
    pub sequence: JournalNumber,
    pub record_digest: AssistantDigest,
}
impl RunMessageAdmissionCursor {
    pub fn initial(identity: AssistantJournalIdentity) -> Result<Self, String> {
        identity.validate().map_err(|e| e.to_string())?;
        let bytes = serde_json::to_vec(&("NOLA-MESSAGE-ADMISSION-GENESIS-1", &identity))
            .map_err(|e| e.to_string())?;
        Ok(Self {
            identity,
            sequence: JournalNumber(0),
            record_digest: admission_digest(&bytes),
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunMessageAdmissionRecord {
    pub cursor: RunMessageAdmissionCursor,
    pub previous_digest: AssistantDigest,
    pub message_id: String,
    pub offset: JournalNumber,
    pub total_utf8_bytes: JournalNumber,
    pub declaration_digest: AssistantDigest,
    pub text: String,
}
impl RunMessageAdmissionRecord {
    pub fn computed_digest(&self) -> Result<AssistantDigest, String> {
        serde_json::to_vec(&(
            "NOLA-MESSAGE-ADMISSION-RECORD-1",
            &self.cursor.identity,
            self.cursor.sequence,
            self.previous_digest,
            &self.message_id,
            self.offset,
            self.total_utf8_bytes,
            self.declaration_digest,
            &self.text,
        ))
        .map(|v| admission_digest(&v))
        .map_err(|e| e.to_string())
    }
    pub fn validate(&self) -> Result<(), String> {
        self.cursor.identity.validate().map_err(|e| e.to_string())?;
        if self.message_id.trim().is_empty()
            || self.cursor.sequence.0 == 0
            || self.text.is_empty()
            || self.text.len() > ADMISSION_CHUNK_BYTES
            || self
                .offset
                .0
                .checked_add(self.text.len() as u64)
                .is_none_or(|n| n > self.total_utf8_bytes.0)
            || self.computed_digest()? != self.cursor.record_digest
        {
            return Err("invalid message admission record".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunMessageAdmissionPage {
    pub after: RunMessageAdmissionCursor,
    pub through: RunMessageAdmissionCursor,
    pub next: RunMessageAdmissionCursor,
    pub complete: bool,
    pub records: Vec<RunMessageAdmissionRecord>,
}
