fn background_task_within_roots(task: &BackgroundBashTask, roots: &[PathBuf]) -> bool {
    let project_root = task
        .project_root
        .canonicalize()
        .unwrap_or_else(|_| task.project_root.clone());
    let cwd = task
        .cwd
        .canonicalize()
        .unwrap_or_else(|_| task.cwd.clone());
    roots.iter().any(|root| {
        let root = root.canonicalize().unwrap_or_else(|_| root.clone());
        project_root == root || project_root.starts_with(&root) || cwd.starts_with(&root)
    })
}

fn background_elapsed_secs_until(started_at_ms: i64, ended_at_ms: Option<i64>) -> f64 {
    ended_at_ms.unwrap_or_else(unix_epoch_millis)
        .saturating_sub(started_at_ms)
        .max(0) as f64
        / 1_000.0
}

#[cfg(test)]
#[test]
fn cli_polish_terminal_background_duration_stops_at_the_canonical_end() {
    assert_eq!(background_elapsed_secs_until(1000, Some(1250)), 0.25);
    assert_eq!(background_elapsed_secs_until(1000, Some(900)), 0.0);
}

fn terminate_process_tree(
    child: &mut Child,
    deadline: Duration,
) -> Result<nola_domain::execution_evidence::LocalExecutionStopScope, String> {
    crate::task_runtime::stop_borrowed_process_group(child, deadline)
}

struct BackgroundOutputSlice {
    content: String,
    next_seq: u64,
    total_bytes: u64,
    skipped_before_chunk: bool,
}

fn background_output_since(
    path: &Path,
    after_seq: Option<u64>,
    max_bytes: usize,
) -> BackgroundOutputSlice {
    let Ok(mut file) = fs::File::open(path) else {
        return BackgroundOutputSlice {
            content: String::new(),
            next_seq: after_seq.unwrap_or(0),
            total_bytes: 0,
            skipped_before_chunk: false,
        };
    };
    let total_bytes = file.metadata().map(|value| value.len()).unwrap_or(0);
    let requested_start = after_seq.unwrap_or_else(|| total_bytes.saturating_sub(max_bytes as u64));
    let start = requested_start.min(total_bytes);
    let skipped_before_chunk = after_seq.is_none() && start > 0;
    if std::io::Seek::seek(&mut file, std::io::SeekFrom::Start(start)).is_err() {
        return BackgroundOutputSlice {
            content: String::new(),
            next_seq: start,
            total_bytes,
            skipped_before_chunk,
        };
    }
    let mut bytes = Vec::with_capacity(max_bytes.min(total_bytes.saturating_sub(start) as usize));
    let mut limited = std::io::Read::take(file, max_bytes as u64);
    let _ = std::io::Read::read_to_end(&mut limited, &mut bytes);
    BackgroundOutputSlice {
        content: String::from_utf8_lossy(&bytes).to_string(),
        next_seq: start.saturating_add(bytes.len() as u64),
        total_bytes,
        skipped_before_chunk,
    }
}

fn format_background_bash_task(
    task_id: &str,
    task: &mut BackgroundBashTask,
    max_bytes: usize,
    args: &Map<String, Value>,
) -> ToolOutcome {
    let observation = match observe_background_task(task) {
        Ok(observation) => observation,
        Err(error) => return foreground_task_observation_error(task_id, "observe", error),
    };
    format_observed_background_bash_task(task_id, task, max_bytes, args, observation)
}

fn format_observed_background_bash_task(
    task_id: &str,
    task: &mut BackgroundBashTask,
    max_bytes: usize,
    args: &Map<String, Value>,
    observation: BackgroundTaskObservation,
) -> ToolOutcome {
    let status = observation.label;
    let stdout = background_output_since(
        &task.stdout_path,
        args.get("stdout_after_seq").and_then(Value::as_u64),
        max_bytes,
    );
    let stderr = background_output_since(
        &task.stderr_path,
        args.get("stderr_after_seq").and_then(Value::as_u64),
        max_bytes,
    );
    let exited = observation.terminal;
    let exit_code = observation.exit_code;
    let mut outcome = ok(
        format!(
            "# Bash background task: {task_id}\ncommand: {}\ncwd: {}\nstatus: {status}\nexited: {exited}\nexit_code: {}\nelapsed_sec: {:.1}\nstdout_next_seq: {}\nstdout_total_bytes: {}\nstdout_skipped_before_chunk: {}\nstderr_next_seq: {}\nstderr_total_bytes: {}\nstderr_skipped_before_chunk: {}\n\n--- stdout chunk ---\n{}\n--- stderr chunk ---\n{}",
            task.command,
            task.cwd.display(),
            exit_code.map(|value| value.to_string()).unwrap_or_else(|| "null".to_string()),
            background_elapsed_secs_until(task.started_at_ms, observation.ended_at_ms),
            stdout.next_seq,
            stdout.total_bytes,
            stdout.skipped_before_chunk,
            stderr.next_seq,
            stderr.total_bytes,
            stderr.skipped_before_chunk,
            stdout.content,
            stderr.content
        ),
        format!("Background bash {task_id} {status}"),
    );
    if exited {
        if let Some(settlement) = settle_background_output_contract(task, &status) {
            if settlement.execution.as_ref().is_some_and(|receipt| receipt.state == nola_host_protocol::tool_result::ExecutionState::Unknown) {
                return settlement;
            }
            outcome.artifacts = settlement.artifacts;
            if settlement.error {
                outcome.error = true;
                outcome.text.push_str("\n\n--- output contract ---\n");
                outcome.text.push_str(&settlement.text);
                outcome.display = settlement.display;
            }
        }
    }
    outcome.data = Some(json!({"task_id": task_id, "state": observation.state, "terminal": exited, "cancel_requested": observation.cancel_requested, "exit_code": exit_code}));
    // Project only terminal identities and receipts, never the durable input
    // queue or unrelated task metadata (which may contain sensitive values).
    match crate::runtime_task_port::get(task_id) {
        Ok(Some(runtime)) => {
            let metadata: Value = match serde_json::from_str(&runtime.metadata_json) {
                Ok(metadata) => metadata,
                Err(err) => return error(format!("Invalid task metadata: {err}")),
            };
            if metadata.get("terminal").is_some_and(|value| !value.is_null()) {
                let attempt = match crate::runtime_task_port::current_attempt(task_id) {
                    Ok(attempt) => attempt,
                    Err(err) => return error(format!("Read terminal attempt failed: {err}")),
                };
                let receipts: std::collections::BTreeMap<String, nola_domain::terminal_input::TerminalInputReceipt> =
                    match serde_json::from_value(metadata.get(nola_domain::terminal_input::TERMINAL_INPUT_RECEIPTS_METADATA_KEY)
                        .cloned().unwrap_or_else(|| json!({}))) {
                        Ok(receipts) => receipts,
                        Err(err) => return error(format!("Invalid terminal receipts: {err}")),
                    };
                for (id, receipt) in &receipts {
                    if receipt.validate().is_err() || id != &receipt.identity.request_id
                        || receipt.identity.task_id != runtime.id
                        || Some(receipt.identity.owner_session_id.as_str()) != runtime.owner_session_id.as_deref() {
                        return error("Terminal receipt identity is invalid".into());
                    }
                }
                let selected = args.get("input_request_id").and_then(Value::as_str);
                if selected.is_some_and(|id| !receipts.contains_key(id)) {
                    return error("No receipt for this input_request_id; absence does not authorize replay".into());
                }
                let visible = receipts.iter().filter(|(id, _)| selected.is_none_or(|selected| selected == id.as_str()))
                    .take(16).collect::<std::collections::BTreeMap<_, _>>();
                let terminal = json!({
                    "attempt_id": attempt.as_ref().map(|value| &value.id),
                    "worker_generation": attempt.as_ref().map(|value| &value.worker_generation),
                    "input_receipts": visible,
                    "input_receipt_count": receipts.len(),
                    "input_receipts_truncated": selected.is_none() && receipts.len() > visible.len(),
                    "input_receipt_order": "request_id",
                });
                outcome.text.push_str(&format!("\nterminal_session: {terminal}"));
                if let Some(data) = outcome.data.as_mut() { data["terminal_session"] = terminal; }
            }
        }
        Ok(None) => {}
        Err(err) => return error(format!("Read task state failed: {err}")),
    }
    outcome
}

pub fn manage_background_bash_tasks(
    action: &str,
    task_id: Option<&str>,
    allowed_dirs: &[PathBuf],
    owner_session_id: Option<&str>,
    skills: &crate::skill_runtime::SkillRuntimeContext,
) -> ToolOutcome {
    let mut args = Map::new();
    if let Some(task_id) = task_id.map(str::trim).filter(|value| !value.is_empty()) {
        args.insert("task_id".to_string(), Value::String(task_id.to_string()));
    }
    args.insert("max_bytes".to_string(), Value::from(20_000));
    let permission_scope = SessionPermissionScope::default();
    if let Some(owner_session_id) = owner_session_id {
        permission_scope.bind_owner_session(owner_session_id.to_string());
    }
    let ctx = ToolContext {
        memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
        language_services: Default::default(),
        skills: skills.clone(),
        allowed_dirs: allowed_dirs.to_vec(),
        policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), true, "deny".to_string(), Vec::new()),
        permission_scope,
        cancellation: Default::default(),
        plan_state: Default::default(),
        output_store: ToolOutputStore::for_session(
            &tool_try!(crate::platform_paths::application_database_path().map_err(error)),
            owner_session_id.unwrap_or("background-management"),
        ),
        browser: BrowserRuntime::new(
            std::env::temp_dir().join(format!("nola-cli-jobs-{}", std::process::id())),
        ),
    };
    match action.trim().to_ascii_lowercase().as_str() {
        "list" | "status" => bash_background_status(&args, &ctx),
        "stop" => {
            if !args.contains_key("task_id") {
                return error("task_id is required for jobs stop".to_string());
            }
            bash_background_stop(&args, &ctx)
        }
        _ => error("jobs action must be list, status, or stop".to_string()),
    }
}

pub fn background_bash_task_summaries(
    allowed_dirs: &[PathBuf],
    owner_session_id: Option<&str>,
) -> Result<Vec<BackgroundTaskSummary>, String> {
    background_bash_task_summaries_scoped(allowed_dirs, Some(owner_session_id))
}

/// Returns every shell task in the allowed workspace, regardless of owning
/// conversation. This is reserved for cold-start Supervisor reconciliation;
/// user-facing task views must keep using the owner-scoped function above.
pub fn background_bash_task_summaries_for_workspace(
    allowed_dirs: &[PathBuf],
) -> Result<Vec<BackgroundTaskSummary>, String> {
    background_bash_task_summaries_scoped(allowed_dirs, None)
}

fn background_bash_task_summaries_scoped(
    allowed_dirs: &[PathBuf],
    owner_filter: Option<Option<&str>>,
) -> Result<Vec<BackgroundTaskSummary>, String> {
    let state = BACKGROUND_BASH_TASKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = state
        .lock()
        .map_err(|_| "background task state lock poisoned".to_string())?;
    sync_background_tasks(&mut guard)?;
    let mut visible = guard
        .iter()
        .filter(|(_, task)| {
            owner_filter
                .is_none_or(|owner_session_id| task.owner_session_id.as_deref() == owner_session_id)
                && background_task_within_roots(task, allowed_dirs)
        })
        .map(|(id, task)| (id.clone(), task.started_at_ms))
        .collect::<Vec<_>>();
    visible.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));

    let mut summaries = Vec::with_capacity(visible.len());
    for (id, _) in visible {
        let Some(task) = guard.get_mut(&id) else {
            continue;
        };
        let observation = observe_background_task(task)?;
        let status = observation.label;
        if observation.terminal {
            let _ = settle_background_output_contract(task, &status);
        }
        let command = task.command.clone();
        let pid = task.pid;
        let elapsed_secs = background_elapsed_secs_until(task.started_at_ms, observation.ended_at_ms);
        summaries.push(BackgroundTaskSummary {
            id,
            canonical_state: observation.state,
            status,
            command,
            pid,
            elapsed_secs,
            owner_session_id: task.owner_session_id.clone(),
        });
    }
    Ok(summaries)
}

pub fn take_background_task_notifications(
    allowed_dirs: &[PathBuf],
    owner_session_id: Option<&str>,
) -> Result<Vec<BackgroundTaskNotification>, String> {
    drain_background_task_notification_wake();
    let state = BACKGROUND_BASH_TASKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = state
        .lock()
        .map_err(|_| "background task state lock poisoned".to_string())?;
    sync_background_tasks(&mut guard)?;
    let mut candidates = guard
        .iter()
        .filter(|(_, task)| {
            task.notify_on_completion
                && task.owner_session_id.as_deref() == owner_session_id
                && background_task_within_roots(task, allowed_dirs)
        })
        .map(|(id, task)| (id.clone(), task.started_at_ms))
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0)));

    let mut notifications = Vec::new();
    let store = NolaStore::open_default()?;
    for (id, _) in candidates {
        let Some(task) = guard.get_mut(&id) else {
            continue;
        };
        let Some(runtime_task) = store.runtime_task(&id)? else {
            return Err(format!(
                "background task {id} disappeared before its completion notification was acknowledged"
            ));
        };
        // Notification label/state and delivery identity derive from the same
        // canonical task snapshot, not two independently observed revisions.
        let observation = classify_background_task_observation(task, Ok(Some(runtime_task.clone())))?;
        if !observation.terminal { continue; }
        let status = observation.label;
        let _ = settle_background_output_contract(task, &status);
        let delivery = crate::runtime_task_port::terminal_notification_delivery(&runtime_task)?;
        if crate::runtime_task_port::notification_delivered(&delivery)? { continue; }
        // Preparing a notification is read-only with respect to UI delivery.
        // The presenter must acknowledge it after successful terminal output.
        notifications.push(BackgroundTaskNotification { delivery, summary: BackgroundTaskSummary {
            id: id.clone(),
            canonical_state: observation.state,
            status,
            command: task.command.clone(),
            pid: task.pid,
            elapsed_secs: background_elapsed_secs_until(task.started_at_ms, observation.ended_at_ms),
            owner_session_id: task.owner_session_id.clone(),
        }});
    }
    Ok(notifications)
}

pub fn acknowledge_background_task_notification(notification: &BackgroundTaskNotification) -> Result<nola_application::RuntimeTaskNotificationAcknowledgement, String> {
    crate::runtime_task_port::acknowledge_notification_delivery(&notification.delivery)
}

pub fn deliver_background_task_notification(notification: &BackgroundTaskNotification,
    present: impl FnOnce() -> std::io::Result<crate::task_runtime::NotificationPresentation>) -> Result<Option<nola_application::RuntimeTaskNotificationAcknowledgement>, String> {
    match present().map_err(|error| format!("Background notification was not presented: {error}"))? {
        crate::task_runtime::NotificationPresentation::Deferred => Ok(None),
        crate::task_runtime::NotificationPresentation::Presented => acknowledge_background_task_notification(notification).map(Some),
    }
}

#[cfg(unix)]
static BACKGROUND_NOTIFICATION_WRITE_FD: AtomicI32 = AtomicI32::new(-1);

#[cfg(unix)]
struct BackgroundNotificationPipe {
    read_fd: libc::c_int,
    _write_fd: libc::c_int,
}

#[cfg(unix)]
static BACKGROUND_NOTIFICATION_PIPE: OnceLock<Option<BackgroundNotificationPipe>> = OnceLock::new();

#[cfg(unix)]
extern "C" fn background_sigchld_handler(_signal: libc::c_int) {
    let write_fd = BACKGROUND_NOTIFICATION_WRITE_FD.load(Ordering::Relaxed);
    if write_fd < 0 {
        return;
    }
    let byte = 1_u8;
    unsafe {
        libc::write(write_fd, (&byte as *const u8).cast::<libc::c_void>(), 1);
    }
}

/// Wake the interactive composer when any asynchronous runtime task changes.
///
/// Agent tasks share the same self-pipe as background shell tasks so the UI
/// has one event-driven notification path instead of polling either runner.
#[cfg(unix)]
pub(crate) fn wake_background_task_notification() {
    background_sigchld_handler(0);
}

#[cfg(not(unix))]
pub(crate) fn wake_background_task_notification() {}

