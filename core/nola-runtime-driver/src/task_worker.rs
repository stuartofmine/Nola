// SPDX-License-Identifier: Apache-2.0
//! Child Run lifecycle shared by process and embedded Hosts. Application owns
//! Task admission, the Task lease, Run binding, and terminal reconciliation.
//! The executor below never keeps a second queue or retries an uncertain call.
use crate::task_execution_authority::{RuntimeTaskExecutionAuthority, RuntimeTaskLeaseHandle, register_runtime_task_attempt};
use nola_domain::task_execution::RuntimeTaskAttempt;
use nola_domain::task_execution::RuntimeTaskRunBinding;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskChildIdentity {
    pub task_id: String,
    pub owner_conversation_id: String,
    pub child_conversation_id: String,
    pub input_watermark: i64,
}

impl TaskChildIdentity {
    fn validate(&self) -> Result<(), String> {
        if [&self.task_id, &self.owner_conversation_id, &self.child_conversation_id].iter()
            .any(|id| id.is_empty() || id.trim() != id.as_str() || id.chars().any(char::is_control))
            || self.owner_conversation_id == self.child_conversation_id || self.input_watermark < 0 {
            return Err("invalid child Task execution identity".into());
        }
        Ok(())
    }
    fn validate_binding(&self, binding: &RuntimeTaskRunBinding) -> Result<(), String> {
        if binding.task_id != self.task_id || binding.owner_session_id != self.owner_conversation_id
            || binding.child_session_id != self.child_conversation_id
            || binding.input_watermark != Some(self.input_watermark)
            || binding.run_id.is_empty() || binding.admission_attempt_id.is_empty() {
            return Err("Task Run binding does not match the worker's immutable identity".into());
        }
        Ok(())
    }
}

/// Adopts only the exact grant returned by ClaimAgentTaskFair. A promoted Task
/// or a grant belonging to another worker is not permission to execute a child.
pub fn adopt_task_worker_claim(
    task_id: &str, worker_id: &str, worker_generation: &str, lease_token: &str,
    now_ms: i64, attempt: RuntimeTaskAttempt,
) -> Result<RuntimeTaskLeaseHandle, String> {
    if attempt.task_id != task_id || attempt.worker_id != worker_id
        || attempt.worker_generation != worker_generation || attempt.lease_token != lease_token
        || attempt.state != "leased" || attempt.lease_expires_at_ms <= now_ms
        || attempt.claimed_at_ms > now_ms || attempt.attempt_number <= 0 || attempt.revision <= 0 {
        return Err("Task worker claim has no exact live Application grant".into());
    }
    Ok(register_runtime_task_attempt(attempt))
}

/// Host adapter around existing Application operations and the existing
/// CommittedRuntimeDriver. Both execution methods use their own child driver;
/// neither may borrow foreground mutable state or admit a successor Run.
pub trait TaskChildAuthorityPort {
    fn load_binding(&mut self, task_id: &str) -> Result<Option<RuntimeTaskRunBinding>, String>;
    /// Calls Application ReconcileBoundRunTerminal, whose receipt is derived
    /// solely from the exact committed child seal, never a local pump result.
    fn reconcile_bound_terminal(&mut self, task_id: &str) -> Result<bool, String>;
}

pub trait TaskChildRunPort: TaskChildAuthorityPort {
    type Outcome;
    /// Atomically BeginTaskAssistantTurn with this authority, then pump its
    /// committed HostRequests. The Application rejects duplicate admission.
    fn begin_and_execute(&mut self, authority: &RuntimeTaskExecutionAuthority) -> Result<Self::Outcome, String>;
    /// Load the exact frozen invocation and resume this bound Run only.
    fn resume_and_execute(&mut self, run_id: &str, authority: &RuntimeTaskExecutionAuthority) -> Result<Self::Outcome, String>;
}

/// A missing binding is the sole fresh-admission path. Permission resume must
/// also match the immutable Task binding. Errors propagate without reissuing a
/// Host operation or falling back to a new child; uncertain delivery remains
/// Application-owned recovery work.
pub fn execute_task_child<P: TaskChildRunPort>(
    identity: &TaskChildIdentity,
    authority: &RuntimeTaskExecutionAuthority,
    expected_permission_run: Option<&str>,
    port: &mut P,
) -> Result<P::Outcome, String> {
    identity.validate()?;
    let lease = authority.active_lease()?.ok_or("Task worker has no active execution authority")?;
    let attempt = lease.snapshot()?;
    if attempt.task_id != identity.task_id { return Err("Task worker lease belongs to another Task".into()); }
    let binding = port.load_binding(&identity.task_id)?;
    // The read above must not allow an uncertain permission transfer to reuse
    // the former grant. This is an in-process fence, not a replacement for the
    // exact Store fence enforced by BeginTaskAssistantTurn/HostResponse.
    let current = authority.active_lease()?.ok_or("Task execution authority changed during binding read")?;
    if !current.same_owner(&lease) { return Err("Task execution grant changed during binding read".into()); }
    match binding {
        Some(binding) => {
            identity.validate_binding(&binding)?;
            if expected_permission_run.is_some_and(|run| run != binding.run_id) {
                return Err("permission continuation does not own the bound child Run".into());
            }
            port.resume_and_execute(&binding.run_id, authority)
        }
        None if expected_permission_run.is_some() => Err("permission continuation has no bound child Run".into()),
        None => port.begin_and_execute(authority),
    }
}

