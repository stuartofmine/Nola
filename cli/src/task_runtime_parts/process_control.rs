// SPDX-License-Identifier: Apache-2.0
//! Bounded resource observations and lease ownership independent of sampling.

use crate::runtime_task_port::RuntimeTaskLeaseHandle;
use crate::store::NolaStore;
use std::path::Path;
use std::sync::{atomic::{AtomicBool, AtomicI64, Ordering}, Arc, Mutex, mpsc};
use std::time::Duration;
use std::time::Instant;

#[cfg(windows)]
#[path = "process_tree_windows.rs"]
mod windows_tree;

pub(super) struct ShellLeaseHeartbeat {
    stop: mpsc::Sender<()>,
    thread: Option<std::thread::JoinHandle<()>>,
    watchdog_stop: mpsc::Sender<()>,
    watchdog: Option<std::thread::JoinHandle<()>>,
    lost: Arc<AtomicBool>,
    child: Arc<Mutex<Option<TrackedChild>>>,
    #[cfg(test)]
    confirmed_expiry: Arc<AtomicI64>,
}

struct TrackedChild {
    process: std::process::Child, exited: bool,
    stop_request_error: Option<String>,
    #[cfg(windows)]
    job: windows_tree::OwnedJob,
}

impl TrackedChild {
    fn terminate(&mut self) {
        #[cfg(windows)]
        if let Err(error) = self.job.terminate() {
            self.stop_request_error.get_or_insert_with(|| error.to_string());
        }
        if self.exited { return; }
        #[cfg(unix)]
        {
            // ECHILD (for example an unexpected external reaper) removes our
            // kernel proof of the leader's identity. Never signal that PID.
            if let Err(error) = child_exit_is_observable(self.process.id()) {
                self.stop_request_error.get_or_insert_with(|| format!("Shell Child ownership cannot be verified before stop: {error}"));
                return;
            }
            if let Ok(pid) = i32::try_from(self.process.id()) {
                if unsafe { libc::kill(-pid, libc::SIGKILL) } < 0 {
                    let error = std::io::Error::last_os_error();
                    if error.raw_os_error() != Some(libc::ESRCH) {
                        self.stop_request_error.get_or_insert_with(|| error.to_string());
                    }
                }
            }
        }
        let _ = self.process.kill();
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag="state", rename_all="snake_case")]
pub(super) enum TreeStopFact {
    NotRequested,
    Requested,
    Confirmed { untransferred_processes: Option<usize>, prior_observation_error: Option<String> },
    Unknown { reason: String },
}

pub(super) fn unowned_shell_tree_stop_fact(pid: u32) -> TreeStopFact {
    TreeStopFact::Unknown { reason: format!("PID {pid} is only a persisted observation; no exact owned Child/Job or process-creation identity proves this tree") }
}

pub(crate) struct ShellChild {
    child: Arc<Mutex<Option<TrackedChild>>>, pid: u32,
    residual_processes_stopped: Option<usize>,
    stop_fact: TreeStopFact,
}

impl ShellChild {
    pub(crate) fn id(&self) -> u32 { self.pid }

    /// Transfer only protocol pipes; the supervisor retains the exact Child
    /// and process-tree ownership used for cancellation and reaping.
    pub(crate) fn take_protocol_pipes(&mut self) -> std::io::Result<(std::process::ChildStdin, std::process::ChildStdout)> {
        let mut guard = self.child.lock().map_err(|_| std::io::Error::other("process ownership is poisoned"))?;
        let tracked = guard.as_mut().ok_or_else(|| std::io::Error::other("process ownership is missing"))?;
        if tracked.process.stdin.is_none() || tracked.process.stdout.is_none() {
            return Err(std::io::Error::other("protocol requires piped stdin and stdout"));
        }
        Ok((tracked.process.stdin.take().expect("checked stdin"), tracked.process.stdout.take().expect("checked stdout")))
    }

    /// Uses the same exact owned OS object as supervised Shell execution,
    /// without manufacturing a durable lease for a synchronous invocation.
    pub(crate) fn spawn_owned(command: &mut std::process::Command) -> std::io::Result<Self> {
        #[cfg(unix)]
        let tracked = {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
            TrackedChild { process: command.spawn()?, exited: false, stop_request_error: None }
        };
        #[cfg(windows)]
        let (tracked, thread) = {
            let (process, job, thread) = windows_tree::OwnedJob::prepare(command)?;
            (TrackedChild { process, exited: false, stop_request_error: None, job }, thread)
        };
        let pid = tracked.process.id();
        let child = Arc::new(Mutex::new(Some(tracked)));
        let owned = Self {
            child, pid, residual_processes_stopped: None, stop_fact: TreeStopFact::NotRequested,
        };
        #[cfg(windows)]
        let mut owned = owned;
        #[cfg(windows)]
        {
            let resumed = {
                let guard = owned.child.lock().map_err(|_| std::io::Error::other("Shell child ownership is poisoned"))?;
                let tracked = guard.as_ref().ok_or_else(|| std::io::Error::other("Shell child ownership is missing"))?;
                windows_tree::OwnedJob::resume(&thread, &tracked.process)
            };
            if let Err(error) = resumed {
                owned.terminate();
                let _ = owned.wait();
                return Err(error);
            }
        }
        Ok(owned)
    }

