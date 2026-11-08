use nola_application::{AgentCommand, AgentCommandLease, ApplicationCommand};

use crate::application_port::CliApplicationPort;

#[cfg(test)]
#[path = "../../core/nola-application/tests/support/mod.rs"]
mod queue_source_support;

#[cfg(test)]
pub(crate) fn with_terminal_fixture(test: impl FnOnce(&std::path::Path, &str)) {
    let _guard=crate::store::test_env_lock();
    let root=tempfile::tempdir().unwrap();
    struct Restore(Vec<(&'static str,Option<std::ffi::OsString>)>);
    impl Drop for Restore {fn drop(&mut self) {for (key,value) in &self.0 {match value {Some(value)=>std::env::set_var(key,value),None=>std::env::remove_var(key)}}}}
    let _restore=Restore(["NOLA_DB_PATH","NOLA_HOME"].into_iter().map(|key|(key,std::env::var_os(key))).collect());
    let path=root.path().join("queue.sqlite");
    std::env::set_var("NOLA_DB_PATH",&path);std::env::set_var("NOLA_HOME",root.path());
    let mut app=queue_source_support::open_runtime(&path).unwrap();
    queue_source_support::agent_task_admission::draft(&mut app,None);
    let mut config=crate::config::Config::from_args(vec!["nola".into(),"--project".into(),
        root.path().to_string_lossy().into_owned(),"--endpoint".into(),"https://provider.invalid/v1".into(),
        "--api-key".into(),"fixture-key".into(),"--model".into(),"fixture-model".into()]).unwrap();
    config.session_id=Some("parent".into());
    let mut config = config.host_invocation_request().unwrap();
    let binding=crate::core_agent_runtime::fixture_host_invocation_binding(&mut config).unwrap();
    let profile = nola_core::agent::model_capability::resolve_model_capability_profile(
        Some("openai-compatible"), "fixture-model");
    let admission=queue_source_support::agent_task_admission::source_fact_for_with_turn_identity(
        &mut app,"task","parent","child","task-call",Some(7),
        nola_core::agent::AgentTurnIdentity {host_invocation:Some(binding),
            model_capability_id:Some(profile.profile_id.clone()), model_capability_revision:profile.revision,
            model_capability:profile,..Default::default()});
    app.execute(ApplicationCommand::ImportHostAgentTaskAdmission {admission}).unwrap();
    app.execute(ApplicationCommand::RuntimeTask {command:nola_application::RuntimeTaskCommand::RequestCancel {task_id:"task".into(),now_ms:700}}).unwrap();
    let receipt:Option<nola_application::RuntimeTaskTerminalReceipt>=serde_json::from_value(app.execute(ApplicationCommand::RuntimeTask {
        command:nola_application::RuntimeTaskCommand::TerminalReceipt {task_id:"task".into()}}).unwrap()).unwrap();
    let receipt=receipt.unwrap();
    app.execute(ApplicationCommand::EnqueueTaskTerminalReconciliation {task_id:"task".into(),expected_receipt_id:receipt.receipt_id.clone()}).unwrap();
    drop(app);
    test(root.path(),&receipt.receipt_id);
}

/// One queue schema and identity check for composer, execution boundary and
/// Task projection consumers. A decode failure never retains a claimed lease.
pub(crate) fn decode_claimed(command: &AgentCommandLease) -> Result<nola_core::agent::host::AgentBoundaryInput, String> {
    use nola_core::agent::host::AgentBoundaryInput;
    let result = (|| {
        let input: AgentBoundaryInput=serde_json::from_str(&command.payload_json)
            .map_err(|error|format!("invalid durable Agent envelope {}: {error}",command.command_id))?;
        let expected=match &input {
            AgentBoundaryInput::PermissionPolicyAvailable { policy } => {
                policy.validate()?;
                Some(policy.command_id())
            }
            AgentBoundaryInput::TaskTerminalAvailable {task_id,receipt_id} if !task_id.is_empty() && !receipt_id.is_empty()=>Some(format!("terminal:{receipt_id}")),
            AgentBoundaryInput::TaskGuidanceAvailable {task_id,message_id,settlement} if !task_id.is_empty() && !message_id.is_empty()=>
                Some(format!("task-guidance-{}:{task_id}:{message_id}",if *settlement {"result"}else{"admit"})),
            AgentBoundaryInput::Submission {..}=>None,
            _=>return Err("Task notification has empty identity".into()),
        };
        if expected.as_ref().is_some_and(|expected|expected!=&command.command_id) {return Err("Task notification command identity mismatch".into());}
        Ok(input)
    })();
    if result.is_err() {
        release_agent_command_lease(&command.session_id,&command.command_id,&command.lease_owner)?;
    }
    result
}

pub fn enqueue_agent_command(
    session_id: &str,
    command_id: &str,
    payload_json: &str,
    created_at_ms: i64,
) -> Result<bool, String> {
    let mut application = CliApplicationPort::persistent_default()?;
    application.execute(ApplicationCommand::EnqueueAgentCommand {
        input: AgentCommand {
            session_id: session_id.to_string(),
            command_id: command_id.to_string(),
            payload_json: payload_json.to_string(),
            created_at_ms,
        },
    })
}

pub fn claim_next_agent_command(
    session_id: &str,
    lease_owner: &str,
    now_ms: i64,
    lease_until_ms: i64,
) -> Result<Option<AgentCommandLease>, String> {
    if lease_owner.trim().is_empty() || lease_until_ms <= now_ms {
        return Err("Agent command claim requires a consumer and a future lease deadline".into());
    }
    let mut application = CliApplicationPort::persistent_default()?;
    let head: Option<nola_application::AgentCommandHead> = application.execute(ApplicationCommand::PeekAgentCommand {
        session_id: session_id.to_owned(),
    })?;
    let Some(head) = head else {
        // Empty waits are queries, not thousands of durable null receipts.
        return Ok(None);
    };
    if head.lease_until_ms.is_some_and(|deadline| deadline > now_ms) {
        let owner = head.lease_owner.ok_or("Agent command head has no lease owner")?;
        if !dead_local_consumer(&owner) {
            return Ok(None);
        }
        let released: bool = application.execute(ApplicationCommand::ReleaseAgentCommandLease {
            session_id: session_id.to_owned(),
            command_id: head.command_id,
            lease_owner: owner,
        })?;
        if !released { return Ok(None); }
    }
    // Peeking grants no authority: the transaction still arbitrates competing
    // consumers. A lost race may yield null; empty polling never gets here.
    application.execute(ApplicationCommand::ClaimNextAgentCommand {
        session_id: session_id.to_owned(),
        lease_owner: lease_owner.to_owned(),
        now_ms,
        lease_until_ms,
    })
}

pub fn acknowledge_agent_command(
    session_id: &str,
    command_id: &str,
    lease_owner: &str,
) -> Result<bool, String> {
    let mut application = CliApplicationPort::persistent_default()?;
    application.execute(ApplicationCommand::AcknowledgeAgentCommand {
        session_id: session_id.to_string(),
        command_id: command_id.to_string(),
        lease_owner: lease_owner.to_string(),
    })
}

pub fn release_agent_command_lease(
    session_id: &str,
    command_id: &str,
    lease_owner: &str,
) -> Result<bool, String> {
    let mut application = CliApplicationPort::persistent_default()?;
    application.execute(ApplicationCommand::ReleaseAgentCommandLease {
        session_id: session_id.to_string(),
        command_id: command_id.to_string(),
        lease_owner: lease_owner.to_string(),
    })
}

fn dead_local_consumer(lease_owner: &str) -> bool {
    #[cfg(unix)]
    {
        let mut fields = lease_owner.split(':');
        let kind = fields.next().unwrap_or_default();
        if kind != "boundary-consumer"
            && kind != "agent-command-consumer"
            && kind != "composer-consumer"
        {
            return false;
        }
        let Some(pid) = fields.next().and_then(|value| value.parse::<i32>().ok()) else {
            return false;
        };
        if pid <= 0 {
            return false;
        }
        let result = unsafe { libc::kill(pid, 0) };
        if result == 0 {
            return false;
        }
        return std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH);
    }
    #[cfg(not(unix))]
    {
        let _ = lease_owner;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::dead_local_consumer;

    #[test]
    fn dead_local_consumer_rejects_foreign_and_live_owners() {
        assert!(!dead_local_consumer("remote-host:999999:lease"));
        assert!(!dead_local_consumer(&format!(
            "boundary-consumer:{}:lease",
            std::process::id()
        )));
    }

    #[cfg(unix)]
    #[test]
    fn dead_local_consumer_recognizes_a_missing_local_pid() {
        assert!(dead_local_consumer(
            "agent-command-consumer:2147483647:lease"
        ));
    }
}
