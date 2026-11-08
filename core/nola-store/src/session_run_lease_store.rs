// SPDX-License-Identifier: Apache-2.0
//! Cross-process session run lease acquisition, inspection, and release.

use super::*;

impl NolaStore {
    pub fn acquire_session_run_lease(
        &mut self,
        lease: &StoredSessionRunLease,
    ) -> StoreResult<bool> {
        require_id("session_run_lease.session_id", &lease.session_id)?;
        require_id("session_run_lease.owner", &lease.owner)?;
        Ok(self.connection.execute(
            "INSERT OR IGNORE INTO session_run_leases(
                session_id,owner,pid,process_start,acquired_at_ms
             ) VALUES(?1,?2,?3,?4,?5)",
            params![
                lease.session_id,
                lease.owner,
                i64::from(lease.pid),
                lease.process_start,
                lease.acquired_at_ms
            ],
        )? == 1)
    }

    pub fn session_run_lease(
        &self,
        session_id: &str,
    ) -> StoreResult<Option<StoredSessionRunLease>> {
        require_id("session_run_lease.session_id", session_id)?;
        self.connection
            .query_row(
                &format!(
                    "SELECT {SESSION_RUN_LEASE_COLUMNS} FROM session_run_leases
                     WHERE session_id=?1"
                ),
                params![session_id],
                session_run_lease_row,
            )
            .optional()
            .map_err(StoreError::from)
    }

    pub fn session_run_leases(&self) -> StoreResult<Vec<StoredSessionRunLease>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {SESSION_RUN_LEASE_COLUMNS} FROM session_run_leases ORDER BY session_id"
        ))?;
        let rows = statement.query_map([], session_run_lease_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    pub fn release_session_run_lease(
        &mut self,
        session_id: &str,
        owner: &str,
    ) -> StoreResult<bool> {
        require_id("session_run_lease.session_id", session_id)?;
        require_id("session_run_lease.owner", owner)?;
        Ok(self.connection.execute(
            "DELETE FROM session_run_leases WHERE session_id=?1 AND owner=?2",
            params![session_id, owner],
        )? == 1)
    }
}
