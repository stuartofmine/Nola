// SPDX-License-Identifier: Apache-2.0

use rusqlite::Connection;

pub(crate) const SCHEMA_VERSION: i64 = 1;
pub(crate) const EXTRACTOR_VERSION: &str = "nola-native-arborium-v5";

pub(crate) fn initialize(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA temp_store = MEMORY;
         PRAGMA busy_timeout = 5000;
         CREATE TABLE IF NOT EXISTS metadata (
             key TEXT PRIMARY KEY,
             value TEXT NOT NULL
         );",
    )?;
    let existing_schema = metadata_value(connection, "schema_version")?;
    let existing_extractor = metadata_value(connection, "extractor_version")?;
    let has_index_tables: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='projects')",
        [],
        |row| row.get(0),
    )?;
    let expected_schema = SCHEMA_VERSION.to_string();
    if has_index_tables
        && (existing_schema.as_deref() != Some(expected_schema.as_str())
            || existing_extractor.as_deref() != Some(EXTRACTOR_VERSION))
    {
        // Single-version contract: an incompatible store is rejected, never
        // silently destroyed or converted, even when it contains derived data.
        return Err(rusqlite::Error::InvalidParameterName(
            "incompatible code-memory schema or extractor version; existing data was not rebuilt".into(),
        ));
    }
    connection.execute_batch(
        "
         CREATE TABLE IF NOT EXISTS projects (
             id INTEGER PRIMARY KEY,
             name TEXT NOT NULL UNIQUE,
             root_path TEXT NOT NULL UNIQUE,
             generation INTEGER NOT NULL DEFAULT 0,
             indexed_at_unix_ms INTEGER NOT NULL DEFAULT 0
         );
         CREATE TABLE IF NOT EXISTS files (
             id INTEGER PRIMARY KEY,
             project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
             rel_path TEXT NOT NULL,
             language TEXT NOT NULL,
             content_sha256 TEXT NOT NULL,
             mtime_ns INTEGER NOT NULL,
             byte_count INTEGER NOT NULL,
             indexed_generation INTEGER NOT NULL,
             coverage_kind TEXT,
             coverage_detail TEXT,
             UNIQUE(project_id, rel_path)
         );
         CREATE TABLE IF NOT EXISTS symbols (
             id INTEGER PRIMARY KEY,
             project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
             file_id INTEGER REFERENCES files(id) ON DELETE CASCADE,
             kind TEXT NOT NULL,
             name TEXT NOT NULL,
             qualified_name TEXT NOT NULL,
             start_byte INTEGER NOT NULL,
             end_byte INTEGER NOT NULL,
             start_line INTEGER NOT NULL,
             end_line INTEGER NOT NULL,
             signature TEXT NOT NULL DEFAULT '',
             UNIQUE(project_id, qualified_name, start_byte)
         );
         CREATE TABLE IF NOT EXISTS references_raw (
             id INTEGER PRIMARY KEY,
             project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
             file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
             source_qualified_name TEXT NOT NULL,
             kind TEXT NOT NULL,
             target_name TEXT NOT NULL,
             source_line INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS edges (
             id INTEGER PRIMARY KEY,
             project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
             source_id INTEGER NOT NULL REFERENCES symbols(id) ON DELETE CASCADE,
             target_id INTEGER NOT NULL REFERENCES symbols(id) ON DELETE CASCADE,
             kind TEXT NOT NULL,
             confidence REAL NOT NULL,
             strategy TEXT NOT NULL,
             source_line INTEGER,
             UNIQUE(project_id,source_id,target_id,kind,source_line)
         );
         CREATE INDEX IF NOT EXISTS idx_files_project ON files(project_id, rel_path);
         CREATE INDEX IF NOT EXISTS idx_symbols_project_name ON symbols(project_id, name);
         CREATE INDEX IF NOT EXISTS idx_symbols_project_kind ON symbols(project_id, kind);
         CREATE INDEX IF NOT EXISTS idx_symbols_file ON symbols(file_id, start_byte);
         CREATE INDEX IF NOT EXISTS idx_references_project_target
             ON references_raw(project_id,target_name);
         CREATE INDEX IF NOT EXISTS idx_references_file
             ON references_raw(file_id);
         CREATE INDEX IF NOT EXISTS idx_edges_source_kind ON edges(project_id,source_id,kind);
         CREATE INDEX IF NOT EXISTS idx_edges_target_kind ON edges(project_id,target_id,kind);
         CREATE INDEX IF NOT EXISTS idx_edges_source_fk ON edges(source_id);
         CREATE INDEX IF NOT EXISTS idx_edges_target_fk ON edges(target_id);
         CREATE VIRTUAL TABLE IF NOT EXISTS symbols_fts USING fts5(
             name, qualified_name, kind, path,
             content='symbols_search', content_rowid='symbol_id',
             tokenize='unicode61 remove_diacritics 2'
         );
         CREATE TABLE IF NOT EXISTS symbols_search (
             symbol_id INTEGER PRIMARY KEY,
             name TEXT NOT NULL,
             qualified_name TEXT NOT NULL,
             kind TEXT NOT NULL,
             path TEXT NOT NULL
         );
         CREATE TRIGGER IF NOT EXISTS symbols_search_ai AFTER INSERT ON symbols_search BEGIN
             INSERT INTO symbols_fts(rowid,name,qualified_name,kind,path)
             VALUES(new.symbol_id,new.name,new.qualified_name,new.kind,new.path);
         END;
         CREATE TRIGGER IF NOT EXISTS symbols_search_ad AFTER DELETE ON symbols_search BEGIN
             INSERT INTO symbols_fts(symbols_fts,rowid,name,qualified_name,kind,path)
             VALUES('delete',old.symbol_id,old.name,old.qualified_name,old.kind,old.path);
         END;
         CREATE TRIGGER IF NOT EXISTS symbols_ad AFTER DELETE ON symbols BEGIN
             DELETE FROM symbols_search WHERE symbol_id=old.id;
         END;",
    )?;
    connection.execute(
        "INSERT INTO metadata(key,value) VALUES('schema_version',?1)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        [SCHEMA_VERSION.to_string()],
    )?;
    connection.execute(
        "INSERT INTO metadata(key,value) VALUES('extractor_version',?1)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        [EXTRACTOR_VERSION],
    )?;
    Ok(())
}

