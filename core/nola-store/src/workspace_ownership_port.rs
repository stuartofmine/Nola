// SPDX-License-Identifier: Apache-2.0
//! Narrow workspace ownership adapter sharing the command transaction.
use crate::{NolaStore, StoreError};
use nola_store_port::workspace_ownership::*;

/// Borrowed workspace ownership capability; recovery maintenance remains separate.
///
/// ```compile_fail
/// use nola_store::{NolaStore, WorkspaceOwnershipRepository};
/// fn requires_workspace(_: &mut impl WorkspaceOwnershipRepository) {}
/// requires_workspace(&mut NolaStore::open(":memory:").unwrap());
/// ```
///
/// ```compile_fail
/// use nola_store::NolaStore;
/// use nola_store_port::store_maintenance::StoreMaintenanceRepository;
/// fn requires_recovery(_: &mut impl StoreMaintenanceRepository) {}
/// requires_recovery(&mut NolaStore::open(":memory:").unwrap().workspace_ownership_repository());
/// ```
pub struct SqliteWorkspaceOwnershipRepository<'store> {
    store: &'store mut NolaStore,
}

impl NolaStore {
    pub fn workspace_ownership_repository(&mut self) -> SqliteWorkspaceOwnershipRepository<'_> {
        SqliteWorkspaceOwnershipRepository { store: self }
    }
}

impl WorkspaceOwnershipRepositoryProvider for NolaStore {
    type Error = StoreError;
    type Repository<'a> = SqliteWorkspaceOwnershipRepository<'a>;

    fn workspace_ownership_repository(&mut self) -> Self::Repository<'_> {
        NolaStore::workspace_ownership_repository(self)
    }
}

impl WorkspaceOwnershipRepository for SqliteWorkspaceOwnershipRepository<'_> {
    type Error = StoreError;
    fn acquire_session_run_lease(
        &mut self,
        lease: &StoredSessionRunLease,
    ) -> Result<bool, Self::Error> {
        self.store.acquire_session_run_lease(lease)
    }
    fn save_workspace_checkpoint(
        &mut self,
        checkpoint: &StoredWorkspaceCheckpoint,
    ) -> Result<bool, Self::Error> {
        self.store.save_workspace_checkpoint(checkpoint)
    }
    fn session_run_lease(
        &self,
        session_id: &str,
    ) -> Result<Option<StoredSessionRunLease>, Self::Error> {
        self.store.session_run_lease(session_id)
    }
    fn session_run_leases(&self) -> Result<Vec<StoredSessionRunLease>, Self::Error> {
        self.store.session_run_leases()
    }
    fn workspace_checkpoints(
        &self,
        root: &str,
    ) -> Result<Vec<StoredWorkspaceCheckpoint>, Self::Error> {
        self.store.workspace_checkpoints(root)
    }
    fn delete_workspace_checkpoint(&mut self, id: &str) -> Result<bool, Self::Error> {
        self.store.delete_workspace_checkpoint(id)
    }
    fn release_session_run_lease(
        &mut self,
        session_id: &str,
        owner: &str,
    ) -> Result<bool, Self::Error> {
        self.store.release_session_run_lease(session_id, owner)
    }
}
