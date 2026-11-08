// SPDX-License-Identifier: Apache-2.0
//! Narrow, immutable scheduled occurrence facts in the caller's read snapshot.
use crate::{NolaStore, StoreError};
use nola_store_port::automation_facts::{AutomationFactsRepository, QueuedAutomationFact};

impl AutomationFactsRepository for NolaStore {
    type Error = StoreError;
    fn queued_automation_facts(&self, limit: usize) -> Result<Vec<QueuedAutomationFact>, StoreError> {
        if !(1..=51).contains(&limit) {
            return Err(StoreError::Invariant("queued automation fact limit must be between 1 and 51".into()));
        }
        let mut query = self.connection.prepare(
            "SELECT r.id,r.automation_id,r.definition_revision,d.name,r.scheduled_for_ms
             FROM automation_runs r LEFT JOIN automation_definitions d
               ON d.id=r.automation_id AND d.revision=r.definition_revision
             WHERE r.status='queued' ORDER BY r.scheduled_for_ms,r.id LIMIT ?1")?;
        let rows = query.query_map([limit as i64], |row| Ok(QueuedAutomationFact {
            occurrence_id: row.get(0)?, automation_id: row.get(1)?,
            definition_revision: row.get(2)?, title: row.get(3)?, scheduled_for_ms: row.get(4)?,
        }))?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}
