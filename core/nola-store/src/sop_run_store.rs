// SPDX-License-Identifier: Apache-2.0
//! Durable SOP run creation, compare-and-swap updates, and queries.

use super::*;
use nola_store_port::sop_run::{VisibleActiveSopRuns, MAX_VISIBLE_ACTIVE_SOP_RUNS};

impl NolaStore {
    /// One SELECT snapshot; visibility and active state are evaluated before
    /// the resource guard. Overflow is explicit, not incomplete model context.
    pub fn visible_active_sop_runs(
        &self,
        query: &VisibleActiveSopRuns,
    ) -> StoreResult<Vec<StoredSopRun>> {
        if query.project_root.trim().is_empty()
            || query.owner_session_id.as_deref().is_some_and(|id| id.trim().is_empty())
        {
            return Err(StoreError::Invariant("SOP context requires a nonempty workspace and owner identity".into()));
        }
        let mut statement = self.connection.prepare(
            "SELECT run_id,sop_id,sop_version,owner_session_id,project_root,status,
                definition_json,record_json,revision,created_at_ms,updated_at_ms
             FROM sop_runs
             WHERE project_root=?1 AND (owner_session_id IS NULL OR owner_session_id=?2)
               AND status NOT IN ('completed','failed','cancelled')
             ORDER BY updated_at_ms DESC,run_id LIMIT ?3",
        )?;
        let rows = statement.query_map(
            params![query.project_root, query.owner_session_id, (MAX_VISIBLE_ACTIVE_SOP_RUNS + 1) as i64],
            sop_run_row,
        )?.collect::<Result<Vec<_>, _>>()?;
        if rows.len() > MAX_VISIBLE_ACTIVE_SOP_RUNS {
            return Err(StoreError::Invariant("active SOP context exceeds resource bound; resolve active runs before retrying".into()));
        }
        Ok(rows)
    }

    pub fn create_sop_run(&mut self, run: &StoredSopRun) -> StoreResult<()> {
        validate_sop_run(run)?;
        self.connection.execute(
            "INSERT INTO sop_runs(run_id,sop_id,sop_version,owner_session_id,project_root,status,
                definition_json,record_json,revision,created_at_ms,updated_at_ms)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![
                run.run_id,
                run.sop_id,
                run.sop_version,
                run.owner_session_id,
                run.project_root,
                run.status,
                run.definition_json,
                run.record_json,
                run.revision,
                run.created_at_ms,
                run.updated_at_ms,
            ],
        )?;
        Ok(())
    }

    pub fn update_sop_run_cas(
        &mut self,
        run: &StoredSopRun,
        expected_revision: i64,
    ) -> StoreResult<bool> {
        validate_sop_run(run)?;
        if run.revision != expected_revision + 1 {
            return Err(StoreError::Invariant(
                "SOP run revision must advance by exactly one".into(),
            ));
        }
        Ok(self.connection.execute(
            "UPDATE sop_runs SET status=?2,record_json=?3,revision=?4,updated_at_ms=?5
             WHERE run_id=?1 AND sop_id=?6 AND sop_version=?7 AND revision=?8",
            params![
                run.run_id,
                run.status,
                run.record_json,
                run.revision,
                run.updated_at_ms,
                run.sop_id,
                run.sop_version,
                expected_revision,
            ],
        )? == 1)
    }

    pub fn sop_run(&self, run_id: &str) -> StoreResult<Option<StoredSopRun>> {
        self.connection
            .query_row(
                "SELECT run_id,sop_id,sop_version,owner_session_id,project_root,status,
                    definition_json,record_json,revision,created_at_ms,updated_at_ms
                 FROM sop_runs WHERE run_id=?1",
                params![run_id],
                sop_run_row,
            )
            .optional()
            .map_err(StoreError::from)
    }

    pub fn list_sop_runs(
        &self,
        project_root: Option<&str>,
        owner_session_id: Option<&str>,
        limit: usize,
    ) -> StoreResult<Vec<StoredSopRun>> {
        let mut statement = self.connection.prepare(
            "SELECT run_id,sop_id,sop_version,owner_session_id,project_root,status,
                definition_json,record_json,revision,created_at_ms,updated_at_ms
             FROM sop_runs
             WHERE (?1 IS NULL OR project_root=?1) AND (?2 IS NULL OR owner_session_id=?2)
             ORDER BY updated_at_ms DESC,run_id LIMIT ?3",
        )?;
        let rows = statement.query_map(
            params![project_root, owner_session_id, limit.clamp(1, 2_000) as i64],
            sop_run_row,
        )?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }
}

