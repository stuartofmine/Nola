fn current_process_tree_usage() -> Result<ProcessGroupUsage, String> {
    process_tree_usage(std::process::id(), None)
}

pub(crate) fn process_tree_usage(
    root_pid: u32,
    excluded_pid: Option<u32>,
) -> Result<ProcessGroupUsage, String> {
    process_tree_usage_if_present(root_pid, excluded_pid)?.ok_or_else(||
        format!("process-tree sampler found no root process for pid {root_pid}"))
}

/// Absence is an observation, not a zero-valued resource sample. A caller
/// owning Child must separately prove exit before treating it as completion.
pub(crate) fn process_tree_usage_if_present(
    root_pid: u32,
    excluded_pid: Option<u32>,
) -> Result<Option<ProcessGroupUsage>, String> {
    #[cfg(windows)]
    {
        let samples = windows_process_samples()?;
        if !samples.iter().any(|sample| sample.0 == root_pid) { return Ok(None); }
        return aggregate_process_tree_usage(root_pid, excluded_pid, &samples).map(Some);
    }
    #[cfg(not(windows))]
    {
        let mut command = Command::new("ps");
        command
            .args(["-axo", "pid=,ppid=,rss=,time="])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        command.process_group(0);
        let (sampler_pid, output) = process_control::bounded_process_output(
            command, Duration::from_millis(500), 8 * 1024 * 1024,
        )?;
        if !output.status.success() {
            return Err(format!(
                "process-tree sampler exited with {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        let samples = parse_ps_process_samples(&output.stdout)?;
        if !samples.iter().any(|sample| sample.0 == root_pid) { return Ok(None); }
        aggregate_process_tree_usage(root_pid, excluded_pid.or(Some(sampler_pid)), &samples).map(Some)
    }
}

#[cfg(any(not(windows), test))]
fn parse_ps_process_samples(bytes: &[u8]) -> Result<Vec<(u32, u32, u64, u64)>, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "process sample is not UTF-8")?;
    let mut samples = Vec::new();
    let mut identities = std::collections::BTreeSet::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() != 4 { return Err("process sample has an invalid field count".into()); }
        let pid = fields[0].parse::<u32>().map_err(|_| "process sample has an invalid PID")?;
        let parent = fields[1].parse::<u32>().map_err(|_| "process sample has an invalid parent PID")?;
        let rss = fields[2].parse::<u64>().ok().and_then(|value| value.checked_mul(1_024))
            .ok_or("process sample has invalid or overflowing RSS")?;
        let cpu = parse_process_cpu_time_ms(fields[3]).ok_or("process sample has invalid or overflowing CPU time")?;
        if !identities.insert(pid) { return Err("process snapshot contains duplicate PID identities".into()); }
        samples.push((pid, parent, rss, cpu));
    }
    if samples.is_empty() { return Err("process snapshot is empty".into()); }
    Ok(samples)
}

#[cfg(unix)]
pub(crate) fn terminate_external_process_tree(root_pid: u32) {
    let mut members = process_tree_usage(root_pid, None)
        .map(|usage| usage.pids)
        .unwrap_or_else(|_| vec![root_pid]);
    members.sort_unstable();
    members.dedup();
    let signal = |signal| {
        if let Ok(group) = i32::try_from(root_pid) {
            unsafe {
                libc::kill(-group, signal);
            }
        }
        for pid in members.iter().rev().copied() {
            if let Ok(pid) = i32::try_from(pid) {
                unsafe {
                    libc::kill(pid, signal);
                }
            }
        }
    };
    signal(libc::SIGTERM);
    std::thread::sleep(Duration::from_millis(150));
    signal(libc::SIGKILL);
}

#[cfg(windows)]
pub(crate) fn terminate_external_process_tree(root_pid: u32) {
    let mut members = process_tree_usage(root_pid, None)
        .map(|usage| usage.pids)
        .unwrap_or_else(|_| vec![root_pid]);
    members.retain(|pid| *pid != root_pid);
    members.sort_unstable();
    members.dedup();
    for pid in members.into_iter().rev() {
        terminate_windows_process(pid);
    }
    terminate_windows_process(root_pid);
}

#[cfg(windows)]
fn terminate_windows_process(pid: u32) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
    let handle = unsafe { OpenProcess(PROCESS_TERMINATE, 0, pid) };
    if handle.is_null() {
        return;
    }
    unsafe {
        TerminateProcess(handle, 1);
        CloseHandle(handle);
    }
}

#[cfg(windows)]
fn windows_process_samples() -> Result<Vec<(u32, u32, u64, u64)>, String> {
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::ProcessStatus::{
        K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION,
        PROCESS_VM_READ,
    };

    struct Handle(windows_sys::Win32::Foundation::HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
                unsafe {
                    CloseHandle(self.0);
                }
            }
        }
    }

    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(format!(
            "create Windows process snapshot failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    let snapshot = Handle(snapshot);
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = u32::try_from(std::mem::size_of::<PROCESSENTRY32W>())
        .map_err(|_| "Windows process entry size overflow".to_string())?;
    if unsafe { Process32FirstW(snapshot.0, &mut entry) } == 0 {
        return Err(format!(
            "enumerate Windows processes failed: {}",
            std::io::Error::last_os_error()
        ));
    }

    let mut samples = Vec::new();
    loop {
        let pid = entry.th32ProcessID;
        let parent_pid = entry.th32ParentProcessID;
        let mut memory_bytes = 0_u64;
        let mut cpu_time_ms = 0_u64;
        let process = unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_QUERY_INFORMATION | PROCESS_VM_READ,
                0,
                pid,
            )
        };
        if !process.is_null() {
            let process = Handle(process);
            let mut created: FILETIME = unsafe { std::mem::zeroed() };
            let mut exited: FILETIME = unsafe { std::mem::zeroed() };
            let mut kernel: FILETIME = unsafe { std::mem::zeroed() };
            let mut user: FILETIME = unsafe { std::mem::zeroed() };
            if unsafe {
                GetProcessTimes(process.0, &mut created, &mut exited, &mut kernel, &mut user)
            } != 0
            {
                cpu_time_ms =
                    windows_filetime_parts_to_ms(kernel.dwLowDateTime, kernel.dwHighDateTime)
                        .saturating_add(windows_filetime_parts_to_ms(
                            user.dwLowDateTime,
                            user.dwHighDateTime,
                        ));
            }
            let mut counters: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
            counters.cb =
                u32::try_from(std::mem::size_of::<PROCESS_MEMORY_COUNTERS>()).unwrap_or(u32::MAX);
            if unsafe { K32GetProcessMemoryInfo(process.0, &mut counters, counters.cb) } != 0 {
                memory_bytes = u64::try_from(counters.WorkingSetSize).unwrap_or(u64::MAX);
            }
        }
        samples.push((pid, parent_pid, memory_bytes, cpu_time_ms));
        if unsafe { Process32NextW(snapshot.0, &mut entry) } == 0 {
            break;
        }
    }
    Ok(samples)
}

fn windows_filetime_parts_to_ms(low: u32, high: u32) -> u64 {
    ((u64::from(high) << 32) | u64::from(low)) / 10_000
}

fn aggregate_process_tree_usage(
    root_pid: u32,
    excluded_pid: Option<u32>,
    samples: &[(u32, u32, u64, u64)],
) -> Result<ProcessGroupUsage, String> {
    if !samples.iter().any(|sample| sample.0 == root_pid) {
        return Err(format!(
            "process-tree sampler found no root process for pid {root_pid}"
        ));
    }
    let mut members = std::collections::BTreeSet::from([root_pid]);
    loop {
        let previous_len = members.len();
        for (pid, parent_pid, _, _) in samples {
            if Some(*pid) != excluded_pid && members.contains(parent_pid) {
                members.insert(*pid);
            }
        }
        if members.len() == previous_len {
            break;
        }
    }
    let mut usage = ProcessGroupUsage::default();
    for (pid, _, memory_bytes, cpu_time_ms) in samples {
        if !members.contains(pid) || Some(*pid) == excluded_pid {
            continue;
        }
        usage.cpu_time_ms = usage.cpu_time_ms.checked_add(*cpu_time_ms)
            .ok_or("process snapshot CPU sum overflow")?;
        usage.memory_bytes = usage.memory_bytes.checked_add(*memory_bytes)
            .ok_or("process snapshot RSS sum overflow")?;
        usage.processes = usage.processes.checked_add(1)
            .ok_or("process snapshot process count overflow")?;
        usage.pids.push(*pid);
    }
    Ok(usage)
}

fn parse_process_cpu_time_ms(value: &str) -> Option<u64> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit() || matches!(byte, b'-' | b':' | b'.')) {
        return None;
    }
    let (days, clock) = match value.split_once('-') {
        Some((days, clock)) => (days.parse::<u64>().ok()?, clock),
        None => (0, value),
    };
    let fields = clock.split(':').collect::<Vec<_>>();
    let (hours, minutes, seconds) = match fields.as_slice() {
        [minutes, seconds] => (
            0,
            minutes.parse::<u64>().ok()?,
            seconds.parse::<f64>().ok()?,
        ),
        [hours, minutes, seconds] => (
            hours.parse::<u64>().ok()?,
            minutes.parse::<u64>().ok()?,
            seconds.parse::<f64>().ok()?,
        ),
        _ => return None,
    };
    if !seconds.is_finite() || !(0.0..60.0).contains(&seconds)
        || (fields.len() == 3 && minutes >= 60)
        || (value.contains('-') && (fields.len() != 3 || hours >= 24)) {
        return None;
    }
    days.checked_mul(86_400)?
        .checked_add(hours.checked_mul(3_600)?)?
        .checked_add(minutes.checked_mul(60)?)?
        .checked_mul(1_000)?
        .checked_add((seconds * 1_000.0).round() as u64)
}

fn terminate_process_tree_descendants() {
    #[cfg(unix)]
    {
        let self_pid = std::process::id();
        let signal_members = |signal| {
            if let Ok(usage) = current_process_tree_usage() {
                for pid in usage.pids.into_iter().filter(|pid| *pid != self_pid) {
                    if let Ok(pid) = i32::try_from(pid) {
                        unsafe {
                            libc::kill(pid, signal);
                        }
                    }
                }
            }
        };
        signal_members(libc::SIGTERM);
        std::thread::sleep(Duration::from_millis(150));
        signal_members(libc::SIGKILL);
    }
    #[cfg(windows)]
    {
        let self_pid = std::process::id();
        if let Ok(usage) = current_process_tree_usage() {
            for pid in usage.pids.into_iter().filter(|pid| *pid != self_pid) {
                terminate_windows_process(pid);
            }
        }
    }
}

#[cfg(test)]
mod resource_supervisor_tests {
    use super::{
        aggregate_process_tree_usage, parse_process_cpu_time_ms, unix_epoch_millis,
        windows_filetime_parts_to_ms, ExternalExecutionTaskLease, parse_ps_process_samples,
    };

    #[test]
    fn parses_ps_cpu_time_without_losing_days_or_fractional_seconds() {
        assert_eq!(parse_process_cpu_time_ms("01:02"), Some(62_000));
        assert_eq!(parse_process_cpu_time_ms("01:02:03"), Some(3_723_000));
        assert_eq!(parse_process_cpu_time_ms("2-01:02:03.5"), Some(176_523_500));
        assert_eq!(parse_process_cpu_time_ms("invalid"), None);
    }

    #[test]
    fn invalid_process_measurements_are_not_zero_or_saturated_usage() {
        for value in ["00:NaN", "00:inf", "00:-1", "00:60", "01:60:00", "1-25:00:00",
            "18446744073709551615:00", "18446744073709551615-01:00:00", "1-01:00"] {
            assert_eq!(parse_process_cpu_time_ms(value), None, "{value}");
        }
        for input in ["", "10 1 bad 00:01", "10 1 18446744073709551615 00:01",
            "10 1 12 bad", "bad 1 12 00:01", "10 bad 12 00:01", "10 1 12",
            "10 1 12 00:01 extra", "10 1 12 00:01\n10 1 12 00:01"] {
            assert!(parse_ps_process_samples(input.as_bytes()).is_err(), "{input}");
        }
        assert!(parse_ps_process_samples(&[0xff]).is_err());
        assert_eq!(parse_ps_process_samples(b"10 1 0 00:00.00\n20 10 2 01:02.50\n").unwrap(),
            vec![(10, 1, 0, 0), (20, 10, 2048, 62_500)]);
        assert!(aggregate_process_tree_usage(10, None, &[(10, 1, 1, u64::MAX), (20, 10, 1, 1)]).is_err());
        assert!(aggregate_process_tree_usage(10, None, &[(10, 1, u64::MAX, 1), (20, 10, 1, 1)]).is_err());
    }

    #[test]
    fn converts_windows_filetime_parts_from_100ns_ticks_to_milliseconds() {
        assert_eq!(windows_filetime_parts_to_ms(10_000, 0), 1);
        assert_eq!(windows_filetime_parts_to_ms(0, 1), 429_496);
        assert_eq!(windows_filetime_parts_to_ms(u32::MAX, 1), 858_993);
    }

    #[cfg(unix)]
    #[test]
    fn samples_live_unix_process_with_strict_measurement_parsing() {
        let pid = std::process::id();
        let usage = super::process_tree_usage(pid, None).expect("sample the real test process");
        assert!(usage.pids.contains(&pid));
        assert!(usage.processes >= 1);
        assert!(usage.memory_bytes > 0);
    }

    #[cfg(windows)]
    #[test]
    fn samples_the_live_windows_test_process_without_shell_commands() {
        let usage = super::process_tree_usage(std::process::id(), None)
            .expect("sample live Windows process tree");
        assert!(usage.processes >= 1);
        assert!(usage.memory_bytes > 0);
        assert!(usage.pids.contains(&std::process::id()));
    }

    #[test]
    fn aggregates_descendants_across_process_groups_and_excludes_the_sampler() {
        let usage = aggregate_process_tree_usage(
            10,
            Some(99),
            &[
                (10, 1, 1_000, 10),
                (20, 10, 2_000, 20),
                (30, 20, 3_000, 30),
                (40, 1, 4_000, 40),
                (99, 10, 5_000, 50),
            ],
        )
        .expect("process tree should aggregate");

        assert_eq!(usage.processes, 3);
        assert_eq!(usage.memory_bytes, 6_000);
        assert_eq!(usage.cpu_time_ms, 60);
        assert_eq!(usage.pids, vec![10, 20, 30]);
    }

    #[test]
    fn external_executor_waits_for_the_same_fair_slot_as_a_shell_task() {
        let _environment = crate::store::test_env_lock();
        let root =
            std::env::temp_dir().join(format!("nola-external-supervisor-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("create external Supervisor test root");
        std::env::set_var("NOLA_DB_PATH", root.join("nola.db"));
        std::env::set_var("NOLA_MAX_CONCURRENT_TASKS", "1");
        std::env::set_var("NOLA_MAX_PROJECT_TASKS", "1");
        let mut config =
            crate::host_invocation_request::HostInvocationRequest::for_test().expect("external Supervisor test config");
        config.tool_settings.project_root = root.clone();
        config.tool_settings.allowed_dirs = vec![root.clone()];
        config.session.session_id = Some("external-supervisor-session".to_string());
        config.execution.execution_backend = "claude_code".to_string();

        let now = unix_epoch_millis();
        let mut blocker = crate::store::DbRuntimeTask {
            id: "shell-slot-blocker".to_string(),
            kind: "bash".to_string(),
            status: "queued".to_string(),
            title: "Shell slot blocker".to_string(),
            project_root: root.to_string_lossy().into_owned(),
            owner_session_id: config.session.session_id.clone(),
            parent_task_id: None,
            parent_tool_call_id: Some("shell-blocker-call".to_string()),
            child_session_id: None,
            agent_type: None,
            prompt: None,
            progress_summary: Some("queued".to_string()),
            last_activity: None,
            result_summary: None,
            error: None,
            output_path: None,
            unread: false,
            cancel_requested: false,
            cancel_requested_at_ms: None,
            worker_id: None,
            ownership_token: None,
            lease_until_ms: None,
            heartbeat_at_ms: None,
            retry_count: 0,
            retry_at_ms: None,
            last_error: None,
            input_watermark: Some(now),
            completion_watermark: None,
            created_at_ms: now,
            started_at_ms: None,
            ended_at_ms: None,
            updated_at_ms: now,
            metadata_json: "{}".to_string(),
        };
        blocker
            .set_scheduling(nola_application::RuntimeTaskScheduling {
                priority: nola_application::RuntimeTaskPriority::Normal,
                queued_at_ms: now,
                resource_budget: nola_application::RuntimeTaskResourceBudget::default(),
                output_bytes: 0,
                resource_usage: nola_application::RuntimeTaskResourceUsage::default(),
                budget_exhaustion: None,
            })
            .expect("set Shell blocker scheduling");
        let store = crate::store::NolaStore::open_default().expect("open Supervisor store");
        store
            .create_runtime_task(&blocker)
            .expect("create Shell slot blocker");
        let blocker_attempt = store
            .claim_runtime_task_fair(
                &blocker.id,
                "shell-worker",
                "shell-generation",
                "shell-token",
                now,
                now + 60_000,
                1,
                1,
            )
            .expect("claim Shell slot blocker")
            .expect("Shell blocker attempt");
        let blocker_lease =
            crate::runtime_task_port::register_runtime_task_attempt(blocker_attempt);

        let marker = root.join("external-started.txt");
        let worker_marker = marker.clone();
        let worker_config = config.clone();
        let cancellation = crate::runtime_control::RuntimeExecutionControl::default();
        let (acquired_tx, acquired_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let lease = ExternalExecutionTaskLease::acquire(
                &worker_config.external_execution_settings(),
                worker_config.session.session_id.clone(),
                "external-effect-1",
                "claude_code",
                &cancellation,
            )
            .expect("external executor must eventually acquire the shared slot");
            acquired_tx.send(()).unwrap();
            let status = std::process::Command::new(crate::platform_paths::bash_program())
                .arg("-c")
                .arg(format!(
                    "printf 'external-started\\n' > '{}'",
                    crate::platform_paths::shell_path(&worker_marker)
                ))
                .status()
                .expect("start real external test process");
            assert!(status.success());
            lease
                .finish_completed()
                .expect("commit external executor completion");
        });

        assert!(
            acquired_rx
                .recv_timeout(std::time::Duration::from_millis(300))
                .is_err(),
            "external executor bypassed the occupied Shell Supervisor slot"
        );
        assert!(
            !marker.exists(),
            "external process started before slot release"
        );
        let queued = store
            .list_runtime_tasks(&root.to_string_lossy())
            .expect("list queued external task");
        assert!(queued
            .iter()
            .any(|task| task.kind == "external_executor" && task.status == "queued"));

        assert!(store
            .finish_runtime_task_detailed_with_event(
                &blocker_lease,
                "completed",
                None,
                Some("completed"),
                None,
                None,
                None,
                false,
                "slot_released",
                "completed",
                None,
                "{}",
                unix_epoch_millis(),
            )
            .expect("release Shell Supervisor slot"));
        acquired_rx
            .recv_timeout(std::time::Duration::from_secs(3))
            .expect("external executor did not acquire the released slot");
        worker.join().expect("external Supervisor worker");
        assert_eq!(
            std::fs::read_to_string(&marker).expect("read external process marker"),
            "external-started\n"
        );
        let terminal = store
            .list_runtime_tasks(&root.to_string_lossy())
            .expect("list terminal external task")
            .into_iter()
            .find(|task| task.kind == "external_executor")
            .expect("external task persisted");
        assert_eq!(terminal.status, "completed");
        assert!(terminal.ended_at_ms.is_some());

        let mut abandoned = blocker.clone();
        abandoned.id = "external-abandoned-before-start".to_string();
        abandoned.kind = "external_executor".to_string();
        abandoned.status = "queued".to_string();
        abandoned.parent_tool_call_id = Some("abandoned-effect".to_string());
        abandoned.worker_id = None;
        abandoned.ownership_token = None;
        abandoned.lease_until_ms = None;
        abandoned.started_at_ms = None;
        abandoned.ended_at_ms = None;
        abandoned.cancel_requested = false;
        abandoned.cancel_requested_at_ms = None;
        abandoned.metadata_json = serde_json::json!({"host_pid":u32::MAX}).to_string();
        abandoned
            .set_scheduling(blocker.scheduling().unwrap().unwrap())
            .expect("preserve abandoned task scheduling");
        store
            .create_runtime_task(&abandoned)
            .expect("create abandoned queued external task");
        super::recover_orphaned_external_executor_tasks(&config);
        let abandoned = store
            .runtime_task(&abandoned.id)
            .expect("load abandoned external task")
            .expect("abandoned external task persisted");
        assert_eq!(abandoned.status, "cancelled");
        let events = store
            .runtime_task_events(&abandoned.id, 10)
            .expect("load abandoned external events");
        assert!(events.iter().any(|event| {
            event.kind == "abandoned_before_start"
                && event.detail_json.contains("\"safe_to_retry\":true")
        }));

        let mut readonly_mcp = blocker.clone();
        readonly_mcp.id = "mcp-readonly-cold-takeover".to_string();
        readonly_mcp.kind = "mcp".to_string();
        readonly_mcp.status = "queued".to_string();
        readonly_mcp.title = "docs / read_page".to_string();
        readonly_mcp.parent_tool_call_id = Some("readonly-effect-1".to_string());
        readonly_mcp.agent_type = Some("docs".to_string());
        readonly_mcp.last_activity = Some("read_page".to_string());
        readonly_mcp.worker_id = None;
        readonly_mcp.ownership_token = None;
        readonly_mcp.lease_until_ms = None;
        readonly_mcp.started_at_ms = None;
        readonly_mcp.ended_at_ms = None;
        // The expired historical claim below still follows task creation and
        // queue admission; it must not manufacture a backwards Task timeline.
        readonly_mcp.created_at_ms = now - 300;
        readonly_mcp.updated_at_ms = now - 300;
        readonly_mcp.metadata_json = serde_json::json!({
            "host_pid": u32::MAX,
            "read_only": true,
            "arguments_digest": super::McpExecutionTaskLease::arguments_digest(&serde_json::json!({})),
            "safe_to_retry_before_claim": true,
            "safe_to_retry_after_claim": true,
            "server_id": "docs",
            "tool_name": "read_page",
            "parent_effect_id": "readonly-effect-1"
        })
        .to_string();
        let mut readonly_scheduling = blocker.scheduling().unwrap().unwrap();
        readonly_scheduling.queued_at_ms = now - 300;
        readonly_mcp
            .set_scheduling(readonly_scheduling)
            .expect("preserve read-only MCP scheduling");
        store
            .create_runtime_task(&readonly_mcp)
            .expect("create read-only MCP task");
        let choose = |rows| super::McpExecutionTaskLease::recover_task_id(
            rows, &readonly_mcp.owner_session_id, "readonly-effect-1", "docs", "read_page", true,
            &super::McpExecutionTaskLease::arguments_digest(&serde_json::json!({})),
        );
        assert!(choose(Err("injected recovery read failure".into()))
            .unwrap_err().contains("readonly-effect-1"));
        let mut deferred = readonly_mcp.clone();
        deferred.status = "retry_waiting".into();
        deferred.retry_at_ms = Some(now + 60_000);
        assert_eq!(choose(Ok(vec![deferred])).unwrap(), Some(readonly_mcp.id.clone()));
        for state in ["running", "completed", "failed", "cancelled"] {
            let mut original = readonly_mcp.clone();
            original.status = state.into();
            assert!(choose(Ok(vec![original])).unwrap_err().contains(&readonly_mcp.id));
        }
        let mut mismatched = readonly_mcp.clone();
        mismatched.last_activity = Some("different_tool".into());
        assert!(choose(Ok(vec![mismatched])).is_err());
        let mut corrupt = readonly_mcp.clone();
        corrupt.metadata_json = "{".into();
        assert!(choose(Ok(vec![corrupt])).is_err());
        let mut changed_arguments = readonly_mcp.clone();
        let mut metadata: serde_json::Value = serde_json::from_str(&changed_arguments.metadata_json).unwrap();
        metadata["arguments_digest"] = serde_json::json!(
            super::McpExecutionTaskLease::arguments_digest(&serde_json::json!({"path":"different"})));
        changed_arguments.metadata_json = metadata.to_string();
        assert!(choose(Ok(vec![changed_arguments])).is_err());
        let mut duplicate = readonly_mcp.clone();
        duplicate.id = "duplicate-effect-task".into();
        assert!(choose(Ok(vec![readonly_mcp.clone(), duplicate])).is_err());
        // Injected recovery failures do not write or replace the durable task.
        assert_eq!(store.runtime_task(&readonly_mcp.id).unwrap().unwrap().status, "queued");
        assert_eq!(store.list_runtime_tasks(&readonly_mcp.project_root).unwrap().iter()
            .filter(|task| task.parent_tool_call_id == readonly_mcp.parent_tool_call_id).count(), 1);
        assert!(store
            .claim_runtime_task(
                &readonly_mcp.id,
                "dead-mcp-host",
                "dead-mcp-generation",
                "expired-readonly-token",
                now - 200,
                now - 100,
            )
            .expect("claim read-only MCP task with expired lease")
            .is_some());
        super::recover_orphaned_external_executor_tasks(&config);
        let recovered_readonly = store
            .runtime_task(&readonly_mcp.id)
            .expect("load recovered read-only MCP task")
            .expect("read-only MCP task persisted");
        assert_eq!(recovered_readonly.status, "retry_waiting");
        assert!(recovered_readonly.retry_count >= 1);
        let cancellation = crate::runtime_control::RuntimeExecutionControl::default();
        let lease = super::McpExecutionTaskLease::acquire(
            &config.tool_settings.project_root,
            config.session.session_id.clone(),
            "readonly-effect-1",
            "docs",
            "read_page",
            true,
            &serde_json::json!({}),
            &cancellation,
        )
        .expect("reclaim the same read-only MCP invocation");
        assert_eq!(lease.task_id, readonly_mcp.id);
        lease
            .finish_completed()
            .expect("finish reclaimed read-only MCP invocation");
        let readonly_events = store
            .runtime_task_events(&readonly_mcp.id, 20)
            .expect("load read-only MCP recovery events");
        assert!(readonly_events.iter().any(|event| {
            event.kind == "retry_scheduled" && event.detail_json.contains("\"safe_to_retry\":true")
        }));
        assert!(readonly_events
            .iter()
            .any(|event| event.kind == "takeover_attempted"));

        let mut mutating_mcp = readonly_mcp.clone();
        mutating_mcp.id = "mcp-mutate-cold-reconciliation".to_string();
        mutating_mcp.title = "docs / write_page".to_string();
        mutating_mcp.parent_tool_call_id = Some("mutate-effect-1".to_string());
        mutating_mcp.last_activity = Some("write_page".to_string());
        mutating_mcp.status = "queued".to_string();
        mutating_mcp.worker_id = None;
        mutating_mcp.ownership_token = None;
        mutating_mcp.lease_until_ms = None;
        mutating_mcp.started_at_ms = None;
        mutating_mcp.ended_at_ms = None;
        mutating_mcp.retry_count = 0;
        mutating_mcp.metadata_json = serde_json::json!({
            "host_pid": u32::MAX,
            "read_only": false,
            "safe_to_retry_before_claim": true,
            "safe_to_retry_after_claim": false,
            "server_id": "docs",
            "tool_name": "write_page",
            "parent_effect_id": "mutate-effect-1"
        })
        .to_string();
        mutating_mcp
            .set_scheduling(blocker.scheduling().unwrap().unwrap())
            .expect("preserve mutating MCP scheduling");
        store
            .create_runtime_task(&mutating_mcp)
            .expect("create mutating MCP task");
        assert!(store
            .claim_runtime_task(
                &mutating_mcp.id,
                "dead-mcp-host",
                "dead-mcp-generation",
                "expired-mutate-token",
                now - 200,
                now - 100,
            )
            .expect("claim mutating MCP task with expired lease")
            .is_some());
        super::recover_orphaned_external_executor_tasks(&config);
        let recovered_mutation = store
            .runtime_task(&mutating_mcp.id)
            .expect("load recovered mutating MCP task")
            .expect("mutating MCP task persisted");
        assert_eq!(recovered_mutation.status, "failed");
        assert_eq!(recovered_mutation.retry_count, 0);
        let mutation_events = store
            .runtime_task_events(&mutating_mcp.id, 20)
            .expect("load mutating MCP recovery events");
        assert!(mutation_events.iter().any(|event| {
            event.kind == "reconciliation_required"
                && event.detail_json.contains("\"safe_to_retry\":false")
        }));

        // Two first callers that both observed absence contend on the same
        // canonical Store key, not two randomly allocated task identities.
        let mut racing_task = readonly_mcp.clone();
        racing_task.parent_tool_call_id = Some("simultaneous-effect".into());
        racing_task.id = super::McpExecutionTaskLease::effect_task_id(
            &racing_task.project_root, &racing_task.owner_session_id, "simultaneous-effect",
        );
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let callers: Vec<_> = (0..2).map(|_| {
            let task = racing_task.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let store = crate::store::NolaStore::open_default().unwrap();
                barrier.wait();
                store.create_runtime_task(&task)
            })
        }).collect();
        let outcomes: Vec<_> = callers.into_iter().map(|caller| caller.join().unwrap()).collect();
        assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
        assert!(store.create_runtime_task(&racing_task).is_err());
        assert_eq!(store.list_runtime_tasks(&racing_task.project_root).unwrap().iter()
            .filter(|task| task.parent_tool_call_id == racing_task.parent_tool_call_id).count(), 1);
        assert_eq!(store.runtime_task_events(&racing_task.id, 10).unwrap().iter()
            .filter(|event| event.kind == "created").count(), 1);
        let mut conflict = racing_task.clone();
        conflict.owner_session_id = Some("foreign-owner".into());
        conflict.last_activity = Some("different-tool".into());
        assert!(store.create_runtime_task(&conflict).is_err());
        let original = store.runtime_task(&racing_task.id).unwrap().unwrap();
        assert_eq!(original.owner_session_id, racing_task.owner_session_id);
        assert_eq!(original.last_activity, racing_task.last_activity);

