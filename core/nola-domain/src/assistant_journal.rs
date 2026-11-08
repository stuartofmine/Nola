// SPDX-License-Identifier: Apache-2.0
//! Versioned primitives for the *planned* committed assistant journal.
//! No persistence, visibility projection, network authentication, or operation
//! batch schema is implemented here. Trace cursors are not journal cursors.
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};

pub const ASSISTANT_JOURNAL_VERSION: u16 = 1;
pub const ASSISTANT_CHUNK_MAX_BYTES: usize = 64 * 1024;
/// Raw UTF-8 range budgets, independent of stored chunks and encoded frames.
pub const ASSISTANT_RANGE_MIN_BYTES: usize = 4;
pub const ASSISTANT_RANGE_MAX_BYTES: usize = 32 * 1024;
pub const ASSISTANT_FRAME_MAX_BYTES: usize = 256 * 1024;
pub const ASSISTANT_PAGE_MAX_RECORDS: usize = 128;
pub const ASSISTANT_RECORD_MAX_OPERATIONS: usize = 128;
pub const ASSISTANT_ID_MAX_BYTES: usize = 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssistantJournalError {
    UnsupportedVersion,
    InvalidIdentity,
    InvalidDigest,
    InvalidSequence,
    InvalidSourceRevision,
    ChunkTooLarge,
    EmptyChunk,
    BudgetExceeded,
    IdentityMismatch,
    RecordDigestMismatch,
    PreviousDigestMismatch,
    ConflictingReplay,
    SequenceGap { expected: u64, received: u64 },
    HistoricalVerificationRequired { sequence: u64 },
}

impl std::fmt::Display for AssistantJournalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for AssistantJournalError {}

/// Wire representation is exactly 64 lowercase hexadecimal characters.
/// This is SHA-256 of the versioned canonical bytes, NOT serde JSON bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AssistantDigest([u8; 32]);