#[cfg(unix)]
fn create_background_notification_pipe() -> Option<BackgroundNotificationPipe> {
    let mut fds = [-1; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return None;
    }

    for fd in fds {
        let status_flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        let descriptor_flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        if status_flags < 0
            || descriptor_flags < 0
            || unsafe { libc::fcntl(fd, libc::F_SETFL, status_flags | libc::O_NONBLOCK) } < 0
            || unsafe { libc::fcntl(fd, libc::F_SETFD, descriptor_flags | libc::FD_CLOEXEC) } < 0
        {
            unsafe {
                libc::close(fds[0]);
                libc::close(fds[1]);
            }
            return None;
        }
    }

    BACKGROUND_NOTIFICATION_WRITE_FD.store(fds[1], Ordering::Release);
    let mut action = unsafe { std::mem::zeroed::<libc::sigaction>() };
            action.sa_sigaction = background_sigchld_handler as *const () as usize;
    action.sa_flags = libc::SA_RESTART | libc::SA_NOCLDSTOP;
    unsafe {
        libc::sigemptyset(&mut action.sa_mask);
    }
    if unsafe { libc::sigaction(libc::SIGCHLD, &action, std::ptr::null_mut()) } != 0 {
        BACKGROUND_NOTIFICATION_WRITE_FD.store(-1, Ordering::Release);
        unsafe {
            libc::close(fds[0]);
            libc::close(fds[1]);
        }
        return None;
    }

    Some(BackgroundNotificationPipe {
        read_fd: fds[0],
        _write_fd: fds[1],
    })
}

#[cfg(unix)]
pub fn background_task_notification_fd() -> Option<libc::c_int> {
    BACKGROUND_NOTIFICATION_PIPE
        .get_or_init(create_background_notification_pipe)
        .as_ref()
        .map(|pipe| pipe.read_fd)
}

#[cfg(not(unix))]
pub fn background_task_notification_fd() -> Option<i32> {
    None
}

#[cfg(unix)]
fn drain_background_task_notification_wake() {
    let Some(read_fd) = background_task_notification_fd() else {
        return;
    };
    let mut buffer = [0_u8; 128];
    loop {
        let read = unsafe {
            libc::read(
                read_fd,
                buffer.as_mut_ptr().cast::<libc::c_void>(),
                buffer.len(),
            )
        };
        if read <= 0 {
            break;
        }
    }
}

#[cfg(not(unix))]
fn drain_background_task_notification_wake() {}

fn bash_background_status(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    if args.contains_key("input_request_id") {
        if let Err(outcome) = required_string(args, "input_request_id") { return outcome; }
        if let Err(outcome) = required_string(args, "task_id") { return outcome; }
    }
    let max_bytes = tool_try!(validate_integer_arg(
        args,
        "max_bytes",
        20_000,
        1_000,
        MAX_BACKGROUND_OUTPUT_BYTES
    )) as usize;
    let state = BACKGROUND_BASH_TASKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = match state.lock() {
        Ok(guard) => guard,
        Err(_) => return error("background task state lock poisoned".to_string()),
    };
    if let Err(err) = sync_background_tasks(&mut guard) {
        return error(format!("Load background tasks failed: {err}"));
    }
    if let Some(task_id) = arg_string(args, "task_id") {
        if let Err(err) = hydrate_background_task(&mut guard, &task_id) {
            return foreground_task_observation_error(&task_id, "observe", format!("Load background task failed: {err}"));
        }
        let Some(task) = guard.get_mut(&task_id) else {
            return foreground_task_observation_error(&task_id, "observe", "No visible task record; original execution outcome remains unknown".into());
        };
        if !background_task_visible(task, ctx) {
            return error(format!("Unknown background bash task: {task_id}"));
        }
        return format_background_bash_task(&task_id, task, max_bytes, args);
    }
    let mut visible_tasks = guard
        .iter()
        .filter(|(_, task)| background_task_visible(task, ctx))
        .map(|(task_id, task)| (task_id.clone(), task.started_at_ms))
        .collect::<Vec<_>>();
    visible_tasks.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    if visible_tasks.is_empty() {
        return ok(
            "(no background bash tasks)".to_string(),
            "0 background bash tasks".to_string(),
        );
    }
    let mut lines = vec!["# Bash background tasks".to_string()];
    for (task_id, _) in &visible_tasks {
        let Some(task) = guard.get_mut(task_id) else {
            continue;
        };
        let observation = match observe_background_task(task) {
            Ok(observation) => observation,
            Err(error) => return foreground_task_observation_error(task_id, "observe", error),
        };
        let status = observation.label;
        if observation.terminal {
            let _ = settle_background_output_contract(task, &status);
        }
        lines.push(format!(
            "- {task_id}: {status} | pid={} | {} | cwd={}",
            task.pid,
            truncate(&task.command, 100),
            task.cwd.display()
        ));
    }
    ok(
        lines.join("\n"),
        format!("{} background bash tasks", visible_tasks.len()),
    )
}

fn bash_background_wait(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let task_id = tool_try!(required_string(args, "task_id"));
    let timeout =
        Duration::from_secs(
            tool_try!(validate_integer_arg(args, "timeout_sec", 30, 1, 300)) as u64,
        );
    let poll_interval = Duration::from_millis(tool_try!(validate_integer_arg(
        args,
        "poll_interval_ms",
        250,
        100,
        5_000
    )) as u64);
    let max_bytes = tool_try!(validate_integer_arg(
        args,
        "max_bytes",
        20_000,
        1_000,
        MAX_BACKGROUND_OUTPUT_BYTES
    )) as usize;
    let started = Instant::now();
    let state = BACKGROUND_BASH_TASKS.get_or_init(|| Mutex::new(HashMap::new()));
    {
        let mut guard = match state.lock() {
            Ok(guard) => guard,
            Err(_) => return error("background task state lock poisoned".to_string()),
        };
        if let Err(err) = sync_background_tasks(&mut guard) {
            return error(format!("Load background tasks failed: {err}"));
        }
        if let Err(err) = hydrate_background_task(&mut guard, &task_id) {
            return foreground_task_observation_error(&task_id, "observe", format!("Load background task failed: {err}"));
        }
    }
    loop {
        if ctx.cancellation.is_requested() {
            return foreground_task_observation_error(&task_id, "observe", "Background wait interrupted; original task execution outcome remains unconfirmed".into());
        }
        let mut guard = match state.lock() {
            Ok(guard) => guard,
            Err(_) => return error("background task state lock poisoned".to_string()),
        };
        let Some(task) = guard.get_mut(&task_id) else {
            return foreground_task_observation_error(&task_id, "observe", "No visible task record; original execution outcome remains unknown".into());
        };
        if !background_task_visible(task, ctx) {
            return error(format!("Unknown background bash task: {task_id}"));
        }
        let observation = match observe_background_task(task) {
            Ok(observation) => observation,
            Err(error) => return foreground_task_observation_error(&task_id, "observe", error),
        };
        let finished = observation.terminal;
        if finished || started.elapsed() >= timeout {
            let mut outcome = format_observed_background_bash_task(&task_id, task, max_bytes, args, observation);
            if !finished {
                outcome.text.push_str(&format!(
                    "\nwait_timed_out: true\nwaited_sec: {:.1}\nnext_action: call bash_background_wait again with a suitable timeout, do other independent work, or stop the task if it is no longer needed",
                    started.elapsed().as_secs_f64()
                ));
                outcome.display = format!("Background bash {task_id} remains pending");
            }
            return outcome;
        }
        drop(guard);
        let remaining = timeout.saturating_sub(started.elapsed());
        std::thread::sleep(poll_interval.min(remaining));
    }
}

fn terminal_input(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let Some(identity) = current_local_execution_identity() else {
        return error("Terminal input requires an admitted execution identity".into());
    };
    let Some(owner) = ctx.permission_scope.owner_session_id() else {
        return error("Terminal input requires an owning conversation".into());
    };
    if identity.session_id != owner {
        return error("Terminal input execution belongs to another conversation".into());
    }
    let input = nola_domain::terminal_input::TerminalInputRequest {
        identity: nola_domain::terminal_input::TerminalInputIdentity {
            host_id: identity.host_id,
            owner_session_id: owner,
            task_id: tool_try!(required_string(args, "task_id")),
            attempt_id: tool_try!(required_string(args, "attempt_id")),
            worker_generation: tool_try!(required_string(args, "worker_generation")),
            request_id: tool_try!(required_string(args, "request_id")),
        },
        text: match args.get("text").and_then(Value::as_str) {
            Some(text) => text.to_owned(),
            None => return error("text must be a string".into()),
        },
    };
    match crate::runtime_task_port::submit_terminal_input(input, unix_epoch_millis()) {
        Ok(receipt) => {
            let data = match serde_json::to_value(receipt) {
                Ok(data) => data,
                Err(err) => return error(format!("Encode terminal receipt failed: {err}")),
            };
            let mut outcome = ok(data.to_string(), "Terminal input receipt recorded; inspect delivery status".into());
            outcome.data = Some(data);
            outcome
        }
        Err(err) => error(err),
    }
}

fn bash_background_stop(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let task_id = tool_try!(required_string(args, "task_id"));
    let max_bytes = tool_try!(validate_integer_arg(
        args,
        "max_bytes",
        20_000,
        1_000,
        MAX_BACKGROUND_OUTPUT_BYTES
    )) as usize;
    let state = BACKGROUND_BASH_TASKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = match state.lock() {
        Ok(mut guard) => {
            if let Err(err) = sync_background_tasks(&mut guard) {
                return error(format!("Load background tasks failed: {err}"));
            }
            if let Err(err) = hydrate_background_task(&mut guard, &task_id) {
                return foreground_task_observation_error(&task_id, "observe", format!("Load background task failed: {err}"));
            }
            match guard.get(&task_id) {
                Some(task) if background_task_visible(task, ctx) => {}
                _ => return error(format!("Unknown background bash task: {task_id}")),
            }
            guard
        }
        Err(_) => return error("background task state lock poisoned".to_string()),
    };
    let task = guard.get_mut(&task_id).expect("visibility checked under same lock");
    let observation = match observe_background_task(task) {
        Ok(observation) => observation,
        Err(error) => return foreground_task_observation_error(&task_id, "observe", error),
    };
    if !observation.terminal && !observation.cancel_requested {
        if let Err(error) = crate::runtime_task_port::request_cancel(&task_id, unix_epoch_millis()) {
            return foreground_task_observation_error(&task_id, "cancel_submit", error);
        }
    }
    // Only the supervisor owning the canonical attempt may terminate it.
    // Keep output evidence and the local index regardless of cancel ACK.
    let after = match observe_background_task(task) {
        Ok(observation) => observation,
        Err(error) => return foreground_task_observation_error(&task_id, "cancel_observe", error),
    };
    if !after.terminal && !after.cancel_requested {
        return foreground_task_observation_error(&task_id, "cancel_observe", "Cancellation not recorded; original task outcome remains unknown".into());
    }
    let pending = !after.terminal;
    let mut outcome = format_observed_background_bash_task(&task_id, task, max_bytes, args, after);
    if pending {
        outcome.display = format!("Cancellation requested for background bash {task_id}; awaiting terminal status");
        outcome.execution = Some(nola_host_protocol::tool_result::ExecutionReceipt {
            state: nola_host_protocol::tool_result::ExecutionState::Deferred, ..Default::default()
        });
    }
    outcome
}


// Every transport invocation is gated by the same URL policy, including redirects.
// Production transport additionally validates and pins every resolved address.
fn fetch_public_web_response(
    mut target: reqwest::Url,
    mut send: impl FnMut(&reqwest::Url) -> Result<reqwest::blocking::Response, ToolOutcome>,
) -> Result<reqwest::blocking::Response, ToolOutcome> {
    let mut redirects = 0;
    loop {
        validate_public_web_url(target.as_str(), "request_url")?;
        let response = send(&target)?;
        if !response.status().is_redirection() { return Ok(response); }
        let Some(location) = response.headers().get(reqwest::header::LOCATION) else { return Ok(response); };
        if redirects >= 10 { return Err(error("Fetch exceeded 10 redirects".into())); }
        target = location.to_str().ok().and_then(|value| target.join(value).ok())
            .ok_or_else(|| error("Fetch received invalid redirect URL".into()))?;
        redirects += 1;
    }
}

fn web_fetch(args: &Map<String, Value>) -> ToolOutcome {
    let url = tool_try!(required_string(args, "url"));
    if let Err(outcome) = validate_public_web_url(&url, "url") {
        return outcome;
    }
    let max_chars = tool_try!(validate_integer_arg(
        args,
        "max_chars",
        12_000,
        500,
        MAX_WEB_RESPONSE_CHARS,
    )) as usize;
    let target = tool_try!(reqwest::Url::parse(&url).map_err(|e| error(e.to_string())));
    let mut response = tool_try!(fetch_public_web_response(target, |target| {
        let host = target.host_str().unwrap_or_default().to_string();
        let addresses = match std::net::ToSocketAddrs::to_socket_addrs(&(host.trim_matches(['[', ']']), target.port_or_known_default().unwrap_or(443))) {
            Ok(values) => values.collect::<Vec<_>>(),
            Err(err) => return Err(error(format!("Fetch DNS resolution failed: {err}"))),
        };
        if let Err(outcome) = validate_public_web_addresses(&addresses) { return Err(outcome); }
        // Pin the validated answers so a second DNS lookup cannot rebind the connection.
        let client = match reqwest::blocking::Client::builder().no_proxy()
            .resolve_to_addrs(&host, &addresses)
            .timeout(Duration::from_secs(20)).redirect(reqwest::redirect::Policy::none())
            .user_agent("NolaCLI/1.0 (+https://nola.local)").build() {
            Ok(client) => client, Err(err) => return Err(error(format!("Fetch client failed: {err}"))),
        };
        let response = match client.get(target.clone()).header("Accept-Language", browser_accept_language()).send() {
            Ok(response) => response, Err(err) => return Err(error(format!("Fetch failed: {err}"))),
        };
        Ok(response)
    }));
    let final_url = response.url().as_str().to_string();
    if let Err(outcome) = validate_public_web_url(&final_url, "final_url") {
        return outcome;
    }
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let mut bytes = Vec::new();
    if let Err(err) = Read::by_ref(&mut response)
        .take(MAX_WEB_DOWNLOAD_BYTES + 1)
        .read_to_end(&mut bytes)
    {
        return error(format!("Fetch read failed: {err}"));
    }
    if bytes.len() as u64 > MAX_WEB_DOWNLOAD_BYTES { return error("Fetch download exceeds size limit; response was not complete".into()); }
    let raw_text = String::from_utf8_lossy(&bytes).to_string();
    let (title, body) = if content_type.to_ascii_lowercase().contains("html")
        || raw_text.trim_start().starts_with('<')
    {
        html_to_text(&raw_text)
    } else {
        (String::new(), raw_text)
    };
    let mut output = if title.is_empty() {
        body
    } else {
        format!("# {title}\n\n{body}")
    };
    let original_chars = output.chars().count();
    if original_chars > max_chars {
        output = format!(
            "{}\n\n...[truncated, fetched {original_chars} chars, returning {max_chars}]",
            output.chars().take(max_chars).collect::<String>()
        );
    }
    ok(
        format!(
            "URL: {url}\nFinal URL: {final_url}\nStatus: {status}\nContent-Type: {}\n\n{}",
            content_type.to_ascii_lowercase(),
            output
        ),
        format!("Fetched {final_url} ({status}, {original_chars} chars)"),
    )
}

#[derive(Clone)]
struct NativeBrowserBridgeConfig {
    base_url: String,
    token: String,
    local_user_id: String,
    host_id: String,
}

fn run_chrome_native_tool(name: &str, args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let Some(action) = chrome_native_action(name) else {
        return error(format!("Unknown tool '{name}'"));
    };
    if let Err(outcome) = validate_chrome_native_tool_args(action, args) {
        return outcome;
    }
    let Some(owner) = ctx.permission_scope.owner_session_id() else {
        return error("Native browser actions require a canonical conversation owner".into());
    };
    let mut backend_args = args.clone();
    backend_args.insert("nola_owner_id".into(), json!(canonical_managed_browser_session_id(&owner)));
    if let Some(expected) = current_browser_permission_resource_expectation() {
        backend_args.insert("_nola_permission_resource".into(), expected);
    }
    let result = match run_chrome_native_action(action, &backend_args) {
        Ok(result) => result,
        Err(message) => return error(message),
    };
    format_chrome_native_result(action, result, ctx, true)
}

struct ManagedBrowserBridge {
    child: std::process::Child,
    rpc_url: String,
    token: String,
}

static MANAGED_BROWSER_BRIDGE: std::sync::OnceLock<std::sync::Mutex<Option<ManagedBrowserBridge>>> =
    std::sync::OnceLock::new();

fn browser_target(args: &Map<String, Value>) -> Result<&str, ToolOutcome> {
    let target = args.get("target").and_then(Value::as_str).unwrap_or("managed");
    matches!(target, "managed" | "attached")
        .then_some(target)
        .ok_or_else(|| error("browser target must be managed or attached".to_string()))
}

