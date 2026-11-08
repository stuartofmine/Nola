// SPDX-License-Identifier: Apache-2.0
//! Opt-in, same-thread, bounded diagnostic observations; never business authority.
use serde::Serialize;
use std::{
    cell::{Cell, RefCell},
    marker::PhantomData,
    rc::Rc,
    time::Instant,
};

const CAPACITY: usize = 128;
thread_local! { static CAPTURE: RefCell<Option<Report>> = const { RefCell::new(None) }; }
thread_local! { static GENERATION: Cell<u64> = const { Cell::new(0) }; }

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    DisclosureFlush,
    SqlCommit,
    PostCommitArchive,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Succeeded,
    Failed,
    Skipped,
    Incomplete,
}
#[derive(Debug, Serialize)]
pub struct Observation {
    pub stage: Stage,
    pub elapsed_ns: String,
    pub outcome: Outcome,
}
#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub observations: Vec<Observation>,
    pub dropped: usize,
    #[serde(skip)]
    generation: u64,
}

/// Captures only this thread. Explicitly refuses nesting; Drop disables capture on unwind.
pub struct Capture(PhantomData<Rc<()>>);
pub fn begin_capture() -> Result<Capture, &'static str> {
    CAPTURE.with(|state| {
        let mut state = state.borrow_mut();
        if state.is_some() {
            return Err("commit diagnostic capture already active");
        }
        let generation = GENERATION.with(|value| {
            let next = value
                .get()
                .checked_add(1)
                .ok_or("diagnostic generation exhausted")?;
            value.set(next);
            Ok::<u64, &'static str>(next)
        })?;
        *state = Some(Report {
            generation,
            ..Default::default()
        });
        Ok(Capture(PhantomData))
    })
}
impl Capture {
    pub fn finish(self) -> Report {
        CAPTURE.with(|state| state.borrow_mut().take().unwrap_or_default())
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        CAPTURE.with(|state| {
            state.borrow_mut().take();
        });
    }
}

