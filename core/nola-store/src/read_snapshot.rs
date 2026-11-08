// SPDX-License-Identifier: Apache-2.0
//! Keep multi-query canonical reads on one SQLite revision. Never owns or
//! completes an enclosing Application write transaction.

use rusqlite::Transaction;

use super::{Connection, StoreResult};

pub(super) fn pin(connection: &Connection) -> StoreResult<Option<Transaction<'_>>> {
    if !connection.is_autocommit() {
        return Ok(None);
    }
    let transaction = connection.unchecked_transaction()?;
    // BEGIN DEFERRED alone does not acquire a read snapshot. Pin it now, before
    // a caller performs separate metadata and payload queries.
    transaction.query_row("SELECT COUNT(*) FROM sqlite_master", [], |row| {
        row.get::<_, i64>(0)
    })?;
    Ok(Some(transaction))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_commit_cannot_mix_metadata_and_payload_revisions() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("snapshot.sqlite");
        let writer = Connection::open(&path).unwrap();
        writer
            .execute_batch(
                "PRAGMA journal_mode=WAL;
            CREATE TABLE facts(revision INTEGER, body TEXT);
            INSERT INTO facts VALUES(1, 'first');",
            )
            .unwrap();
        let reader =
            Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let snapshot = pin(&reader).unwrap();
        let revision: i64 = reader
            .query_row("SELECT revision FROM facts", [], |row| row.get(0))
            .unwrap();
        writer
            .execute("UPDATE facts SET revision=2, body='second'", [])
            .unwrap();
        let body: String = reader
            .query_row("SELECT body FROM facts", [], |row| row.get(0))
            .unwrap();
        assert_eq!((revision, body.as_str()), (1, "first"));
        assert!(pin(&reader).unwrap().is_none());
        drop(snapshot);
        assert!(reader.is_autocommit());
        let current: String = reader
            .query_row("SELECT body FROM facts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(current, "second");
    }

    #[test]
    fn nested_read_does_not_commit_or_rollback_callers_write() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch("CREATE TABLE facts(value INTEGER)")
            .unwrap();
        let outer = connection.transaction().unwrap();
        outer.execute("INSERT INTO facts VALUES(1)", []).unwrap();
        assert!(pin(&outer).unwrap().is_none());
        assert!(!outer.is_autocommit());
        let count: i64 = outer
            .query_row("SELECT COUNT(*) FROM facts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
        outer.rollback().unwrap();
        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM facts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn failed_read_releases_its_snapshot() {
        let connection = Connection::open_in_memory().unwrap();
        let read = || -> StoreResult<()> {
            let _snapshot = pin(&connection)?;
            connection.prepare("SELECT * FROM missing_table")?;
            Ok(())
        };
        assert!(read().is_err());
        assert!(connection.is_autocommit());
    }
}
