// SPDX-License-Identifier: Apache-2.0
//! Narrow command-protocol adapters sharing the canonical transaction owner.
use crate::{NolaStore, StoreError};
use nola_store_port::protocol_journal::*;

/// Receipt admission and transaction control intentionally remain one object:
/// splitting them could admit a receipt on a different Unit of Work.
///
/// ```compile_fail
/// use nola_store::{CommandReceiptRepository, NolaStore};
/// fn requires_receipts(_: &mut impl CommandReceiptRepository) {}
/// let mut store = NolaStore::open(":memory:").unwrap();
/// requires_receipts(&mut store);
/// ```
///
/// ```compile_fail
/// use nola_store::NolaStore;
/// use nola_store_port::model::ModelRepository;
/// fn requires_models(_: &mut impl ModelRepository) {}
/// let mut store = NolaStore::open(":memory:").unwrap();
/// let mut protocol = store.command_protocol_repository();
/// requires_models(&mut protocol);
/// ```
pub struct SqliteCommandProtocolRepository<'store> {
    store: &'store mut NolaStore,
}

impl NolaStore {
    pub fn command_protocol_repository(&mut self) -> SqliteCommandProtocolRepository<'_> {
        SqliteCommandProtocolRepository { store: self }
    }
}

impl CommandProtocolRepositoryProvider for NolaStore {
    type Error = StoreError;
    type Repository<'a> = SqliteCommandProtocolRepository<'a>;

    fn command_protocol_repository(&mut self) -> Self::Repository<'_> {
        NolaStore::command_protocol_repository(self)
    }
}

impl CommandReceiptRepository for SqliteCommandProtocolRepository<'_> {
    type Error = StoreError;
    fn begin_command_receipt(
        &mut self,
        input: &BeginCommandReceiptInput,
    ) -> Result<BeginCommandReceiptResult, ReceiptAdmissionError<Self::Error>> {
        self.store
            .begin_command_receipt(input)
            .map_err(|error| match error {
                StoreError::Conflict(message) => ReceiptAdmissionError::IdentityConflict(message),
                error => ReceiptAdmissionError::Persistence(error),
            })
    }
    fn command_receipt(
        &self,
        client_id: &str,
        request_id: &str,
    ) -> Result<Option<StoredCommandReceipt>, Self::Error> {
        self.store.command_receipt(client_id, request_id)
    }
    fn complete_command_receipt(
        &mut self,
        client_id: &str,
        request_id: &str,
        completed_at: &str,
        outcome: &CompleteCommandReceiptOutcome,
    ) -> Result<StoredCommandReceipt, Self::Error> {
        self.store
            .complete_command_receipt(client_id, request_id, completed_at, outcome)
    }
}

impl ProjectionJournalRepository for NolaStore {
    type Error = StoreError;
    fn append_projection_event(
        &mut self,
        input: &AppendProjectionEventInput,
    ) -> Result<StoredProjectionEvent, Self::Error> {
        NolaStore::append_projection_event(self, input)
    }
    fn projection_cursor(
        &self,
        scope: &ProjectionScopeKey,
    ) -> Result<Option<StoredProjectionCursor>, Self::Error> {
        NolaStore::projection_cursor(self, scope)
    }
    fn projection_events_after(
        &self,
        scope: &ProjectionScopeKey,
        after_sequence: u64,
        limit: usize,
    ) -> Result<Vec<StoredProjectionEvent>, Self::Error> {
        NolaStore::projection_events_after(self, scope, after_sequence, limit)
    }
}

