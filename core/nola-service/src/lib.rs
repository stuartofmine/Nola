// SPDX-License-Identifier: Apache-2.0
//! Local single-writer process boundary for Nola product state.

#[derive(Debug)]
pub struct ServiceFailure {
    pub code: &'static str,
    pub detail: String,
}

impl std::fmt::Display for ServiceFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl From<String> for ServiceFailure {
    fn from(detail: String) -> Self {
        Self {
            code: "service_startup_failed",
            detail,
        }
    }
}

#[cfg(unix)]
mod projection_wakeup;

#[cfg(unix)]
mod transport_deadline;

#[cfg(unix)]
mod connection_budget;

#[cfg(unix)]
mod unix {
    use super::projection_wakeup::{ProjectionWakeReceiver, ProjectionWakeSender};
    use super::ServiceFailure;
    use std::fs;
    use std::io::{BufRead, BufReader, BufWriter, Read, Write};
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex, Weak};
    use std::thread;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use nola_application::{
        ApplicationCommand, ApplicationCommandReceipt, ApplicationProjectionScope,
        ExecuteReceivedCommand, LoadRuntimeEffectWorkSchedule,
    };
    use nola_application_bootstrap::ApplicationRuntime;
    use nola_application_bootstrap::{ApplicationOpenFailureKind, ApplicationPersistencePolicy};
    use nola_protocol::{
        CommandEnvelope, CommandReceipt, CommandReceiptLookupRequest, CommandReceiptOutcome,
        EventSuffix, EventSuffixRequest, ProjectionCursor, ProjectionEvent, ProjectionPage,
        ProjectionRecord, ProjectionScope, ProjectionSnapshot, ProjectionSnapshotRequest,
        ProtocolFailure, RevisionConflict, RuntimeEffectWorkSchedule,
        APPLICATION_SERVICE_MAX_FRAME_BYTES, APPLICATION_SERVICE_PROTOCOL_VERSION,
    };
    use serde_json::{json, Map, Value};

    struct SocketGuard(PathBuf);

    fn acquire_database_service_lock(database: &Path) -> Result<fs::File, String> {
        let parent = database
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)
            .map_err(|error| format!("create database directory failed: {error}"))?;
        let canonical = if database.exists() {
            fs::canonicalize(database)
        } else {
            fs::canonicalize(parent)
                .map(|parent| parent.join(database.file_name().unwrap_or_default()))
        }
        .map_err(|error| format!("resolve database service lock identity failed: {error}"))?;
        let mut lock_path = canonical.into_os_string();
        lock_path.push(".service.lock");
        // Keep the lock inode after release. Unlinking it can let a new process
        // lock a different inode while a waiter still owns the original file.
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(PathBuf::from(lock_path))
            .map_err(|error| format!("open database service lock failed: {error}"))?;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(format!(
                "cannot acquire exclusive Application Service database ownership: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(file)
    }

    struct SharedApplication {
        runtime: Mutex<Option<ApplicationRuntime>>,
        database_path: PathBuf,
        projection_waiters: Mutex<Vec<Weak<ProjectionWakeSender>>>,
    }

    type ApplicationRuntimeGuard<'a> = nola_application_bootstrap::RuntimeOwnerGuard<'a>;

    impl SharedApplication {
        fn lock(&self) -> Result<ApplicationRuntimeGuard<'_>, String> {
            nola_application_bootstrap::acquire_owned_runtime(&self.runtime, &self.database_path)
        }

        fn wait_for_projection<T>(
            &self,
            connection: &UnixStream,
            deadline: Option<Instant>,
            load: impl Fn(&ApplicationRuntime) -> Result<T, String>,
            has_events: impl Fn(&T) -> bool,
        ) -> Result<T, String> {
            let mut runtime = self
                .lock()
                .map_err(|_| "application runtime lock is poisoned".to_string())?;
            let mut notification: Option<(Arc<ProjectionWakeSender>, ProjectionWakeReceiver)> =
                None;
            loop {
                let loaded = load(&runtime)?;
                let Some(deadline) = deadline else {
                    return Ok(loaded);
                };
                if has_events(&loaded) || Instant::now() >= deadline {
                    return Ok(loaded);
                }
                if notification.is_none() {
                    let mut waiters = self
                        .projection_waiters
                        .lock()
                        .map_err(|_| "projection waiter registry is poisoned".to_string())?;
                    waiters.retain(|waiter| waiter.strong_count() > 0);
                    if waiters.len() >= 128 {
                        return Err("projection waiter limit reached".into());
                    }
                    let (sender, receiver) =
                        ProjectionWakeReceiver::channel().map_err(|error| {
                            format!("create projection notification failed: {error}")
                        })?;
                    waiters.push(Arc::downgrade(&sender));
                    notification = Some((sender, receiver));
                }
                // Registration occurs under the writer mutex. A commit
                // after unlock queues a byte even before poll starts.
                drop(runtime);
                if let Some((_, receiver)) = notification.as_mut() {
                    receiver.wait(connection, deadline)?;
                }
                runtime = self
                    .lock()
                    .map_err(|_| "application runtime lock is poisoned".to_string())?;
            }
        }

        fn wait_for_runtime_effect_work(
            &self,
            connection: &UnixStream,
            deadline: Instant,
            conversation_ids: &[String],
            limit: usize,
        ) -> Result<RuntimeEffectWorkSchedule, String> {
            self.wait_for_runtime_effect_work_with(
                connection,
                deadline,
                limit,
                |runtime, now_ms, limit| {
                    let value =
                        runtime.execute(ApplicationCommand::LoadRuntimeEffectWorkSchedule {
                            input: LoadRuntimeEffectWorkSchedule {
                                conversation_ids: conversation_ids.to_vec(),
                                now_ms,
                                limit,
                            },
                        })?;
                    serde_json::from_value(value).map_err(|error| {
                        format!("decode Runtime effect-work schedule failed: {error}")
                    })
                },
            )
        }

        fn wait_for_runtime_effect_work_with(
            &self,
            connection: &UnixStream,
            deadline: Instant,
            limit: usize,
            mut load: impl FnMut(
                &mut ApplicationRuntime,
                i64,
                usize,
            ) -> Result<RuntimeEffectWorkSchedule, String>,
        ) -> Result<RuntimeEffectWorkSchedule, String> {
            let mut runtime = self
                .lock()
                .map_err(|_| "application runtime lock is poisoned".to_string())?;
            let mut notification: Option<(Arc<ProjectionWakeSender>, ProjectionWakeReceiver)> =
                None;
            loop {
                let now_ms = epoch_now_ms()?;
                let loaded = load(&mut runtime, now_ms, limit)?;
                let now = Instant::now();
                if !loaded.runs.is_empty() || now >= deadline {
                    return Ok(loaded);
                }
                let wake_deadline = match loaded.next_wake_at_ms {
                    Some(next_wake_at_ms) if next_wake_at_ms <= now_ms => {
                        return Err("Runtime effect-work schedule returned a due deadline without recoverable work".into());
                    }
                    Some(next_wake_at_ms) => {
                        let delay_ms = u64::try_from(next_wake_at_ms - now_ms).unwrap_or(u64::MAX);
                        deadline.min(now + Duration::from_millis(delay_ms))
                    }
                    None => deadline,
                };
                if notification.is_none() {
                    let mut waiters = self
                        .projection_waiters
                        .lock()
                        .map_err(|_| "projection waiter registry is poisoned".to_string())?;
                    waiters.retain(|waiter| waiter.strong_count() > 0);
                    if waiters.len() >= 128 {
                        return Err("projection waiter limit reached".into());
                    }
                    let (sender, receiver) =
                        ProjectionWakeReceiver::channel().map_err(|error| {
                            format!("create Runtime work notification failed: {error}")
                        })?;
                    waiters.push(Arc::downgrade(&sender));
                    notification = Some((sender, receiver));
                }
                drop(runtime);
                if let Some((_, receiver)) = notification.as_mut() {
                    receiver.wait(connection, wake_deadline)?;
                }
                runtime = self
                    .lock()
                    .map_err(|_| "application runtime lock is poisoned".to_string())?;
            }
        }

        fn waiter_count(&self) -> usize {
            self.projection_waiters
                .lock()
                .map(|waiters| {
                    waiters
                        .iter()
                        .filter(|waiter| waiter.strong_count() > 0)
                        .count()
                })
                .unwrap_or(0)
        }

        fn notify_projection_waiters(&self) {
            if let Ok(mut waiters) = self.projection_waiters.lock() {
                waiters.retain(|waiter| {
                    if let Some(waiter) = waiter.upgrade() {
                        waiter.notify();
                        true
                    } else {
                        false
                    }
                });
            }
        }
    }

    impl Drop for SocketGuard {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    pub fn serve(database_path: &Path, socket_path: &Path) -> Result<(), String> {
        serve_with_options(database_path, socket_path, false)
    }

    pub fn serve_with_options(
        database_path: &Path,
        socket_path: &Path,
        recreate_incompatible_store: bool,
    ) -> Result<(), String> {
        serve_with_options_and_parent(
            database_path,
            socket_path,
            recreate_incompatible_store,
            None,
        )
    }

    pub fn serve_with_options_and_parent(
        database_path: &Path,
        socket_path: &Path,
        recreate_incompatible_store: bool,
        parent_pid: Option<u32>,
    ) -> Result<(), String> {
        serve_with_recovery_and_parent(
            database_path,
            socket_path,
            recreate_incompatible_store,
            None,
            parent_pid,
        )
        .map_err(|error| error.detail)
    }

    pub fn serve_with_recovery_and_parent(
        database_path: &Path,
        socket_path: &Path,
        recreate_incompatible_store: bool,
        archive_incompatible_store: Option<&Path>,
        parent_pid: Option<u32>,
    ) -> Result<(), ServiceFailure> {
        let _database_service_lock = acquire_database_service_lock(database_path)?;
        if let Some(parent) = socket_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("create service socket directory failed: {error}"))?;
        }
        if socket_path.exists() {
            if UnixStream::connect(socket_path).is_ok() {
                return Err(format!(
                    "another Nola Application Service owns {}",
                    socket_path.display()
                )
                .into());
            }
            fs::remove_file(socket_path)
                .map_err(|error| format!("remove stale service socket failed: {error}"))?;
        }

        let listener = UnixListener::bind(socket_path)
            .map_err(|error| format!("bind application service socket failed: {error}"))?;
        if parent_pid.is_some() {
            listener.set_nonblocking(true).map_err(|error| {
                format!("configure application service listener failed: {error}")
            })?;
        }
        fs::set_permissions(socket_path, fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("restrict application service socket failed: {error}"))?;
        let _socket_guard = SocketGuard(socket_path.to_path_buf());
        let policy = if let Some(archive_directory) = archive_incompatible_store {
            ApplicationPersistencePolicy::ArchiveAndRecreateIncompatible {
                archive_directory: archive_directory.to_path_buf(),
            }
        } else if recreate_incompatible_store {
            ApplicationPersistencePolicy::RecreateIncompatible
        } else {
            ApplicationPersistencePolicy::RequireCurrent
        };
        // Connections are independent transport sessions. Keep the durable
        // Application runtime behind one mutex so Execute requests remain a
        // strict single-writer stream, while Health and request decoding do
        // not queue behind a long transaction or a client that sent only a
        // partial frame.
        let application = Arc::new(SharedApplication {
            runtime: Mutex::new(Some(
                nola_application_bootstrap::open_with_policy_diagnosed(
                    database_path,
                    policy,
                )
                .map_err(|error| ServiceFailure {
                    code: match error.kind {
                        ApplicationOpenFailureKind::IncompatibleStore => {
                            "incompatible_canonical_store"
                        }
                        ApplicationOpenFailureKind::Other => "service_startup_failed",
                    },
                    detail: error.detail,
                })?,
            )),
            database_path: database_path.to_path_buf(),
            projection_waiters: Mutex::new(Vec::new()),
        });
        let connections = super::connection_budget::ConnectionBudget::for_process();
        loop {
            if parent_pid.is_some_and(|pid| !parent_process_is_alive(pid)) {
                return Ok(());
            }
            if parent_pid.is_some() && !listener_is_ready(&listener)? {
                continue;
            }
            match listener.accept() {
                // A readiness probe or a client crash may close one socket at
                // any byte boundary. That is a connection failure, not a
                // reason to release the device's single-writer service.
                Ok((stream, _)) => {
                    let Some(permit) = connections.try_acquire() else {
                        // Reject before reading or executing any command. Keep
                        // accepting so overload cannot grow the worker registry.
                        drop(stream);
                        continue;
                    };
                    // On macOS an accepted socket may inherit O_NONBLOCK from
                    // the listener. The listener is nonblocking only so a
                    // parent-bound service can periodically observe parent
                    // death; request frames themselves must be read to their
                    // newline boundary. Without this reset, payloads larger
                    // than the socket's immediately available bytes are
                    // mistaken for disconnected clients and produce EPIPE.
                    stream.set_nonblocking(false).map_err(|error| {
                        format!("configure application service client stream failed: {error}")
                    })?;
                    let application = Arc::clone(&application);
                    let _ = thread::Builder::new()
                        .name("nola-service-connection".into())
                        // Core restores complete frozen Run/delegation snapshots here.
                        // Budget the worker stack explicitly, including unoptimized Hosts.
                        .stack_size(16 * 1024 * 1024)
                        .spawn(move || {
                            let _permit = permit;
                            let _ = handle_connection(stream, &application);
                        });
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => continue,
                Err(error)
                    if matches!(
                        error.raw_os_error(),
                        Some(libc::EMFILE) | Some(libc::ENFILE)
                    ) =>
                {
                    // Existing connection deadlines can release descriptors.
                    // Resource pressure must not tear down the canonical writer.
                    thread::sleep(Duration::from_millis(25));
                    continue;
                }
                Err(error) => {
                    return Err(format!("accept application service client failed: {error}").into())
                }
            }
        }
    }

    fn process_is_alive(pid: u32) -> bool {
        let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
        result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }

    fn parent_process_is_alive(expected_pid: u32) -> bool {
        let current_parent = unsafe { libc::getppid() };
        current_parent > 0
            && current_parent as u32 == expected_pid
            && process_is_alive(expected_pid)
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn absent_owner_cannot_turn_an_original_mutation_into_a_rejection() {
            let directory = tempfile::tempdir().unwrap();
            let shared = super::SharedApplication {
                runtime: super::Mutex::new(None),
                // Opening this directory as an SQLite file actually fails.
                database_path: directory.path().to_owned(),
                projection_waiters: super::Mutex::new(Vec::new()),
            };
            let (connection, _peer) = super::UnixStream::pair().unwrap();
            let envelope = serde_json::json!({
                "protocol_version": nola_protocol::APPLICATION_SERVICE_PROTOCOL_VERSION,
                "client_id":"original-owner", "request_id":"original-intent",
                "command":{"type":"put_product_setting","arguments":{
                    "key":"test.owner-unavailable","value_json":"true","updated_at_ms":1
                }}
            });
            let receipt = super::handle_frame(&serde_json::to_vec(&envelope).unwrap(), &shared, &connection);
            assert_eq!(receipt.client_id, "original-owner");
            assert_eq!(receipt.request_id, "original-intent");
            assert!(matches!(receipt.outcome, nola_protocol::CommandReceiptOutcome::Accepted));
            assert!(shared.runtime.lock().unwrap().is_none());
        }

        use super::parent_process_is_alive;

        #[test]
        fn parent_liveness_requires_the_original_parent_identity() {
            let parent = unsafe { libc::getppid() } as u32;
            assert!(parent_process_is_alive(parent));
            assert!(!parent_process_is_alive(parent.saturating_add(1)));
        }

        #[test]
        fn task_cancellation_queued_past_deadline_uses_locked_execution_time() {
            use nola_protocol::{CommandEnvelope, CommandPayload, CommandReceiptOutcome};
            use nola_domain::agent_task_admission::{AgentTaskAdmission, AgentTaskCancellationAuthorization, AGENT_TASK_ADMISSION_VERSION};
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("queued-control.db");
            let shared = super::SharedApplication {
                runtime: super::Mutex::new(Some(nola_application_bootstrap::open(&path).unwrap())),
                database_path:path, projection_waiters:super::Mutex::new(Vec::new()),
            };
            // Synthetic valid metadata tests only the service queue-time gate;
            // successful source export and destination ownership have separate
            // real SQLite coverage in task_cancellation_transfer_e2e.
            let admission:AgentTaskAdmission = serde_json::from_value(serde_json::json!({
                "protocol_version":AGENT_TASK_ADMISSION_VERSION,"task_id":"task","parent_conversation_id":"parent",
                "parent_core_session_id":"core","parent_run_id":"run","parent_turn_id":"turn",
                "child_conversation_id":"child","parent_tool_call_id":"call","parent_effect_id":"effect",
                "input_watermark":1,"source_store_epoch":"source","source_transition_id":"transition",
                "source_host_event_id":"event","source_checkpoint_revision":1,"core_revision":1,
                "core_event_cursor":1,"admission_digest":""
            })).unwrap();
            let now=super::epoch_now_ms().unwrap();
            let deadline=now+500;
            let attempt:nola_domain::EffectDeliveryAttempt = serde_json::from_value(serde_json::json!({
                "id":"attempt","run_id":"run","effect_id":"cancel-effect","request_id":"request",
                "predecessor_id":null,"attempt_number":1,"replay_safety":"idempotent","state":"execution_started",
                "worker_generation":"generation","lease_owner":"owner","lease_token":"token","lease_expires_at_ms":deadline,
                "settlement":null,"recovery_plan":null,"manual_resolution":null,"interruption_reason":null,
                "created_at_ms":now-2,"claimed_at_ms":now-1,"execution_started_at_ms":now,
                "heartbeat_at_ms":now,"finished_at_ms":null,"revision":2
            })).unwrap();
            let authorization=AgentTaskCancellationAuthorization { admission:admission.seal().unwrap(),
                source_host_id:"host".into(),control_attempt:attempt,deadline_ms:deadline };
            authorization.validate().unwrap();
            let envelope=CommandEnvelope {protocol_version:nola_protocol::APPLICATION_SERVICE_PROTOCOL_VERSION,
                request_id:"queued-cancel".into(),client_id:"trusted-host".into(),expected_revision:None,
                command:CommandPayload {command_type:"host_import_task_cancellation".into(),
                    arguments:serde_json::json!({"authorization":authorization})}};
            let frame=serde_json::to_vec(&envelope).unwrap();
            let lock=shared.lock().unwrap();
            std::thread::scope(|scope| {
                let (started, ready)=std::sync::mpsc::channel();
                let shared = &shared;
                let frame = &frame;
                let worker=scope.spawn(move || {
                    let (connection,_peer)=super::UnixStream::pair().unwrap();
                    started.send(()).unwrap();
                    super::handle_frame(&frame,&shared,&connection)
                });
                ready.recv().unwrap();
                std::thread::sleep(std::time::Duration::from_millis(650));
                drop(lock);
                let receipt=worker.join().unwrap();
                let CommandReceiptOutcome::Rejected {error,..}=receipt.outcome else {panic!("expired queued control executed")};
                assert!(error.message.contains("outside its exact delivery lifetime"),"{}",error.message);
            });
        }

        #[test]
        fn task_admission_is_not_an_ordinary_application_json_command() {
            use nola_protocol::{CommandEnvelope, CommandPayload, CommandReceiptOutcome};
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("admission-boundary.db");
            let runtime = nola_application_bootstrap::open(&path).unwrap();
            let shared = super::SharedApplication {
                runtime: super::Mutex::new(Some(runtime)),
                database_path: path.clone(),
                projection_waiters: super::Mutex::new(Vec::new()),
            };
            let (connection, _peer) = super::UnixStream::pair().unwrap();
            // Syntactically valid metadata is not proof of a real source commit.
            // This test checks transport rejection, not successful admission.
            let admission: nola_protocol::AgentTaskAdmission = serde_json::from_value(serde_json::json!({
                "protocol_version":nola_domain::agent_task_admission::AGENT_TASK_ADMISSION_VERSION,"task_id":"task","parent_conversation_id":"parent",
                "parent_core_session_id":"core","parent_run_id":"run","parent_turn_id":"turn",
                "child_conversation_id":"child","parent_tool_call_id":"call","parent_effect_id":"effect",
                "input_watermark":1,"source_store_epoch":"source","source_transition_id":"transition",
                "source_host_event_id":"event","source_checkpoint_revision":1,"core_revision":1,
                "core_event_cursor":1,"admission_digest":""
            })).unwrap();
            let admission = admission.seal().unwrap();
            // A valid administrative cancellation has no admitted child Run
            // result; transport rejection must not depend on an invalid receipt.
            let terminal = serde_json::json!({
                "protocol_version":2,"admission":admission,"authority_revision":2,
                "receipt_id":"receipt","status":"cancelled","input_watermark":1,
                "completion_watermark":1,"result_ref":null,"run_result":null,
                "result_digest":null,"ended_at_ms":2,"guidance_results":[],
            });
            serde_json::from_value::<nola_protocol::AgentTaskTerminalProjection>(terminal.clone()).unwrap().validate().unwrap();
            let guidance = serde_json::json!({
                "admission":admission,"authority_revision":2,"settlement":false,
                "guidance":{"task_id":"task","message_id":"message","parent_session_id":"core",
                    "parent_turn_id":"turn","input_watermark":1,"content":"exact input",
                    "submitted_at_ms":1,"status":"accepted","reason":null},
            });
            serde_json::from_value::<nola_protocol::AgentTaskGuidanceProjection>(guidance.clone()).unwrap().validate().unwrap();
            for (index, command_type, expected_revision, arguments, expected_code) in [
                (0, "import_host_agent_task_admission", None, serde_json::json!({"admission":admission}), "invalid_command"),
                (1, "host_import_agent_task_admission", Some(1), serde_json::json!({"admission":admission}), "unsupported_revision_precondition"),
                (2, "host_import_agent_task_admission", None, serde_json::json!({"admission":admission,"force":true}), "invalid_command"),
                (3, "import_host_agent_task_terminal_projection", None, serde_json::json!({"projection":terminal}), "invalid_command"),
                (4, "host_import_agent_task_terminal_projection", Some(1), serde_json::json!({"projection":terminal}), "unsupported_revision_precondition"),
                (5, "host_import_agent_task_terminal_projection", None, serde_json::json!({"projection":terminal,"force":true}), "invalid_command"),
                (6, "import_host_agent_task_guidance_projection", None, serde_json::json!({"projection":guidance}), "invalid_command"),
                (7, "host_import_agent_task_guidance_projection", Some(1), serde_json::json!({"projection":guidance}), "unsupported_revision_precondition"),
                (8, "host_import_agent_task_guidance_projection", None, serde_json::json!({"projection":guidance,"force":true}), "invalid_command"),
                (9,"load_edited_resend_host_material",None,serde_json::json!({"conversation_id":"target"}),"invalid_command"),
                (10,"host_load_edited_resend_material",None,serde_json::json!({"conversation_id":"target","handle":"injected"}),"invalid_command"),
                (11,"host_load_edited_resend_material",Some(1),serde_json::json!({"conversation_id":"target"}),"unsupported_revision_precondition"),
                (12,"attest_edited_resend_host_policy",None,serde_json::json!({"conversation_id":"target","core_config":{}}),"invalid_command"),
                (13,"host_attest_edited_resend_policy",None,serde_json::json!({"conversation_id":"target","core_config":{},"force":true}),"invalid_command"),
                (14,"host_attest_edited_resend_policy",Some(1),serde_json::json!({"conversation_id":"target","core_config":{}}),"unsupported_revision_precondition"),
            ] {
                let envelope = CommandEnvelope {
                    protocol_version: nola_protocol::APPLICATION_SERVICE_PROTOCOL_VERSION,
                    request_id: format!("rejected-admission-{index}"),
                    client_id: "boundary-client".into(),
                    expected_revision,
                    command: CommandPayload { command_type: command_type.into(), arguments },
                };
                let receipt = super::handle_frame(&serde_json::to_vec(&envelope).unwrap(), &shared, &connection);
                assert_eq!(receipt.request_id, envelope.request_id);
                assert_eq!(receipt.client_id, envelope.client_id);
                let CommandReceiptOutcome::Rejected { error, .. } = receipt.outcome else {
                    panic!("invalid admission crossed the Application write boundary");
                };
                assert_eq!(error.code, expected_code);
            }
            let database = rusqlite::Connection::open(path).unwrap();
            let receipts: i64 = database.query_row("SELECT count(*) FROM command_receipts", [], |row| row.get(0)).unwrap();
            let tasks: i64 = database.query_row("SELECT count(*) FROM durable_tasks", [], |row| row.get(0)).unwrap();
            assert_eq!((receipts, tasks), (0, 0));
        }

        #[test]
        fn commit_uncertainty_reopens_the_same_owner_before_receipt_reconciliation() {
            use nola_protocol::{CommandEnvelope, CommandPayload, CommandReceiptOutcome};

            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("unknown-commit.db");
            let mut runtime = nola_application_bootstrap::open(&path).unwrap();
            runtime
                .execute(nola_application::ApplicationCommand::CreateConversation {
                    input: nola_application::CreateConversation {
                        id: "conversation".into(),
                        title: "Before".into(),
                        settings_json: None,
                        created_at_ms: 1,
                        updated_at_ms: 1,
                    },
                })
                .unwrap();
            runtime
                .execute(nola_application::ApplicationCommand::RuntimeTask {
                    command: nola_application::RuntimeTaskCommand::Create {
                        input: nola_application::CreateRuntimeTask {
                            id: "uncertain-task".into(),
                            kind: "shell_process".into(),
                            title: "Receipt reconciliation".into(),
                            project_root: "/fixture".into(),
                            owner_session_id: Some("conversation".into()),
                            parent_task_id: None,
                            parent_tool_call_id: None,
                            child_session_id: None,
                            agent_type: None,
                            prompt: None,
                            initial_progress_summary: None,
                            initial_activity: None,
                            output_path: None,
                            initially_unread: false,
                            input_watermark: None,
                            metadata: serde_json::json!({}),
                            scheduling: nola_application::RuntimeTaskScheduling {
                                priority: Default::default(),
                                queued_at_ms: 1,
                                resource_budget: Default::default(),
                                output_bytes: 0,
                                resource_usage: Default::default(),
                                budget_exhaustion: None,
                            },
                            created_at_ms: 1,
                        },
                    },
                })
                .unwrap();
            let attempt: Option<nola_application::RuntimeTaskAttempt> = serde_json::from_value(
                runtime
                    .execute(nola_application::ApplicationCommand::RuntimeTask {
                        command: nola_application::RuntimeTaskCommand::Claim {
                            task_id: "uncertain-task".into(),
                            worker_id: "original-worker".into(),
                            worker_generation: "original-generation".into(),
                            lease_token: "original-token".into(),
                            now_ms: 2,
                            lease_until_ms: 60_000,
                        },
                    })
                    .unwrap(),
            )
            .unwrap();
            let attempt = attempt.unwrap();
            let shared = super::SharedApplication {
                runtime: super::Mutex::new(Some(runtime)),
                database_path: path.clone(),
                projection_waiters: super::Mutex::new(Vec::new()),
            };
            let fixture = rusqlite::Connection::open(&path).unwrap();
            fixture
                .execute_batch(
                    "CREATE TABLE service_commit_parent(id INTEGER PRIMARY KEY);
                 CREATE TABLE service_commit_child(
                     parent_id INTEGER REFERENCES service_commit_parent(id)
                         DEFERRABLE INITIALLY DEFERRED
                 );
                 CREATE TRIGGER fail_service_commit
                 AFTER UPDATE OF status ON command_receipts
                 WHEN NEW.status='committed'
                 BEGIN
                     INSERT INTO service_commit_child(parent_id) VALUES (1);
                 END;",
                )
                .unwrap();
            let (connection, _peer) = super::UnixStream::pair().unwrap();
            let execute = |request_id: &str, command_type: &str, arguments| {
                let envelope = CommandEnvelope {
                    protocol_version: nola_protocol::APPLICATION_SERVICE_PROTOCOL_VERSION,
                    request_id: request_id.into(),
                    client_id: "original-client".into(),
                    expected_revision: None,
                    command: CommandPayload {
                        command_type: command_type.into(),
                        arguments,
                    },
                };
                super::handle_frame(
                    &serde_json::to_vec(&envelope).unwrap(),
                    &shared,
                    &connection,
                )
            };
            let unknown = execute(
                "original-request",
                "rename_conversation",
                serde_json::json!({
                    "conversation_id": "conversation", "title": "Uncertain", "updated_at_ms": 2,
                }),
            );
            assert_eq!(unknown.client_id, "original-client");
            assert_eq!(unknown.request_id, "original-request");
            let CommandReceiptOutcome::Rejected { error, .. } = unknown.outcome else {
                panic!("expected transaction uncertainty")
            };
            assert_eq!(error.code, "command_transaction_commit_outcome_unknown");
            assert!(!error.recoverable);
            // The receipt fault is still active. Recovery control must be
            // able to fence this Task without completing another receipt.
            let suspension = execute(
                "suspend-original-task",
                "runtime_task",
                serde_json::json!({
                    "command": nola_application::RuntimeTaskCommand::SuspendForCommandReconciliation {
                        input: nola_application::SuspendRuntimeTaskForCommandReconciliation {
                            lease: nola_application::RuntimeTaskLeaseWitness::from(&attempt),
                            client_id: "original-client".into(), request_id: "original-request".into(),
                            failure_code: "command_transaction_commit_outcome_unknown".into(),
                            progress_summary: Some("Awaiting the original receipt".into()), now_ms: 3,
                        },
                    },
                }),
            );
            let CommandReceiptOutcome::Committed { result, .. } = suspension.outcome else {
                panic!("recovery control required another receipt")
            };
            assert_eq!(result["request_id"], "original-request");
            let status: String = fixture
                .query_row(
                    "SELECT status FROM durable_tasks WHERE id='uncertain-task'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(status, "awaiting_reconciliation");
            let pending = execute(
                "inspect-original-task",
                "runtime_task",
                serde_json::json!({
                    "command": nola_application::RuntimeTaskCommand::PendingCommandReconciliation { task_id: "uncertain-task".into() },
                }),
            );
            let CommandReceiptOutcome::Committed { result, .. } = pending.outcome else {
                panic!("pending reconciliation read required a write receipt")
            };
            assert_eq!(result["client_id"], "original-client");
            assert_eq!(result["request_id"], "original-request");
            // Recovery polling remains usable even while every receipt COMMIT
            // is faulted. No recovery query may create a replacement write receipt.
            for command in [
                nola_application::RuntimeTaskCommand::PendingPermissionSuspension {
                    task_id: "uncertain-task".into(),
                },
                nola_application::RuntimeTaskCommand::LatestPermissionSuspension {
                    task_id: "uncertain-task".into(),
                },
                nola_application::RuntimeTaskCommand::Attempt {
                    attempt_id: "missing-attempt".into(),
                },
                nola_application::RuntimeTaskCommand::PermissionResume {
                    task_id: "uncertain-task".into(), suspension_id: "not-resumed".into(),
                    permission_request_id: "permission".into(), continuation_id: "continuation".into(),
                },
                nola_application::RuntimeTaskCommand::PermissionContinuationGrant {
                    task_id: "uncertain-task".into(), suspension_id: "not-resumed".into(),
                    selector: nola_application::PermissionContinuationGrantSelector::ExactAttempt {
                        attempt_id: "missing-attempt".into(),
                    },
                },
            ] {
                let reply = execute("permission-query-with-receipt-fault", "runtime_task",
                    serde_json::json!({"command": command}));
                assert!(matches!(reply.outcome, CommandReceiptOutcome::Committed { result, .. } if result.is_null()));
            }
            let original_attempt = execute("original-attempt-with-receipt-fault", "runtime_task",
                serde_json::json!({"command": nola_application::RuntimeTaskCommand::Attempt {
                    attempt_id: attempt.id.clone(),
                }}));
            let CommandReceiptOutcome::Committed { result, .. } = original_attempt.outcome else {
                panic!("recovery identity read required a write receipt")
            };
            assert_eq!(result["id"], attempt.id);
            assert_eq!(result["lease_token"], attempt.lease_token);
            assert_eq!(result["state"], "superseded");
            fixture
                .execute_batch("DROP TRIGGER fail_service_commit;")
                .unwrap();

            let lookup = execute(
                "lookup-request",
                "command_receipt",
                serde_json::json!({"request_id":"original-request"}),
            );
            let CommandReceiptOutcome::Committed { result, .. } = lookup.outcome else {
                panic!("receipt lookup failed")
            };
            assert!(result.is_null());
            // No automatic resubmission occurred and the original Store data
            // remains in this database after its composition owner reopens.
            let title: String = fixture
                .query_row(
                    "SELECT title FROM conversations WHERE id='conversation'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(title, "Before");
            let count: i64 = fixture
                .query_row("SELECT count(*) FROM command_receipts", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 0);
            let fresh = execute(
                "known-after-reopen",
                "rename_conversation",
                serde_json::json!({
                    "conversation_id": "conversation", "title": "After", "updated_at_ms": 3,
                }),
            );
            assert!(matches!(
                fresh.outcome,
                CommandReceiptOutcome::Committed { .. }
            ));
            let title: String = fixture
                .query_row(
                    "SELECT title FROM conversations WHERE id='conversation'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(title, "After");
        }

        #[test]
        fn idle_projection_wait_does_not_periodically_reload_the_store() {
            use std::sync::atomic::{AtomicUsize, Ordering};
            let directory = tempfile::tempdir().unwrap();
            let shared = super::SharedApplication {
                runtime: super::Mutex::new(Some(
                    nola_application_bootstrap::open(directory.path().join("idle.db")).unwrap(),
                )),
                database_path: directory.path().join("idle.db"),
                projection_waiters: super::Mutex::new(Vec::new()),
            };
            let reads = AtomicUsize::new(0);
            let (connection, _peer) = super::UnixStream::pair().unwrap();
            let started = super::Instant::now();
            let result = shared
                .wait_for_projection(
                    &connection,
                    Some(started + super::Duration::from_millis(150)),
                    |_| {
                        reads.fetch_add(1, Ordering::SeqCst);
                        Ok(false)
                    },
                    |loaded| *loaded,
                )
                .unwrap();
            assert!(!result);
            assert!(started.elapsed() >= super::Duration::from_millis(150));
            // The real socket path reads once initially and once at timeout.
            assert_eq!(reads.load(Ordering::SeqCst), 2);
            assert_eq!(shared.waiter_count(), 0);
        }

        #[test]
        fn idle_runtime_effect_wait_queries_only_initially_and_at_its_deadline() {
            use std::sync::atomic::{AtomicUsize, Ordering};
            let directory = tempfile::tempdir().unwrap();
            let shared = super::SharedApplication {
                runtime: super::Mutex::new(Some(
                    nola_application_bootstrap::open(directory.path().join("idle-runtime-work.db"))
                        .unwrap(),
                )),
                database_path: directory.path().join("idle-runtime-work.db"),
                projection_waiters: super::Mutex::new(Vec::new()),
            };
            let reads = AtomicUsize::new(0);
            let (connection, _peer) = super::UnixStream::pair().unwrap();
            let started = super::Instant::now();
            let schedule = shared
                .wait_for_runtime_effect_work_with(
                    &connection,
                    started + super::Duration::from_millis(150),
                    8,
                    |_, _, limit| {
                        assert_eq!(limit, 8);
                        reads.fetch_add(1, Ordering::SeqCst);
                        Ok(nola_application::RuntimeEffectWorkSchedule {
                            observed_at_ms: 0,
                            runs: Vec::new(),
                            next_wake_at_ms: None,
                        })
                    },
                )
                .unwrap();
            assert!(schedule.runs.is_empty());
            assert!(started.elapsed() >= super::Duration::from_millis(150));
            assert_eq!(reads.load(Ordering::SeqCst), 2);
            assert_eq!(shared.waiter_count(), 0);
        }

        #[test]
        fn runtime_effect_wait_rechecks_at_the_durable_schedule_deadline() {
            use std::sync::atomic::{AtomicUsize, Ordering};
            let directory = tempfile::tempdir().unwrap();
            let shared = super::SharedApplication {
                runtime: super::Mutex::new(Some(
                    nola_application_bootstrap::open(directory.path().join("due-runtime-work.db"))
                        .unwrap(),
                )),
                database_path: directory.path().join("due-runtime-work.db"),
                projection_waiters: super::Mutex::new(Vec::new()),
            };
            let reads = AtomicUsize::new(0);
            let (connection, _peer) = super::UnixStream::pair().unwrap();
            let started = super::Instant::now();
            let schedule = shared.wait_for_runtime_effect_work_with(
                &connection,
                started + super::Duration::from_secs(2),
                1,
                |_, now_ms, _| {
                    let read = reads.fetch_add(1, Ordering::SeqCst);
                    Ok(if read == 0 {
                        nola_application::RuntimeEffectWorkSchedule {
                            observed_at_ms: now_ms,
                            runs: Vec::new(),
                            next_wake_at_ms: Some(now_ms + 100),
                        }
                    } else {
                        nola_application::RuntimeEffectWorkSchedule {
                            observed_at_ms: now_ms,
                            runs: vec![nola_application::RuntimeEffectWorkRunHead {
                                conversation_id: "conversation".into(),
                                run_id: "run".into(),
                                effect_id: "effect".into(),
                                request_id: "request".into(),
                                attempt_id: Some("attempt".into()),
                                reason: nola_application::RuntimeEffectWorkReason::RecoverExpiredLease,
                                ready_at_ms: now_ms,
                                created_at_ms: 1,
                            }],
                            next_wake_at_ms: None,
                        }
                    })
                },
            ).unwrap();
            assert_eq!(schedule.runs.len(), 1);
            assert_eq!(reads.load(Ordering::SeqCst), 2);
            assert!(started.elapsed() >= super::Duration::from_millis(90));
            assert!(started.elapsed() < super::Duration::from_secs(1));
        }

        #[test]
        fn disconnected_runtime_effect_wait_releases_its_registration() {
            let directory = tempfile::tempdir().unwrap();
            let shared = super::SharedApplication {
                runtime: super::Mutex::new(Some(
                    nola_application_bootstrap::open(
                        directory.path().join("disconnected-runtime-work.db"),
                    )
                    .unwrap(),
                )),
                database_path: directory.path().join("disconnected-runtime-work.db"),
                projection_waiters: super::Mutex::new(Vec::new()),
            };
            let (connection, peer) = super::UnixStream::pair().unwrap();
            drop(peer);
            let error = shared
                .wait_for_runtime_effect_work_with(
                    &connection,
                    super::Instant::now() + super::Duration::from_secs(2),
                    1,
                    |_, _, _| {
                        Ok(nola_application::RuntimeEffectWorkSchedule {
                            observed_at_ms: 0,
                            runs: Vec::new(),
                            next_wake_at_ms: None,
                        })
                    },
                )
                .unwrap_err();
            assert!(error.contains("disconnected"));
            assert_eq!(shared.waiter_count(), 0);
        }
    }

    /// Waits efficiently for a client while retaining a bounded opportunity
    /// to observe parent death. A fixed sleep loop adds up to one polling
    /// interval to every short-lived IPC connection and turns a resume path
    /// with several typed reads into seconds of artificial latency.
    fn listener_is_ready(listener: &UnixListener) -> Result<bool, String> {
        let mut descriptor = libc::pollfd {
            fd: listener.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut descriptor, 1, 1_000) };
        if ready > 0 {
            return Ok(descriptor.revents & libc::POLLIN != 0);
        }
        if ready == 0 {
            return Ok(false);
        }
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::Interrupted {
            return Ok(false);
        }
        Err(format!("poll application service listener failed: {error}"))
    }

    fn handle_connection(
        stream: UnixStream,
        application: &SharedApplication,
    ) -> Result<(), String> {
        let mut reader = BufReader::new(super::transport_deadline::DeadlineStream::new(
            &stream,
            Duration::from_secs(30),
        ));
        let mut frame = Vec::new();
        let read = reader
            .by_ref()
            .take((APPLICATION_SERVICE_MAX_FRAME_BYTES + 1) as u64)
            .read_until(b'\n', &mut frame)
            .map_err(|error| format!("read application service request failed: {error}"))?;

        let response = if read == 0 {
            rejected_receipt(
                "unknown",
                "unknown",
                "empty_request",
                "application service request is empty",
                true,
            )
        } else if frame.len() > APPLICATION_SERVICE_MAX_FRAME_BYTES {
            rejected_receipt(
                "unknown",
                "unknown",
                "frame_too_large",
                "application service request exceeds the frame limit",
                false,
            )
        } else {
            handle_frame(&frame, application, &stream)
        };

        let mut writer = BufWriter::new(super::transport_deadline::DeadlineStream::new(
            &stream,
            Duration::from_secs(30),
        ));
        serde_json::to_writer(&mut writer, &response)
            .map_err(|error| format!("encode application service response failed: {error}"))?;
        writer
            .write_all(b"\n")
            .and_then(|_| writer.flush())
            .map_err(|error| format!("write application service response failed: {error}"))
    }

    fn handle_frame(
        frame: &[u8],
        application: &SharedApplication,
        connection: &UnixStream,
    ) -> CommandReceipt {
        let received_at = timestamp_now();
        let envelope: CommandEnvelope = match serde_json::from_slice(frame) {
            Ok(envelope) => envelope,
            Err(error) => {
                return rejected_receipt(
                    "unknown",
                    "unknown",
                    "invalid_request",
                    format!("decode application service request failed: {error}"),
                    true,
                )
            }
        };
        if let Err(error) = envelope.validate() {
            return rejected_receipt(
                &envelope.request_id,
                &envelope.client_id,
                "invalid_request",
                error.to_string(),
                false,
            );
        }

        let request_id = envelope.request_id;
        let client_id = envelope.client_id;
        let expected_revision = envelope.expected_revision;
        let mut command_type = envelope.command.command_type;
        if command_type == "assistant_journal_wait" {
            if expected_revision.is_some() {
                return rejected_receipt(request_id, client_id, "invalid_query_revision",
                    "journal waits use exact cursors, not write revisions", false);
            }
            return assistant_journal_wait_receipt(application, connection, request_id, client_id,
                received_at, envelope.command.arguments);
        }
        if command_type == "projection_cursor_wait" {
            if expected_revision.is_some() {
                return rejected_receipt(
                    request_id,
                    client_id,
                    "invalid_query_revision",
                    "projection reads use projection cursors, not expected write revisions",
                    false,
                );
            }
            return projection_cursor_wait_receipt(
                application,
                connection,
                request_id,
                client_id,
                received_at,
                envelope.command.arguments,
            );
        }
        if command_type == "runtime_effect_work_wait" {
            if expected_revision.is_some() {
                return rejected_receipt(
                    request_id,
                    client_id,
                    "invalid_query_revision",
                    "Runtime effect-work reads do not accept write revisions",
                    false,
                );
            }
            return runtime_effect_work_wait_receipt(
                application,
                connection,
                request_id,
                client_id,
                received_at,
                envelope.command.arguments,
            );
        }
        if command_type == "health" {
            return committed_receipt(
                request_id,
                client_id,
                received_at,
                None,
                json!({
                    "service": "nola-application",
                    "protocol_version": APPLICATION_SERVICE_PROTOCOL_VERSION,
                    "projection_waiters": application.waiter_count(),
                }),
            );
        }
        if command_type == "projection_event_suffix" {
            return projection_event_suffix_receipt(
                application,
                connection,
                request_id,
                client_id,
                received_at,
                envelope.command.arguments,
            );
        }
        if command_type == "command_receipt" {
            return command_receipt_lookup_receipt(
                application,
                request_id,
                client_id,
                received_at,
                envelope.command.arguments,
            );
        }
        if command_type == "projection_snapshot" {
            return projection_snapshot_receipt(
                application,
                request_id,
                client_id,
                received_at,
                envelope.command.arguments,
            );
        }

        if command_type == "host_expert_template_operation" {
            // Same authenticated local Host boundary as Host task imports and
            // resend attestation. Generic Application JSON cannot decode these.
            if expected_revision.is_some() {
                return rejected_receipt(request_id, client_id, "unsupported_revision_precondition",
                    "Expert Host operations use exact source and configuration facts", false);
            }
            let operation = (|| -> Result<nola_application::host_expert_template_operation::HostExpertTemplateOperation, String> {
                let args = envelope.command.arguments.as_object().ok_or("invalid Expert Host arguments")?;
                if args.len() != 1 { return Err("invalid Expert Host fields".into()); }
                serde_json::from_value(args.get("operation").cloned().ok_or("Expert Host operation missing")?)
                    .map_err(|_| "invalid current Expert Host operation".into())
            })();
            let operation = match operation {
                Ok(value) => value,
                Err(error) => return rejected_receipt(request_id, client_id, "invalid_expert_host_operation", error, false),
            };
            let mutating = matches!(&operation,
                nola_application::host_expert_template_operation::HostExpertTemplateOperation::Admit { .. } |
                nola_application::host_expert_template_operation::HostExpertTemplateOperation::PrepareEditedResend { .. });
            let mut runtime = match application.lock() {
                Ok(runtime) => runtime,
                Err(_) if mutating => return CommandReceipt {
                    protocol_version: nola_protocol::APPLICATION_SERVICE_PROTOCOL_VERSION,
                    request_id, client_id, received_at, outcome: nola_protocol::CommandReceiptOutcome::Accepted,
                },
                Err(_) => return rejected_receipt(request_id, client_id, "application_runtime_unavailable",
                    "Application runtime lock is poisoned", true),
            };
            let result = runtime.execute_host_expert_template_operation(&client_id, &request_id, operation);
            if mutating { application.notify_projection_waiters(); }
            return match result {
                Ok(result) => committed_receipt(request_id, client_id, received_at, None, result),
                Err(nola_application::host_expert_template_operation::HostExpertOperationError::Rejected { code, message }) =>
                    rejected_receipt(request_id, client_id, code, message, false),
                Err(nola_application::host_expert_template_operation::HostExpertOperationError::OutcomePending { reopen_required, .. }) => {
                    if reopen_required {
                        // A failed reopen leaves the owner retired. It cannot
                        // turn the original uncertain commit into a rejection.
                        let _ = runtime.reopen();
                    }
                    CommandReceipt { protocol_version: nola_protocol::APPLICATION_SERVICE_PROTOCOL_VERSION,
                        request_id, client_id, received_at, outcome: nola_protocol::CommandReceiptOutcome::Accepted }
                },
            };
        }

        let command_json = match serde_json::to_string(&envelope.command.arguments) {
            Ok(command_json) => command_json,
            Err(error) => {
                return rejected_receipt(
                    request_id,
                    client_id,
                    "invalid_command",
                    format!("encode Application command arguments failed: {error}"),
                    false,
                )
            }
        };
        let decoded_command: Result<ApplicationCommand, String> = if matches!(command_type.as_str(),"host_load_edited_resend_material"|"host_attest_edited_resend_policy") {
            // Same authenticated local Host boundary as host_import below.
            // The internal variants cannot deserialize from Application JSON.
            if expected_revision.is_some() {
                return rejected_receipt(request_id,client_id,"unsupported_revision_precondition",
                    "Edited resend Host reads use exact durable target evidence",false);
            }
            let attest=command_type=="host_attest_edited_resend_policy";
            command_type=if attest {"attest_edited_resend_host_policy"} else {"load_edited_resend_host_material"}.into();
            (|| -> Result<ApplicationCommand,String> {
                let args=envelope.command.arguments.as_object().ok_or("invalid edited resend Host arguments")?;
                let expected=if attest {3} else {1};
                if args.len()!=expected || !args.contains_key("conversation_id") || (attest && (!args.contains_key("core_config") || !args.contains_key("submission"))) {
                    return Err("invalid edited resend Host fields".into());
                }
                let owner=args["conversation_id"].as_str().ok_or("edited resend Host target required")?.to_string();
                if attest {
                    let core_config=serde_json::from_value(args["core_config"].clone()).map_err(|_|"invalid fresh resend Host configuration")?;
                    let submission=serde_json::from_value(args["submission"].clone()).map_err(|_|"invalid fresh resend Host submission")?;
                    Ok(ApplicationCommand::AttestEditedResendHostPolicy {conversation_id:owner,core_config,submission})
                } else {Ok(ApplicationCommand::LoadEditedResendHostMaterial {conversation_id:owner})}
            })()
        } else if command_type == "host_import_agent_task_admission" {
            // Only this trusted Host transport branch constructs the internal
            // variant. Ordinary Application JSON cannot deserialize it.
            if expected_revision.is_some() {
                return rejected_receipt(request_id, client_id,
                    "unsupported_revision_precondition",
                    "Task admission uses immutable source evidence, not an envelope revision", false);
            }
            command_type = "import_host_agent_task_admission".into();
            serde_json::from_value::<nola_protocol::HostAgentTaskAdmissionImport>(envelope.command.arguments)
                .map_err(|error| error.to_string())
                .and_then(|request| {
                    request.admission.validate()?;
                    Ok(ApplicationCommand::ImportHostAgentTaskAdmission { admission: request.admission })
                })
        } else if command_type == "host_import_task_cancellation" {
            if expected_revision.is_some() {
                return rejected_receipt(request_id, client_id, "unsupported_revision_precondition",
                    "Task cancellation uses its exact source control fence", false);
            }
            command_type = "import_host_task_cancellation".into();
            serde_json::from_value::<nola_protocol::HostTaskCancellationImport>(envelope.command.arguments)
                .map_err(|error| error.to_string())
                .and_then(|request| {
                    request.authorization.validate()?;
                    Ok(ApplicationCommand::ImportHostTaskCancellation {
                        authorization: request.authorization, now_ms: epoch_now_ms()?,
                    })
                })
        } else if command_type == "host_import_agent_task_terminal_projection" {
            if expected_revision.is_some() {
                return rejected_receipt(request_id, client_id, "unsupported_revision_precondition",
                    "Task terminal projection uses its exact source identity", false);
            }
            command_type = "import_host_agent_task_terminal_projection".into();
            serde_json::from_value::<nola_protocol::HostAgentTaskTerminalProjectionImport>(envelope.command.arguments)
                .map_err(|error| error.to_string())
                .and_then(|request| {
                    request.projection.validate()?;
                    Ok(ApplicationCommand::ImportHostAgentTaskTerminalProjection { projection: request.projection })
                })
        } else if command_type == "host_import_agent_task_guidance_projection" {
            if expected_revision.is_some() {
                return rejected_receipt(request_id, client_id, "unsupported_revision_precondition",
                    "Task guidance projection uses its exact source identity", false);
            }
            command_type = "import_host_agent_task_guidance_projection".into();
            serde_json::from_value::<nola_protocol::HostAgentTaskGuidanceProjectionImport>(envelope.command.arguments)
                .map_err(|error| error.to_string())
                .and_then(|request| {
                    request.projection.validate()?;
                    Ok(ApplicationCommand::ImportHostAgentTaskGuidanceProjection { projection: request.projection })
                })
        } else {
            let command_value = application_command_value(command_type.clone(), envelope.command.arguments);
            serde_json::from_value(command_value).map_err(|error| error.to_string())
        };
        let mut command: ApplicationCommand = match decoded_command {
            Ok(command) => command,
            Err(error) => {
                return rejected_receipt(
                    request_id,
                    client_id,
                    "invalid_command",
                    format!("decode Application command failed: {error}"),
                    false,
                )
            }
        };
        let query_timing = std::env::var_os("NOLA_SERVICE_QUERY_TIMING")
            .is_some_and(|value| value == "1");
        let lock_started = std::time::Instant::now();
        let mut runtime = match application.lock() {
            Ok(application) => application,
            Err(_) => {
                if !command.is_read_only_query() {
                    // This may be a retry of an earlier ambiguous commit. An
                    // absent owner is not a durable business rejection.
                    return CommandReceipt {
                        protocol_version: APPLICATION_SERVICE_PROTOCOL_VERSION,
                        request_id, client_id, received_at,
                        outcome: nola_protocol::CommandReceiptOutcome::Accepted,
                    };
                }
                return rejected_receipt(
                    request_id,
                    client_id,
                    "runtime_reopen_required",
                    "the canonical Application owner is unavailable",
                    true,
                )
            }
        };
        let lock_elapsed = lock_started.elapsed();
        // Source controls may expire while queued for this single writer.
        // Keep the original arguments as receipt identity, but evaluate actual
        // execution time only after acquiring the Application owner.
        if let ApplicationCommand::ImportHostTaskCancellation { now_ms, .. } = &mut command {
            *now_ms = match epoch_now_ms() {
                Ok(now) => now,
                Err(error) => return rejected_receipt(request_id, client_id,
                    "host_clock_unavailable", error, true),
            };
        }
        if matches!(
            &command,
            ApplicationCommand::RuntimeTask {
                command: nola_application::RuntimeTaskCommand::SuspendForCommandReconciliation { .. }
            }
        ) {
            if expected_revision.is_some() {
                return rejected_receipt(
                    request_id,
                    client_id,
                    "unsupported_revision_precondition",
                    "command reconciliation uses its exact lease witness, not an envelope revision",
                    false,
                );
            }
            // This exact-witness recovery control operation is the boundary
            // used when receipt COMMIT itself is unavailable. Its Store port
            // checks the original receipt and writes Task + reconciliation
            // facts in one canonical transaction. Requiring another admission
            // receipt here would prevent the original uncertainty from being
            // durably fenced by the same failure that caused it.
            return match runtime.execute(command) {
                Ok(result) => {
                    application.notify_projection_waiters();
                    committed_receipt(request_id, client_id, received_at, None, result)
                }
                Err(mut message) => {
                    if let Err(reopen_error) = runtime.reopen() {
                        message.push_str(&format!(
                            "; reopen canonical Application owner failed: {reopen_error}"
                        ));
                    }
                    application.notify_projection_waiters();
                    rejected_receipt(
                        request_id,
                        client_id,
                        "runtime_reconciliation_commit_outcome_unknown",
                        message,
                        false,
                    )
                }
            };
        }
        // Projection reads have the same fresh-read semantics as
        // projection_snapshot/event_suffix, not durable command admission.
        // In particular an idle window subscription must not append a receipt
        // (or retain another copy of the window) on every cursor check.
        // Classify decoded variants, never a client-provided read-only flag.
        if command.is_read_only_query() {
            if expected_revision.is_some() {
                return rejected_receipt(
                    request_id,
                    client_id,
                    "invalid_query_revision",
                    "projection reads use projection cursors, not expected write revisions",
                    false,
                );
            }
            let execute_started = std::time::Instant::now();
            let result = runtime.execute(command);
            if query_timing {
                // Opt-in diagnostics contain command kinds and durations only,
                // never identifiers, arguments, credentials or result bodies.
                eprintln!("nola_query_timing command={} lock_us={} execute_us={}",
                    command_type, lock_elapsed.as_micros(), execute_started.elapsed().as_micros());
            }
            return match result {
                Ok(result) => committed_receipt(request_id, client_id, received_at, None, result),
                Err(message) => rejected_receipt(
                    request_id,
                    client_id,
                    "application_rejected",
                    message,
                    false,
                ),
            };
        }
        let receipt = match runtime.execute_received_command(
            ExecuteReceivedCommand {
                client_id: client_id.clone(),
                request_id: request_id.clone(),
                command_type: command_type.clone(),
                command_json,
                expected_revision,
                received_at,
            },
            command,
        ) {
            Ok(receipt) => receipt,
            Err(error) => {
                let mut message = error.message;
                if runtime_failure_requires_reopen(&error.code) {
                    if let Err(reopen_error) = runtime.reopen() {
                        message.push_str(&format!(
                            "; reopen canonical Application owner failed: {reopen_error}"
                        ));
                    }
                    // The failed acknowledgement might still have committed.
                    // Wake readers to inspect canonical state under the fresh
                    // owner; do not execute the original command again here.
                    application.notify_projection_waiters();
                }
                return rejected_receipt(
                    request_id,
                    client_id,
                    error.code,
                    message,
                    error.recoverable,
                );
            }
        };
        // Admission/commit has completed while the same mutex still protects
        // readers. Waiters recheck their own scope, including on unrelated
        // command completion or a spurious wake, without periodic SQL polling.
        application.notify_projection_waiters();
        receipt.into_protocol_receipt()
    }

    fn runtime_failure_requires_reopen(code: &str) -> bool {
        nola_application::ApplicationCommandExecutionError::code_requires_owner_reopen(code)
    }

    #[test]
    fn failed_savepoint_settlement_retires_the_service_owner() {
        assert!(runtime_failure_requires_reopen("command_savepoint_release_failed"));
        assert!(runtime_failure_requires_reopen("command_savepoint_rollback_failed"));
        assert!(!runtime_failure_requires_reopen("application_rejected"));
        assert!(!runtime_failure_requires_reopen("command_savepoint_begin_failed"));
    }

    fn assistant_journal_wait_receipt(
        application: &SharedApplication, connection: &UnixStream,
        request_id: String, client_id: String, received_at: String, arguments: Value,
    ) -> CommandReceipt {
        let request = serde_json::from_value::<nola_protocol::AssistantJournalWaitRequest>(arguments)
            .map_err(|error| error.to_string()).and_then(|request| { request.validate()?; Ok(request) });
        let request = match request {
            Ok(request) => request,
            Err(error) => return rejected_receipt(request_id, client_id, "invalid_journal_wait", error, false),
        };
        let loaded = application.wait_for_projection(connection,
            Some(Instant::now() + Duration::from_millis(u64::from(request.wait_ms))),
            |runtime| {
                let head = runtime.local_journal_head(&request.after.identity)?;
                if head.identity != request.after.identity || head.sequence < request.after.sequence
                    || (head.sequence == request.after.sequence && head != request.after) {
                    return Err("journal wait cursor differs from committed source".into());
                }
                Ok(head)
            }, |head| head.sequence > request.after.sequence);
        match loaded {
            Ok(head) => committed_receipt(request_id, client_id, received_at, None, json!(head)),
            Err(error) => rejected_receipt(request_id, client_id, "journal_wait_failed", error, false),
        }
    }

    fn projection_cursor_wait_receipt(
        application: &SharedApplication,
        connection: &UnixStream,
        request_id: String,
        client_id: String,
        received_at: String,
        arguments: Value,
    ) -> CommandReceipt {
        let request: nola_protocol::ProjectionCursorWaitRequest =
            match serde_json::from_value(arguments) {
                Ok(request) => request,
                Err(error) => {
                    return rejected_receipt(
                        request_id,
                        client_id,
                        "invalid_projection_request",
                        error.to_string(),
                        false,
                    )
                }
            };
        if let Err(error) = request.validate() {
            return rejected_receipt(
                request_id,
                client_id,
                "invalid_projection_request",
                error.to_string(),
                false,
            );
        }
        let scope = application_projection_scope(&request.after.scope);
        let loaded = application.wait_for_projection(
            connection,
            Some(Instant::now() + Duration::from_millis(u64::from(request.wait_ms))),
            |runtime| {
                let sequence = runtime.projection_cursor(scope.clone())?;
                if request.after.sequence > sequence {
                    return Err("projection cursor is ahead of the committed journal".to_string());
                }
                Ok(sequence)
            },
            |sequence| *sequence > request.after.sequence,
        );
        match loaded {
            Ok(sequence) => committed_receipt(
                request_id,
                client_id,
                received_at,
                None,
                json!(ProjectionCursor {
                    scope: request.after.scope,
                    sequence
                }),
            ),
            Err(error) => rejected_receipt(
                request_id,
                client_id,
                "projection_read_failed",
                error,
                false,
            ),
        }
    }

    fn runtime_effect_work_wait_receipt(
        application: &SharedApplication,
        connection: &UnixStream,
        request_id: String,
        client_id: String,
        received_at: String,
        arguments: Value,
    ) -> CommandReceipt {
        let request: nola_protocol::RuntimeEffectWorkWaitRequest =
            match serde_json::from_value(arguments) {
                Ok(request) => request,
                Err(error) => {
                    return rejected_receipt(
                        request_id,
                        client_id,
                        "invalid_runtime_effect_work_wait",
                        error.to_string(),
                        false,
                    )
                }
            };
        if let Err(error) = request.validate() {
            return rejected_receipt(
                request_id,
                client_id,
                "invalid_runtime_effect_work_wait",
                error.to_string(),
                false,
            );
        }
        let deadline = Instant::now() + Duration::from_millis(u64::from(request.wait_ms));
        match application.wait_for_runtime_effect_work(
            connection,
            deadline,
            &request.conversation_ids,
            request.limit as usize,
        ) {
            Ok(schedule) => match schedule.validate_for_request(&request) {
                Ok(()) => {
                    committed_receipt(request_id, client_id, received_at, None, json!(schedule))
                }
                Err(error) => rejected_receipt(
                    request_id,
                    client_id,
                    "runtime_effect_work_read_failed",
                    error.to_string(),
                    false,
                ),
            },
            Err(error) => rejected_receipt(
                request_id,
                client_id,
                "runtime_effect_work_read_failed",
                error,
                false,
            ),
        }
    }

    fn application_command_value(command_type: String, arguments: Value) -> Value {
        let mut fields = match arguments {
            Value::Object(fields) => fields,
            _ => Map::new(),
        };
        fields.insert("type".to_string(), Value::String(command_type));
        Value::Object(fields)
    }

    fn command_receipt_lookup_receipt(
        application: &SharedApplication,
        request_id: String,
        client_id: String,
        received_at: String,
        arguments: Value,
    ) -> CommandReceipt {
        let request: CommandReceiptLookupRequest = match serde_json::from_value(arguments) {
            Ok(request) => request,
            Err(error) => {
                return rejected_receipt(
                    request_id,
                    client_id,
                    "invalid_receipt_lookup",
                    format!("decode command receipt lookup failed: {error}"),
                    false,
                )
            }
        };
        if let Err(error) = request.validate() {
            return rejected_receipt(
                request_id,
                client_id,
                "invalid_receipt_lookup",
                error.to_string(),
                false,
            );
        }
        let target = {
            let mut application = match application.lock() {
                Ok(application) => application,
                Err(_) => {
                    return rejected_receipt(
                        request_id,
                        client_id,
                        "application_runtime_unavailable",
                        "application runtime lock is poisoned",
                        true,
                    )
                }
            };
            match application.command_receipt(&client_id, &request.request_id) {
                Ok(receipt) => receipt.map(protocol_receipt_from_application),
                Err(error) => {
                    return rejected_receipt(
                        request_id,
                        client_id,
                        error.code,
                        error.message,
                        error.recoverable,
                    )
                }
            }
        };
        committed_receipt(request_id, client_id, received_at, None, json!(target))
    }

    fn protocol_receipt_from_application(receipt: ApplicationCommandReceipt) -> CommandReceipt {
        receipt.into_protocol_receipt()
    }

    fn projection_event_suffix_receipt(
        application: &SharedApplication,
        connection: &UnixStream,
        request_id: String,
        client_id: String,
        received_at: String,
        arguments: Value,
    ) -> CommandReceipt {
        let request: EventSuffixRequest = match serde_json::from_value(arguments) {
            Ok(request) => request,
            Err(error) => {
                return rejected_receipt(
                    request_id,
                    client_id,
                    "invalid_projection_request",
                    format!("decode projection suffix request failed: {error}"),
                    false,
                )
            }
        };
        if let Err(error) = request.validate() {
            return rejected_receipt(
                request_id,
                client_id,
                "invalid_projection_request",
                error.to_string(),
                false,
            );
        }
        if request.scope == ProjectionScope::Conversations {
            return conversation_list_projection_event_suffix_receipt(
                application,
                connection,
                request_id,
                client_id,
                received_at,
                request,
            );
        }
        let store_scope = application_projection_scope(&request.scope);
        let deadline = request
            .wait_ms
            .map(|wait_ms| Instant::now() + Duration::from_millis(u64::from(wait_ms)));
        let loaded = match application.wait_for_projection(
            connection,
            deadline,
            |application| {
                application.projection_event_suffix(
                    store_scope.clone(),
                    request.after.sequence,
                    request.limit as usize,
                )
            },
            |loaded| !loaded.events.is_empty(),
        ) {
            Ok(loaded) => loaded,
            Err(error) => {
                return rejected_receipt(
                    request_id,
                    client_id,
                    "projection_read_failed",
                    error,
                    true,
                )
            }
        };
        let mut events = Vec::with_capacity(loaded.events.len());
        for stored in loaded.events {
            let domain = stored.event;
            if domain.identity.conversation_id.0.as_str()
                != match &request.scope {
                    ProjectionScope::Conversation { conversation_id } => conversation_id.as_str(),
                    _ => return rejected_receipt(
                        request_id,
                        client_id,
                        "projection_journal_corrupt",
                        "a conversation projection event was stored outside a conversation scope",
                        false,
                    ),
                }
            {
                return rejected_receipt(
                    request_id,
                    client_id,
                    "projection_journal_corrupt",
                    "projection event conversation identity does not match its scope",
                    false,
                );
            }
            let message_id = domain
                .mutation
                .message_id()
                .map(|identity| identity.0.clone());
            let part_id = domain.mutation.part_id().map(|identity| identity.0.clone());
            events.push(ProjectionEvent {
                protocol_version: APPLICATION_SERVICE_PROTOCOL_VERSION,
                scope: request.scope.clone(),
                cursor: ProjectionCursor {
                    scope: request.scope.clone(),
                    sequence: domain.identity.sequence,
                },
                identity: nola_protocol::ProjectionEventIdentity {
                    event_id: domain.identity.event_id.0,
                    conversation_id: Some(domain.identity.conversation_id.0),
                    run_id: domain.identity.run_id.map(|identity| identity.0),
                    attempt_id: domain.identity.attempt_id.map(|identity| identity.0),
                    delegated_invocation_id: domain.identity.delegated_invocation_id,
                    message_id,
                    part_id,
                    call_id: None,
                },
                mutation: nola_protocol::ProjectionMutation::Conversation {
                    mutation: domain.mutation,
                },
                occurred_at: stored.occurred_at,
            });
        }
        let next_sequence = events
            .last()
            .map(|event| event.cursor.sequence)
            .unwrap_or(request.after.sequence);
        let suffix = EventSuffix {
            scope: request.scope.clone(),
            after: request.after,
            events,
            next_cursor: ProjectionCursor {
                scope: request.scope,
                sequence: next_sequence,
            },
            has_more: loaded.has_more,
        };
        if let Err(error) = suffix.validate() {
            return rejected_receipt(
                request_id,
                client_id,
                "projection_journal_corrupt",
                error.to_string(),
                false,
            );
        }
        committed_receipt(request_id, client_id, received_at, None, json!(suffix))
    }

    fn conversation_list_projection_event_suffix_receipt(
        application: &SharedApplication,
        connection: &UnixStream,
        request_id: String,
        client_id: String,
        received_at: String,
        request: EventSuffixRequest,
    ) -> CommandReceipt {
        let deadline = request
            .wait_ms
            .map(|wait_ms| Instant::now() + Duration::from_millis(u64::from(wait_ms)));
        let loaded = match application.wait_for_projection(
            connection,
            deadline,
            |application| {
                application.conversation_list_projection_event_suffix(
                    request.after.sequence,
                    request.limit as usize,
                )
            },
            |loaded| !loaded.events.is_empty(),
        ) {
            Ok(loaded) => loaded,
            Err(error) => {
                return rejected_receipt(
                    request_id,
                    client_id,
                    "projection_read_failed",
                    error,
                    true,
                )
            }
        };
        let events = loaded
            .events
            .into_iter()
            .map(|stored| {
                let domain = stored.event;
                let (conversation_id, mutation) = match domain.mutation {
                    nola_domain::ConversationListProjectionMutation::Upsert { conversation } => {
                        let conversation_id = conversation.conversation_id.0.clone();
                        (
                            conversation_id.clone(),
                            nola_protocol::ProjectionMutation::Upsert {
                                record: ProjectionRecord {
                                    entity_type: "conversation_summary".to_string(),
                                    entity_id: conversation_id,
                                    revision: conversation.revision,
                                    payload: json!({
                                        "id": conversation.conversation_id.0,
                                        "title": conversation.title,
                                        "revision": conversation.revision,
                                        "message_count": conversation.message_count,
                                        "created_at_ms": conversation.created_at_ms,
                                        "updated_at_ms": conversation.updated_at_ms,
                                    }),
                                },
                            },
                        )
                    }
                    nola_domain::ConversationListProjectionMutation::Remove {
                        conversation_id,
                        revision,
                    } => {
                        let conversation_id = conversation_id.0;
                        (
                            conversation_id.clone(),
                            nola_protocol::ProjectionMutation::Remove {
                                entity_type: "conversation_summary".to_string(),
                                entity_id: conversation_id,
                                revision,
                            },
                        )
                    }
                };
                ProjectionEvent {
                    protocol_version: APPLICATION_SERVICE_PROTOCOL_VERSION,
                    scope: request.scope.clone(),
                    cursor: ProjectionCursor {
                        scope: request.scope.clone(),
                        sequence: domain.identity.sequence,
                    },
                    identity: nola_protocol::ProjectionEventIdentity {
                        event_id: domain.identity.event_id.0,
                        conversation_id: Some(conversation_id),
                        run_id: None,
                        attempt_id: None,
                        delegated_invocation_id: None,
                        message_id: None,
                        part_id: None,
                        call_id: None,
                    },
                    mutation,
                    occurred_at: stored.occurred_at,
                }
            })
            .collect::<Vec<_>>();
        let next_sequence = events
            .last()
            .map(|event| event.cursor.sequence)
            .unwrap_or(request.after.sequence);
        let suffix = EventSuffix {
            scope: request.scope.clone(),
            after: request.after,
            events,
            next_cursor: ProjectionCursor {
                scope: request.scope,
                sequence: next_sequence,
            },
            has_more: loaded.has_more,
        };
        if let Err(error) = suffix.validate() {
            return rejected_receipt(
                request_id,
                client_id,
                "projection_journal_corrupt",
                error.to_string(),
                false,
            );
        }
        committed_receipt(request_id, client_id, received_at, None, json!(suffix))
    }

    fn projection_snapshot_receipt(
        application: &SharedApplication,
        request_id: String,
        client_id: String,
        received_at: String,
        arguments: Value,
    ) -> CommandReceipt {
        let request: ProjectionSnapshotRequest = match serde_json::from_value(arguments) {
            Ok(request) => request,
            Err(error) => {
                return rejected_receipt(
                    request_id,
                    client_id,
                    "invalid_projection_request",
                    format!("decode projection snapshot request failed: {error}"),
                    false,
                )
            }
        };
        if let Err(error) = request.validate() {
            return rejected_receipt(
                request_id,
                client_id,
                "invalid_projection_request",
                error.to_string(),
                false,
            );
        }
        if request.scope == ProjectionScope::Conversations {
            return conversation_list_projection_snapshot_receipt(
                application,
                request_id,
                client_id,
                received_at,
                request,
            );
        }
        let ProjectionScope::Conversation { conversation_id } = &request.scope else {
            return rejected_receipt(
                request_id,
                client_id,
                "unsupported_projection_scope",
                "projection snapshots currently require a conversation scope",
                false,
            );
        };
        let application = match application.lock() {
            Ok(application) => application,
            Err(_) => {
                return rejected_receipt(
                    request_id,
                    client_id,
                    "application_runtime_unavailable",
                    "application runtime lock is poisoned",
                    true,
                )
            }
        };
        let loaded_result = match request.page.as_ref() {
            Some(page) => {
                let offset = match usize::try_from(page.offset) {
                    Ok(offset) => offset,
                    Err(_) => {
                        return rejected_receipt(
                            request_id,
                            client_id,
                            "invalid_projection_request",
                            "projection page offset exceeds the platform range",
                            false,
                        )
                    }
                };
                application.conversation_projection_page_snapshot(
                    conversation_id,
                    offset,
                    page.limit as usize,
                )
            }
            None => application.conversation_projection_snapshot(conversation_id),
        };
        let loaded = match loaded_result {
            Ok(loaded) => loaded,
            Err(error) => {
                return rejected_receipt(
                    request_id,
                    client_id,
                    "projection_read_failed",
                    error,
                    true,
                )
            }
        };
        let projection_message_count = loaded
            .projection
            .as_ref()
            .map(|projection| projection.messages.len())
            .unwrap_or(0);
        if let (Some(page), Some(projection)) = (&request.page, loaded.projection.as_ref()) {
            if let Some(expected_revision) = page.expected_revision {
                if projection.revision != expected_revision {
                    return CommandReceipt {
                        protocol_version: APPLICATION_SERVICE_PROTOCOL_VERSION,
                        request_id,
                        client_id,
                        received_at,
                        outcome: CommandReceiptOutcome::Conflict {
                            completed_at: timestamp_now(),
                            conflict: RevisionConflict {
                                scope_id: conversation_id.clone(),
                                expected_revision,
                                actual_revision: projection.revision,
                            },
                        },
                    };
                }
            }
        }
        let records = loaded
            .projection
            .map(|projection| {
                vec![ProjectionRecord {
                    entity_type: "conversation_projection".to_string(),
                    entity_id: conversation_id.clone(),
                    revision: projection.revision,
                    payload: json!(projection),
                }]
            })
            .unwrap_or_default();
        let snapshot = ProjectionSnapshot {
            protocol_version: APPLICATION_SERVICE_PROTOCOL_VERSION,
            scope: request.scope.clone(),
            revision: records.first().map(|record| record.revision).unwrap_or(0),
            cursor: ProjectionCursor {
                scope: request.scope,
                sequence: loaded.current_sequence,
            },
            generated_at: timestamp_now(),
            records,
            page: request.page.as_ref().map(|page| ProjectionPage {
                offset: page.offset,
                limit: page.limit,
                has_more: projection_message_count == page.limit as usize,
            }),
        };
        if let Err(error) = snapshot.validate() {
            return rejected_receipt(
                request_id,
                client_id,
                "projection_snapshot_invalid",
                error.to_string(),
                false,
            );
        }
        committed_receipt(request_id, client_id, received_at, None, json!(snapshot))
    }

    fn conversation_list_projection_snapshot_receipt(
        application: &SharedApplication,
        request_id: String,
        client_id: String,
        received_at: String,
        request: ProjectionSnapshotRequest,
    ) -> CommandReceipt {
        let application = match application.lock() {
            Ok(application) => application,
            Err(_) => {
                return rejected_receipt(
                    request_id,
                    client_id,
                    "application_runtime_unavailable",
                    "application runtime lock is poisoned",
                    true,
                )
            }
        };
        let loaded = match application.conversation_list_projection_snapshot() {
            Ok(loaded) => loaded,
            Err(error) => {
                return rejected_receipt(
                    request_id,
                    client_id,
                    "projection_read_failed",
                    error,
                    true,
                )
            }
        };
        let records = match loaded
            .conversations
            .into_iter()
            .map(|conversation| {
                let revision = u64::try_from(conversation.revision).map_err(|_| {
                    "conversation projection contains a negative revision".to_string()
                })?;
                Ok(ProjectionRecord {
                    entity_type: "conversation_summary".to_string(),
                    entity_id: conversation.id.clone(),
                    revision,
                    payload: json!(conversation),
                })
            })
            .collect::<Result<Vec<_>, String>>()
        {
            Ok(records) => records,
            Err(error) => {
                return rejected_receipt(
                    request_id,
                    client_id,
                    "projection_snapshot_invalid",
                    error,
                    false,
                )
            }
        };
        let revision = records
            .iter()
            .map(|record| record.revision)
            .max()
            .unwrap_or(0);
        let snapshot = ProjectionSnapshot {
            protocol_version: APPLICATION_SERVICE_PROTOCOL_VERSION,
            scope: request.scope.clone(),
            revision,
            cursor: ProjectionCursor {
                scope: request.scope,
                sequence: loaded.current_sequence,
            },
            generated_at: timestamp_now(),
            records,
            page: None,
        };
        if let Err(error) = snapshot.validate() {
            return rejected_receipt(
                request_id,
                client_id,
                "projection_snapshot_invalid",
                error.to_string(),
                false,
            );
        }
        committed_receipt(request_id, client_id, received_at, None, json!(snapshot))
    }

    fn application_projection_scope(scope: &ProjectionScope) -> ApplicationProjectionScope {
        match scope {
            ProjectionScope::Conversations => ApplicationProjectionScope {
                scope_type: "conversations".to_string(),
                scope_id: String::new(),
            },
            ProjectionScope::Conversation { conversation_id } => ApplicationProjectionScope {
                scope_type: "conversation".to_string(),
                scope_id: conversation_id.clone(),
            },
            ProjectionScope::Run { run_id } => ApplicationProjectionScope {
                scope_type: "run".to_string(),
                scope_id: run_id.clone(),
            },
            ProjectionScope::Workspace { workspace_id } => ApplicationProjectionScope {
                scope_type: "workspace".to_string(),
                scope_id: workspace_id.clone(),
            },
        }
    }

    fn committed_receipt(
        request_id: impl Into<String>,
        client_id: impl Into<String>,
        received_at: String,
        committed_revision: Option<u64>,
        result: Value,
    ) -> CommandReceipt {
        CommandReceipt {
            protocol_version: APPLICATION_SERVICE_PROTOCOL_VERSION,
            request_id: request_id.into(),
            client_id: client_id.into(),
            received_at,
            outcome: CommandReceiptOutcome::Committed {
                completed_at: timestamp_now(),
                committed_revision,
                result,
            },
        }
    }

    fn rejected_receipt(
        request_id: impl Into<String>,
        client_id: impl Into<String>,
        code: impl Into<String>,
        message: impl Into<String>,
        recoverable: bool,
    ) -> CommandReceipt {
        let now = timestamp_now();
        CommandReceipt {
            protocol_version: APPLICATION_SERVICE_PROTOCOL_VERSION,
            request_id: request_id.into(),
            client_id: client_id.into(),
            received_at: now.clone(),
            outcome: CommandReceiptOutcome::Rejected {
                completed_at: now,
                error: ProtocolFailure {
                    code: code.into(),
                    message: message.into(),
                    recoverable,
                    details: None,
                },
            },
        }
    }

    fn timestamp_now() -> String {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis().to_string())
            .unwrap_or_else(|_| "0".to_string())
    }

    fn epoch_now_ms() -> Result<i64, String> {
        let milliseconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("read Runtime effect-work clock failed: {error}"))?
            .as_millis();
        i64::try_from(milliseconds)
            .map_err(|_| "Runtime effect-work clock exceeds the supported range".to_string())
    }
}

