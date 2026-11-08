// SPDX-License-Identifier: Apache-2.0
//! CLI composition for the process-level committed Runtime Effect Supervisor.

use std::sync::{Arc, Mutex, OnceLock};

use nola_runtime_supervisor::{
    RuntimeEffectCancellation, RuntimeEffectSupervisor, RuntimeEffectSupervisorHandle,
    RuntimeEffectWorker, RuntimeEffectWorkerOutcome, RuntimeSupervisorConfig,
    RuntimeSupervisorSnapshot,
};

use crate::host_invocation_request::HostInvocationRequest;
use crate::core_agent_runtime::{
    drain_registered_core_agent_run, registered_core_agent_conversation_ids,
    resume_cold_core_agent_run, ColdCoreAgentRecoveryError, RegisteredCoreAgentDrain,
};
use crate::runtime_control::RuntimeExecutionControl;

static ACTIVE_SUPERVISOR: OnceLock<Mutex<Option<RuntimeEffectSupervisorHandle>>> = OnceLock::new();

#[derive(Default)]
struct CliRuntimeEffectWorker;

impl RuntimeEffectWorker for CliRuntimeEffectWorker {
    fn run(
        &self,
        work: &nola_protocol::RuntimeEffectWorkRunHead,
        cancellation: &RuntimeEffectCancellation,
    ) -> Result<RuntimeEffectWorkerOutcome, String> {
        let execution_cancellation = RuntimeExecutionControl::default();
        let execution_abort = execution_cancellation.clone();
        cancellation
            .on_cancel(Arc::new(move || execution_abort.request()))
            .map_err(|error| format!("bind Runtime effect cancellation failed: {error}"))?;
        let warm = drain_registered_core_agent_run(
            &work.conversation_id,
            &work.run_id,
            execution_cancellation.clone(),
            Some(&work.reason),
        )?;
        if !matches!(
            warm,
            RegisteredCoreAgentDrain::Unregistered | RegisteredCoreAgentDrain::StaleRun
        ) {
            return Ok(classify_drain_outcome(warm));
        }
        match resume_cold_core_agent_run(
            &work.conversation_id,
            &work.run_id,
            execution_cancellation,
        ) {
            Ok(outcome) => Ok(classify_drain_outcome(outcome)),
            Err(ColdCoreAgentRecoveryError::BlockedConfiguration { reason }) => {
                Ok(RuntimeEffectWorkerOutcome::blocked_configuration(reason))
            }
            Err(ColdCoreAgentRecoveryError::RetryableUnavailable { reason }) => {
                Ok(RuntimeEffectWorkerOutcome::retryable_unavailable(reason))
            }
        }
    }
}

fn classify_drain_outcome(outcome: RegisteredCoreAgentDrain) -> RuntimeEffectWorkerOutcome {
    match outcome {
        RegisteredCoreAgentDrain::Drained { .. } => RuntimeEffectWorkerOutcome::Drained,
        RegisteredCoreAgentDrain::Busy => {
            RuntimeEffectWorkerOutcome::retryable_unavailable("registered Core Agent Host is busy")
        }
        RegisteredCoreAgentDrain::Unregistered => {
            RuntimeEffectWorkerOutcome::retryable_unavailable(
                "registered Core Agent Host is unavailable",
            )
        }
        RegisteredCoreAgentDrain::StaleRun => RuntimeEffectWorkerOutcome::retryable_unavailable(
            "registered Core Agent Host does not own the scheduled Run",
        ),
    }
}

pub(crate) struct CliRuntimeEffectSupervisor {
    supervisor: Option<RuntimeEffectSupervisor>,
}

impl CliRuntimeEffectSupervisor {
    pub(crate) fn start_if_persistent(config: &HostInvocationRequest) -> Result<Option<Self>, String> {
        if !config.session.save_session {
            return Ok(None);
        }
        Self::start(config).map(Some)
    }