#[cfg(test)]
mod context_tests {
    use super::*;

    fn row(id: &str, owner: Option<&str>, root: &str, status: &str, updated: i64) -> StoredSopRun {
        StoredSopRun {
            run_id: id.into(), sop_id: "workflow".into(), sop_version: 1,
            owner_session_id: owner.map(str::to_owned), project_root: root.into(),
            status: status.into(), definition_json: "{}".into(), record_json: "{}".into(),
            revision: 1, created_at_ms: 1, updated_at_ms: updated,
        }
    }

    #[test]
    fn active_context_filters_before_bound_and_keeps_workspace_owned_runs() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = NolaStore::open(directory.path().join("sop.sqlite")).unwrap();
        for index in 0..40 {
            store.create_sop_run(&row(&format!("own-{index}"), Some("session"), "/workspace", "running", 1)).unwrap();
            store.create_sop_run(&row(&format!("foreign-{index}"), Some("other"), "/workspace", "running", 100)).unwrap();
            store.create_sop_run(&row(&format!("other-root-{index}"), Some("session"), "/other", "running", 100)).unwrap();
            for status in ["completed", "failed", "cancelled"] {
                store.create_sop_run(&row(&format!("{status}-{index}"), Some("session"), "/workspace", status, 100)).unwrap();
            }
        }
        store.create_sop_run(&row("workspace", None, "/workspace", "paused", 1)).unwrap();
        let query = VisibleActiveSopRuns { project_root: "/workspace".into(), owner_session_id: Some("session".into()) };
        let visible = store.visible_active_sop_runs(&query).unwrap();
        assert_eq!(visible.len(), 41);
        assert!(visible.iter().all(|item| item.run_id.starts_with("own-") || item.run_id == "workspace"));
        let workspace_only = store.visible_active_sop_runs(&VisibleActiveSopRuns { owner_session_id: None, ..query }).unwrap();
        assert_eq!(workspace_only.len(), 1);
        assert_eq!(workspace_only[0].run_id, "workspace");
    }

    #[test]
    fn active_context_overflow_is_an_error_not_a_partial_page() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = NolaStore::open(directory.path().join("sop.sqlite")).unwrap();
        let query = VisibleActiveSopRuns { project_root: "/workspace".into(), owner_session_id: Some("session".into()) };
        for index in 0..MAX_VISIBLE_ACTIVE_SOP_RUNS {
            store.create_sop_run(&row(&format!("run-{index}"), Some("session"), "/workspace", "queued", 1)).unwrap();
        }
        assert_eq!(store.visible_active_sop_runs(&query).unwrap().len(), MAX_VISIBLE_ACTIVE_SOP_RUNS);
        store.create_sop_run(&row("overflow", None, "/workspace", "waiting_user", 1)).unwrap();
        assert!(store.visible_active_sop_runs(&query).unwrap_err().to_string().contains("resource bound"));
    }

    #[test]
    fn active_context_requires_exact_durable_workspace_and_nonblank_identity() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = NolaStore::open(directory.path().join("sop.sqlite")).unwrap();
        let root = directory.path().to_string_lossy().into_owned();
        store.create_sop_run(&row("run", None, &root, "running", 1)).unwrap();
        let mut query = VisibleActiveSopRuns { project_root: format!("{root}/."), owner_session_id: None };
        // A filesystem-equivalent spelling is not an alternate durable identity.
        assert!(store.visible_active_sop_runs(&query).unwrap().is_empty());
        query.project_root = root;
        assert_eq!(store.visible_active_sop_runs(&query).unwrap().len(), 1);
        query.owner_session_id = Some(" ".into());
        assert!(store.visible_active_sop_runs(&query).is_err());
        query.owner_session_id = None;
        query.project_root.clear();
        assert!(store.visible_active_sop_runs(&query).is_err());
    }
}
