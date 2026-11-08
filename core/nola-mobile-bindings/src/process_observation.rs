// SPDX-License-Identifier: Apache-2.0
//! Embedded Linux/Android Host OS facts. Core evaluates identity and recovery.
use nola_host_protocol::HostProcessObservation;

pub(crate) fn observe(pid: u32) -> HostProcessObservation {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        if pid == 0 || pid > i32::MAX as u32 {
            return HostProcessObservation::Unknown { pid };
        }
        let Ok(boot) = std::fs::read_to_string("/proc/sys/kernel/random/boot_id") else {
            return HostProcessObservation::Unknown { pid };
        };
        let boot_id = boot.trim().to_owned();
        let stat = match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // A hidden/inaccessible proc entry is not proof that PID died.
                let result = unsafe { libc::kill(pid as i32, 0) };
                return if result == -1
                    && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
                {
                    HostProcessObservation::Gone { pid, boot_id }
                } else {
                    HostProcessObservation::Unknown { pid }
                };
            }
            Err(_) => return HostProcessObservation::Unknown { pid },
        };
        let Some((head, fields)) = stat.rsplit_once(')') else {
            return HostProcessObservation::Unknown { pid };
        };
        if head
            .split_whitespace()
            .next()
            .and_then(|s| s.parse::<u32>().ok())
            != Some(pid)
        {
            return HostProcessObservation::Unknown { pid };
        }
        let Some(start) = fields
            .split_whitespace()
            .nth(19)
            .filter(|s| s.parse::<u64>().is_ok_and(|v| v > 0))
        else {
            return HostProcessObservation::Unknown { pid };
        };
        HostProcessObservation::Running {
            pid,
            boot_id,
            start_id: start.to_owned(),
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        HostProcessObservation::Unknown { pid }
    }
}