        std::env::remove_var("NOLA_MAX_PROJECT_TASKS");
        std::env::remove_var("NOLA_MAX_CONCURRENT_TASKS");
        std::env::remove_var("NOLA_DB_PATH");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn mcp_effect_identity_is_framed_and_keeps_optional_owner_distinct() {
        let id = super::McpExecutionTaskLease::effect_task_id;
        assert_eq!(id("/workspace", &Some("owner".into()), "effect"),
            id("/workspace", &Some("owner".into()), "effect"));
        assert_ne!(id("/workspace", &None, "effect"),
            id("/workspace", &Some("null".into()), "effect"));
        assert_ne!(id("/workspace", &Some("a:b".into()), "c"),
            id("/workspace", &Some("a".into()), "b:c"));
    }

    fn isolated_mcp_host_config(root: &std::path::Path) -> crate::host_invocation_request::HostInvocationRequest {
        crate::config::Config::from_args([
            "nola".into(), "--project".into(), root.to_string_lossy().into_owned(),
            "--endpoint".into(), "http://127.0.0.1:9/v1".into(),
            "--api-key".into(), "fixture-no-provider".into(),
            "--model".into(), "fixture-no-model".into(),
        ]).and_then(|config| config.host_invocation_request()).expect("isolated MCP Host configuration")
    }

    #[test]
    fn mcp_host_kill_fixture() {
        let Some(marker) = std::env::var_os("NOLA_TEST_MCP_HOST_MARKER") else {
            return;
        };
        let root = std::path::PathBuf::from(
            std::env::var_os("NOLA_TEST_MCP_HOST_ROOT").expect("fixture root"),
        );
        let read_only = std::env::var("NOLA_TEST_MCP_READ_ONLY").as_deref() == Ok("1");
        let mut config = isolated_mcp_host_config(&root);
        config.tool_settings.project_root = root.clone();
        config.tool_settings.allowed_dirs = vec![root];
        config.session.session_id = Some("mcp-host-kill-session".to_string());
        let cancellation = crate::runtime_control::RuntimeExecutionControl::default();
        let lease = super::McpExecutionTaskLease::acquire(
            &config.tool_settings.project_root,
            config.session.session_id.clone(),
            if read_only {
                "host-kill-readonly-effect"
            } else {
                "host-kill-mutate-effect"
            },
            "host-kill-server",
            if read_only { "read" } else { "write" },
            read_only,
            &serde_json::json!({}),
            &cancellation,
        )
        .expect("fixture acquires MCP lease");
        std::fs::write(marker, &lease.task_id).expect("publish acquired task id");
        if let Some(endpoint) = std::env::var_os("NOLA_TEST_MCP_HOST_ENDPOINT") {
            let endpoint = endpoint.to_string_lossy().into_owned();
            let discovered = crate::mcp_connection_authority::connect_fixture_servers(&[serde_json::json!({
                "server_id":"host-kill-http-server",
                "endpoint": endpoint,
                "timeout_ms": 120_000
            })])
            .expect("fixture discovers real HTTP MCP server");
            let connection_ref = discovered[0]["connection_ref"]
                .as_str()
                .expect("fixture connection ref");
            let cancellation = crate::runtime_control::RuntimeExecutionControl::default();
            let _ = crate::mcp_host::call_tool_with_cancellation(
                nola_core::agent::capability::McpTransportKind::StreamableHttp,
                connection_ref,
                if read_only { "read" } else { "write" },
                serde_json::json!({
                    "invocation_id": if read_only {
                        "host-kill-readonly-effect"
                    } else {
                        "host-kill-mutate-effect"
                    }
                }),
                &cancellation,
            );
        }
        std::thread::sleep(std::time::Duration::from_secs(120));
        std::mem::forget(lease);
    }