    pub(crate) fn stop_and_confirm(
        &mut self,
        timeout: Duration,
    ) -> Result<nola_domain::execution_evidence::LocalExecutionStopScope, String> {
        self.terminate();
        match self.confirm_and_reap(timeout) {
            Ok(_) => {
                self.stop_fact = TreeStopFact::Confirmed {
                    untransferred_processes: None, prior_observation_error: None,
                };
                #[cfg(unix)]
                return Ok(nola_domain::execution_evidence::LocalExecutionStopScope::OwnedProcessGroup { leader_pid: self.pid });
                #[cfg(windows)]
                return Ok(nola_domain::execution_evidence::LocalExecutionStopScope::OwnedJob { leader_pid: self.pid });
            }
            Err(error) => {
                let reason = format!("{error}; stop request error: {:?}", self.stop_request_error());
                self.stop_fact = TreeStopFact::Unknown { reason: reason.clone() };
                Err(reason)
            }
        }
    }

    pub(crate) fn confirmed_local_stop_scope(
        &self,
    ) -> Option<nola_domain::execution_evidence::LocalExecutionStopScope> {
        if !matches!(self.stop_fact, TreeStopFact::Confirmed { .. }) {
            return None;
        }
        #[cfg(unix)]
        return Some(nola_domain::execution_evidence::LocalExecutionStopScope::OwnedProcessGroup { leader_pid: self.pid });
        #[cfg(windows)]
        return Some(nola_domain::execution_evidence::LocalExecutionStopScope::OwnedJob { leader_pid: self.pid });
    }

