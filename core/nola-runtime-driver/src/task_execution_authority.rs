// SPDX-License-Identifier: Apache-2.0
//! Process-local authority for a worker holding exact Store-issued task grants.
//! Persistence and OS execution remain Host callbacks; no callback grants authority
//! until its complete committed receipt is validated and explicitly adopted.
use nola_domain::task_execution::{
    RuntimeTaskAttempt, RuntimeTaskLeaseWitness, RuntimeTaskPermissionSuspension,
};

#[derive(Clone)]
pub struct RuntimeTaskLeaseHandle {
    original: RuntimeTaskLeaseWitness,
    worker_id: String,
    attempt_number: i64,
    state: std::sync::Arc<std::sync::Mutex<RuntimeTaskAttempt>>,
}

/// One worker's explicitly transferred execution authority. Individual lease
/// handles keep their immutable attempt identity across every transfer.
#[derive(Clone)]
pub struct RuntimeTaskExecutionAuthority {
    origin: RuntimeTaskLeaseHandle,
    state: std::sync::Arc<std::sync::Mutex<RuntimeTaskExecutionState>>,
}

impl std::fmt::Debug for RuntimeTaskExecutionAuthority {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Config diagnostics must never expose execution lease credentials.
        formatter.write_str("RuntimeTaskExecutionAuthority(<redacted>)")
    }
}

enum RuntimeTaskExecutionState {
    Active(RuntimeTaskLeaseHandle),
    PermissionSuspended {
        original: RuntimeTaskAttempt,
        suspension: RuntimeTaskPermissionSuspension,
    },
    ResumePending {
        original: RuntimeTaskAttempt,
        suspension: RuntimeTaskPermissionSuspension,
        grant: RuntimeTaskLeaseHandle,
        admitted: RuntimeTaskAttempt,
    },
    PermissionUncertain,
    Revoked,
}

impl RuntimeTaskExecutionAuthority {
    pub fn new(lease: RuntimeTaskLeaseHandle) -> Self {
        Self {
            origin: lease.clone(),
            state: std::sync::Arc::new(std::sync::Mutex::new(RuntimeTaskExecutionState::Active(
                lease,
            ))),
        }
    }

    pub fn owns(&self, lease: &RuntimeTaskLeaseHandle) -> bool {
        if self.origin.same_owner(lease) {
            return true;
        }
        self.state.lock().ok().is_some_and(|state| match &*state {
            RuntimeTaskExecutionState::Active(current)
            | RuntimeTaskExecutionState::ResumePending { grant: current, .. } => {
                current.same_owner(lease)
            }
            _ => false,
        })
    }

