// SPDX-License-Identifier: Apache-2.0
//! Disconnect-aware projection waits, with no timer-driven database polling.

use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::time::Instant;

pub(crate) struct ProjectionWakeSender(UnixStream);
pub(crate) struct ProjectionWakeReceiver(UnixStream);

impl ProjectionWakeSender {
    pub(crate) fn notify(&self) {
        // A full notification socket already contains a wake. Never block the
        // single writer on a slow subscriber or a concurrently closed wait.
        let _ = (&self.0).write(&[1]);
    }
}

impl ProjectionWakeReceiver {
    pub(crate) fn channel() -> std::io::Result<(Arc<ProjectionWakeSender>, Self)> {
        let (sender, receiver) = UnixStream::pair()?;
        sender.set_nonblocking(true)?;
        receiver.set_nonblocking(true)?;
        Ok((Arc::new(ProjectionWakeSender(sender)), Self(receiver)))
    }

    pub(crate) fn wait(&mut self, client: &UnixStream, deadline: Instant) -> Result<(), String> {
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(());
            }
            let timeout = remaining
                .as_millis()
                .saturating_add(1)
                .min(i32::MAX as u128) as i32;
            let mut descriptors = [
                libc::pollfd {
                    fd: client.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: self.0.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            let ready = unsafe { libc::poll(descriptors.as_mut_ptr(), 2, timeout) };
            if ready == 0 {
                return Ok(());
            }
            if ready < 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(format!("projection wait failed: {error}"));
            }
            let failed = libc::POLLHUP | libc::POLLERR | libc::POLLNVAL;
            if descriptors[0].revents & failed != 0 {
                return Err("projection client disconnected".into());
            }
            if descriptors[0].revents & libc::POLLIN != 0 {
                let mut byte = 0u8;
                let count = unsafe {
                    libc::recv(
                        client.as_raw_fd(),
                        (&mut byte as *mut u8).cast(),
                        1,
                        libc::MSG_PEEK | libc::MSG_DONTWAIT,
                    )
                };
                if count == 0 {
                    return Err("projection client disconnected".into());
                }
                if count > 0 {
                    return Err("projection connection accepts only one request frame".into());
                }
                let error = std::io::Error::last_os_error();
                if error.kind() != std::io::ErrorKind::WouldBlock
                    && error.kind() != std::io::ErrorKind::Interrupted
                {
                    return Err(format!("inspect projection client failed: {error}"));
                }
            }
            if descriptors[1].revents & failed != 0 {
                return Err("projection notification channel closed".into());
            }
            if descriptors[1].revents & libc::POLLIN != 0 {
                let mut pending = [0u8; 256];
                loop {
                    match self.0.read(&mut pending) {
                        Ok(0) => return Err("projection notification channel closed".into()),
                        Ok(_) => continue,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(error) => {
                            return Err(format!("read projection notification failed: {error}"))
                        }
                    }
                }
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn queued_notification_is_not_lost_before_poll_starts() {
        let (sender, mut receiver) = ProjectionWakeReceiver::channel().unwrap();
        let (connection, _peer) = UnixStream::pair().unwrap();
        for _ in 0..1024 {
            sender.notify();
        }
        let started = Instant::now();
        receiver
            .wait(&connection, started + Duration::from_secs(5))
            .unwrap();
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn peer_disconnect_ends_an_unnotified_wait() {
        let (_sender, mut receiver) = ProjectionWakeReceiver::channel().unwrap();
        let (connection, peer) = UnixStream::pair().unwrap();
        drop(peer);
        let started = Instant::now();
        let error = receiver
            .wait(&connection, started + Duration::from_secs(5))
            .unwrap_err();
        assert!(error.contains("disconnected"));
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