    pub(crate) fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        #[cfg(unix)]
        {
            // Observe the exact Child without reaping its group leader. The
            // unreaped PID reserves group identity during bounded inspection
            // and cleanup, even if a background descendant outlived bash.
            {
                let mut guard = self.child.lock().map_err(|_| std::io::Error::other("Shell child ownership is poisoned"))?;
                let child = guard.as_mut().ok_or_else(|| std::io::Error::other("Shell child ownership is missing"))?;
                if !child.exited && !child_exit_is_observable(self.pid)? { return Ok(None); }
                if child.exited { return child.process.try_wait(); }
            }
            let observation = self.observe_live_tree(Duration::from_millis(500));
            let remaining = observation.as_ref().ok().copied();
            let prior_observation_error = observation.err();
            self.terminate();
            let status = self.wait()?;
            self.residual_processes_stopped = remaining;
            self.stop_fact = TreeStopFact::Confirmed { untransferred_processes: remaining, prior_observation_error };
            return Ok(Some(status));
        }
        #[cfg(windows)]
        {
            let status = {
                let mut guard = self.child.lock().map_err(|_| std::io::Error::other("Shell child ownership is poisoned"))?;
                let child = guard.as_mut().ok_or_else(|| std::io::Error::other("Shell child ownership is missing"))?;
                let status = child.process.try_wait()?;
                if status.is_some() { child.exited = true; }
                status
            };
            if status.is_some() {
                let observation = self.observe_live_tree(Duration::from_millis(500));
                let remaining = observation.as_ref().ok().copied();
                let prior_observation_error = observation.err();
                self.terminate();
                self.wait()?;
                self.residual_processes_stopped = remaining;
                self.stop_fact = TreeStopFact::Confirmed { untransferred_processes: remaining, prior_observation_error };
            }
            Ok(status)
        }
    }

    pub(crate) fn residual_processes_stopped(&self) -> Option<usize> { self.residual_processes_stopped }
    pub(super) fn stop_fact(&self) -> &TreeStopFact { &self.stop_fact }

    pub(super) fn stop_request_error(&self) -> Option<String> {
        self.child.lock().ok().and_then(|child| child.as_ref().and_then(|child| child.stop_request_error.clone()))
    }

    fn observe_live_tree(&self, timeout: Duration) -> Result<usize, String> {
        #[cfg(unix)]
        { live_owned_group_members(self.pid, timeout) }
        #[cfg(windows)]
        {
            if timeout.is_zero() { return Err("Shell Job observation deadline expired".into()); }
            let job = {
                let guard = self.child.lock().map_err(|_| "Shell child ownership is poisoned")?;
                guard.as_ref().ok_or("Shell child ownership is missing")?.job.try_clone().map_err(|error|error.to_string())?
            };
            let deadline = Instant::now() + timeout;
            loop {
                let active = job.active_processes().map_err(|error| error.to_string())?;
                // The Job accounting transition can lag the root's signalled
                // process handle. Confirm zero; never infer it from root exit.
                if active == 0 || Instant::now() >= deadline { return Ok(active); }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }

    pub(super) fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        let confirmed = self.confirm_and_reap(Duration::from_secs(1));
        match confirmed {
            Ok(status) => {
                self.stop_fact = TreeStopFact::Confirmed { untransferred_processes: None, prior_observation_error: None };
                Ok(status)
            }
            Err(error) => {
                self.stop_fact = TreeStopFact::Unknown { reason: error.to_string() };
                Err(error)
            }
        }
    }

    fn confirm_and_reap(&mut self, timeout: Duration) -> std::io::Result<std::process::ExitStatus> {
        #[cfg(unix)]
        {
            let mut guard = self.child.lock().map_err(|_| std::io::Error::other("Shell child ownership is poisoned"))?;
            let child = guard.as_mut().ok_or_else(|| std::io::Error::other("Shell child ownership is missing"))?;
            // On Unix this flag is only set AFTER group-empty confirmation.
            // A second wait reads Child's cached result, never a reused PGID.
            if child.exited {
                return child.process.try_wait()?.ok_or_else(|| std::io::Error::other("confirmed Shell root lost its cached exit status"));
            }
        }
        let deadline = Instant::now() + timeout;
        // Sampling and waiting never own the Child mutex. The watchdog may
        // signal the exact tree at any point, including blocked observations.
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() { return Err(std::io::Error::other("Shell tree stop confirmation deadline expired")); }
            let live = self.observe_live_tree(remaining.min(Duration::from_millis(500))).map_err(std::io::Error::other)?;
            if Instant::now() >= deadline { return Err(std::io::Error::other("Shell tree stop confirmation arrived after its deadline")); }
            if live == 0 { break; }
            if Instant::now() >= deadline {
                return Err(std::io::Error::other("Shell tree stop could not be confirmed before its deadline"));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        loop {
            let status = {
                let mut guard = self.child.lock().map_err(|_| std::io::Error::other("Shell child ownership is poisoned"))?;
                let child = guard.as_mut().ok_or_else(|| std::io::Error::other("Shell child ownership is missing"))?;
                let status = child.process.try_wait()?;
                if status.is_some() { child.exited = true; }
                status
            };
            if let Some(status) = status { return Ok(status); }
            if Instant::now() >= deadline { return Err(std::io::Error::other("stopped Shell root could not be reaped before its deadline")); }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    pub(super) fn terminate(&mut self) {
        self.stop_fact = TreeStopFact::Requested;
        if let Ok(mut guard) = self.child.lock() {
            if let Some(child) = guard.as_mut() { child.terminate(); }
        }
    }
}

impl Drop for ShellChild {
    fn drop(&mut self) {
        if !matches!(self.stop_fact, TreeStopFact::Confirmed { .. }) {
            // Best-effort cleanup retains the same owned object. It never
            // changes the already committed Unknown into a success receipt.
            self.terminate();
            let _ = self.wait();
        }
    }
}

impl ShellLeaseHeartbeat {
    pub(super) fn start(path: &Path, lease: RuntimeTaskLeaseHandle, duration_ms: i64) -> Result<Self, String> {
        let store = NolaStore::open(path)?;
        Self::start_with_renewal(duration_ms, move || renew_shell_lease(&store, &lease, duration_ms))
    }

    fn start_with_renewal(
        duration_ms: i64,
        mut renew: impl FnMut() -> Result<i64, String> + Send + 'static,
    ) -> Result<Self, String> {
        if duration_ms <= 0 { return Err("Shell lease has no positive duration".into()); }
        let expiry = renew()?;
        if expiry <= nola_runtime_driver::monotonic_epoch_millis() {
            return Err("Shell lease renewal did not establish a current deadline".into());
        }
        let expiry = Arc::new(AtomicI64::new(expiry));
        let renewed_expiry = Arc::clone(&expiry);
        #[cfg(test)]
        let confirmed_expiry = Arc::clone(&expiry);
        let (stop, receiver) = mpsc::channel();
        let lost = Arc::new(AtomicBool::new(false));
        let failure = Arc::clone(&lost);
        let child = Arc::new(Mutex::new(None::<TrackedChild>));
        let abort_child = Arc::clone(&child);
        let interval = Duration::from_millis((duration_ms / 3).clamp(1, 10_000) as u64);
        let thread = std::thread::Builder::new().name("nola-shell-lease-heartbeat".into())
            .spawn(move || loop {
                match receiver.recv_timeout(interval) {
                    Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
                if failure.load(Ordering::Acquire) { break; }
                let renewal = renew();
                if let Ok(expiry) = renewal {
                    renewed_expiry.store(expiry, Ordering::Release);
                } else {
                    failure.store(true, Ordering::Release);
                    if let Ok(mut child) = abort_child.lock() {
                        if let Some(child) = child.as_mut() { child.terminate(); }
                    }
                    break;
                }
            }).map_err(|error| format!("start Shell lease heartbeat failed: {error}"))?;
        // Store renewal can itself wait on IPC/SQLite while holding the lease
        // handle. A separate timer never takes that lock: only a confirmed
        // deadline extends physical authority, not an in-flight renewal.
        let (watchdog_stop, watchdog_receiver) = mpsc::channel();
        let expired = Arc::clone(&lost);
        let abort_child = Arc::clone(&child);
        let watchdog = match std::thread::Builder::new().name("nola-shell-lease-deadline".into())
            .spawn(move || loop {
                if expired.load(Ordering::Acquire) { break; }
                let remaining = expiry.load(Ordering::Acquire)
                    .saturating_sub(nola_runtime_driver::monotonic_epoch_millis());
                if remaining <= 0 {
                    expired.store(true, Ordering::Release);
                    if let Ok(mut child) = abort_child.lock() {
                        if let Some(child) = child.as_mut() { child.terminate(); }
                    }
                    break;
                }
                match watchdog_receiver.recv_timeout(Duration::from_millis(remaining.min(10) as u64)) {
                    Ok(()) => break,
                    // Losing the controller is not a stop confirmation. The
                    // watchdog retains its owned Child until the deadline.
                    Err(mpsc::RecvTimeoutError::Disconnected) => std::thread::sleep(Duration::from_millis(5)),
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
            }) {
            Ok(watchdog) => watchdog,
            Err(error) => {
                let _ = stop.send(());
                let _ = thread.join();
                return Err(format!("start Shell lease deadline watchdog failed: {error}"));
            }
        };
        Ok(Self { stop, thread: Some(thread), watchdog_stop, watchdog: Some(watchdog), lost, child,
            #[cfg(test)] confirmed_expiry })
    }

    pub(super) fn lost(&self) -> bool { self.lost.load(Ordering::Acquire) }

    fn track_owned(&self, tracked: TrackedChild) -> ShellChild {
        let pid = tracked.process.id();
        let mut child = self.child.lock().expect("new Shell child ownership is not poisoned");
        assert!(child.is_none(), "one lease heartbeat owns one physical Shell child");
        *child = Some(tracked);
        if self.lost() { child.as_mut().unwrap().terminate(); }
        ShellChild { child: Arc::clone(&self.child), pid, residual_processes_stopped: None, stop_fact: TreeStopFact::NotRequested }
    }

    #[cfg(unix)]
    pub(super) fn track(&self, process: std::process::Child) -> ShellChild {
        self.track_owned(TrackedChild { process, exited: false, stop_request_error: None })
    }

    pub(super) fn spawn(&self, command: &mut std::process::Command) -> std::io::Result<ShellChild> {
        if self.lost() { return Err(std::io::Error::other("Shell lease lost before physical spawn")); }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
            Ok(self.track(command.spawn()?))
        }
        #[cfg(windows)]
        {
            let (process, job, thread) = windows_tree::OwnedJob::prepare(command)?;
            let mut child = self.track_owned(TrackedChild { process, exited: false, stop_request_error: None, job });
            let resumed = {
                let guard = self.child.lock().map_err(|_| std::io::Error::other("Shell child ownership is poisoned"))?;
                let tracked = guard.as_ref().ok_or_else(|| std::io::Error::other("Shell child ownership is missing"))?;
                if self.lost() { Err(std::io::Error::other("Shell lease lost before initial thread resume")) }
                else { windows_tree::OwnedJob::resume(&thread, &tracked.process) }
            };
            if let Err(error) = resumed {
                child.terminate();
                let _ = child.wait();
                return Err(error);
            }
            Ok(child)
        }
    }

    #[cfg(unix)]
    pub(super) fn spawn_terminal(&self, command: &mut std::process::Command,
        slave: nola_runtime_supervisor::terminal::TerminalSlave) -> std::io::Result<ShellChild> {
        if self.lost() { return Err(std::io::Error::other("Shell lease lost before terminal spawn")); }
        // The PTY child creates a session; process_group(0) here would make
        // setsid fail. Session creation already establishes the owned group.
        slave.attach(command)?;
        Ok(self.track(command.spawn()?))
    }

    pub(super) fn stop(&mut self) {
        let _ = self.stop.send(());
        let tracked = self.child.lock().map(|child| child.as_ref().map(|child| (child.process.id(), child.exited)));
        let confirmed = match tracked {
            Ok(None) => true, // No business process was ever attached.
            #[cfg(unix)]
            Ok(Some((_, true))) => true, // This flag requires prior group-empty proof.
            Ok(Some((pid, _))) => {
                let mut child = ShellChild { child: Arc::clone(&self.child), pid,
                    residual_processes_stopped: None, stop_fact: TreeStopFact::NotRequested };
                child.terminate();
                child.wait().is_ok()
            }
            Err(_) => false, // Poisoned ownership is not proof of no process.
        };
        if confirmed { let _ = self.watchdog_stop.send(()); }
        if let Some(watchdog) = self.watchdog.take() {
            if confirmed { let _ = watchdog.join(); }
            // Otherwise the watchdog still owns the tree and its deadline.
            // It exits after requesting stop at that deadline, not by a fake
            // acknowledgement from this controller's shutdown.
        }
        if let Some(thread) = self.thread.take() {
            let deadline = Instant::now() + Duration::from_millis(100);
            while !thread.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            if thread.is_finished() { let _ = thread.join(); }
            else { eprintln!("Shell lease renewal drain timed out; the old renewal remains fenced and no further renewal will be scheduled"); }
        }
    }
}

#[cfg(unix)]
fn child_exit_is_observable(pid: u32) -> std::io::Result<bool> {
    let mut info = unsafe { std::mem::zeroed::<libc::siginfo_t>() };
    let result = unsafe { libc::waitid(libc::P_PID, pid as libc::id_t, &mut info,
        libc::WEXITED | libc::WNOHANG | libc::WNOWAIT) };
    if result < 0 { return Err(std::io::Error::last_os_error()); }
    Ok(info.si_signo == libc::SIGCHLD)
}

/// A raw Child has no Windows Job ownership. Unix callers must have spawned
/// their own execution group, and must not have reaped its original leader.
pub(crate) fn stop_borrowed_process_group(
    child: &mut std::process::Child,
    timeout: Duration,
) -> Result<nola_domain::execution_evidence::LocalExecutionStopScope, String> {
    let deadline = Instant::now() + timeout;
    #[cfg(unix)]
    {
        let pid = child.id();
        child_exit_is_observable(pid).map_err(|error| format!("original Child identity is unavailable: {error}"))?;
        let group = unsafe { libc::getpgid(pid as i32) };
        if group != pid as i32 {
            // The original Child remains owned, but not an owned group.
            let _ = child.kill();
            return Err("original Child is not its owned execution group leader".into());
        }
        if unsafe { libc::kill(-group, libc::SIGKILL) } < 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(format!("request owned execution group stop failed: {error}"));
            }
        }
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() { return Err("owned execution group stop confirmation deadline expired".into()); }
            let live = live_owned_group_members(pid, remaining.min(Duration::from_millis(500)))?;
            if Instant::now() >= deadline { return Err("owned execution group observation arrived after its deadline".into()); }
            if live == 0 { break; }
            std::thread::sleep(Duration::from_millis(5));
        }
        // No signal follows this reap. A reused numeric PGID has no authority.
        loop {
            if child.try_wait().map_err(|error| error.to_string())?.is_some() {
                return Ok(nola_domain::execution_evidence::LocalExecutionStopScope::OwnedProcessGroup { leader_pid: pid });
            }
            if Instant::now() >= deadline { return Err("stopped execution group leader could not be reaped before its deadline".into()); }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
        while Instant::now() < deadline {
            if child.try_wait().map_err(|error| error.to_string())?.is_some() { break; }
            std::thread::sleep(Duration::from_millis(5));
        }
        Err("a raw Child has no owned Job stop proof".into())
    }
}

#[cfg(unix)]
fn live_owned_group_members(group: u32, timeout: Duration) -> Result<usize, String> {
    let mut command = std::process::Command::new("ps");
    command.args(["-axo", "pid=,pgid=,stat="]);
    let (_, output) = bounded_process_output(command, timeout, 8 * 1024 * 1024)?;
    if !output.status.success() { return Err("Shell group completion observation failed".into()); }
    let text = std::str::from_utf8(&output.stdout).map_err(|_| "Shell group observation is not UTF-8")?;
    let mut found_leader = false;
    let mut remaining = 0;
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.len() != 3 { return Err("Shell group observation has invalid fields".into()); }
        let pid = fields[0].parse::<u32>().map_err(|_| "Shell group observation has invalid PID")?;
        let pgid = fields[1].parse::<u32>().map_err(|_| "Shell group observation has invalid group")?;
        if pgid != group { continue; }
        if pid == group { found_leader = true; }
        if !fields[2].starts_with('Z') { remaining += 1; }
    }
    if !found_leader { return Err("Shell group leader identity is absent before reaping".into()); }
    Ok(remaining)
}

impl Drop for ShellLeaseHeartbeat {
    fn drop(&mut self) { self.stop(); }
}

fn renew_shell_lease(store: &NolaStore, lease: &RuntimeTaskLeaseHandle, duration_ms: i64) -> Result<i64, String> {
    let now = nola_runtime_driver::monotonic_epoch_millis();
    let current = lease.snapshot()?;
    if current.lease_expires_at_ms <= now { return Err("Shell lease expired before renewal".into()); }
    let expiry = now.checked_add(duration_ms).ok_or("Shell lease expiry overflow")?;
    // A progress ACK can retain a longer startup interval. Never shorten it,
    // and never manufacture a new attempt merely to keep this worker alive.
    if expiry <= current.lease_expires_at_ms { return Ok(current.lease_expires_at_ms); }
    store.renew_runtime_task_lease(lease, now, expiry).map(|attempt| attempt.lease_expires_at_ms)
}

#[cfg(unix)]
pub(crate) fn bounded_process_output(
    mut command: std::process::Command,
    timeout: Duration,
    maximum_bytes: usize,
) -> Result<(u32, std::process::Output), String> {
    use std::io::Read;
    use std::os::fd::AsRawFd;
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;

    if timeout.is_zero() || maximum_bytes == 0 {
        return Err("process sampler requires a positive deadline and output limit".into());
    }
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).process_group(0);
    let mut child = command.spawn().map_err(|error| format!("start process sampler failed: {error}"))?;
    let sampler_pid = child.id();
    let mut stdout = child.stdout.take().ok_or("process sampler stdout is missing")?;
    let mut stderr = child.stderr.take().ok_or("process sampler stderr is missing")?;
    let nonblocking = |fd| -> Result<(), String> {
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(format!("configure process sampler pipe failed: {}", std::io::Error::last_os_error()));
        }
        Ok(())
    };
    let started = Instant::now();
    let result = (|| {
        nonblocking(stdout.as_raw_fd())?;
        nonblocking(stderr.as_raw_fd())?;
        let mut out = Vec::new();
        let mut err = Vec::new();
        let mut out_done = false;
        let mut err_done = false;
        let mut status = None;
        let mut total_bytes = 0_usize;
        loop {
            for (reader, bytes, done) in [
                (&mut stdout as &mut dyn Read, &mut out, &mut out_done),
                (&mut stderr as &mut dyn Read, &mut err, &mut err_done),
            ] {
                if *done { continue; }
                loop {
                    let mut buffer = [0_u8; 16 * 1024];
                    match reader.read(&mut buffer) {
                        Ok(0) => { *done = true; break; }
                        Ok(count) => {
                            total_bytes = total_bytes.checked_add(count).ok_or("process sampler byte count overflow")?;
                            if total_bytes > maximum_bytes {
                                return Err("process sampler exceeded its output byte limit".into());
                            }
                            bytes.extend_from_slice(&buffer[..count]);
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(error) => return Err(format!("read process sampler output failed: {error}")),
                    }
                    if started.elapsed() >= timeout { return Err("process sampler timed out".into()); }
                }
            }
            // Keep the root unreaped until both owned pipes close. Its zombie
            // then reserves the group leader PID while deadline/error cleanup
            // may still signal that group; a reaped PID could be reused.
            if status.is_none() && out_done && err_done {
                status = child.try_wait().map_err(|error| format!("observe process sampler failed: {error}"))?;
            }
            if let Some(status) = status.filter(|_| out_done && err_done) {
                return Ok((sampler_pid, std::process::Output { status, stdout: out, stderr: err }));
            }
            if started.elapsed() >= timeout { return Err("process sampler timed out".into()); }
            std::thread::sleep(Duration::from_millis(5));
        }
    })();
    if result.is_err() {
        // The root has not been reaped on an error path. Only the process
        // group created above is ours, even when descendants kept pipes open.
        if let Ok(pid) = i32::try_from(child.id()) { unsafe { libc::kill(-pid, libc::SIGKILL); } }
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn task_clock_and_delivery_clock_share_one_monotonic_domain() {
        let before = nola_runtime_driver::monotonic_epoch_millis();
        let task = super::super::unix_epoch_millis();
        let after = nola_runtime_driver::monotonic_epoch_millis();
        assert!((before..=after).contains(&task));
    }

    #[test]
    fn sampler_drains_both_pipes_and_keeps_exact_bytes() {
        let mut command = std::process::Command::new("sh");
        command.args(["-c", "printf out; printf err >&2"]);
        let (_, output) = bounded_process_output(command, Duration::from_secs(2), 64).unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"out");
        assert_eq!(output.stderr, b"err");
    }

    #[test]
    fn standalone_owned_group_stop_is_post_confirmed_and_repeat_uses_cached_identity() {
        use nola_domain::execution_evidence::LocalExecutionStopScope;
        let mut command = std::process::Command::new("sh");
        command.args(["-c", "sleep 30"]);
        let mut child = ShellChild::spawn_owned(&mut command).unwrap();
        let pid = child.id();
        assert!(matches!(child.stop_fact(), TreeStopFact::NotRequested));
        assert_eq!(child.stop_and_confirm(Duration::from_secs(2)).unwrap(),
            LocalExecutionStopScope::OwnedProcessGroup { leader_pid: pid });
        assert!(matches!(child.stop_fact(), TreeStopFact::Confirmed { .. }));
        assert_eq!(child_exit_is_observable(pid).unwrap_err().raw_os_error(), Some(libc::ECHILD));
        // A second confirmation must use Child's cached status. Observing or
        // signalling the now-unreserved numeric PGID would be invalid.
        assert_eq!(child.stop_and_confirm(Duration::from_secs(2)).unwrap(),
            LocalExecutionStopScope::OwnedProcessGroup { leader_pid: pid });
    }

    #[test]
    fn standalone_owned_group_root_exit_stops_residual_processes_before_reap() {
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("forbidden-late-write");
        let mut command = std::process::Command::new("sh");
        command.args(["-c", "(sleep 1; printf late > \"$1\") & exit 0", "fixture"]).arg(&marker);
        let mut child = ShellChild::spawn_owned(&mut command).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(child.residual_processes_stopped().is_some_and(|count| count > 0));
        assert!(matches!(child.stop_fact(), TreeStopFact::Confirmed { .. }));
        std::thread::sleep(Duration::from_millis(1_100));
        assert!(!marker.exists());
    }

    #[test]
    fn stop_request_without_observation_deadline_never_attests_local_stop() {
        let mut command = std::process::Command::new("sh");
        command.args(["-c", "sleep 30"]);
        let mut child = ShellChild::spawn_owned(&mut command).unwrap();
        assert!(child.stop_and_confirm(Duration::ZERO).is_err());
        assert!(matches!(child.stop_fact(), TreeStopFact::Unknown { .. }));
        // Cleanup can later obtain its own real proof; the failed observation
        // above never emitted a local stop or business success receipt.
        child.stop_and_confirm(Duration::from_secs(2)).unwrap();
    }

    #[test]
    fn reaped_raw_child_cannot_authorize_a_numeric_group_stop() {
        use std::os::unix::process::CommandExt;
        let mut command = std::process::Command::new("sh");
        command.args(["-c", "exit 0"]).process_group(0);
        let mut child = command.spawn().unwrap();
        child.wait().unwrap();
        assert!(stop_borrowed_process_group(&mut child, Duration::from_secs(1))
            .unwrap_err().contains("original Child identity is unavailable"));
    }

    #[test]
    fn sampler_deadline_reaps_a_child_with_open_pipes() {
        let mut command = std::process::Command::new("sh");
        command.args(["-c", "sleep 10"]);
        let started = Instant::now();
        assert!(bounded_process_output(command, Duration::from_millis(100), 64).unwrap_err().contains("timed out"));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn sampler_output_limit_does_not_deadlock_on_a_full_pipe() {
        let mut command = std::process::Command::new("sh");
        command.args(["-c", "while :; do printf 0123456789; printf abcdefghij >&2; done"]);
        let started = Instant::now();
        assert!(bounded_process_output(command, Duration::from_secs(2), 128).unwrap_err().contains("byte limit"));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn blocked_sampler_cannot_block_the_independent_lease_heartbeat() {
        let (renewed, progress) = mpsc::channel();
        let mut heartbeat = ShellLeaseHeartbeat::start_with_renewal(90, move || {
            let _ = renewed.send(());
            Ok(nola_runtime_driver::monotonic_epoch_millis() + 30_000)
        }).unwrap();
        let root = tempfile::tempdir().unwrap();
        let ready = root.path().join("sampler-ready");
        let marker = ready.clone();
        let (finished, result) = mpsc::channel();
        let sampler = std::thread::spawn(move || {
            let mut command = std::process::Command::new("sh");
            command.args(["-c", "printf ready > \"$1\"; sleep 10", "sampler"]).arg(marker);
            finished.send(bounded_process_output(command, Duration::from_secs(3), 64)).unwrap();
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while !ready.exists() {
            assert!(Instant::now() < deadline, "sampler did not become ready");
            std::thread::sleep(Duration::from_millis(5));
        }
        while progress.try_recv().is_ok() {}
        // These renewals occur after the sampler's real child is ready.
        for _ in 0..2 { progress.recv_timeout(Duration::from_secs(2)).unwrap(); }
        assert!(matches!(result.try_recv(), Err(mpsc::TryRecvError::Empty)));
        assert!(!heartbeat.lost());
        assert!(result.recv_timeout(Duration::from_secs(5)).unwrap().is_err());
        sampler.join().unwrap();
        heartbeat.stop();
    }

    #[test]
    fn lost_lease_stops_only_its_owned_child_without_waiting_for_the_monitor() {
        use std::os::unix::process::CommandExt;
        let renewed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = Arc::clone(&renewed);
        let mut heartbeat = ShellLeaseHeartbeat::start_with_renewal(90, move || {
            if count.fetch_add(1, Ordering::Relaxed) >= 2 { Err("exact lease revoked".into()) }
            else { Ok(nola_runtime_driver::monotonic_epoch_millis() + 90) }
        }).unwrap();
        let mut command = std::process::Command::new("sh");
        command.args(["-c", "sleep 10"]).process_group(0);
        let mut child = heartbeat.track(command.spawn().unwrap());
        let started = Instant::now();
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(!status.success());
                break;
            }
            assert!(started.elapsed() < Duration::from_secs(2));
            std::thread::sleep(Duration::from_millis(10));
        }
        heartbeat.stop();
        assert!(heartbeat.lost());
    }

    #[test]
    fn blocked_renewal_cannot_extend_the_confirmed_physical_deadline() {
        use std::os::unix::process::CommandExt;
        let mut initial = true;
        let (release, blocked) = mpsc::channel();
        let (entered, observed) = mpsc::channel();
        let mut heartbeat = ShellLeaseHeartbeat::start_with_renewal(120, move || {
            if initial { initial = false; }
            else { entered.send(()).unwrap(); blocked.recv().unwrap(); }
            Ok(nola_runtime_driver::monotonic_epoch_millis() + 30_000)
        }).unwrap();
        let mut command = std::process::Command::new("sh");
        command.args(["-c", "sleep 10"]).process_group(0);
        let mut child = heartbeat.track(command.spawn().unwrap());
        observed.recv_timeout(Duration::from_secs(5)).unwrap();
        heartbeat.confirmed_expiry.store(nola_runtime_driver::monotonic_epoch_millis() - 1, Ordering::Release);
        let started = Instant::now();
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(!status.success()); break;
            }
            assert!(started.elapsed() < Duration::from_secs(2));
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(heartbeat.lost(), "a late renewal cannot revive the stopped child");
        release.send(()).unwrap();
        heartbeat.stop();
        assert!(heartbeat.lost());
    }

    #[test]
    fn successful_root_exit_stops_untransferred_background_descendants_before_reaping() {
        use std::os::unix::process::CommandExt;
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("forbidden-late-write");
        let mut unrelated = std::process::Command::new("sh");
        unrelated.args(["-c", "sleep 10"]).process_group(0);
        let mut unrelated = unrelated.spawn().unwrap();
        let mut heartbeat = ShellLeaseHeartbeat::start_with_renewal(30_000, ||
            Ok(nola_runtime_driver::monotonic_epoch_millis() + 30_000)).unwrap();
        let mut command = std::process::Command::new("sh");
        command.args(["-c", "(sleep 1; printf late > \"$1\") & exit 0", "fixture"])
            .arg(&marker);
        let mut child = heartbeat.spawn(&mut command).unwrap();
        let started = Instant::now();
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success(), "preserve the root's actual exit status"); break;
            }
            assert!(started.elapsed() < Duration::from_secs(3));
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(child.residual_processes_stopped().is_some_and(|count| count > 0),
            "root exit is not a successful whole-group receipt");
        std::thread::sleep(Duration::from_millis(1_100));
        assert!(!marker.exists(), "no unowned late write may follow the root's exit");
        // Calling the completed handle again cannot signal a recycled root
        // PID, and it must never terminate an unrelated process group.
        child.terminate();
        assert!(unrelated.try_wait().unwrap().is_none());
        unsafe { libc::kill(-(unrelated.id() as i32), libc::SIGKILL); }
        unrelated.kill().ok(); unrelated.wait().unwrap();
        heartbeat.stop();
    }

    #[test]
    fn absent_output_is_unknown_not_a_zero_byte_budget_observation() {
        let root = tempfile::tempdir().unwrap();
        let out = root.path().join("out"); let err = root.path().join("err");
        std::fs::write(&out, b"abc").unwrap();
        assert!(super::super::shell_output_bytes(out.to_str().unwrap(), err.to_str().unwrap()).is_err());
        std::fs::write(&err, b"de").unwrap();
        assert_eq!(super::super::shell_output_bytes(out.to_str().unwrap(), err.to_str().unwrap()).unwrap(), 5);
    }

    #[test]
    fn live_tree_without_stop_confirmation_remains_unknown_and_owned() {
        let mut heartbeat = ShellLeaseHeartbeat::start_with_renewal(30_000, ||
            Ok(nola_runtime_driver::monotonic_epoch_millis() + 30_000)).unwrap();
        let mut command = std::process::Command::new("sh");
        command.args(["-c", "sleep 10"]);
        let mut child = heartbeat.spawn(&mut command).unwrap();
        assert!(matches!(child.stop_fact(), TreeStopFact::NotRequested));
        let started = Instant::now();
        assert!(child.wait().is_err(), "a live group cannot be confirmed stopped merely by waiting for a bounded observation");
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(matches!(child.stop_fact(), TreeStopFact::Unknown { .. }));
        assert!(!child_exit_is_observable(child.id()).unwrap(), "unknown observation did not reap or replace the original root");
        child.terminate();
        assert!(!child.wait().unwrap().success());
        assert!(matches!(child.stop_fact(), TreeStopFact::Confirmed { .. }));
        heartbeat.stop();
    }

    #[test]
    fn a_live_pid_without_ownership_never_authorizes_a_cold_stop_signal() {
        use std::os::unix::process::CommandExt;
        let mut heartbeat = ShellLeaseHeartbeat::start_with_renewal(30_000, ||
            Ok(nola_runtime_driver::monotonic_epoch_millis() + 30_000)).unwrap();
        let mut command = std::process::Command::new("sh");
        command.args(["-c", "sleep 10"]).process_group(0);
        let mut child = heartbeat.spawn(&mut command).unwrap();
        let fact = unowned_shell_tree_stop_fact(child.id());
        assert!(matches!(fact, TreeStopFact::Unknown { .. }));
        assert!(child.try_wait().unwrap().is_none(), "a durable PID observation must not kill a live process");
        assert!(matches!(child.stop_fact(), TreeStopFact::NotRequested));
        child.terminate();
        child.wait().unwrap();
        heartbeat.stop();
    }

    #[test]
    fn stopping_a_heartbeat_does_not_wait_forever_for_a_blocked_renewal() {
        let (entered, observed) = mpsc::channel();
        let (release, blocked) = mpsc::channel();
        let mut initial = true;
        let mut heartbeat = ShellLeaseHeartbeat::start_with_renewal(120, move || {
            if initial { initial = false; }
            else { entered.send(()).unwrap(); blocked.recv().unwrap(); }
            Ok(nola_runtime_driver::monotonic_epoch_millis() + 30_000)
        }).unwrap();
        observed.recv_timeout(Duration::from_secs(5)).unwrap();
        let started = Instant::now();
        heartbeat.stop();
        assert!(started.elapsed() < Duration::from_secs(1), "renewal drain is bounded independently of its blocked operation");
        release.send(()).unwrap();
    }
}

#[cfg(all(test, windows))]
mod owned_job_tests {
    use super::*;
    use nola_domain::execution_evidence::LocalExecutionStopScope;

    #[test]
    fn synchronous_shell_job_is_bound_before_running_and_confirmed_empty_before_stop_receipt() {
        let mut command = std::process::Command::new("cmd.exe");
        command.args(["/C", "ping -n 31 127.0.0.1 >nul"]);
        let mut child = ShellChild::spawn_owned(&mut command).unwrap();
        let pid = child.id();
        assert!(matches!(child.stop_fact(), TreeStopFact::NotRequested));
        assert_eq!(child.stop_and_confirm(Duration::from_secs(2)).unwrap(),
            LocalExecutionStopScope::OwnedJob { leader_pid: pid });
        assert!(matches!(child.stop_fact(), TreeStopFact::Confirmed { .. }));
        assert_eq!(child.stop_and_confirm(Duration::from_secs(2)).unwrap(),
            LocalExecutionStopScope::OwnedJob { leader_pid: pid });
    }
}