/// Reconciliation remains valid after an execution lease has ended; immutable
/// lineage and the committed seal, not a mutable worker token, prove ownership.
pub fn reconcile_task_child<P: TaskChildAuthorityPort>(identity: &TaskChildIdentity, port: &mut P) -> Result<bool, String> {
    identity.validate()?;
    let Some(binding) = port.load_binding(&identity.task_id)? else { return Ok(false); };
    identity.validate_binding(&binding)?;
    port.reconcile_bound_terminal(&identity.task_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Port {
        binding: Option<RuntimeTaskRunBinding>, calls: Vec<&'static str>, uncertain: bool,
        revoke_on_read: Option<RuntimeTaskExecutionAuthority>, terminal: bool,
    }
    impl TaskChildAuthorityPort for Port {
        fn load_binding(&mut self, _: &str) -> Result<Option<RuntimeTaskRunBinding>, String> {
            if let Some(authority) = &self.revoke_on_read { authority.revoke(); }
            Ok(self.binding.clone())
        }
        fn reconcile_bound_terminal(&mut self, _: &str) -> Result<bool, String> {
            self.calls.push("reconcile"); Ok(self.terminal)
        }
    }
    impl TaskChildRunPort for Port {
        type Outcome = String;
        fn begin_and_execute(&mut self, _: &RuntimeTaskExecutionAuthority) -> Result<String, String> {
            self.calls.push("begin"); Ok("new".into())
        }
        fn resume_and_execute(&mut self, run: &str, _: &RuntimeTaskExecutionAuthority) -> Result<String, String> {
            self.calls.push("resume");
            if self.uncertain { Err("delivery outcome uncertain".into()) } else { Ok(run.into()) }
        }
    }
    fn fixture(task: &str, bound: bool) -> (TaskChildIdentity, RuntimeTaskExecutionAuthority, Port) {
        let identity = TaskChildIdentity { task_id: task.into(), owner_conversation_id: "parent".into(),
            child_conversation_id: "child".into(), input_watermark: 42 };
        let lease = adopt_task_worker_claim(task, "worker", "generation", "token", 200, RuntimeTaskAttempt {
            id: format!("{task}-attempt"), task_id: task.into(), attempt_number: 1,
            worker_id: "worker".into(), worker_generation: "generation".into(), lease_token: "token".into(),
            lease_expires_at_ms: 1000, state: "leased".into(), revision: 1, claimed_at_ms: 100,
        }).unwrap();
        let port = Port { binding: bound.then(|| RuntimeTaskRunBinding {
            task_id: task.into(), owner_session_id: "parent".into(), child_session_id: "child".into(),
            run_id: "exact-run".into(), admission_attempt_id: "original-attempt".into(),
            input_watermark: Some(42), bound_at_ms: 100,
        }), calls: vec![], uncertain: false, revoke_on_read: None, terminal: false };
        (identity, RuntimeTaskExecutionAuthority::new(lease), port)
    }
    #[test]
    fn only_unbound_child_can_begin_and_bound_child_resumes_once() {
        let (id, authority, mut port) = fixture("worker-new", false);
        assert_eq!(execute_task_child(&id, &authority, None, &mut port).unwrap(), "new");
        assert_eq!(port.calls, ["begin"]);
        let (id, authority, mut port) = fixture("worker-cold", true);
        assert_eq!(execute_task_child(&id, &authority, None, &mut port).unwrap(), "exact-run");
        assert_eq!(port.calls, ["resume"]);
    }
    #[test]
    fn uncertain_resume_never_falls_back_to_new_admission() {
        let (id, authority, mut port) = fixture("worker-uncertain", true);
        port.uncertain = true;
        assert_eq!(execute_task_child(&id, &authority, None, &mut port).unwrap_err(), "delivery outcome uncertain");
        assert_eq!(port.calls, ["resume"]);
    }
    #[test]
    fn foreign_binding_or_permission_run_cannot_execute_or_reconcile() {
        let (id, authority, mut port) = fixture("worker-foreign", true);
        assert!(execute_task_child(&id, &authority, Some("other-run"), &mut port).is_err());
        port.binding.as_mut().unwrap().owner_session_id = "other-parent".into();
        assert!(execute_task_child(&id, &authority, None, &mut port).is_err());
        assert!(reconcile_task_child(&id, &mut port).is_err());
        assert!(port.calls.is_empty());
    }
    #[test]
    fn permission_resume_without_binding_never_starts_a_new_turn() {
        let (id, authority, mut port) = fixture("worker-no-binding", false);
        assert!(execute_task_child(&id, &authority, Some("original"), &mut port).is_err());
        assert!(port.calls.is_empty());
    }
    #[test]
    fn authority_revocation_during_binding_read_stops_execution() {
        let (id, authority, mut port) = fixture("worker-revoked", true);
        port.revoke_on_read = Some(authority.clone());
        assert!(execute_task_child(&id, &authority, None, &mut port).is_err());
        assert!(port.calls.is_empty());
    }
    #[test]
    fn only_core_terminal_reconciliation_can_report_settlement() {
        let (id, _, mut port) = fixture("worker-seal", true);
        assert!(!reconcile_task_child(&id, &mut port).unwrap());
        port.terminal = true;
        assert!(reconcile_task_child(&id, &mut port).unwrap());
        assert_eq!(port.calls, ["reconcile", "reconcile"]);
    }
}
