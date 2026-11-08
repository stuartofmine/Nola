// SPDX-License-Identifier: Apache-2.0
//! Verifies that incompatible pre-release stores fail closed instead of being migrated.

use nola_store::{NolaStore, STORE_SCHEMA_EPOCH};
use rusqlite::Connection;
use tempfile::tempdir;

#[test]
fn fresh_store_records_current_schema_epoch() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("current.db");
    drop(NolaStore::open(&database).unwrap());

    let connection = Connection::open(&database).unwrap();
    let stored_epoch: i64 = connection
        .query_row(
            "SELECT CAST(value AS INTEGER) FROM store_metadata WHERE key='schema_epoch'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored_epoch, STORE_SCHEMA_EPOCH);
    assert_eq!(stored_epoch, 1);
}

#[test]
fn previous_epoch_store_is_rejected_without_rewriting_data() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("incompatible.db");
    let previous_epoch = STORE_SCHEMA_EPOCH - 1;
    drop(NolaStore::open(&database).unwrap());

    let connection = Connection::open(&database).unwrap();
    connection
        .execute(
            "UPDATE store_metadata SET value=?1 WHERE key='schema_epoch'",
            [previous_epoch.to_string()],
        )
        .unwrap();
    drop(connection);

    let error = match NolaStore::open(&database) {
        Ok(_) => panic!("previous epoch store must be rejected"),
        Err(error) => error,
    };
    assert!(error.is_schema_incompatible());

    let connection = Connection::open(&database).unwrap();
    let stored_epoch: String = connection
        .query_row(
            "SELECT value FROM store_metadata WHERE key='schema_epoch'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        stored_epoch, previous_epoch.to_string(),
        "failed open must not rewrite the old store"
    );
}

#[test]
fn reissued_baseline_is_one_and_development_format_is_preserved_on_rejection() {
    assert_eq!(nola_store::STORE_SCHEMA_VERSION, 1);
    assert_eq!(nola_store::STORE_SCHEMA_EPOCH, 1);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("unsupported-development.sqlite");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.execute_batch("PRAGMA application_id=1313823809; PRAGMA user_version=83; CREATE TABLE valuable(content TEXT); INSERT INTO valuable VALUES('keep this data');").unwrap();
    drop(connection);
    let original = std::fs::read(&path).unwrap();
    assert!(nola_store::NolaStore::open(&path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), original);
}