impl CommandTransactionRepository for SqliteCommandProtocolRepository<'_> {
    type Error = StoreError;
    fn has_write_unit_of_work(&self) -> bool {
        self.store.has_write_unit_of_work()
    }
    fn begin_write_unit_of_work(&mut self) -> Result<(), Self::Error> {
        self.store.begin_write_unit_of_work()
    }
    fn commit_write_unit_of_work(&mut self) -> Result<(), Self::Error> {
        self.store.commit_write_unit_of_work()
    }
    fn rollback_write_unit_of_work(&mut self) -> Result<(), Self::Error> {
        self.store.rollback_write_unit_of_work()
    }
    fn begin_command_mutations(&mut self) -> Result<(), Self::Error> {
        self.store.begin_command_mutations()
    }
    fn commit_command_mutations(&mut self) -> Result<(), Self::Error> {
        self.store.commit_command_mutations()
    }
    fn rollback_command_mutations(&mut self) -> Result<(), Self::Error> {
        self.store.rollback_command_mutations()
    }
    fn command_revision(
        &self,
        entity: CommandRevisionEntity<'_>,
    ) -> Result<Option<i64>, Self::Error> {
        match entity {
            CommandRevisionEntity::ModelProfile(id) => self.store.model_profile_revision(id),
            CommandRevisionEntity::Conversation(id) => self.store.conversation_revision(id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nola_store_port::product_setting::ProductSettingRepository;

    fn admission(request_id: &str) -> BeginCommandReceiptInput {
        BeginCommandReceiptInput {
            client_id: "client-a".into(),
            request_id: request_id.into(),
            command_type: "put_product_setting".into(),
            command_json: r#"{"key":"setting-a","valueJson":"true","updatedAtMs":10}"#.into(),
            expected_revision: None,
            received_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    fn journal(request_id: &str) -> AppendProjectionEventInput {
        AppendProjectionEventInput {
            event_id: format!("event-{request_id}"),
            scope: ProjectionScopeKey {
                scope_type: "test".into(),
                scope_id: "scope-a".into(),
            },
            expected_after_sequence: 0,
            payload_json: "{}".into(),
            occurred_at: "2026-01-01T00:00:01Z".into(),
        }
    }

    fn execute_atomic_fixture(store: &mut NolaStore, request_id: &str) {
        {
            let mut protocol = store.command_protocol_repository();
            assert!(matches!(
                protocol
                    .begin_command_receipt(&admission(request_id))
                    .expect("admit receipt"),
                BeginCommandReceiptResult::Started(_)
            ));
        }
        store
            .product_setting_repository()
            .put_product_setting("setting-a", "true", 10)
            .expect("write business mutation");
        store
            .append_projection_event(&journal(request_id))
            .expect("append journal in owner transaction");
        store
            .command_protocol_repository()
            .complete_command_receipt(
                "client-a",
                request_id,
                "2026-01-01T00:00:02Z",
                &CompleteCommandReceiptOutcome::Committed {
                    committed_revision: Some(1),
                    result_json: "{}".into(),
                },
            )
            .expect("complete receipt");
    }

    #[test]
    fn narrow_protocol_keeps_receipt_mutation_and_journal_in_one_unit_of_work() {
        let mut store = NolaStore::open(":memory:").expect("open Store");
        store
            .command_protocol_repository()
            .begin_write_unit_of_work()
            .expect("begin rollback fixture");
        execute_atomic_fixture(&mut store, "rolled-back");
        store
            .command_protocol_repository()
            .rollback_write_unit_of_work()
            .expect("rollback fixture");
        assert!(store
            .command_protocol_repository()
            .command_receipt("client-a", "rolled-back")
            .expect("read rolled back receipt")
            .is_none());
        assert!(store
            .product_setting_repository()
            .product_setting("setting-a")
            .expect("read rolled back mutation")
            .is_none());
        assert!(store
            .projection_events_after(&journal("rolled-back").scope, 0, 10)
            .expect("read rolled back journal")
            .is_empty());

        store
            .command_protocol_repository()
            .begin_write_unit_of_work()
            .expect("begin commit fixture");
        execute_atomic_fixture(&mut store, "committed");
        store
            .command_protocol_repository()
            .commit_write_unit_of_work()
            .expect("commit fixture");
        assert!(store
            .command_protocol_repository()
            .command_receipt("client-a", "committed")
            .expect("read committed receipt")
            .is_some());
        assert!(store
            .product_setting_repository()
            .product_setting("setting-a")
            .expect("read committed mutation")
            .is_some());
        assert_eq!(
            store
                .projection_events_after(&journal("committed").scope, 0, 10)
                .expect("read committed journal")
                .len(),
            1
        );
    }
}
