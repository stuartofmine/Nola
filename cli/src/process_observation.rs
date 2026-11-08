// SPDX-License-Identifier: Apache-2.0
//! Host process observations, independent of terminal presentation.
//! Unavailable observations never prove that another owner has exited.

pub(crate) fn process_id_is_alive(pid: u32) -> bool {
    if pid == 0 || pid > i32::MAX as u32 {
        return false;
    }
    #[cfg(target_os = "linux")]
    if matches!(process_start_identity(pid), Ok(None)) { return false; }
    platform_process_is_alive(pid)
}

#[cfg(unix)]
fn platform_process_is_alive(pid: u32) -> bool {
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if result == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

#[cfg(not(unix))]
fn platform_process_is_alive(_pid: u32) -> bool {
    // No reliable observation is available here; do not retire another owner
    // based on localized command output or a failed diagnostic subprocess.
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_process_is_alive() {
        assert!(process_id_is_alive(std::process::id()));
    }

    #[test]
    fn invalid_identifiers_cannot_become_process_group_probes() {
        assert!(!process_id_is_alive(0));
        assert!(!process_id_is_alive(u32::MAX));
    }
}

/// OS facts stay in the Host; interpretation and lease encoding live in Core.
pub(crate) fn current_worker_identity() -> Result<nola_host_protocol::HostWorkerProcessIdentity, String> {
    match observe_process(std::process::id()) {
        nola_host_protocol::HostProcessObservation::Running { pid, boot_id, start_id } => {
            let identity = nola_host_protocol::HostWorkerProcessIdentity {
                pid, boot_id, start_id, nonce: uuid::Uuid::new_v4().to_string(),
            };
            identity.validate()?;
            Ok(identity)
        }
        _ => Err("Cannot establish this Host process identity".into()),
    }
}

pub(crate) fn observe_process(pid: u32) -> nola_host_protocol::HostProcessObservation {
    use nola_host_protocol::HostProcessObservation;
    if pid == 0 || pid > i32::MAX as u32 { return HostProcessObservation::Unknown { pid }; }
    let Ok(boot_id) = boot_identity() else { return HostProcessObservation::Unknown { pid }; };
    match process_start_identity(pid) {
        Ok(Some(start_id)) => HostProcessObservation::Running { pid, boot_id, start_id },
        Ok(None) => HostProcessObservation::Gone { pid, boot_id },
        Err(()) => HostProcessObservation::Unknown { pid },
    }
}

#[cfg(target_os = "macos")]
fn boot_identity() -> Result<String, ()> {
    let mut bytes = [0u8; 128];
    let mut length = bytes.len();
    let result = unsafe { libc::sysctlbyname(c"kern.bootsessionuuid".as_ptr(), bytes.as_mut_ptr().cast(),
        &mut length, std::ptr::null_mut(), 0) };
    if result != 0 || length == 0 || length > bytes.len() { return Err(()); }
    let text = std::str::from_utf8(&bytes[..length]).map_err(|_| ())?.trim_end_matches('\0');
    if text.is_empty() { return Err(()); }
    Ok(text.to_owned())
}

#[cfg(target_os = "macos")]
fn process_start_identity(pid: u32) -> Result<Option<String>, ()> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of_val(&info) as libc::c_int;
    let count = unsafe { libc::proc_pidinfo(pid as i32, libc::PROC_PIDTBSDINFO, 0,
        (&mut info as *mut libc::proc_bsdinfo).cast(), size) };
    if count == size && info.pbi_pid == pid && info.pbi_start_tvsec > 0 {
        return Ok(Some(format!("{}:{}", info.pbi_start_tvsec, info.pbi_start_tvusec)));
    }
    if count == 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) { return Ok(None); }
    Err(())
}

#[cfg(target_os = "linux")]
fn boot_identity() -> Result<String, ()> {
    std::fs::read_to_string("/proc/sys/kernel/random/boot_id").map(|v| v.trim().to_owned()).map_err(|_| ())
}

#[cfg(target_os = "linux")]
fn process_start_identity(pid: u32) -> Result<Option<String>, ()> {
    let stat = match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(()),
    };
    let (head, tail) = stat.rsplit_once(')').ok_or(())?;
    if head.split_whitespace().next().and_then(|v| v.parse::<u32>().ok()) != Some(pid) { return Err(()); }
    if matches!(tail.split_whitespace().next(), Some("Z" | "X" | "x")) { return Ok(None); }
    let start = tail.split_whitespace().nth(19).ok_or(())?.parse::<u64>().map_err(|_| ())?;
    if start == 0 { return Err(()); }
    Ok(Some(start.to_string()))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn boot_identity() -> Result<String, ()> { Err(()) }
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn process_start_identity(_pid: u32) -> Result<Option<String>, ()> { Err(()) }

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod worker_identity_tests {
    use super::*;
    #[test]
    fn observed_current_worker_round_trips_and_remains_live() {
        let owner = current_worker_identity().unwrap();
        let lease = owner.lease_identity().unwrap();
        assert_eq!(nola_host_protocol::HostWorkerProcessIdentity::from_lease(&lease.worker_generation, &lease.lease_owner).unwrap(), owner);
        assert!(!owner.is_terminated(&observe_process(owner.pid)).unwrap());
    }
    #[test]
    fn child_exit_is_os_observed_not_inferred_from_a_new_nonce() {
        let mut child = std::process::Command::new("sleep").arg("30").spawn().unwrap();
        let observed = observe_process(child.id());
        let nola_host_protocol::HostProcessObservation::Running {pid,boot_id,start_id} = observed else {
            let _ = child.kill(); let _ = child.wait(); panic!("Child identity unavailable");
        };
        let owner = nola_host_protocol::HostWorkerProcessIdentity {pid,boot_id,start_id,nonce:"test".into()};
        assert!(!owner.is_terminated(&observe_process(pid)).unwrap());
        child.kill().unwrap();
        #[cfg(target_os = "linux")]
        {
            let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
            assert_eq!(unsafe { libc::waitid(libc::P_PID, pid, &mut info, libc::WEXITED | libc::WNOWAIT) }, 0);
            assert!(!process_id_is_alive(pid));
            assert!(owner.is_terminated(&observe_process(pid)).unwrap());
        }
        child.wait().unwrap();
        assert!(owner.is_terminated(&observe_process(pid)).unwrap());
    }
}

#[cfg(test)]
pub(crate) fn test_worker_lease(pid: u32) -> nola_host_protocol::HostWorkerLeaseIdentity {
    let mut identity = current_worker_identity().unwrap();
    identity.pid = pid;
    identity.lease_identity().unwrap()
}