    pub fn with_active<T>(
        &self,
        execute: impl FnOnce(&RuntimeTaskLeaseHandle) -> Result<T, String>,
    ) -> Result<Option<T>, String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "task execution authority is poisoned".to_string())?;
        match &*state {
            RuntimeTaskExecutionState::Active(lease) => execute(lease).map(Some),
            _ => Ok(None),
        }
    }

    /// A fresh Store grant may be renewed while local adoption is pending.
    /// It does not authorize work until `activate_permission_resume`.
    pub fn with_renewal<T>(
        &self,
        renew: impl FnOnce(&RuntimeTaskLeaseHandle) -> Result<T, String>,
    ) -> Result<Option<T>, String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "task execution authority is poisoned".to_string())?;
        match &*state {
            RuntimeTaskExecutionState::Active(lease)
            | RuntimeTaskExecutionState::ResumePending { grant: lease, .. } => {
                renew(lease).map(Some)
            }
            _ => Ok(None),
        }
    }

    pub fn active_lease(&self) -> Result<Option<RuntimeTaskLeaseHandle>, String> {
        self.with_active(|lease| Ok(lease.clone()))
    }

    pub fn suspend_permission_with(
        &self,
        request_id: &str,
        continuation_id: &str,
        continuation_json: &str,
        suspend: impl FnOnce(
            &RuntimeTaskAttempt,
        ) -> Result<Option<RuntimeTaskPermissionSuspension>, String>,
    ) -> Result<RuntimeTaskPermissionSuspension, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "task execution authority is poisoned".to_string())?;
        let RuntimeTaskExecutionState::Active(lease) = &*state else {
            return Err("task execution is already suspended or revoked".into());
        };
        let original = lease.snapshot()?;
        let result = suspend(&original);
        // Never renew a potentially revoked original grant after an uncertain
        // suspension acknowledgement. The exact receipt/fact must reconcile it.
        *state = RuntimeTaskExecutionState::PermissionUncertain;
        let suspension = result?.ok_or_else(|| {
            "original task attempt did not admit this permission suspension".to_string()
        })?;
        let expected: serde_json::Value =
            serde_json::from_str(continuation_json).map_err(|error| error.to_string())?;
        let mut received: serde_json::Value = serde_json::from_str(&suspension.continuation_json)
            .map_err(|error| error.to_string())?;
        let source =
            received
                .as_object_mut()
                .and_then(|object| object.remove("core_permission_source"))
                .map(
                    serde_json::from_value::<
                        nola_domain::permission_closure::CorePermissionSourceProof,
                    >,
                )
                .transpose()
                .map_err(|error| error.to_string())?;
        // Store owns provenance, while all Host-supplied continuation fields
        // must remain identical. Never accept caller-supplied proof or ignore
        // arbitrary extra fields in the acknowledgement.
        let source_matches = source.as_ref().is_none_or(|source| {
            expected.get("run_id").and_then(serde_json::Value::as_str)
                == Some(source.permission.run_id.as_str())
                && expected
                    .get("core_permission_request_id")
                    .and_then(serde_json::Value::as_str)
                    == Some(source.permission.core_permission_request_id.as_str())
                && !source.permission.effect_id.trim().is_empty()
                && !source.host_request_id.trim().is_empty()
        });
        if suspension.task_id != original.task_id
            || suspension.attempt_id != original.id
            || suspension.permission_request_id != request_id
            || suspension.continuation_id != continuation_id
            || !expected.is_object()
            || expected.get("core_permission_source").is_some()
            || received != expected
            || !source_matches
            || suspension.state != "pending"
            || suspension.revision != 1
        {
            return Err(
                "permission suspension acknowledgement belongs to another continuation".into(),
            );
        }
        *state = RuntimeTaskExecutionState::PermissionSuspended {
            original,
            suspension: suspension.clone(),
        };
        Ok(suspension)
    }

    pub fn prepare_permission_resume_with(
        &self,
        resume: impl FnOnce(
            &RuntimeTaskAttempt,
            &RuntimeTaskPermissionSuspension,
        ) -> Result<Option<RuntimeTaskAttempt>, String>,
    ) -> Result<Option<RuntimeTaskAttempt>, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "task execution authority is poisoned".to_string())?;
        let RuntimeTaskExecutionState::PermissionSuspended {
            original,
            suspension,
        } = &*state
        else {
            return Err(
                "permission resume requires this worker's exact suspended continuation".into(),
            );
        };
        let original = original.clone();
        let suspension = suspension.clone();
        let result = resume(&original, &suspension);
        // A failed acknowledgement may follow a committed resume. Never
        // restore suspended authority and issue a new proposal in that case.
        *state = RuntimeTaskExecutionState::PermissionUncertain;
        let Some(admitted) = result? else {
            *state = RuntimeTaskExecutionState::PermissionSuspended {
                original,
                suspension,
            };
            return Ok(None);
        };
        if admitted.task_id != original.task_id
            || admitted.id == original.id
            || admitted.attempt_number != original.attempt_number.saturating_add(1)
            || admitted.worker_generation == original.worker_generation
            || admitted.lease_token == original.lease_token
            || admitted.state != "leased"
            || admitted.revision != 1
            || admitted.claimed_at_ms < suspension.suspended_at_ms
            || admitted.lease_expires_at_ms <= admitted.claimed_at_ms
        {
            return Err("permission resume did not return a fresh execution grant".into());
        }
        let original = original.clone();
        let suspension = suspension.clone();
        // Registration preserves old handles. It never changes their witness.
        let grant = register_runtime_task_attempt(admitted.clone());
        if grant.snapshot()? != admitted {
            return Err("permission resume grant was replaced before explicit adoption".into());
        }
        *state = RuntimeTaskExecutionState::ResumePending {
            original,
            suspension,
            grant,
            admitted: admitted.clone(),
        };
        Ok(Some(admitted))
    }

    pub fn activate_permission_resume(
        &self,
        suspension_id: &str,
        admitted: &RuntimeTaskAttempt,
    ) -> Result<RuntimeTaskLeaseHandle, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "task execution authority is poisoned".to_string())?;
        let RuntimeTaskExecutionState::ResumePending {
            suspension,
            grant,
            admitted: original_grant,
            ..
        } = &*state
        else {
            return Err("task execution has no parent-confirmed resume pending".into());
        };
        if suspension.id != suspension_id || original_grant != admitted {
            return Err("permission activation does not match the admitted immutable grant".into());
        }
        let grant = grant.clone();
        *state = RuntimeTaskExecutionState::Active(grant.clone());
        Ok(grant)
    }

    pub fn revoke(&self) {
        if let Ok(mut state) = self.state.lock() {
            *state = RuntimeTaskExecutionState::Revoked;
        }
    }
}

