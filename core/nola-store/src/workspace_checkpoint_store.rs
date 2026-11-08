// SPDX-License-Identifier: Apache-2.0
//! Durable workspace checkpoint creation, listing, and deletion.

use super::*;

impl NolaStore {
    pub fn save_workspace_checkpoint(
        &mut self,
        checkpoint: &StoredWorkspaceCheckpoint,
    ) -> StoreResult<bool> {
        require_id("workspace_checkpoint.id", &checkpoint.id)?;
        require_id("workspace_checkpoint.root", &checkpoint.root)?;
        require_id("workspace_checkpoint.git_root", &checkpoint.git_root)?;
        require_id("workspace_checkpoint.revision", &checkpoint.revision)?;
        let untracked_json = serde_json::to_string(&checkpoint.untracked)?;
        Ok(self.connection.execute(
            "INSERT OR IGNORE INTO workspace_checkpoints(
                id,root,git_root,revision,source,label,created_at_ms,status,untracked_json
             ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                checkpoint.id,
                checkpoint.root,
                checkpoint.git_root,
                checkpoint.revision,
                checkpoint.source,
                checkpoint.label,
                checkpoint.created_at_ms,
                checkpoint.status,
                untracked_json
            ],
        )? == 1)
    }

    pub fn workspace_checkpoints(&self, root: &str) -> StoreResult<Vec<StoredWorkspaceCheckpoint>> {
        require_id("workspace_checkpoint.root", root)?;
        let mut statement = self.connection.prepare(&format!(
            "SELECT {WORKSPACE_CHECKPOINT_COLUMNS} FROM workspace_checkpoints
             WHERE root=?1 ORDER BY created_at_ms,id"
        ))?;
        let rows = statement.query_map(params![root], workspace_checkpoint_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    pub fn delete_workspace_checkpoint(&mut self, id: &str) -> StoreResult<bool> {
        require_id("workspace_checkpoint.id", id)?;
        Ok(self
            .connection
            .execute("DELETE FROM workspace_checkpoints WHERE id=?1", params![id])?
            == 1)
    }
}
