// SPDX-License-Identifier: Apache-2.0
//! Compact model Part ownership survives cleanup of large outbox payloads.

use super::{NolaStore, StoreError, StoreResult};
use rusqlite::{params, OptionalExtension};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredModelRequestParts {
    pub run_id: String,
    pub attempt_id: String,
    pub request_id: String,
    pub text_part_id: String,
    pub reasoning_part_id: String,
}

impl NolaStore {
    /// Resolves an indexed compact binding within the requested conversation.
    pub fn model_text_part_owner(
        &self,
        conversation_id: &str,
        part_id: &str,
    ) -> StoreResult<Option<StoredModelRequestParts>> {
        if conversation_id.trim().is_empty() || part_id.trim().is_empty() {
            return Err(StoreError::Invariant(
                "model Part owner scope is empty".into(),
            ));
        }
        let mut statement = self.connection.prepare(
            "SELECT owner.run_id, COALESCE((SELECT request_id FROM run_attempt_deliveries WHERE attempt_id=owner.id ORDER BY request_id LIMIT 1),''), owner.text_part_id, owner.reasoning_part_id, owner.id
             FROM run_attempts AS owner JOIN runs ON runs.id = owner.run_id
             WHERE owner.text_part_id = ?1 AND runs.conversation_id = ?2 LIMIT 2"
        )?;
        let rows = statement
            .query_map(params![part_id, conversation_id], |row| {
                Ok(StoredModelRequestParts {
                    run_id: row.get(0)?,
                    request_id: row.get(1)?,
                    attempt_id: row.get(4)?,
                    text_part_id: row.get(2)?,
                    reasoning_part_id: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        if rows.len() > 1 {
            return Err(StoreError::Invariant(
                "text Part has ambiguous model request ownership".into(),
            ));
        }
        Ok(rows.into_iter().next())
    }

    /// Reads compact durable ownership by primary key, including completed requests.
    /// Neither SQLite nor Rust needs to parse the original model input here.
    pub fn model_request_parts(
        &self,
        request_id: &str,
    ) -> StoreResult<Option<StoredModelRequestParts>> {
        if request_id.trim().is_empty() {
            return Err(StoreError::Invariant(
                "model request identity is empty".into(),
            ));
        }
        Ok(self.connection.query_row(
            "SELECT a.run_id, d.request_id, a.text_part_id, a.reasoning_part_id, a.id
             FROM run_attempt_deliveries d JOIN run_attempts a ON a.id=d.attempt_id WHERE d.request_id = ?1",
            params![request_id],
            |row| Ok(StoredModelRequestParts {
                run_id: row.get(0)?, request_id: row.get(1)?, attempt_id: row.get(4)?,
                text_part_id: row.get(2)?, reasoning_part_id: row.get(3)?,
            }),
        ).optional()?)
    }
}