impl AssistantDigest {
    pub fn parse_hex(value: &str) -> Result<Self, AssistantJournalError> {
        if value.len() != 64
            || !value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(AssistantJournalError::InvalidDigest);
        }
        let mut result = [0; 32];
        for (i, pair) in value.as_bytes().chunks_exact(2).enumerate() {
            let nibble = |b: u8| if b <= b'9' { b - b'0' } else { b - b'a' + 10 };
            result[i] = (nibble(pair[0]) << 4) | nibble(pair[1]);
        }
        Ok(Self(result))
    }
    pub fn to_hex(self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }
    fn hash(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }
}
impl Serialize for AssistantDigest {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_hex())
    }
}
impl<'de> Deserialize<'de> for AssistantDigest {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Self::parse_hex(&String::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantJournalIdentity {
    /// Durable source-store incarnation, not a per-connection nonce or Host alias.
    pub source_store_epoch: String,
    pub conversation_id: String,
    pub run_id: String,
    pub assistant_message_id: String,
}
impl AssistantJournalIdentity {
    pub fn validate(&self) -> Result<(), AssistantJournalError> {
        for id in [
            &self.source_store_epoch,
            &self.conversation_id,
            &self.run_id,
            &self.assistant_message_id,
        ] {
            validate_id(id)?;
        }
        Ok(())
    }
    fn encode(&self, bytes: &mut Vec<u8>) {
        for value in [
            &self.source_store_epoch,
            &self.conversation_id,
            &self.run_id,
            &self.assistant_message_id,
        ] {
            encode_bytes(bytes, value.as_bytes());
        }
    }
}

fn validate_id(id: &str) -> Result<(), AssistantJournalError> {
    if id.trim().is_empty() || id.len() > ASSISTANT_ID_MAX_BYTES || id.chars().any(char::is_control)
    {
        return Err(AssistantJournalError::InvalidIdentity);
    }
    Ok(())
}

// Canonical encoding v1: ASCII NOLA-AJ, u16 version BE, u8 kind,
// then fixed field order. Strings/bytes are u32 byte-length BE + raw UTF-8;
// integers are u64 BE, digests are raw 32 bytes. No normalization or JSON.
// Kinds: 0 genesis, 1 text chunk, 2 record header. Future kinds never reuse tags.
fn prefix(kind: u8) -> Vec<u8> {
    let mut bytes = b"NOLA-AJ".to_vec();
    bytes.extend_from_slice(&ASSISTANT_JOURNAL_VERSION.to_be_bytes());
    bytes.push(kind);
    bytes
}
fn encode_bytes(encoded: &mut Vec<u8>, value: &[u8]) {
    encoded.extend_from_slice(&(value.len() as u32).to_be_bytes());
    encoded.extend_from_slice(value);
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantTextChunkV1 {
    protocol_version: u16,
    digest: AssistantDigest,
    text: String,
}
impl AssistantTextChunkV1 {
    pub fn new(text: String) -> Result<Self, AssistantJournalError> {
        validate_chunk_text(&text)?;
        let mut result = Self {
            protocol_version: ASSISTANT_JOURNAL_VERSION,
            digest: AssistantDigest([0; 32]),
            text,
        };
        result.digest = AssistantDigest::hash(&result.canonical_bytes()?);
        Ok(result)
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn digest(&self) -> AssistantDigest {
        self.digest
    }
    pub fn byte_length(&self) -> usize {
        self.text.len()
    }
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, AssistantJournalError> {
        if self.protocol_version != ASSISTANT_JOURNAL_VERSION {
            return Err(AssistantJournalError::UnsupportedVersion);
        }
        validate_chunk_text(&self.text)?;
        let mut bytes = prefix(1);
        encode_bytes(&mut bytes, self.text.as_bytes());
        Ok(bytes)
    }
    pub fn verify(&self) -> Result<(), AssistantJournalError> {
        if AssistantDigest::hash(&self.canonical_bytes()?) != self.digest {
            return Err(AssistantJournalError::RecordDigestMismatch);
        }
        Ok(())
    }
}
fn validate_chunk_text(text: &str) -> Result<(), AssistantJournalError> {
    if text.is_empty() {
        return Err(AssistantJournalError::EmptyChunk);
    }
    if text.len() > ASSISTANT_CHUNK_MAX_BYTES {
        return Err(AssistantJournalError::ChunkTooLarge);
    }
    Ok(())
}

/// Only a header contract. Operation-batch canonical encoding/visibility still
/// needs its own typed implementation; this digest alone cannot validate it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantJournalRecordHeaderV1 {
    pub protocol_version: u16,
    pub identity: AssistantJournalIdentity,
    #[serde(with = "decimal_u64")]
    pub sequence: u64,
    pub previous_record_digest: AssistantDigest,
    pub operation_id: String,
    #[serde(with = "decimal_u64")]
    pub source_commit_revision: u64,
    pub operation_batch_digest: AssistantDigest,
    pub record_digest: AssistantDigest,
}
impl AssistantJournalRecordHeaderV1 {
    pub fn seal(
        identity: AssistantJournalIdentity,
        sequence: u64,
        previous_record_digest: AssistantDigest,
        operation_id: String,
        source_commit_revision: u64,
        operation_batch_digest: AssistantDigest,
    ) -> Result<Self, AssistantJournalError> {
        let mut record = Self {
            protocol_version: ASSISTANT_JOURNAL_VERSION,
            identity,
            sequence,
            previous_record_digest,
            operation_id,
            source_commit_revision,
            operation_batch_digest,
            record_digest: AssistantDigest([0; 32]),
        };
        record.record_digest = AssistantDigest::hash(&record.canonical_bytes()?);
        Ok(record)
    }
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, AssistantJournalError> {
        if self.protocol_version != ASSISTANT_JOURNAL_VERSION {
            return Err(AssistantJournalError::UnsupportedVersion);
        }
        self.identity.validate()?;
        validate_id(&self.operation_id)?;
        if self.sequence == 0 {
            return Err(AssistantJournalError::InvalidSequence);
        }
        if self.source_commit_revision == 0 {
            return Err(AssistantJournalError::InvalidSourceRevision);
        }
        let mut bytes = prefix(2);
        self.identity.encode(&mut bytes);
        bytes.extend_from_slice(&self.sequence.to_be_bytes());
        bytes.extend_from_slice(&self.previous_record_digest.0);
        encode_bytes(&mut bytes, self.operation_id.as_bytes());
        bytes.extend_from_slice(&self.source_commit_revision.to_be_bytes());
        bytes.extend_from_slice(&self.operation_batch_digest.0);
        Ok(bytes)
    }
    pub fn verify(&self) -> Result<(), AssistantJournalError> {
        if AssistantDigest::hash(&self.canonical_bytes()?) != self.record_digest {
            return Err(AssistantJournalError::RecordDigestMismatch);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantJournalCursor {
    pub identity: AssistantJournalIdentity,
    #[serde(with = "decimal_u64")]
    pub sequence: u64,
    pub record_digest: AssistantDigest,
}

/// JSON integers are decimal strings to preserve all u64 values in JavaScript.
/// Canonical binary fields remain u64 BE. Store adapters must separately reject
/// values outside their database integer domain, never truncate or saturate.
mod decimal_u64 {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&value.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
        let value = String::deserialize(deserializer)?;
        if value.is_empty()
            || value.len() > 20
            || !value.bytes().all(|b| b.is_ascii_digit())
            || (value.len() > 1 && value.starts_with('0'))
        {
            return Err(serde::de::Error::custom(
                "expected canonical unsigned decimal u64 string",
            ));
        }
        value.parse::<u64>().map_err(serde::de::Error::custom)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssistantRecordAdmission {
    Apply,
    Replay,
}
impl AssistantJournalCursor {
    pub fn initial(identity: AssistantJournalIdentity) -> Result<Self, AssistantJournalError> {
        identity.validate()?;
        let mut bytes = prefix(0);
        identity.encode(&mut bytes);
        Ok(Self {
            identity,
            sequence: 0,
            record_digest: AssistantDigest::hash(&bytes),
        })
    }
    /// Does not advance this cursor. Store must atomically apply operations and
    /// install the new head. Older duplicates require the actual retained record.
    pub fn admit(
        &self,
        record: &AssistantJournalRecordHeaderV1,
    ) -> Result<AssistantRecordAdmission, AssistantJournalError> {
        self.identity.validate()?;
        record.verify()?;
        if self.identity != record.identity {
            return Err(AssistantJournalError::IdentityMismatch);
        }
        if self.sequence == 0
            && Self::initial(self.identity.clone())?.record_digest != self.record_digest
        {
            return Err(AssistantJournalError::PreviousDigestMismatch);
        }
        if record.sequence == self.sequence {
            return if record.record_digest == self.record_digest {
                Ok(AssistantRecordAdmission::Replay)
            } else {
                Err(AssistantJournalError::ConflictingReplay)
            };
        }
        if record.sequence < self.sequence {
            return Err(AssistantJournalError::HistoricalVerificationRequired {
                sequence: record.sequence,
            });
        }
        let expected = self
            .sequence
            .checked_add(1)
            .ok_or(AssistantJournalError::InvalidSequence)?;
        if record.sequence != expected {
            return Err(AssistantJournalError::SequenceGap {
                expected,
                received: record.sequence,
            });
        }
        if record.previous_record_digest != self.record_digest {
            return Err(AssistantJournalError::PreviousDigestMismatch);
        }
        Ok(AssistantRecordAdmission::Apply)
    }
}

/// Actual encoded wire bytes, not Rust string length estimates. The adapter
/// must check before sending/allocating a complete frame; chunks are separate.
pub fn validate_assistant_page_budget(
    encoded_bytes: usize,
    records: usize,
) -> Result<(), AssistantJournalError> {
    if encoded_bytes > ASSISTANT_FRAME_MAX_BYTES || records > ASSISTANT_PAGE_MAX_RECORDS {
        return Err(AssistantJournalError::BudgetExceeded);
    }
    Ok(())
}

pub fn validate_assistant_record_budget(
    encoded_bytes: usize,
    operations: usize,
) -> Result<(), AssistantJournalError> {
    if encoded_bytes > ASSISTANT_FRAME_MAX_BYTES
        || operations == 0
        || operations > ASSISTANT_RECORD_MAX_OPERATIONS
    {
        return Err(AssistantJournalError::BudgetExceeded);
    }
    Ok(())
}

#[cfg(test)]
#[path = "assistant_journal_tests.rs"]
mod tests;
