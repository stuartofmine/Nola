// SPDX-License-Identifier: Apache-2.0
//! Durable terminal input identity and transition rules. These facts do not
//! authorize input: the Application must validate the current owner and grant.

use crate::assistant_journal::AssistantDigest;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const MAX_TERMINAL_INPUT_BYTES: usize = 64 * 1024;
pub const TERMINAL_INPUT_RECEIPTS_METADATA_KEY: &str = "_nola_terminal_input_receipts";
pub const TERMINAL_INPUT_QUEUE_METADATA_KEY: &str = "_nola_terminal_input_queue";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalInputRequest {
    pub identity: TerminalInputIdentity,
    pub text: String,
}

impl TerminalInputRequest {
    pub fn prepared_receipt(&self) -> Result<TerminalInputReceipt, String> {
        TerminalInputReceipt::prepare(self.identity.clone(), self.text.as_bytes())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalDimensions {
    pub rows: u16,
    pub columns: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalSessionConfiguration {
    pub size: TerminalDimensions,
    /// Set by the admitted Host, never read from model terminal arguments.
    pub host_id: String,
}

impl TerminalSessionConfiguration {
    pub fn validate(&self) -> Result<(), String> {
        self.size.validate()?;
        if self.host_id.trim().is_empty() || self.host_id.len() > 512 || self.host_id.chars().any(char::is_control) {
            return Err("invalid terminal Host identity".into());
        }
        Ok(())
    }
}

impl TerminalDimensions {
    pub fn validate(self) -> Result<(), String> {
        if self.rows == 0 || self.columns == 0 || self.rows > 4096 || self.columns > 4096 {
            return Err("terminal dimensions must be 1..=4096".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalInputIdentity {
    pub host_id: String,
    pub owner_session_id: String,
    pub task_id: String,
    pub attempt_id: String,
    pub worker_generation: String,
    pub request_id: String,
}

impl TerminalInputIdentity {
    pub fn validate(&self) -> Result<(), String> {
        if [&self.host_id, &self.owner_session_id, &self.task_id, &self.attempt_id,
            &self.worker_generation, &self.request_id].iter()
            .any(|value| value.trim().is_empty() || value.len() > 512 || value.chars().any(char::is_control)) {
            return Err("invalid terminal input identity".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalInputPhase {
    Prepared,
    /// Persisted before each physical write. Loss of the worker in this phase
    /// leaves an unknown delivery; it is not permission to resend the suffix.
    Writing,
    /// Exact prefix observed written. A live worker may deliver the suffix only
    /// after another Writing transition has committed.
    Partial,
    Delivered,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalInputReceipt {
    pub identity: TerminalInputIdentity,
    pub content_digest: AssistantDigest,
    pub size_bytes: usize,
    pub acknowledged_bytes: usize,
    pub phase: TerminalInputPhase,
}

impl TerminalInputReceipt {
    pub fn prepare(identity: TerminalInputIdentity, bytes: &[u8]) -> Result<Self, String> {
        identity.validate()?;
        if bytes.is_empty() || bytes.len() > MAX_TERMINAL_INPUT_BYTES {
            return Err("invalid terminal input size".into());
        }
        Ok(Self { identity, content_digest: digest(bytes), size_bytes: bytes.len(),
            acknowledged_bytes: 0, phase: TerminalInputPhase::Prepared })
    }

    pub fn matches_input(&self, identity: &TerminalInputIdentity, bytes: &[u8]) -> bool {
        self.validate().is_ok() && &self.identity == identity
            && bytes.len() == self.size_bytes && digest(bytes) == self.content_digest
    }

    pub fn validate(&self) -> Result<(), String> {
        self.identity.validate()?;
        if self.size_bytes == 0 || self.size_bytes > MAX_TERMINAL_INPUT_BYTES
            || self.acknowledged_bytes > self.size_bytes {
            return Err("invalid terminal input byte accounting".into());
        }
        let valid = match self.phase {
            TerminalInputPhase::Prepared => self.acknowledged_bytes == 0,
            TerminalInputPhase::Writing | TerminalInputPhase::Partial => self.acknowledged_bytes < self.size_bytes,
            TerminalInputPhase::Delivered => self.acknowledged_bytes == self.size_bytes,
            TerminalInputPhase::Unknown => true,
        };
        if !valid { return Err("invalid terminal input phase".into()); }
        Ok(())
    }

    /// Store must compare with its actual previous record inside a transaction.
    /// A caller-supplied predecessor is never proof of committed delivery state.
    pub fn follows(&self, previous: Option<&Self>) -> Result<(), String> {
        self.validate()?;
        let Some(previous) = previous else {
            return if self.phase == TerminalInputPhase::Prepared { Ok(()) }
                else { Err("terminal input requires a prepared receipt".into()) };
        };
        previous.validate()?;
        if self.identity != previous.identity || self.content_digest != previous.content_digest
            || self.size_bytes != previous.size_bytes || self.acknowledged_bytes < previous.acknowledged_bytes {
            return Err("terminal input identity or acknowledged prefix changed".into());
        }
        if self == previous { return Ok(()); }
        use TerminalInputPhase::*;
        let allowed = match (previous.phase, self.phase) {
            (Prepared | Partial, Writing) => self.acknowledged_bytes == previous.acknowledged_bytes,
            (Writing, Partial | Delivered) => true,
            (Prepared | Writing | Partial, Unknown) => self.acknowledged_bytes == previous.acknowledged_bytes,
            _ => false,
        };
        if allowed { Ok(()) } else { Err("terminal input cannot replay or skip its write witness".into()) }
    }
}

fn digest(bytes: &[u8]) -> AssistantDigest {
    AssistantDigest::parse_hex(&format!("{:x}", Sha256::digest(bytes))).expect("SHA-256 digest")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prepared() -> TerminalInputReceipt {
        TerminalInputReceipt::prepare(TerminalInputIdentity { host_id:"host".into(), owner_session_id:"session".into(),
            task_id:"task".into(), attempt_id:"attempt".into(), worker_generation:"generation".into(),
            request_id:"request".into() }, b"run\n").unwrap()
    }

    #[test]
    fn input_binds_owner_generation_and_exact_bytes() {
        let receipt = prepared();
        assert!(receipt.matches_input(&receipt.identity, b"run\n"));
        assert!(!receipt.matches_input(&receipt.identity, b"RUN\n"));
        for identity in [TerminalInputIdentity { owner_session_id:"other".into(), ..receipt.identity.clone() },
            TerminalInputIdentity { worker_generation:"new".into(), ..receipt.identity.clone() }] {
            assert!(!receipt.matches_input(&identity, b"run\n"));
        }
    }

    #[test]
    fn write_intent_and_partial_acknowledgments_cannot_be_skipped() {
        let prepared = prepared();
        prepared.follows(None).unwrap();
        let mut writing = prepared.clone(); writing.phase = TerminalInputPhase::Writing;
        writing.follows(Some(&prepared)).unwrap();
        let mut partial = writing.clone(); partial.phase = TerminalInputPhase::Partial; partial.acknowledged_bytes = 2;
        partial.follows(Some(&writing)).unwrap();
        assert!(partial.follows(Some(&prepared)).is_err());
        let mut next = partial.clone(); next.phase = TerminalInputPhase::Writing;
        next.follows(Some(&partial)).unwrap();
        let mut delivered = next.clone(); delivered.phase = TerminalInputPhase::Delivered; delivered.acknowledged_bytes = 4;
        delivered.follows(Some(&next)).unwrap();
        assert!(next.follows(Some(&delivered)).is_err());
        assert!(delivered.follows(None).is_err());
    }

    #[test]
    fn unknown_delivery_never_resets_or_resends() {
        let receipt = prepared();
        let mut writing = receipt.clone(); writing.phase = TerminalInputPhase::Writing;
        let mut unknown = writing.clone(); unknown.phase = TerminalInputPhase::Unknown;
        unknown.follows(Some(&writing)).unwrap();
        assert!(writing.follows(Some(&unknown)).is_err());
        assert!(receipt.follows(Some(&unknown)).is_err());
        let mut changed = writing.clone(); changed.identity.worker_generation = "new".into();
        assert!(changed.follows(Some(&writing)).is_err());
    }
}