#[cfg(unix)]
pub use unix::serve;
#[cfg(unix)]
pub use unix::serve_with_options;
#[cfg(unix)]
pub use unix::{
    serve_with_options_and_parent, serve_with_recovery_and_parent,
};

#[cfg(not(unix))]
pub fn serve(
    _database_path: &std::path::Path,
    _socket_path: &std::path::Path,
) -> Result<(), String> {
    Err("nola-service local transport is not implemented for this platform".to_string())
}

#[cfg(not(unix))]
pub fn serve_with_options_and_parent(
    _database_path: &std::path::Path,
    _socket_path: &std::path::Path,
    _recreate_incompatible_store: bool,
    _parent_pid: Option<u32>,
) -> Result<(), String> {
    Err("nola-service local transport is not implemented for this platform".to_string())
}

#[cfg(not(unix))]
pub fn serve_with_recovery_and_parent(
    _database_path: &std::path::Path,
    _socket_path: &std::path::Path,
    _recreate_incompatible_store: bool,
    _archive_incompatible_store: Option<&std::path::Path>,
    _parent_pid: Option<u32>,
) -> Result<(), ServiceFailure> {
    Err(ServiceFailure {
        code: "service_startup_failed",
        detail: "nola-service local transport is not implemented for this platform".to_string(),
    })
}
