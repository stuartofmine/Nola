// SPDX-License-Identifier: Apache-2.0
//! Condvar wakes on commits; Winsock reports abandoned long polls without querying the database.
use std::os::windows::io::AsRawSocket;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use uds_windows::UnixStream;
use windows_sys::Win32::Networking::WinSock::{WSAPoll, WSAPOLLFD, POLLRDNORM};

pub(crate) struct ProjectionWakeSender {
    pending: Mutex<bool>,
    changed: Condvar,
}

pub(crate) struct ProjectionWakeReceiver(Arc<ProjectionWakeSender>);

impl ProjectionWakeSender {
    pub(crate) fn notify(&self) {
        *self.pending.lock().unwrap_or_else(|poison| poison.into_inner()) = true;
        self.changed.notify_one();
    }
}

impl ProjectionWakeReceiver {
    pub(crate) fn channel() -> std::io::Result<(Arc<ProjectionWakeSender>, Self)> {
        let sender = Arc::new(ProjectionWakeSender { pending: Mutex::new(false), changed: Condvar::new() });
        Ok((sender.clone(), Self(sender)))
    }

    pub(crate) fn wait(&mut self, client: &UnixStream, deadline: Instant) -> Result<(), String> {
        let mut pending = self.0.pending.lock().map_err(|_| "projection wake lock poisoned")?;
        while !*pending {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() { return Ok(()); }
            let mut descriptor = WSAPOLLFD { fd: client.as_raw_socket() as _, events: POLLRDNORM, revents: 0 };
            // After its single request frame a client must remain silent until the reply.
            if unsafe { WSAPoll(&mut descriptor, 1, 0) } != 0 {
                return Err("projection client disconnected or sent a second frame".into());
            }
            (pending, _) = self.0.changed.wait_timeout(pending, remaining.min(Duration::from_millis(250)))
                .map_err(|_| "projection wake lock poisoned")?;
        }
        *pending = false;
        Ok(())
    }
}