/// Disabled mode does not allocate observations or read the monotonic clock.
pub(crate) struct Timer {
    stage: Stage,
    start: Option<Instant>,
    generation: Option<u64>,
    outcome: Outcome,
}
pub(crate) fn start(stage: Stage) -> Timer {
    let generation = CAPTURE.with(|state| state.borrow().as_ref().map(|report| report.generation));
    Timer {
        stage,
        start: generation.map(|_| Instant::now()),
        generation,
        outcome: Outcome::Incomplete,
    }
}
impl Timer {
    pub(crate) fn finish(mut self, outcome: Outcome) {
        self.outcome = outcome;
    }
}
impl Drop for Timer {
    fn drop(&mut self) {
        let Some(start) = self.start else { return };
        let elapsed = start.elapsed();
        CAPTURE.with(|state| {
            let mut state = state.borrow_mut();
            if let Some(report) = state.as_mut() {
                if Some(report.generation) != self.generation {
                    return;
                }
                if report.observations.len() < CAPACITY {
                    report.observations.push(Observation {
                        stage: self.stage,
                        elapsed_ns: elapsed.as_nanos().to_string(),
                        outcome: self.outcome,
                    });
                } else {
                    report.dropped = report.dropped.saturating_add(1);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disabled_collection_allocates_no_report_or_timer_clock() {
        let timer = start(Stage::SqlCommit);
        assert!(timer.start.is_none());
        timer.finish(Outcome::Succeeded);
        CAPTURE.with(|state| assert!(state.borrow().is_none()));
    }
    #[test]
    fn failures_missing_completion_and_capacity_are_not_success() {
        let capture = begin_capture().unwrap();
        assert!(begin_capture().is_err());
        start(Stage::SqlCommit).finish(Outcome::Failed);
        drop(start(Stage::DisclosureFlush));
        for _ in 0..CAPACITY {
            start(Stage::PostCommitArchive).finish(Outcome::Skipped);
        }
        let report = capture.finish();
        assert_eq!(report.observations.len(), CAPACITY);
        assert_eq!(report.dropped, 2);
        assert_eq!(report.observations[0].outcome, Outcome::Failed);
        assert_eq!(report.observations[1].outcome, Outcome::Incomplete);
        assert!(start(Stage::SqlCommit).start.is_none());
    }
    #[test]
    fn collection_is_thread_local_and_unwind_disables_it() {
        let _ = std::panic::catch_unwind(|| {
            let _capture = begin_capture().unwrap();
            std::thread::spawn(|| assert!(start(Stage::SqlCommit).start.is_none()))
                .join()
                .unwrap();
            panic!("fixture");
        });
        assert!(start(Stage::SqlCommit).start.is_none());
    }
    #[test]
    fn real_sql_commit_failure_is_recorded_and_archive_is_not_attempted() {
        let mut store = crate::NolaStore::open(":memory:").unwrap();
        store.connection.execute_batch("CREATE TABLE diagnostic_parent(id INTEGER PRIMARY KEY);
            CREATE TABLE diagnostic_child(id INTEGER REFERENCES diagnostic_parent(id) DEFERRABLE INITIALLY DEFERRED);").unwrap();
        store.begin_write_unit_of_work().unwrap();
        store
            .connection
            .execute("INSERT INTO diagnostic_child(id) VALUES(1)", [])
            .unwrap();
        let capture = begin_capture().unwrap();
        assert!(store.commit_write_unit_of_work().is_err());
        let report = capture.finish();
        assert_eq!(report.observations.len(), 2);
        assert_eq!(report.observations[0].outcome, Outcome::Succeeded);
        assert_eq!(report.observations[1].outcome, Outcome::Failed);
        assert!(matches!(report.observations[1].stage, Stage::SqlCommit));
    }
    #[test]
    fn old_timer_does_not_cross_capture_generation() {
        let first = begin_capture().unwrap();
        let timer = start(Stage::SqlCommit);
        assert!(first.finish().observations.is_empty());
        let second = begin_capture().unwrap();
        timer.finish(Outcome::Succeeded);
        assert!(second.finish().observations.is_empty());
    }
    #[test]
    fn archive_failure_preserves_database_commit_and_failed_observation() {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("diagnostic.sqlite");
        let mut store = crate::NolaStore::open(&db).unwrap();
        store
            .create_conversation(&crate::CreateConversationInput {
                id: "diagnostic".into(),
                title: "fixture".into(),
                revision: 1,
                settings_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
            })
            .unwrap();
        std::fs::write(store.work_archive_root().unwrap(), b"not a directory").unwrap();
        store.begin_write_unit_of_work().unwrap();
        store
            .save_checkpoint(&crate::SaveCheckpointInput {
                conversation_id: "diagnostic".into(),
                expected_revision: 0,
                revision: 1,
                event_cursor: 1,
                updated_at_ms: 1,
                checkpoint_json: r#"{"revision":1,"state":{"conversation":{"transcript":[]}}}"#
                    .into(),
            })
            .unwrap();
        let capture = begin_capture().unwrap();
        store.commit_write_unit_of_work().unwrap();
        let report = capture.finish();
        assert_eq!(
            report.observations.len(),
            2,
            "ordinary ACK does not wait for archive"
        );
        assert_eq!(report.observations[1].outcome, Outcome::Succeeded);
        let capture = begin_capture().unwrap();
        assert!(store.flush_work_archives().is_err());
        let report = capture.finish();
        assert_eq!(report.observations.len(), 1);
        assert_eq!(report.observations[0].outcome, Outcome::Failed);
        assert!(matches!(
            report.observations[0].stage,
            Stage::PostCommitArchive
        ));
        drop(store);
        let reopened = crate::NolaStore::open(&db).unwrap();
        assert_eq!(
            reopened
                .load_checkpoint("diagnostic")
                .unwrap()
                .unwrap()
                .revision,
            1
        );
    }
}