impl RuntimeTaskLeaseHandle {
    pub fn admitted_expiry_ms(&self) -> i64 {
        self.original.current_expiry_ms
    }
    pub fn same_owner(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.state, &other.state)
    }
    pub fn snapshot(&self) -> Result<RuntimeTaskAttempt, String> {
        self.with_attempt(|attempt| Ok(attempt.clone()))
    }

    pub fn with_attempt<T>(
        &self,
        command: impl FnOnce(&mut RuntimeTaskAttempt) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut attempt = self
            .state
            .lock()
            .map_err(|_| "runtime task lease state is poisoned".to_string())?;
        if !self.matches(&attempt)
            || attempt.state != "leased"
            || attempt.revision <= 0
            || attempt.attempt_number <= 0
            || [
                &attempt.id,
                &attempt.task_id,
                &attempt.worker_id,
                &attempt.worker_generation,
                &attempt.lease_token,
            ]
            .iter()
            .any(|value| value.is_empty() || value.trim() != value.as_str())
        {
            return Err("runtime task lease handle identity changed".to_string());
        }
        command(&mut attempt)
    }

    fn matches(&self, attempt: &RuntimeTaskAttempt) -> bool {
        self.original.task_id == attempt.task_id
            && self.original.attempt_id == attempt.id
            && self.original.worker_generation == attempt.worker_generation
            && self.original.lease_token == attempt.lease_token
            && self.worker_id == attempt.worker_id
            && self.attempt_number == attempt.attempt_number
    }
}

pub fn accept_runtime_task_attempt(
    current: &mut RuntimeTaskAttempt,
    next: &RuntimeTaskAttempt,
    expected_expiry_ms: i64,
) -> Result<(), String> {
    if current.task_id != next.task_id
        || current.id != next.id
        || current.worker_id != next.worker_id
        || current.worker_generation != next.worker_generation
        || current.lease_token != next.lease_token
        || current.attempt_number != next.attempt_number
        || current.claimed_at_ms != next.claimed_at_ms
        || next.state != "leased"
        || current.revision.checked_add(1) != Some(next.revision)
        || next.lease_expires_at_ms != expected_expiry_ms
    {
        return Err(
            "runtime task response does not refresh the original attempt identity".to_string(),
        );
    }
    *current = next.clone();
    Ok(())
}

fn lease_states(
) -> &'static std::sync::Mutex<std::collections::HashMap<String, RuntimeTaskLeaseHandle>> {
    static STATES: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, RuntimeTaskLeaseHandle>>,
    > = std::sync::OnceLock::new();
    STATES.get_or_init(Default::default)
}

pub fn register_runtime_task_attempt(attempt: RuntimeTaskAttempt) -> RuntimeTaskLeaseHandle {
    let handle = RuntimeTaskLeaseHandle {
        original: RuntimeTaskLeaseWitness::from(&attempt),
        worker_id: attempt.worker_id.clone(),
        attempt_number: attempt.attempt_number,
        state: std::sync::Arc::new(std::sync::Mutex::new(attempt.clone())),
    };
    if let Ok(mut states) = lease_states().lock() {
        if let Some(current) = states
            .get(&attempt.task_id)
            .filter(|current| current.matches(&attempt))
        {
            return current.clone();
        }
        let replace = states.get(&attempt.task_id).is_none_or(|current| {
            current.original.attempt_id != attempt.id
                && current.attempt_number < attempt.attempt_number
        });
        if replace {
            states.insert(attempt.task_id.clone(), handle.clone());
        }
    }
    handle
}

pub fn release_runtime_task_handle(handle: &RuntimeTaskLeaseHandle) {
    if let Ok(mut states) = lease_states().lock() {
        if states
            .get(&handle.original.task_id)
            .is_some_and(|current| std::sync::Arc::ptr_eq(&current.state, &handle.state))
        {
            states.remove(&handle.original.task_id);
        }
    }
}

