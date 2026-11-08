// SPDX-License-Identifier: Apache-2.0
//! Core-owned input admission and boundary application; queue position is not identity.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use super::protocol::{AgentInputDelivery, AgentInputOrigin, AgentItem};
const MAX_QUEUED_INPUT_ITEMS: usize = 256;

pub fn input_digest(value: &serde_json::Value) -> String {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(b"NOLA-ACCEPTED-INPUT-V1\0");
    hash.update(super::loop_guard::canonical_json(value));
    format!("{:x}", hash.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn batch(queue: &mut AgentInputQueue, id: &str, delivery: AgentInputDelivery) -> AdmittedInputBatch {
        let items = vec![AgentItem::user_message(super::super::ItemId::from_string(id.into()), "input")];
        queue.admit(id.into(), Some(format!("submission-{id}")), AgentInputOrigin::InteractiveUser,
            delivery, Some("turn".into()), input_digest(&serde_json::json!({"items":items,"delivery":delivery})), items).unwrap()
    }
    #[test]
    fn checkpoint_input_ledger_rejects_conflicting_application_and_queue_sources() {
        let mut queue = AgentInputQueue::default();
        let input = batch(&mut queue, "input", AgentInputDelivery::CurrentTurn);
        queue.enqueue_batch(input.clone()).unwrap();
        queue.validate().unwrap();
        let mut wrong = queue.clone();
        wrong.current_turn[0].admission.payload_digest = "0".repeat(64);
        assert!(wrong.validate().is_err());
        let drained = queue.drain_current_turn();
        queue.apply(&drained[0], "turn").unwrap();
        queue.validate().unwrap();
        assert!(queue.apply(&input, "other-turn").is_err());
        queue.turn_application_sequences.insert("turn".into(), 2);
        assert!(queue.validate().is_err());
    }
    #[test]
    fn admission_overflow_and_capacity_rejection_preserve_ledger() {
        let mut queue = AgentInputQueue::default();
        queue.accepted_sequence = u64::MAX;
        let before = serde_json::to_value(&queue).unwrap();
        assert!(queue.admit("id".into(), None, AgentInputOrigin::Maintenance,
            AgentInputDelivery::CurrentTurn, None, "0".repeat(64), vec![]).is_err());
        assert_eq!(serde_json::to_value(&queue).unwrap(), before);
        let mut queue = AgentInputQueue::default();
        let before = serde_json::to_value(&queue).unwrap();
        let items = (0..257).map(|i| AgentItem::user_message(super::super::ItemId::from_string(i.to_string()), "x")).collect();
        assert!(queue.admit("id".into(), None, AgentInputOrigin::InteractiveUser,
            AgentInputDelivery::CurrentTurn, Some("turn".into()), "0".repeat(64), items).is_err());
        assert_eq!(serde_json::to_value(&queue).unwrap(), before);
    }
    #[test]
    fn input_digest_is_order_stable_and_binds_delivery_and_origin() {
        let one: serde_json::Value = serde_json::from_str(r#"{"origin":"interactive_user","delivery":"current_turn","items":[]}"#).unwrap();
        let two: serde_json::Value = serde_json::from_str(r#"{"items":[],"delivery":"current_turn","origin":"interactive_user"}"#).unwrap();
        assert_eq!(input_digest(&one), input_digest(&two));
        for (key, value) in [("origin", "delegated_agent"), ("delivery", "next_turn")] {
            let mut changed = one.clone(); changed[key] = value.into();
            assert_ne!(input_digest(&one), input_digest(&changed));
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputAdmission {
    pub admission_id: String,
    /// None denotes a direct unkeyed Core input, never an invented user ID.
    pub submission_id: Option<String>,
    pub accepted_sequence: u64,
    pub origin: AgentInputOrigin,
    pub delivery: AgentInputDelivery,
    pub anchor_turn_id: Option<String>,
    pub payload_digest: String,
    pub item_ids: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputApplication {
    pub admission_id: String,
    pub destination_turn_id: String,
    /// Turn-local application order, not a Store message/replay ordinal.
    pub applied_sequence: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdmittedInputBatch {
    pub admission: InputAdmission,
    pub items: Vec<AgentItem>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentInputQueue {
    current_turn: VecDeque<AdmittedInputBatch>,
    next_turn: VecDeque<AdmittedInputBatch>,
    accepted_sequence: u64,
    admissions: BTreeMap<String, InputAdmission>,
    applications: BTreeMap<String, InputApplication>,
    turn_application_sequences: BTreeMap<String, u64>,
    withdrawn: BTreeMap<String, String>,
}
impl AgentInputQueue {
    pub(crate) fn applied_admission(&self, reference: &nola_domain::core_requirements::CoreAppliedInputRef) -> Option<&InputAdmission> {
        self.matches_applied_reference(reference).then(|| self.admissions.get(&reference.admission_id)).flatten()
    }
    pub(crate) fn matches_applied_reference(&self, reference: &nola_domain::core_requirements::CoreAppliedInputRef) -> bool {
        self.applications.get(&reference.admission_id).is_some_and(|applied|
            applied.destination_turn_id == reference.turn_id && applied.applied_sequence == reference.applied_sequence.0)
    }
    pub(crate) fn applied_item_ids(&self, reference:&nola_domain::core_requirements::CoreAppliedInputRef)->Option<&[String]> {
        self.matches_applied_reference(reference).then(||self.admissions.get(&reference.admission_id).map(|admission|admission.item_ids.as_slice())).flatten()
    }
    pub(crate) fn admit(&mut self, admission_id: String, submission_id: Option<String>,
        origin: AgentInputOrigin, delivery: AgentInputDelivery, anchor_turn_id: Option<String>,
        payload_digest: String, items: Vec<AgentItem>) -> Result<AdmittedInputBatch, String> {
        if admission_id.trim().is_empty()
            || submission_id.as_ref().is_some_and(|id| id.trim().is_empty()
                || self.admissions.values().any(|admission| admission.submission_id.as_ref() == Some(id)))
            || anchor_turn_id.as_ref().is_some_and(|id| id.trim().is_empty())
            || payload_digest.len() != 64 || !payload_digest.bytes().all(|b| b.is_ascii_hexdigit())
            || items.iter().any(|item| item.id().as_str().trim().is_empty())
            || items.iter().map(|item| item.id().as_str()).collect::<std::collections::BTreeSet<_>>().len() != items.len() {
            return Err("input admission has invalid or duplicate source identity".into());
        }
        if anchor_turn_id.is_some() && self.len().saturating_add(items.len()) > MAX_QUEUED_INPUT_ITEMS {
            return Err(format!("queued user input exceeds the {MAX_QUEUED_INPUT_ITEMS}-item session limit"));
        }
        if self.admissions.contains_key(&admission_id) { return Err("duplicate input admission".into()); }
        let accepted_sequence = self.accepted_sequence.checked_add(1).filter(|value| *value <= i64::MAX as u64)
            .ok_or("input admission sequence exhausted")?;
        let admission = InputAdmission { admission_id: admission_id.clone(), submission_id,
            accepted_sequence, origin, delivery, anchor_turn_id, payload_digest,
            item_ids: items.iter().map(|item| item.id().as_str().into()).collect() };
        self.admissions.insert(admission_id, admission.clone());
        self.accepted_sequence = accepted_sequence;
        Ok(AdmittedInputBatch { admission, items })
    }
    pub(crate) fn enqueue_batch(&mut self, batch: AdmittedInputBatch) -> Result<usize, String> {
        if self.len().saturating_add(batch.items.len()) > MAX_QUEUED_INPUT_ITEMS {
            return Err(format!("queued user input exceeds the {MAX_QUEUED_INPUT_ITEMS}-item session limit"));
        }
        let count = batch.items.len();
        match batch.admission.delivery {
            AgentInputDelivery::CurrentTurn => self.current_turn.push_back(batch),
            AgentInputDelivery::NextTurn => self.next_turn.push_back(batch),
        }
        Ok(count)
    }
    pub(crate) fn apply(&mut self, batch: &AdmittedInputBatch, turn: &str) -> Result<InputApplication, String> {
        let id = &batch.admission.admission_id;
        if self.admissions.get(id) != Some(&batch.admission) || self.applications.contains_key(id) || self.withdrawn.contains_key(id) {
            return Err("input admission is missing, already applied or withdrawn".into());
        }
        let sequence = self.turn_application_sequences.get(turn).copied().unwrap_or(0)
            .checked_add(1).filter(|value| *value <= i64::MAX as u64).ok_or("input application sequence exhausted")?;
        let value = InputApplication { admission_id: id.clone(), destination_turn_id: turn.into(), applied_sequence: sequence };
        self.turn_application_sequences.insert(turn.into(), sequence);
        self.applications.insert(id.clone(), value.clone());
        Ok(value)
    }
    pub(crate) fn withdraw_current(&mut self, turn: &str, reason: &str) -> Vec<String> {
        let mut ids = Vec::new();
        self.current_turn.retain(|batch| {
            if batch.admission.anchor_turn_id.as_deref() == Some(turn) {
                ids.push(batch.admission.admission_id.clone()); false
            } else { true }
        });
        for id in &ids { self.withdrawn.insert(id.clone(), reason.into()); }
        ids
    }
    pub(crate) fn withdraw_pending(&mut self, reason: &str) -> Vec<InputAdmission> {
        let pending = self.current_turn.drain(..).chain(self.next_turn.drain(..))
            .map(|batch| batch.admission).collect::<Vec<_>>();
        for admission in &pending { self.withdrawn.insert(admission.admission_id.clone(), reason.into()); }
        pending
    }
    pub(crate) fn drain_current_turn(&mut self) -> Vec<AdmittedInputBatch> { self.current_turn.drain(..).collect() }
    pub fn pending_successor(&self) -> Option<&AdmittedInputBatch> { self.next_turn.front() }
    pub(crate) fn take_successor(&mut self, admission_id: &str) -> Result<AdmittedInputBatch, String> {
        if self.next_turn.front().map(|batch| batch.admission.admission_id.as_str()) != Some(admission_id) {
            return Err("successor admission does not match the pending queue frontier".into());
        }
        Ok(self.next_turn.pop_front().expect("checked queue frontier"))
    }
    pub(crate) fn has_current_turn_input(&self) -> bool { !self.current_turn.is_empty() }
    pub(crate) fn has_next_turn_input(&self) -> bool { !self.next_turn.is_empty() }
    pub(crate) fn len(&self) -> usize { self.current_turn.iter().chain(self.next_turn.iter()).map(|b| b.items.len()).sum() }
    pub(crate) fn validate(&self) -> Result<(), String> {
        let mut sequences = std::collections::BTreeSet::new();
        let mut submissions = std::collections::BTreeSet::new();
        for (id, admission) in &self.admissions {
            if id.trim().is_empty() || &admission.admission_id != id || admission.accepted_sequence == 0
                || admission.accepted_sequence > i64::MAX as u64
                || admission.submission_id.as_ref().is_some_and(|id| id.trim().is_empty() || !submissions.insert(id))
                || admission.anchor_turn_id.as_ref().is_some_and(|id| id.trim().is_empty())
                || admission.item_ids.iter().any(|id| id.trim().is_empty())
                || admission.item_ids.iter().collect::<std::collections::BTreeSet<_>>().len() != admission.item_ids.len()
                || admission.accepted_sequence > self.accepted_sequence || !sequences.insert(admission.accepted_sequence)
                || admission.payload_digest.len() != 64 || !admission.payload_digest.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("input admission identity or sequence is inconsistent".into());
            }
        }
        if sequences.len() as u64 != self.accepted_sequence { return Err("input admission sequence has a gap".into()); }
        let mut queued = std::collections::BTreeSet::new();
        for (delivery, batches) in [(AgentInputDelivery::CurrentTurn, &self.current_turn), (AgentInputDelivery::NextTurn, &self.next_turn)] {
            for batch in batches {
                let id = &batch.admission.admission_id;
                if self.admissions.get(id) != Some(&batch.admission) || batch.admission.delivery != delivery
                    || batch.admission.anchor_turn_id.is_none()
                    || !queued.insert(id) || self.applications.contains_key(id) || self.withdrawn.contains_key(id)
                    || batch.admission.item_ids != batch.items.iter().map(|item| item.id().as_str().to_owned()).collect::<Vec<_>>() {
                    return Err("queued input provenance is inconsistent".into());
                }
            }
        }
        let mut per_turn = BTreeMap::<&str, std::collections::BTreeSet<u64>>::new();
        for (id, application) in &self.applications {
            if id != &application.admission_id || !self.admissions.contains_key(id) || self.withdrawn.contains_key(id)
                || application.destination_turn_id.trim().is_empty() || application.applied_sequence == 0
                || application.applied_sequence > i64::MAX as u64
                || !per_turn.entry(&application.destination_turn_id).or_default().insert(application.applied_sequence) {
                return Err("input application provenance is inconsistent".into());
            }
        }
        if per_turn.len() != self.turn_application_sequences.len() { return Err("input application counter differs".into()); }
        for (turn, sequences) in per_turn {
            if self.turn_application_sequences.get(turn).copied() != Some(sequences.len() as u64)
                || sequences.last().copied() != Some(sequences.len() as u64) {
                return Err("input application sequence has a gap".into());
            }
        }
        if self.withdrawn.keys().any(|id| !self.admissions.contains_key(id)) {
            return Err("withdrawn input has no admission".into());
        }
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn enqueue(&mut self, delivery: AgentInputDelivery, items: impl IntoIterator<Item=AgentItem>) -> Result<usize,String> {
        let batch = self.admit(format!("fixture-{}", self.accepted_sequence + 1), None,
            AgentInputOrigin::InteractiveUser, delivery, Some("fixture-turn".into()), input_digest(&serde_json::json!("fixture")), items.into_iter().collect())?;
        self.enqueue_batch(batch)
    }
}
