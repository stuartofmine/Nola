// SPDX-License-Identifier: Apache-2.0
//! Re-resolve changed definitions and their callers, preserving unrelated edges.

use super::*;

#[derive(Default)]
pub(super) struct EdgeChanges {
    names: HashSet<String>,
    qualified_names: HashSet<String>,
}

impl EdgeChanges {
    pub(super) fn record_file(
        &mut self,
        tx: &Transaction<'_>,
        project: i64,
        path: &str,
    ) -> Result<(), String> {
        let mut statement = tx
            .prepare_cached(
                "SELECT s.name,s.qualified_name FROM symbols s JOIN files f ON f.id=s.file_id
             WHERE f.project_id=?1 AND f.rel_path=?2",
            )
            .map_err(db_error)?;
        for row in statement
            .query_map(params![project, path], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(db_error)?
        {
            let (name, qualified) = row.map_err(db_error)?;
            self.names.insert(name);
            self.qualified_names.insert(qualified);
        }
        Ok(())
    }

    pub(super) fn stage(
        tx: &Transaction<'_>,
        project: i64,
        changes: Option<&Self>,
    ) -> Result<(), String> {
        // Connection-local scratch tables, never a persisted schema change.
        tx.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS nola_dirty_names(name TEXT PRIMARY KEY);
             CREATE TEMP TABLE IF NOT EXISTS nola_dirty_qns(name TEXT PRIMARY KEY);
             CREATE TEMP TABLE IF NOT EXISTS nola_dirty_sources(name TEXT PRIMARY KEY);
             DELETE FROM nola_dirty_names;
             DELETE FROM nola_dirty_qns;
             DELETE FROM nola_dirty_sources;",
        )
        .map_err(db_error)?;
        let Some(changes) = changes else {
            return Ok(());
        };
        for name in &changes.names {
            tx.prepare_cached("INSERT INTO temp.nola_dirty_names VALUES(?1)")
                .map_err(db_error)?
                .execute([name])
                .map_err(db_error)?;
        }
        for name in &changes.qualified_names {
            tx.prepare_cached("INSERT INTO temp.nola_dirty_qns VALUES(?1)")
                .map_err(db_error)?
                .execute([name])
                .map_err(db_error)?;
        }
        // A changed name can create/remove ambiguity in an unchanged caller.
        // Rebuild all outgoing references of those sources, not just the
        // reference to that name, since one source may call several targets.
        tx.execute(
            "INSERT OR IGNORE INTO temp.nola_dirty_sources
                    SELECT name FROM temp.nola_dirty_qns",
            [],
        )
        .map_err(db_error)?;
        tx.execute(
            "INSERT OR IGNORE INTO temp.nola_dirty_sources
             SELECT source_qualified_name FROM references_raw WHERE project_id=?1
               AND target_name IN (SELECT name FROM temp.nola_dirty_names)",
            [project],
        )
        .map_err(db_error)?;
        tx.execute(
            "DELETE FROM edges WHERE project_id=?1 AND (
              (kind NOT IN ('CONTAINS','DEFINES') AND source_id IN (
                SELECT id FROM symbols WHERE project_id=?1 AND qualified_name IN
                  (SELECT name FROM temp.nola_dirty_sources)))
              OR (kind IN ('CONTAINS','DEFINES') AND (
                source_id IN (SELECT id FROM symbols WHERE project_id=?1 AND qualified_name IN
                  (SELECT name FROM temp.nola_dirty_qns))
                OR target_id IN (SELECT id FROM symbols WHERE project_id=?1 AND qualified_name IN
                  (SELECT name FROM temp.nola_dirty_qns)))))",
            [project],
        )
        .map_err(db_error)?;
        Ok(())
    }
}