fn browser_action(name: &str) -> Option<&'static str> {
    match name {
        "browser_status" => Some("status"),
        "browser_start" => Some("start"),
        "browser_stop" => Some("stop"),
        "browser_snapshot" => Some("snapshot"),
        "browser_navigate" => Some("navigate"),
        "browser_click" => Some("click"),
        "browser_type_text" => Some("type_text"),
        "browser_screenshot" => Some("screenshot"),
        "browser_tabs" => Some("tabs"),
        "browser_switch_page" => Some("switch_page"),
        _ => None,
    }
}

fn run_browser_tool(name: &str, args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let Some(action) = browser_action(name) else { return error(format!("Unknown tool '{name}'")); };
    let target = match browser_target(args) {
        Ok(value) => value,
        Err(outcome) => return outcome,
    };
    if let Err(outcome) = validate_unified_browser_args(action, target, args) { return outcome; }
    let mut backend_args = args.clone();
    backend_args.remove("target");
    let Some(owner) = ctx.permission_scope.owner_session_id() else {
        return error("Browser actions require a canonical conversation owner".into());
    };
    if target == "managed" {
        // Browser identity belongs to the conversation, not to model prose.
        // Reusing one canonical id prevents follow-up turns from opening a
        // second window merely because the model invented a new session name.
        backend_args.insert(
            "session_id".to_string(),
            Value::String(canonical_managed_browser_session_id(&owner)),
        );
        backend_args.insert("persistent".to_string(), Value::Bool(true));
        // Nola owns the visible browser surface. Playwright runs without a
        // separate desktop window and publishes its viewport to the client.
        backend_args.insert("headless".to_string(), Value::Bool(true));
    } else if target == "attached" {
        backend_args.insert(
            "nola_owner_id".to_string(),
            Value::String(canonical_managed_browser_session_id(&owner)),
        );
    }
    let backend_action = match (target, action) {
        ("attached", "status" | "start") => "status",
        ("attached", "stop") => return error("Nola never closes the user's attached Chrome browser".to_string()),
        ("attached", "snapshot") => "inspect",
        ("attached", "navigate") => "open_page",
        ("attached", "tabs") => "list_pages",
        ("attached", "switch_page") => "switch_page",
        (_, value) => value,
    };
    if target == "attached" {
        if action == "navigate" {
            backend_args.insert("new_tab".into(), json!(args.get("new_page").and_then(Value::as_bool).unwrap_or(false)));
        }
        if action == "switch_page" {
            let index = args.get("index").and_then(Value::as_u64).ok_or_else(|| error("browser page index is missing".into()));
            let index = match index { Ok(index) => index, Err(outcome) => return outcome };
            let Some(index) = index.checked_add(1) else { return error("browser page index is too large".into()) };
            backend_args.insert("index".into(), json!(index));
        }
        for key in ["session_id", "persistent", "headless", "locale", "revision", "ref", "full_page", "max_elements", "timeout_ms", "clear", "delay_ms", "new_page"] {
            backend_args.remove(key);
        }
        if action == "type_text" { backend_args.insert("clear".into(), json!(args.get("clear").and_then(Value::as_bool).unwrap_or(true))); }
    }
    if let Some(expected) = current_browser_permission_resource_expectation() {
        backend_args.insert("_nola_permission_resource".into(), expected);
    }
    let result = if target == "attached" {
        run_chrome_native_action(backend_action, &backend_args)
    } else {
        run_managed_browser_action(backend_action, &backend_args, &ctx.browser.state_dir)
    };
    match result {
        Ok(mut result) => {
            if target == "attached" {
                if let Some(pages) = result.get_mut("pages").and_then(Value::as_array_mut) {
                    for page in pages {
                        if let Some(index) = page.get("index").and_then(Value::as_u64).and_then(|index| index.checked_sub(1)) {
                            page["index"] = json!(index);
                        }
                    }
                }
                if let Some(index) = result.get("active_index").and_then(Value::as_u64) {
                    result.insert("active_index".into(), json!(index.checked_sub(1)));
                }
                result.insert("page_index_base".into(), json!(0));
            }
            format_chrome_native_result(backend_action, result, ctx, target == "attached")
        }
        Err(message) => error(message),
    }
}

fn canonical_managed_browser_session_id(owner: &str) -> String {
    // Replacing punctuation is lossy: distinct opaque conversation IDs can
    // otherwise control the same persistent browser session.
    format!("nola-{}", nola_core::mobile_crypto::mobile_body_sha256(owner.as_bytes()))
}

pub(crate) fn managed_browser_pages_for_conversation(
    conversation_id: &str,
    page_index: Option<u64>,
) -> Result<Map<String, Value>, String> {
    let session_id = canonical_managed_browser_session_id(conversation_id);
    let mut args = Map::new();
    args.insert("session_id".to_string(), Value::String(session_id.clone()));
    if let Some(index) = page_index {
        args.insert("index".to_string(), Value::Number(index.into()));
    }
    let state_dir = env::var_os("NOLA_PLAYWRIGHT_STATE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| env::temp_dir().join("nola-browser"));
    let status = run_managed_browser_action("status", &args, &state_dir)?;
    let existing = status.get("sessions").and_then(Value::as_array)
        .is_some_and(|sessions| sessions.iter().any(|session| {
            session.get("session_id").and_then(Value::as_str) == Some(session_id.as_str())
                && session.get("running").and_then(Value::as_bool) == Some(true)
        }));
    if !existing {
        return Err("Managed browser session is not running for this conversation".to_string());
    }
    let result = run_managed_browser_action(
        if page_index.is_some() { "switch_page" } else { "tabs" },
        &args,
        &state_dir,
    )?;
    if result.get("session_id").and_then(Value::as_str)
        .is_some_and(|returned| returned != session_id)
    {
        return Err("Managed browser returned a different conversation session".to_string());
    }
    Ok(result)
}

#[cfg(test)]
mod managed_browser_session_identity_tests {
    use super::{canonical_managed_browser_session_id, managed_control_endpoint_is_local, specs};

    #[test]
    fn child_control_endpoint_cannot_redirect_credentials_or_escape_loopback() {
        assert!(managed_control_endpoint_is_local("http://127.0.0.1:32145/rpc"));
        for endpoint in [
            "https://127.0.0.1:32145/rpc", "http://localhost:32145/rpc",
            "http://192.168.31.18:32145/rpc", "http://127.0.0.1:32145/other",
            "http://user@127.0.0.1:32145/rpc", "http://127.0.0.1:32145/rpc?token=secret",
            "http://127.0.0.1:32145/rpc#other", "not a URL",
        ] { assert!(!managed_control_endpoint_is_local(endpoint), "{endpoint}"); }
    }

    #[test]
    fn model_supplied_browser_names_cannot_change_conversation_session_identity() {
        let owner = "conversation:64D1F162-79A2-44C0-BC86-BB576305BCF8";
        let first = canonical_managed_browser_session_id(owner);
        let follow_up = canonical_managed_browser_session_id(owner);

        assert_eq!(first, follow_up);
        assert!(first.starts_with("nola-"));
        assert_eq!(first.len(), "nola-".len() + 64);
        assert_ne!(
            canonical_managed_browser_session_id("conversation:a/b"),
            canonical_managed_browser_session_id("conversation:a-b")
        );
    }

    #[test]
    fn browser_identity_is_not_exposed_to_the_model() {
        let browser_specs = specs()
            .into_iter()
            .filter(|spec| spec.name.starts_with("browser_") && spec.name != "browser_status")
            .collect::<Vec<_>>();
        assert!(!browser_specs.is_empty());
        for spec in browser_specs {
            let properties = spec.parameters["properties"]
                .as_object()
                .expect("browser tool properties");
            assert!(
                !properties.contains_key("session_id"),
                "{} must use runtime-owned browser identity",
                spec.name
            );
            let requires_session = spec.parameters["required"]
                .as_array()
                .is_some_and(|items| items.iter().any(|item| item == "session_id"));
            assert!(!requires_session, "{} must not require session_id", spec.name);
        }
    }
}

fn validate_unified_browser_args(action: &str, target: &str, args: &Map<String, Value>) -> Result<(), ToolOutcome> {
    if action == "navigate" {
        validate_browser_page_url(args.get("url").and_then(Value::as_str).unwrap_or(""))?;
    }
    if matches!(action, "click" | "type_text") {
        let element_ref = args.get("ref").and_then(Value::as_str).filter(|value| !value.trim().is_empty());
        let selector = args.get("selector").and_then(Value::as_str).filter(|value| !value.trim().is_empty());
        let text_target = (action == "click").then(|| args.get("text").and_then(Value::as_str)).flatten().filter(|value| !value.trim().is_empty());
        if target == "managed" && (element_ref.is_none()
            || !args.get("revision").and_then(Value::as_u64).is_some_and(|revision| revision > 0)) {
            return Err(error("managed browser actions require ref and revision from the latest browser_snapshot".to_string()));
        }
        if target == "attached" && selector.is_none() && text_target.is_none() {
            return Err(error("attached Chrome actions require an exact selector or unique text target".to_string()));
        }
    }
    if action == "type_text" && args.get("text").and_then(Value::as_str).is_none() {
        return Err(error("browser_type_text requires text".to_string()));
    }
    if action == "switch_page" {
        let index = args.get("index").and_then(Value::as_i64)
            .ok_or_else(|| error("browser_switch_page requires a non-negative integer index".to_string()))?;
        if index < 0 {
            return Err(error("browser_switch_page index must be non-negative".to_string()));
        }
    }
    for key in ["screenshot", "full_page", "persistent", "headless", "clear", "new_page"] {
        validate_boolean_arg(args, key)?;
    }
    Ok(())
}

fn start_managed_browser_bridge(state_dir: &Path) -> Result<ManagedBrowserBridge, String> {
    let node = env::var("NOLA_PLAYWRIGHT_NODE").map(PathBuf::from)
        .map_err(|_| "Nola's bundled Playwright runtime is unavailable".to_string())?;
    let bridge_script = env::var("NOLA_PLAYWRIGHT_BRIDGE").map(PathBuf::from)
        .map_err(|_| "Nola's bundled Playwright bridge is unavailable".to_string())?;
    if !node.is_file() || !bridge_script.is_file() {
        return Err("Nola's bundled Playwright runtime is incomplete; rebuild the Mac app".to_string());
    }
    std::fs::create_dir_all(state_dir).map_err(|err| format!("Managed browser state directory failed: {err}"))?;
    let control_file = state_dir.join(format!(".runtime-control-{}.json", uuid::Uuid::new_v4()));
    let mut child = std::process::Command::new(node)
        .arg(bridge_script)
        .env("NOLA_PLAYWRIGHT_STATE_ROOT", state_dir)
        .env("NOLA_PLAYWRIGHT_CONTROL_FILE", &control_file)
        .env("NOLA_PLAYWRIGHT_PARENT_PID", std::process::id().to_string())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|err| format!("Failed to start Nola's managed browser runtime: {err}"))?;
    #[derive(serde::Deserialize)]
    struct Control { pid: u32, rpc_url: String, token: String }
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    let ready = (|| -> Result<Control, String> { loop {
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            return Err("Managed browser runtime exited before becoming ready".into());
        }
        if let Ok(bytes) = std::fs::read(&control_file) {
            let control: Control = serde_json::from_slice(&bytes)
                .map_err(|_| "Managed browser control identity is invalid".to_string())?;
            if control.pid != child.id() || control.token.is_empty()
                || !managed_control_endpoint_is_local(&control.rpc_url) {
                return Err("Managed browser control owner differs".into());
            }
            return Ok(control);
        }
        if std::time::Instant::now() >= deadline {
            return Err("Managed browser runtime readiness deadline exceeded".into());
        }
        std::thread::sleep(Duration::from_millis(25));
    } })();
    match ready {
        Ok(control) => Ok(ManagedBrowserBridge { child, rpc_url: control.rpc_url, token: control.token }),
        Err(error) => {
            // Readiness has not admitted any action. Every failure path owns
            // and reaps this child, including malformed control metadata.
            let _ = child.kill();
            let _ = child.wait();
            Err(error)
        }
    }
}

fn managed_control_endpoint_is_local(endpoint: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(endpoint) else { return false; };
    url.scheme() == "http" && url.host_str() == Some("127.0.0.1")
        && url.port().is_some() && url.path() == "/rpc"
        && url.username().is_empty() && url.password().is_none()
        && url.query().is_none() && url.fragment().is_none()
}

fn run_managed_browser_action(action: &str, args: &Map<String, Value>, state_dir: &Path) -> Result<Map<String, Value>, String> {
    let (endpoint, token) = match (
        env::var("NOLA_PLAYWRIGHT_RPC_URL"),
        env::var("NOLA_PLAYWRIGHT_RPC_TOKEN"),
    ) {
        (Ok(endpoint), Ok(token)) => (endpoint, token),
        (Err(_), Err(_)) => {
            // Startup is serialized; actual owner operations never hold this
            // process-wide mutex or wait on an unbounded child read_line.
            let mutex = MANAGED_BROWSER_BRIDGE.get_or_init(|| std::sync::Mutex::new(None));
            let mut slot = mutex.lock().map_err(|_| "Managed browser runtime lock failed".to_string())?;
            let needs_start = slot.as_mut().is_none_or(|bridge| bridge.child.try_wait().ok().flatten().is_some());
            if needs_start { *slot = Some(start_managed_browser_bridge(state_dir)?); }
            let bridge = slot.as_ref().ok_or("Managed browser runtime did not start")?;
            (bridge.rpc_url.clone(), bridge.token.clone())
        }
        _ => return Err("Managed browser RPC configuration requires both endpoint and credential".into()),
    };
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|err| format!("Managed browser client failed: {err}"))?;
    let response = client
        .post(endpoint)
        .header("X-Nola-Playwright-Token", token)
        .json(&json!({"action": action, "args": args}))
        .send()
        .map_err(|err| format!("Managed browser request failed: {err}"))?;
    let status = response.status();
    let payload: Value = response
        .json()
        .map_err(|err| format!("Managed browser returned invalid data: {err}"))?;
    if !status.is_success() {
        return Err(payload.get("error").and_then(Value::as_str)
            .unwrap_or("Managed browser request failed").to_string());
    }
    payload.get("result").and_then(Value::as_object).cloned()
        .ok_or_else(|| "Managed browser response has no result".to_string())
}

pub(super) fn run_chrome_native_action(
    action: &str,
    args: &Map<String, Value>,
) -> Result<Map<String, Value>, String> {
    let cfg = native_browser_bridge_config_from_env().map_err(|outcome| outcome.text)?;
    let mut body = json!({
        "action": action,
        "args": args,
        "timeout_sec": chrome_native_timeout_sec(action, args),
        "local_user_id": cfg.local_user_id.clone(),
    });
    if !cfg.host_id.is_empty() {
        body["host_id"] = Value::String(cfg.host_id.clone());
    }
    let endpoint = format!(
        "{}/api/native-browser/command",
        cfg.base_url.trim_end_matches('/')
    );
    let client = match reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(130))
        .build()
    {
        Ok(client) => client,
        Err(err) => return Err(format!("Native Chrome client failed: {err}")),
    };
    let response = match client
        .post(endpoint)
        .header("X-Nola-Native-Browser-Token", cfg.token)
        .header("X-Nola-Local-User-Id", cfg.local_user_id)
        .json(&body)
        .send()
    {
        Ok(response) => response,
        Err(err) => return Err(format!("Native Chrome request failed: {err}")),
    };
    let status = response.status();
    let text = match response.text() {
        Ok(text) => text,
        Err(err) => return Err(format!("Native Chrome response read failed: {err}")),
    };
    let parsed = match serde_json::from_str::<Value>(&text) {
        Ok(value) => value,
        Err(_) => {
            return Err(format!(
                "Native Chrome request failed ({status}): {}",
                truncate(&text, 500)
            ))
        }
    };
    if !status.is_success() {
        let detail = parsed
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("Native Chrome request failed");
        return Err(format!("Native Chrome request failed ({status}): {detail}"));
    }
    let result = parsed
        .get("result")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if !result.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        let detail = result
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("Native Chrome command failed");
        return Err(format!("Native Chrome command failed: {detail}"));
    }
    Ok(result)
}

fn chrome_native_action(name: &str) -> Option<&'static str> {
    match name {
        "chrome_native_status" => Some("status"),
        "chrome_native_inspect" => Some("inspect"),
        "chrome_native_screenshot" => Some("screenshot"),
        "chrome_native_dom_snapshot" => Some("dom_snapshot"),
        "chrome_native_accessibility_snapshot" => Some("accessibility_snapshot"),
        "chrome_native_storage_state" => Some("storage_state"),
        "chrome_native_console_logs" => Some("console_logs"),
        "chrome_native_network_events" => Some("network_events"),
        "chrome_native_list_pages" => Some("list_pages"),
        "chrome_native_switch_page" => Some("switch_page"),
        "chrome_native_open_page" => Some("open_page"),
        "chrome_native_hover" => Some("hover"),
        "chrome_native_click" => Some("click"),
        "chrome_native_drag" => Some("drag"),
        "chrome_native_type_text" => Some("type_text"),
        "chrome_native_select_option" => Some("select_option"),
        "chrome_native_set_checked" => Some("set_checked"),
        "chrome_native_press_key" => Some("press_key"),
        "chrome_native_scroll" => Some("scroll"),
        "chrome_native_assert" => Some("assert"),
        "chrome_native_wait_for" => Some("wait_for"),
        _ => None,
    }
}