pub fn release_runtime_task_attempt(task_id: &str, attempt_id: &str) {
    if let Ok(mut states) = lease_states().lock() {
        let matches = states
            .get(task_id)
            .is_some_and(|handle| handle.original.attempt_id == attempt_id);
        if matches {
            states.remove(task_id);
        }
    }
}

pub fn runtime_task_lease_state(task_id: &str) -> Result<RuntimeTaskLeaseHandle, String> {
    lease_states()
        .lock()
        .map_err(|_| "runtime task lease registry is poisoned".to_string())?
        .get(task_id)
        .cloned()
        .ok_or_else(|| format!("runtime task {task_id} has no retained attempt identity"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attempt(task: &str, number: i64) -> RuntimeTaskAttempt {
        RuntimeTaskAttempt {
            id: format!("{task}-attempt-{number}"),
            task_id: task.into(),
            attempt_number: number,
            worker_id: "worker".into(),
            worker_generation: format!("generation-{number}"),
            lease_token: format!("token-{number}"),
            lease_expires_at_ms: 1000,
            state: "leased".into(),
            revision: 1,
            claimed_at_ms: 100 + number,
        }
    }

    fn suspension(original: &RuntimeTaskAttempt) -> RuntimeTaskPermissionSuspension {
        RuntimeTaskPermissionSuspension {
            id: format!("{}-suspension", original.task_id),
            task_id: original.task_id.clone(),
            attempt_id: original.id.clone(),
            permission_request_id: "permission".into(),
            continuation_id: "continuation".into(),
            continuation_json: "{}".into(),
            state: "pending".into(),
            revision: 1,
            suspended_at_ms: 102,
            resumed_at_ms: None,
        }
    }

    #[test]
    fn lease_callback_holds_identity_lock_through_host_observation() {
        let lease = register_runtime_task_attempt(attempt("shared-lock-scope", 1));
        lease
            .with_attempt(|_| {
                assert!(
                    lease.state.try_lock().is_err(),
                    "Host callback escaped the authority lock"
                );
                Ok(())
            })
            .unwrap();
        release_runtime_task_handle(&lease);
    }

    #[test]
    fn uncertain_suspension_acknowledgement_revokes_execution_and_renewal() {
        let lease = register_runtime_task_attempt(attempt("shared-uncertain", 1));
        let authority = RuntimeTaskExecutionAuthority::new(lease.clone());
        assert!(authority
            .suspend_permission_with("permission", "continuation", "{}", |_| {
                Err("Host lost the committed receipt".into())
            })
            .is_err());
        assert!(authority
            .with_active::<()>(|_| panic!("uncertain grant executed"))
            .unwrap()
            .is_none());
        assert!(authority
            .with_renewal::<()>(|_| panic!("uncertain grant renewed"))
            .unwrap()
            .is_none());
        release_runtime_task_handle(&lease);
    }

    #[test]
    fn fresh_grant_is_renewable_but_not_executable_before_exact_adoption() {
        let lease = register_runtime_task_attempt(attempt("shared-adoption", 1));
        let authority = RuntimeTaskExecutionAuthority::new(lease.clone());
        let receipt = authority
            .suspend_permission_with("permission", "continuation", "{}", |original| {
                Ok(Some(suspension(original)))
            })
            .unwrap();
        let admitted = authority
            .prepare_permission_resume_with(|_, _| Ok(Some(attempt("shared-adoption", 2))))
            .unwrap()
            .unwrap();
        assert!(authority.active_lease().unwrap().is_none());
        assert_eq!(
            authority.with_renewal(|grant| grant.snapshot()).unwrap(),
            Some(admitted.clone())
        );
        let mut foreign = admitted.clone();
        foreign.lease_token = "foreign".into();
        assert!(authority
            .activate_permission_resume(&receipt.id, &foreign)
            .is_err());
        assert!(authority.active_lease().unwrap().is_none());
        let grant = authority
            .activate_permission_resume(&receipt.id, &admitted)
            .unwrap();
        assert!(!grant.same_owner(&lease));
        assert_eq!(
            authority
                .active_lease()
                .unwrap()
                .unwrap()
                .snapshot()
                .unwrap(),
            admitted
        );
        release_runtime_task_handle(&lease);
        assert!(runtime_task_lease_state("shared-adoption")
            .unwrap()
            .same_owner(&grant));
        release_runtime_task_handle(&grant);
    }
}
