// SPDX-License-Identifier: Apache-2.0
//! Unix deadline-aware stdio transport. Accepts an already admitted, supervised
//! child; never launches commands or grants process authority itself.

use std::io::{self, BufReader, Read, Write};
use std::os::fd::AsRawFd;
use std::sync::Arc;
use std::time::{Duration, Instant};
use serde_json::Value;
use crate::task_runtime::OwnedShellChild;

pub(super) type Cancellation = Arc<dyn Fn() -> bool + Send + Sync>;

struct DeadlinePipe<T> {
    pipe: T,
    deadline: Instant,
    cancelled: Cancellation,
}

impl<T: AsRawFd> DeadlinePipe<T> {
    fn new(pipe: T, deadline: Instant, cancelled: Cancellation) -> io::Result<Self> {
        let fd = pipe.as_raw_fd();
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { pipe, deadline, cancelled })
    }

    fn ready(&self, events: libc::c_short) -> io::Result<()> {
        loop {
            if (self.cancelled)() {
                // Not Interrupted: read_exact/write_all retry Interrupted.
                return Err(io::Error::other("language service operation cancelled"));
            }
            let remaining = self.deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() { return Err(io::Error::new(io::ErrorKind::TimedOut, "language service deadline expired")); }
            let mut descriptor = libc::pollfd { fd: self.pipe.as_raw_fd(), events, revents: 0 };
            let result = unsafe { libc::poll(&mut descriptor, 1, remaining.as_millis().clamp(1, 50) as i32) };
            if result < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted { continue; }
                return Err(error);
            }
            if result == 0 { continue; }
            if descriptor.revents & libc::POLLNVAL != 0 {
                return Err(io::Error::other("language service pipe is invalid"));
            }
            // HUP/ERR are consumed by read/write for EOF or a precise error.
            return Ok(());
        }
    }
}

impl<T: AsRawFd + Read> Read for DeadlinePipe<T> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if bytes.is_empty() { return Ok(0); }
        loop {
            self.ready(libc::POLLIN)?;
            match self.pipe.read(bytes) {
                Err(error) if matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted) => continue,
                result => return result,
            }
        }
    }
}

impl<T: AsRawFd + Write> Write for DeadlinePipe<T> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.is_empty() { return Ok(0); }
        loop {
            self.ready(libc::POLLOUT)?;
            match self.pipe.write(bytes) {
                Err(error) if matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted) => continue,
                result => return result,
            }
        }
    }
    fn flush(&mut self) -> io::Result<()> { self.pipe.flush() }
}

pub(super) struct Transport {
    child: OwnedShellChild,
    writer: DeadlinePipe<std::process::ChildStdin>,
    reader: BufReader<DeadlinePipe<std::process::ChildStdout>>,
    failed: bool,
    stopped: bool,
}

impl Transport {
    pub(super) fn attach(mut child: OwnedShellChild, deadline: Instant, cancelled: Cancellation) -> io::Result<Self> {
        let (stdin, stdout) = child.take_protocol_pipes()?;
        Ok(Self {
            writer: DeadlinePipe::new(stdin, deadline, cancelled.clone())?,
            reader: BufReader::new(DeadlinePipe::new(stdout, deadline, cancelled)?),
            child, failed: false, stopped: false,
        })
    }

    pub(super) fn send(&mut self, message: &Value, deadline: Instant) -> io::Result<()> {
        if self.failed { return Err(io::Error::other("language service transport is retired")); }
        self.writer.deadline = deadline;
        let result = super::language_service_wire::write_message(&mut self.writer, message);
        if result.is_err() { self.failed = true; }
        result
    }

    pub(super) fn receive(&mut self, deadline: Instant) -> io::Result<Value> {
        if self.failed { return Err(io::Error::other("language service transport is retired")); }
        if Instant::now() >= deadline || (self.reader.get_ref().cancelled)() {
            self.failed = true;
            return Err(io::Error::new(io::ErrorKind::TimedOut, "language service deadline or cancellation reached"));
        }
        self.reader.get_mut().deadline = deadline;
        // Keep the BufReader across messages: it may already hold bytes from
        // the next frame. An incomplete frame retires the connection.
        let result = super::language_service_wire::read_message(&mut self.reader);
        if result.is_err() { self.failed = true; }
        result
    }

    pub(super) fn stop(&mut self) -> Result<(), String> {
        self.failed = true;
        if self.stopped { return Ok(()); }
        self.child.stop_and_confirm(Duration::from_secs(2))?;
        self.stopped = true;
        Ok(())
    }
}

impl Drop for Transport {
    fn drop(&mut self) { let _ = self.stop(); }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn idle_pipe_times_out_and_cancellation_is_not_retried() {
        let (read, _write) = UnixStream::pair().unwrap();
        let cancelled = Arc::new(AtomicBool::new(false));
        let control = cancelled.clone();
        let mut pipe = DeadlinePipe::new(read, Instant::now(), Arc::new(move || control.load(Ordering::Acquire))).unwrap();
        assert_eq!(pipe.read(&mut [0; 1]).unwrap_err().kind(), io::ErrorKind::TimedOut);
        pipe.deadline = Instant::now() + Duration::from_secs(10);
        cancelled.store(true, Ordering::Release);
        assert_eq!(pipe.read_exact(&mut [0; 1]).unwrap_err().kind(), io::ErrorKind::Other);
    }
}
