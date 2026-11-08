// SPDX-License-Identifier: Apache-2.0
//! Immutable completion frontier, distinct from the mutable presentation head.
use crate::assistant_journal::{AssistantDigest, AssistantJournalCursor, AssistantJournalIdentity};
use crate::assistant_journal_operations::JournalNumber;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantRunTerminalStatus {
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AssistantRunTerminalManifestEntry {
    Assistant {
        through: AssistantJournalCursor,
        ordinal: i64,
        created_at_ms: i64,
        state: AssistantRunTerminalStatus,
        completed_at_ms: i64,
    },
    ChildReceipt {
        task_id: String,
        receipt_id: String,
        parent_turn_id: String,
        child_session_id: Option<String>,
        child_run_id: Option<String>,
        input_watermark: Option<i64>,
        completion_watermark: Option<i64>,
        status: String,
        receipt_digest: AssistantDigest,
        result_digest: AssistantDigest,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantRunTerminalSeal {
    pub protocol_version: u16,
    pub identity: AssistantJournalIdentity,
    pub core_session_id: String,
    pub transition_id: String,
    pub checkpoint_revision: JournalNumber,
    pub core_revision: JournalNumber,
    pub event_cursor: JournalNumber,
    pub result: crate::run_result::RunResult,
    pub result_digest: AssistantDigest,
    pub completed_at_ms: i64,
    pub admission: crate::run_message_admission::RunMessageAdmissionCursor,
    pub entry_count: JournalNumber,
    pub heads_count: JournalNumber,
    pub child_receipt_count: JournalNumber,
    pub manifest_digest: AssistantDigest,
    pub seal_digest: AssistantDigest,
}

pub fn terminal_manifest_digest(
    entries: &[AssistantRunTerminalManifestEntry],
) -> Result<AssistantDigest, String> {
    digest(b"NOLA-RUN-TERMINAL-MANIFEST-V2", entries)
}

pub fn terminal_manifest_canonical_bytes(
    entries: &[AssistantRunTerminalManifestEntry],
) -> Result<Vec<u8>, String> {
    serde_json::to_vec(entries).map_err(|e| e.to_string())
}

/// Fixed-order typed JSON with a versioned domain separator. No maps, floats,
/// client formatting or unordered canonical objects enter this digest.
fn digest<T: Serialize + ?Sized>(tag: &[u8], value: &T) -> Result<AssistantDigest, String> {
    let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    hash.update(tag);
    hash.update((bytes.len() as u64).to_be_bytes());
    hash.update(bytes);
    AssistantDigest::parse_hex(&format!("{:x}", hash.finalize())).map_err(|e| e.to_string())
}
/// First pass over an immutable manifest: typed JSON array byte length, without
/// retaining entries. The existing digest prefixes this length before content.
pub struct TerminalManifestJsonLength {
    bytes: u64,
    count: u64,
}
impl Default for TerminalManifestJsonLength {
    fn default() -> Self {
        Self { bytes: 2, count: 0 }
    }
}
impl TerminalManifestJsonLength {
    pub fn observe(&mut self, entry: &AssistantRunTerminalManifestEntry) -> Result<(), String> {
        let bytes = serde_json::to_vec(entry).map_err(|error| error.to_string())?;
        self.bytes = self
            .bytes
            .checked_add(bytes.len() as u64)
            .and_then(|length| length.checked_add(u64::from(self.count > 0)))
            .ok_or("terminal manifest JSON length overflow")?;
        self.count = self
            .count
            .checked_add(1)
            .ok_or("terminal manifest count overflow")?;
        Ok(())
    }
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
}

/// Second pass validates the unchanged digest format and exact ownership.
/// Only duplicate-detection identities are retained, never full entries/body.
pub struct TerminalManifestValidator<'a> {
    seal: &'a AssistantRunTerminalSeal,
    hash: Sha256,
    expected_bytes: u64,
    length: TerminalManifestJsonLength,
    owners: std::collections::BTreeSet<String>,
    tasks: std::collections::BTreeSet<String>,
    last_ordinal: Option<i64>,
}
impl<'a> TerminalManifestValidator<'a> {
    pub fn new(seal: &'a AssistantRunTerminalSeal, json_bytes: u64) -> Result<Self, String> {
        seal.validate()?;
        let mut hash = Sha256::new();
        hash.update(b"NOLA-RUN-TERMINAL-MANIFEST-V2");
        hash.update(json_bytes.to_be_bytes());
        hash.update(b"[");
        Ok(Self {
            seal,
            hash,
            expected_bytes: json_bytes,
            length: Default::default(),
            owners: Default::default(),
            tasks: Default::default(),
            last_ordinal: None,
        })
    }
    pub fn observe(&mut self, entry: &AssistantRunTerminalManifestEntry) -> Result<(), String> {
        if self.length.count >= self.seal.entry_count.0 {
            return Err("terminal manifest exceeds entry count".into());
        }
        match entry {
            AssistantRunTerminalManifestEntry::Assistant {
                through,
                ordinal,
                created_at_ms,
                completed_at_ms,
                ..
            } => {
                through
                    .identity
                    .validate()
                    .map_err(|error| error.to_string())?;
                if !self.tasks.is_empty()
                    || through.identity.source_store_epoch != self.seal.identity.source_store_epoch
                    || through.identity.conversation_id != self.seal.identity.conversation_id
                    || through.identity.run_id != self.seal.identity.run_id
                    || (through.sequence == 0
                        && through.identity.assistant_message_id == self.seal.identity.assistant_message_id
                        && !matches!(self.seal.result.value,
                        crate::run_result::RunResultValue::Blocked { .. }))
                    || *ordinal < 0
                    || *created_at_ms < 0
                    || *completed_at_ms < 0
                    || self.last_ordinal.is_some_and(|old| old >= *ordinal)
                    || !self
                        .owners
                        .insert(through.identity.assistant_message_id.clone())
                {
                    return Err("terminal manifest assistant ownership/order differs".into());
                }
                self.last_ordinal = Some(*ordinal);
            }
            AssistantRunTerminalManifestEntry::ChildReceipt {
                task_id,
                receipt_id,
                parent_turn_id,
                child_session_id,
                child_run_id,
                status,
                ..
            } => {
                if task_id.is_empty()
                    || receipt_id.is_empty()
                    || parent_turn_id.is_empty()
                    || child_session_id.as_deref().is_none_or(|id|id.trim().is_empty())
                    || child_run_id.as_deref().is_none_or(|id|id.trim().is_empty())
                    || status != "sealed"
                    || !self.tasks.insert(task_id.clone())
                {
                    return Err("terminal manifest child evidence differs".into());
                }
            }
        }
        if self.length.count > 0 {
            self.hash.update(b",");
        }
        self.hash
            .update(serde_json::to_vec(entry).map_err(|error| error.to_string())?);
        self.length.observe(entry)?;
        Ok(())
    }
    pub fn finish(mut self) -> Result<(), String> {
        self.hash.update(b"]");
        let digest = AssistantDigest::parse_hex(&format!("{:x}", self.hash.finalize()))
            .map_err(|error| error.to_string())?;
        if self.length.count != self.seal.entry_count.0
            || self.length.bytes() != self.expected_bytes
            || digest != self.seal.manifest_digest
        {
            return Err("terminal manifest count or digest differs".into());
        }
        if self.owners.len() as u64 != self.seal.heads_count.0
            || self.tasks.len() as u64 != self.seal.child_receipt_count.0
            || !self
                .owners
                .contains(&self.seal.identity.assistant_message_id)
        {
            return Err("terminal manifest owner counts/root differ".into());
        }
        Ok(())
    }
}

impl AssistantRunTerminalSeal {
    /// Validate the fully collected, ordered manifest before staging ownership.
    pub fn validate_manifest(
        &self,
        entries: &[AssistantRunTerminalManifestEntry],
    ) -> Result<(), String> {
        let mut length = TerminalManifestJsonLength::default();
        for entry in entries {
            length.observe(entry)?;
        }
        let mut validator = TerminalManifestValidator::new(self, length.bytes())?;
        for entry in entries {
            validator.observe(entry)?;
        }
        validator.finish()
    }
    pub fn computed_digest(&self) -> Result<AssistantDigest, String> {
        digest(
            b"NOLA-RUN-TERMINAL-SEAL-V2",
            &(
                self.protocol_version,
                &self.identity,
                &self.core_session_id,
                &self.transition_id,
                self.checkpoint_revision,
                self.core_revision,
                self.event_cursor,
                &self.result_digest,
                self.completed_at_ms,
                &self.admission,
                self.entry_count,
                self.heads_count,
                self.child_receipt_count,
                self.manifest_digest,
            ),
        )
    }
    pub fn validate(&self) -> Result<(), String> {
        self.identity.validate().map_err(|e| e.to_string())?;
        self.result.validate()?;
        if self.protocol_version != 2
            || self.result.run_id != self.identity.run_id
            || self.result.conversation_id != self.identity.conversation_id
            || self.result.computed_digest()? != self.result_digest
            || self.result.decision_event_cursor.0 > self.event_cursor.0
            || self.result.decided_at_revision.0 > self.core_revision.0
            || self.core_session_id.trim().is_empty()
            || self.transition_id.trim().is_empty()
            || self.checkpoint_revision.0 == 0
            || self.core_revision.0 == 0
            || self.completed_at_ms < 0
            || self.admission.identity != self.identity
            || self.admission.sequence.0 == 0
            || self.heads_count.0 == 0
            || self.heads_count.0.checked_add(self.child_receipt_count.0)
                != Some(self.entry_count.0)
            || self.computed_digest()? != self.seal_digest
        {
            return Err("invalid assistant Run terminal seal".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantRunTerminalManifestPage {
    pub seal: AssistantRunTerminalSeal,
    pub after_entry: JournalNumber,
    pub next_entry: JournalNumber,
    pub complete: bool,
    pub entries: Vec<AssistantRunTerminalManifestEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantRunTerminalReception {
    pub seal: AssistantRunTerminalSeal,
    pub next_entry: JournalNumber,
    pub manifest_complete: bool,
    pub committed: bool,
}

#[cfg(test)]
mod streaming_tests {
    use super::*;

    #[test]
    fn incremental_manifest_preserves_wire_digest_and_rejects_tampering() {
        let identity = AssistantJournalIdentity {
            source_store_epoch: "epoch".into(),
            conversation_id: "conversation".into(),
            run_id: "run".into(),
            assistant_message_id: "root".into(),
        };
        let digest = AssistantDigest::parse_hex(&"a".repeat(64)).unwrap();
        let entries: Vec<_> = ["root", "unicode-中🙂"]
            .into_iter()
            .enumerate()
            .map(|(ordinal, id)| {
                let mut owner = identity.clone();
                owner.assistant_message_id = id.into();
                AssistantRunTerminalManifestEntry::Assistant {
                    through: AssistantJournalCursor {
                        identity: owner,
                        sequence: 1,
                        record_digest: digest,
                    },
                    ordinal: ordinal as i64,
                    created_at_ms: 1,
                    completed_at_ms: 2,
                    state: AssistantRunTerminalStatus::Completed,
                }
            })
            .collect();
        let result = crate::run_result::RunResult {
            result_id: "result".into(), run_id: "run".into(), conversation_id: "conversation".into(),
            originating_submission_id: "submission".into(),
            input: crate::run_input_commitment::RunInputCommitment {
                watermark: JournalNumber(0),
                applications_digest: crate::run_input_commitment::RunInputCommitment::empty_applications_digest(),
            },
            decided_at_revision: JournalNumber(1), decision_event_cursor: JournalNumber(2),
            requirement_revision: JournalNumber(1), evidence_frontier: JournalNumber(0),
            value: crate::run_result::RunResultValue::Answered { answer_part_refs: vec!["text".into()] },
        };
        let mut seal = AssistantRunTerminalSeal {
            protocol_version: 2,
            identity: identity.clone(),
            core_session_id: "core".into(),
            transition_id: "terminal".into(),
            checkpoint_revision: JournalNumber(1),
            core_revision: JournalNumber(1),
            event_cursor: JournalNumber(2),
            result_digest: result.computed_digest().unwrap(),
            result,
            completed_at_ms: 2,
            admission: crate::run_message_admission::RunMessageAdmissionCursor {
                identity,
                sequence: JournalNumber(1),
                record_digest: digest,
            },
            entry_count: JournalNumber(2),
            heads_count: JournalNumber(2),
            child_receipt_count: JournalNumber(0),
            manifest_digest: terminal_manifest_digest(&entries).unwrap(),
            seal_digest: digest,
        };
        seal.seal_digest = seal.computed_digest().unwrap();
        seal.validate().unwrap();
        for damage in ["version", "result", "core_revision", "owner"] {
            let mut changed=seal.clone();
            match damage {
                "version" => changed.protocol_version=1,
                "result" => changed.result.result_id="substituted".into(),
                "core_revision" => changed.core_revision=JournalNumber(0),
                _ => changed.result.run_id="another-run".into(),
            }
            changed.seal_digest=changed.computed_digest().unwrap();
            assert!(changed.validate().is_err(),"{damage}");
        }
        let mut old=serde_json::to_value(&seal).unwrap();
        old.as_object_mut().unwrap().remove("result");
        assert!(serde_json::from_value::<AssistantRunTerminalSeal>(old).is_err());
        let mut length = TerminalManifestJsonLength::default();
        for entry in &entries {
            length.observe(entry).unwrap();
        }
        assert_eq!(
            length.bytes(),
            terminal_manifest_canonical_bytes(&entries).unwrap().len() as u64
        );
        let mut validator = TerminalManifestValidator::new(&seal, length.bytes()).unwrap();
        for entry in &entries {
            validator.observe(entry).unwrap();
        }
        validator.finish().unwrap();
        seal.validate_manifest(&entries).unwrap();
        let mut wrong_length = TerminalManifestValidator::new(&seal, length.bytes() + 1).unwrap();
        for entry in &entries {
            wrong_length.observe(entry).unwrap();
        }
        assert!(wrong_length.finish().is_err());
        let mut missing = TerminalManifestValidator::new(&seal, length.bytes()).unwrap();
        missing.observe(&entries[0]).unwrap();
        assert!(missing.finish().is_err());
        let mut duplicate = TerminalManifestValidator::new(&seal, length.bytes()).unwrap();
        duplicate.observe(&entries[0]).unwrap();
        assert!(duplicate.observe(&entries[0]).is_err());
        let mut changed = entries.clone();
        if let AssistantRunTerminalManifestEntry::Assistant {
            completed_at_ms, ..
        } = &mut changed[1]
        {
            *completed_at_ms += 1;
        }
        assert!(seal.validate_manifest(&changed).is_err());
        // An applied-input segment may terminate before publishing its first Part.
        // Its Run admission is checked by the receiving Store, not guessed here.
        let mut empty_segment = entries.clone();
        if let AssistantRunTerminalManifestEntry::Assistant { through, .. } = &mut empty_segment[1] {
            *through = AssistantJournalCursor::initial(through.identity.clone()).unwrap();
        }
        let mut empty_seal = seal.clone();
        empty_seal.manifest_digest = terminal_manifest_digest(&empty_segment).unwrap();
        empty_seal.seal_digest = empty_seal.computed_digest().unwrap();
        empty_seal.validate_manifest(&empty_segment).unwrap();
        for index in [0, 1] {
            let mut invalid = empty_segment.clone();
            if let AssistantRunTerminalManifestEntry::Assistant { through, .. } = &mut invalid[index] {
                if index == 0 {
                    *through = AssistantJournalCursor::initial(through.identity.clone()).unwrap();
                } else {
                    through.identity.run_id = "foreign-run".into();
                }
            }
            let mut invalid_seal = empty_seal.clone();
            invalid_seal.manifest_digest = terminal_manifest_digest(&invalid).unwrap();
            invalid_seal.seal_digest = invalid_seal.computed_digest().unwrap();
            assert!(invalid_seal.validate_manifest(&invalid).is_err());
        }
    }
}
