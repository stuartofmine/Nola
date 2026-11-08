// SPDX-License-Identifier: Apache-2.0
//! Narrow replica adapter sharing the canonical command transaction.
use crate::{NolaStore, StoreError};
use nola_store_port::replica_sync::*;

/// Borrowed synchronization capability; it cannot be widened into operations peers.
///
/// ```compile_fail
/// use nola_store::{NolaStore, ReplicaSyncRepository};
/// fn requires_sync(_: &mut impl ReplicaSyncRepository) {}
/// requires_sync(&mut NolaStore::open(":memory:").unwrap());
/// ```
///
/// ```compile_fail
/// use nola_store::NolaStore;
/// use nola_store_port::automation::AutomationRepository;
/// fn requires_automation(_: &mut impl AutomationRepository) {}
/// requires_automation(&mut NolaStore::open(":memory:").unwrap().replica_sync_repository());
/// ```
pub struct SqliteReplicaSyncRepository<'store> {
    store: &'store mut NolaStore,
}

impl NolaStore {
    pub fn replica_sync_repository(&mut self) -> SqliteReplicaSyncRepository<'_> {
        SqliteReplicaSyncRepository { store: self }
    }
}

impl ReplicaSyncRepositoryProvider for NolaStore {
    type Error = StoreError;
    type Repository<'a> = SqliteReplicaSyncRepository<'a>;

    fn replica_sync_repository(&mut self) -> Self::Repository<'_> {
        NolaStore::replica_sync_repository(self)
    }
}

impl ReplicaSyncRepository for SqliteReplicaSyncRepository<'_> {
    type Error = StoreError;

    fn pending_sync_changes(&self, limit: usize) -> Result<Vec<StoredSyncChange>, Self::Error> {
        self.store.pending_sync_changes(limit)
    }

    fn resolve_sync_change(
        &mut self,
        operation_id: &str,
        status: &str,
        remote_cursor: Option<i64>,
        resolved_at_ms: i64,
    ) -> Result<bool, Self::Error> {
        self.store
            .resolve_sync_change(operation_id, status, remote_cursor, resolved_at_ms)
    }

    fn sync_cursor(&self, replica_id: &str) -> Result<Option<StoredSyncCursor>, Self::Error> {
        self.store.sync_cursor(replica_id)
    }

    fn save_sync_cursor(
        &mut self,
        replica_id: &str,
        cursor: i64,
        updated_at_ms: i64,
    ) -> Result<StoredSyncCursor, Self::Error> {
        self.store
            .save_sync_cursor(replica_id, cursor, updated_at_ms)
    }

    fn apply_remote_sync_page(
        &mut self,
        input: &ApplyRemoteSyncPageInput,
    ) -> Result<ApplyRemoteSyncPageResult, Self::Error> {
        self.store.apply_remote_sync_page(input)
    }
}