fn metadata_value(connection: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    use rusqlite::OptionalExtension as _;
    connection
        .query_row("SELECT value FROM metadata WHERE key=?1", [key], |row| {
            row.get(0)
        })
        .optional()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query_plan(connection: &Connection, sql: &str) -> Vec<String> {
        let mut statement = connection.prepare(sql).unwrap();
        statement
            .query_map([], |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    }

    #[test]
    fn incompatible_derived_index_is_rejected_without_rebuilding_data() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT NOT NULL);
                 INSERT INTO metadata VALUES('schema_version','0');
                 INSERT INTO metadata VALUES('extractor_version','old-extractor');
                 CREATE TABLE projects(id INTEGER PRIMARY KEY,name TEXT);
                 INSERT INTO projects(name) VALUES('stale-project');",
            )
            .unwrap();

        assert!(initialize(&connection).is_err());

        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM projects", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(
            metadata_value(&connection, "schema_version").unwrap(),
            Some("0".to_string())
        );
        assert_eq!(
            metadata_value(&connection, "extractor_version").unwrap(),
            Some("old-extractor".to_string())
        );
    }

    #[test]
    fn replacement_cascades_use_foreign_key_indexes() {
        let connection = Connection::open_in_memory().unwrap();
        initialize(&connection).unwrap();

        // Replacing a symbol-dense file deletes its old symbols. SQLite also
        // checks both edge foreign keys for every deleted symbol, so a scan in
        // this plan turns the update path quadratic as the graph grows.
        let symbol_plan = query_plan(
            &connection,
            "EXPLAIN QUERY PLAN DELETE FROM symbols WHERE file_id=1",
        );
        assert!(
            symbol_plan
                .iter()
                .any(|detail| detail.contains("idx_symbols_file")),
            "symbol replacement must seek by file_id: {symbol_plan:?}"
        );
        assert!(
            symbol_plan
                .iter()
                .any(|detail| detail.contains("idx_edges_source_fk")),
            "source-edge cascade must use its foreign-key index: {symbol_plan:?}"
        );
        assert!(
            symbol_plan
                .iter()
                .any(|detail| detail.contains("idx_edges_target_fk")),
            "target-edge cascade must use its foreign-key index: {symbol_plan:?}"
        );
        assert!(
            !symbol_plan
                .iter()
                .any(|detail| detail.contains("SCAN edges")),
            "symbol replacement must not scan the edge table: {symbol_plan:?}"
        );

        let file_plan = query_plan(
            &connection,
            "EXPLAIN QUERY PLAN DELETE FROM files WHERE project_id=1 AND rel_path='src/lib.rs'",
        );
        assert!(
            file_plan
                .iter()
                .any(|detail| detail.contains("idx_symbols_file")),
            "file cascade must seek dependent symbols: {file_plan:?}"
        );
        assert!(
            file_plan
                .iter()
                .any(|detail| detail.contains("idx_references_file")),
            "file cascade must seek dependent references: {file_plan:?}"
        );
        assert!(
            !file_plan.iter().any(|detail| {
                detail.contains("SCAN symbols") || detail.contains("SCAN references_raw")
            }),
            "file replacement must not scan dependent tables: {file_plan:?}"
        );
    }
}
