// SPDX-License-Identifier: Apache-2.0
use crate::{NolaStore, StoreError};
use nola_store_port::sop_run::*;
impl SopRunRepository for NolaStore {
    type Error = StoreError;
    fn create_sop_run(&mut self, run: &StoredSopRun) -> Result<(), Self::Error> {
        NolaStore::create_sop_run(self, run)
    }
    fn update_sop_run_cas(
        &mut self,
        run: &StoredSopRun,
        expected_revision: i64,
    ) -> Result<bool, Self::Error> {
        NolaStore::update_sop_run_cas(self, run, expected_revision)
    }
    fn sop_run(&self, run_id: &str) -> Result<Option<StoredSopRun>, Self::Error> {
        NolaStore::sop_run(self, run_id)
    }
    fn list_sop_runs(
        &self,
        project_root: Option<&str>,
        owner_session_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<StoredSopRun>, Self::Error> {
        NolaStore::list_sop_runs(self, project_root, owner_session_id, limit)
    }
    fn visible_active_sop_runs(
        &self,
        query: &VisibleActiveSopRuns,
    ) -> Result<Vec<StoredSopRun>, Self::Error> {
        NolaStore::visible_active_sop_runs(self, query)
    }
}
