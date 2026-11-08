// SPDX-License-Identifier: Apache-2.0
#[derive(Default)]
struct Repository {
    row: Option<StoredSopRun>,
    reject_cas: bool,
}
impl SopRunRepository for Repository {
    type Error = String;
    fn create_sop_run(&mut self, row: &StoredSopRun) -> Result<(), String> {
        if self.row.is_some() {
            return Err("duplicate".into());
        };
        self.row = Some(row.clone());
        Ok(())
    }
    fn update_sop_run_cas(&mut self, row: &StoredSopRun, expected: i64) -> Result<bool, String> {
        if self.reject_cas || self.row.as_ref().is_none_or(|v| v.revision != expected) {
            return Ok(false);
        };
        self.row = Some(row.clone());
        Ok(true)
    }
    fn sop_run(&self, id: &str) -> Result<Option<StoredSopRun>, String> {
        Ok(self.row.clone().filter(|v| v.run_id == id))
    }
    fn visible_active_sop_runs(&self, query: &VisibleActiveSopRuns) -> Result<Vec<StoredSopRun>, String> {
        Ok(self.row.iter().filter(|row| {
            row.project_root == query.project_root
                && (row.owner_session_id.is_none() || row.owner_session_id == query.owner_session_id)
                && !matches!(row.status.as_str(), "completed" | "failed" | "cancelled")
        }).cloned().collect())
    }
    fn list_sop_runs(
        &self,
        project: Option<&str>,
        owner: Option<&str>,
        limit: usize,
    ) -> Result<Vec<StoredSopRun>, String> {
        Ok(self
            .row
            .iter()
            .filter(|v| {
                project.is_none_or(|p| p == v.project_root)
                    && owner.is_none_or(|o| Some(o) == v.owner_session_id.as_deref())
            })
            .take(limit)
            .cloned()
            .collect())
    }
}
#[test]
fn non_sqlite_sop_core_transition_and_cas_failure() {
    let mut repository = Repository::default();
    execute_sop_run_command(
        &mut repository,
        SopRunCommand::Create {
            definition: definition(),
            run_id: "run".into(),
            inputs: json!({}),
            owner_session_id: None,
            project_root: "/workspace".into(),
            now_ms: 1,
        },
    )
    .unwrap();
    let start = || SopRunCommand::Apply {
        run_id: "run".into(),
        expected_revision: 1,
        event: SopRunEvent::StartStep {
            step_id: "work".into(),
            executor_ref: "executor".into(),
        },
        now_ms: 2,
    };
    repository.reject_cas = true;
    assert!(execute_sop_run_command(&mut repository, start())
        .unwrap_err()
        .contains("revision conflict during commit"));
    assert_eq!(repository.row.as_ref().unwrap().revision, 1);
    repository.reject_cas = false;
    let started = execute_sop_run_command(&mut repository, start()).unwrap();
    assert_eq!(started["record"]["steps"]["work"]["status"], "running");
    assert!(execute_sop_run_command(&mut repository, start())
        .unwrap_err()
        .contains("revision conflict"));
    let wait = || SopRunCommand::EnsurePermissionWait {
        run_id: "run".into(),
        step_id: "work".into(),
        reference_id: "permission".into(),
        reason: "review".into(),
        now_ms: 3,
    };
    let waiting = execute_sop_run_command(&mut repository, wait()).unwrap();
    assert_eq!(
        execute_sop_run_command(&mut repository, wait()).unwrap(),
        waiting
    );
    assert_eq!(waiting["record"]["status"], "waiting_permission");
}