    pub(crate) fn start(config: &HostInvocationRequest) -> Result<Self, String> {
        let database = crate::platform_paths::application_database_path()?;
        let client = nola_service_client::ApplicationServiceClient::for_database(database);
        let supervisor = RuntimeEffectSupervisor::spawn(
            client,
            CliRuntimeEffectWorker,
            RuntimeSupervisorConfig::default(),
        )?;
        let handle = supervisor.handle();
        {
            let mut active = ACTIVE_SUPERVISOR
                .get_or_init(|| Mutex::new(None))
                .lock()
                .map_err(|_| "Runtime Effect Supervisor registry is poisoned".to_string())?;
            if active.is_some() {
                drop(active);
                let _ = supervisor.shutdown();
                return Err("Runtime Effect Supervisor is already active".to_string());
            }
            *active = Some(handle.clone());
        }
        if let Some(conversation_id) = config.session.session_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            if let Err(error) = register_conversation(conversation_id, config) {
                clear_active_supervisor();
                let _ = supervisor.shutdown();
                return Err(error);
            }
        }
        let conversations = match registered_core_agent_conversation_ids() {
            Ok(conversations) => conversations,
            Err(error) => {
                clear_active_supervisor();
                let _ = supervisor.shutdown();
                return Err(error);
            }
        };
        for conversation_id in conversations {
            if let Err(error) = handle.register_conversation(conversation_id) {
                clear_active_supervisor();
                let _ = supervisor.shutdown();
                return Err(error);
            }
        }
        let discovery = (|| -> Result<(), String> {
            let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
            let mut after_run_id = None;
            loop {
                let runs: Vec<nola_store_port::run_recovery_inspection::RecoverableRunRef> = application.execute(
                    nola_application::ApplicationCommand::ListRecoverableRuns { after_run_id: after_run_id.clone(), limit: 256 },
                )?;
                if runs.is_empty() { break; }
                after_run_id = runs.last().map(|run| run.run_id.clone());
                for run in runs { handle.register_conversation(run.conversation_id)?; }
            }
            Ok(())
        })();
        if let Err(error) = discovery {
            clear_active_supervisor();
            let _ = supervisor.shutdown();
            return Err(error);
        }
        Ok(Self {
            supervisor: Some(supervisor),
        })
    }

    pub(crate) fn shutdown(mut self) -> Result<RuntimeSupervisorSnapshot, String> {
        clear_active_supervisor();
        self.supervisor
            .take()
            .expect("CLI Runtime Effect Supervisor owns its worker")
            .shutdown()
    }

    /// A UI exit budget, not evidence that every external effect has stopped.
    /// The worker keeps the ordinary cancellation protocol and durable fences.
    pub(crate) fn shutdown_for_cli_exit(self) -> Result<RuntimeSupervisorSnapshot, String> {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        std::thread::Builder::new().name("cli-shutdown".into()).spawn(move || {
            let _ = sender.send(self.shutdown());
        }).map_err(|error| format!("start CLI shutdown: {error}"))?;
        receiver.recv_timeout(std::time::Duration::from_secs(2))
            .map_err(|_| "CLI shutdown was not confirmed within 2 seconds; pending operations remain recorded for recovery. No cancellation success was assumed.".to_string())?
    }
}

impl Drop for CliRuntimeEffectSupervisor {
    fn drop(&mut self) {
        clear_active_supervisor();
        if let Some(supervisor) = self.supervisor.take() {
            let _ = supervisor.shutdown();
        }
    }
}

pub(crate) fn register_conversation(conversation_id: &str, config: &HostInvocationRequest) -> Result<(), String> {
    if !config.session.save_session {
        return Ok(());
    }
    wake_committed_conversation(conversation_id).map(|_| ())
}

/// Wake existing committed work without inventing another invocation/config.
pub(crate) fn wake_committed_conversation(conversation_id: &str) -> Result<bool, String> {
    let conversation_id = conversation_id.trim();
    if conversation_id.is_empty() {
        return Err("cold Host configuration requires a conversation identity".to_string());
    }
    let handle = ACTIVE_SUPERVISOR.get().map(|registry| {
        registry.lock().map(|active| active.clone())
            .map_err(|_| "Runtime Effect Supervisor registry is poisoned".to_string())
    }).transpose()?.flatten();
    if let Some(handle) = handle {
        handle.register_conversation(conversation_id.to_string())?;
        return Ok(true);
    }
    Ok(false)
}