fn native_browser_bridge_config_from_env() -> Result<NativeBrowserBridgeConfig, ToolOutcome> {
    let persisted = env::var("HOME")
        .ok()
        .map(|home| std::path::PathBuf::from(home).join(".nola/native_browser_host.json"))
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    let persisted_string = |key: &str| {
        persisted
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_default()
    };
    let environment_or_persisted = |environment_name: &str, persisted_key: &str| {
        let environment_value = first_nonempty_env(&[environment_name]);
        if environment_value.is_empty() {
            persisted_string(persisted_key)
        } else {
            environment_value
        }
    };
    let base_url = environment_or_persisted("NOLA_NATIVE_BROWSER_SERVER_URL", "server_url");
    let token = environment_or_persisted("NOLA_NATIVE_BROWSER_TOKEN", "token");
    let local_user_id = {
        let value = environment_or_persisted("NOLA_NATIVE_BROWSER_LOCAL_USER_ID", "local_user_id");
        if value.is_empty() {
            "cli-local-user".to_string()
        } else {
            value
        }
    };
    let host_id = environment_or_persisted("NOLA_NATIVE_BROWSER_HOST_ID", "host_id");
    let missing = [
        ("NOLA_NATIVE_BROWSER_SERVER_URL", base_url.as_str()),
        ("NOLA_NATIVE_BROWSER_TOKEN", token.as_str()),
        ("NOLA_NATIVE_BROWSER_LOCAL_USER_ID", local_user_id.as_str()),
    ]
    .into_iter()
    .filter_map(|(name, value)| if value.is_empty() { Some(name) } else { None })
    .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(error(format!(
            "the browser connector is not configured: missing {} (set environment values or install the native browser host)",
            missing.join(", ")
        )));
    }
    let base_url = match normalize_web_host_base_url(&base_url) {
        Ok(value) => value,
        Err(message) => return Err(error(message)),
    };
    if !is_safe_bridge_id(&local_user_id) {
        return Err(error(
            "local user id contains invalid characters".to_string(),
        ));
    }
    if !host_id.is_empty() && !is_safe_bridge_id(&host_id) {
        return Err(error(
            "native browser host id contains invalid characters".to_string(),
        ));
    }
    Ok(NativeBrowserBridgeConfig {
        base_url,
        token,
        local_user_id,
        host_id,
    })
}

fn normalize_web_host_base_url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    let mut url = reqwest::Url::parse(trimmed).map_err(|_| {
        "NOLA_NATIVE_BROWSER_SERVER_URL must be an absolute http(s) URL".to_string()
    })?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("NOLA_NATIVE_BROWSER_SERVER_URL must be an absolute http(s) URL".to_string());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("NOLA_NATIVE_BROWSER_SERVER_URL credentials are not allowed".to_string());
    }
    url.set_query(None);
    url.set_fragment(None);
    let normalized_path = strip_web_host_api_suffix(url.path());
    url.set_path(&normalized_path);
    Ok(url.to_string().trim_end_matches('/').to_string())
}

fn strip_web_host_api_suffix(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() || trimmed == "/" {
        return String::new();
    }
    let lower = trimmed.to_ascii_lowercase();
    for marker in ["/api/mobile", "/api"] {
        if let Some(index) = lower.rfind(marker) {
            let tail = &lower[index..];
            if tail == marker || tail.as_bytes().get(marker.len()) == Some(&b'/') {
                return trimmed[..index].trim_end_matches('/').to_string();
            }
        }
    }
    trimmed.to_string()
}

fn first_nonempty_env(names: &[&str]) -> String {
    names
        .iter()
        .filter_map(|name| env::var(name).ok())
        .map(|value| value.trim().to_string())
        .find(|value| !value.is_empty())
        .unwrap_or_default()
}

fn is_safe_bridge_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 120
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | ':' | '-'))
}

fn validate_chrome_native_tool_args(
    action: &str,
    args: &Map<String, Value>,
) -> Result<(), ToolOutcome> {
    match action {
        "status" | "screenshot" | "list_pages" => {
            if !args.is_empty() {
                return Err(error(format!(
                    "chrome_native_{action} does not accept arguments"
                )));
            }
        }
        "inspect" => validate_browser_tool_args("browser_inspect", args)?,
        "dom_snapshot" => validate_browser_tool_args("browser_dom_snapshot", args)?,
        "accessibility_snapshot" => {
            validate_browser_tool_args("browser_accessibility_snapshot", args)?
        }
        "storage_state" => validate_browser_tool_args("browser_storage_state", args)?,
        "console_logs" => validate_browser_tool_args("browser_console_logs", args)?,
        "network_events" => validate_browser_tool_args("browser_network_events", args)?,
        "switch_page" => validate_chrome_native_switch_page_args(args)?,
        "open_page" => {
            let url = required_string(args, "url")?;
            validate_chrome_native_url(&url)?;
            validate_boolean_args(args, &["new_tab", "screenshot"])?;
            validate_integer_arg(args, "max_text_chars", 8000, 500, 40_000)?;
        }
        "hover" => validate_browser_tool_args("browser_hover", args)?,
        "click" => {
            let selector = optional_nonempty_string(args, "selector");
            let text = optional_nonempty_string(args, "text");
            let coords = validate_optional_coordinates(args)?;
            let targets = [selector.is_some(), text.is_some(), coords]
                .into_iter()
                .filter(|value| *value)
                .count();
            if targets != 1 {
                return Err(error(
                    "provide exactly one target: selector, text, or x+y".to_string(),
                ));
            }
            validate_optional_enum(args, "button", &["left", "right", "middle"])?;
            validate_integer_arg(args, "clicks", 1, 1, 3)?;
            validate_optional_browser_click_modifiers(args)?;
            validate_integer_arg(args, "wait_after_ms", 500, 0, 5000)?;
            validate_boolean_arg(args, "screenshot")?;
        }
        "drag" => validate_browser_tool_args("browser_drag", args)?,
        "type_text" => validate_browser_tool_args("browser_type_text", args)?,
        "select_option" => validate_browser_tool_args("browser_select_option", args)?,
        "set_checked" => validate_browser_tool_args("browser_set_checked", args)?,
        "press_key" => validate_browser_tool_args("browser_press_key", args)?,
        "scroll" => validate_browser_tool_args("browser_scroll", args)?,
        "assert" => validate_browser_tool_args("browser_assert", args)?,
        "wait_for" => validate_browser_tool_args("browser_wait_for", args)?,
        _ => return Err(error("native Chrome action is not supported".to_string())),
    }
    Ok(())
}

fn validate_chrome_native_switch_page_args(args: &Map<String, Value>) -> Result<(), ToolOutcome> {
    let has_index = has_non_empty_arg(args, "index");
    let target_text = |key: &str, limit: usize| -> Result<Option<String>, ToolOutcome> {
        let Some(value) = args.get(key) else {
            return Ok(None);
        };
        if value.is_null() {
            return Ok(None);
        }
        let Some(text) = value.as_str() else {
            return Err(error(format!("{key} is invalid")));
        };
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }
        if trimmed.chars().count() > limit
            || trimmed.contains('\0')
            || trimmed.contains('\n')
            || trimmed.contains('\r')
        {
            return Err(error(format!("{key} is invalid")));
        }
        Ok(Some(trimmed.to_owned()))
    };
    let title = target_text("title", 500)?;
    let url = target_text("url", 2000)?;
    let targets = [has_index, title.is_some(), url.is_some()]
        .into_iter()
        .filter(|value| *value)
        .count();
    if targets != 1 {
        return Err(error(
            "provide exactly one switch target: index, title, or url".to_string(),
        ));
    }
    if has_index {
        validate_integer_arg(args, "index", 1, 1, 200)?;
    }
    validate_boolean_arg(args, "screenshot")?;
    Ok(())
}

fn validate_chrome_native_url(raw_url: &str) -> Result<(), ToolOutcome> {
    let url = reqwest::Url::parse(raw_url)
        .map_err(|_| error("url must be an absolute http(s) URL".to_string()))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(error(
            "native Chrome navigation only supports http(s) URLs".to_string(),
        ));
    }
    if url
        .host_str()
        .map(str::trim)
        .filter(|host| !host.is_empty())
        .is_none()
    {
        return Err(error("URL host is required".to_string()));
    }
    Ok(())
}

fn chrome_native_timeout_sec(action: &str, args: &Map<String, Value>) -> f64 {
    if action == "wait_for" {
        let timeout_ms = args
            .get("timeout_ms")
            .and_then(Value::as_i64)
            .unwrap_or(5_000)
            .clamp(0, 60_000) as f64;
        return ((timeout_ms / 1000.0) + 15.0).max(45.0);
    }
    45.0
}