    fn host_kill_http_mcp_server(mutation_file: Option<std::path::PathBuf>) -> (
        String,
        std::sync::Arc<std::sync::atomic::AtomicUsize>,
        std::sync::Arc<std::sync::atomic::AtomicBool>,
        std::sync::Arc<std::sync::atomic::AtomicBool>,
        std::sync::Arc<std::sync::atomic::AtomicBool>,
        std::thread::JoinHandle<()>,
    ) {
        use std::io::{Read, Write};
        use std::sync::atomic::Ordering;

        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").expect("bind Host kill HTTP MCP server");
        listener
            .set_nonblocking(true)
            .expect("configure nonblocking MCP listener");
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let tool_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let first_call_received = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let release_first_call = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let shutdown = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let calls = std::sync::Arc::clone(&tool_calls);
        let received = std::sync::Arc::clone(&first_call_received);
        let release = std::sync::Arc::clone(&release_first_call);
        let stop = std::sync::Arc::clone(&shutdown);
        let handle = std::thread::spawn(move || {
            let mut handlers = Vec::new();
            while !stop.load(Ordering::SeqCst) {
                let (mut stream, _) = match listener.accept() {
                    Ok(value) => value,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                        continue;
                    }
                    Err(error) => panic!("accept Host kill MCP request: {error}"),
                };
                stream
                    .set_nonblocking(false)
                    .expect("configure blocking Host kill MCP stream");
                let calls = std::sync::Arc::clone(&calls);
                let received = std::sync::Arc::clone(&received);
                let release = std::sync::Arc::clone(&release);
                let mutation_file = mutation_file.clone();
                handlers.push(std::thread::spawn(move || {
                    let mut request = Vec::new();
                    let mut buffer = [0_u8; 4096];
                    let header_end = loop {
                        let count = stream.read(&mut buffer).expect("read Host kill MCP request");
                        if count == 0 {
                            return;
                        }
                        request.extend_from_slice(&buffer[..count]);
                        if let Some(position) =
                            request.windows(4).position(|value| value == b"\r\n\r\n")
                        {
                            break position + 4;
                        }
                    };
                    let headers = String::from_utf8_lossy(&request[..header_end]);
                    let content_length = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().ok())
                                .flatten()
                        })
                        .unwrap_or_default();
                    while request.len() < header_end + content_length {
                        let count = stream.read(&mut buffer).expect("read Host kill MCP body");
                        if count == 0 {
                            break;
                        }
                        request.extend_from_slice(&buffer[..count]);
                    }
                    let payload: serde_json::Value = serde_json::from_slice(
                        &request[header_end..request.len().min(header_end + content_length)],
                    )
                    .expect("decode Host kill MCP request");
                    let method = payload["method"].as_str().unwrap_or_default();
                    let id = payload.get("id").cloned();
                    if method == "tools/call" {
                        if let Some(path) = &mutation_file {
                            let mut file = std::fs::OpenOptions::new().create(true).append(true)
                                .open(path).expect("open isolated mutation counter");
                            file.write_all(b"applied\n").expect("persist isolated mutation");
                            file.sync_all().expect("flush mutation before Host crash");
                        }
                        let ordinal = calls.fetch_add(1, Ordering::SeqCst) + 1;
                        if ordinal == 1 {
                            received.store(true, Ordering::SeqCst);
                            while !release.load(Ordering::SeqCst) {
                                std::thread::sleep(std::time::Duration::from_millis(5));
                            }
                            return;
                        }
                    }
                    let response = match method {
                        "notifications/initialized" => String::new(),
                        "initialize" => serde_json::json!({
                            "jsonrpc":"2.0", "id":id,
                            "result":{"protocolVersion":"2025-03-26","capabilities":{}}
                        })
                        .to_string(),
                        "tools/list" => serde_json::json!({
                            "jsonrpc":"2.0", "id":id,
                            "result":{"tools":[
                                {"name":"read","description":"read-only test","inputSchema":{"type":"object"},"annotations":{"readOnlyHint":true}},
                                {"name":"write","description":"mutation test","inputSchema":{"type":"object"}}
                            ]}
                        })
                        .to_string(),
                        "resources/list" | "resources/templates/list" | "prompts/list" =>
                            serde_json::json!({
                                "jsonrpc":"2.0", "id":id,
                                "error":{"code":-32601,"message":"Method not found"}
                            })
                            .to_string(),
                        "tools/call" => serde_json::json!({
                            "jsonrpc":"2.0", "id":id,
                            "result":{"content":[{"type":"text","text":"stable-result"}]}
                        })
                        .to_string(),
                        unexpected => panic!("unexpected Host kill MCP method {unexpected}"),
                    };
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        response.len(),
                        response
                    )
                    .expect("write Host kill MCP response");
                }));
            }
            release.store(true, Ordering::SeqCst);
            for handler in handlers {
                handler.join().expect("Host kill MCP handler");
            }
        });
        (
            endpoint,
            tool_calls,
            first_call_received,
            release_first_call,
            shutdown,
            handle,
        )
    }

    #[test]
    fn killed_host_is_cold_recovered_by_replay_safety() {
        let _environment = crate::store::test_env_lock();
        // reqwest on macOS can inherit the system proxy even without proxy
        // environment variables. This fixture only talks to its loopback server.
        struct RestoreEnvironment(Vec<(&'static str, Option<std::ffi::OsString>)>);
        impl Drop for RestoreEnvironment {
            fn drop(&mut self) {
                for (key, value) in &self.0 { match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }}
            }
        }
        let _environment_values = RestoreEnvironment([
            "NO_PROXY", "NOLA_HOME", "NOLA_DB_PATH", "NOLA_MAX_CONCURRENT_TASKS", "NOLA_MAX_PROJECT_TASKS"
        ].into_iter().map(|key| (key, std::env::var_os(key))).collect());
        std::env::set_var("NO_PROXY", "127.0.0.1,localhost,::1");
        let root =
            std::env::temp_dir().join(format!("nola-mcp-host-kill-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("create MCP Host kill root");
        let database = root.join("nola.db");
        std::env::set_var("NOLA_HOME", &root);
        std::env::set_var("NOLA_DB_PATH", &database);
        std::env::set_var("NOLA_MAX_CONCURRENT_TASKS", "2");
        std::env::set_var("NOLA_MAX_PROJECT_TASKS", "2");
        let mut config = isolated_mcp_host_config(&root);
        config.tool_settings.project_root = root.clone();
        config.tool_settings.allowed_dirs = vec![root.clone()];
        config.session.session_id = Some("mcp-host-kill-session".to_string());

        for read_only in [true, false] {
            let (
                endpoint,
                remote_calls,
                first_remote_call,
                release_remote_call,
                shutdown_server,
                server,
            ) = host_kill_http_mcp_server((!read_only).then(|| root.join("mutations.log")));
            let marker = root.join(if read_only {
                "readonly-ready"
            } else {
                "mutate-ready"
            });
            let mut child = std::process::Command::new(
                std::env::current_exe().expect("resolve current test executable"),
            )
            .args([
                "--exact",
                "task_runtime::resource_supervisor_tests::mcp_host_kill_fixture",
                "--nocapture",
            ])
            .env("NOLA_HOME", &root)
            .env("NOLA_DB_PATH", &database)
            .env("NOLA_MAX_CONCURRENT_TASKS", "2")
            .env("NOLA_MAX_PROJECT_TASKS", "2")
            .env("NOLA_TEST_MCP_HOST_ROOT", &root)
            .env("NOLA_TEST_MCP_HOST_MARKER", &marker)
            .env("NOLA_TEST_MCP_HOST_ENDPOINT", &endpoint)
            .env("NO_PROXY", "127.0.0.1,localhost,::1")
            .env("NOLA_TEST_MCP_READ_ONLY", if read_only { "1" } else { "0" })
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::inherit())
            .spawn()
            .expect("spawn isolated MCP Host fixture");
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
            while (!marker.exists() || !first_remote_call.load(std::sync::atomic::Ordering::SeqCst))
                && std::time::Instant::now() < deadline
            {
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            if !marker.exists() || !first_remote_call.load(std::sync::atomic::Ordering::SeqCst) {
                let _ = child.kill();
                let _ = child.wait();
                release_remote_call.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            assert!(
                marker.exists(),
                "isolated MCP Host did not acquire its lease"
            );
            assert!(
                first_remote_call.load(std::sync::atomic::Ordering::SeqCst),
                "real HTTP MCP Server did not receive the first tools/call"
            );
            let task_id = std::fs::read_to_string(&marker).expect("read fixture task id");
            if !read_only {
                assert_eq!(std::fs::read(root.join("mutations.log")).unwrap(), b"applied\n",
                    "the file side effect must be durable before killing the Host");
            }
            child.kill().expect("kill isolated MCP Host");
            let exit = child.wait().expect("reap killed isolated Host");
            assert!(!exit.success(), "this fixture requires an actual process failure");
            release_remote_call.store(true, std::sync::atomic::Ordering::SeqCst);

            super::expire_task_lease_for_test(&task_id, super::unix_epoch_millis())
                .expect("expire killed Host lease at the test clock boundary");
            super::recover_orphaned_external_executor_tasks(&config);
            let store = crate::store::NolaStore::open_default().expect("open recovered task store");
            let recovered = store
                .runtime_task(&task_id)
                .expect("load recovered MCP task")
                .expect("recovered MCP task exists");
            if read_only {
                assert_eq!(recovered.status, "retry_waiting");
                let cancellation = crate::runtime_control::RuntimeExecutionControl::default();
                let lease = super::McpExecutionTaskLease::acquire(
                    &config.tool_settings.project_root,
                    config.session.session_id.clone(),
                    "host-kill-readonly-effect",
                    "host-kill-server",
                    "read",
                    true,
                    &serde_json::json!({}),
                    &cancellation,
                )
                .expect("second Host reclaims read-only invocation");
                assert_eq!(lease.task_id, task_id);
                let discovered =
                    crate::mcp_connection_authority::connect_fixture_servers(&[serde_json::json!({
                        "server_id":"host-kill-http-server",
                        "endpoint":endpoint,
                        "timeout_ms":5_000
                    })])
                    .expect("second Host discovers the same HTTP MCP server");
                let execution = crate::mcp_host::call_tool_with_cancellation(
                    nola_core::agent::capability::McpTransportKind::StreamableHttp,
                    discovered[0]["connection_ref"].as_str().unwrap(),
                    "read",
                    serde_json::json!({"invocation_id":"host-kill-readonly-effect"}),
                    &cancellation,
                )
                .expect("second Host obtains the read-only result");
                assert_eq!(execution.output["content"][0]["text"], "stable-result");
                lease
                    .finish_completed()
                    .expect("finish reclaimed invocation");
                assert_eq!(remote_calls.load(std::sync::atomic::Ordering::SeqCst), 2);
            } else {
                assert_eq!(recovered.status, "failed");
                assert_eq!(recovered.retry_count, 0);
                let events = store
                    .runtime_task_events(&task_id, 20)
                    .expect("load mutate recovery events");
                assert!(events
                    .iter()
                    .any(|event| event.kind == "reconciliation_required"));
                std::thread::sleep(std::time::Duration::from_millis(100));
                assert_eq!(remote_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
                for _ in 0..3 { super::recover_orphaned_external_executor_tasks(&config); }
                let reopened = crate::store::NolaStore::open_default().unwrap();
                let after = reopened.runtime_task(&task_id).unwrap().unwrap();
                assert_eq!(after.status, "failed");
                assert_eq!(after.retry_count, 0);
                assert_eq!(remote_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
                assert_eq!(std::fs::read(root.join("mutations.log")).unwrap(), b"applied\n",
                    "cold recovery must not repeat a mutation whose result was lost");
                let cancellation = crate::runtime_control::RuntimeExecutionControl::default();
                assert!(super::McpExecutionTaskLease::acquire(
                    &config.tool_settings.project_root, config.session.session_id.clone(),
                    "host-kill-mutate-effect", "host-kill-server", "write", false,
                    &serde_json::json!({}), &cancellation).is_err(),
                    "an unreconciled write must not be admitted again under its original effect identity");
                eprintln!("file-crash-evidence: task={task_id} file_appends=1 remote_calls=1 retries=0 reconciliation_required=true host_exit={exit}");
            }
            shutdown_server.store(true, std::sync::atomic::Ordering::SeqCst);
            server.join().expect("stop Host kill HTTP MCP server");
        }

        if std::env::var_os("NOLA_TEST_KEEP_MCP_HOST_KILL_ARTIFACTS").is_some() {
            eprintln!("file-crash-artifacts: {}", root.display());
        } else {
            let _ = std::fs::remove_dir_all(root);
        }
    }
}

fn next_task_operation_id(kind: &str, task_id: &str) -> String {
    let sequence = TASK_OPERATION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("{kind}:{task_id}:{sequence}")
}

fn list_tasks(config: &HostInvocationRequest) -> ToolOutcome {
    let project_root = config.tool_settings.project_root.to_string_lossy();
    let mut tasks =
        match NolaStore::open_default().and_then(|store| store.list_runtime_tasks(&project_root)) {
            Ok(tasks) => tasks,
            Err(error) => return task_error(error),
        };
    tasks.retain(|task| task_scope_matches(config, task));
    let items = tasks.iter().map(task_json).collect::<Vec<_>>();
    task_ok(
        json!({"tasks": items, "count": items.len()}).to_string(),
        format!("{} {}", items.len(), ui_text(UiText::TasksUnit)),
    )
}

fn render_task_list(config: &HostInvocationRequest) -> ToolOutcome {
    let root = config.tool_settings.project_root.to_string_lossy();
    let mut tasks =
        match NolaStore::open_default().and_then(|store| store.list_runtime_tasks(&root)) {
            Ok(tasks) => tasks,
            Err(error) => return task_error(error),
        };
    tasks.retain(|task| task_scope_matches(config, task));
    let core_projection = config.session.session_id.as_deref().and_then(|id| {
        crate::session::load_session_task_projection(id)
            .ok()
            .flatten()
    });
    let core_tasks = core_projection
        .as_ref()
        .map(|projection| {
            projection
                .order
                .iter()
                .filter_map(|id| projection.tasks.get(id))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if tasks.is_empty() && core_tasks.is_empty() {
        return task_ok(
            ui_text(UiText::NoBackgroundTasks).to_string(),
            format!("0 {}", ui_text(UiText::TasksUnit)),
        );
    }
    let mut lines = vec![ui_text(UiText::BackgroundTasksTitle).to_string()];
    for task in &core_tasks {
        let status = core_task_state(task);
        let marker = match task.state {
            nola_core::agent::TaskState::Completed => "✓",
            nola_core::agent::TaskState::Sealed => "□",
            nola_core::agent::TaskState::Failed | nola_core::agent::TaskState::Cancelled => "!",
            _ => "●",
        };
        let step = task.current_step.as_deref().unwrap_or_default();
        let progress = task
            .progress
            .map(|value| format!(" · {:.0}%", value * 100.0))
            .unwrap_or_default();
        let runner = tasks
            .iter()
            .find(|runtime_task| runtime_task.id == task.id)
            .and_then(|runtime_task| runtime_task.agent_type.as_deref())
            .map(str::to_string)
            .unwrap_or_else(|| format!("{:?}", task.kind));
        lines.push(format!(
            "- {marker} `{}` · {} · {}{}{}",
            task.id,
            localized_status(&status, false),
            runner,
            progress,
            if step.is_empty() {
                String::new()
            } else {
                format!(" · {}", truncate_chars(step, 60))
            }
        ));
    }
    let mut rendered_runtime_count = 0usize;
    for task in &tasks {
        if core_tasks.iter().any(|core_task| core_task.id == task.id) {
            continue;
        }
        rendered_runtime_count = rendered_runtime_count.saturating_add(1);
        let marker = if is_live_status(&task.status) {
            "●"
        } else if task.status == "sealed" {
            "□"
        } else if task.status == "completed" {
            "✓"
        } else {
            "!"
        };
        let runner = task.agent_type.as_deref().unwrap_or(&task.kind);
        let progress = task.progress_summary.as_deref().unwrap_or_default();
        lines.push(format!(
            "- {marker} `{}` · {} · {} · {}{}",
            task.id,
            localized_status(&task.status, false),
            runner,
            truncate_chars(&task.title, 72),
            if progress.is_empty() {
                String::new()
            } else {
                format!(" · {}", truncate_chars(progress, 60))
            }
        ));
    }
    lines.push(format!("\n{}", ui_text(UiText::TasksFooter)));
    task_ok(
        lines.join("\n"),
        format!(
            "{} {}",
            rendered_runtime_count + core_tasks.len(),
            ui_text(UiText::TasksUnit)
        ),
    )
}

fn core_task(config: &HostInvocationRequest, task_id: &str) -> Option<nola_core::agent::task::TaskHandle> {
    let session_id = config.session.session_id.as_deref()?;
    crate::session::load_session_task_projection(session_id)
        .ok()
        .flatten()?
        .tasks
        .get(task_id)
        .cloned()
}

fn agent_terminal_state_is_reconciled(config: &HostInvocationRequest, task: &DbRuntimeTask) -> bool {
    if task.kind != "agent" || is_live_status(&task.status) {
        return true;
    }
    if config.session.session_id.as_deref() != task.owner_session_id.as_deref() {
        return false;
    }
    let core = core_task(config, &task.id);
    terminal_receipt_state_matches(task, core.as_ref())
}

fn task_terminal_fact_confirmed(task: &DbRuntimeTask) -> bool {
    if is_live_status(&task.status) {
        return false;
    }
    if task.kind != "agent" {
        return true;
    }
    let receipt: Result<Option<nola_application::RuntimeTaskTerminalReceipt>, String> =
        crate::application_port::CliApplicationPort::persistent_default().and_then(|mut app| {
            app.execute(nola_application::ApplicationCommand::RuntimeTask {
                command: nola_application::RuntimeTaskCommand::TerminalReceipt {
                    task_id: task.id.clone(),
                },
            })
        });
    receipt.ok().flatten().is_some_and(|receipt| {
        receipt.task_id == task.id
            && Some(receipt.owner_session_id.as_str()) == task.owner_session_id.as_deref()
            && receipt.child_session_id == task.child_session_id
            && receipt.input_watermark == task.input_watermark
            && receipt.status == task.status
            && receipt.completion_watermark == task.completion_watermark
            && task.ended_at_ms == Some(receipt.ended_at_ms)
    })
}

fn terminal_receipt_state_matches(
    task: &DbRuntimeTask,
    core: Option<&nola_core::agent::task::TaskHandle>,
) -> bool {
    let receipt: Result<Option<nola_application::RuntimeTaskTerminalReceipt>, String> =
        crate::application_port::CliApplicationPort::persistent_default().and_then(|mut app| {
            app.execute(nola_application::ApplicationCommand::RuntimeTask {
                command: nola_application::RuntimeTaskCommand::TerminalReceipt {
                    task_id: task.id.clone(),
                },
            })
        });
    let Ok(Some(receipt)) = receipt else {
        return false;
    };
    receipt.task_id == task.id
        && Some(receipt.owner_session_id.as_str()) == task.owner_session_id.as_deref()
        && receipt.child_session_id == task.child_session_id
        && receipt.input_watermark == task.input_watermark
        && receipt.status == task.status
        && receipt.completion_watermark == task.completion_watermark
        && task.ended_at_ms == Some(receipt.ended_at_ms)
        && core.is_some_and(|core| {
            core.id == task.id && core.durability.input_watermark == receipt.input_watermark
                && core.durability.run_result.as_ref() == receipt.journal_result.as_ref().map(|result| &result.seal.result)
                && core.durability.result_digest == receipt.journal_result.as_ref().map(|result| result.seal.result_digest)
        })
        && terminal_states_match(
            &task.status,
            task.completion_watermark,
            core,
            Some(&receipt.receipt_id),
            receipt.result_ref.as_deref(),
        )
}

fn terminal_states_match(
    runtime_status: &str,
    runtime_completion_watermark: Option<i64>,
    core: Option<&nola_core::agent::task::TaskHandle>,
    receipt_id: Option<&str>,
    result_ref: Option<&str>,
) -> bool {
    core.is_some_and(|core| {
        matches!(
            (runtime_status, core.state),
            ("sealed", nola_core::agent::TaskState::Sealed)
                | ("failed", nola_core::agent::TaskState::Failed)
                | ("cancelled", nola_core::agent::TaskState::Cancelled)
        ) && core.durability.completion_watermark == runtime_completion_watermark
            && receipt_id.is_some_and(|id| {
                !id.is_empty() && core.durability.terminal_receipt_id.as_deref() == Some(id)
            })
            && core.result_ref.as_deref() == result_ref
    })
}

/// Requeues the Store's terminal receipt after a worker exits. A current
/// parent projection is only a completion check, never execution authority.
fn replay_terminal_agent_reconciliation(task: &DbRuntimeTask) -> bool {
    if task.kind != "agent" || is_live_status(&task.status) {
        return false;
    }
    let Some(_owner_session_id) = task.owner_session_id.as_deref() else {
        return false;
    };
    // The parent may be an in-memory Application. Its durable absence cannot
    // suppress delivery; the exact original owner validates the exported fact.
    crate::runtime_task_port::enqueue_terminal_reconciliation(&task.id).is_ok()
}

fn core_task_state(task: &nola_core::agent::task::TaskHandle) -> String {
    serde_json::to_value(task.state)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{:?}", task.state).to_ascii_lowercase())
}

fn render_core_task_detail(task: &nola_core::agent::task::TaskHandle) -> ToolOutcome {
    let status = core_task_state(task);
    let mut lines = vec![format!(
        "# {}",
        task.current_step.as_deref().unwrap_or(&task.id)
    )];
    lines.push(format!("- {}: `{}`", ui_text(UiText::Task), task.id));
    lines.push(format!(
        "- {}: {}",
        ui_text(UiText::Status),
        localized_status(&status, false)
    ));
    lines.push(format!("- {}: `{:?}`", ui_text(UiText::Agent), task.kind));
    if let Some(progress) = task.progress {
        lines.push(format!(
            "- {}: {:.0}%",
            ui_text(UiText::Progress),
            progress * 100.0
        ));
    }
    if let Some(result) = task.result_ref.as_deref() {
        lines.push(format!("\n## {}\n\n{}", ui_text(UiText::Result), result));
    }
    if task.cancel_requested && !task.state.is_terminal() {
        lines.push("\n- Cancellation requested".to_string());
    }
    task_ok(lines.join("\n"), format!("Task {} is {status}", task.id))
}

fn runtime_task_kind(config: &HostInvocationRequest, task_id: &str) -> Option<String> {
    NolaStore::open_default()
        .and_then(|store| store.runtime_task(task_id))
        .ok()
        .flatten()
        .filter(|task| task_scope_matches(config, task))
        .map(|task| task.kind)
}

fn task_scope_matches(config: &HostInvocationRequest, task: &DbRuntimeTask) -> bool {
    task_scope_matches_values(
        task,
        &config.tool_settings.project_root.to_string_lossy(),
        config.session.session_id.as_deref(),
    )
}

fn task_scope_matches_values(
    task: &DbRuntimeTask,
    project_root: &str,
    owner_session_id: Option<&str>,
) -> bool {
    let task_root = std::path::Path::new(&task.project_root);
    let requested_root = std::path::Path::new(project_root);
    let same_project = match (task_root.canonicalize(), requested_root.canonicalize()) {
        (Ok(task_root), Ok(requested_root)) => task_root == requested_root,
        _ => task_root == requested_root,
    };
    same_project && task.owner_session_id.as_deref() == owner_session_id
}

fn render_task_detail(
    config: &HostInvocationRequest,
    task_id: &str,
    skills: &crate::skill_runtime::SkillRuntimeContext,
) -> ToolOutcome {
    let store = match NolaStore::open_default() {
        Ok(store) => store,
        Err(error) => return task_error(error),
    };
    let Some(task) = (match store.runtime_task(task_id) {
        Ok(task) => task,
        Err(error) => return task_error(error),
    }) else {
        return tools::manage_background_bash_tasks(
            "status",
            Some(task_id),
            &config.tool_settings.allowed_dirs,
            config.session.session_id.as_deref(),
            skills,
        );
    };
    if !task_scope_matches(config, &task) {
        return task_error(format!("Unknown task: {task_id}"));
    }
    let zh = current_language() == Language::ZhHans;
    let events = store.runtime_task_events(task_id, 8).unwrap_or_default();
    let mut lines = vec![format!("# {}", task.title)];
    lines.push(format!("- {}: `{}`", ui_text(UiText::Task), task.id));
    lines.push(format!(
        "- {}: {}",
        ui_text(UiText::Status),
        localized_status(&task.status, zh)
    ));
    if let Some(agent_type) = task.agent_type.as_deref() {
        lines.push(format!("- {}: `{agent_type}`", ui_text(UiText::Agent)));
    }
    if let Some(progress) = task.progress_summary.as_deref() {
        lines.push(format!("- {}: {}", ui_text(UiText::Progress), progress));
    }
    if let Some(result) = task.result_summary.as_deref() {
        lines.push(format!("\n## {}\n\n{}", ui_text(UiText::Result), result));
    }
    if let Some(error) = task.error.as_deref() {
        lines.push(format!("\n## {}\n\n{}", ui_text(UiText::Error), error));
    }
    if !events.is_empty() {
        lines.push(format!("\n## {}", ui_text(UiText::RecentActivity)));
        for event in events.into_iter().rev() {
            lines.push(format!(
                "- {} · {}",
                localized_status(&event.status, zh),
                event.summary.unwrap_or(event.kind)
            ));
        }
    }
    task_ok(
        lines.join("\n"),
        format!("Task {} is {}", task.id, task.status),
    )
}

fn localized_status(status: &str, _zh: bool) -> &str {
    match status {
        "queued" => ui_text(UiText::Queued),
        "running" => ui_text(UiText::Running),
        "waiting" => ui_text(UiText::WaitingPermission),
        "waiting_permission" => ui_text(UiText::WaitingPermission),
        "waiting_input" => if _zh { "等待用户回答" } else { "Waiting for user input" },
        "sealed" => if _zh { "执行已封存" } else { "Execution sealed" },
        "completed" => ui_text(UiText::Completed),
        "failed" => ui_text(UiText::Failed),
        "cancelled" => ui_text(UiText::Cancelled),
        _ => status,
    }
}

fn read_task(
    config: &HostInvocationRequest,
    args: &Map<String, Value>,
    skills: &crate::skill_runtime::SkillRuntimeContext,
) -> ToolOutcome {
    let Some(task_id) = string_arg(args, "task_id").map(str::trim) else {
        return task_error("read_task requires task_id");
    };
    if runtime_task_kind(config, task_id).as_deref() == Some("shell") {
        return tools::manage_background_bash_tasks(
            "status",
            Some(task_id),
            &config.tool_settings.allowed_dirs,
            config.session.session_id.as_deref(),
            skills,
        );
    }
    let store = match NolaStore::open_default() {
        Ok(store) => store,
        Err(error) => return task_error(error),
    };
    let Some(task) = (match store.runtime_task(task_id) {
        Ok(task) => task,
        Err(error) => return task_error(error),
    }) else {
        return tools::manage_background_bash_tasks(
            "status",
            Some(task_id),
            &config.tool_settings.allowed_dirs,
            config.session.session_id.as_deref(),
            skills,
        );
    };
    if !task_scope_matches(config, &task) {
        return task_error(format!("Unknown task: {task_id}"));
    }
    let events = store.runtime_task_events(task_id, 8).unwrap_or_default();
    task_ok(
        json!({
            "task": task_json(&task),
            "events": events.into_iter().rev().map(|event| json!({
                "kind": event.kind,
                "status": event.status,
                "summary": event.summary,
                "created_at_ms": event.created_at_ms,
            })).collect::<Vec<_>>()
        })
        .to_string(),
        format!("Task {} is {}", task.id, task.status),
    )
}

fn wait_task(config: &HostInvocationRequest, args: &Map<String, Value>, context: &ToolContext) -> ToolOutcome {
    let mut outcome = wait_task_until_return(config, args, context);
    // This receipt describes the bounded wait invocation, never the task it
    // observes. Returning (including cancellation/timeout) ends this local
    // execution even when the separate child task is still running.
    outcome.execution = Some(nola_host_protocol::tool_result::ExecutionReceipt {
        state: nola_host_protocol::tool_result::ExecutionState::Completed,
        ..Default::default()
    });
    outcome
}

fn wait_task_until_return(config: &HostInvocationRequest, args: &Map<String, Value>, context: &ToolContext) -> ToolOutcome {
    // If the parent immediately waits after spawn_agent, this is the first
    // safe boundary after TaskPromoted at which the queued worker may start.
    reconcile_supervised_tasks(config);
    let Some(task_id) = string_arg(args, "task_id").map(str::trim) else {
        return task_error("wait_task requires task_id");
    };
    let timeout_seconds = args
        .get("timeout_sec")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_WAIT_SECONDS)
        .clamp(1, MAX_WAIT_SECONDS);
    let deadline = Instant::now() + Duration::from_secs(timeout_seconds);
    let stored_task = match NolaStore::open_default().and_then(|store| store.runtime_task(task_id))
    {
        Ok(task) => task,
        Err(error) => return task_error(error),
    };
    if stored_task
        .as_ref()
        .is_some_and(|task| !task_scope_matches(config, task))
    {
        return task_error(format!("Unknown task: {task_id}"));
    }
    if stored_task.as_ref().is_none_or(|task| task.kind == "shell") {
        loop {
            let summaries = match tools::background_bash_task_summaries(
                &config.tool_settings.allowed_dirs,
                config.session.session_id.as_deref(),
            ) {
                Ok(summaries) => summaries,
                Err(error) => return task_error(error),
            };
            let Some(task) = summaries.into_iter().find(|task| task.id == task_id) else {
                return task_error(format!("Unknown task: {task_id}"));
            };
            if !task.status.to_ascii_lowercase().starts_with("running") {
                return tools::manage_background_bash_tasks(
                    "status",
                    Some(task_id),
                    &config.tool_settings.allowed_dirs,
                    config.session.session_id.as_deref(),
                    &context.skills,
                );
            }
            if context.cancellation.is_requested() {
                return task_error(format!(
                    "Task wait cancelled; task {task_id} is still running unless stopped separately."
                ));
            }
            if Instant::now() >= deadline {
                return task_ok(
                    json!({
                        "task": {
                            "task_id": task.id,
                            "kind": "shell",
                            "status": task.status,
                            "title": task.command,
                        },
                        "wait_timed_out": true
                    })
                    .to_string(),
                    format!("Task {task_id} is still running"),
                );
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    loop {
        // Observe the wake generation before reading the durable row. A task
        // may finish between the SQLite read and the condvar wait; retaining
        // this generation turns that race into an immediate retry instead of
        // losing the notification and falling back to periodic polling.
        let observed_generation = task_wake_generation();
        let store = match NolaStore::open_default() {
            Ok(store) => store,
            Err(error) => return task_error(error),
        };
        let Some(task) = (match store.runtime_task(task_id) {
            Ok(task) => task,
            Err(error) => return task_error(error),
        }) else {
            return task_error(format!("Unknown task: {task_id}"));
        };
        if !task_scope_matches(config, &task) {
            return task_error(format!("Unknown task: {task_id}"));
        }
        // Permission is presented independently by the Host/CLI notification
        // channel. Do not turn an unresolved approval into an immediate model
        // polling loop; keep observing this bounded wait until a wake/deadline.
        if task.status == "waiting_input" {
            let questions = match load_waiting_task_questions(
                task.child_session_id.as_deref(), crate::user_input_wait::open,
            ) {
                Ok(questions) => questions,
                Err(error) => return task_error(error),
            };
            return task_ok(json!({"task":task_json(&task),"wait_timed_out":false,
                "action_required":"user_input","questions":questions}).to_string(),
                format!("Task {task_id} is waiting for user input"));
        }
        let terminal_reconciled = agent_terminal_state_is_reconciled(config, &task);
        if !is_live_status(&task.status) {
            if terminal_reconciled {
                return completed_wait_task_result(
                    &task,
                    json!({
                        "task": task_json(&task),
                        "wait_timed_out": false,
                        "completion_confirmed": true,
                        "parent_delivery": "applied",
                    })
                    .to_string(),
                    format!("Task {} {}", task.id, task.status),
                );
            }
            if replay_terminal_agent_reconciliation(&task) {
                // `wait_task` runs while the active Core Host is locked. The
                // replay above therefore durably queues the terminal command,
                // but cannot apply it until this tool returns to the Host
                // boundary. Return the authoritative worker result now; the
                // queued command is drained before the next model step.
                return completed_wait_task_result(
                    &task,
                    json!({
                        "task": task_json(&task),
                        "wait_timed_out": false,
                        "completion_confirmed": task_terminal_fact_confirmed(&task),
                        "parent_delivery": "pending",
                        "reconciliation_pending": true,
                    })
                    .to_string(),
                    format!("Task {} {}", task.id, task.status),
                );
            }
        }
        if context.cancellation.is_requested() {
            return task_error(format!(
                "Task wait cancelled; task {task_id} was not changed."
            ));
        }
        let worker_exit: Result<Option<String>, String> = store.application().and_then(|mut app|
            app.execute(nola_application::ApplicationCommand::RuntimeTask {
                command: nola_application::RuntimeTaskCommand::CurrentWorkerExit { task_id: task_id.into() },
            }));
        match worker_exit {
            Ok(Some(reason)) => return task_ok(json!({
                "task": task_json(&task), "wait_timed_out": false,
                "completion_confirmed": false, "action_required": "recovery",
                "worker_exit": reason,
            }).to_string(), format!("Task {task_id} worker exited; recovery is required")),
            Err(error) => return task_error(error),
            Ok(None) => {},
        }
        let now = Instant::now();
        if now >= deadline {
            if task.status == "waiting_permission" {
                return task_ok(
                    json!({"task": task_json(&task), "wait_timed_out": true,
                        "action_required": "permission"}).to_string(),
                    format!("Task {task_id} needs permission"),
                );
            }
            if !is_live_status(&task.status) {
                return task_ok(
                    json!({
                        "task": task_json(&task),
                        "wait_timed_out": true,
                        "completion_confirmed": task_terminal_fact_confirmed(&task),
                        "parent_delivery": "pending",
                        "reconciliation_pending": true,
                    })
                    .to_string(),
                    format!(
                        "Task {task_id} worker result is still awaiting authoritative reconciliation"
                    ),
                );
            }
            return task_ok(
                json!({"task": task_json(&task), "wait_timed_out": true, "completion_confirmed": false}).to_string(),
                format!("Task {task_id} is still running"),
            );
        }
        let remaining = deadline.saturating_duration_since(now);
        wait_for_task_wake(observed_generation, remaining.min(Duration::from_millis(100)));
    }
}

fn load_waiting_task_questions(
    child_session_id: Option<&str>,
    load: impl FnOnce(&str) -> Result<Vec<nola_store_port::user_input_wait::StoredUserInputWait>, String>,
) -> Result<Vec<nola_store_port::user_input_wait::StoredUserInputWait>, String> {
    let child = child_session_id.filter(|id| !id.trim().is_empty())
        .ok_or_else(|| "waiting Task has no child conversation identity".to_string())?;
    load(child)
}

fn completed_wait_task_result(task: &DbRuntimeTask, text: String, display: String) -> ToolOutcome {
    match crate::runtime_task_port::result_delivery_cursor(task) {
        Ok(cursor) => {
            let data = serde_json::to_value(
                crate::runtime_task_port::RuntimeTaskResultData::RuntimeTaskResultDelivery {
                    cursor,
                    text: text.clone(),
                },
            )
            .expect("bounded Task result cursor serializes");
            task_ok(text, display).with_data(data)
        }
        Err(error) => task_error(error),
    }
}

fn stop_task(
    config: &HostInvocationRequest,
    args: &Map<String, Value>,
    skills: &crate::skill_runtime::SkillRuntimeContext,
) -> ToolOutcome {
    let Some(task_id) = string_arg(args, "task_id").map(str::trim) else {
        return task_error("stop_task requires task_id");
    };
    if task_id.starts_with("bash-")
        || runtime_task_kind(config, task_id).as_deref() == Some("shell")
    {
        return tools::manage_background_bash_tasks(
            "stop",
            Some(task_id),
            &config.tool_settings.allowed_dirs,
            config.session.session_id.as_deref(),
            skills,
        );
    }
    let store = match NolaStore::open_default() {
        Ok(store) => store,
        Err(error) => return task_error(error),
    };
    let Some(task) = (match store.runtime_task(task_id) {
        Ok(task) => task,
        Err(error) => return task_error(error),
    }) else {
        return tools::manage_background_bash_tasks(
            "stop",
            Some(task_id),
            &config.tool_settings.allowed_dirs,
            config.session.session_id.as_deref(),
            skills,
        );
    };
    if !task_scope_matches(config, &task) {
        return task_error(format!("Unknown task: {task_id}"));
    }
    if task.kind == "shell" {
        return tools::manage_background_bash_tasks(
            "stop",
            Some(task_id),
            &config.tool_settings.allowed_dirs,
            config.session.session_id.as_deref(),
            skills,
        );
    }
    let cancellation_persisted = match crate::runtime_task_port::request_cancel(task_id, unix_epoch_millis()) {
        Ok(value) => value,
        Err(error) => return task_error(error),
    };
    let cancellation_requested = cancellation_persisted || task.cancel_requested;
    let bound = match crate::runtime_task_port::run_binding(task_id) {
        Ok(value) => value.is_some(),
        Err(error) => return task_error(error),
    };
    if bound && cancellation_requested {
        // The Task flag is only cancellation intent. Core owns withdrawing the
        // question and sealing the Run; no Task terminal is fabricated here.
        if let Err(error) = cancel_bound_task_execution(
            &task, &format!("task-user-input-cancel:{}", task.id), "cancelled by user") {
            let _ = append_task_event(task_id,"task_cancellation_pending",&task.status,
                Some("Core cancellation awaits independent effect quiescence"),json!({"error":error}));
        }
    }
    let handle = task_handles()
        .lock()
        .ok()
        .and_then(|handles| handles.get(task_id).cloned());
    let Some(handle) = handle else {
        if is_live_status(&task.status) && cancellation_requested {
            wake_task_waiters();
            return task_ok(
                json!({"task_id": task_id, "status": "cancelling"}).to_string(),
                format!("Stopping task {task_id}"),
            );
        }
        return task_ok(
            json!({"task": task_json(&task), "already_finished": true}).to_string(),
            format!("Task {task_id} already {}", task.status),
        );
    };
    handle.cancellation.request();
    wake_task_waiters();
    task_ok(
        json!({"task_id": task_id, "status": "cancelling"}).to_string(),
        format!("Stopping task {task_id}"),
    )
}

pub(crate) fn wait_for_background_agent_permission(
    child_session_id: &str,
    capability: &str,
    input: &Value,
    instructions: &[nola_core::agent::host::PermissionInstruction],
    core_permission_request_id: &str,
) -> Result<Option<BackgroundPermissionWait>, String> {
    let Some((task_id, cancellation, execution, cold_resume)) =
        task_handles().lock().ok().and_then(|handles| {
            handles.iter().find_map(|(task_id, handle)| {
                (handle.child_session_id == child_session_id).then(|| {
                    (
                        task_id.clone(),
                        handle.cancellation.clone(),
                        handle.execution.clone(),
                        handle.permission_resume.clone(),
                    )
                })
            })
        })
    else {
        return Ok(None);
    };
    if let Some(resume) = cold_resume {
        let continuation: Value = serde_json::from_str(&resume.suspension.continuation_json)
            .map_err(|error| error.to_string())?;
        if continuation
            .get("core_permission_request_id")
            .and_then(Value::as_str)
            == Some(core_permission_request_id)
        {
            if cancellation.is_requested() {
                return stop_background_permission_wait(&task_id, execution.as_ref());
            }
            let permission = crate::permission_request_port::permission_request(
                &resume.suspension.permission_request_id,
            )?
            .ok_or("recovered permission occurrence disappeared")?;
            if permission.task_id != task_id
                || permission.child_session_id != child_session_id
                || permission.core_permission_request_id.as_deref()
                    != Some(core_permission_request_id)
                || permission.capability != capability
                || permission.input != *input
                || serde_json::to_value(&permission.permission_instructions)
                    .map_err(|e| e.to_string())?
                    != serde_json::to_value(instructions).map_err(|e| e.to_string())?
                || continuation.get("child_session_id").and_then(Value::as_str)
                    != Some(child_session_id)
                || continuation.get("capability").and_then(Value::as_str) != Some(capability)
                || continuation.get("input") != Some(input)
            {
                return Err("recovered permission does not match the original Core effect".into());
            }
            let run = crate::application_port::CliApplicationPort::persistent_default()?
                .active_run(child_session_id)?
                .ok_or("recovered permission Run disappeared")?;
            if continuation.get("run_id").and_then(Value::as_str) != Some(run.run_id.as_str()) {
                return Err("recovered permission belongs to a different Run".into());
            }
            validate_cold_permission_grant(&resume)?;
            let authority = execution
                .as_ref()
                .ok_or("recovered permission has no execution authority")?;
            let active = authority
                .active_lease()?
                .ok_or("recovered permission parent is not confirmed")?;
            let current = active.snapshot()?;
            if current.id != resume.grant.id || current.lease_token != resume.grant.lease_token {
                return Err("recovered permission worker no longer owns its grant".into());
            }
            // Startup's heartbeat may already cover a longer interval. Never
            // turn that valid grant into an invalid shortening renewal; use
            // the shared serialized refresh (which still rejects expiry).
            crate::runtime_task_port::refresh_lease_before_execution(&active, 45_000)?;
            return background_permission_decision(permission)
                .map(|decision| Some(BackgroundPermissionWait::Decision(decision)))
                .ok_or_else(|| "recovered permission has no resolved decision".to_string());
        }
    }
    let request = tools::ToolRequest {
        id: format!("background-permission-{task_id}"),
        name: capability.to_string(),
        args: input.as_object().cloned().unwrap_or_default(),
    };
    let scope_rule = tools::session_permission_rule(&request);
    // Core has already decided that this concrete effect needs a decision.
    // A Host-generated directory/command similarity rule must not override
    // that decision. Reusable authorization belongs to Core's checkpoint;
    // this adapter only transports the selected occurrence and its receipt.
    let now = unix_epoch_millis();
    let request_id = format!(
        "background-permission-{task_id}-{now}-{}",
        TASK_OPERATION_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let execution = execution.ok_or("permission wait has no retained execution authority")?;
    let permission_request = nola_store_port::permission::StoredPermissionRequest {
        closure: None,
        request_id: request_id.clone(),
        task_id: task_id.clone(),
        child_session_id: child_session_id.to_string(),
        capability: capability.to_string(),
        input_json: input.to_string(),
        scope_rule: scope_rule.clone(),
        created_at_ms: now,
        core_permission_request_id: Some(core_permission_request_id.to_string()),
        permission_instructions_json: serde_json::to_string(instructions)
            .map_err(|error| error.to_string())?,
        status: "pending".into(),
        decision: None,
        custom_response: None,
        resolved_at_ms: None,
    };
    let store = NolaStore::open_default().map_err(|error| error.to_string())?;
    let runtime_task = store
        .runtime_task(&task_id)?
        .ok_or("permission task disappeared")?;
    let owner_session_id = runtime_task
        .owner_session_id
        .as_deref()
        .ok_or("permission task has no owner session")?;
    let original = execution
        .active_lease()?
        .ok_or("permission task is not active")?
        .snapshot()?;
    let authority = crate::runtime_task_port::agent_authority(&task_id)?
        .ok_or("permission task has no admitted authority")?;
    authority.validate()?;
    if authority.phase
        != nola_domain::agent_task_admission::RuntimeAgentTaskAuthorityPhase::Admitted
        || authority.admission.as_ref().is_none_or(|admission| {
            admission.parent_conversation_id != owner_session_id
                || admission.child_conversation_id != child_session_id
        })
        || runtime_task.cancel_requested
        || runtime_task.ownership_token.as_deref() != Some(original.lease_token.as_str())
    {
        return Err("permission task authority does not match the retained attempt".into());
    }
    let input_watermark = runtime_task.input_watermark;
    let localized_capability = crate::i18n::localize_tool_display_name(capability);
    let permission_summary = format!(
        "{}: {}",
        ui_text(UiText::WaitingPermission),
        localized_capability
    );
    let permission_summary = if instructions.is_empty() {
        permission_summary
    } else {
        format!(
            "{}\n{}",
            permission_summary,
            instructions
                .iter()
                .map(|entry| entry.instruction.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        )
    };
    let continuation_id = uuid::Uuid::new_v4().to_string();
    let active_run = crate::application_port::CliApplicationPort::persistent_default()?
        .active_run(child_session_id)?
        .ok_or("permission continuation has no active child Run")?;
    let continuation_json =
        json!({"child_session_id": child_session_id, "run_id": active_run.run_id,
        "core_permission_request_id": core_permission_request_id,
        "capability": capability, "input": input, "input_watermark": input_watermark})
        .to_string();
    let suspension = execution.suspend_permission_with(
        &request_id,
        &continuation_id,
        &continuation_json,
        |attempt| {
            let command = nola_application::RuntimeTaskCommand::SuspendForPermission {
                input: nola_application::SuspendRuntimeTaskForPermission {
                    task_id: task_id.clone(),
                    attempt_id: attempt.id.clone(),
                    expected_attempt_revision: attempt.revision,
                    worker_generation: attempt.worker_generation.clone(),
                    lease_token: attempt.lease_token.clone(),
                    current_expiry_ms: attempt.lease_expires_at_ms,
                    permission_request,
                    continuation_id: continuation_id.clone(),
                    continuation_json: continuation_json.clone(),
                    progress_summary: permission_summary.clone(),
                    last_activity: Some(capability.to_string()),
                    now_ms: unix_epoch_millis(),
                },
            };
            let result = crate::application_port::CliApplicationPort::persistent_default()?
                .execute(nola_application::ApplicationCommand::RuntimeTask { command });
            match result {
                Ok(value) => Ok(value),
                Err(error) => {
                    let fact: Option<nola_application::RuntimeTaskPermissionSuspension> =
                        crate::application_port::CliApplicationPort::persistent_default()?
                            .execute(nola_application::ApplicationCommand::RuntimeTask {
                            command:
                                nola_application::RuntimeTaskCommand::PendingPermissionSuspension {
                                    task_id: task_id.clone(),
                                },
                        })?;
                    fact.map(Some).ok_or(error)
                }
            }
        },
    )?;
    let _ = append_task_event(
        &task_id,
        "permission",
        "waiting_permission",
        Some(&permission_summary),
        json!({"request_id": request_id, "capability": capability, "input": input}),
    );
    tools::wake_background_task_notification();

    loop {
        if cancellation.is_requested() {
            return stop_background_permission_wait(&task_id, Some(&execution));
        }
        let permission = crate::permission_request_port::permission_request(&request_id)?;
        if let Some(closure) = permission.as_ref().and_then(|request| request.closure.as_ref()) {
            execution.revoke();
            if matches!(closure.source, nola_domain::permission_closure::PermissionClosureSource::CoreSettled { .. }) {
                return Ok(Some(BackgroundPermissionWait::Settled));
            }
            return Err("permission request was closed by its execution lifecycle".into());
        }
        let decision = permission.and_then(background_permission_decision);
        if let Some(decision) = decision {
            let resolution_summary = match &decision {
                BackgroundPermissionDecision::Allow => ui_text(UiText::AllowedTaskOnce),
                BackgroundPermissionDecision::AllowTask => ui_text(UiText::AllowResourceForTask),
                BackgroundPermissionDecision::Custom(_) => ui_text(UiText::SentCustomTaskResponse),
                BackgroundPermissionDecision::ReturnToComposer => {
                    crate::i18n::interaction_text().custom_permission_return
                }
                BackgroundPermissionDecision::Deny => ui_text(UiText::DeniedTaskRequest),
            }
            .to_string();
            let (max_tasks, max_project_tasks) = runtime_task_concurrency_limits();
            let resume_lease_duration_ms = std::env::var("NOLA_AGENT_TASK_LEASE_MS")
                .ok()
                .and_then(|value| value.parse::<i64>().ok())
                .unwrap_or(45_000)
                .clamp(1_000, 10 * 60 * 1_000);
            let admitted = loop {
                if cancellation.is_requested() {
                    return stop_background_permission_wait(&task_id, Some(&execution));
                }
                let resume_at = unix_epoch_millis();
                let resumed = execution.prepare_permission_resume_with(|previous, pending| {
                    let proposal = nola_application::ResumeRuntimeTaskAfterPermissionFair {
                        task_id: task_id.clone(),
                        suspension_id: pending.id.clone(),
                        expected_suspension_revision: pending.revision,
                        permission_request_id: pending.permission_request_id.clone(),
                        continuation_id: pending.continuation_id.clone(),
                        worker_id: previous.worker_id.clone(),
                        worker_generation: uuid::Uuid::new_v4().to_string(),
                        lease_token: uuid::Uuid::new_v4().to_string(),
                        now_ms: resume_at,
                        lease_until_ms: resume_at.saturating_add(resume_lease_duration_ms),
                        max_running_global: max_tasks,
                        max_running_project: max_project_tasks,
                    };
                    let result: Result<Option<nola_application::RuntimeTaskAttempt>, String> =
                        crate::application_port::CliApplicationPort::persistent_default()?
                            .execute(nola_application::ApplicationCommand::RuntimeTask {
                            command:
                                nola_application::RuntimeTaskCommand::ResumeAfterPermissionFair {
                                    input: proposal.clone(),
                                },
                        });
                    let admitted = match result {
                        Ok(value) => value,
                        Err(error) => {
                            let fact: Option<nola_application::RuntimeTaskPermissionResume> =
                                crate::application_port::CliApplicationPort::persistent_default()?
                                    .execute(nola_application::ApplicationCommand::RuntimeTask {
                                        command:
                                            nola_application::RuntimeTaskCommand::PermissionResume {
                                                task_id: task_id.clone(),
                                                suspension_id: pending.id.clone(),
                                                permission_request_id: pending
                                                    .permission_request_id
                                                    .clone(),
                                                continuation_id: pending.continuation_id.clone(),
                                            },
                                    })?;
                            let fact = fact.ok_or(error)?;
                            if fact.suspension.attempt_id != previous.id
                                || fact.suspension.continuation_json != pending.continuation_json
                                || fact.suspension.state != "resumed"
                            {
                                return Err(
                                    "permission resume receipt changed original continuation"
                                        .into(),
                                );
                            }
                            Some(fact.granted_attempt)
                        }
                    };
                    if let Some(grant) = &admitted {
                        if grant.worker_id != proposal.worker_id
                            || grant.worker_generation != proposal.worker_generation
                            || grant.lease_token != proposal.lease_token
                            || grant.claimed_at_ms != proposal.now_ms
                            || grant.lease_expires_at_ms != proposal.lease_until_ms
                        {
                            return Err(
                                "permission resume receipt does not match this proposal".into()
                            );
                        }
                    }
                    Ok(admitted)
                })?;
                if let Some(admitted) = resumed {
                    break admitted;
                }
                if let Some(closure) = crate::permission_request_port::permission_request(&request_id)?
                    .and_then(|request| request.closure) {
                    execution.revoke();
                    if matches!(closure.source, nola_domain::permission_closure::PermissionClosureSource::CoreSettled { .. }) {
                        return Ok(Some(BackgroundPermissionWait::Settled));
                    }
                    return Err("permission request closed while waiting for execution admission".into());
                }
                let (generation, condvar) = task_wake();
                let guard = generation
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let _ = condvar.wait_timeout(guard, Duration::from_millis(100));
            };
            // The exact Store resume atomically replaced the original attempt
            // under the same admitted Task authority. Parent Core is display,
            // not a second gate or permission issuer.
            if cancellation.is_requested() {
                return stop_background_permission_wait(&task_id, Some(&execution));
            }
            let resumed = execution.activate_permission_resume(&suspension.id, &admitted)?;
            validate_durable_task_execution(
                owner_session_id,
                &task_id,
                input_watermark.unwrap_or(runtime_task.created_at_ms),
                &resumed,
            )?;
            crate::runtime_task_port::refresh_lease_before_execution(
                &resumed,
                resume_lease_duration_ms,
            )?;
            let _ = append_task_event(
                &task_id,
                "permission_execution_activated",
                "running",
                Some(&resolution_summary),
                json!({"suspension_id": suspension.id, "attempt_id": admitted.id}),
            );
            return Ok(Some(BackgroundPermissionWait::Decision(decision)));
        }
        let (generation, condvar) = task_wake();
        let guard = generation
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _ = condvar.wait_timeout(guard, Duration::from_millis(250));
    }
}

fn stop_background_permission_wait(
    task_id: &str,
    execution: Option<&crate::runtime_task_port::RuntimeTaskExecutionAuthority>,
) -> Result<Option<BackgroundPermissionWait>, String> {
    // Revoke local execution even when persisting stop intent fails. The
    // canonical cancellation flow, not this callback, confirms quiescence.
    if let Some(execution) = execution { execution.revoke(); }
    crate::runtime_task_port::request_cancel(task_id, unix_epoch_millis())?;
    Ok(Some(BackgroundPermissionWait::StopRequested))
}

#[derive(Debug, Clone)]
pub struct TaskPermissionPrompt {
    pub prompt: PermissionPrompt,
    permission: nola_application::PermissionRequest,
}

impl TaskPermissionPrompt {
    pub fn request_id(&self) -> &str { &self.permission.request_id }
}

pub fn permission_prompt_for_task(config: &HostInvocationRequest, task_id: &str) -> Option<TaskPermissionPrompt> {
    let task = NolaStore::open_default()
        .ok()?
        .runtime_task(task_id)
        .ok()??;
    if !task_scope_matches(config, &task) || task.status != "waiting_permission" {
        return None;
    }
    let permission =
        crate::permission_request_port::pending_permission_for_task(task_id).ok()??;
    Some(render_task_permission_prompt(&task, permission))
}

fn render_task_permission_prompt(task: &DbRuntimeTask, permission: nola_application::PermissionRequest) -> TaskPermissionPrompt {
    let input = permission.input.clone();
    let args = input.as_object().cloned().unwrap_or_default();
    let request = tools::ToolRequest {
        id: format!("background-permission-{}", task.id),
        name: permission.capability.clone(),
        args,
    };
    let mut prompt = tools::permission_prompt_for_tool_request(&request);
    // Render the original child Core offer, not a tool-name or JSON heuristic.
    // Missing/foreign pending facts cannot advertise reusable authority.
    prompt.allow_conversation = permission.core_permission_request_id.as_ref().and_then(|core_id| {
        let projection = session::load_session_permission_projection(&permission.child_session_id).ok()??;
        projection.pending.iter()
            .find(|pending| pending.request_id.as_str() == core_id.as_str())
            .map(|pending| pending.allow_conversation)
    }).unwrap_or(false);
    prompt.kind = PermissionPromptKind::Agent;
    let localized_capability = crate::i18n::localize_tool_display_name(&permission.capability);
    prompt.title = format!(
        "{} `{}`",
        ui_text(UiText::SubagentPermissionRequest),
        localized_capability
    );
    prompt.rows.insert(
        0,
        (
            ui_text(UiText::Task).to_string(),
            format!("{} · {}", task.id, truncate_chars(&task.title, 72)),
        ),
    );
    if let Some(agent_type) = task.agent_type.clone().filter(|value| !value.trim().is_empty()) {
        prompt
            .rows
            .insert(1, (ui_text(UiText::Agent).to_string(), agent_type));
    }
    prompt.session_label = format!(
        "{}: {}",
        ui_text(UiText::AllowResourceForTask),
        localized_capability
    );
    prompt.session_value = permission.scope_rule.clone();
    TaskPermissionPrompt { prompt, permission }
}

pub fn resolve_task_permission_choice(
    config: &HostInvocationRequest,
    prompt: &TaskPermissionPrompt,
    choice: PermissionChoice,
) -> ToolOutcome {
    resolve_task_permission_in_scope(&config.tool_settings.project_root.to_string_lossy(),
        config.session.session_id.as_deref(), prompt, choice)
}

fn resolve_task_permission_in_scope(project_root: &str, owner_session_id: Option<&str>,
    prompt: &TaskPermissionPrompt, choice: PermissionChoice) -> ToolOutcome {
    if matches!(&choice, PermissionChoice::Session(_)) && !prompt.prompt.allow_conversation {
        return task_error("Conversation approval is not available for this invocation; choose allow once");
    }
    let decision = match choice {
        PermissionChoice::Once => BackgroundPermissionDecision::Allow,
        PermissionChoice::Session(_) => BackgroundPermissionDecision::AllowTask,
        PermissionChoice::Custom(note) => BackgroundPermissionDecision::Custom(note),
        PermissionChoice::ReturnToComposer => BackgroundPermissionDecision::ReturnToComposer,
        PermissionChoice::TransportInterrupted(reason) => {
            return task_error(format!(
                "Permission transport interrupted ({reason:?}); task remains awaiting permission"
            ))
        }
        PermissionChoice::Deny => BackgroundPermissionDecision::Deny,
    };
    resolve_pending_permission(project_root, owner_session_id, &prompt.permission, decision)
}

fn resolve_pending_permission(
    project_root: &str,
    owner_session_id: Option<&str>,
    expected: &nola_application::PermissionRequest,
    decision: BackgroundPermissionDecision,
) -> ToolOutcome {
    let task_id = expected.task_id.as_str();
    let store = match NolaStore::open_default() {
        Ok(store) => store,
        Err(error) => return task_error(error),
    };
    let Some(task) = (match store.runtime_task(task_id) {
        Ok(task) => task,
        Err(error) => return task_error(error),
    }) else {
        return task_error(format!("Unknown task: {task_id}"));
    };
    if !task_scope_matches_values(&task, project_root, owner_session_id) || task.status != "waiting_permission" {
        return task_error(format!("Task {task_id} is not waiting for permission"));
    }
    let permission = match crate::permission_request_port::permission_request(&expected.request_id) {
        Ok(Some(permission)) => permission,
        Ok(None) => return task_error("The displayed permission request no longer exists"),
        Err(error) => return task_error(error),
    };
    if &permission != expected || permission.status != nola_application::PermissionRequestStatus::Pending
        || task.child_session_id.as_deref() != Some(permission.child_session_id.as_str())
    {
        return task_error("The displayed permission request changed or closed; inspect the current request before choosing again");
    }
    let capability = permission.capability.clone();
    let (label, display) = match &decision {
        BackgroundPermissionDecision::Allow => (
            "allowed_once",
            format!("{} {task_id}", ui_text(UiText::AllowedTaskOnce)),
        ),
        BackgroundPermissionDecision::AllowTask => (
            "allowed_for_task",
            format!("{} {task_id}", ui_text(UiText::AllowResourceForTask)),
        ),
        BackgroundPermissionDecision::Custom(_) => (
            "custom_response",
            format!("{} {task_id}", ui_text(UiText::SentCustomTaskResponse)),
        ),
        BackgroundPermissionDecision::ReturnToComposer => (
            "returned_to_composer",
            crate::i18n::interaction_text()
                .custom_permission_return
                .to_string(),
        ),
        BackgroundPermissionDecision::Deny => (
            "denied",
            format!("{} {task_id}", ui_text(UiText::DeniedTaskRequest)),
        ),
    };
    let resolution = match &decision {
        BackgroundPermissionDecision::Allow => {
            crate::permission_request_port::authorize_permission(
                &permission.request_id,
                nola_application::PermissionAuthorizationDecision::AllowOnce,
                unix_epoch_millis(),
            )
        }
        BackgroundPermissionDecision::AllowTask => {
            crate::permission_request_port::authorize_permission(
                &permission.request_id,
                nola_application::PermissionAuthorizationDecision::AllowConversation,
                unix_epoch_millis(),
            )
        }
        BackgroundPermissionDecision::Deny => crate::permission_request_port::authorize_permission(
            &permission.request_id,
            nola_application::PermissionAuthorizationDecision::Deny,
            unix_epoch_millis(),
        ),
        BackgroundPermissionDecision::Custom(instruction) => {
            crate::permission_request_port::intervene_permission(
                &permission.request_id,
                nola_application::PermissionIntervention::ProvideInstruction {
                    instruction: instruction.clone(),
                },
                unix_epoch_millis(),
            )
        }
        BackgroundPermissionDecision::ReturnToComposer => {
            crate::permission_request_port::intervene_permission(
                &permission.request_id,
                nola_application::PermissionIntervention::ReturnToComposer,
                unix_epoch_millis(),
            )
        }
    };
    let newly_resolved = match resolution.and_then(|outcome| {
        crate::permission_request_port::validate_resolution(outcome, &permission)
    }) {
        Ok(newly_resolved) => newly_resolved,
        Err(error) => return task_error(error),
    };
    // Application atomically settles the derived grant and exact-scope
    // sibling waits. Clients only wake readers of those committed facts.
    if newly_resolved { let _ = append_task_event(
        task_id,
        "permission_resolved",
        "waiting_permission",
        Some(&format!("Permission {label}: {capability}")),
        json!({"request_id": permission.request_id, "capability": capability, "decision": label}),
    ); }
    wake_task_waiters();
    task_ok(
        json!({"task_id": task_id, "permission": label, "capability": capability}).to_string(),
        display,
    )
}

fn task_json(task: &DbRuntimeTask) -> Value {
    let metadata: Value = serde_json::from_str(&task.metadata_json).unwrap_or_else(|_| json!({}));
    let journal_result = if task.kind == "agent" && !is_live_status(&task.status) {
        crate::application_port::CliApplicationPort::persistent_default()
            .ok()
            .and_then(|mut app| {
                app.execute::<Option<nola_application::RuntimeTaskTerminalReceipt>>(
                    nola_application::ApplicationCommand::RuntimeTask {
                        command: nola_application::RuntimeTaskCommand::TerminalReceipt {
                            task_id: task.id.clone(),
                        },
                    },
                )
                .ok()
                .flatten()
            })
            .filter(|receipt| {
                receipt.task_id == task.id
                    && receipt.status == task.status
                    && receipt.child_session_id == task.child_session_id
                    && receipt.input_watermark == task.input_watermark
                    && receipt.completion_watermark == task.completion_watermark
                    && task.ended_at_ms == Some(receipt.ended_at_ms)
                    && Some(receipt.owner_session_id.as_str()) == task.owner_session_id.as_deref()
            })
            .and_then(|receipt| receipt.journal_result)
            .and_then(|result| serde_json::to_value(result).ok())
    } else {
        None
    };
    let usage_observed = metadata.get("terminal_usage_observed_at_ms").is_some();
    let mut scheduling = task
        .scheduling()
        .ok()
        .flatten()
        .and_then(|value| serde_json::to_value(value).ok());
    if !is_live_status(&task.status) && task.kind == "agent" {
        if let Some(scheduling) = scheduling.as_mut().and_then(Value::as_object_mut) {
            if metadata
                .get("public_output_observed")
                .and_then(Value::as_bool)
                != Some(true)
            {
                scheduling.insert("output_bytes".into(), Value::Null);
            }
            if !usage_observed {
                scheduling.insert("resource_usage".into(), Value::Null);
            }
        }
    }
    json!({
        "task_id": task.id,
        "kind": task.kind,
        "status": task.status,
        "title": task.title,
        "agent_type": task.agent_type,
        "parent_task_id": task.parent_task_id,
        "resumed_from_task_id": metadata.get("resumed_from_task_id"),
        "task_worktree_selection": metadata.pointer("/delegated_invocation/worktree/selection"),
        "child_session_id": task.child_session_id,
        "progress": task.progress_summary,
        "last_activity": task.last_activity,
        "result": journal_result.as_ref().and_then(|result| result.get("preview")).cloned()
            .or_else(|| (task.status != "sealed").then(|| task.result_summary.clone().map(Value::String)).flatten()),
        "result_available": task.status != "sealed" || journal_result.is_some(),
        "journal_result": journal_result,
        "run_result": journal_result.as_ref().and_then(|result| result.get("seal")).and_then(|seal| seal.get("result")),
        "result_digest": journal_result.as_ref().and_then(|result| result.get("seal")).and_then(|seal| seal.get("result_digest")),
        // Execution settlement is not acceptance of the parent's obligations.
        "delivery_acceptance": if task.kind == "agent" { Some("not_assessed") } else { None },
        "public_output_bytes": journal_result.as_ref().and_then(|result| result.get("public_utf8_bytes")),
        "resource_observation": if usage_observed { "observed" } else { "unknown" },
        "error": task.error,
        "unread": task.unread,
        "scheduling": scheduling,
        "durability": {
            "worker_id": task.worker_id,
            "lease_until_ms": task.lease_until_ms,
            "heartbeat_at_ms": task.heartbeat_at_ms,
            "retry_count": task.retry_count,
            "retry_at_ms": task.retry_at_ms,
            "last_error": task.last_error,
            "input_watermark": task.input_watermark,
            "completion_watermark": task.completion_watermark,
            "cancel_requested_at_ms": task.cancel_requested_at_ms,
        },
        "created_at_ms": task.created_at_ms,
        "started_at_ms": task.started_at_ms,
        "ended_at_ms": task.ended_at_ms,
    })
}

pub fn pending_task_context(config: &HostInvocationRequest) -> Option<(ContextComponent, Vec<String>)> {
    if !config.session.save_session {
        return None;
    }
    if config.session.session_id.is_none() {
        return None;
    }
    let root = config.tool_settings.project_root.to_string_lossy();
    let store = NolaStore::open_default().ok()?;
    let mut tasks = store.list_runtime_tasks(&root).ok()?;
    tasks.retain(|task| {
        task_scope_matches(config, task) && !is_live_status(&task.status) && task.unread
    });
    tasks.sort_by_key(|task| task.ended_at_ms.unwrap_or(task.updated_at_ms));
    if tasks.is_empty() {
        return None;
    }
    mailbox_context(&tasks)
}

/// Returns terminal task evidence owned by the latest parent Turn.
///
/// Goal verification runs in a fresh read-only session, so it cannot query
/// another conversation through list_tasks/read_task. This projection gives
/// the verifier the bounded canonical ledger slice for the executor Turn,
/// without trusting text written by the executor or exposing older tasks.
pub fn current_turn_task_context(config: &HostInvocationRequest) -> Option<(ContextComponent, Vec<String>)> {
    let session_id = config.session.session_id.as_deref()?;
    let projection = session::load_session_projection(session_id).ok()??;
    let latest_turn_id = projection
        .conversation_projection
        .order
        .iter()
        .rev()
        .filter_map(|part_id| projection.conversation_projection.parts.get(part_id))
        .map(|part| part.turn_id())
        .find(|turn_id| !turn_id.trim().is_empty())?;
    let task_ids = task_ids_owned_by_turn(&projection.task_projection, latest_turn_id);
    if task_ids.is_empty() {
        return None;
    }
    let root = config.tool_settings.project_root.to_string_lossy();
    let mut tasks = NolaStore::open_default()
        .ok()?
        .list_runtime_tasks(&root)
        .ok()?;
    tasks.retain(|task| {
        task_ids.contains(&task.id)
            && task_scope_matches(config, task)
            && !is_live_status(&task.status)
    });
    tasks.sort_by_key(|task| task.created_at_ms);
    let (mut component, ids) = mailbox_context(&tasks)?;
    component.id = format!("nola-goal-audit-task-ledger-{latest_turn_id}");
    component.source = Some(internal_task_context_source(
        "Canonical task ledger for the executor Turn",
    ));
    if let Value::String(content) = &mut component.content {
        content.insert_str(
            0,
            "Canonical task ledger evidence: Application loaded these terminal tasks from the parent executor Turn. Treat status, lineage, watermarks, and retained RunResult as read-only runtime facts. A sealed Task only means execution settled; Answered is not action completion, and no child result accepts the parent's requirements. Evaluate the parent's requirements against the exact result and evidence. The executor did not author this component.\n",
        );
        for task in &tasks {
            content.push_str(&format!(
                "\nTask ledger {}: owner_session={}, child_session={}, status={}, input_watermark={}, completion_watermark={}, started_at_ms={}, ended_at_ms={}",
                task.id,
                task.owner_session_id.as_deref().unwrap_or("none"),
                task.child_session_id.as_deref().unwrap_or("none"),
                task.status,
                task.input_watermark.map(|value| value.to_string()).unwrap_or_else(|| "none".into()),
                task.completion_watermark.map(|value| value.to_string()).unwrap_or_else(|| "none".into()),
                task.started_at_ms.map(|value| value.to_string()).unwrap_or_else(|| "none".into()),
                task.ended_at_ms.map(|value| value.to_string()).unwrap_or_else(|| "none".into()),
            ));
        }
    }
    Some((component, ids))
}

fn task_ids_owned_by_turn(
    projection: &nola_core::agent::task::TaskProjection,
    turn_id: &str,
) -> std::collections::HashSet<String> {
    projection
        .tasks
        .values()
        .filter(|task| task.owner_turn_id.as_str() == turn_id)
        .map(|task| task.id.clone())
        .collect()
}

/// Reconciles Host-owned process state before Application captures its one-shot
/// recovery snapshot. The snapshot remains the only read model consumed by
/// the recovered Goal; this step only projects external process facts.
pub fn reconcile_for_resume_snapshot(config: &HostInvocationRequest) {
    // Resume discovery runs before the CLI switches from its fresh empty
    // session to the canonical resumable session. Reconcile the whole
    // workspace here; filtering by the fresh session would hide completed
    // tasks owned by the conversation that is about to be restored.
    let mut workspace_config = config.clone();
    workspace_config.session.session_id = None;
    recover_orphaned_shell_tasks(&workspace_config);
    recover_orphaned_agent_tasks(&workspace_config);
}

pub fn resume_snapshot_task_context(
    snapshot: &nola_application::ResumeSnapshot,
) -> Option<(ContextComponent, Vec<String>)> {
    let (mut component, task_ids) = mailbox_context(&snapshot.mailbox_tasks).unwrap_or_else(|| {
        (
            ContextComponent {
                kind: ContextComponentKind::PlatformData,
                id: format!(
                    "nola-supervisor-reconciliation-{}",
                    snapshot.requested_at_ms
                ),
                content: Value::String(String::new()),
                source: Some(internal_task_context_source("Supervisor recovery report")),
            },
            Vec::new(),
        )
    });
    let mut report = vec![format!(
        "Supervisor recovery action: {:?}. Use this typed recovery report as authoritative; do not infer task state from display text.",
        snapshot.action
    )];
    for task in &snapshot.supervisor_reconciliation.tasks {
        report.push(format!(
            "Task {}: status={}, disposition={:?}, unread={}, lease_expired={}",
            task.task_id, task.status, task.disposition, task.unread, task.lease_expired
        ));
    }
    for side_effect in &snapshot.supervisor_reconciliation.side_effects {
        report.push(format!(
            "Side effect {}: phase={:?}, disposition={:?}",
            side_effect.transaction_id, side_effect.phase, side_effect.disposition
        ));
    }
    if let Value::String(content) = &mut component.content {
        if !content.is_empty() {
            content.push_str("\n\n");
        }
        content.push_str(&report.join("\n"));
    }
    Some((component, task_ids))
}

fn mailbox_context(tasks: &[DbRuntimeTask]) -> Option<(ContextComponent, Vec<String>)> {
    if tasks.is_empty() {
        return None;
    }
    let mut lines = vec![
        "Background task mailbox: these tasks finished since the parent last received results. Reconcile their evidence with the current request before acting; sub-agent output is advisory, and completion does not by itself prove the parent task is complete.".to_string(),
    ];
    for task in tasks {
        let result = task
            .result_summary
            .as_deref()
            .or(task.error.as_deref())
            .unwrap_or("No result was captured");
        lines.push(format!(
            "\nTask {} [{} / {}] {}\n{}",
            task.id,
            task.agent_type.as_deref().unwrap_or(&task.kind),
            task.status,
            task.title,
            truncate_chars(result, 8_000)
        ));
    }
    let task_ids = tasks.iter().map(|task| task.id.clone()).collect::<Vec<_>>();
    let component_id = format!(
        "nola-task-mailbox-{}-{}",
        task_ids.len(),
        task_ids.last().map(String::as_str).unwrap_or("empty")
    );
    Some((
        ContextComponent {
            kind: ContextComponentKind::PlatformData,
            id: component_id,
            content: Value::String(lines.join("\n")),
            source: Some(internal_task_context_source("Background task mailbox")),
        },
        task_ids,
    ))
}

pub fn mark_task_context_notified(task_ids: &[String]) -> Result<(), String> {
    let store = NolaStore::open_default()?;
    for task_id in task_ids {
        let Some(task) = store.runtime_task(task_id)? else {
            continue;
        };
        if is_live_status(&task.status) {
            continue;
        }
        // Context consumption is not evidence that any client displayed the
        // terminal notification. The UI acknowledges its own occurrence.
        crate::runtime_task_port::acknowledge_model_delivery(task_id)?;
    }
    Ok(())
}

#[test]
fn expired_dispatched_terminal_is_reconciled_without_replay() {
    let _lock = crate::store::test_env_lock();
    let root = tempfile::tempdir().unwrap();
    struct RestoreDatabase(Option<std::ffi::OsString>);
    impl Drop for RestoreDatabase {
        fn drop(&mut self) {
            match self.0.take() {
                Some(value) => std::env::set_var("NOLA_DB_PATH", value),
                None => std::env::remove_var("NOLA_DB_PATH"),
            }
        }
    }
    let _environment = RestoreDatabase(std::env::var_os("NOLA_DB_PATH"));
    std::env::set_var("NOLA_DB_PATH", root.path().join("fixture.db"));
    let mut config = HostInvocationRequest::for_test().unwrap();
    config.tool_settings.project_root = root.path().to_owned();
    config.session.session_id = Some("terminal-recovery-fixture".into());
    let now = unix_epoch_millis();
    let record = DbBackgroundTask {
        process_sandbox: None,
        id: "dispatched-terminal".into(),
        terminal: Some(nola_domain::terminal_input::TerminalSessionConfiguration {
            size: nola_domain::terminal_input::TerminalDimensions { rows: 24, columns: 80 },
            host_id: "fixture-host".into(),
        }),
        project_root: root.path().to_string_lossy().into_owned(),
        command: "printf unexpected-replay".into(),
        cwd: root.path().to_string_lossy().into_owned(),
        pid: 0,
        stdout_path: root.path().join("stdout").to_string_lossy().into_owned(),
        stderr_path: root.path().join("stderr").to_string_lossy().into_owned(),
        status_path: root.path().join("status").to_string_lossy().into_owned(),
        started_at_ms: now,
        owner_session_id: config.session.session_id.clone(),
        output_contract_json: None,
        sop_invocation: None,
    };
    let store = NolaStore::open_default().unwrap();
    store.save_background_task(&record).unwrap();
    std::fs::write(&record.stdout_path, "original-output\n").unwrap();
    let attempt = store.claim_runtime_task(&record.id, "fixture-worker", "generation-1",
        "fixture-token", now, now + 60_000).unwrap().unwrap();
    let lease = crate::runtime_task_port::register_runtime_task_attempt(attempt);
    mark_shell_worker_dispatch(&lease).unwrap();
    // A live lease must remain untouched even without a PID observation.
    recover_orphaned_shell_tasks(&config);
    assert_eq!(store.runtime_task(&record.id).unwrap().unwrap().status, "running");
    // Fault injection, not a claim of real process death or timeout waiting.
    expire_task_lease_for_test(&record.id, unix_epoch_millis()).unwrap();
    recover_orphaned_shell_tasks(&config);
    recover_orphaned_shell_tasks(&config);
    let recovered = store.runtime_task(&record.id).unwrap().unwrap();
    assert_eq!(recovered.status, "failed");
    assert_eq!(recovered.retry_count, 0);
    assert_eq!(std::fs::read_to_string(&record.stdout_path).unwrap(), "original-output\n");
    assert_eq!(std::fs::read_to_string(&record.status_path).unwrap(), "needs_reconciliation\n");
    let events = store.runtime_task_events(&record.id, 100).unwrap();
    let reconciliations: Vec<_> = events.iter().filter(|e| e.kind == "reconciliation_required").collect();
    assert_eq!(reconciliations.len(), 1, "cold observation must be idempotent");
    let detail: Value = serde_json::from_str(&reconciliations[0].detail_json).unwrap();
    assert_eq!(detail["safe_to_retry"], false);
    assert_eq!(detail["signal_requested"], false);
    assert!(!events.iter().any(|e| e.kind == "recovery_scheduled"));
}

fn recover_orphaned_shell_tasks(config: &HostInvocationRequest) {
    let root = config.tool_settings.project_root.to_string_lossy();
    let Ok(store) = NolaStore::open_default() else {
        return;
    };
    let Ok(tasks) = store.list_runtime_tasks(&root) else {
        return;
    };
    let records = store
        .list_background_tasks()
        .unwrap_or_default()
        .into_iter()
        .map(|task| (task.id.clone(), task))
        .collect::<HashMap<_, _>>();
    for mut runtime_task in tasks {
        if runtime_task.kind != "shell_process" {
            continue;
        }
        let Some(record) = records.get(&runtime_task.id).cloned() else {
            continue;
        };
        let now = unix_epoch_millis();
        if matches!(runtime_task.status.as_str(), "queued" | "retry_waiting")
            && runtime_task
                .retry_at_ms
                .is_none_or(|retry_at| retry_at <= now)
        {
            let _ = spawn_shell_task_process(record);
            continue;
        }
        if runtime_task.status != "running"
            || runtime_task.lease_until_ms.is_some_and(|lease| lease > now)
        {
            continue;
        }
        let reason = "The exact Prepared Shell worker delivery expired before dispatch; retrying the durable queued execution.";
        if store
            .recover_expired_runtime_task(&runtime_task.id, reason, now, now)
            .unwrap_or(false)
        {
            runtime_task.status = "retry_waiting".to_string();
            let _ = append_task_event(
                &runtime_task.id,
                "recovery_scheduled",
                "retry_waiting",
                Some(reason),
                json!({"safe_to_retry": true}),
            );
            let _ = spawn_shell_task_process(record);
            continue;
        }
        // Missing/stale PID is neither admission nor non-execution evidence.
        // Only the exact current Prepared proof above authorizes safe retry.

        // This cold record carries a PID, not an owned Child/Job handle or a
        // stable process-creation identity. Lease expiry is neither proof of
        // worker death nor authority to signal a possibly reused numeric PID.
        // Even Windows KILL_ON_JOB_CLOSE cannot be inferred without evidence
        // that the original, exactly bound Job owner actually closed it.
        let tree_stop = process_control::unowned_shell_tree_stop_fact(record.pid);
        let reason = "Shell worker lease expired after dispatch or without a verified non-execution witness; physical tree stop is unknown and requires output/side-effect reconciliation before retry. No owned process-tree object is available; the command will not be replayed.";
        let event_detail = json!({"pid": record.pid, "safe_to_retry": false,
            "process_tree_stop": tree_stop, "signal_requested": false}).to_string();
        if !store
            .fail_expired_runtime_task_with_event(
                &runtime_task.id, reason, "reconciliation_required", "failed",
                Some(reason), &event_detail, now,
            )
            .unwrap_or(false)
        {
            // A renewed/reclaimed/terminal Task is not owned by this stale
            // observation. Do not modify its status file or SOP wait.
            continue;
        }
        let _ = std::fs::write(&record.status_path, "needs_reconciliation\n");
        if let Some(invocation) = record.sop_invocation.as_ref() {
            let wait = SopWaitRecord {
                kind: SopWaitKind::UserInput,
                reference_id: format!("runtime-task:{}:reconciliation", runtime_task.id),
                reason: reason.to_string(),
                requested_at_epoch_ms: now,
                disposition: None,
                evidence_refs: Vec::new(),
                resolved_at_epoch_ms: None,
            };
            if let Err(error) = store.ensure_sop_wait(
                invocation.sop_run_id.clone(),
                invocation.sop_step_id.clone(),
                wait,
            ) {
                let _ = append_task_event(
                    &runtime_task.id,
                    "sop_reconciliation_wait_failed",
                    "failed",
                    Some(&error),
                    json!({"retryable": true}),
                );
            }
        }
        wake_task_waiters();
    }
}

fn validate_cold_permission_grant(
    resume: &ColdPermissionResume,
) -> Result<nola_application::RuntimeTaskPermissionContinuationGrant, String> {
    use nola_application::{ApplicationCommand, RuntimeTaskCommand};
    let fact: Option<nola_application::RuntimeTaskPermissionContinuationGrant> =
        crate::application_port::CliApplicationPort::persistent_default()?.execute(
            ApplicationCommand::RuntimeTask {
                command: RuntimeTaskCommand::PermissionContinuationGrant {
                    task_id: resume.suspension.task_id.clone(),
                    suspension_id: resume.suspension.id.clone(),
                    selector: nola_application::PermissionContinuationGrantSelector::ExactAttempt {
                        attempt_id: resume.grant.id.clone(),
                    },
                },
            },
        )?;
    let fact = fact.ok_or("cold permission has no committed resume grant")?;
    let original: Option<nola_application::RuntimeTaskAttempt> =
        crate::application_port::CliApplicationPort::persistent_default()?.execute(
            ApplicationCommand::RuntimeTask {
                command: RuntimeTaskCommand::Attempt {
                    attempt_id: resume.original.id.clone(),
                },
            },
        )?;
    let original = original.ok_or("cold permission original execution disappeared")?;
    if original.task_id != resume.original.task_id
        || original.worker_id != resume.original.worker_id
        || original.worker_generation != resume.original.worker_generation
        || original.lease_token != resume.original.lease_token
        || original.attempt_number != resume.original.attempt_number
        || original.claimed_at_ms != resume.original.claimed_at_ms
        || original.lease_expires_at_ms != resume.original.lease_expires_at_ms
    {
        return Err("cold permission original execution identity changed in transport".into());
    }
    if fact.granted_attempt != resume.grant
        || fact.suspension.attempt_id != resume.original.id
        || fact.suspension.permission_request_id != resume.suspension.permission_request_id
        || fact.suspension.continuation_id != resume.suspension.continuation_id
        || fact.suspension.continuation_json != resume.suspension.continuation_json
        || fact.suspension.task_id != resume.original.task_id
        || resume.original.lease_token == resume.grant.lease_token
        || resume.original.worker_generation == resume.grant.worker_generation
    {
        return Err("cold permission grant does not match its original continuation".into());
    }
    Ok(fact)
}

fn validate_durable_task_execution(
    owner_session_id: &str,
    task_id: &str,
    input_watermark: i64,
    lease: &crate::runtime_task_port::RuntimeTaskLeaseHandle,
) -> Result<(), String> {
    lease.with_attempt(|attempt| {
        let mut app = crate::application_port::CliApplicationPort::persistent_default()?;
        let leaf: Option<nola_domain::agent_task_admission::RuntimeAgentTaskAuthority> = app
            .execute(nola_application::ApplicationCommand::RuntimeTask {
                command: nola_application::RuntimeTaskCommand::AgentAuthority {
                    task_id: task_id.into(),
                },
            })?;
        let leaf = leaf.ok_or("Task authority disappeared")?;
        leaf.validate()?;
        let admission = leaf.admission.ok_or("Task is prepared, not admitted")?;
        let task: Option<DbRuntimeTask> =
            app.execute(nola_application::ApplicationCommand::RuntimeTask {
                command: nola_application::RuntimeTaskCommand::Get { id: task_id.into() },
            })?;
        let task = task.ok_or("Task execution disappeared")?;
        let current: Option<nola_application::RuntimeTaskAttempt> =
            app.execute(nola_application::ApplicationCommand::RuntimeTask {
                command: nola_application::RuntimeTaskCommand::CurrentAttempt {
                    task_id: task_id.into(),
                },
            })?;
        if admission.parent_conversation_id != owner_session_id
            || admission.task_id != task_id
            || task.child_session_id.as_deref() != Some(admission.child_conversation_id.as_str())
            || task.owner_session_id.as_deref() != Some(owner_session_id)
            || task.input_watermark != Some(input_watermark)
            || task.status != "running"
            || task.cancel_requested
            || current.as_ref() != Some(&*attempt)
        {
            return Err("Task execution has no live exact admitted authority".into());
        }
        Ok(())
    })
}

fn validate_cold_permission_execution(
    owner_session_id: &str,
    task_id: &str,
    input_watermark: i64,
    resume: &ColdPermissionResume,
    lease: &crate::runtime_task_port::RuntimeTaskLeaseHandle,
    cancellation: &RuntimeExecutionControl,
) -> Result<(), String> {
    validate_cold_permission_grant(resume)?;
    let current = lease.snapshot()?;
    if cancellation.is_requested()
        || current.id != resume.grant.id
        || current.task_id != resume.grant.task_id
        || current.worker_id != resume.grant.worker_id
        || current.worker_generation != resume.grant.worker_generation
        || current.lease_token != resume.grant.lease_token
        || current.attempt_number != resume.grant.attempt_number
        || current.claimed_at_ms != resume.grant.claimed_at_ms
    {
        return Err("cold permission execution does not own its exact continuation grant".into());
    }
    validate_durable_task_execution(owner_session_id, task_id, input_watermark, lease)
}

fn admit_cold_permission_resume(
    task: &DbRuntimeTask,
) -> Result<Option<ColdPermissionResume>, String> {
    use nola_application::{ApplicationCommand, RuntimeTaskCommand};
    let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
    let suspension: Option<nola_application::RuntimeTaskPermissionSuspension> = application
        .execute(ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::LatestPermissionSuspension {
                task_id: task.id.clone(),
            },
        })?;
    let Some(suspension) = suspension else {
        return Ok(None);
    };
    if !matches!(suspension.state.as_str(), "pending" | "resumed") {
        return Ok(None);
    }
    let permission =
        crate::permission_request_port::permission_request(&suspension.permission_request_id)?
            .ok_or("cold permission request disappeared")?;
    if background_permission_decision(permission.clone()).is_none() {
        return Ok(None);
    }
    let continuation: Value =
        serde_json::from_str(&suspension.continuation_json).map_err(|e| e.to_string())?;
    let child = task
        .child_session_id
        .as_deref()
        .ok_or("cold permission has no child")?;
    let run = application
        .active_run(child)?
        .ok_or("cold permission has no active child Run")?;
    if continuation.get("child_session_id").and_then(Value::as_str) != Some(child)
        || continuation.get("run_id").and_then(Value::as_str) != Some(run.run_id.as_str())
        || continuation
            .get("core_permission_request_id")
            .and_then(Value::as_str)
            != permission.core_permission_request_id.as_deref()
        || permission.task_id != task.id
        || permission.child_session_id != child
    {
        return Err("cold permission continuation identity does not match the child Run".into());
    }
    let original: Option<nola_application::RuntimeTaskAttempt> =
        application.execute(ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::Attempt {
                attempt_id: suspension.attempt_id.clone(),
            },
        })?;
    let original = original.ok_or("cold permission original attempt disappeared")?;
    crate::core_agent_runtime::ensure_cold_run_owner_absent(child, &run.run_id)?;
    let owner = task
        .owner_session_id
        .as_deref()
        .ok_or("cold permission has no parent session")?;
    let authority = crate::runtime_task_port::agent_authority(&task.id)?
        .ok_or("cold permission has no admitted authority")?;
    authority.validate()?;
    if authority.admission.as_ref().is_none_or(|admission| {
        admission.parent_conversation_id != owner || admission.child_conversation_id != child
    }) || task.cancel_requested
    {
        return Err("cold permission immutable Task ownership changed".into());
    }
    let now = unix_epoch_millis();
    let (max_running_global, max_running_project) = runtime_task_concurrency_limits();
    if suspension.state == "resumed" {
        let candidate: Option<nola_application::RuntimeTaskPermissionContinuationGrant> =
            application.execute(ApplicationCommand::RuntimeTask {
                command: RuntimeTaskCommand::PermissionContinuationGrant {
                    task_id: task.id.clone(),
                    suspension_id: suspension.id.clone(),
                    selector:
                        nola_application::PermissionContinuationGrantSelector::RecoveryCandidate,
                },
            })?;
        let candidate = candidate.ok_or("permission recovery has no verified predecessor")?;
        let predecessor: Option<nola_application::RuntimeTaskAttempt> =
            application.execute(ApplicationCommand::RuntimeTask {
                command: RuntimeTaskCommand::Attempt {
                    attempt_id: candidate.granted_attempt.id,
                },
            })?;
        let predecessor = predecessor.ok_or("permission predecessor disappeared")?;
        if predecessor.state != "leased" || predecessor.lease_expires_at_ms > now {
            return Ok(None);
        }
        let proposal = nola_application::RecoverRuntimeTaskPermissionContinuationFair {
            suspension_id: suspension.id.clone(),
            expected_suspension_revision: suspension.revision,
            permission_request_id: suspension.permission_request_id.clone(),
            continuation_id: suspension.continuation_id.clone(),
            expected_child_session_id: child.to_string(),
            expected_run_id: run.run_id,
            predecessor: nola_application::RuntimeTaskLeaseWitness::from(&predecessor),
            grant_attempt_id: uuid::Uuid::new_v4().to_string(),
            worker_id: format!(
                "cli-permission-successor:{}:{}",
                std::process::id(),
                uuid::Uuid::new_v4()
            ),
            worker_generation: uuid::Uuid::new_v4().to_string(),
            lease_token: uuid::Uuid::new_v4().to_string(),
            now_ms: now,
            lease_until_ms: now.saturating_add(agent_task_permission_lease_ms()),
            max_running_global,
            max_running_project,
            max_recoveries: agent_task_recovery_limit() as u32,
        };
        let grant: Result<Option<nola_application::RuntimeTaskAttempt>, String> = application
            .execute(ApplicationCommand::RuntimeTask {
                command: RuntimeTaskCommand::RecoverPermissionContinuationFair {
                    input: proposal.clone(),
                },
            });
        let grant = match grant {
            Ok(grant) => grant,
            Err(error) => {
                let proof: Option<nola_application::RuntimeTaskPermissionContinuationGrant> = application.execute(ApplicationCommand::RuntimeTask {
                    command: RuntimeTaskCommand::PermissionContinuationGrant {
                        task_id: task.id.clone(), suspension_id: suspension.id.clone(),
                        selector: nola_application::PermissionContinuationGrantSelector::ExactAttempt { attempt_id: proposal.grant_attempt_id.clone() },
                    },
                })?;
                Some(proof.ok_or(error)?.granted_attempt)
            }
        };
        let Some(grant) = grant else {
            return Ok(None);
        };
        if grant.id != proposal.grant_attempt_id
            || grant.worker_id != proposal.worker_id
            || grant.worker_generation != proposal.worker_generation
            || grant.lease_token != proposal.lease_token
            || grant.claimed_at_ms != proposal.now_ms
            || grant.lease_expires_at_ms != proposal.lease_until_ms
        {
            return Err("permission recovery acknowledgement belongs to another proposal".into());
        }
        let resume = ColdPermissionResume {
            suspension,
            original,
            grant,
        };
        validate_cold_permission_grant(&resume)?;
        return Ok(Some(resume));
    }
    let proposal = nola_application::ResumeRuntimeTaskAfterPermissionFair {
        task_id: task.id.clone(),
        suspension_id: suspension.id.clone(),
        expected_suspension_revision: suspension.revision,
        permission_request_id: suspension.permission_request_id.clone(),
        continuation_id: suspension.continuation_id.clone(),
        worker_id: format!(
            "cli-permission-recovery:{}:{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ),
        worker_generation: uuid::Uuid::new_v4().to_string(),
        lease_token: uuid::Uuid::new_v4().to_string(),
        now_ms: now,
        lease_until_ms: now.saturating_add(agent_task_permission_lease_ms()),
        max_running_global,
        max_running_project,
    };
    let grant: Result<Option<nola_application::RuntimeTaskAttempt>, String> =
        application.execute(ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::ResumeAfterPermissionFair {
                input: proposal.clone(),
            },
        });
    let grant = match grant {
        Ok(grant) => grant,
        Err(error) => {
            let fact: Option<nola_application::RuntimeTaskPermissionResume> =
                application.execute(ApplicationCommand::RuntimeTask {
                    command: RuntimeTaskCommand::PermissionResume {
                        task_id: task.id.clone(),
                        suspension_id: suspension.id.clone(),
                        permission_request_id: suspension.permission_request_id.clone(),
                        continuation_id: suspension.continuation_id.clone(),
                    },
                })?;
            Some(fact.ok_or(error)?.granted_attempt)
        }
    };
    let Some(grant) = grant else {
        return Ok(None);
    };
    if grant.worker_id != proposal.worker_id
        || grant.worker_generation != proposal.worker_generation
        || grant.lease_token != proposal.lease_token
        || grant.claimed_at_ms != proposal.now_ms
        || grant.lease_expires_at_ms != proposal.lease_until_ms
    {
        return Err("cold permission acknowledgement belongs to another proposal".into());
    }
    let resume = ColdPermissionResume {
        suspension,
        original,
        grant,
    };
    validate_cold_permission_grant(&resume)?;
    Ok(Some(resume))
}

fn agent_task_recovery_limit() -> i64 {
    std::env::var("NOLA_AGENT_TASK_MAX_RECOVERIES")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(3)
        .clamp(1, 20)
}

fn agent_task_permission_lease_ms() -> i64 {
    std::env::var("NOLA_AGENT_TASK_LEASE_MS")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(45_000)
        .clamp(1_000, 600_000)
}

// The reason belongs to the supervising Task. Core cancellation settles only
// the exact child Run; it must not start a normal Host or a replacement Turn.
fn cancel_bound_task_execution(task: &DbRuntimeTask, cancellation_id: &str, reason: &str) -> Result<bool, String> {
    let Some(binding) = crate::runtime_task_port::run_binding(&task.id)? else {
        return Err("Task stop has no exact Run binding".into());
    };
    if task.child_session_id.as_deref() != Some(binding.child_session_id.as_str())
        || task.owner_session_id.as_deref() != Some(binding.owner_session_id.as_str()) {
        return Err("Task stop identity differs".into());
    }
    let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
    if let Some(active) = application.active_run(&binding.child_session_id)? {
        if active.run_id != binding.run_id { return Err("Task stop Run differs".into()); }
        let assistant = active.assistant_message_id.as_deref().ok_or("Run has no assistant identity")?;
        let mut emitter = crate::native_runtime_output::NativeRuntimeOutput::with_event_sink(|_| {});
        crate::core_agent_runtime::stop_agent_core_expected_for_conversation(
            &binding.child_session_id, &mut emitter, &binding.run_id, assistant,
            cancellation_id, reason)?;
    }
    // This returns false while independent effects still require quiescence;
    // recovery will retry the cancel-only path, never claim ordinary execution.
    crate::runtime_task_port::reconcile_bound_run_terminal(&task.id, unix_epoch_millis())
}

/// The control API can outlive the foreground pump or the child process.
/// Drive the existing cancel-only path from persisted, already-owned intent.
pub(crate) fn reconcile_persisted_task_cancellation(task_id: &str) -> Result<(), String> {
    let task = NolaStore::open_default()?.runtime_task(task_id)?
        .ok_or("cancellation task is unavailable")?;
    if task.kind == "agent" && task.cancel_requested && is_live_status(&task.status) {
        cancel_bound_task_execution(&task,
            &format!("task-user-input-cancel:{}", task.id), "cancelled by user")?;
        wake_task_waiters();
    }
    Ok(())
}

fn reconcile_task_budget_stop(task: &DbRuntimeTask) -> Result<Option<bool>, String> {
    let observed;
    let task = if task.status == "waiting_permission" && !task.cancel_requested {
        // No worker lease exists while waiting. Only Application may derive
        // and CAS the elapsed budget against the exact permission suspension.
        observed = crate::runtime_task_port::observe_permission_wait_budget(&task.id, unix_epoch_millis())?;
        &observed
    } else { task };
    let scheduling = task.scheduling()?.ok_or("Task stop has no durable scheduling policy")?;
    let Some(_) = scheduling.measured_budget_failure(task.started_at_ms, unix_epoch_millis())? else {
        return Ok(None);
    };
    // Only a persisted, measured exhaustion authorizes this stop. Observation
    // failure, a process exit or an elapsed local timer cannot invent it.
    cancel_bound_task_execution(task, &format!("task-budget-stop:{}", task.id), "task resource budget exhausted").map(Some)
}

fn settle_worker_budget_stop(task_id: &str) -> bool {
    let result = NolaStore::open_default()
        .and_then(|store| store.runtime_task(task_id))
        .and_then(|task| task.ok_or_else(|| "Task budget stop record disappeared".to_string()))
        .and_then(|task| reconcile_task_budget_stop(&task));
    match result {
        Ok(Some(true)) => {
            wake_task_waiters();
            tools::wake_background_task_notification();
            true
        }
        Ok(_) => false,
        Err(error) => {
            crate::runtime_task_port::task_recovery_diagnostic(task_id, "budget_stop_reconciliation", error);
            false
        }
    }
}

fn recover_orphaned_agent_tasks(config: &HostInvocationRequest) {
    let root = config.tool_settings.project_root.to_string_lossy();
    let Ok(store) = NolaStore::open_default() else {
        return;
    };
    let Ok(tasks) = store.list_runtime_tasks(&root) else {
        return;
    };
    for mut task in tasks {
        if task.kind != "agent" {
            continue;
        }
        if !is_live_status(&task.status) {
            if replay_terminal_agent_reconciliation(&task) {
                wake_task_waiters();
            }
            continue;
        }
        // Prepared declarations are not execution authority. This immutable
        // fact was exported by the original committed Application, including
        // process-retained parents that do not exist in this durable database.
        if !crate::runtime_task_port::agent_authority(&task.id)
            .ok()
            .flatten()
            .is_some_and(|leaf| {
                leaf.phase
                    == nola_domain::agent_task_admission::RuntimeAgentTaskAuthorityPhase::Admitted
            })
        {
            continue;
        }
        match reconcile_task_budget_stop(&task) {
            Ok(None) => {},
            Ok(Some(_)) => {
                // Pending cancellation/quiescence is recovery work, never a
                // reason to claim another normal model execution attempt.
                wake_task_waiters();
                continue;
            }
            Err(error) => {
                crate::runtime_task_port::task_recovery_diagnostic(&task.id, "budget_stop_reconciliation", error);
                continue;
            }
        }
        if task.cancel_requested {
            let result = match crate::runtime_task_port::run_binding(&task.id) {
                Ok(Some(_)) => cancel_bound_task_execution(
                    &task, &format!("task-user-input-cancel:{}", task.id), "cancelled by user"),
                Ok(None) => store.request_runtime_task_cancel(&task.id, unix_epoch_millis()),
                Err(error) => Err(error),
            };
            match result {
                Ok(true) => {
                    wake_task_waiters();
                    tools::wake_background_task_notification();
                }
                Ok(false) => {} // Pending quiescence is not authority to respawn.
                Err(error) => {
                    crate::runtime_task_port::task_recovery_diagnostic(
                        &task.id, "cancel_requested_reconciliation", error);
                }
            }
            continue;
        }
        // A durable permission request is itself the owner of this wait. Do
        // not convert it into a retry or start a replacement worker before
        // the user resolves the exact structured request.
        if task.status == "waiting_permission"
            && crate::permission_request_port::pending_permission_for_task(&task.id)
                .ok()
                .flatten()
                .is_some()
        {
            continue;
        }
        // Durable questions are not orphaned workers or expired permissions.
        if task.status == "waiting_input" { continue; }
        let live_here = task_handles()
            .lock()
            .ok()
            .is_some_and(|handles| handles.contains_key(&task.id));
        if live_here {
            continue;
        }
        let now = unix_epoch_millis();
        let retry_limit = agent_task_recovery_limit();
        match crate::runtime_task_port::reconcile_bound_run_terminal(&task.id, now) {
            Ok(true) => {
                wake_task_waiters();
                tools::wake_background_task_notification();
                continue;
            }
            Err(_) => continue,
            Ok(false) => {}
        }
        if task.retry_count >= retry_limit && task.status != "waiting_permission" {
            let error = format!("The agent runner ended repeatedly; automatic recovery stopped after {retry_limit} attempts.");
            let _ = crate::runtime_task_port::fail_expired_with_event(
                &task.id,
                error.clone(),
                "recovery_exhausted",
                "failed",
                Some(error),
                json!({"retry_limit":retry_limit}).to_string(),
                now,
            );
            continue;
        }
        if task.status == "waiting_permission" {
            if let Ok(Some(resume)) = admit_cold_permission_resume(&task) {
                spawn_recovered_agent_task(config, task, Some(resume));
            }
            // A permission occurrence is not an expired ordinary attempt.
            // Keep unresolved or identity-rejected continuations intact.
            continue;
        }
        if task.status == "running" {
            let latest: Result<Option<nola_application::RuntimeTaskPermissionSuspension>, String> =
                crate::application_port::CliApplicationPort::persistent_default().and_then(
                    |mut app| {
                        app.execute(nola_application::ApplicationCommand::RuntimeTask {
                            command:
                                nola_application::RuntimeTaskCommand::LatestPermissionSuspension {
                                    task_id: task.id.clone(),
                                },
                        })
                    },
                );
            match latest {
                Err(_) => continue,
                Ok(Some(suspension)) if suspension.state == "resumed" => {
                    let Ok(continuation) =
                        serde_json::from_str::<Value>(&suspension.continuation_json)
                    else {
                        continue;
                    };
                    if continuation
                        .get("run_id")
                        .and_then(Value::as_str)
                        .is_none_or(str::is_empty)
                    {
                        continue;
                    }
                    let Some(child) = task.child_session_id.as_deref() else {
                        continue;
                    };
                    let active = crate::application_port::CliApplicationPort::persistent_default()
                        .and_then(|mut app| app.active_run(child));
                    match active {
                        Ok(Some(run))
                            if continuation["run_id"].as_str() == Some(run.run_id.as_str()) =>
                        {
                            if let Ok(Some(resume)) = admit_cold_permission_resume(&task) {
                                spawn_recovered_agent_task(config, task, Some(resume));
                            }
                            continue;
                        }
                        Ok(None) => {}
                        _ => continue,
                    }
                }
                _ => {}
            }
        }
        let recovery_reason =
            "The agent runner process ended before completion; resuming its durable child session.";
        if matches!(
            task.status.as_str(),
            "running" | "waiting_permission" | "suspended"
        ) {
            if !store
                .recover_expired_runtime_task(&task.id, recovery_reason, now, now)
                .unwrap_or(false)
            {
                continue;
            }
            task.status = "retry_waiting".to_string();
            task.retry_count = task.retry_count.saturating_add(1);
            task.retry_at_ms = Some(now);
            // The dead worker can no longer own its Host delivery. Mark the
            // child Run's delivery attempt interrupted before the successor
            // asks Core to rebind it. Canonical replay-safety still decides
            // whether that effect may be resumed.
            if let Some(child_session_id) = task.child_session_id.as_deref() {
                let _ = store.interrupt_orphaned_runs_for_session(child_session_id);
            }
            let _ = append_task_event(
                &task.id,
                "recovery_scheduled",
                "retry_waiting",
                Some(recovery_reason),
                json!({"retry_count": task.retry_count}),
            );
        }
        if matches!(task.status.as_str(), "queued" | "retry_waiting")
            && task.retry_at_ms.is_none_or(|retry_at| retry_at <= now)
        {
            spawn_recovered_agent_task(config, task, None);
        }
    }
}

pub fn reconcile_after_permission_resolution(config: &HostInvocationRequest) {
    recover_orphaned_agent_tasks(config);
}

fn recovered_agent_tool_schemas(
    registry: &AgentRegistry,
    agent_type: &str,
    child_config: &HostInvocationRequest,
    requested_read_only: bool,
) -> Result<Vec<Value>, String> {
    let skills = crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(
        child_config.tool_settings.skill_dirs.clone(),
    );
    // The profile may narrow this child's catalog, but must never replace its
    // source with the managed standalone catalog or hide an invalid root.
    skills.index()?;
    if !child_config.tool_settings.tools_enabled { return Ok(Vec::new()); }
    let mut host = crate::agent_host_driver::build_cli_host_catalog_with_skills("child-catalog", &skills);
    for descriptor in nola_core::agent::code_mode::capability_descriptors(&child_config.runtime.code_mode_limits)? {
        host.capabilities.insert(descriptor.id.clone(), descriptor);
    }
    let catalog = host.capabilities.values().filter(|capability|
        capability.route == nola_core::agent::host::HostCapabilityRoute::Tool
            && capability.availability == nola_core::agent::host::CapabilityAvailability::Available
            && capability.exposure.is_model_discoverable())
        .map(crate::agent_host_driver::discovery_tool_schema).collect::<Vec<_>>();
    Ok(registry.tool_schemas_for(agent_type, &catalog).into_iter().filter(|schema| {
        let Some(name) = schema.pointer("/function/name").and_then(Value::as_str) else { return false };
        child_config.tool_settings.tool_allowlist.as_ref()
            .is_none_or(|allowed| allowed.iter().any(|id| id.eq_ignore_ascii_case(name)))
            && (!requested_read_only || tools::tool_is_read_only(name))
    }).collect())
}

fn spawn_recovered_agent_task(
    config: &HostInvocationRequest,
    task: DbRuntimeTask,
    permission_resume: Option<ColdPermissionResume>,
) {
    let resumed_attempt = permission_resume
        .as_ref()
        .map(|resume| resume.grant.clone());
    let Some(owner_session_id) = task.owner_session_id.as_deref() else {
        return;
    };
    let Some(child_session_id) = task.child_session_id.as_deref() else {
        return;
    };
    let Some(agent_type) = task.agent_type.as_deref() else {
        return;
    };
    let Some(prompt) = task.prompt.as_deref() else {
        return;
    };
    let metadata = serde_json::from_str::<Value>(&task.metadata_json).unwrap_or_else(|_| json!({}));
    let restored = NolaStore::open_default().and_then(|store| {
        let snapshot: crate::delegated_invocation::DelegatedInvocation = serde_json::from_value(metadata["delegated_invocation"].clone())
            .map_err(|_| "Delegated invocation snapshot is missing or invalid".to_string())?;
        snapshot.restore(config, owner_session_id, &store)
    });
    let (profile, mut child_config) = match restored {
        Ok(value) => value,
        Err(error) => {
            let _ = crate::runtime_task_port::fail_expired_with_event(
                &task.id, error.clone(), "recovery_configuration_failed", "failed",
                Some(error), json!({"safe_to_retry": false}).to_string(), unix_epoch_millis(),
            );
            return;
        }
    };
    if profile.name != agent_type { return; }
    let registry = AgentRegistry::frozen(profile.clone());
    let requested_read_only = metadata
        .get("requested_read_only")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let max_turns = metadata
        .get("max_turns")
        .and_then(Value::as_u64)
        .map(|value| value.clamp(1, 20) as usize)
        .or(profile.max_turns);
    let cancellation = RuntimeExecutionControl::default();
    let control_id = uuid::Uuid::new_v4().to_string();
    let Ok(mut handles) = task_handles().lock() else {
        return;
    };
    if handles.contains_key(&task.id) {
        return;
    }
    handles.insert(
        task.id.clone(),
        AgentTaskHandle {
            control_id: control_id.clone(),
            lease: resumed_attempt.as_ref().map(|attempt| {
                crate::runtime_task_port::register_runtime_task_attempt(attempt.clone())
            }),
            execution: None,
            permission_resume: permission_resume.clone(),
            cancellation: cancellation.clone(),
            child_session_id: child_session_id.to_string(),
        },
    );
    drop(handles);

    // Recovery is scoped to the durable task's project, not to whichever
    // unrelated project happened to start the successor supervisor. Rebuild
    // the least-privilege filesystem scope used by the original delegated
    // child so the frozen Host policy remains stable across process takeover.
    if child_config.tool_settings.task_worktree.is_none() {
        child_config.tool_settings.project_root = std::path::PathBuf::from(&task.project_root);
    }
    child_config.tool_settings.allowed_dirs = vec![child_config.tool_settings.project_root.clone()];
    child_config.session.session_id = Some(child_session_id.to_string());
    child_config.session.save_session = true;
    child_config.input.message.clear();
    isolate_delegated_child_turn(&mut child_config);
    // Backend/model authority comes from the immutable delegated snapshot.
    // Only the child process's committed session identity is recovered here.
    child_config.execution.external_execution_session_id = metadata
        .get("external_session_id").and_then(Value::as_str).map(str::to_string);
    child_config.execution.max_iterations = max_turns.unwrap_or(child_config.execution.max_iterations);
    child_config.runtime.permissions.extend(
        profile
            .permission
            .iter()
            .filter(|rule| rule.action != crate::runtime_config::PermissionAction::Allow)
            .cloned(),
    );
    let child_catalog = match recovered_agent_tool_schemas(
        &registry,
        agent_type,
        &child_config,
        requested_read_only,
    ) {
        Ok(catalog) => catalog,
        Err(error) => {
            release_agent_task_control(&task.id, &control_id);
            let _ = append_task_event(
                &task.id,
                "recovery_spawn_failed",
                "retry_waiting",
                Some(&error),
                json!({}),
            );
            return;
        }
    };
    // The unrestricted profile is an authorization scope, not a request to
    // expose the full catalog. Reuse Host tiers and Core discovery. Explicit
    // narrow profiles retain their directly selected surface.
    child_config.tool_settings.tool_allowlist_is_authorization_only = profile.tools.is_empty();
    let unrestricted = profile.tools.is_empty()
        && child_config.tool_settings.tools_enabled
        && child_config.tool_settings.tool_allowlist.is_none()
        && !requested_read_only;
    child_config.tool_settings.tool_allowlist = if unrestricted { None } else { Some(
        child_catalog
            .iter()
            .filter_map(|schema| {
                schema
                    .pointer("/function/name")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .collect(),
    ) };
    let capability_scope = if requested_read_only {
        "Read-only. Write and command tools are absent from your catalog. If the assigned task requires an excluded operation, report that unmet requirement to the parent; do not repeatedly search for excluded tools or claim the operation ran."
    } else {
        "Profile-eligible tools. Every concrete invocation remains subject to Core permission checks."
    };
    child_config.input.host_context_components.push(ContextComponent {
        kind: ContextComponentKind::System,
        id: format!("nola-subagent-profile-{}", task.id),
        content: Value::String(format!(
            "You are the `{agent_type}` sub-agent for task `{}`.\n{}\n\nDelegated capability scope: {capability_scope}\n\n{}\n\nResume the delegated task from its durable child session. Complete only this task, preserve unrelated work, and report concise evidence to the parent agent.",
            task.id, profile.prompt, nola_domain::delegation::WORKSPACE_GUIDANCE
        )),
        source: Some(ContextComponentSource {
            platform: Some(ContextPlatform::Cli),
            title: Some(format!("Sub-agent profile: {agent_type}")),
            ..ContextComponentSource::default()
        }),
    });
    let task_id = task.id.clone();
    let title = task.title.clone();
    let prompt = prompt.to_string();
    let owner_session_id = owner_session_id.to_string();
    let input_watermark = task.input_watermark.unwrap_or(task.created_at_ms);
    let resumed_worker_id = resumed_attempt
        .as_ref()
        .map(|attempt| attempt.worker_id.clone())
        .unwrap_or_default();
    let resumed_lease_token = resumed_attempt
        .as_ref()
        .map(|attempt| attempt.lease_token.clone())
        .unwrap_or_default();
    let resumed_lease_duration_ms = resumed_attempt
        .as_ref()
        .map(|attempt| {
            attempt
                .lease_expires_at_ms
                .saturating_sub(unix_epoch_millis())
        })
        .unwrap_or_default();
    let spawn_result = spawn_agent_task_process(AgentTaskProcessRequest {
        task_id,
        title,
        prompt,
        owner_session_id,
        input_watermark,
        worker_id: resumed_worker_id,
        ownership_token: resumed_lease_token,
        runtime_task_attempt: resumed_attempt,
        lease_duration_ms: resumed_lease_duration_ms,
        config: child_config,
        permission_resume,
    });
    match spawn_result {
        Ok(true) => {}
        Ok(false) => {
            release_agent_task_control(&task.id, &control_id);
        }
        Err(error) => {
            release_agent_task_control(&task.id, &control_id);
            let _ = append_task_event(
                &task.id,
                "recovery_spawn_failed",
                "retry_waiting",
                Some(&error.to_string()),
                json!({}),
            );
        }
    }
}

fn isolate_delegated_child_turn(config: &mut HostInvocationRequest) {
    // A connection generation belongs to its original Host/session. The child
    // rebinds configured launch inputs and asks Core to authorize its own connection.
    config.tool_settings.mcp_capability_catalog = None;
    config.session.supports_reviewed_text_parts = false;
    config.session.host_task_execution = None;
    config.session.automation_execution_lease = None;
    config.session.automation_execution_spec = None;
    config.session.automation_artifacts.clear();
    // A delegated task is a new canonical conversation with its own Turn.
    // The spawning HostInvocationRequest may be a Goal executor or native-host request and
    // therefore carries one-shot user/context identities. Inheriting those
    // fields makes the child receive the parent's hidden Goal prompt beside
    // its delegated prompt, can expose goal_complete without a child Goal,
    // and lets a real model recursively repeat the parent's delegation.
    config.input.host_input_origin = nola_core::agent::AgentInputOrigin::InteractiveUser;
    config.input.host_turn_context = None;
    config.input.host_context_components.clear();
    config.input.host_attachments.clear();
    config.session.resume_snapshot = None;
    config.session.host_submission_id = None;
    config.session.host_user_message_id = None;
    config.session.host_assistant_message_id = None;
    config.session.expected_configuration_revision = None;
    config.session.expected_parent_message_id = None;
}

#[cfg(test)]
mod delegated_child_turn_isolation_tests {
    use super::*;

    #[test]
    fn child_never_inherits_parent_automation_claim() {
        let _environment = crate::store::test_env_lock();
        let mut config = HostInvocationRequest::for_test().unwrap();
        config.session.automation_execution_lease = Some(nola_domain::automation_execution::AutomationExecutionLease {
            occurrence_id: "parent-occurrence".into(), definition_revision: 1,
            worker_id: "parent-worker".into(), lease_token: "parent-lease".into(),
        });
        // Both the in-process fork and a serialized child launch must drop the
        // parent's one-shot admission authority.
        let encoded = serde_json::to_value(&config).unwrap();
        assert!(encoded.get("automation_execution_lease").is_none());
        isolate_delegated_child_turn(&mut config);
        assert!(config.session.automation_execution_lease.is_none());
    }

    #[test]
    fn child_never_inherits_parent_task_execution_authority() {
        let _environment = crate::store::test_env_lock();
        let mut config = HostInvocationRequest::for_test().unwrap();
        let lease = crate::runtime_task_port::register_runtime_task_attempt(
            nola_application::RuntimeTaskAttempt {
                id: "isolation-attempt".into(),
                task_id: "isolation-task".into(),
                attempt_number: 1,
                worker_id: "parent-worker".into(),
                worker_generation: "parent-generation".into(),
                lease_token: "parent-token".into(),
                lease_expires_at_ms: 100,
                state: "leased".into(),
                revision: 1,
                claimed_at_ms: 1,
            },
        );
        config.session.host_task_execution =
            Some(crate::runtime_task_port::RuntimeTaskExecutionAuthority::new(lease.clone()));
        isolate_delegated_child_turn(&mut config);
        assert!(config.session.host_task_execution.is_none());
        crate::runtime_task_port::release_runtime_task_handle(&lease);
    }

    #[test]
    fn parent_goal_turn_context_and_identities_do_not_enter_child_session() {
        let _environment = crate::store::test_env_lock();
        let mut config = crate::host_invocation_request::HostInvocationRequest::for_test().expect("delegated child config");
        config.input.host_input_origin = nola_core::agent::AgentInputOrigin::GoalContinuation;
        config.input.host_turn_context = Some("parent-private-goal-context".to_string());
        config.input.host_context_components.push(ContextComponent {
            kind: ContextComponentKind::System,
            id: "parent-turn-context".into(),
            content: Value::String("parent-only".into()),
            source: None,
        });
        config.session.host_submission_id = Some("parent-submission".into());
        config.session.host_user_message_id = Some("parent-user".into());
        config.session.host_assistant_message_id = Some("parent-assistant".into());
        config.session.expected_configuration_revision = Some(7);
        config.session.expected_parent_message_id = Some("parent-position".into());
        config.tool_settings.skill_dirs = Some(vec![std::path::PathBuf::from("/fixture/owner-skill-root")]);
        config.tool_settings.mcp_capability_catalog = Some(Default::default());
        config.tool_settings.mcp_server_registry = vec![json!({"server_id":"fixture-child","endpoint":"http://127.0.0.1:1/mcp"})];
        config.runtime.reasoning_tier = nola_domain::reasoning_tiers::ReasoningTier::Detailed;

        isolate_delegated_child_turn(&mut config);

        assert_eq!(config.runtime.reasoning_tier, nola_domain::reasoning_tiers::ReasoningTier::Detailed);
        assert_eq!(
            config.input.host_input_origin,
            nola_core::agent::AgentInputOrigin::InteractiveUser
        );
        assert!(config.input.host_turn_context.is_none());
        assert!(config.input.host_context_components.is_empty());
        assert!(config.session.host_submission_id.is_none());
        assert!(config.session.host_user_message_id.is_none());
        assert!(config.session.host_assistant_message_id.is_none());
        assert!(config.session.expected_configuration_revision.is_none());
        assert!(config.session.expected_parent_message_id.is_none());
        assert!(config.tool_settings.mcp_capability_catalog.is_none());
        assert_eq!(config.tool_settings.mcp_server_registry.len(), 1);
        assert_eq!(
            config.tool_settings.skill_dirs,
            Some(vec![std::path::PathBuf::from("/fixture/owner-skill-root")])
        );
    }
}

#[cfg(test)]
mod recovered_agent_catalog_tests {
    use super::*;
    use std::fs;

    fn config_with_skills(roots: Vec<std::path::PathBuf>) -> HostInvocationRequest {
        let mut config = HostInvocationRequest::for_test().expect("recovered child config");
        config.runtime = crate::host_invocation_request::HostRuntimeOptions::default();
        config.tool_settings.skill_dirs = Some(roots);
        isolate_delegated_child_turn(&mut config);
        config
    }

    #[test]
    fn recovery_uses_the_child_configured_catalog_and_keeps_profile_filters() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("skills");
        let skill = root.join("configured-child");
        fs::create_dir_all(&skill).unwrap();
        fs::write(skill.join("SKILL.md"), "---\nname: configured-child-only\ndescription: owned child workflow\n---\n\nUse only this configured workflow.\n").unwrap();
        let config = config_with_skills(vec![root]);
        let registry = AgentRegistry::from_profiles(&config.runtime.agents);
        let schemas = recovered_agent_tool_schemas(&registry, "general", &config, false).unwrap();
        assert!(schemas.iter().any(|schema| schema.pointer("/function/name").and_then(Value::as_str) == Some("code_mode_exec")));
        let mut narrow = config.clone();
        narrow.tool_settings.tool_allowlist = Some(vec!["read_file".into(), "tool_search".into()]);
        let restricted = recovered_agent_tool_schemas(&registry, "general", &narrow, false).unwrap();
        assert_eq!(restricted.len(), 2);
        assert!(!restricted.iter().any(|schema| schema.pointer("/function/name").and_then(Value::as_str) == Some("code_mode_exec")));
        narrow.tool_settings.tools_enabled = false;
        assert!(recovered_agent_tool_schemas(&registry, "general", &narrow, false).unwrap().is_empty());
        let invocation = schemas
            .iter()
            .find(|schema| {
                schema.pointer("/function/name").and_then(Value::as_str) == Some("Skill")
            })
            .expect("configured child Skill");
        assert_eq!(
            invocation.pointer("/function/parameters/properties/skill/enum"),
            Some(&json!(["configured-child-only"]))
        );
        assert!(schemas.iter().any(|schema| schema
            .pointer("/function/name")
            .and_then(Value::as_str)
            == Some("write_file")));
        assert!(!schemas.iter().any(|schema| schema
            .pointer("/function/name")
            .and_then(Value::as_str)
            == Some("spawn_agent")));
        let read_only = recovered_agent_tool_schemas(&registry, "general", &config, true).unwrap();
        assert!(read_only.iter().any(|schema| schema
            .pointer("/function/name")
            .and_then(Value::as_str)
            == Some("read_file")));
        assert!(!read_only.iter().any(|schema| schema
            .pointer("/function/name")
            .and_then(Value::as_str)
            == Some("write_file")));
        assert!(!read_only.iter().any(|schema| schema.pointer("/function/name").and_then(Value::as_str) == Some("code_mode_exec")));
        assert!(read_only.iter().all(|schema| tools::tool_is_read_only(
            schema
                .pointer("/function/name")
                .and_then(Value::as_str)
                .unwrap()
        )));
    }

    #[test]
    fn recovery_does_not_restore_managed_skills_for_disabled_or_invalid_sources() {
        let disabled = config_with_skills(Vec::new());
        let registry = AgentRegistry::from_profiles(&disabled.runtime.agents);
        let schemas = recovered_agent_tool_schemas(&registry, "general", &disabled, false).unwrap();
        for name in ["Skill", "skill_skip", "skill_update"] {
            assert!(!schemas
                .iter()
                .any(
                    |schema| schema.pointer("/function/name").and_then(Value::as_str) == Some(name)
                ));
        }
        let directory = tempfile::tempdir().unwrap();
        let invalid = config_with_skills(vec![directory.path().join("missing-owner-root")]);
        let error =
            recovered_agent_tool_schemas(&registry, "general", &invalid, false).unwrap_err();
        assert!(
            error.contains("configured Skill root") && error.contains("missing-owner-root"),
            "{error}"
        );
    }
}

#[cfg(test)]
fn expire_task_lease_for_test(task_id: &str, now_ms: i64) -> Result<(), String> {
    let database = std::env::var("NOLA_DB_PATH")
        .map_err(|_| "NOLA_DB_PATH is required for lease fault injection".to_string())?;
    let connection = rusqlite::Connection::open(database)
        .map_err(|error| format!("open task store for lease fault injection failed: {error}"))?;
    let changed = connection
        .execute(
            "UPDATE durable_tasks SET lease_until_ms = ?2, updated_at_ms = ?3 WHERE id = ?1",
            rusqlite::params![task_id, now_ms - 1, now_ms],
        )
        .map_err(|error| format!("expire task lease for fault injection failed: {error}"))?;
    if changed != 1 {
        return Err(format!("Unknown task: {task_id}"));
    }
    wake_task_waiters();
    Ok(())
}

fn append_task_event(
    task_id: &str,
    kind: &str,
    status: &str,
    summary: Option<&str>,
    detail: Value,
) -> Result<(), String> {
    crate::runtime_task_port::append_event(
        task_id,
        kind,
        status,
        summary.map(str::to_string),
        detail.to_string(),
        unix_epoch_millis(),
    )
    .map(|_| ())
}

fn task_handles() -> &'static Mutex<HashMap<String, AgentTaskHandle>> {
    AGENT_TASK_HANDLES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn task_wake() -> &'static (Mutex<u64>, Condvar) {
    TASK_WAKE.get_or_init(|| (Mutex::new(0), Condvar::new()))
}

fn wake_task_waiters() {
    let (generation, condvar) = task_wake();
    let mut value = generation
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *value = value.wrapping_add(1);
    condvar.notify_all();
    notify_parent_task_waiter();
}

fn notify_parent_task_waiter() {
    if !std::env::args().any(|argument| argument == AGENT_TASK_WORKER_ARG) {
        return;
    }
    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();
    let _ = stdout.write_all(&[1]);
    let _ = stdout.flush();
}

fn task_wake_generation() -> u64 {
    let (generation, _) = task_wake();
    *generation
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn wait_for_task_wake(observed_generation: u64, timeout: Duration) {
    let (generation, condvar) = task_wake();
    let guard = generation
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if *guard != observed_generation {
        return;
    }
    let _ = condvar.wait_timeout_while(guard, timeout, |current| *current == observed_generation);
}

fn background_permission_decision(
    request: nola_application::PermissionRequest,
) -> Option<BackgroundPermissionDecision> {
    if request.status != nola_application::PermissionRequestStatus::Resolved || request.closure.is_some() {
        return None;
    }
    match request.resolution? {
        nola_application::PermissionRequestResolution::Authorization {
            decision: nola_application::PermissionAuthorizationDecision::AllowOnce,
        } => Some(BackgroundPermissionDecision::Allow),
        nola_application::PermissionRequestResolution::Authorization {
            decision: nola_application::PermissionAuthorizationDecision::AllowConversation,
        } => Some(BackgroundPermissionDecision::AllowTask),
        nola_application::PermissionRequestResolution::Authorization {
            decision: nola_application::PermissionAuthorizationDecision::Deny,
        } => Some(BackgroundPermissionDecision::Deny),
        nola_application::PermissionRequestResolution::Intervention {
            intervention:
                nola_application::PermissionIntervention::ProvideInstruction { instruction },
        } => Some(BackgroundPermissionDecision::Custom(instruction)),
        nola_application::PermissionRequestResolution::Intervention {
            intervention: nola_application::PermissionIntervention::ReturnToComposer,
        } => Some(BackgroundPermissionDecision::ReturnToComposer),
    }
}

fn string_arg<'a>(args: &'a Map<String, Value>, name: &str) -> Option<&'a str> {
    args.get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn resumable_child_session(
    task: &DbRuntimeTask,
    project_root: &str,
    owner_session_id: &str,
    agent_type: &str,
) -> Result<String, String> {
    if canonical_project_root(Path::new(&task.project_root)) != canonical_project_root(Path::new(project_root)) {
        return Err("The requested Agent task belongs to another project".to_string());
    }
    if task.owner_session_id.as_deref() != Some(owner_session_id) {
        return Err("The requested Agent task belongs to another conversation".to_string());
    }
    if task.kind != "agent" {
        return Err("Only Agent tasks can be continued with spawn_agent".to_string());
    }
    if is_live_status(&task.status) {
        return Err(
            "The requested Agent task is still active; wait for or stop it first".to_string(),
        );
    }
    if !matches!(task.status.as_str(), "sealed" | "failed" | "cancelled") {
        return Err(format!(
            "The requested Agent task cannot be continued from state {}",
            task.status
        ));
    }
    if task.agent_type.as_deref() != Some(agent_type) {
        return Err(format!(
            "The requested Agent task used profile {}; resume it with the same agent_type",
            task.agent_type.as_deref().unwrap_or("unknown")
        ));
    }
    task.child_session_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "The requested Agent task has no resumable child session".to_string())
}

fn is_live_status(status: &str) -> bool {
    matches!(
        status,
        "queued"
            | "running"
            | "waiting_permission"
            | "waiting_input"
            | "suspended"
            | "retry_waiting"
            | "awaiting_reconciliation"
    )
}

fn unix_epoch_millis() -> i64 {
    nola_runtime_driver::monotonic_epoch_millis()
}

fn truncate_chars(value: &str, limit: usize) -> String {
    let mut output = value.chars().take(limit).collect::<String>();
    if value.chars().count() > limit {
        output.push('…');
    }
    output
}

fn nonempty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

fn task_ok(text: String, display: String) -> ToolOutcome {
    ToolOutcome {
        full_output_ref: None,
        data: None,
        execution: None,
        text,
        display,
        error: false,
        artifacts: Vec::new(),
        activation: None,
    }
}

fn task_error(message: impl Into<String>) -> ToolOutcome {
    let message = message.into();
    ToolOutcome {
        full_output_ref: None,
        data: None,
        execution: None,
        text: message.clone(),
        display: message,
        error: true,
        artifacts: Vec::new(),
        activation: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waiting_task_question_read_failure_is_not_an_empty_success() {
        let error = load_waiting_task_questions(Some("child"), |child| {
            assert_eq!(child, "child");
            Err("question query transport failed".to_string())
        }).unwrap_err();
        let outcome = task_error(error);
        assert!(outcome.error);
        assert_eq!(outcome.text, "question query transport failed");
        assert!(outcome.data.is_none());
        for child in [None, Some(" ")] {
            assert!(load_waiting_task_questions(child, |_| panic!("missing owner must not query")).is_err());
        }
        assert!(load_waiting_task_questions(Some("child"), |_| Ok(Vec::new())).unwrap().is_empty());
    }

    #[test]
    fn task_wait_observes_a_wake_that_precedes_the_block() {
        let observed = task_wake_generation();
        wake_task_waiters();
        let started = Instant::now();

        wait_for_task_wake(observed, Duration::from_secs(1));

        assert!(started.elapsed() < Duration::from_millis(100));
    }

    #[test]
    fn task_wait_blocks_until_a_structured_runtime_wake() {
        let observed = task_wake_generation();
        let notifier = std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(30));
            wake_task_waiters();
        });
        let started = Instant::now();

        wait_for_task_wake(observed, Duration::from_secs(1));

        notifier.join().expect("join task wake notifier");
        assert!(started.elapsed() >= Duration::from_millis(20));
        assert!(started.elapsed() < Duration::from_millis(250));
    }

    fn core_task_fixture(
        state: nola_core::agent::TaskState,
        completion_watermark: Option<i64>,
    ) -> nola_core::agent::task::TaskHandle {
        nola_core::agent::task::TaskHandle {
            id: "task-1".to_string(),
            owner_session_id: nola_core::agent::SessionId::from_string("parent".to_string()),
            owner_turn_id: nola_core::agent::TurnId::from_string("turn-1".to_string()),
            kind: nola_core::agent::TaskKind::Agent,
            state,
            progress: None,
            current_step: None,
            result_ref: None,
            cancel_requested: false,
            cancel_requested_at_epoch_ms: None,
            lineage: Default::default(),
            durability: nola_core::agent::TaskDurability {
                completion_watermark,
                ..Default::default()
            },
        }
    }

    #[test]
    fn goal_auditor_task_slice_contains_only_tasks_owned_by_executor_turn() {
        let mut projection = nola_core::agent::task::TaskProjection::default();
        let current = core_task_fixture(nola_core::agent::TaskState::Sealed, Some(7));
        projection.order.push(current.id.clone());
        projection.tasks.insert(current.id.clone(), current);
        let mut older = core_task_fixture(nola_core::agent::TaskState::Sealed, Some(5));
        older.id = "older-task".to_string();
        older.owner_turn_id = nola_core::agent::TurnId::from_string("older-turn".to_string());
        projection.order.push(older.id.clone());
        projection.tasks.insert(older.id.clone(), older);

        let selected = task_ids_owned_by_turn(&projection, "turn-1");

        assert_eq!(selected.len(), 1);
        assert!(selected.contains("task-1"));
        assert!(!selected.contains("older-task"));
    }

    #[test]
    fn single_terminal_result_primitive_preserves_independent_ui_delivery() {
        let _environment = crate::store::test_env_lock();
        let root =
            std::env::temp_dir().join(format!("nola-wait-task-delivery-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("create test root");
        let database = root.join("nola.db");
        std::env::set_var("NOLA_DB_PATH", &database);

        let session_id = "wait-task-parent";
        let task_id = "wait-task-result";
        let mut canonical = nola_store::NolaStore::open(&database).expect("open canonical store");
        canonical
            .create_conversation(&nola_store::CreateConversationInput {
                id: session_id.into(),
                title: "Wait task delivery".into(),
                revision: 1,
                settings_json: Some(format!(
                    "{{\"project_root\":{}}}",
                    serde_json::to_string(root.to_string_lossy().as_ref()).unwrap()
                )),
                created_at_ms: 1,
                updated_at_ms: 1,
            })
            .expect("create conversation");
        let stored: nola_store_port::runtime_task::StoredRuntimeTask =
            serde_json::from_value(serde_json::json!({
                "id": task_id,
                "kind": "agent",
                "status": "queued",
                "title": "Delivered result",
                "project_root": root.to_string_lossy(),
                "owner_session_id": session_id,
                "parent_tool_call_id": "wait-task-call",
                "child_session_id": "wait-task-child",
                "agent_type": "general",
                "result_summary": "WAIT-TASK-RESULT",
                "unread": true,
                "cancel_requested": false,
                "retry_count": 0,
                "input_watermark": 7,
                "created_at_ms": 1,
                "updated_at_ms": 2,
                "metadata_json": "{}"
            }))
            .expect("stored runtime task");
        canonical
            .create_runtime_task_with_event(&stored)
            .expect("create runtime task");
        // wait_task acknowledges the actual source conversation's resume
        // cursor, so retain its real Core admission, not a fabricated parent.
        let mut source = source_admission_support::open_runtime(&database).unwrap();
        let fact = source_admission_support::agent_task_admission::source_fact_for(
            &mut source,
            task_id,
            session_id,
            "wait-task-child",
            "wait-task-call",
            Some(7),
        );
        source
            .execute(
                nola_application::ApplicationCommand::ImportHostAgentTaskAdmission {
                    admission: fact,
                },
            )
            .unwrap();
        drop(source);
        let attempt = canonical
            .claim_runtime_task(
                task_id,
                "delivery-worker",
                "delivery-generation",
                "delivery-token",
                3,
                1000,
            )
            .unwrap()
            .unwrap();
        assert!(canonical
            .finish_runtime_task_detailed_with_event(
                &nola_application::RuntimeTaskLeaseWitness::from(&attempt),
                "failed",
                Some(7),
                Some("failed"),
                Some("WAIT-TASK-RESULT"),
                None,
                None,
                true,
                "finished",
                "failed",
                None,
                "{}",
                None,
                4,
            )
            .unwrap());
        drop(canonical);

        let mut config = crate::host_invocation_request::HostInvocationRequest::for_test().expect("wait task config");
        config.session.session_id = Some(session_id.into());
        config.tool_settings.project_root = root.clone();
        let task = NolaStore::open_default()
            .expect("open cli store")
            .runtime_task(task_id)
            .expect("load runtime task")
            .expect("runtime task exists");

        let cursor = crate::runtime_task_port::result_delivery_cursor(&task).unwrap();
        let business = json!({"completion_confirmed":true,"task":task_json(&task)}).to_string();
        let prepared = completed_wait_task_result(&task, business.clone(), "failed".into());
        assert!(!prepared.error);
        let typed: crate::runtime_task_port::RuntimeTaskResultData =
            serde_json::from_value(prepared.data.clone().unwrap()).unwrap();
        let crate::runtime_task_port::RuntimeTaskResultData::RuntimeTaskResultDelivery {
            cursor: prepared_cursor,
            text,
        } = typed;
        assert_eq!(prepared_cursor, cursor);
        assert_eq!(text, business);
        assert!(
            NolaStore::open_default()
                .unwrap()
                .runtime_task(task_id)
                .unwrap()
                .unwrap()
                .unread,
            "preparing a wait result must not consume a Task before its Host response commits"
        );
        session::acknowledge_wait_task_result(&cursor, "primitive-test")
            .expect("consume exact result primitive");

        let consumed = NolaStore::open_default()
            .expect("reopen cli store")
            .runtime_task(task_id)
            .expect("reload runtime task")
            .expect("runtime task remains");
        assert!(!consumed.unread);
        let metadata: Value = serde_json::from_str(&consumed.metadata_json).unwrap();
        assert_eq!(metadata["model_notified"], true);
        assert_ne!(
            metadata["ui_notified"], true,
            "model delivery is not UI presentation"
        );
        let args = Map::from_iter([("task_id".into(), Value::String(task_id.into()))]);
        let skills = crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(
            config.tool_settings.skill_dirs.clone(),
        );
        assert!(!read_task(&config, &args, &skills).error);
        assert!(!manage_runtime_tasks(&config, "status", Some(task_id)).error);
        let after_reads = NolaStore::open_default().unwrap().runtime_task(task_id).unwrap().unwrap();
        assert_eq!(after_reads.metadata_json, consumed.metadata_json,
            "model reads and UI detail preparation must not acknowledge presentation");
        let notifications = take_runtime_task_notifications(&config).unwrap();
        assert_eq!(
            notifications.len(),
            1,
            "a model-delivered result remains visible to the UI"
        );
        assert_eq!(notifications[0].id, task_id);
        assert_eq!(take_runtime_task_notifications(&config).unwrap().len(), 1,
            "preparing a notification, including after a lost renderer, is not delivery");
        assert!(deliver_runtime_task_notification(&notifications[0], || Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "renderer flush failed"))).is_err());
        assert_eq!(take_runtime_task_notifications(&config).unwrap().len(), 1,
            "failed presentation cannot acknowledge a notification");
        assert_eq!(deliver_runtime_task_notification(&notifications[0], || Ok(crate::terminal_ui::HistoryPresentation::Deferred)).unwrap(), None);
        assert_eq!(take_runtime_task_notifications(&config).unwrap().len(), 1);
        assert_eq!(deliver_runtime_task_notification(&notifications[0], || Ok(crate::terminal_ui::HistoryPresentation::Presented)).unwrap(), Some(nola_application::RuntimeTaskNotificationAcknowledgement::Delivered));
        assert_eq!(acknowledge_runtime_task_notification_delivery(&notifications[0]).unwrap(), nola_application::RuntimeTaskNotificationAcknowledgement::AlreadyDelivered);
        assert!(take_runtime_task_notifications(&config).unwrap().is_empty());

        std::env::remove_var("NOLA_DB_PATH");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn worker_terminal_state_is_not_completion_until_core_matches() {
        let running = core_task_fixture(nola_core::agent::TaskState::Running, None);
        assert!(!terminal_states_match(
            "sealed",
            Some(7),
            Some(&running),
            Some("receipt"),
            None
        ));

        let stale = core_task_fixture(nola_core::agent::TaskState::Sealed, Some(6));
        assert!(!terminal_states_match(
            "sealed",
            Some(7),
            Some(&stale),
            Some("receipt"),
            None
        ));

        let mut committed = core_task_fixture(nola_core::agent::TaskState::Sealed, Some(7));
        assert!(!terminal_states_match(
            "sealed",
            Some(7),
            Some(&committed),
            Some("receipt"),
            None
        ));
        committed.durability.terminal_receipt_id = Some("receipt".into());
        assert!(terminal_states_match(
            "sealed",
            Some(7),
            Some(&committed),
            Some("receipt"),
            None
        ));
        assert!(!terminal_states_match(
            "failed",
            Some(7),
            Some(&committed),
            Some("receipt"),
            None
        ));
        assert!(!terminal_states_match(
            "sealed",
            Some(7),
            Some(&committed),
            None,
            None
        ));
        assert!(!terminal_states_match(
            "sealed",
            Some(7),
            Some(&committed),
            Some("foreign"),
            None
        ));
        assert!(!terminal_states_match(
            "sealed",
            Some(7),
            Some(&committed),
            Some("receipt"),
            Some("foreign-result")
        ));
    }

    #[test]
    fn cold_stop_cannot_invent_denial_for_unbound_permission() {
        let _environment = crate::store::test_env_lock();
        let root = std::env::temp_dir().join(format!(
            "nola-cold-stop-permission-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).expect("create cold stop test root");
        std::env::set_var("NOLA_DB_PATH", root.join("nola.db"));
        let mut config = crate::host_invocation_request::HostInvocationRequest::for_test().expect("cold stop test config");
        config.tool_settings.project_root = root.clone();
        config.tool_settings.allowed_dirs = vec![root.clone()];
        config.session.session_id = Some("cold-stop-owner".to_string());
        let task_id = format!("task-{}", uuid::Uuid::new_v4());
        let mut task = DbRuntimeTask {
            id: task_id.clone(),
            kind: "agent".to_string(),
            status: "queued".to_string(),
            title: "Waiting for permission".to_string(),
            project_root: root.to_string_lossy().into_owned(),
            owner_session_id: config.session.session_id.clone(),
            parent_task_id: None,
            parent_tool_call_id: Some("spawn-call".to_string()),
            child_session_id: Some("cold-stop-child".to_string()),
            agent_type: Some("general".to_string()),
            prompt: Some("continue".to_string()),
            progress_summary: Some("waiting".to_string()),
            last_activity: None,
            result_summary: None,
            error: None,
            output_path: None,
            unread: true,
            cancel_requested: false,
            cancel_requested_at_ms: None,
            worker_id: None,
            ownership_token: None,
            lease_until_ms: None,
            heartbeat_at_ms: None,
            retry_count: 0,
            retry_at_ms: None,
            last_error: None,
            input_watermark: Some(1),
            completion_watermark: None,
            created_at_ms: 1,
            started_at_ms: None,
            ended_at_ms: None,
            updated_at_ms: 1,
            metadata_json: "{}".to_string(),
        };
        task.set_scheduling(nola_application::RuntimeTaskScheduling {
            priority: nola_application::RuntimeTaskPriority::Normal,
            queued_at_ms: task.created_at_ms,
            resource_budget: nola_application::RuntimeTaskResourceBudget::default(),
            output_bytes: 0,
            resource_usage: nola_application::RuntimeTaskResourceUsage::default(),
            budget_exhaustion: None,
        })
        .expect("attach queued scheduling metadata");
        NolaStore::open_default()
            .expect("open cold stop store")
            .create_runtime_task(&task)
            .expect("create waiting task");
        let permission = crate::permission_request_port::ensure_core_permission(
            nola_application::NewPermissionRequest {
                request_id: "cold-stop-permission".to_string(),
                task_id: task_id.clone(),
                child_session_id: "cold-stop-child".to_string(),
                capability: "write_file".to_string(),
                input: json!({"path": root.join("result.txt")}),
                scope_rule: format!("write_file:{}", root.display()),
                created_at_ms: 2,
                core_permission_request_id: Some("core-cold-stop-permission".to_string()),
                permission_instructions: Vec::new(),
            },
        )
        .expect("persist pending permission");

        let mut other_config = config.clone();
        other_config.session.session_id = Some("other-session".to_string());
        assert!(permission_prompt_for_task(&other_config, &task_id).is_none());
        let displayed = TaskPermissionPrompt {
            prompt: tools::permission_prompt_for_tool_request(&tools::ToolRequest {
                id: permission.request_id.clone(), name: permission.capability.clone(),
                args: permission.input.as_object().cloned().unwrap_or_default(),
            }),
            permission: permission.clone(),
        };
        let cross_session_resolution =
            resolve_task_permission_choice(&other_config, &displayed, PermissionChoice::Deny);
        assert!(cross_session_resolution.error);
        let still_pending =
            crate::permission_request_port::permission_request(&permission.request_id)
                .expect("read cross-session permission")
                .expect("permission exists");
        assert_eq!(
            still_pending.status,
            nola_application::PermissionRequestStatus::Pending
        );

        let outcome = stop_task(
            &config,
            &Map::from_iter([("task_id".to_string(), Value::String(task_id.clone()))]),
            &crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(
                config.tool_settings.skill_dirs.clone(),
            ),
        );
        assert!(!outcome.error, "{}", outcome.text);
        let retained = crate::permission_request_port::permission_request(&permission.request_id)
            .expect("read permission")
            .expect("permission exists");
        // This fixture has no actual Core Run/effect or suspension. The Task's
        // own cancellation receipt closes its permission surface without
        // pretending that the user denied it or a Core effect was withdrawn.
        assert_eq!(retained.status, nola_application::PermissionRequestStatus::Withdrawn);
        assert_eq!(retained.resolution, permission.resolution);
        assert_eq!(retained.resolved_at_ms, permission.resolved_at_ms);
        assert!(matches!(retained.closure.as_ref().unwrap().source,
            nola_domain::permission_closure::PermissionClosureSource::UnboundTaskTerminal { .. }));
        assert!(matches!(
            crate::permission_request_port::authorize_permission(&permission.request_id,
                nola_application::PermissionAuthorizationDecision::AllowOnce, unix_epoch_millis()).unwrap(),
            nola_application::PermissionResolutionOutcome::Closed { request }
                if request == retained
        ));
    }

    #[test]
    fn persisted_return_to_composer_maps_to_background_cancellation_decision() {
        let request = nola_application::PermissionRequest {
            closure: None,
            core_permission_request_id: Some("core-fixture-request".into()),
            permission_instructions: Vec::new(),
            request_id: "permission-1".to_string(),
            task_id: "task-1".to_string(),
            child_session_id: "child-1".to_string(),
            capability: "write_file".to_string(),
            input: json!({}),
            scope_rule: "write_file:/tmp".to_string(),
            status: nola_application::PermissionRequestStatus::Resolved,
            resolution: Some(
                nola_application::PermissionRequestResolution::Intervention {
                    intervention: nola_application::PermissionIntervention::ReturnToComposer,
                },
            ),
            created_at_ms: 1,
            resolved_at_ms: Some(2),
        };
        assert!(matches!(
            background_permission_decision(request),
            Some(BackgroundPermissionDecision::ReturnToComposer)
        ));
    }

    #[test]
    fn task_json_exposes_stable_public_state() {
        let mut task = DbRuntimeTask {
            id: "task-1".to_string(),
            kind: "agent".to_string(),
            status: "running".to_string(),
            title: "Inspect code".to_string(),
            project_root: "/workspace".to_string(),
            owner_session_id: Some("parent".to_string()),
            parent_task_id: None,
            parent_tool_call_id: Some("call".to_string()),
            child_session_id: Some("child".to_string()),
            agent_type: Some("explore".to_string()),
            prompt: Some("Inspect".to_string()),
            progress_summary: Some("Searching".to_string()),
            last_activity: Some("grep".to_string()),
            result_summary: None,
            error: None,
            output_path: None,
            unread: false,
            cancel_requested: false,
            cancel_requested_at_ms: None,
            worker_id: None,
            ownership_token: None,
            lease_until_ms: None,
            heartbeat_at_ms: None,
            retry_count: 0,
            retry_at_ms: None,
            last_error: None,
            input_watermark: None,
            completion_watermark: None,
            created_at_ms: 1,
            started_at_ms: Some(2),
            ended_at_ms: None,
            updated_at_ms: 3,
            metadata_json: "{}".to_string(),
        };
        let value = task_json(&task);
        assert_eq!(value["task_id"], "task-1");
        assert_eq!(value["status"], "running");
        assert_eq!(value["agent_type"], "explore");
        assert!(value["parent_task_id"].is_null());
        assert!(value.get("prompt").is_none());
        assert!(task_scope_matches_values(
            &task,
            "/workspace",
            Some("parent")
        ));
        assert!(!task_scope_matches_values(
            &task,
            "/workspace",
            Some("other")
        ));
        assert!(!task_scope_matches_values(&task, "/workspace", None));
        task.owner_session_id = None;
        assert!(!task_scope_matches_values(
            &task,
            "/workspace",
            Some("other")
        ));
        assert!(task_scope_matches_values(&task, "/workspace", None));
        assert!(!task_scope_matches_values(&task, "/other", Some("parent")));
    }

    #[test]
    fn another_session_cannot_observe_control_or_consume_a_persisted_agent_task() {
        let _environment = crate::store::test_env_lock();
        let root = std::env::temp_dir().join(format!(
            "nola-cross-session-task-scope-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).expect("create task scope test root");
        let database = root.join("nola.db");
        std::env::set_var("NOLA_DB_PATH", &database);

        let mut owner_config = crate::host_invocation_request::HostInvocationRequest::for_test().expect("owner task config");
        owner_config.tool_settings.project_root = root.clone();
        owner_config.tool_settings.allowed_dirs = vec![root.clone()];
        owner_config.session.session_id = Some("owner-session".to_string());

        let task_id = format!("task-{}", uuid::Uuid::new_v4());
        let now = unix_epoch_millis();
        let mut task = DbRuntimeTask {
            id: task_id.clone(),
            kind: "agent".to_string(),
            status: "queued".to_string(),
            title: "Private child task".to_string(),
            project_root: root.to_string_lossy().into_owned(),
            owner_session_id: owner_config.session.session_id.clone(),
            parent_task_id: None,
            parent_tool_call_id: Some("private-spawn-call".to_string()),
            child_session_id: Some("private-child".to_string()),
            agent_type: Some("general".to_string()),
            prompt: Some("private prompt".to_string()),
            progress_summary: Some("queued".to_string()),
            last_activity: None,
            result_summary: None,
            error: None,
            output_path: None,
            unread: true,
            cancel_requested: false,
            cancel_requested_at_ms: None,
            worker_id: None,
            ownership_token: None,
            lease_until_ms: None,
            heartbeat_at_ms: None,
            retry_count: 0,
            retry_at_ms: None,
            last_error: None,
            input_watermark: Some(now),
            completion_watermark: None,
            created_at_ms: now,
            started_at_ms: None,
            ended_at_ms: None,
            updated_at_ms: now,
            metadata_json: "{}".to_string(),
        };
        task.set_scheduling(nola_application::RuntimeTaskScheduling {
            priority: nola_application::RuntimeTaskPriority::Normal,
            queued_at_ms: now,
            resource_budget: nola_application::RuntimeTaskResourceBudget::default(),
            output_bytes: 0,
            resource_usage: nola_application::RuntimeTaskResourceUsage::default(),
            budget_exhaustion: None,
        })
        .expect("attach scheduling metadata");
        NolaStore::open(&database)
            .expect("open task scope store")
            .create_runtime_task(&task)
            .expect("persist private task");
        source_admission_support::agent_task_admission::admit_existing(
            &database,
            &task_id,
            "owner-session",
            "private-child",
            "private-spawn-call",
            Some(now),
        );

        let mut other_config = owner_config.clone();
        other_config.session.session_id = Some("other-session".to_string());
        let args = Map::from_iter([("task_id".to_string(), Value::String(task_id.clone()))]);
        assert_eq!(running_task_count(&other_config), 0);
        let listed: Value = serde_json::from_str(&list_tasks(&other_config).text)
            .expect("decode task list for other session");
        assert_eq!(listed["count"], 0);
        let skills = crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(
            other_config.tool_settings.skill_dirs.clone(),
        );
        let read = read_task(&other_config, &args, &skills);
        assert!(read.error);
        assert!(read.text.contains("Unknown task"));
        let stop = stop_task(&other_config, &args, &skills);
        assert!(stop.error);
        assert!(stop.text.contains("Unknown task"));

        let reopened = NolaStore::open(&database).expect("reopen task scope store");
        let persisted = reopened
            .runtime_task(&task_id)
            .expect("read private task")
            .expect("private task exists");
        assert!(!persisted.cancel_requested);
        assert_eq!(persisted.owner_session_id.as_deref(), Some("owner-session"));
        assert_eq!(running_task_count(&owner_config), 1);

        assert!(!list_tasks(&owner_config).error);
        assert!(!manage_runtime_tasks(&owner_config, "list", None).error);
        assert!(!manage_runtime_tasks(&owner_config, "status", Some(&task_id)).error);
        assert!(pending_task_context(&owner_config).is_none());
        let observed = reopened.runtime_task(&task_id).unwrap().unwrap();
        assert_eq!(observed.status, "queued", "queries must not launch an admitted worker");
        assert!(observed.worker_id.is_none());
        let database_check = rusqlite::Connection::open(&database).unwrap();
        let attempts: i64 = database_check.query_row(
            "SELECT COUNT(*) FROM runtime_task_attempts WHERE task_id=?1",
            [&task_id], |row| row.get(0),
        ).unwrap();
        assert_eq!(attempts, 0, "query composition must not create attempts");

        let attempt = reopened
            .claim_runtime_task(
                &task_id,
                "scope-test-worker",
                "scope-test-generation",
                "scope-test-token",
                now + 1,
                now + 10_000,
            )
            .expect("claim private task")
            .expect("private task is claimable");
        assert_eq!(attempt.task_id, task_id);
        let lease = crate::runtime_task_port::register_runtime_task_attempt(attempt);
        // A cancelled wait is a completed local Execute invocation, not proof
        // that the independently leased child stopped. Exercise the actual
        // wait_task wrapper against this admitted, running durable task.
        let cancellation = RuntimeExecutionControl::default();
        cancellation.request();
        let context = ToolContext {
            memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
            language_services: Default::default(),
            output_store: tools::ToolOutputStore::for_test(),
            skills: crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(None),
            allowed_dirs: vec![root.clone()],
            policy: tools::ToolExecutionPolicy::new(Vec::new(), false, "ask".into(), Vec::new()),
            permission_scope: Default::default(),
            cancellation,
            plan_state: Default::default(),
            browser: tools::BrowserRuntime::new(root.join("browser")),
        };
        let waited = wait_task(&owner_config, &args, &context);
        assert!(waited.error, "cancelled waiting must remain an error result");
        assert!(waited.text.contains("Task wait cancelled"), "{}", waited.text);
        assert_eq!(waited.execution.as_ref().map(|receipt| receipt.state),
            Some(nola_host_protocol::tool_result::ExecutionState::Completed));
        let still_running = reopened.runtime_task(&task_id).unwrap().unwrap();
        assert_eq!(still_running.status, "running");
        assert!(!still_running.cancel_requested, "wait cancellation cannot cancel its child");
        assert!(still_running.ended_at_ms.is_none());
        assert_eq!(still_running.ownership_token, Some("scope-test-token".into()));
        let attempt_id = lease.snapshot().unwrap().id;
        reopened.append_runtime_task_event(&task_id, "worker_process_exited", "suspended",
            Some("worker exited before completion"), &json!({"attempt_id":attempt_id}).to_string(),
            unix_epoch_millis()).unwrap();
        let mut context = context;
        context.cancellation = RuntimeExecutionControl::default();
        let mut exit_args = args.clone();
        exit_args.insert("timeout_sec".into(), json!(1));
        let exit = wait_task(&owner_config, &exit_args, &context);
        assert!(!exit.error, "{}", exit.text);
        let exit:Value=serde_json::from_str(&exit.text).unwrap();
        assert_eq!(exit["action_required"], "recovery");
        assert_eq!(exit["wait_timed_out"], false);
        assert_eq!(exit["completion_confirmed"], false);
        assert_eq!(reopened.runtime_task(&task_id).unwrap().unwrap().status, "running");
        assert!(reopened
            .finish_runtime_task_detailed_with_event(
                &lease,
                "failed",
                Some(now),
                Some("failed"),
                Some("private result"),
                None,
                None,
                true,
                "finished",
                "failed",
                Some("private task finished"),
                "{}",
                now + 2,
            )
            .expect("finish private task"));
        assert!(take_runtime_task_notifications(&other_config)
            .expect("read other-session notifications")
            .is_empty());
        assert!(
            reopened
                .runtime_task(&task_id)
                .expect("read private notification state")
                .expect("private task exists")
                .unread
        );
        let owner_notifications =
            take_runtime_task_notifications(&owner_config).expect("read owner notifications");
        assert_eq!(owner_notifications.len(), 1);
        assert_eq!(owner_notifications[0].id, task_id);

        std::env::remove_var("NOLA_DB_PATH");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn completed_agent_task_can_resume_only_inside_its_owner_boundary() {
        let mut task = DbRuntimeTask {
            id: "task-1".to_string(),
            kind: "agent".to_string(),
            status: "sealed".to_string(),
            title: "Inspect code".to_string(),
            project_root: "/workspace".to_string(),
            owner_session_id: Some("parent".to_string()),
            parent_task_id: None,
            parent_tool_call_id: Some("call".to_string()),
            child_session_id: Some("child".to_string()),
            agent_type: Some("explore".to_string()),
            prompt: Some("Inspect".to_string()),
            progress_summary: None,
            last_activity: None,
            result_summary: Some("Done".to_string()),
            error: None,
            output_path: None,
            unread: false,
            cancel_requested: false,
            cancel_requested_at_ms: None,
            worker_id: None,
            ownership_token: None,
            lease_until_ms: None,
            heartbeat_at_ms: None,
            retry_count: 0,
            retry_at_ms: None,
            last_error: None,
            input_watermark: Some(1),
            completion_watermark: Some(2),
            created_at_ms: 1,
            started_at_ms: Some(1),
            ended_at_ms: Some(2),
            updated_at_ms: 2,
            metadata_json: "{}".to_string(),
        };

        assert_eq!(
            resumable_child_session(&task, "/workspace", "parent", "explore").unwrap(),
            "child"
        );
        assert!(resumable_child_session(&task, "/other", "parent", "explore").is_err());
        assert!(resumable_child_session(&task, "/workspace", "other", "explore").is_err());
        task.status = "running".to_string();
        assert!(resumable_child_session(&task, "/workspace", "parent", "explore").is_err());
    }

    #[test]
    fn bounded_output_preserves_utf8_and_requests_cancellation_once_limit_is_reached() {
        let mut output = String::new();
        let failure = Arc::new(Mutex::new(None));
        let cancellation = RuntimeExecutionControl::default();
        let observed = std::sync::atomic::AtomicU64::new(0);
        append_bounded_output(&mut output, "ab水面", &observed, 5, &failure, &cancellation);
        assert_eq!(output, "ab水");
        assert_eq!(observed.load(std::sync::atomic::Ordering::Relaxed), 8);
        assert!(cancellation.is_requested());
        assert!(failure
            .lock()
            .unwrap()
            .as_deref()
            .is_some_and(|reason| reason.contains("5 bytes")));
        append_bounded_output(&mut output, "later", &observed, 5, &failure, &cancellation);
        assert_eq!(output, "ab水");
    }
}

// These records come from the Application task ledger, not an OS data source.
fn internal_task_context_source(title: &str) -> ContextComponentSource {
    ContextComponentSource {
        platform: Some(ContextPlatform::Cli),
        title: Some(title.to_string()),
        authorization: Some("not_required".to_string()),
        ..ContextComponentSource::default()
    }
}

#[cfg(test)]
mod internal_task_context_authorization_tests {
    use super::*;

    #[test]
    fn supervisor_context_survives_shared_authorization_gate() {
        for title in ["Supervisor recovery report", "Background task mailbox"] {
            let component = ContextComponent {
                kind: ContextComponentKind::PlatformData,
                id: "task-ledger-context".into(),
                content: Value::String("Task is waiting for user input".into()),
                source: Some(internal_task_context_source(title)),
            };
            let prepared = nola_core::context_engine::prepare_context_components(
                ContextPlatform::Cli,
                &[component],
                4096,
            );
            assert!(serde_json::to_string(&prepared.messages)
                .unwrap()
                .contains("Task is waiting for user input"));
            assert!(prepared
                .component_metrics
                .iter()
                .any(|metric| metric.used_ids.contains(&"task-ledger-context".to_string())));
        }
    }
}
