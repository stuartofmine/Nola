// SPDX-License-Identifier: Apache-2.0
//! Windows physical ownership established before the first business instruction.
//!
//! https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects
//! https://learn.microsoft.com/en-us/windows/win32/procthread/process-creation-flags
//! No breakaway flag, PID-based termination, or undocumented resume API.

use std::os::windows::{io::{AsRawHandle, FromRawHandle, OwnedHandle}, process::CommandExt};
use std::process::{Child, Command};
use windows_sys::Win32::{
    Foundation::{CloseHandle, GetLastError, ERROR_NO_MORE_FILES, INVALID_HANDLE_VALUE},
    System::{
        Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Thread32First, Thread32Next, THREADENTRY32, TH32CS_SNAPTHREAD},
        JobObjects::{AssignProcessToJobObject, CreateJobObjectW, QueryInformationJobObject,
            SetInformationJobObject, TerminateJobObject, JobObjectBasicAccountingInformation,
            JobObjectExtendedLimitInformation, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE},
        Threading::{GetProcessIdOfThread, OpenThread, ResumeThread, CREATE_SUSPENDED,
            THREAD_QUERY_LIMITED_INFORMATION, THREAD_SUSPEND_RESUME},
    },
};

pub(super) struct OwnedJob { handle: OwnedHandle }

