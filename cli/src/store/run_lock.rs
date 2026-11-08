use nola_application::{ApplicationCommand, SessionRunLease};

use super::{DbSessionRunLock, NolaStore};
use crate::application_port::CliApplicationPort;

impl NolaStore {
    pub fn session_run_locks(&self) -> Result<Vec<DbSessionRunLock>, String> {
        let mut application = CliApplicationPort::persistent(self.path())?;
        let leases: Vec<SessionRunLease> =
            application.execute(ApplicationCommand::ListSessionRunLeases)?;
        Ok(leases.into_iter().map(db_lease).collect())
    }

    pub fn try_acquire_session_run_lock(&self, lock: &DbSessionRunLock) -> Result<bool, String> {
        let mut application = CliApplicationPort::persistent(self.path())?;
        application.execute(ApplicationCommand::AcquireSessionRunLease {
            input: stored_lease(lock),
        })
    }

    pub fn session_run_lock(&self, session_id: &str) -> Result<Option<DbSessionRunLock>, String> {
        let mut application = CliApplicationPort::persistent(self.path())?;
        let lease: Option<SessionRunLease> =
            application.execute(ApplicationCommand::GetSessionRunLease {
                session_id: session_id.to_string(),
            })?;
        Ok(lease.map(db_lease))
    }

    pub fn release_session_run_lock(&self, session_id: &str, owner: &str) -> Result<bool, String> {
        let mut application = CliApplicationPort::persistent(self.path())?;
        application.execute(ApplicationCommand::ReleaseSessionRunLease {
            session_id: session_id.to_string(),
            owner: owner.to_string(),
        })
    }
}

fn stored_lease(value: &DbSessionRunLock) -> SessionRunLease {
    SessionRunLease {
        session_id: value.session_id.clone(),
        owner: value.owner.clone(),
        pid: value.pid,
        process_start: value.process_start.clone(),
        acquired_at_ms: value.acquired_at_ms,
    }
}

fn db_lease(value: SessionRunLease) -> DbSessionRunLock {
    DbSessionRunLock {
        session_id: value.session_id,
        owner: value.owner,
        pid: value.pid,
        process_start: value.process_start,
        acquired_at_ms: value.acquired_at_ms,
    }
}
