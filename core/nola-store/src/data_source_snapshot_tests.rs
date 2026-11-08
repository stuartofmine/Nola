// SPDX-License-Identifier: Apache-2.0
//! Real SQLite read-snapshot and caller-transaction boundary tests.
use crate::{CreateConversationInput, NolaStore, StoredDataSourceRecord};
use nola_store_port::data_source::DataSourceReadRepository;
use std::{sync::mpsc, time::Duration};

fn fixture() -> (tempfile::TempDir, NolaStore) {
    let directory = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(directory.path().join("nola.db")).unwrap();
    store.create_conversation(&CreateConversationInput {
        id: "conversation".into(), title: "Source snapshot".into(), revision: 1,
        settings_json: None, created_at_ms: 1, updated_at_ms: 1,
    }).unwrap();
    store
        .upsert_data_source(&StoredDataSourceRecord {
            id: "source".into(),
            name: "Source".into(),
            kind: "custom".into(),
            location: "fixture://source".into(),
            enabled: false,
            access_policy: "conversationOptIn".into(),
            validation_contract_json: "{}".into(),
            capabilities_json: "[\"read\"]".into(),
            privacy: "private".into(),
            metadata_json: "{}".into(),
            revision: 0,
            created_at_ms: 1,
            updated_at_ms: 1,
            deleted_at_ms: None,
        })
        .unwrap();
    store
        .set_data_source_override("conversation", "source", Some(true), 1)
        .unwrap();
    (directory, store)
}

#[test]
fn batch_read_does_not_commit_or_replace_the_callers_transaction() {
    let (_directory, mut store) = fixture();
    store
        .connection
        .execute_batch(
            "BEGIN IMMEDIATE;
         UPDATE data_sources SET enabled=1 WHERE id='source';
         UPDATE data_source_overrides SET enabled=0 WHERE conversation_id='conversation';",
        )
        .unwrap();
    let selected = store
        .data_source_repository()
        .list_data_sources_for_conversation("conversation")
        .unwrap();
    assert_eq!(selected.len(), 1);
    assert!(selected[0].0.enabled);
    assert_eq!(selected[0].1, Some(false));
    assert!(!store.connection.is_autocommit());
    store.connection.execute_batch("ROLLBACK").unwrap();
    let original = store
        .data_source_repository()
        .list_data_sources_for_conversation("conversation")
        .unwrap();
    assert!(!original[0].0.enabled);
    assert_eq!(original[0].1, Some(true));
    assert!(store.connection.is_autocommit());
}

#[test]
fn concurrent_commit_between_batch_queries_cannot_mix_source_and_override_versions() {
    let (directory, mut store) = fixture();
    let (reached, wait_until_read) = mpsc::channel();
    let (committed, wait_until_commit) = mpsc::channel();
    let mut first_read = true;
    store
        .connection
        .create_scalar_function(
            "nola_snapshot_barrier",
            1,
            rusqlite::functions::FunctionFlags::SQLITE_UTF8,
            move |context| {
                let id: String = context.get(0)?;
                if first_read {
                    first_read = false;
                    reached
                        .send(())
                        .map_err(|error| rusqlite::Error::UserFunctionError(Box::new(error)))?;
                    wait_until_commit
                        .recv_timeout(Duration::from_secs(5))
                        .map_err(|error| rusqlite::Error::UserFunctionError(Box::new(error)))?;
                }
                Ok(id)
            },
        )
        .unwrap();
    // A fixture-only view makes the interleaving deterministic. Production
    // repositories and Application have no synchronization/testing hooks.
    store
        .connection
        .execute_batch(
            "ALTER TABLE data_sources RENAME TO source_snapshot_fixture;
         CREATE VIEW data_sources AS SELECT nola_snapshot_barrier(id) AS id,
             name,kind,location,enabled,access_policy,validation_contract_json,
             capabilities_json,privacy,metadata_json,revision,created_at_ms,
             updated_at_ms,deleted_at_ms FROM source_snapshot_fixture;",
        )
        .unwrap();
    let path = directory.path().join("nola.db");
    let writer = std::thread::spawn(move || {
        wait_until_read
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        let connection = rusqlite::Connection::open(path).unwrap();
        connection.busy_timeout(Duration::from_secs(2)).unwrap();
        connection
            .execute_batch(
                "BEGIN IMMEDIATE;
             UPDATE source_snapshot_fixture SET enabled=1 WHERE id='source';
             UPDATE data_source_overrides SET enabled=0 WHERE conversation_id='conversation';
             COMMIT;",
            )
            .unwrap();
        committed.send(()).unwrap();
    });
    let result = store
        .data_source_repository()
        .list_data_sources_for_conversation("conversation");
    writer.join().unwrap();
    let selected = result.unwrap();
    assert_eq!(selected.len(), 1);
    assert!(!selected[0].0.enabled);
    assert_eq!(
        selected[0].1,
        Some(true),
        "override must belong to the source read snapshot"
    );
    // The read transaction ends at return; a subsequent read sees the new pair.
    assert!(store.connection.is_autocommit());
    let current = store
        .data_source_repository()
        .list_data_sources_for_conversation("conversation")
        .unwrap();
    assert!(current[0].0.enabled);
    assert_eq!(current[0].1, Some(false));
}
