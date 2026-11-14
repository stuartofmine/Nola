// SPDX-License-Identifier: Apache-2.0
//! Narrow canonical-conversation writer sharing the Application-owned transaction.

use crate::{params, NolaStore, StoreError, StoreResult};
use nola_store_port::{
    canonical_branch::{CanonicalBranchIntent, CanonicalBranchRepository, CanonicalBranchResult},
    conversation_deletion::ConversationDeletionRepository,
    conversation_profile::{
        ConversationProfileRepository, ConversationWriteRepositoryProvider,
        CreateConversationInput, CreateConversationResult,
    },
    transcript_write::{
        AppendMessagesInput, AppendMessagesResult, TranscriptEditRepository,
        TranscriptWriteRepository,
    },
};

/// Borrowed conversation mutation capability.
///
/// ```compile_fail
/// use nola_store::{ConversationProfileRepository, NolaStore};
/// fn requires_writer(_: &mut impl ConversationProfileRepository) {}
/// let mut store = NolaStore::open(":memory:").unwrap();
/// requires_writer(&mut store);
/// ```
///
/// ```compile_fail
/// use nola_store::NolaStore;
/// use nola_store_port::model::ModelRepository;
/// fn requires_models(_: &mut impl ModelRepository) {}
/// let mut store = NolaStore::open(":memory:").unwrap();
/// let mut conversations = store.conversation_write_repository();
/// requires_models(&mut conversations);
/// ```
pub struct SqliteConversationWriteRepository<'store> {
    store: &'store mut NolaStore,
}

impl NolaStore {
    pub fn conversation_write_repository(&mut self) -> SqliteConversationWriteRepository<'_> {
        SqliteConversationWriteRepository { store: self }
    }
}

impl ConversationWriteRepositoryProvider for NolaStore {
    type Error = StoreError;
    type Repository<'a> = SqliteConversationWriteRepository<'a>;

    fn conversation_write_repository(&mut self) -> Self::Repository<'_> {
        NolaStore::conversation_write_repository(self)
    }
}

impl ConversationProfileRepository for SqliteConversationWriteRepository<'_> {
    type Error = StoreError;
    fn patch_conversation_metadata(
        &mut self,
        input: &nola_store_port::conversation_profile::PatchConversationMetadataInput,
    ) -> StoreResult<Option<i64>> {
        self.store.patch_conversation_metadata(input)
    }
    fn patch_conversation_configuration(
        &mut self,
        input: &nola_store_port::conversation_profile::PatchConversationConfigurationInput,
    ) -> Result<nola_domain::conversation_configuration::ConfigurationCommitResult, Self::Error> {
        self.store.patch_conversation_configuration(input)
    }

    fn create_conversation(
        &mut self,
        input: &CreateConversationInput,
    ) -> StoreResult<CreateConversationResult> {
        self.store.create_conversation(input)
    }

    fn rename_conversation(
        &mut self,
        id: &str,
        title: &str,
        title_source: Option<nola_domain::conversation_configuration::ConversationTitleSource>,
        updated_at_ms: i64,
    ) -> StoreResult<bool> {
        self.store.rename_conversation(id, title, title_source, updated_at_ms)
    }

}

impl CanonicalBranchRepository for SqliteConversationWriteRepository<'_> {
    type Error = StoreError;

    fn create_conversation_branch(
        &mut self,
        intent: &CanonicalBranchIntent,
    ) -> StoreResult<CanonicalBranchResult> {
        self.store.create_conversation_branch(intent)
    }
}

impl ConversationDeletionRepository for SqliteConversationWriteRepository<'_> {
    type Error = StoreError;

    fn delete_conversation(&mut self, conversation_id: &str) -> StoreResult<bool> {
        self.store.delete_conversation(conversation_id)
    }
}

impl TranscriptWriteRepository for SqliteConversationWriteRepository<'_> {
    type Error = StoreError;
    fn message_frontier(&self, conversation_id: &str) -> StoreResult<(usize, i64)> {
        self.store.connection.query_row(
            "SELECT COUNT(*),COALESCE(MAX(ordinal),0) FROM messages WHERE conversation_id=?1",
            params![conversation_id], |row| Ok((row.get(0)?,row.get(1)?)),
        ).map_err(StoreError::from)
    }

    fn has_active_run(&self, conversation_id: &str) -> StoreResult<bool> {
        self.store
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM runs WHERE conversation_id=?1
                 AND status NOT IN ('sealed'))",
                params![conversation_id],
                |row| row.get(0),
            )
            .map_err(StoreError::from)
    }

    fn conversation_id_for_run(&self, run_id: &str) -> StoreResult<Option<String>> {
        self.store.conversation_id_for_run(run_id)
    }

    fn append_messages(
        &mut self,
        input: &AppendMessagesInput,
    ) -> StoreResult<AppendMessagesResult> {
        self.store.append_messages(input)
    }
}

impl TranscriptEditRepository for SqliteConversationWriteRepository<'_> {
    fn apply_conversation_projection(
        &mut self,
        input: &nola_store_port::conversation_revert::ApplyConversationProjectionInput,
    ) -> StoreResult<nola_store_port::conversation_revert::ApplyConversationProjectionResult> {
        self.store.apply_conversation_projection(input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conversation(id: &str) -> CreateConversationInput {
        CreateConversationInput {
            id: id.into(),
            title: "Original".into(),
            revision: 1,
            settings_json: Some("{}".into()),
            created_at_ms: 10,
            updated_at_ms: 10,
        }
    }

    #[test]
    fn narrow_writer_preserves_owner_commit_and_rollback() {
        let mut store = NolaStore::open(":memory:").expect("open Store");
        store.begin_write_unit_of_work().expect("begin rollback");
        store
            .conversation_write_repository()
            .create_conversation(&conversation("rolled-back"))
            .expect("create in transaction");
        store.rollback_write_unit_of_work().expect("rollback");
        assert!(store
            .load_conversation("rolled-back")
            .expect("read rollback result")
            .is_none());

        store.begin_write_unit_of_work().expect("begin commit");
        store
            .conversation_write_repository()
            .create_conversation(&conversation("committed"))
            .expect("create in transaction");
        store.commit_write_unit_of_work().expect("commit");
        assert!(store
            .load_conversation("committed")
            .expect("read commit result")
            .is_some());
    }
}
