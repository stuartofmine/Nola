// SPDX-License-Identifier: Apache-2.0
//! Narrow SQLite context repository sharing the Application-owned transaction.
use crate::{NolaStore, StoreError};
use nola_store_port::context::{
    CommitContextCheckpointInput, ContextAggregateRecord, ContextRepository,
    ContextRepositoryProvider,
};

#[path = "context_compaction.rs"]
mod compaction;

/// Borrowed context-checkpoint capability.
///
/// ```compile_fail
/// use nola_store::{ContextRepository, NolaStore};
/// fn requires_context(_: &mut impl ContextRepository) {}
/// let mut store = NolaStore::open(":memory:").unwrap();
/// requires_context(&mut store);
/// ```
///
/// ```compile_fail
/// use nola_store::NolaStore;
/// use nola_store_port::product_setting::ProductSettingRepository;
/// fn requires_settings(_: &mut impl ProductSettingRepository) {}
/// let mut store = NolaStore::open(":memory:").unwrap();
/// let mut context = store.context_repository();
/// requires_settings(&mut context);
/// ```
pub struct SqliteContextRepository<'store> {
    store: &'store mut NolaStore,
}

impl NolaStore {
    pub fn context_repository(&mut self) -> SqliteContextRepository<'_> {
        SqliteContextRepository { store: self }
    }
}

impl ContextRepositoryProvider for NolaStore {
    type Error = StoreError;
    type Repository<'a> = SqliteContextRepository<'a>;

    fn context_repository(&mut self) -> Self::Repository<'_> {
        NolaStore::context_repository(self)
    }
}

impl ContextRepository for SqliteContextRepository<'_> {
    type Error = StoreError;
    fn prepare_context_compaction(&self, conversation_id: &str) -> Result<nola_store_port::context::ContextCompactionSource, Self::Error> {
        self.store.prepare_context_compaction(conversation_id)
    }
    fn commit_context_compaction(&mut self, input: &nola_store_port::context::CommitContextCompactionInput) -> Result<(), Self::Error> {
        self.store.commit_context_compaction(input)
    }
    fn commit_context_checkpoint(
        &mut self,
        input: &CommitContextCheckpointInput,
    ) -> Result<(), Self::Error> {
        self.store.commit_context_checkpoint(input)
    }
    fn load_context_checkpoint(
        &self,
        checkpoint_id: &str,
    ) -> Result<Option<ContextAggregateRecord>, Self::Error> {
        self.store.load_context_checkpoint(checkpoint_id)
    }
    fn count_context_checkpoints(&self, conversation_id: &str) -> Result<u64, Self::Error> {
        self.store.count_context_checkpoints(conversation_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nola_store_port::{
        context::ContextCheckpointInput, conversation_profile::CreateConversationInput,
    };

    fn checkpoint(id: &str) -> CommitContextCheckpointInput {
        CommitContextCheckpointInput {
            checkpoint: ContextCheckpointInput {
                id: id.into(),
                conversation_id: "conversation-a".into(),
                run_id: None,
                model_profile_id: "model-a".into(),
                policy_version: "v1".into(),
                covered_through_message_id: None,
                covered_through_sequence: None,
                summary_part_id: None,
                retained_tail_start_message_id: None,
                estimated_tokens: 1,
                measured_tokens: None,
                context_window_tokens: 4096,
                prompt_limit_tokens: 3072,
                payload_digest: "digest-a".into(),
                created_at_ms: 10,
            },
            components: vec![],
        }
    }

    #[test]
    fn narrow_repository_checkpoint_joins_owner_unit_of_work() {
        let mut store = NolaStore::open(":memory:").expect("open Store");
        store
            .create_conversation(&CreateConversationInput {
                id: "conversation-a".into(),
                title: "Test".into(),
                revision: 1,
                settings_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
            })
            .expect("create conversation");

        store.begin_write_unit_of_work().expect("begin rollback");
        store
            .context_repository()
            .commit_context_checkpoint(&checkpoint("checkpoint-a"))
            .expect("commit checkpoint into owner transaction");
        store.rollback_write_unit_of_work().expect("rollback");
        assert_eq!(
            store
                .context_repository()
                .count_context_checkpoints("conversation-a")
                .expect("count after rollback"),
            0
        );

        store.begin_write_unit_of_work().expect("begin commit");
        store
            .context_repository()
            .commit_context_checkpoint(&checkpoint("checkpoint-a"))
            .expect("commit checkpoint into owner transaction");
        store.commit_write_unit_of_work().expect("commit");
        assert!(store
            .context_repository()
            .load_context_checkpoint("checkpoint-a")
            .expect("load after commit")
            .is_some());
    }
}
