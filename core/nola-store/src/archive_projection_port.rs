// SPDX-License-Identifier: Apache-2.0
use crate::{NolaStore, StoreError};
use nola_store_port::archive_restore::ArchiveProjectionRepository;
impl ArchiveProjectionRepository for NolaStore {
    type Error = StoreError;
    fn archive_restore_projection_ids<'a>(
        &self,
        changes: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<Vec<String>, Self::Error> {
        NolaStore::archive_restore_projection_ids(self, changes)
    }
    fn archive_restore_conversation_ids(
        &self,
        restore_id: &str,
    ) -> Result<Vec<String>, Self::Error> {
        NolaStore::archive_restore_conversation_ids(self, restore_id)
    }
}
