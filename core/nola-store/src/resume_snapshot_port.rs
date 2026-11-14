// SPDX-License-Identifier: Apache-2.0
//! One connection for candidate selection, recovery facts and mailbox CAS.
use crate::{NolaStore, StoreError, StoreResult};
use nola_store_port::consistent_read::ConsistentReadRepository;
use nola_store_port::resume_snapshot::*;
impl ConsistentReadRepository for NolaStore {
    fn with_consistent_read<T>(
        &mut self,
        reader: impl FnOnce(&Self) -> Result<T, String>,
    ) -> Result<T, String> {
        NolaStore::with_consistent_read(self, reader)
    }

    fn with_consistent_read_mut<T>(
        &mut self,
        reader: impl FnOnce(&mut Self) -> Result<T, String>,
    ) -> Result<T, String> {
        NolaStore::with_consistent_read_mut(self, reader)
    }
}
impl ResumeSnapshotRepository for NolaStore {
    type ResumeError = StoreError;
    fn resumable_conversation_ids(&self, root: &str) -> StoreResult<Vec<String>> {
        let mut statement=self.connection.prepare("SELECT c.id FROM conversations c
            JOIN conversation_settings s ON s.conversation_id=c.id
            JOIN core_checkpoints h ON h.conversation_id=c.id
            LEFT JOIN core_checkpoint_bodies k ON k.conversation_id=h.conversation_id
            WHERE CASE WHEN json_valid(s.settings_json) THEN
                json_type(s.settings_json,'$.project_root')='text' AND json_extract(s.settings_json,'$.project_root')=?1 ELSE 0 END
              AND CASE WHEN k.checkpoint_json IS NULL OR NOT json_valid(k.checkpoint_json) THEN 1
                  ELSE json_type(k.checkpoint_json,'$.state.goal_projection.active')='object'
                    AND json_extract(k.checkpoint_json,'$.state.goal_projection.active.status')
                        NOT IN ('completed','cancelled') END
            ORDER BY c.updated_at_ms DESC,c.revision DESC,c.rowid DESC
            LIMIT 1")?;
        let rows = statement.query_map([root], |row| row.get(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
    fn load_resume_facts(
        &self,
        id: &str,
        root: &str,
        host: Option<&str>,
    ) -> StoreResult<ResumeSnapshotFacts> {
        // The outer ConsistentReadRepository scope also includes candidate and
        // session selection. These existing readers all use self.connection.
        Ok(ResumeSnapshotFacts {
            active_run: self.active_run(id)?,
            runtime_tasks: self.list_runtime_tasks(root)?,
            permission_grants: self.effective_permission_grants(Some(id), host)?,
            pending_permissions: self.pending_permissions_for_owner(id)?,
            incomplete_effects: self.incomplete_work_step_transactions(None)?,
        })
    }
    fn acknowledge_runtime_task_reads(
        &mut self,
        input: &AcknowledgeRuntimeTaskReadsInput,
    ) -> StoreResult<()> {
        NolaStore::acknowledge_runtime_task_reads(self, input)
    }
}
