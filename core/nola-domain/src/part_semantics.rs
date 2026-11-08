// SPDX-License-Identifier: Apache-2.0
//! Original-field identities and raw JSON token locations for bounded semantics.
//! A locator is descriptive, not authorization: readers must recheck canonical
//! ownership, visibility, revision and source before returning any bytes.
use crate::{CanonicalBodyReference, PartBodyField};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawPartOwner", into = "RawPartOwner")]
pub struct CanonicalPartOwner {
    conversation_id: String,
    message_id: String,
    part_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPartOwner {
    conversation_id: String,
    message_id: String,
    part_id: String,
}

impl CanonicalPartOwner {
    pub fn new(
        conversation_id: String,
        message_id: String,
        part_id: String,
    ) -> Result<Self, SemanticIdentityError> {
        for id in [&conversation_id, &message_id, &part_id] {
            if id.trim().is_empty() || id.len() > 128 {
                return Err(SemanticIdentityError::InvalidOwner);
            }
        }
        Ok(Self {
            conversation_id,
            message_id,
            part_id,
        })
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
}
impl TryFrom<RawPartOwner> for CanonicalPartOwner {
    type Error = SemanticIdentityError;
    fn try_from(raw: RawPartOwner) -> Result<Self, Self::Error> {
        Self::new(raw.conversation_id, raw.message_id, raw.part_id)
    }
}
impl From<CanonicalPartOwner> for RawPartOwner {
    fn from(owner: CanonicalPartOwner) -> Self {
        Self {
            conversation_id: owner.conversation_id,
            message_id: owner.message_id,
            part_id: owner.part_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "storage", rename_all = "snake_case", deny_unknown_fields)]
pub enum SemanticSourceIdentity {
    /// Digest is over the exact original UTF-8 bytes, never a parsed/reencoded AST.
    InlineSha256 {
        digest: String,
        total_utf8_bytes: u64,
    },
    Immutable {
        reference: CanonicalBodyReference,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawSourceKey", into = "RawSourceKey")]
pub struct SemanticSourceKey {
    owner: CanonicalPartOwner,
    field: PartBodyField,
    identity: SemanticSourceIdentity,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSourceKey {
    owner: CanonicalPartOwner,
    field: PartBodyField,
    identity: SemanticSourceIdentity,
}
impl SemanticSourceKey {
    pub fn new(
        owner: CanonicalPartOwner,
        field: PartBodyField,
        identity: SemanticSourceIdentity,
    ) -> Result<Self, SemanticIdentityError> {
        match &identity {
            SemanticSourceIdentity::InlineSha256 {
                digest,
                total_utf8_bytes,
            } => {
                if digest.len() != 64
                    || !digest
                        .bytes()
                        .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
                    || *total_utf8_bytes > i64::MAX as u64
                {
                    return Err(SemanticIdentityError::InvalidSource);
                }
            }
            SemanticSourceIdentity::Immutable { reference } => {
                if reference.conversation_id() != owner.conversation_id()
                    || reference.message_id() != owner.message_id()
                    || reference.part_id() != owner.part_id()
                    || reference.field() != field
                {
                    return Err(SemanticIdentityError::SourceOwnerMismatch);
                }
            }
        }
        Ok(Self {
            owner,
            field,
            identity,
        })
    }
    pub fn owner(&self) -> &CanonicalPartOwner {
        &self.owner
    }
    pub fn field(&self) -> PartBodyField {
        self.field
    }
    pub fn identity(&self) -> &SemanticSourceIdentity {
        &self.identity
    }
    pub fn total_utf8_bytes(&self) -> u64 {
        match &self.identity {
            SemanticSourceIdentity::InlineSha256 {
                total_utf8_bytes, ..
            } => *total_utf8_bytes,
            SemanticSourceIdentity::Immutable { reference } => reference.total_utf8_bytes(),
        }
    }
}
impl TryFrom<RawSourceKey> for SemanticSourceKey {
    type Error = SemanticIdentityError;
    fn try_from(raw: RawSourceKey) -> Result<Self, Self::Error> {
        Self::new(raw.owner, raw.field, raw.identity)
    }
}
impl From<SemanticSourceKey> for RawSourceKey {
    fn from(key: SemanticSourceKey) -> Self {
        Self {
            owner: key.owner,
            field: key.field,
            identity: key.identity,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticTokenKind {
    Null,
    Boolean,
    Number,
    String,
    Array,
    Object,
}

/// Half-open byte span of an original JSON token, including quotes/escapes.
/// Only a trusted parser can attest that a range actually has this token kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawTokenLocator", into = "RawTokenLocator")]
pub struct SemanticTokenLocator {
    source: SemanticSourceKey,
    start_bytes: u64,
    end_bytes: u64,
    token_kind: SemanticTokenKind,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTokenLocator {
    source: SemanticSourceKey,
    start_bytes: u64,
    end_bytes: u64,
    token_kind: SemanticTokenKind,
}
impl SemanticTokenLocator {
    pub fn new(
        source: SemanticSourceKey,
        start_bytes: u64,
        end_bytes: u64,
        token_kind: SemanticTokenKind,
    ) -> Result<Self, SemanticIdentityError> {
        if source.field() != PartBodyField::ContentJson
            || start_bytes >= end_bytes
            || end_bytes > source.total_utf8_bytes()
        {
            return Err(SemanticIdentityError::InvalidTokenRange);
        }
        Ok(Self {
            source,
            start_bytes,
            end_bytes,
            token_kind,
        })
    }
    pub fn source(&self) -> &SemanticSourceKey {
        &self.source
    }
    pub fn start_bytes(&self) -> u64 {
        self.start_bytes
    }
    pub fn end_bytes(&self) -> u64 {
        self.end_bytes
    }
    pub fn token_kind(&self) -> SemanticTokenKind {
        self.token_kind
    }
    pub fn total_utf8_bytes(&self) -> u64 {
        self.end_bytes - self.start_bytes
    }
}
impl TryFrom<RawTokenLocator> for SemanticTokenLocator {
    type Error = SemanticIdentityError;
    fn try_from(raw: RawTokenLocator) -> Result<Self, Self::Error> {
        Self::new(raw.source, raw.start_bytes, raw.end_bytes, raw.token_kind)
    }
}
impl From<SemanticTokenLocator> for RawTokenLocator {
    fn from(locator: SemanticTokenLocator) -> Self {
        Self {
            source: locator.source,
            start_bytes: locator.start_bytes,
            end_bytes: locator.end_bytes,
            token_kind: locator.token_kind,
        }
    }
}

/// T must be admitted against the projection's total byte/item budget. Deferred
/// is never substituted by null or a truncated value that could be executed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "availability", rename_all = "snake_case", deny_unknown_fields)]
pub enum SemanticValue<T> {
    Complete { value: T },
    Deferred { locator: SemanticTokenLocator },
}

/// Identity matching only. This never grants permission to replay a tool or
/// restore a Run. A derivation requires independent, durable source evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticIdentityAssessment {
    Canonical,
    /// Historical identity matched the root of a revalidated local branch.
    /// This is descriptive binding, never permission to execute a historical tool.
    VerifiedLocalDerivation,
    DerivationRequired,
    CanonicalTurnUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticIdentityError {
    InvalidOwner,
    InvalidSource,
    SourceOwnerMismatch,
    InvalidTokenRange,
}
impl std::fmt::Display for SemanticIdentityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid semantic identity: {self:?}")
    }
}
impl std::error::Error for SemanticIdentityError {}

#[cfg(test)]
mod tests {
    use super::*;
    fn owner() -> CanonicalPartOwner {
        CanonicalPartOwner::new("c".into(), "m".into(), "p".into()).unwrap()
    }
    fn source() -> SemanticSourceKey {
        SemanticSourceKey::new(
            owner(),
            PartBodyField::ContentJson,
            SemanticSourceIdentity::InlineSha256 {
                digest: "a".repeat(64),
                total_utf8_bytes: 100,
            },
        )
        .unwrap()
    }
    #[test]
    fn deserialization_checks_nested_ownership_and_range() {
        let reference = CanonicalBodyReference::for_field(
            "body".into(),
            "c".into(),
            "m".into(),
            "p".into(),
            PartBodyField::ContentJson,
            100,
            1,
        )
        .unwrap();
        let key = SemanticSourceKey::new(
            owner(),
            PartBodyField::ContentJson,
            SemanticSourceIdentity::Immutable { reference },
        )
        .unwrap();
        let locator = SemanticTokenLocator::new(key, 10, 30, SemanticTokenKind::String).unwrap();
        let encoded = serde_json::to_value(&locator).unwrap();
        assert_eq!(
            serde_json::from_value::<SemanticTokenLocator>(encoded.clone()).unwrap(),
            locator
        );
        for (name, value) in [("start_bytes", 30), ("end_bytes", 101)] {
            let mut raw = encoded.clone();
            raw[name] = value.into();
            assert!(serde_json::from_value::<SemanticTokenLocator>(raw).is_err());
        }
        let mut raw = encoded.clone();
        raw["source"]["owner"]["part_id"] = "other".into();
        assert!(serde_json::from_value::<SemanticTokenLocator>(raw).is_err());
        let mut raw = encoded;
        raw["source"]["field"] = "content_text".into();
        assert!(serde_json::from_value::<SemanticTokenLocator>(raw).is_err());
    }
    #[test]
    fn inline_identity_and_explicit_deferred_cannot_be_confused_with_null() {
        assert!(CanonicalPartOwner::new(" ".into(), "m".into(), "p".into()).is_err());
        assert!(SemanticSourceKey::new(
            owner(),
            PartBodyField::ContentJson,
            SemanticSourceIdentity::InlineSha256 {
                digest: "A".repeat(64),
                total_utf8_bytes: 4
            }
        )
        .is_err());
        let complete: SemanticValue<Option<bool>> = SemanticValue::Complete { value: None };
        let deferred: SemanticValue<Option<bool>> = SemanticValue::Deferred {
            locator: SemanticTokenLocator::new(source(), 0, 4, SemanticTokenKind::Null).unwrap(),
        };
        let encoded = serde_json::to_value(&complete).unwrap();
        assert_ne!(encoded, serde_json::to_value(&deferred).unwrap());
        assert_eq!(
            serde_json::from_value::<SemanticValue<Option<bool>>>(encoded).unwrap(),
            complete
        );
    }
}
