// SPDX-License-Identifier: Apache-2.0
//! Revisioned conversation plan persistence, compare-and-swap updates, and history.

use super::*;

impl NolaStore {
    pub fn save_conversation_plan(
        &mut self,
        conversation_id: &str,
        sop_id: &str,
        sop_version: u32,
        run_id: &str,
        todos_json: &str,
        updated_at_ms: i64,
    ) -> StoreResult<StoredConversationPlan> {
        require_id("conversation_plan.conversation_id", conversation_id)?;
        require_id("conversation_plan.sop_id", sop_id)?;
        require_id("conversation_plan.run_id", run_id)?;
        let todos: serde_json::Value = serde_json::from_str(todos_json)?;
        if !todos.is_array() || sop_version == 0 {
            return Err(StoreError::Invariant(
                "conversation_plan.todos_json must be a JSON array".to_string(),
            ));
        }
        let transaction = self.write_transaction()?;
        let current: Option<(i64, Option<i64>, String, u32, String)> = transaction
            .query_row(
                "SELECT revision,frozen_revision,sop_id,sop_version,run_id FROM conversation_plans WHERE conversation_id=?1",
                params![conversation_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .optional()?;
        if current.is_some() {
            return Err(StoreError::Invariant("conversation plan already exists; use a caller-versioned update".into()));
        }
        let next_revision = 1;
        transaction.execute(
            "INSERT INTO conversation_plans(conversation_id,sop_id,sop_version,run_id,todos_json,revision,frozen_revision,updated_at_ms)
             VALUES(?1,?2,?3,?4,?5,1,NULL,?6)",
            params![conversation_id, sop_id, sop_version, run_id, todos_json, updated_at_ms],
        )?;
        transaction.execute(
            "INSERT INTO conversation_plan_revisions(
                conversation_id,revision,sop_id,sop_version,run_id,todos_json,change_kind,change_summary,updated_at_ms
             ) VALUES(?1,?2,?3,?4,?5,?6,'save','',?7)",
            params![conversation_id, next_revision, sop_id, sop_version, run_id, todos_json, updated_at_ms],
        )?;
        transaction.commit()?;
        self.conversation_plan(conversation_id)?.ok_or_else(|| {
            StoreError::Invariant("saved conversation plan is unavailable".to_string())
        })
    }

    pub fn conversation_plan(
        &self,
        conversation_id: &str,
    ) -> StoreResult<Option<StoredConversationPlan>> {
        require_id("conversation_plan.conversation_id", conversation_id)?;
        self.connection
            .query_row(
                "SELECT conversation_id,sop_id,sop_version,run_id,todos_json,revision,frozen_revision,updated_at_ms
                 FROM conversation_plans WHERE conversation_id=?1",
                params![conversation_id],
                |row| {
                    Ok(StoredConversationPlan {
                        conversation_id: row.get(0)?,
                        sop_id: row.get(1)?,
                        sop_version: row.get(2)?,
                        run_id: row.get(3)?,
                        todos_json: row.get(4)?,
                        revision: row.get(5)?,
                        frozen_revision: row.get(6)?,
                        updated_at_ms: row.get(7)?,
                    })
                },
            )
            .optional()
            .map_err(StoreError::from)
    }

    pub fn save_conversation_plan_if_revision(
        &mut self,
        conversation_id: &str,
        sop_id: &str,
        sop_version: u32,
        run_id: &str,
        todos_json: &str,
        expected_revision: i64,
        updated_at_ms: i64,
    ) -> StoreResult<Option<StoredConversationPlan>> {
        self.save_conversation_plan_revision_if_revision(
            conversation_id,
            sop_id,
            sop_version,
            run_id,
            todos_json,
            expected_revision,
            None,
            "approval",
            "",
            updated_at_ms,
        )
    }

    pub fn save_conversation_plan_revision_if_revision(
        &mut self,
        conversation_id: &str,
        sop_id: &str,
        sop_version: u32,
        run_id: &str,
        todos_json: &str,
        expected_revision: i64,
        frozen_revision: Option<Option<i64>>,
        change_kind: &str,
        change_summary: &str,
        updated_at_ms: i64,
    ) -> StoreResult<Option<StoredConversationPlan>> {
        require_id("conversation_plan.conversation_id", conversation_id)?;
        require_id("conversation_plan.sop_id", sop_id)?;
        require_id("conversation_plan.run_id", run_id)?;
        let todos: serde_json::Value = serde_json::from_str(todos_json)?;
        if !todos.is_array()
            || sop_version == 0
            || expected_revision <= 0
            || !matches!(
                change_kind,
                "save" | "approval" | "edit" | "freeze" | "replan"
            )
        {
            return Err(StoreError::Invariant(
                "conversation plan CAS requires an array, positive revision, and valid change kind"
                    .to_string(),
            ));
        }
        let transaction = self.write_transaction()?;
        let current_identity: Option<(String, u32, String)> = transaction
            .query_row(
                "SELECT sop_id,sop_version,run_id FROM conversation_plans WHERE conversation_id=?1 AND revision=?2",
                params![conversation_id, expected_revision],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        if let Some((current_sop_id, current_sop_version, current_run_id)) = &current_identity {
            if change_kind == "replan" {
                if current_run_id == run_id {
                    return Err(StoreError::Invariant(
                        "conversation plan replan requires a new run id".to_string(),
                    ));
                }
            } else if current_sop_id != sop_id
                || *current_sop_version != sop_version
                || current_run_id != run_id
            {
                return Err(StoreError::Invariant(
                    "conversation plan SOP identity cannot change outside replan".to_string(),
                ));
            }
        }
        let updated = match frozen_revision {
            Some(frozen_revision) => transaction.execute(
                "UPDATE conversation_plans
                 SET sop_id=?2,sop_version=?3,run_id=?4,todos_json=?5,revision=revision+1,frozen_revision=?6,updated_at_ms=?7
                 WHERE conversation_id=?1 AND revision=?8",
                params![
                    conversation_id,
                    sop_id,
                    sop_version,
                    run_id,
                    todos_json,
                    frozen_revision,
                    updated_at_ms,
                    expected_revision
                ],
            )?,
            None => transaction.execute(
                "UPDATE conversation_plans
                 SET sop_id=?2,sop_version=?3,run_id=?4,todos_json=?5,revision=revision+1,updated_at_ms=?6
                 WHERE conversation_id=?1 AND revision=?7",
                params![
                    conversation_id,
                    sop_id,
                    sop_version,
                    run_id,
                    todos_json,
                    updated_at_ms,
                    expected_revision
                ],
            )?,
        };
        if updated == 0 {
            transaction.commit()?;
            return Ok(None);
        }
        transaction.execute(
            "INSERT INTO conversation_plan_revisions(
                conversation_id,revision,sop_id,sop_version,run_id,todos_json,change_kind,change_summary,updated_at_ms
             ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                conversation_id,
                expected_revision + 1,
                sop_id,
                sop_version,
                run_id,
                todos_json,
                change_kind,
                change_summary,
                updated_at_ms
            ],
        )?;
        transaction.commit()?;
        self.conversation_plan(conversation_id)
    }

    pub fn conversation_plan_revisions(
        &self,
        conversation_id: &str,
    ) -> StoreResult<Vec<StoredConversationPlanRevision>> {
        require_id("conversation_plan.conversation_id", conversation_id)?;
        let mut statement = self.connection.prepare(
            "SELECT conversation_id,sop_id,sop_version,run_id,todos_json,revision,change_kind,change_summary,updated_at_ms
             FROM conversation_plan_revisions WHERE conversation_id=?1 ORDER BY revision ASC",
        )?;
        let rows = statement.query_map(params![conversation_id], |row| {
            Ok(StoredConversationPlanRevision {
                conversation_id: row.get(0)?,
                sop_id: row.get(1)?,
                sop_version: row.get(2)?,
                run_id: row.get(3)?,
                todos_json: row.get(4)?,
                revision: row.get(5)?,
                change_kind: row.get(6)?,
                change_summary: row.get(7)?,
                updated_at_ms: row.get(8)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }
}
