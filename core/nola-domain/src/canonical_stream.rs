// SPDX-License-Identifier: Apache-2.0
//! Canonical-private text transitions, not a public journal or a read capability.
//! Callers must independently authorize persisted owner identities. No production
//! writer is enabled by this contract; storage must atomically CAS the entire head.

use crate::{PartBodyField, CANONICAL_BODY_CHUNK_MAX_BYTES};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const CANONICAL_STREAM_VERSION: u32 = 1;
const DOMAIN: &[u8] = b"nola.canonical-private-stream.v1\0";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalStreamOwner {
    pub conversation_id: String,
    pub run_id: String,
    pub message_id: String,
    pub part_id: String,
    pub field: PartBodyField,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalStreamHead {
    pub version: u32,
    pub owner: CanonicalStreamOwner,
    pub generation: u64,
    pub revision: u64,
    pub root: String,
    pub total_utf8_bytes: u64,
    /// Number of nonempty payload chunks in this generation, not revision count.
    pub chunk_count: u64,
    pub closed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CanonicalStreamOperation {
    Declare {
        owner: CanonicalStreamOwner,
    },
    Append {
        owner: CanonicalStreamOwner,
        expected_head: CanonicalStreamHead,
        delta: String,
    },
    /// One bounded chunk, or empty to start a new generation before appending.
    Replace {
        owner: CanonicalStreamOwner,
        expected_head: CanonicalStreamHead,
        text: String,
    },
    Close {
        owner: CanonicalStreamOwner,
        expected_head: CanonicalStreamHead,
    },
}

impl CanonicalStreamOperation {
    pub fn owner(&self) -> &CanonicalStreamOwner {
        match self {
            Self::Declare { owner }
            | Self::Append { owner, .. }
            | Self::Replace { owner, .. }
            | Self::Close { owner, .. } => owner,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanonicalStreamError {
    InvalidOwner,
    UnsupportedField,
    InvalidHead,
    HeadConflict,
    Closed,
    EmptyDelta,
    ChunkTooLarge,
    Overflow,
}

impl CanonicalStreamOwner {
    pub fn validate(&self) -> Result<(), CanonicalStreamError> {
        for id in [
            &self.conversation_id,
            &self.run_id,
            &self.message_id,
            &self.part_id,
        ] {
            if id.is_empty()
                || id.len() > 128
                || id.trim() != id
                || id.chars().any(char::is_control)
            {
                return Err(CanonicalStreamError::InvalidOwner);
            }
        }
        if self.field != PartBodyField::ContentText {
            return Err(CanonicalStreamError::UnsupportedField);
        }
        Ok(())
    }
}

impl CanonicalStreamHead {
    /// Shape validation is not proof that chunks exist or that a root is trusted.
    pub fn validate(&self) -> Result<(), CanonicalStreamError> {
        self.owner.validate()?;
        // Declare contributes one revision. Each Replace contributes another
        // generation/revision and may supply that generation's first chunk.
        // All other chunks and Close each require an additional revision.
        let appended_chunks = if self.generation == 1 {
            self.chunk_count
        } else {
            self.chunk_count.saturating_sub(1)
        };
        let minimum_revision = self
            .generation
            .checked_add(appended_chunks)
            .and_then(|value| value.checked_add(u64::from(self.closed)))
            .ok_or(CanonicalStreamError::InvalidHead)?;
        if self.version != CANONICAL_STREAM_VERSION
            || self.generation == 0
            || self.revision == 0
            || self.revision < minimum_revision
            || self.root.len() != 64
            || !self
                .root
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || (self.chunk_count == 0) != (self.total_utf8_bytes == 0)
            || self.chunk_count > self.total_utf8_bytes
            || self
                .total_utf8_bytes
                .div_ceil(CANONICAL_BODY_CHUNK_MAX_BYTES)
                > self.chunk_count
        {
            return Err(CanonicalStreamError::InvalidHead);
        }
        Ok(())
    }
}

/// A replay against an already advanced head is a conflict, never a second append.
/// A future transaction receipt layer may recognize exact replay before this CAS.
pub fn apply_canonical_stream(
    current: Option<&CanonicalStreamHead>,
    operation: &CanonicalStreamOperation,
) -> Result<CanonicalStreamHead, CanonicalStreamError> {
    use CanonicalStreamOperation::*;
    if let Declare { owner } = operation {
        owner.validate()?;
        if current.is_some() {
            return Err(CanonicalStreamError::HeadConflict);
        }
        let mut head = CanonicalStreamHead {
            version: CANONICAL_STREAM_VERSION,
            owner: owner.clone(),
            generation: 1,
            revision: 1,
            root: String::new(),
            total_utf8_bytes: 0,
            chunk_count: 0,
            closed: false,
        };
        head.root = transition_root(&head, "declare", "", "");
        return Ok(head);
    }
    let (owner, expected) = match operation {
        Append {
            owner,
            expected_head,
            ..
        }
        | Replace {
            owner,
            expected_head,
            ..
        }
        | Close {
            owner,
            expected_head,
        } => (owner, expected_head),
        Declare { .. } => unreachable!(),
    };
    owner.validate()?;
    expected.validate()?;
    let old = current.ok_or(CanonicalStreamError::HeadConflict)?;
    old.validate()?;
    if old != expected || &old.owner != owner {
        return Err(CanonicalStreamError::HeadConflict);
    }
    if old.closed {
        return Err(CanonicalStreamError::Closed);
    }
    let mut head = old.clone();
    head.revision = old
        .revision
        .checked_add(1)
        .ok_or(CanonicalStreamError::Overflow)?;
    let (kind, payload) = match operation {
        Append { delta, .. } => {
            if delta.is_empty() {
                return Err(CanonicalStreamError::EmptyDelta);
            }
            let bytes = chunk_bytes(delta)?;
            head.total_utf8_bytes = old
                .total_utf8_bytes
                .checked_add(bytes)
                .ok_or(CanonicalStreamError::Overflow)?;
            head.chunk_count = old
                .chunk_count
                .checked_add(1)
                .ok_or(CanonicalStreamError::Overflow)?;
            ("append", delta.as_str())
        }
        Replace { text, .. } => {
            head.generation = old
                .generation
                .checked_add(1)
                .ok_or(CanonicalStreamError::Overflow)?;
            head.total_utf8_bytes = chunk_bytes(text)?;
            head.chunk_count = u64::from(!text.is_empty());
            ("replace", text.as_str())
        }
        Close { .. } => {
            head.closed = true;
            ("close", "")
        }
        Declare { .. } => unreachable!(),
    };
    head.root = transition_root(&head, kind, &old.root, payload);
    Ok(head)
}

fn chunk_bytes(text: &str) -> Result<u64, CanonicalStreamError> {
    let bytes = u64::try_from(text.len()).map_err(|_| CanonicalStreamError::Overflow)?;
    if bytes > CANONICAL_BODY_CHUNK_MAX_BYTES {
        return Err(CanonicalStreamError::ChunkTooLarge);
    }
    Ok(bytes)
}

// Length framing prevents concatenation collisions. Cost is O(delta), never O(prefix).
fn transition_root(
    head: &CanonicalStreamHead,
    kind: &str,
    previous: &str,
    payload: &str,
) -> String {
    let mut hash = Sha256::new();
    hash.update(DOMAIN);
    for value in [
        kind,
        &head.owner.conversation_id,
        &head.owner.run_id,
        &head.owner.message_id,
        &head.owner.part_id,
        "content_text",
        previous,
        payload,
    ] {
        hash.update((value.len() as u64).to_be_bytes());
        hash.update(value.as_bytes());
    }
    hash.update(head.version.to_be_bytes());
    for value in [
        head.generation,
        head.revision,
        head.total_utf8_bytes,
        head.chunk_count,
    ] {
        hash.update(value.to_be_bytes());
    }
    hash.update([u8::from(head.closed)]);
    format!("{:x}", hash.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn owner() -> CanonicalStreamOwner {
        CanonicalStreamOwner {
            conversation_id: "c".into(),
            run_id: "r".into(),
            message_id: "m".into(),
            part_id: "p".into(),
            field: PartBodyField::ContentText,
        }
    }
    fn declared() -> CanonicalStreamHead {
        apply_canonical_stream(None, &CanonicalStreamOperation::Declare { owner: owner() }).unwrap()
    }
    fn append(head: &CanonicalStreamHead, delta: &str) -> CanonicalStreamOperation {
        CanonicalStreamOperation::Append {
            owner: head.owner.clone(),
            expected_head: head.clone(),
            delta: delta.into(),
        }
    }
    #[test]
    fn unicode_replace_close_and_exact_replay() {
        let first = declared();
        let operation = append(&first, "中🙂e\u{301}");
        let second = apply_canonical_stream(Some(&first), &operation).unwrap();
        assert_eq!(second.total_utf8_bytes, "中🙂e\u{301}".len() as u64);
        assert_eq!(second.chunk_count, 1);
        assert_eq!(
            apply_canonical_stream(Some(&second), &operation),
            Err(CanonicalStreamError::HeadConflict)
        );
        let replaced = apply_canonical_stream(
            Some(&second),
            &CanonicalStreamOperation::Replace {
                owner: owner(),
                expected_head: second.clone(),
                text: String::new(),
            },
        )
        .unwrap();
        assert_eq!(
            (
                replaced.generation,
                replaced.revision,
                replaced.total_utf8_bytes,
                replaced.chunk_count
            ),
            (2, 3, 0, 0)
        );
        let closed = apply_canonical_stream(
            Some(&replaced),
            &CanonicalStreamOperation::Close {
                owner: owner(),
                expected_head: replaced.clone(),
            },
        )
        .unwrap();
        assert!(closed.closed);
        assert_eq!(
            apply_canonical_stream(Some(&closed), &append(&closed, "x")),
            Err(CanonicalStreamError::Closed)
        );
        assert_eq!(
            apply_canonical_stream(
                Some(&closed),
                &CanonicalStreamOperation::Replace {
                    owner: owner(),
                    expected_head: closed.clone(),
                    text: "x".into(),
                }
            ),
            Err(CanonicalStreamError::Closed)
        );
    }
    #[test]
    fn owner_field_root_and_counter_attacks_fail() {
        let head = declared();
        for key in ["conversation_id", "run_id", "message_id", "part_id"] {
            let mut value = serde_json::to_value(append(&head, "x")).unwrap();
            value["owner"][key] = "foreign".into();
            let op = serde_json::from_value(value).unwrap();
            assert_eq!(
                apply_canonical_stream(Some(&head), &op),
                Err(CanonicalStreamError::HeadConflict)
            );
        }
        for key in ["generation", "revision", "total_utf8_bytes", "chunk_count"] {
            let mut value = serde_json::to_value(append(&head, "x")).unwrap();
            value["expected_head"][key] = 99.into();
            let op = serde_json::from_value(value).unwrap();
            assert!(apply_canonical_stream(Some(&head), &op).is_err());
        }
        let mut bad = owner();
        bad.field = PartBodyField::ContentJson;
        assert_eq!(
            apply_canonical_stream(None, &CanonicalStreamOperation::Declare { owner: bad }),
            Err(CanonicalStreamError::UnsupportedField)
        );
        let mut bad = head.clone();
        bad.root = "f".repeat(64);
        assert_eq!(
            apply_canonical_stream(Some(&head), &append(&bad, "x")),
            Err(CanonicalStreamError::HeadConflict)
        );
    }
    #[test]
    fn bounded_chunks_empty_input_and_overflow() {
        let head = declared();
        assert_eq!(
            apply_canonical_stream(Some(&head), &append(&head, "")),
            Err(CanonicalStreamError::EmptyDelta)
        );
        assert_eq!(
            apply_canonical_stream(Some(&head), &append(&head, &"x".repeat(32769))),
            Err(CanonicalStreamError::ChunkTooLarge)
        );
        let mut huge = head.clone();
        huge.generation = u64::MAX;
        huge.revision = u64::MAX;
        assert_eq!(
            apply_canonical_stream(Some(&huge), &append(&huge, "x")),
            Err(CanonicalStreamError::Overflow)
        );
        huge = head.clone();
        huge.generation = u64::MAX;
        huge.revision = u64::MAX;
        assert_eq!(
            apply_canonical_stream(
                Some(&huge),
                &CanonicalStreamOperation::Replace {
                    owner: owner(),
                    expected_head: huge.clone(),
                    text: "".into()
                }
            ),
            Err(CanonicalStreamError::Overflow)
        );
        huge = head;
        huge.total_utf8_bytes = u64::MAX;
        huge.chunk_count = u64::MAX.div_ceil(CANONICAL_BODY_CHUNK_MAX_BYTES);
        huge.revision = huge.chunk_count + 1;
        assert_eq!(huge.validate(), Ok(()));
        assert_eq!(
            apply_canonical_stream(Some(&huge), &append(&huge, "x")),
            Err(CanonicalStreamError::Overflow)
        );
    }
    #[test]
    fn wire_is_explicit_and_hash_binds_owner_and_boundaries() {
        let head = declared();
        let op = append(&head, "\n🙂");
        let wire = serde_json::to_value(&op).unwrap();
        assert_eq!(wire["kind"], "append");
        assert_eq!(wire["owner"]["field"], "content_text");
        assert_eq!(
            serde_json::from_value::<CanonicalStreamOperation>(wire.clone()).unwrap(),
            op
        );
        let mut missing = wire;
        missing["owner"].as_object_mut().unwrap().remove("run_id");
        assert!(serde_json::from_value::<CanonicalStreamOperation>(missing).is_err());
        let mut other = owner();
        other.run_id = "other".into();
        let other_head =
            apply_canonical_stream(None, &CanonicalStreamOperation::Declare { owner: other })
                .unwrap();
        assert_ne!(head.root, other_head.root);
        let a = apply_canonical_stream(Some(&head), &append(&head, "ab")).unwrap();
        let b = apply_canonical_stream(Some(&head), &append(&head, "a")).unwrap();
        let b = apply_canonical_stream(Some(&b), &append(&b, "b")).unwrap();
        assert_ne!(a.root, b.root);
        assert_eq!(a.total_utf8_bytes, b.total_utf8_bytes);
    }

    #[test]
    fn malformed_heads_and_unknown_wire_fields_fail_closed() {
        let head = declared();
        for key in ["version", "generation", "revision"] {
            let mut wire = serde_json::to_value(&head).unwrap();
            wire[key] = 0.into();
            let malformed: CanonicalStreamHead = serde_json::from_value(wire).unwrap();
            assert_eq!(malformed.validate(), Err(CanonicalStreamError::InvalidHead));
        }
        for root in ["", "F", &"F".repeat(64), &"g".repeat(64)] {
            let mut malformed = head.clone();
            malformed.root = root.into();
            assert_eq!(malformed.validate(), Err(CanonicalStreamError::InvalidHead));
        }
        let mut wire = serde_json::to_value(&head).unwrap();
        wire["authorization"] = true.into();
        assert!(serde_json::from_value::<CanonicalStreamHead>(wire).is_err());
        let mut wire = serde_json::to_value(append(&head, "x")).unwrap();
        wire["owner"]["source_host"] = "other".into();
        assert!(serde_json::from_value::<CanonicalStreamOperation>(wire).is_err());
        let mut wire = serde_json::to_value(append(&head, "x")).unwrap();
        wire["force"] = true.into();
        assert!(serde_json::from_value::<CanonicalStreamOperation>(wire).is_err());
        assert_eq!(
            apply_canonical_stream(
                Some(&head),
                &CanonicalStreamOperation::Declare { owner: owner() }
            ),
            Err(CanonicalStreamError::HeadConflict)
        );
    }

    #[test]
    fn head_requires_revisions_for_generations_chunks_and_close() {
        let initial = declared();
        for (generation, revision, chunks, closed) in [
            (2, 1, 0, false),
            (1, 1, 1, false),
            (1, 1, 0, true),
            (2, 2, 2, false),
            (2, 2, 1, true),
            (u64::MAX, u64::MAX, 2, false),
            (u64::MAX, u64::MAX, 0, true),
        ] {
            let mut head = initial.clone();
            head.generation = generation;
            head.revision = revision;
            head.chunk_count = chunks;
            head.total_utf8_bytes = chunks;
            head.closed = closed;
            assert_eq!(head.validate(), Err(CanonicalStreamError::InvalidHead));
            assert_eq!(
                apply_canonical_stream(Some(&head), &append(&head, "x")),
                Err(CanonicalStreamError::InvalidHead)
            );
        }
        // Nonempty Replace supplies one chunk at the same revision; it must
        // not be mistaken for an additional Append.
        let replaced = apply_canonical_stream(
            Some(&initial),
            &CanonicalStreamOperation::Replace {
                owner: owner(),
                expected_head: initial.clone(),
                text: "🙂".into(),
            },
        )
        .unwrap();
        assert_eq!(
            (replaced.generation, replaced.revision, replaced.chunk_count),
            (2, 2, 1)
        );
        assert_eq!(replaced.validate(), Ok(()));
        let closed = apply_canonical_stream(
            Some(&replaced),
            &CanonicalStreamOperation::Close {
                owner: owner(),
                expected_head: replaced.clone(),
            },
        )
        .unwrap();
        assert_eq!(closed.validate(), Ok(()));
    }
}
