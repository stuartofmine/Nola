// SPDX-License-Identifier: Apache-2.0
//! Exact changed-item journal inputs; no checkpoint/body scan or ambient grant.

use crate::{params, NolaStore, OptionalExtension, StoreError, StoreResult};
use nola_domain::assistant_journal::AssistantJournalIdentity;
use nola_domain::assistant_journal_operations::{JournalNumber, JournalOrderKey, JournalSection};
use nola_domain::ModelAttemptRecord;
use nola_store_port::assistant_journal::AssistantSourceCommitStamp;
use nola_store_port::assistant_ownership::{
    AssistantJournalPlanningRepository, AssistantOwner, AssistantPartOwner,
};

fn section_key(section: JournalSection) -> i64 {
    match section {
        JournalSection::Transcript => 0,
        JournalSection::Tasks => 1,
        JournalSection::Skills => 2,
        JournalSection::ModelAttempts => 3,
    }
}

impl NolaStore {
    pub fn reserve_assistant_journal_order(
        &mut self,
        stamp: &AssistantSourceCommitStamp,
        identity: &AssistantJournalIdentity,
        section: JournalSection,
        part: &str,
    ) -> StoreResult<JournalOrderKey> {
        crate::require_id("journal_order.part_id", part)?;
        let key = crate::assistant_journal::planning_owner_key(self, stamp, identity)?;
        let section_number = section_key(section);
        let transaction = self.write_transaction()?;
        let previous: Option<(i64,u64)> = transaction.query_row(
            "SELECT section,position FROM assistant_journal_order_reservations WHERE owner=?1 AND part_id=?2",
            params![key,part], |row|Ok((row.get(0)?,row.get(1)?)),
        ).optional()?;
        if let Some((old_section, position)) = previous {
            if old_section != section_number {
                return Err(StoreError::Conflict(
                    "journal Part section reservation is immutable".into(),
                ));
            }
            transaction.commit()?;
            return Ok(JournalOrderKey {
                section,
                position: JournalNumber(position),
            });
        }
        transaction.execute(
            "INSERT OR IGNORE INTO assistant_journal_order_counters(owner,section,next_position) VALUES(?1,?2,1)",
            params![key,section_number],
        )?;
        let position: i64 = transaction.query_row(
            "SELECT next_position FROM assistant_journal_order_counters WHERE owner=?1 AND section=?2",
            params![key,section_number], |row|row.get(0),
        )?;
        let next = position
            .checked_add(1)
            .filter(|value| *value > 1)
            .ok_or_else(|| StoreError::Invariant("journal order counter exhausted".into()))?;
        transaction.execute(
            "INSERT INTO assistant_journal_order_reservations(owner,part_id,section,position) VALUES(?1,?2,?3,?4)",
            params![key,part,section_number,position],
        )?;
        let changed = transaction.execute(
            "UPDATE assistant_journal_order_counters SET next_position=?3 WHERE owner=?1 AND section=?2 AND next_position=?4",
            params![key,section_number,next,position],
        )?;
        if changed != 1 {
            return Err(StoreError::Conflict("journal order counter changed".into()));
        }
        transaction.commit()?;
        Ok(JournalOrderKey {
            section,
            position: JournalNumber(position as u64),
        })
    }

    pub fn model_attempt_successor(
        &self,
        run: &str,
        effect: &str,
        invocation: &str,
        predecessor: &str,
    ) -> StoreResult<Option<ModelAttemptRecord>> {
        for (key, value) in [
            ("run", run),
            ("effect", effect),
            ("invocation", invocation),
            ("predecessor", predecessor),
        ] {
            crate::require_id(key, value)?;
        }
        let Some(parent) = self.model_attempt(predecessor)? else {
            return Ok(None);
        };
        if parent.id != predecessor
            || parent.run_id != run
            || parent.effect_id != effect
            || parent.invocation_id != invocation
        {
            return Err(StoreError::Conflict(
                "Attempt predecessor is outside requested ownership".into(),
            ));
        }
        let child: Option<(String,String,String,String,String)> = self.connection.query_row(
            "SELECT id,run_id,effect_id,invocation_id,record_json FROM run_attempts WHERE predecessor_id=?1",
            [predecessor], |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)),
        ).optional()?;
        let Some((id, actual_run, actual_effect, actual_invocation, json)) = child else {
            return Ok(None);
        };
        let record: ModelAttemptRecord = serde_json::from_str(&json)?;
        if !parent.state.is_terminal()
            || record.id != id
            || actual_run != run
            || actual_effect != effect
            || actual_invocation != invocation
            || record.run_id != run
            || record.effect_id != effect
            || record.invocation_id != invocation
            || record.predecessor_id.as_deref() != Some(predecessor)
        {
            return Err(StoreError::Conflict(
                "Attempt successor ownership or lineage mismatch".into(),
            ));
        }
        Ok(Some(record))
    }
}

impl AssistantJournalPlanningRepository for NolaStore {
    type Error = StoreError;
    fn bind_tool_source(&mut self, stamp: &AssistantSourceCommitStamp,
        identity: &AssistantJournalIdentity, input: &nola_store_port::tool_source::ToolSourceInput) -> StoreResult<()> {
        NolaStore::bind_tool_source(self, stamp, identity, input)
    }
    fn assistant_owner_for_core_turn(&self, conversation: &str, turn: &str) -> StoreResult<Option<AssistantOwner>> {
        NolaStore::assistant_owner_for_core_turn(self, conversation, turn)
    }
    fn assistant_owner_for_turn(
        &self,
        conversation: &str,
        run: &str,
        turn: &str,
    ) -> StoreResult<Option<AssistantOwner>> {
        NolaStore::assistant_owner_for_turn(self, conversation, run, turn)
    }
    fn assistant_owner_for_activity(&self, conversation: &str, run: &str, kind: &str, id: &str) -> StoreResult<Option<AssistantOwner>> {
        NolaStore::assistant_owner_for_activity(self,conversation,run,kind,id)
    }
    fn assistant_owner_for_effect(&self, conversation: &str, run: &str, effect: &str)
        -> StoreResult<Option<AssistantOwner>> {
        NolaStore::assistant_owner_for_effect(self, conversation, run, effect)
    }
    fn assistant_owner_for_part(
        &self,
        conversation: &str,
        run: &str,
        part: &str,
    ) -> StoreResult<Option<AssistantPartOwner>> {
        NolaStore::assistant_owner_for_part(self, conversation, run, part)
    }
    fn reserve_assistant_journal_order(
        &mut self,
        stamp: &AssistantSourceCommitStamp,
        identity: &AssistantJournalIdentity,
        section: JournalSection,
        part: &str,
    ) -> StoreResult<JournalOrderKey> {
        NolaStore::reserve_assistant_journal_order(self, stamp, identity, section, part)
    }
    fn model_attempt_successor(
        &self,
        run: &str,
        effect: &str,
        invocation: &str,
        predecessor: &str,
    ) -> StoreResult<Option<ModelAttemptRecord>> {
        NolaStore::model_attempt_successor(self, run, effect, invocation, predecessor)
    }
}