fn format_chrome_native_result(
    action: &str,
    mut result: Map<String, Value>,
    ctx: &ToolContext,
    attached: bool,
) -> ToolOutcome {
    if !result.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        let error_text = result.get("error").and_then(|error| error.as_str()
            .or_else(|| error.get("message").and_then(Value::as_str)))
            .unwrap_or("Browser command failed").to_string();
        let mut outcome = error_data_with_display(Value::Object(result), error_text);
        let not_started = outcome.data.as_ref().and_then(|data| data.pointer("/action/status")).and_then(Value::as_str) == Some("not_started");
        outcome.execution = Some(nola_host_protocol::tool_result::ExecutionReceipt {
            state: if not_started { nola_host_protocol::tool_result::ExecutionState::NotStarted }
                else { nola_host_protocol::tool_result::ExecutionState::Unknown },
            ..Default::default()
        });
        return outcome;
    }
    if matches!(action, "list_pages" | "tabs") {
        return format_browser_pages(result, attached);
    }
    if action == "console_logs" || action == "network_events" {
        return format_chrome_native_diagnostics(action, result);
    }

    let screenshot_path = result
        .remove("screenshot_data_url")
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .and_then(|data_url| save_chrome_native_screenshot(ctx, &data_url).ok());
    let final_url = result
        .get("final_url")
        .or_else(|| result.get("url"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let title = result
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("(untitled)")
        .to_string();
    let (action_prefix, browser_label) = browser_result_identity(attached);
    let mut lines = vec![
        format!("Action: {action_prefix}_{action}"),
        format!("Final URL: {final_url}"),
        format!("Title: {title}"),
        format!("Browser: {browser_label}"),
    ];
    if let Some(path) = screenshot_path.as_ref() {
        lines.push(format!("Screenshot: {path}"));
    }
    if let Some(message) = result.get("message").and_then(Value::as_str) {
        if !message.trim().is_empty() {
            lines.push(format!("Message: {}", truncate(message.trim(), 1000)));
        }
    }
    if let Some(condition) = result.get("condition").and_then(Value::as_object) {
        lines.extend(chrome_native_condition_lines(condition));
    }
    if let Some(visible_text) = result.get("visible_text").and_then(Value::as_str) {
        lines.push(String::new());
        lines.push("Visible text:".to_string());
        lines.push(truncate(visible_text, 40_000));
    }
    if let Some(elements) = result.get("interactive_elements").and_then(Value::as_array) {
        lines.push(String::new());
        lines.push("Interactive elements:".to_string());
        lines.push(format_chrome_native_interactive_elements(elements));
    }
    if !matches!(
        action,
        "status"
            | "inspect"
            | "screenshot"
            | "open_page"
            | "hover"
            | "click"
            | "drag"
            | "type_text"
            | "select_option"
            | "set_checked"
            | "press_key"
            | "scroll"
            | "switch_page"
            | "assert"
            | "wait_for"
    ) {
        lines.push(String::new());
        lines.push("Result JSON:".to_string());
        lines.push(
            serde_json::to_string_pretty(&Value::Object(result.clone()))
                .unwrap_or_else(|_| "{}".to_string()),
        );
    }

    let display = format!("{action_prefix}_{action}: {title}");
    let outcome = structured_browser_outcome(result, lines.join("\n"), display);
    if let Some(path) = screenshot_path {
        outcome.with_file_artifact(Path::new(&path), "observe")
    } else {
        outcome
    }
}

fn structured_browser_outcome(
    result: Map<String, Value>,
    human_text: String,
    display: String,
) -> ToolOutcome {
    // Presentation is not serialized back into the business data. Keeping a
    // second text/JSON rendering here doubled browser evidence in every turn.
    ok(human_text, display).with_data(Value::Object(result))
}

fn browser_result_identity(attached: bool) -> (&'static str, &'static str) {
    if attached {
        ("chrome_native", "Chrome extension connected through Nola for Mac")
    } else {
        ("browser_managed", "Nola-managed Playwright browser")
    }
}

#[cfg(test)]
mod browser_result_identity_tests {
    use super::browser_result_identity;

    #[test]
    fn managed_and_attached_results_have_distinct_truthful_identity() {
        assert_eq!(
            browser_result_identity(false),
            ("browser_managed", "Nola-managed Playwright browser")
        );
        assert_eq!(
            browser_result_identity(true),
            ("chrome_native", "Chrome extension connected through Nola for Mac")
        );
    }
}

fn format_browser_pages(result: Map<String, Value>, attached: bool) -> ToolOutcome {
    let pages = result
        .get("pages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let active_index = result.get("active_index").and_then(Value::as_i64)
        .or_else(|| pages.iter().find(|page| page["active"].as_bool() == Some(true))?.get("index")?.as_i64());
    let zero_based = !attached || result.get("page_index_base").and_then(Value::as_i64) == Some(0);
    let page_count = result
        .get("page_count")
        .and_then(Value::as_i64)
        .unwrap_or(pages.len() as i64);
    let mut lines = vec![
        if attached { "Attached Chrome pages:" } else { "Managed browser pages:" }.to_string(),
        format!(
            "Active index: {}",
            active_index.filter(|index| zero_based || *index > 0).map(|index| index.to_string()).unwrap_or_else(|| "(none)".into())
        ),
        format!("Page count: {page_count}"),
    ];
    if pages.is_empty() {
        lines.push("(none)".to_string());
    }
    for page in pages.iter().take(100) {
        let Some(item) = page.as_object() else {
            continue;
        };
        let index = item.get("index").and_then(Value::as_i64).unwrap_or(0);
        let marker = if item.get("active").and_then(Value::as_bool).unwrap_or(false) {
            "*"
        } else {
            " "
        };
        let title = item
            .get("title")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("(untitled)");
        let url = item.get("url").and_then(Value::as_str).unwrap_or("");
        let pinned = if item.get("pinned").and_then(Value::as_bool).unwrap_or(false) {
            " pinned"
        } else {
            ""
        };
        lines.push(format!(
            "{marker} {index}. {}{pinned}\n   URL: {}",
            truncate(title, 160),
            truncate(url, 2_000)
        ));
    }
    if pages.len() > 100 {
        lines.push(format!("... truncated at 100 of {} page(s)", pages.len()));
    }
    structured_browser_outcome(
        result,
        lines.join("\n"),
        format!("{}_list_pages: {page_count} pages", browser_result_identity(attached).0),
    )
}

fn format_chrome_native_diagnostics(action: &str, result: Map<String, Value>) -> ToolOutcome {
    let events = result
        .get("events")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let matching_count = result
        .get("matching_count")
        .and_then(Value::as_i64)
        .unwrap_or(events.len() as i64);
    let total_count = result
        .get("total_count")
        .and_then(Value::as_i64)
        .unwrap_or(events.len() as i64);
    let level =
        result
            .get("level")
            .and_then(Value::as_str)
            .unwrap_or(if action == "network_events" {
                "warnings"
            } else {
                "all"
            });
    let title = if action == "network_events" {
        "Native Chrome network events"
    } else {
        "Native Chrome console/page errors"
    };
    let mut lines = vec![format!(
        "{title}: showing {} of {matching_count} matching event(s), {total_count} total, level={level}",
        events.len()
    )];
    if events.is_empty() {
        lines.push("(no matching native Chrome diagnostic events captured)".to_string());
    }
    for (index, event) in events.iter().take(100).enumerate() {
        let Some(item) = event.as_object() else {
            continue;
        };
        if action == "network_events" {
            let kind = item
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("response");
            let method = item.get("method").and_then(Value::as_str).unwrap_or("GET");
            let resource_type = item
                .get("resource_type")
                .and_then(Value::as_str)
                .unwrap_or("resource");
            let url = item.get("url").and_then(Value::as_str).unwrap_or("");
            let status = item
                .get("status")
                .filter(|value| !value.is_null())
                .map(|value| value.to_string())
                .unwrap_or_default();
            let status_part = if status.is_empty() {
                String::new()
            } else {
                format!(" status={status}")
            };
            let failure = item.get("failure").and_then(Value::as_str).unwrap_or("");
            let failure_part = if failure.is_empty() {
                String::new()
            } else {
                format!(" failure={}", truncate(failure, 500))
            };
            lines.push(format!(
                "{}. [{kind}] {method} {resource_type}{status_part}{failure_part} {}",
                index + 1,
                truncate(url, 2_000)
            ));
        } else {
            let kind = item
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("console");
            let event_level = item.get("level").and_then(Value::as_str).unwrap_or("log");
            let text = item.get("text").and_then(Value::as_str).unwrap_or("");
            let source = item
                .get("source_url")
                .or_else(|| item.get("url"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let location = if source.is_empty() {
                String::new()
            } else {
                format!(" ({})", truncate(source, 500))
            };
            lines.push(format!(
                "{}. [{kind}/{event_level}]{location} {}",
                index + 1,
                truncate(text, 2_000)
            ));
        }
    }
    ok(
        lines.join("\n"),
        format!("chrome_native_{action}: {matching_count} matching events"),
    )
}

fn chrome_native_condition_lines(condition: &Map<String, Value>) -> Vec<String> {
    let kind = condition
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("condition");
    let expected = condition
        .get("expected")
        .and_then(Value::as_str)
        .unwrap_or("");
    let found = condition
        .get("found")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    vec![
        format!("Condition: {kind} `{expected}`"),
        format!("Condition found: {found}"),
    ]
}

fn format_chrome_native_interactive_elements(elements: &[Value]) -> String {
    if elements.is_empty() {
        return "(none)".to_string();
    }
    elements
        .iter()
        .take(120)
        .enumerate()
        .filter_map(|(index, item)| {
            let item = item.as_object()?;
            let selector = item.get("selector").and_then(Value::as_str).unwrap_or("");
            let text = item
                .get("text")
                .or_else(|| item.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let role = item.get("role").and_then(Value::as_str).unwrap_or("");
            Some(format!(
                "{}. role={} text={} selector={}",
                index + 1,
                truncate(role, 80),
                truncate(text, 140),
                truncate(selector, 220)
            ))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn save_chrome_native_screenshot(ctx: &ToolContext, data_url: &str) -> Result<String, String> {
    let prefix = "data:image/png;base64,";
    let Some(encoded) = data_url.strip_prefix(prefix) else {
        return Err("screenshot data URL is invalid".to_string());
    };
    if encoded.len() > ((MAX_NATIVE_BROWSER_SCREENSHOT_BYTES + 2) / 3) * 4 {
        return Err("native Chrome screenshot is too large".to_string());
    }
    let raw = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|err| format!("native Chrome screenshot decode failed: {err}"))?;
    if raw.len() > MAX_NATIVE_BROWSER_SCREENSHOT_BYTES {
        return Err("native Chrome screenshot is too large".to_string());
    }
    let dir = ctx.browser.state_dir.join("native_chrome");
    fs::create_dir_all(&dir)
        .map_err(|err| format!("native Chrome screenshot dir failed: {err}"))?;
    let filename = format!(
        "{}.png",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or(0)
    );
    let path = dir.join(filename);
    fs::write(&path, raw).map_err(|err| format!("native Chrome screenshot save failed: {err}"))?;
    Ok(path.display().to_string())
}

fn run_computer_tool(action: &str, args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    if let Err(outcome) = validate_computer_tool_args(action, args) {
        return outcome;
    }
    let prepared_args = tool_try!(prepare_computer_tool_args(action, args, ctx));
    if let Some(outcome) = computer::run_native_computer_tool(action, &prepared_args, ctx) {
        return outcome;
    }
    let payload = json!({
        "action": action,
        "args": prepared_args,
        "state_dir": ctx.browser.state_dir.display().to_string(),
    });
    match run_python_once(
        COMPUTER_HELPER,
        &payload,
        computer_timeout_sec(args),
        &ctx.cancellation,
    ) {
        Ok(outcome) => outcome,
        Err(err) => error(err),
    }
}

fn prepare_computer_tool_args(
    action: &str,
    args: &Map<String, Value>,
    ctx: &ToolContext,
) -> Result<Map<String, Value>, ToolOutcome> {
    let mut prepared = args.clone();
    if computer_tool_uses_output_path(action) {
        prepare_computer_output_path(&mut prepared, ctx)?;
    }
    if action == "computer_open_item" {
        prepare_computer_open_item_target(&mut prepared, ctx)?;
    }
    Ok(prepared)
}

fn computer_tool_uses_output_path(action: &str) -> bool {
    matches!(
        action,
        "computer_observe"
            | "computer_screenshot"
            | "computer_screenshot_element"
            | "computer_screenshot_window"
            | "computer_read_text"
            | "computer_find_text"
            | "computer_move_to_text"
            | "computer_click_text"
            | "computer_assert_text"
            | "computer_wait_for_text"
            | "computer_wait"
    )
}

fn prepare_computer_output_path(
    args: &mut Map<String, Value>,
    ctx: &ToolContext,
) -> Result<(), ToolOutcome> {
    let Some(raw_path) = optional_nonempty_string(args, "path") else {
        return Ok(());
    };
    if raw_path.chars().count() > 4096
        || raw_path.contains('\0')
        || raw_path.contains('\n')
        || raw_path.contains('\r')
    {
        return Err(error("path is invalid".to_string()));
    }
    let target = resolve_allowed(&raw_path, &ctx.allowed_dirs, false)?;
    args.insert(
        "path".to_string(),
        Value::String(target.display().to_string()),
    );
    Ok(())
}

fn prepare_computer_open_item_target(
    args: &mut Map<String, Value>,
    ctx: &ToolContext,
) -> Result<(), ToolOutcome> {
    let target = required_string(args, "target")?;
    let scheme = url_scheme(&target)
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default();
    if matches!(scheme.as_str(), "http" | "https") {
        return Ok(());
    }

    let raw_path = if scheme == "file" {
        browser_file_url_path(&target)?
    } else {
        PathBuf::from(&target)
    };
    let raw_path = raw_path
        .to_str()
        .ok_or_else(|| error("target path must be valid UTF-8".to_string()))?;
    let resolved = resolve_allowed(raw_path, &ctx.allowed_dirs, true)?;
    if !resolved.is_file() && !resolved.is_dir() {
        return Err(error(format!(
            "target path is not a file or directory: {}",
            resolved.display()
        )));
    }
    args.insert(
        "target".to_string(),
        Value::String(resolved.display().to_string()),
    );
    Ok(())
}

struct TemporaryPythonHelper(PathBuf);

impl Drop for TemporaryPythonHelper {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn run_python_once(
    script: &str,
    payload: &Value,
    timeout_sec: u64,
    cancellation: &RuntimeExecutionControl,
) -> Result<ToolOutcome, String> {
    let request = serde_json::to_string(payload).map_err(|err| err.to_string())?;
    let script_stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let script_path = std::env::temp_dir().join(format!(
        "nola-cli-python-{}-{script_stamp}.py",
        std::process::id()
    ));
    fs::write(&script_path, script).map_err(|err| format!("helper script write failed: {err}"))?;
    let _script_guard = TemporaryPythonHelper(script_path.clone());
    let mut last_error = String::new();
    for python in python_candidates() {
        for attempt in 0..2 {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let stdout_path = std::env::temp_dir().join(format!(
                "nola-cli-python-{}-{stamp}.out",
                std::process::id()
            ));
            let stderr_path = std::env::temp_dir().join(format!(
                "nola-cli-python-{}-{stamp}.err",
                std::process::id()
            ));
            let stdout =
                File::create(&stdout_path).map_err(|err| format!("helper stdout failed: {err}"))?;
            let stderr =
                File::create(&stderr_path).map_err(|err| format!("helper stderr failed: {err}"))?;
            let mut command = Command::new(&python);
            command
                .arg("-u")
                .arg(&script_path)
                .stdin(Stdio::piped())
                .stdout(Stdio::from(stdout))
                .stderr(Stdio::from(stderr));
            #[cfg(unix)]
            command.process_group(0);
            let mut child = match command.spawn() {
                Ok(child) => child,
                Err(err) => {
                    last_error = format!("{python}: {err}");
                    let _ = fs::remove_file(&stdout_path);
                    let _ = fs::remove_file(&stderr_path);
                    break;
                }
            };
            if let Some(stdin) = child.stdin.as_mut() {
                if let Err(err) = stdin.write_all(request.as_bytes()) {
                    let _ = terminate_process_tree(&mut child, Duration::from_secs(2));
                    let _ = fs::remove_file(&stdout_path);
                    let _ = fs::remove_file(&stderr_path);
                    return Err(format!("helper write failed: {err}"));
                }
            }
            drop(child.stdin.take());

            let started = Instant::now();
            let status = loop {
                if cancellation.is_requested() {
                    let _ = terminate_process_tree(&mut child, Duration::from_secs(2));
                    let _ = fs::remove_file(&stdout_path);
                    let _ = fs::remove_file(&stderr_path);
                    return Err("Computer helper cancelled by user".to_string());
                }
                match child.try_wait() {
                    Ok(Some(status)) => break status,
                    Ok(None) => {
                        if started.elapsed() > Duration::from_secs(timeout_sec) {
                            let _ = terminate_process_tree(&mut child, Duration::from_secs(2));
                            let _ = fs::remove_file(&stdout_path);
                            let _ = fs::remove_file(&stderr_path);
                            return Err(format!("helper timed out after {timeout_sec}s"));
                        }
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    Err(err) => {
                        let _ = terminate_process_tree(&mut child, Duration::from_secs(2));
                        let _ = fs::remove_file(&stdout_path);
                        let _ = fs::remove_file(&stderr_path);
                        return Err(format!("helper wait failed: {err}"));
                    }
                }
            };
            let stdout = fs::read_to_string(&stdout_path).unwrap_or_default();
            let stderr = fs::read_to_string(&stderr_path).unwrap_or_default();
            let _ = fs::remove_file(&stdout_path);
            let _ = fs::remove_file(&stderr_path);
            if !status.success() {
                last_error = format!(
                    "{python} exited with code {}\n{}",
                    status.code().unwrap_or(-1),
                    truncate(&stderr, 4000)
                );
                continue;
            }
            if stdout.trim().is_empty() {
                last_error = if stderr.trim().is_empty() {
                    format!("{python} exited successfully but returned empty output")
                } else {
                    format!(
                        "{python} exited successfully but returned empty output\n{}",
                        truncate(&stderr, 4000)
                    )
                };
                if attempt == 0 {
                    continue;
                }
                break;
            }
            let value = serde_json::from_str::<Value>(stdout.trim()).map_err(|err| {
                format!(
                    "helper returned invalid JSON: {err}; {}; {}",
                    truncate(&stdout, 4000),
                    truncate(&stderr, 4000)
                )
            })?;
            let ok = value.get("ok").and_then(Value::as_bool).unwrap_or(false);
            let text = value
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let display = value
                .get("display")
                .and_then(Value::as_str)
                .unwrap_or(if ok {
                    "Computer action complete"
                } else {
                    "Computer action failed"
                })
                .to_string();
            return Ok(ToolOutcome {
        full_output_ref: None,
        data: None,
        execution: None,
                text,
                display,
                error: !ok,
                artifacts: Vec::new(),
                activation: None,
            });
        }
    }
    Err(format!(
        "Computer helper failed. Set NOLA_CLI_PYTHON to Python 3. Last error: {last_error}"
    ))
}

fn computer_timeout_sec(args: &Map<String, Value>) -> u64 {
    let explicit = args
        .get("timeout_sec")
        .and_then(Value::as_u64)
        .unwrap_or(20);
    let duration_ms = args.get("duration_ms").and_then(Value::as_u64).unwrap_or(0);
    let wait_after_ms = args
        .get("wait_after_ms")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let wait_timeout_ms = args.get("timeout_ms").and_then(Value::as_u64).unwrap_or(0);
    let derived = duration_ms
        .saturating_add(wait_after_ms)
        .saturating_add(wait_timeout_ms)
        .div_ceil(1000)
        .saturating_add(10);
    explicit.max(derived).clamp(1, 120)
}

fn validate_computer_tool_args(action: &str, args: &Map<String, Value>) -> Result<(), ToolOutcome> {
    match action {
        "conversation_recall" => {
            let query = arg_string(args, "query").unwrap_or_default();
            if query.trim().is_empty() || query.chars().count() > 500 {
                return Err(error("query must contain 1 to 500 characters".to_string()));
            }
            validate_integer_arg(args, "limit", 8, 1, 20)?;
        }
        "computer_status" => {}
        "computer_observe" => {
            for key in [
                "include_screenshot",
                "screenshot",
                "include_accessibility",
                "include_windows",
                "include_apps",
                "include_position",
                "include_values",
                "include_actions",
                "include_state",
                "include_background_apps",
                "include_text",
                "include_ocr_text",
            ] {
                if args.get(key).is_some_and(|value| !value.is_boolean()) {
                    return Err(error(format!("{key} must be a boolean")));
                }
            }
            validate_integer_arg(args, "max_elements", 80, 1, 500)?;
            validate_integer_arg(args, "max_windows", 30, 1, 200)?;
            validate_integer_arg(args, "max_apps", 50, 1, 500)?;
            validate_integer_arg(args, "max_text_results", 80, 1, 500)?;
            validate_float_arg(args, "text_min_confidence", 0.25, 0.0, 1.0)?;
            validate_integer_arg(args, "timeout_sec", 20, 1, 120)?;
            validate_screenshot_crop_args(args)?;
        }
        "computer_list_displays" => {}
        "computer_screenshot" => {
            validate_integer_arg(args, "timeout_sec", 20, 1, 120)?;
            validate_screenshot_crop_args(args)?;
        }
        "computer_screenshot_element" => {
            validate_query_or_index(args)?;
            if let Some(role) = arg_string(args, "role") {
                if role.chars().count() > 80 || role.contains('\n') || role.contains('\r') {
                    return Err(error("role is invalid".to_string()));
                }
            }
            validate_optional_enum(args, "match_mode", &["contains", "exact"])?;
            validate_integer_arg(args, "occurrence", 1, 1, 20)?;
            validate_integer_arg(args, "max_elements", 120, 1, 500)?;
            validate_integer_arg(args, "padding", 0, 0, 5000)?;
            validate_integer_arg(args, "timeout_sec", 20, 1, 120)?;
        }
        "computer_screenshot_window" => {
            validate_window_target(args)?;
            validate_integer_arg(args, "padding", 0, 0, 5000)?;
            validate_integer_arg(args, "timeout_sec", 20, 1, 120)?;
        }
        "computer_read_text" => {
            for key in ["include_accessibility", "include_ocr", "screenshot"] {
                if args.get(key).is_some_and(|value| !value.is_boolean()) {
                    return Err(error(format!("{key} must be a boolean")));
                }
            }
            if args
                .get("include_accessibility")
                .and_then(Value::as_bool)
                .is_some_and(|value| !value)
                && args
                    .get("include_ocr")
                    .and_then(Value::as_bool)
                    .is_some_and(|value| !value)
            {
                return Err(error(
                    "include_accessibility or include_ocr must be enabled".to_string(),
                ));
            }
            validate_integer_arg(args, "max_results", 120, 1, 500)?;
            validate_integer_arg(args, "max_elements", 200, 1, 500)?;
            validate_float_arg(args, "min_confidence", 0.25, 0.0, 1.0)?;
            validate_screenshot_crop_args(args)?;
            validate_integer_arg(args, "timeout_sec", 20, 1, 120)?;
        }
        "computer_find_text"
        | "computer_assert_text"
        | "computer_wait_for_text"
        | "computer_move_to_text"
        | "computer_click_text"
        | "computer_select_text" => {
            let text = required_string(args, "text")?;
            if text.chars().count() > 200 || text.contains('\n') || text.contains('\r') {
                return Err(error("text is invalid".to_string()));
            }
            validate_optional_enum(args, "match_mode", &["contains", "exact"])?;
            for key in [
                "case_sensitive",
                "include_accessibility",
                "include_ocr",
                "screenshot",
            ] {
                if args.get(key).is_some_and(|value| !value.is_boolean()) {
                    return Err(error(format!("{key} must be a boolean")));
                }
            }
            if args
                .get("include_accessibility")
                .and_then(Value::as_bool)
                .is_some_and(|value| !value)
                && args
                    .get("include_ocr")
                    .and_then(Value::as_bool)
                    .is_some_and(|value| !value)
            {
                return Err(error(
                    "include_accessibility or include_ocr must be enabled".to_string(),
                ));
            }
            validate_integer_arg(args, "max_results", 20, 1, 100)?;
            validate_integer_arg(args, "max_elements", 160, 1, 500)?;
            validate_float_arg(args, "min_confidence", 0.25, 0.0, 1.0)?;
            validate_screenshot_crop_args(args)?;
            validate_integer_arg(args, "timeout_sec", 20, 1, 120)?;
            if action != "computer_find_text"
                && action != "computer_move_to_text"
                && action != "computer_click_text"
                && action != "computer_select_text"
                && args.get("exists").is_some_and(|value| !value.is_boolean())
            {
                return Err(error("exists must be a boolean".to_string()));
            }
            if action == "computer_move_to_text"
                || action == "computer_click_text"
                || action == "computer_select_text"
            {
                validate_optional_enum(args, "source", &["any", "accessibility", "ocr"])?;
                validate_integer_arg(args, "occurrence", 1, 1, 20)?;
                validate_integer_arg(args, "wait_after_ms", 250, 0, 5000)?;
            }
            if action == "computer_move_to_text" || action == "computer_click_text" {
                validate_optional_element_offsets(args, &["offset_x", "offset_y"])?;
            }
            if action == "computer_click_text" {
                validate_optional_enum(args, "button", &["left", "right", "middle"])?;
                validate_integer_arg(args, "clicks", 1, 1, 3)?;
                validate_optional_mouse_modifiers(args)?;
                validate_integer_arg(args, "wait_after_ms", 500, 0, 5000)?;
            }
            if action == "computer_select_text" {
                validate_optional_element_offsets(
                    args,
                    &["start_offset_x", "end_offset_x", "offset_y"],
                )?;
                validate_optional_mouse_modifiers(args)?;
                validate_integer_arg(args, "duration_ms", 350, 0, 10_000)?;
                validate_integer_arg(args, "wait_after_ms", 500, 0, 5000)?;
            }
            if action == "computer_wait_for_text" {
                validate_integer_arg(args, "timeout_ms", 5000, 0, 60_000)?;
                validate_integer_arg(args, "interval_ms", 500, 100, 5000)?;
            }
        }
        "computer_wait" => {
            validate_integer_arg(args, "duration_ms", 1000, 0, 30_000)?;
            for key in ["screenshot", "until_stable"] {
                if args.get(key).is_some_and(|value| !value.is_boolean()) {
                    return Err(error(format!("{key} must be a boolean")));
                }
            }
            validate_integer_arg(args, "timeout_ms", 5000, 0, 60_000)?;
            validate_integer_arg(args, "stable_ms", 1000, 100, 10_000)?;
            validate_integer_arg(args, "interval_ms", 500, 100, 5000)?;
            validate_integer_arg(args, "timeout_sec", 20, 1, 120)?;
            validate_screenshot_crop_args(args)?;
        }
        "computer_accessibility_snapshot" => {
            validate_integer_arg(args, "max_elements", 80, 1, 500)?;
            for key in ["include_values", "include_actions", "include_state"] {
                if args.get(key).is_some_and(|value| !value.is_boolean()) {
                    return Err(error(format!("{key} must be a boolean")));
                }
            }
        }
        "computer_element_at" => {
            validate_required_coordinate_args(args, &["x", "y"])?;
            validate_integer_arg(args, "max_elements", 200, 1, 500)?;
            for key in ["include_values", "include_actions", "include_state"] {
                if args.get(key).is_some_and(|value| !value.is_boolean()) {
                    return Err(error(format!("{key} must be a boolean")));
                }
            }
        }
        "computer_press_at" => {
            validate_required_coordinate_args(args, &["x", "y"])?;
            validate_integer_arg(args, "max_elements", 200, 1, 500)?;
            validate_integer_arg(args, "wait_after_ms", 500, 0, 5000)?;
            validate_boolean_arg(args, "screenshot")?;
        }
        "computer_type_at" => {
            validate_required_coordinate_args(args, &["x", "y"])?;
            validate_required_string(args, "text", 10_000)?;
            validate_integer_arg(args, "max_elements", 200, 1, 500)?;
            for key in ["paste", "clear", "press_enter", "screenshot"] {
                validate_boolean_arg(args, key)?;
            }
            validate_integer_arg(args, "paste_settle_ms", 120, 0, 2000)?;
            validate_integer_arg(args, "wait_after_ms", 500, 0, 5000)?;
        }
        "computer_scroll_at" => {
            validate_required_coordinate_args(args, &["x", "y"])?;
            validate_integer_arg(args, "max_elements", 200, 1, 500)?;
            validate_optional_enum(args, "direction", &["down", "up", "left", "right"])?;
            validate_integer_arg(args, "amount", 600, 1, 5000)?;
            validate_optional_mouse_modifiers(args)?;
            validate_integer_arg(args, "wait_after_ms", 250, 0, 5000)?;
            validate_boolean_arg(args, "screenshot")?;
        }
        "computer_move_to_element" => {
            validate_query_or_index(args)?;
            if let Some(role) = arg_string(args, "role") {
                if role.chars().count() > 80 || role.contains('\n') || role.contains('\r') {
                    return Err(error("role is invalid".to_string()));
                }
            }
            validate_optional_enum(args, "match_mode", &["contains", "exact"])?;
            validate_integer_arg(args, "occurrence", 1, 1, 20)?;
            validate_optional_element_offsets(args, &["offset_x", "offset_y"])?;
            validate_integer_arg(args, "max_elements", 120, 1, 500)?;
            validate_integer_arg(args, "wait_after_ms", 250, 0, 5000)?;
            validate_boolean_arg(args, "screenshot")?;
        }
        "computer_click_element" => {
            validate_query_or_index(args)?;
            if let Some(role) = arg_string(args, "role") {
                if role.chars().count() > 80 || role.contains('\n') || role.contains('\r') {
                    return Err(error("role is invalid".to_string()));
                }
            }
            validate_optional_enum(args, "match_mode", &["contains", "exact"])?;
            validate_integer_arg(args, "occurrence", 1, 1, 20)?;
            validate_optional_element_offsets(args, &["offset_x", "offset_y"])?;
            validate_integer_arg(args, "max_elements", 120, 1, 500)?;
            validate_optional_enum(args, "button", &["left", "right", "middle"])?;
            validate_integer_arg(args, "clicks", 1, 1, 3)?;
            validate_optional_mouse_modifiers(args)?;
            validate_integer_arg(args, "wait_after_ms", 500, 0, 5000)?;
            validate_boolean_arg(args, "screenshot")?;
        }
        "computer_press_element" => {
            validate_query_or_index(args)?;
            if let Some(role) = arg_string(args, "role") {
                if role.chars().count() > 80 || role.contains('\n') || role.contains('\r') {
                    return Err(error("role is invalid".to_string()));
                }
            }
            validate_optional_enum(args, "match_mode", &["contains", "exact"])?;
            validate_integer_arg(args, "occurrence", 1, 1, 20)?;
            validate_integer_arg(args, "max_elements", 120, 1, 500)?;
            if args
                .get("screenshot")
                .is_some_and(|value| !value.is_boolean())
            {
                return Err(error("screenshot must be a boolean".to_string()));
            }
            validate_integer_arg(args, "wait_after_ms", 500, 0, 5000)?;
        }
        "computer_focus_element" => {
            validate_query_or_index(args)?;
            if let Some(role) = arg_string(args, "role") {
                if role.chars().count() > 80 || role.contains('\n') || role.contains('\r') {
                    return Err(error("role is invalid".to_string()));
                }
            }
            validate_optional_enum(args, "match_mode", &["contains", "exact"])?;
            validate_integer_arg(args, "occurrence", 1, 1, 20)?;
            validate_integer_arg(args, "max_elements", 120, 1, 500)?;
            validate_integer_arg(args, "wait_after_ms", 500, 0, 5000)?;
            validate_boolean_arg(args, "screenshot")?;
        }
        "computer_perform_element_action" => {
            validate_query_or_index(args)?;
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| error("action is required".to_string()))?;
            if !ALLOWED_ELEMENT_ACTIONS.contains(&action) {
                return Err(error("action is invalid".to_string()));
            }
            if let Some(role) = arg_string(args, "role") {
                if role.chars().count() > 80 || role.contains('\n') || role.contains('\r') {
                    return Err(error("role is invalid".to_string()));
                }
            }
            validate_optional_enum(args, "match_mode", &["contains", "exact"])?;
            validate_integer_arg(args, "occurrence", 1, 1, 20)?;
            validate_integer_arg(args, "max_elements", 120, 1, 500)?;
            validate_integer_arg(args, "wait_after_ms", 500, 0, 5000)?;
            validate_boolean_arg(args, "screenshot")?;
        }
        "computer_type_element" => {
            validate_query_or_index(args)?;
            let text = args
                .get("text")
                .and_then(Value::as_str)
                .ok_or_else(|| error("text is required".to_string()))?;
            if text.chars().count() > 10_000 {
                return Err(error("text must be <= 10000 characters".to_string()));
            }
            if let Some(role) = arg_string(args, "role") {
                if role.chars().count() > 80 || role.contains('\n') || role.contains('\r') {
                    return Err(error("role is invalid".to_string()));
                }
            }
            validate_optional_enum(args, "match_mode", &["contains", "exact"])?;
            validate_integer_arg(args, "occurrence", 1, 1, 20)?;
            validate_optional_element_offsets(args, &["offset_x", "offset_y"])?;
            validate_integer_arg(args, "max_elements", 120, 1, 500)?;
            for key in ["paste", "clear", "press_enter", "screenshot"] {
                if args.get(key).is_some_and(|value| !value.is_boolean()) {
                    return Err(error(format!("{key} must be a boolean")));
                }
            }
            validate_integer_arg(args, "paste_settle_ms", 120, 0, 2000)?;
            validate_integer_arg(args, "wait_after_ms", 500, 0, 5000)?;
        }
        "computer_set_element_value" => {
            validate_query_or_index(args)?;
            let value = args
                .get("value")
                .and_then(Value::as_str)
                .ok_or_else(|| error("value is required".to_string()))?;
            if value.chars().count() > 10_000 {
                return Err(error("value must be <= 10000 characters".to_string()));
            }
            if let Some(role) = arg_string(args, "role") {
                if role.chars().count() > 80 || role.contains('\n') || role.contains('\r') {
                    return Err(error("role is invalid".to_string()));
                }
            }
            validate_optional_enum(args, "match_mode", &["contains", "exact"])?;
            validate_integer_arg(args, "occurrence", 1, 1, 20)?;
            validate_integer_arg(args, "max_elements", 120, 1, 500)?;
            for key in ["press_enter", "screenshot"] {
                if args.get(key).is_some_and(|value| !value.is_boolean()) {
                    return Err(error(format!("{key} must be a boolean")));
                }
            }
            validate_integer_arg(args, "wait_after_ms", 500, 0, 5000)?;
        }
        "computer_assert_element" => {
            validate_query_or_index(args)?;
            if let Some(role) = arg_string(args, "role") {
                if role.chars().count() > 80 || role.contains('\n') || role.contains('\r') {
                    return Err(error("role is invalid".to_string()));
                }
            }
            if let Some(value_contains) = arg_string(args, "value_contains") {
                if value_contains.chars().count() > 200
                    || value_contains.contains('\n')
                    || value_contains.contains('\r')
                {
                    return Err(error("value_contains is invalid".to_string()));
                }
            }
            validate_optional_enum(args, "match_mode", &["contains", "exact"])?;
            for key in ["exists", "enabled", "focused", "selected"] {
                if args.get(key).is_some_and(|value| !value.is_boolean()) {
                    return Err(error(format!("{key} must be a boolean")));
                }
            }
            validate_integer_arg(args, "max_elements", 120, 1, 500)?;
        }
        "computer_wait_for_element" => {
            validate_query_or_index(args)?;
            if let Some(role) = arg_string(args, "role") {
                if role.chars().count() > 80 || role.contains('\n') || role.contains('\r') {
                    return Err(error("role is invalid".to_string()));
                }
            }
            if let Some(value_contains) = arg_string(args, "value_contains") {
                if value_contains.chars().count() > 200
                    || value_contains.contains('\n')
                    || value_contains.contains('\r')
                {
                    return Err(error("value_contains is invalid".to_string()));
                }
            }
            validate_optional_enum(args, "match_mode", &["contains", "exact"])?;
            for key in ["exists", "enabled", "focused", "selected"] {
                if args.get(key).is_some_and(|value| !value.is_boolean()) {
                    return Err(error(format!("{key} must be a boolean")));
                }
            }
            validate_integer_arg(args, "max_elements", 120, 1, 500)?;
            validate_integer_arg(args, "timeout_ms", 5000, 0, 60_000)?;
            validate_integer_arg(args, "interval_ms", 500, 100, 5000)?;
        }
        "computer_get_position" => {}
        "computer_move_mouse" => {
            validate_required_coordinate_args(args, &["x", "y"])?;
            validate_integer_arg(args, "wait_after_ms", 250, 0, 5000)?;
            if args
                .get("screenshot")
                .is_some_and(|value| !value.is_boolean())
            {
                return Err(error("screenshot must be a boolean".to_string()));
            }
        }
        "computer_mouse_down" | "computer_mouse_up" => {
            validate_optional_coordinates(args)?;
            validate_optional_enum(args, "button", &["left", "right", "middle"])?;
            validate_optional_mouse_modifiers(args)?;
            validate_integer_arg(args, "wait_after_ms", 250, 0, 5000)?;
            if args
                .get("screenshot")
                .is_some_and(|value| !value.is_boolean())
            {
                return Err(error("screenshot must be a boolean".to_string()));
            }
        }
        "computer_click" => {
            if !validate_optional_coordinates(args)? {
                return Err(error("x and y are required".to_string()));
            }
            validate_optional_enum(args, "button", &["left", "right", "middle"])?;
            validate_integer_arg(args, "clicks", 1, 1, 3)?;
            validate_optional_mouse_modifiers(args)?;
            validate_integer_arg(args, "wait_after_ms", 500, 0, 5000)?;
            validate_boolean_arg(args, "screenshot")?;
        }
        "computer_drag" => {
            validate_required_coordinate_args(args, &["start_x", "start_y", "end_x", "end_y"])?;
            validate_optional_enum(args, "button", &["left", "right", "middle"])?;
            validate_integer_arg(args, "duration_ms", 500, 0, 10_000)?;
            validate_optional_mouse_modifiers(args)?;
            validate_integer_arg(args, "wait_after_ms", 500, 0, 5000)?;
            validate_boolean_arg(args, "screenshot")?;
        }
        "computer_drag_element" => {
            validate_query_or_index(args)?;
            if let Some(role) = arg_string(args, "role") {
                if role.chars().count() > 80 || role.contains('\n') || role.contains('\r') {
                    return Err(error("role is invalid".to_string()));
                }
            }
            validate_optional_enum(args, "match_mode", &["contains", "exact"])?;
            validate_integer_arg(args, "occurrence", 1, 1, 20)?;
            validate_optional_element_offsets(args, &["offset_x", "offset_y"])?;
            validate_drag_element_target(args)?;
            validate_integer_arg(args, "max_elements", 120, 1, 500)?;
            validate_optional_enum(args, "button", &["left", "right", "middle"])?;
            validate_integer_arg(args, "duration_ms", 500, 0, 10_000)?;
            validate_optional_mouse_modifiers(args)?;
            validate_integer_arg(args, "wait_after_ms", 500, 0, 5000)?;
            validate_boolean_arg(args, "screenshot")?;
        }
        "computer_type_text" => {
            let text = args
                .get("text")
                .and_then(Value::as_str)
                .ok_or_else(|| error("text is required".to_string()))?;
            if text.chars().count() > 10_000 {
                return Err(error("text must be <= 10000 characters".to_string()));
            }
            if args.get("paste").is_some_and(|value| !value.is_boolean()) {
                return Err(error("paste must be a boolean".to_string()));
            }
            for key in ["clear", "press_enter"] {
                if args.get(key).is_some_and(|value| !value.is_boolean()) {
                    return Err(error(format!("{key} must be a boolean")));
                }
            }
            validate_integer_arg(args, "paste_settle_ms", 120, 0, 2000)?;
            validate_integer_arg(args, "wait_after_ms", 250, 0, 5000)?;
            if args
                .get("screenshot")
                .is_some_and(|value| !value.is_boolean())
            {
                return Err(error("screenshot must be a boolean".to_string()));
            }
        }
        "computer_get_clipboard" => {
            validate_integer_arg(args, "max_chars", 4000, 1, 50_000)?;
        }
        "computer_set_clipboard" => {
            let text = args
                .get("text")
                .and_then(Value::as_str)
                .ok_or_else(|| error("text is required".to_string()))?;
            if text.chars().count() > 100_000 {
                return Err(error("text must be <= 100000 characters".to_string()));
            }
        }
        "computer_copy_selection" => {
            validate_integer_arg(args, "max_chars", 4000, 1, 50_000)?;
            validate_integer_arg(args, "wait_after_ms", 250, 0, 5000)?;
            validate_boolean_arg(args, "restore_clipboard")?;
        }
        "computer_assert_clipboard" => {
            validate_clipboard_condition_args(args, false)?;
        }
        "computer_wait_for_clipboard" => {
            validate_clipboard_condition_args(args, true)?;
        }
        "computer_press_key" => {
            let key = required_string(args, "key")?;
            if !is_allowed_computer_key(&key) {
                return Err(error("key is not supported".to_string()));
            }
            validate_integer_arg(args, "wait_after_ms", 250, 0, 5000)?;
            if args
                .get("screenshot")
                .is_some_and(|value| !value.is_boolean())
            {
                return Err(error("screenshot must be a boolean".to_string()));
            }
        }
        "computer_hotkey" => {
            validate_computer_hotkey_args(args)?;
        }
        "computer_key_down" | "computer_key_up" => {
            let key = required_string(args, "key")?;
            if !is_allowed_computer_key_state_key(&key) {
                return Err(error("key is not supported".to_string()));
            }
            validate_integer_arg(args, "wait_after_ms", 250, 0, 5000)?;
            if args
                .get("screenshot")
                .is_some_and(|value| !value.is_boolean())
            {
                return Err(error("screenshot must be a boolean".to_string()));
            }
        }
        "computer_scroll" => {
            validate_optional_enum(args, "direction", &["down", "up", "left", "right"])?;
            validate_integer_arg(args, "amount", 600, 1, 5000)?;
            validate_optional_coordinates(args)?;
            validate_optional_mouse_modifiers(args)?;
            validate_integer_arg(args, "wait_after_ms", 250, 0, 5000)?;
            validate_boolean_arg(args, "screenshot")?;
        }
        "computer_scroll_element" => {
            validate_query_or_index(args)?;
            if let Some(role) = arg_string(args, "role") {
                if role.chars().count() > 80 || role.contains('\n') || role.contains('\r') {
                    return Err(error("role is invalid".to_string()));
                }
            }
            validate_optional_enum(args, "match_mode", &["contains", "exact"])?;
            validate_integer_arg(args, "occurrence", 1, 1, 20)?;
            validate_optional_element_offsets(args, &["offset_x", "offset_y"])?;
            validate_integer_arg(args, "max_elements", 120, 1, 500)?;
            validate_optional_enum(args, "direction", &["down", "up", "left", "right"])?;
            validate_integer_arg(args, "amount", 600, 1, 5000)?;
            validate_optional_mouse_modifiers(args)?;
            validate_integer_arg(args, "wait_after_ms", 250, 0, 5000)?;
            validate_boolean_arg(args, "screenshot")?;
        }
        "computer_list_apps" => {
            if args
                .get("include_background")
                .is_some_and(|value| !value.is_boolean())
            {
                return Err(error("include_background must be a boolean".to_string()));
            }
            validate_integer_arg(args, "max_apps", 100, 1, 500)?;
        }
        "computer_assert_app" => {
            validate_app_condition_args(args, false)?;
        }
        "computer_wait_for_app" => {
            validate_app_condition_args(args, true)?;
        }
        "computer_list_windows" => {
            validate_integer_arg(args, "max_windows", 30, 1, 200)?;
        }
        "computer_focus_window" => {
            validate_window_target(args)?;
            validate_boolean_arg(args, "screenshot")?;
        }
        "computer_set_window_bounds" => {
            validate_window_bounds_args(args)?;
            validate_boolean_arg(args, "screenshot")?;
        }
        "computer_perform_window_action" => {
            validate_window_target(args)?;
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| error("action is required".to_string()))?;
            if !ALLOWED_WINDOW_ACTIONS.contains(&action) {
                return Err(error("action is invalid".to_string()));
            }
            validate_integer_arg(args, "wait_after_ms", 500, 0, 5000)?;
            validate_boolean_arg(args, "screenshot")?;
        }
        "computer_close_window" => {
            validate_window_target(args)?;
            validate_integer_arg(args, "wait_after_ms", 500, 0, 5000)?;
            validate_boolean_arg(args, "screenshot")?;
        }
        "computer_assert_window" => {
            validate_window_condition_args(args, false)?;
        }
        "computer_wait_for_window" => {
            validate_window_condition_args(args, true)?;
        }
        "computer_list_menus" => {
            let app = required_string(args, "app")?;
            if app.len() > 120 || app.contains('\n') || app.contains('\r') {
                return Err(error("app is invalid".to_string()));
            }
            validate_optional_menu_label_arg(args, "top_menu")?;
            validate_integer_arg(args, "max_items", 120, 1, 500)?;
        }
        "computer_select_menu" => {
            let app = required_string(args, "app")?;
            if app.len() > 120 || app.contains('\n') || app.contains('\r') {
                return Err(error("app is invalid".to_string()));
            }
            validate_menu_path_arg(args)?;
            validate_boolean_arg(args, "screenshot")?;
        }
        "computer_activate_app" => {
            let app = required_string(args, "app")?;
            if app.len() > 120 || app.contains('\n') || app.contains('\r') {
                return Err(error("app is invalid".to_string()));
            }
            validate_boolean_arg(args, "screenshot")?;
        }
        "computer_quit_app" => {
            let app = required_string(args, "app")?;
            if app.len() > 120 || app.contains('\n') || app.contains('\r') {
                return Err(error("app is invalid".to_string()));
            }
            validate_integer_arg(args, "wait_after_ms", 500, 0, 5000)?;
            if args
                .get("screenshot")
                .is_some_and(|value| !value.is_boolean())
            {
                return Err(error("screenshot must be a boolean".to_string()));
            }
        }
        "computer_open_item" => {
            validate_open_item_args(args)?;
        }
        _ => {}
    }
    Ok(())
}

fn validate_browser_tool_args(action: &str, args: &Map<String, Value>) -> Result<(), ToolOutcome> {
    match action {
        "browser_open_page" => {
            let url = required_string(args, "url")?;
            validate_browser_page_url(&url)?;
            validate_optional_enum(
                args,
                "wait_until",
                &["domcontentloaded", "load", "networkidle"],
            )?;
            validate_integer_arg(args, "viewport_width", 1440, 320, 3840)?;
            validate_integer_arg(args, "viewport_height", 1000, 240, 3000)?;
            validate_integer_arg(args, "max_text_chars", 8000, 500, 40_000)?;
            validate_boolean_args(args, &["screenshot", "full_page"])?;
        }
        "browser_inspect" => {
            validate_integer_arg(args, "max_text_chars", 8000, 500, 40_000)?;
            validate_boolean_args(args, &["screenshot", "full_page"])?;
        }
        "browser_console_logs" => {
            validate_optional_enum(args, "level", &["all", "warnings", "errors"])?;
            validate_integer_arg(args, "max_entries", 50, 1, 200)?;
            validate_boolean_arg(args, "clear")?;
        }
        "browser_network_events" => {
            validate_optional_enum(args, "level", &["all", "warnings", "errors"])?;
            validate_integer_arg(args, "max_entries", 50, 1, 300)?;
            validate_boolean_arg(args, "clear")?;
        }
        "browser_dom_snapshot" => {
            let selector = optional_nonempty_string(args, "selector");
            let text = optional_nonempty_string(args, "text");
            if let Some(text) = text.as_ref() {
                if text.chars().count() > 1000 || text.contains('\0') {
                    return Err(error("text is invalid".to_string()));
                }
            }
            let coords = validate_optional_coordinates(args)?;
            let targets = [selector.is_some(), text.is_some(), coords]
                .into_iter()
                .filter(|value| *value)
                .count();
            if targets != 1 {
                return Err(error(
                    "provide exactly one target: selector, text, or x+y".to_string(),
                ));
            }
            validate_boolean_args(
                args,
                &["include_styles", "include_html", "include_children"],
            )?;
            validate_integer_arg(args, "max_html_chars", 4000, 0, 20_000)?;
        }
        "browser_storage_state" => {
            validate_boolean_args(
                args,
                &[
                    "include_local_storage",
                    "include_session_storage",
                    "include_cookies",
                    "include_values",
                ],
            )?;
            if let Some(key_filter) = optional_nonempty_string(args, "key_filter") {
                if key_filter.chars().count() > 120
                    || key_filter.contains('\0')
                    || key_filter.contains('\n')
                    || key_filter.contains('\r')
                {
                    return Err(error("key_filter is invalid".to_string()));
                }
            }
            validate_integer_arg(args, "max_items", 100, 1, 500)?;
            validate_integer_arg(args, "max_value_chars", 500, 0, 5000)?;
        }
        "browser_accessibility_snapshot" => {
            let selector = optional_nonempty_string(args, "selector");
            let text = optional_nonempty_string(args, "text");
            if let Some(text) = text.as_ref() {
                if text.chars().count() > 1000 || text.contains('\0') {
                    return Err(error("text is invalid".to_string()));
                }
            }
            let coords = validate_optional_coordinates(args)?;
            let targets = [selector.is_some(), text.is_some(), coords]
                .into_iter()
                .filter(|value| *value)
                .count();
            if targets > 1 {
                return Err(error(
                    "provide at most one target: selector, text, or x+y".to_string(),
                ));
            }
            validate_boolean_args(
                args,
                &[
                    "include_hidden",
                    "include_bounds",
                    "include_selectors",
                    "include_values",
                ],
            )?;
            validate_integer_arg(args, "max_nodes", 120, 1, 1000)?;
            validate_integer_arg(args, "max_name_chars", 160, 20, 1000)?;
        }
        "browser_list_pages" => {}
        "browser_switch_page" => {
            validate_browser_switch_page_args(args)?;
        }
        "browser_screenshot" => {
            validate_browser_screenshot_args(args)?;
        }
        "browser_hover" => {
            let selector = optional_nonempty_string(args, "selector");
            let text = optional_nonempty_string(args, "text");
            let coords = validate_optional_coordinates(args)?;
            let targets = [selector.is_some(), text.is_some(), coords]
                .into_iter()
                .filter(|value| *value)
                .count();
            if targets != 1 {
                return Err(error(
                    "provide exactly one target: selector, text, or x+y".to_string(),
                ));
            }
            validate_integer_arg(args, "wait_after_ms", 500, 0, 5000)?;
            validate_boolean_arg(args, "screenshot")?;
        }
        "browser_click" => {
            let selector = optional_nonempty_string(args, "selector");
            let text = optional_nonempty_string(args, "text");
            let coords = validate_optional_coordinates(args)?;
            let targets = [selector.is_some(), text.is_some(), coords]
                .into_iter()
                .filter(|value| *value)
                .count();
            if targets != 1 {
                return Err(error(
                    "provide exactly one target: selector, text, or x+y".to_string(),
                ));
            }
            validate_optional_enum(args, "button", &["left", "right", "middle"])?;
            validate_integer_arg(args, "clicks", 1, 1, 3)?;
            validate_optional_browser_click_modifiers(args)?;
            validate_browser_dialog_args(args)?;
            validate_browser_download_args(args)?;
            validate_browser_popup_args(args)?;
            validate_integer_arg(args, "wait_after_ms", 500, 0, 5000)?;
            validate_boolean_arg(args, "screenshot")?;
        }
        "browser_drag" => {
            let source_selector = optional_nonempty_string(args, "source_selector");
            let source_text = optional_nonempty_string(args, "source_text");
            let start_coords =
                validate_optional_browser_coordinate_pair(args, "start_x", "start_y")?;
            let source_targets = [
                source_selector.is_some(),
                source_text.is_some(),
                start_coords,
            ]
            .into_iter()
            .filter(|value| *value)
            .count();
            if source_targets != 1 {
                return Err(error(
                    "provide exactly one drag source: source_selector, source_text, or start_x+start_y"
                        .to_string(),
                ));
            }

            let target_selector = optional_nonempty_string(args, "target_selector");
            let target_text = optional_nonempty_string(args, "target_text");
            let end_coords = validate_optional_browser_coordinate_pair(args, "end_x", "end_y")?;
            let target_targets = [target_selector.is_some(), target_text.is_some(), end_coords]
                .into_iter()
                .filter(|value| *value)
                .count();
            if target_targets != 1 {
                return Err(error(
                    "provide exactly one drag target: target_selector, target_text, or end_x+end_y"
                        .to_string(),
                ));
            }

            validate_optional_enum(args, "button", &["left", "right", "middle"])?;
            validate_integer_arg(args, "steps", 12, 1, 100)?;
            validate_optional_browser_click_modifiers(args)?;
            validate_integer_arg(args, "wait_after_ms", 500, 0, 5000)?;
            validate_boolean_arg(args, "screenshot")?;
        }
        "browser_type_text" => {
            let text = args
                .get("text")
                .and_then(Value::as_str)
                .ok_or_else(|| error("text is required".to_string()))?;
            if text.chars().count() > 10_000 {
                return Err(error("text must be <= 10000 characters".to_string()));
            }
            let selector = optional_nonempty_string(args, "selector");
            let label = optional_nonempty_string(args, "label");
            let coords = validate_optional_coordinates(args)?;
            let targets = [selector.is_some(), label.is_some(), coords]
                .into_iter()
                .filter(|value| *value)
                .count();
            if targets > 1 {
                return Err(error(
                    "use at most one target: selector, label, or x+y".to_string(),
                ));
            }
            validate_integer_arg(args, "wait_after_ms", 500, 0, 5000)?;
            validate_boolean_args(args, &["clear_first", "press_enter", "screenshot"])?;
        }
        "browser_select_option" => {
            validate_browser_select_option_args(args)?;
        }
        "browser_set_checked" => {
            validate_browser_set_checked_args(args)?;
        }
        "browser_upload_file" => {
            validate_browser_upload_file_args(args)?;
        }
        "browser_press_key" => {
            let key = required_string(args, "key")?;
            if !is_allowed_browser_key(&key) {
                return Err(error("key is not supported".to_string()));
            }
            validate_integer_arg(args, "wait_after_ms", 500, 0, 5000)?;
            validate_boolean_arg(args, "screenshot")?;
        }
        "browser_scroll" => {
            validate_optional_enum(
                args,
                "direction",
                &["down", "up", "left", "right", "top", "bottom"],
            )?;
            validate_integer_arg(args, "amount", 800, 1, 5000)?;
            validate_integer_arg(args, "wait_after_ms", 300, 0, 5000)?;
            validate_boolean_arg(args, "screenshot")?;
        }
        "browser_assert" => {
            validate_browser_condition_args(args, false)?;
        }
        "browser_wait_for" => {
            validate_browser_condition_args(args, true)?;
        }
        _ => {}
    }
    Ok(())
}

fn validate_browser_condition_args(
    args: &Map<String, Value>,
    include_wait: bool,
) -> Result<(), ToolOutcome> {
    let fields = ["text", "selector", "url_contains", "title_contains"];
    let mut present = 0;
    for field in fields {
        if let Some(value) = optional_nonempty_string(args, field) {
            present += 1;
            if value.chars().count() > 1000 || value.contains('\0') {
                return Err(error(format!("{field} is invalid")));
            }
            if field != "text" && (value.contains('\n') || value.contains('\r')) {
                return Err(error(format!("{field} cannot contain newlines")));
            }
        }
    }
    if present != 1 {
        return Err(error(
            "provide exactly one condition: text, selector, url_contains, or title_contains"
                .to_string(),
        ));
    }
    validate_optional_enum(args, "match_mode", &["contains", "exact"])?;
    validate_integer_arg(args, "max_text_chars", 8000, 500, 40_000)?;
    validate_boolean_args(args, &["exists", "screenshot"])?;
    if include_wait {
        validate_integer_arg(args, "timeout_ms", 5000, 0, 60_000)?;
        validate_integer_arg(args, "interval_ms", 250, 100, 5000)?;
    }
    Ok(())
}

fn validate_browser_http_url(url: &str) -> Result<(), ToolOutcome> {
    let value = url.trim();
    if value.len() > 4096 {
        return Err(error("URL is too long".to_string()));
    }
    if value
        .chars()
        .any(|ch| ch.is_ascii_whitespace() || ch == '\0')
    {
        return Err(error("URL must not contain whitespace".to_string()));
    }
    let lower = value.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return Err(error("only http(s) URLs are allowed".to_string()));
    }
    let host = match http_url_host(value) {
        Some(host) if !host.is_empty() => host,
        _ => return Err(error("URL host is required".to_string())),
    };
    let host = host.trim_matches('.').to_ascii_lowercase();
    if host == "broadcasthost" {
        return Err(error("host is blocked".to_string()));
    }
    if host == "0.0.0.0" || host == "::" {
        return Err(error("unspecified addresses are blocked".to_string()));
    }
    if host.starts_with("169.254.") {
        return Err(error("link-local addresses are blocked".to_string()));
    }
    Ok(())
}

fn validate_browser_page_url(url: &str) -> Result<(), ToolOutcome> {
    let value = url.trim();
    let lower = value.to_ascii_lowercase();
    if lower.starts_with("file://") {
        let _ = browser_file_url_path(value)?;
        return Ok(());
    }
    validate_browser_http_url(value)
}

fn browser_file_url_path(url: &str) -> Result<PathBuf, ToolOutcome> {
    let value = url.trim();
    if value.len() > 4096 {
        return Err(error("URL is too long".to_string()));
    }
    if value
        .chars()
        .any(|ch| ch.is_ascii_whitespace() || ch == '\0')
    {
        return Err(error("URL must not contain whitespace".to_string()));
    }
    if !value.to_ascii_lowercase().starts_with("file://") {
        return Err(error("only file:// URLs are allowed".to_string()));
    }
    let url = url::Url::parse(value).map_err(|_| error("invalid file URL".to_string()))?;
    if url.host_str().is_some_and(|host| !host.eq_ignore_ascii_case("localhost")) {
        return Err(error("file URL host must be empty or localhost".to_string()));
    }
    let path = url.to_file_path().map_err(|_| error("file URL path must be absolute".to_string()))?;
    if path.as_os_str().to_string_lossy().contains('\0') { return Err(error("file URL path is invalid".to_string())); }
    Ok(path)
}

fn percent_decode(value: &str) -> Result<String, ToolOutcome> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return Err(error(
                    "file URL contains invalid percent encoding".to_string(),
                ));
            }
            let high = hex_value(bytes[index + 1])
                .ok_or_else(|| error("file URL contains invalid percent encoding".to_string()))?;
            let low = hex_value(bytes[index + 2])
                .ok_or_else(|| error("file URL contains invalid percent encoding".to_string()))?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).map_err(|_| error("file URL path must be valid UTF-8".to_string()))
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
fn file_url_for_path(path: &Path) -> Result<String, ToolOutcome> {
    url::Url::from_file_path(path).map(|url| url.to_string())
        .map_err(|_| error("file URL path must be absolute".to_string()))
}

fn http_url_host(url: &str) -> Option<String> {
    let (_, rest) = url.split_once("://")?;
    let authority = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .rsplit('@')
        .next()
        .unwrap_or("");
    if authority.starts_with('[') {
        let end = authority.find(']')?;
        return Some(authority[1..end].to_string());
    }
    Some(authority.split(':').next().unwrap_or("").to_string())
}

fn url_authority(url: &str) -> Option<&str> {
    let (_, rest) = url.split_once("://")?;
    Some(rest.split(['/', '?', '#']).next().unwrap_or(""))
}

fn validate_open_item_http_url(target: &str) -> Result<(), ToolOutcome> {
    if target.chars().any(char::is_whitespace) {
        return Err(error("target URL cannot contain whitespace".to_string()));
    }
    let authority = url_authority(target).unwrap_or("");
    if authority.is_empty() {
        return Err(error("target URL host is required".to_string()));
    }
    if authority.contains('@') {
        return Err(error("target URL credentials are not allowed".to_string()));
    }
    let host =
        http_url_host(target).ok_or_else(|| error("target URL host is invalid".to_string()))?;
    if host.is_empty() {
        return Err(error("target URL host is required".to_string()));
    }
    let port_text = if authority.starts_with('[') {
        let end = authority
            .find(']')
            .ok_or_else(|| error("target URL host is invalid".to_string()))?;
        let rest = &authority[end + 1..];
        if rest.is_empty() {
            None
        } else if let Some(port) = rest.strip_prefix(':') {
            Some(port)
        } else {
            return Err(error("target URL host is invalid".to_string()));
        }
    } else {
        authority.rsplit_once(':').map(|(_, port)| port)
    };
    if let Some(port) = port_text {
        if port.is_empty() || port.parse::<u16>().is_err() {
            return Err(error("target URL port is invalid".to_string()));
        }
    }
    Ok(())
}

fn validate_public_web_url(raw_url: &str, field_name: &str) -> Result<(), ToolOutcome> {
    let url = reqwest::Url::parse(raw_url)
        .map_err(|_| error(format!("{field_name} must be an absolute http(s) URL")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(error(format!("{field_name} scheme must be http or https")));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(error(format!("{field_name} credentials are not allowed")));
    }
    let Some(host) = url
        .host_str()
        .map(str::trim)
        .filter(|host| !host.is_empty())
    else {
        return Err(error(format!("{field_name} host is required")));
    };
    if is_blocked_public_web_host(host) {
        return Err(error(format!(
            "{field_name} must use a public host; use chrome_native_open_page for localhost, LAN, or private-address pages"
        )));
    }
    Ok(())
}

fn is_blocked_public_web_host(host: &str) -> bool {
    let normalized = host.trim_matches('.').to_ascii_lowercase();
    if normalized == "localhost"
        || normalized == "localhost.localdomain"
        || normalized.ends_with(".localhost")
    {
        return true;
    }
    let ip_text = normalized.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = ip_text.parse::<IpAddr>() {
        return match ip {
            IpAddr::V4(ip) => is_blocked_public_web_ipv4(ip),
            IpAddr::V6(ip) => {
                ip.is_loopback()
                    || ip.is_unspecified()
                    || ip.is_multicast()
                    || is_ipv6_unique_local(ip)
                    || is_ipv6_unicast_link_local(ip)
                    || is_ipv6_documentation(ip)
                    || ipv6_embedded_ipv4(ip).is_some_and(is_blocked_public_web_ipv4)
            }
        };
    }
    normalized.ends_with(".local")
        || normalized.ends_with(".localdomain")
        || normalized.ends_with(".lan")
        || normalized.ends_with(".internal")
        || normalized.ends_with(".home.arpa")
        || normalized == "metadata"
        || normalized == "metadata.google.internal"
}

fn is_blocked_public_web_ipv4(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        || octets[0] == 0
        || (octets[0] == 100 && (64..=127).contains(&octets[1]))
        || (octets[0] == 192 && octets[1] == 0 && octets[2] == 0)
        || (octets[0] == 192 && octets[1] == 0 && octets[2] == 2)
        || (octets[0] == 198 && (octets[1] == 18 || octets[1] == 19))
        || (octets[0] == 198 && octets[1] == 51 && octets[2] == 100)
        || (octets[0] == 203 && octets[1] == 0 && octets[2] == 113)
        || octets[0] >= 240
}

fn ipv6_embedded_ipv4(ip: Ipv6Addr) -> Option<Ipv4Addr> {
    let segments = ip.segments();
    let is_mapped = segments[..5] == [0, 0, 0, 0, 0] && segments[5] == 0xffff;
    let is_compatible = segments[..6] == [0, 0, 0, 0, 0, 0]
        && !(segments[6] == 0 && (segments[7] == 0 || segments[7] == 1));
    if !is_mapped && !is_compatible {
        return None;
    }
    Some(Ipv4Addr::new(
        (segments[6] >> 8) as u8,
        segments[6] as u8,
        (segments[7] >> 8) as u8,
        segments[7] as u8,
    ))
}

fn is_ipv6_unique_local(ip: Ipv6Addr) -> bool {
    (ip.segments()[0] & 0xfe00) == 0xfc00
}

fn is_ipv6_unicast_link_local(ip: Ipv6Addr) -> bool {
    (ip.segments()[0] & 0xffc0) == 0xfe80
}

fn is_ipv6_documentation(ip: Ipv6Addr) -> bool {
    ip.segments()[0] == 0x2001 && ip.segments()[1] == 0x0db8
}

fn html_to_text(html: &str) -> (String, String) {
    let title_regex = Regex::new("(?is)<title[^>]*>(.*?)</title>").expect("valid title regex");
    let title = title_regex
        .captures(html)
        .and_then(|captures| captures.get(1))
        .map(|value| decode_html_entities(value.as_str()).trim().to_string())
        .unwrap_or_default();
    let head_regex = Regex::new("(?is)<head\\b[^>]*>.*?</head>").expect("valid head regex");
    let title_tag_regex =
        Regex::new("(?is)<title\\b[^>]*>.*?</title>").expect("valid title tag regex");
    let script_regex = Regex::new("(?is)<script\\b[^>]*>.*?</script>").expect("valid script regex");
    let style_regex = Regex::new("(?is)<style\\b[^>]*>.*?</style>").expect("valid style regex");
    let tag_regex = Regex::new("(?is)<[^>]+>").expect("valid html tag regex");
    let without_head = head_regex.replace_all(html, " ");
    let without_title = title_tag_regex.replace_all(&without_head, " ");
    let without_script = script_regex.replace_all(&without_title, " ");
    let without_style = style_regex.replace_all(&without_script, " ");
    let without_tags = tag_regex.replace_all(&without_style, " ");
    let decoded = decode_html_entities(&without_tags);
    (title, normalize_extracted_text(&decoded))
}

fn decode_html_entities(value: &str) -> String {
    value
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
}

fn strip_html(value: &str) -> String {
    let tag_regex = Regex::new("(?is)<[^>]+>").expect("valid html tag regex");
    let decoded = decode_html_entities(&tag_regex.replace_all(value, " "));
    normalize_extracted_text(&decoded)
}

fn normalize_extracted_text(value: &str) -> String {
    let whitespace_regex = Regex::new("\\s+").expect("valid whitespace regex");
    let space_before_punctuation_regex =
        Regex::new("\\s+([.,;:!?])").expect("valid punctuation spacing regex");
    let collapsed = whitespace_regex.replace_all(value, " ");
    space_before_punctuation_regex
        .replace_all(collapsed.trim(), "$1")
        .to_string()
}

fn parse_google_results(html: &str, limit: usize) -> Vec<(String, String, String)> {
    let anchor_regex =
        Regex::new("(?is)<a\\b([^>]*)>(.*?)</a>").expect("valid Google result anchor regex");
    let heading_regex =
        Regex::new("(?is)<h3\\b[^>]*>(.*?)</h3>").expect("valid Google heading regex");
    let mut results = Vec::new();
    for captures in anchor_regex.captures_iter(html) {
        let inner = captures.get(2).map(|value| value.as_str()).unwrap_or("");
        let Some(heading) = heading_regex.captures(inner) else {
            continue;
        };
        let attrs = captures.get(1).map(|value| value.as_str()).unwrap_or("");
        let Some(raw_href) = html_attr_value(attrs, "href") else {
            continue;
        };
        let title = strip_html(heading.get(1).map(|value| value.as_str()).unwrap_or(""));
        let link = normalize_google_url(&raw_href);
        if !search_result_is_usable(&title, &link, &results) {
            continue;
        }
        let snippet_window = captures
            .get(0)
            .map(|matched| {
                let tail = &html[matched.end()..];
                next_google_result_start(tail, &anchor_regex, &heading_regex)
                    .map(|next_start| &tail[..next_start])
                    .unwrap_or(tail)
            })
            .unwrap_or("");
        let snippet =
            parse_classed_snippet(snippet_window, &["VwiC3b", "aCOpRe", "IsZvec", "yXK7lf"]);
        results.push((title, link, snippet));
        if results.len() >= limit {
            break;
        }
    }
    results
}

fn next_google_result_start(
    window: &str,
    anchor_regex: &Regex,
    heading_regex: &Regex,
) -> Option<usize> {
    for captures in anchor_regex.captures_iter(window) {
        let inner = captures.get(2).map(|value| value.as_str()).unwrap_or("");
        if heading_regex.is_match(inner) {
            return captures.get(0).map(|matched| matched.start());
        }
    }
    None
}

fn parse_bing_results(html: &str, limit: usize) -> Vec<(String, String, String)> {
    if html.contains("<rss") || html.contains("<item>") {
        return parse_bing_rss_results(html, limit);
    }
    let block_regex =
        Regex::new("(?is)<li\\b([^>]*)>(.*?)</li>").expect("valid Bing result block regex");
    let heading_regex = Regex::new("(?is)<h2\\b[^>]*>\\s*<a\\b([^>]*)>(.*?)</a>")
        .expect("valid Bing heading regex");
    let paragraph_regex =
        Regex::new("(?is)<p\\b[^>]*>(.*?)</p>").expect("valid Bing snippet regex");
    let mut results = Vec::new();
    for captures in block_regex.captures_iter(html) {
        let attrs = captures.get(1).map(|value| value.as_str()).unwrap_or("");
        if !html_class_contains(attrs, "b_algo") {
            continue;
        }
        let inner = captures.get(2).map(|value| value.as_str()).unwrap_or("");
        let Some(heading) = heading_regex.captures(inner) else {
            continue;
        };
        let anchor_attrs = heading.get(1).map(|value| value.as_str()).unwrap_or("");
        let Some(raw_href) = html_attr_value(anchor_attrs, "href") else {
            continue;
        };
        let title = strip_html(heading.get(2).map(|value| value.as_str()).unwrap_or(""));
        let link = normalize_search_url(&raw_href);
        if !search_result_is_usable(&title, &link, &results) {
            continue;
        }
        let snippet = paragraph_regex
            .captures(inner)
            .map(|value| strip_html(value.get(1).map(|value| value.as_str()).unwrap_or("")))
            .unwrap_or_default();
        results.push((title, link, snippet));
        if results.len() >= limit {
            break;
        }
    }
    results
}

fn parse_bing_rss_results(xml: &str, limit: usize) -> Vec<(String, String, String)> {
    let item_regex =
        Regex::new("(?is)<item\\b[^>]*>(.*?)</item>").expect("valid Bing RSS item regex");
    let title_regex =
        Regex::new("(?is)<title\\b[^>]*>(.*?)</title>").expect("valid Bing RSS title regex");
    let link_regex =
        Regex::new("(?is)<link\\b[^>]*>(.*?)</link>").expect("valid Bing RSS link regex");
    let description_regex = Regex::new("(?is)<description\\b[^>]*>(.*?)</description>")
        .expect("valid Bing RSS description regex");
    let mut results = Vec::new();
    for captures in item_regex.captures_iter(xml) {
        let item = captures.get(1).map(|value| value.as_str()).unwrap_or("");
        let title = title_regex
            .captures(item)
            .map(|value| strip_html(value.get(1).map(|value| value.as_str()).unwrap_or("")))
            .unwrap_or_default();
        let link = link_regex
            .captures(item)
            .map(|value| {
                normalize_search_url(value.get(1).map(|value| value.as_str()).unwrap_or(""))
            })
            .unwrap_or_default();
        if !search_result_is_usable(&title, &link, &results) {
            continue;
        }
        let snippet = description_regex
            .captures(item)
            .map(|value| {
                let raw = value.get(1).map(|value| value.as_str()).unwrap_or("");
                strip_html(&decode_html_entities(raw))
            })
            .unwrap_or_default();
        results.push((title, link, snippet));
        if results.len() >= limit {
            break;
        }
    }
    results
}

fn parse_classed_snippet(window: &str, classes: &[&str]) -> String {
    let element_regex = Regex::new("(?is)<(?:div|span)\\b([^>]*)>(.*?)</(?:div|span)>")
        .expect("valid search snippet regex");
    for captures in element_regex.captures_iter(window) {
        let attrs = captures.get(1).map(|value| value.as_str()).unwrap_or("");
        if classes
            .iter()
            .any(|class_name| html_class_contains(attrs, class_name))
        {
            return strip_html(captures.get(2).map(|value| value.as_str()).unwrap_or(""));
        }
    }
    String::new()
}

fn search_result_is_usable(title: &str, link: &str, existing: &[(String, String, String)]) -> bool {
    !title.is_empty()
        && validate_public_web_url(link, "result_url").is_ok()
        && !is_search_engine_navigation(link)
        && !existing
            .iter()
            .any(|(_, existing_link, _)| existing_link == link)
}

fn is_search_engine_navigation(link: &str) -> bool {
    let normalized = link.to_ascii_lowercase();
    normalized.contains("google.com/search?")
        || normalized.contains("google.com/url?")
        || normalized.contains("bing.com/search?")
        || normalized.contains("bing.com/ck/a?")
        || normalized.contains("duckduckgo.com/?q=")
}

fn normalize_google_url(href: &str) -> String {
    let value = normalize_search_url(href);
    if value.starts_with("/url?") || value.contains("google.com/url?") {
        if let Some(encoded) = query_param(&value, "q").or_else(|| query_param(&value, "url")) {
            return percent_decode_form(&encoded);
        }
    }
    value
}

fn validate_public_web_addresses(addresses: &[std::net::SocketAddr]) -> Result<(), ToolOutcome> {
    if addresses.is_empty() || addresses.iter().any(|address| is_blocked_public_web_host(&address.ip().to_string())) {
        return Err(error("Fetch DNS resolved to a non-public address".into()));
    }
    Ok(())
}