pub(crate) fn unregister_conversation(conversation_id: &str) {
    let Some(registry) = ACTIVE_SUPERVISOR.get() else {
        return;
    };
    let handle = registry.lock().ok().and_then(|active| active.clone());
    if let Some(handle) = handle {
        handle.unregister_conversation(conversation_id);
    }
}

fn clear_active_supervisor() {
    if let Some(registry) = ACTIVE_SUPERVISOR.get() {
        if let Ok(mut active) = registry.lock() {
            active.take();
        }
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use nola_protocol::{
        RuntimeEffectWorkReason, RuntimeEffectWorkRunHead, RuntimeEffectWorkSchedule,
        RuntimeEffectWorkWaitRequest,
    };
    use nola_runtime_supervisor::RuntimeEffectWorkWaitClient;
    use std::io::{BufRead, Read, Write};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    #[test]
    fn ephemeral_mode_does_not_open_a_supervisor_or_service_client() {
        let mut config = HostInvocationRequest::for_test().unwrap();
        config.session.save_session = false;
        assert!(CliRuntimeEffectSupervisor::start_if_persistent(&config)
            .unwrap()
            .is_none());
    }


    #[test]
    fn busy_and_stale_warm_hosts_are_retryable_instead_of_drained() {
        assert!(matches!(
            classify_drain_outcome(RegisteredCoreAgentDrain::Busy),
            RuntimeEffectWorkerOutcome::RetryableUnavailable { .. }
        ));
        assert!(matches!(
            classify_drain_outcome(RegisteredCoreAgentDrain::StaleRun),
            RuntimeEffectWorkerOutcome::RetryableUnavailable { .. }
        ));
        assert_eq!(
            classify_drain_outcome(RegisteredCoreAgentDrain::Drained { terminal: false }),
            RuntimeEffectWorkerOutcome::Drained
        );
    }

    struct OneScheduleClient(Mutex<Option<RuntimeEffectWorkSchedule>>);

    impl RuntimeEffectWorkWaitClient for OneScheduleClient {
        fn wait_for_runtime_effect_work(
            &self,
            _request: &RuntimeEffectWorkWaitRequest,
            cancellation: &nola_service_client::ApplicationServiceRequestCancellation,
        ) -> Result<RuntimeEffectWorkSchedule, String> {
            if let Some(schedule) = self.0.lock().unwrap().take() {
                return Ok(schedule);
            }
            while !cancellation.is_cancelled() {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(nola_service_client::APPLICATION_SERVICE_REQUEST_CANCELLED.into())
        }
    }

    struct ObservedCliWorker {
        inner: CliRuntimeEffectWorker,
        completed: mpsc::Sender<Result<RuntimeEffectWorkerOutcome, String>>,
    }

    impl RuntimeEffectWorker for ObservedCliWorker {
        fn run(
            &self,
            work: &nola_protocol::RuntimeEffectWorkRunHead,
            cancellation: &RuntimeEffectCancellation,
        ) -> Result<RuntimeEffectWorkerOutcome, String> {
            let result = self.inner.run(work, cancellation);
            self.completed.send(result.clone()).unwrap();
            result
        }
    }

    #[test]
    fn supervisor_drains_a_staged_warm_host_without_another_user_input() {
        let _environment_lock = crate::store::test_env_lock();
        struct RestoreDatabase(Option<std::ffi::OsString>);
        impl Drop for RestoreDatabase {
            fn drop(&mut self) {
                match &self.0 {
                    Some(value) => std::env::set_var("NOLA_DB_PATH", value),
                    None => std::env::remove_var("NOLA_DB_PATH"),
                }
            }
        }
        let root = std::env::temp_dir().join(format!(
            "nola-runtime-supervisor-cli-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let _database_environment = RestoreDatabase(std::env::var_os("NOLA_DB_PATH"));
        std::env::set_var("NOLA_DB_PATH", root.join("application.sqlite"));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let chat_posts = Arc::new(AtomicUsize::new(0));
        let server_posts = Arc::clone(&chat_posts);
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(8);
            loop {
                assert!(
                    Instant::now() < deadline,
                    "CLI worker did not reach provider"
                );
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut request_line = String::new();
                reader.read_line(&mut request_line).unwrap();
                let path = request_line.split_whitespace().nth(1).unwrap();
                let mut length = 0;
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).unwrap();
                    if header == "\r\n" {
                        break;
                    }
                    if let Some(value) = header.to_ascii_lowercase().strip_prefix("content-length:")
                    {
                        length = value.trim().parse::<usize>().unwrap();
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                if path.ends_with("/models") {
                    let body = serde_json::json!({
                        "data": [{"id":"fixture-model","context_length":32768}]
                    })
                    .to_string();
                    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                    continue;
                }
                assert!(
                    path.ends_with("/chat/completions"),
                    "unexpected path {path}"
                );
                server_posts.fetch_add(1, Ordering::SeqCst);
                let delta = serde_json::json!({
                    "choices":[{"index":0,"delta":{"content":"drained"},"finish_reason":null}]
                });
                let end = serde_json::json!({
                    "choices":[{"index":0,"delta":{},"finish_reason":"stop"}]
                });
                let body = format!("data: {delta}\n\ndata: {end}\n\ndata: [DONE]\n\n");
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                break;
            }
        });

        let session_id = format!("supervised-{}", uuid::Uuid::new_v4());
        let mut config = crate::config::Config::from_args(vec![
            "nola".into(),
            "--session".into(),
            session_id.clone(),
            "--project".into(),
            root.to_string_lossy().into_owned(),
            "--endpoint".into(),
            endpoint,
            "--api-key".into(),
            "fixture".into(),
            "--model".into(),
            "fixture-model".into(),
        ])
        .unwrap().host_invocation_request().unwrap();
        config.session.save_session = false;
        config.tool_settings.tools_enabled = false;
        config.tool_settings.skill_dirs = Some(Vec::new());
        assert!(!config.session.save_session);
        // This staging-only fixture bypasses the normal foreground admission
        // entry point, so admit the same immutable policies before BeginTurn.
        crate::host_memory_admission::freeze_host_memory_admission(&mut config).unwrap();
        config.runtime.output_admission =
            Some(crate::host_output_admission::admit(&config.model_invocation_settings().unwrap(), true).unwrap());
        config.runtime.admitted_max_retries = Some(crate::llm::configured_llm_max_retries() as u32);
        let (conversation_id, run_id) =
            crate::core_agent_runtime::stage_registered_core_agent_turn_without_dispatch(
                &config,
                "respond once",
            )
            .unwrap();
        let observed_at_ms = chrono::Utc::now().timestamp_millis();
        let schedule = RuntimeEffectWorkSchedule {
            observed_at_ms,
            runs: vec![RuntimeEffectWorkRunHead {
                conversation_id: conversation_id.clone(),
                run_id,
                effect_id: "scheduled-model-effect".into(),
                request_id: "scheduled-model-request".into(),
                attempt_id: None,
                reason: RuntimeEffectWorkReason::Pending,
                ready_at_ms: observed_at_ms,
                created_at_ms: observed_at_ms,
            }],
            next_wake_at_ms: None,
        };
        let (completed_tx, completed_rx) = mpsc::channel();
        let supervisor = RuntimeEffectSupervisor::spawn(
            OneScheduleClient(Mutex::new(Some(schedule))),
            ObservedCliWorker {
                inner: CliRuntimeEffectWorker,
                completed: completed_tx,
            },
            RuntimeSupervisorConfig {
                long_wait: Duration::from_millis(100),
                ..RuntimeSupervisorConfig::default()
            },
        )
        .unwrap();
        supervisor
            .handle()
            .register_conversation(conversation_id.clone())
            .unwrap();
        completed_rx
            .recv_timeout(Duration::from_secs(8))
            .expect("Supervisor did not finish CLI worker")
            .map(|outcome| assert_eq!(outcome, RuntimeEffectWorkerOutcome::Drained))
            .unwrap();
        supervisor.shutdown().unwrap();
        server.join().unwrap();
        assert_eq!(chat_posts.load(Ordering::SeqCst), 1);
        crate::core_agent_runtime::invalidate_core_agent_session(&conversation_id).unwrap();
        let _ = std::fs::remove_dir_all(root);
    }
}
