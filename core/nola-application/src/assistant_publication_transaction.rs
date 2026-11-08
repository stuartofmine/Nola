// SPDX-License-Identifier: Apache-2.0
//! Direct Application callers and Runtime commands share canonical/public commits.
use nola_store_port::protocol_journal::{CommandProtocolRepositoryProvider, CommandTransactionRepository};

pub(crate) fn with_publication_transaction<R: CommandProtocolRepositoryProvider + ?Sized, T>(
    store: &mut R,
    apply: impl FnOnce(&mut R) -> Result<T, String>,
) -> Result<T, String> {
    if store.command_protocol_repository().has_write_unit_of_work() {
        return apply(store);
    }
    store.command_protocol_repository().begin_write_unit_of_work()
        .map_err(|error| format!("begin canonical/publication transaction failed: {error}"))?;
    match apply(store) {
        Ok(result) => {
            let commit = store.command_protocol_repository().commit_write_unit_of_work();
            if let Err(error) = commit {
                // COMMIT failure can leave SQLite inside the transaction. Do
                // not let the next direct call join those unpublished writes.
                // Cleanup cannot prove whether a commit acknowledgement was
                // lost: callers must still reopen/reconcile, never publish.
                let cleanup = store.command_protocol_repository().rollback_write_unit_of_work();
                return Err(match cleanup {
                    Ok(()) => format!("canonical/publication commit outcome requires reconciliation; reopen required: {error}"),
                    Err(rollback) => format!("canonical/publication commit outcome requires reconciliation; reopen required: {error}; rollback failed: {rollback}"),
                });
            }
            Ok(result)
        }
        Err(error) => {
            store.command_protocol_repository().rollback_write_unit_of_work()
                .map_err(|rollback| format!("{error}; canonical/publication rollback failed; reopen required: {rollback}"))?;
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nola_store::NolaStore;

    #[test]
    fn failed_outer_commit_does_not_leak_a_joinable_transaction() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("publication.sqlite");
        let mut store = NolaStore::open(&path).unwrap();
        let fixture = rusqlite::Connection::open(&path).unwrap();
        // The mutation/savepoint succeeds; only the outer COMMIT fails.
        fixture.execute_batch(
            "CREATE TABLE fixture_parent(id INTEGER PRIMARY KEY);
             CREATE TABLE fixture_child(parent_id INTEGER REFERENCES fixture_parent(id)
                 DEFERRABLE INITIALLY DEFERRED);
             CREATE TRIGGER fixture_commit_failure AFTER INSERT ON product_settings
             BEGIN INSERT INTO fixture_child VALUES(1); END;"
        ).unwrap();
        let error = with_publication_transaction(&mut store, |store| {
            store.put_product_setting("fixture", "true", 1)
                .map_err(|error| error.to_string())
        }).unwrap_err();
        assert!(error.contains("requires reconciliation"), "{error}");
        assert!(!store.has_write_unit_of_work(), "failed commit left a joinable transaction");
        assert!(store.product_setting("fixture").unwrap().is_none());
        assert!(store.pending_sync_changes(10).unwrap().is_empty());
        let retry = with_publication_transaction(&mut store, |_| Ok(()));
        assert!(retry.unwrap_err().contains("reopen required"));
        assert!(store.put_product_setting("retry", "true", 2).unwrap_err()
            .to_string().contains("reopen required"));
        drop(store);
        fixture.execute_batch("DROP TRIGGER fixture_commit_failure").unwrap();
        let mut reopened = NolaStore::open(&path).unwrap();
        assert!(reopened.product_setting("fixture").unwrap().is_none());
        assert!(reopened.pending_sync_changes(10).unwrap().is_empty());
        with_publication_transaction(&mut reopened, |store| {
            store.put_product_setting("retry", "true", 2)
                .map_err(|error| error.to_string())
        }).unwrap();
    }

    #[test]
    fn joined_publication_never_settles_its_callers_transaction() {
        let mut store = NolaStore::open(":memory:").unwrap();
        store.begin_write_unit_of_work().unwrap();
        with_publication_transaction(&mut store, |store| {
            store.put_product_setting("fixture", "true", 1)
                .map_err(|error| error.to_string())
        }).unwrap();
        assert!(store.has_write_unit_of_work());
        let error = with_publication_transaction(&mut store, |_| Err::<(), _>("rejected".into()));
        assert_eq!(error.unwrap_err(), "rejected");
        assert!(store.has_write_unit_of_work());
        assert!(store.product_setting("fixture").unwrap().is_some());
        store.rollback_write_unit_of_work().unwrap();
        assert!(store.product_setting("fixture").unwrap().is_none());
    }
}