impl OwnedJob {
    pub(super) fn prepare(command: &mut Command) -> std::io::Result<(Child, Self, OwnedHandle)> {
        let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if raw.is_null() { return Err(std::io::Error::last_os_error()); }
        let job = Self { handle: unsafe { OwnedHandle::from_raw_handle(raw) } };
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if unsafe { SetInformationJobObject(job.handle.as_raw_handle(), JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of_val(&limits) as u32) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        command.creation_flags(CREATE_SUSPENDED);
        let mut child = command.spawn()?;
        let admission = (|| {
            if unsafe { AssignProcessToJobObject(job.handle.as_raw_handle(), child.as_raw_handle()) } == 0 {
                return Err(std::io::Error::last_os_error());
            }
            suspended_initial_thread(child.id())
        })();
        match admission {
            Ok(thread) => Ok((child, job, thread)),
            Err(error) => {
                let _ = job.terminate();
                let _ = child.kill();
                let _ = child.wait();
                Err(error)
            }
        }
    }

    pub(super) fn resume(thread: &OwnedHandle, child: &Child) -> std::io::Result<()> {
        if unsafe { GetProcessIdOfThread(thread.as_raw_handle()) } != child.id() {
            return Err(std::io::Error::other("suspended Shell thread owner changed"));
        }
        let previous = unsafe { ResumeThread(thread.as_raw_handle()) };
        if previous != 1 {
            return Err(std::io::Error::other(format!("unexpected initial Shell thread suspend count: {previous}")));
        }
        Ok(())
    }

    pub(super) fn active_processes(&self) -> std::io::Result<usize> {
        let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        if unsafe { QueryInformationJobObject(self.handle.as_raw_handle(), JobObjectBasicAccountingInformation,
            (&mut accounting as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
            std::mem::size_of_val(&accounting) as u32, std::ptr::null_mut()) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(accounting.ActiveProcesses as usize)
    }

    pub(super) fn try_clone(&self) -> std::io::Result<Self> {
        self.handle.try_clone().map(|handle| Self { handle })
    }

    pub(super) fn terminate(&self) -> std::io::Result<()> {
        if unsafe { TerminateJobObject(self.handle.as_raw_handle(), 1) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
}

fn suspended_initial_thread(process: u32) -> std::io::Result<OwnedHandle> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == INVALID_HANDLE_VALUE { return Err(std::io::Error::last_os_error()); }
    let snapshot = unsafe { OwnedHandle::from_raw_handle(snapshot) };
    let mut entry = THREADENTRY32 { dwSize: std::mem::size_of::<THREADENTRY32>() as u32, ..Default::default() };
    let mut found = None;
    let mut next = unsafe { Thread32First(snapshot.as_raw_handle(), &mut entry) };
    while next != 0 {
        if entry.th32OwnerProcessID == process {
            if found.replace(entry.th32ThreadID).is_some() {
                return Err(std::io::Error::other("suspended Shell has no unique initial thread"));
            }
        }
        next = unsafe { Thread32Next(snapshot.as_raw_handle(), &mut entry) };
    }
    if unsafe { GetLastError() } != ERROR_NO_MORE_FILES { return Err(std::io::Error::last_os_error()); }
    let id = found.ok_or_else(|| std::io::Error::other("suspended Shell has no initial thread"))?;
    let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME | THREAD_QUERY_LIMITED_INFORMATION, 0, id) };
    if thread.is_null() { return Err(std::io::Error::last_os_error()); }
    if unsafe { GetProcessIdOfThread(thread) } != process {
        unsafe { CloseHandle(thread); }
        return Err(std::io::Error::other("suspended Shell thread identity differs from its process"));
    }
    Ok(unsafe { OwnedHandle::from_raw_handle(thread) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_assignment_precedes_every_business_instruction() {
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("not-before-resume.txt");
        let mut command = Command::new("powershell.exe");
        command.args(["-NoProfile", "-NonInteractive", "-Command", &format!("[IO.File]::WriteAllText('{}','actual')",
            marker.to_string_lossy().replace('\'', "''"))]);
        let (mut child, job, thread) = OwnedJob::prepare(&mut command).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(!marker.exists(), "suspended admission must not run the command");
        assert_eq!(job.active_processes().unwrap(), 1);
        OwnedJob::resume(&thread, &child).unwrap();
        assert!(child.wait().unwrap().success());
        assert_eq!(std::fs::read_to_string(marker).unwrap().trim(), "actual");
    }

    #[test]
    fn owned_job_runs_the_native_bash_interpreter_before_reaping_it() {
        for mode in ["-c", "-lc"] {
            let mut command = Command::new(crate::platform_paths::bash_program());
            command.args([mode, "printf actual-owned-bash"]).stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
            let (child, job, thread) = OwnedJob::prepare(&mut command).unwrap();
            OwnedJob::resume(&thread, &child).unwrap();
            let output = child.wait_with_output().unwrap();
            assert!(output.status.success(), "{mode}: {:?}: {}", output.status, String::from_utf8_lossy(&output.stderr));
            assert_eq!(output.stdout, b"actual-owned-bash");
            drop(job);
        }
    }

    #[test]
    fn closing_the_owned_job_stops_an_unreaped_live_child() {
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("started.txt");
        let mut command = Command::new("powershell.exe");
        command.args(["-NoProfile", "-NonInteractive", "-Command", &format!(
            "[IO.File]::WriteAllText('{}','started'); Start-Sleep -Seconds 30",
            marker.to_string_lossy().replace('\'', "''"))]);
        let (mut child, job, thread) = OwnedJob::prepare(&mut command).unwrap();
        OwnedJob::resume(&thread, &child).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !marker.exists() {
            assert!(std::time::Instant::now() < deadline, "child readiness timeout");
            assert!(child.try_wait().unwrap().is_none(), "child exited before readiness");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(child.try_wait().unwrap().is_none());
        drop(job);
        while child.try_wait().unwrap().is_none() {
            assert!(std::time::Instant::now() < deadline, "closing the job left a live process");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        // KILL_ON_JOB_CLOSE need not produce a nonzero process exit code.
        child.wait().unwrap();
    }

    #[test]
    fn job_ownership_survives_root_exit_and_stops_the_late_descendant_write() {
        use std::time::{Duration, Instant};
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("forbidden-late-write.txt");
        let late = format!("Start-Sleep -Milliseconds 1500; [IO.File]::WriteAllText('{}','late')",
            marker.to_string_lossy().replace('\'', "''"));
        let parent = format!("$payload=[Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes('{}')); $info=New-Object Diagnostics.ProcessStartInfo; $info.FileName='powershell.exe'; $info.Arguments='-NoProfile -NonInteractive -EncodedCommand '+$payload; $info.UseShellExecute=$false; [Diagnostics.Process]::Start($info) | Out-Null; exit 0",
            late.replace('\'', "''"));
        let mut command = Command::new("powershell.exe");
        command.args(["-NoProfile", "-NonInteractive", "-Command", &parent]);
        let mut heartbeat = super::super::ShellLeaseHeartbeat::start_with_renewal(30_000,
            || Ok(nola_runtime_driver::monotonic_epoch_millis() + 30_000)).unwrap();
        let mut child = heartbeat.spawn(&mut command).unwrap();
        let started = Instant::now();
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success(), "retain the root's real result independently of its descendants");
                break;
            }
            assert!(started.elapsed() < Duration::from_secs(15));
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(child.residual_processes_stopped().is_some_and(|count| count > 0));
        child.terminate(); // Termination owns the Job, not the reaped root PID.
        std::thread::sleep(Duration::from_millis(1_700));
        assert!(!marker.exists());
        heartbeat.stop();
    }
}
