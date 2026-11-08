// SPDX-License-Identifier: Apache-2.0
//! Worker-owned pseudoterminal IO. Callers must establish execution authority
//! and generation ownership before invoking this module. It never retries input,
//! persists user state, discovers devices, or infers process completion.

use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

pub const MAX_IO_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalSize {
    pub rows: u16,
    pub columns: u16,
}

impl TerminalSize {
    fn native(self) -> io::Result<libc::winsize> {
        if self.rows == 0 || self.columns == 0 || self.rows > 4096 || self.columns > 4096 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "terminal dimensions must be 1..=4096"));
        }
        Ok(libc::winsize { ws_row: self.rows, ws_col: self.columns, ws_xpixel: 0, ws_ypixel: 0 })
    }
}

pub struct TerminalMaster {
    file: File,
}

/// Consumed exactly once while constructing the admitted worker's child.
pub struct TerminalSlave {
    file: File,
}

pub fn open(size: TerminalSize) -> io::Result<(TerminalMaster, TerminalSlave)> {
    let mut dimensions = size.native()?;
    let (mut master, mut slave) = (-1, -1);
    // SAFETY: output descriptors and winsize point to live owned storage; null
    // name and termios ask the OS to use defaults without writing a name buffer.
    if unsafe { libc::openpty(&mut master, &mut slave, std::ptr::null_mut(),
        std::ptr::null_mut(), &mut dimensions) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful openpty transfers two distinct owned descriptors.
    let master = unsafe { File::from_raw_fd(master) };
    let slave = unsafe { File::from_raw_fd(slave) };
    for file in [&master, &slave] {
        if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } == -1 {
            return Err(io::Error::last_os_error());
        }
    }
    let flags = unsafe { libc::fcntl(master.as_raw_fd(), libc::F_GETFL) };
    if flags == -1 || unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok((TerminalMaster { file: master }, TerminalSlave { file: slave }))
}

impl TerminalSlave {
    /// The child creates its own session and controlling terminal. Do not also
    /// configure Command::process_group: a group leader cannot call setsid.
    pub fn attach(self, command: &mut Command) -> io::Result<()> {
        command.stdin(Stdio::from(self.file.try_clone()?))
            .stdout(Stdio::from(self.file.try_clone()?))
            .stderr(Stdio::from(self.file));
        // SAFETY: callback only invokes async-signal-safe OS operations. std
        // installs the child's stdio descriptors before executing this callback.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 { return Err(io::Error::last_os_error()); }
                if libc::ioctl(libc::STDIN_FILENO, libc::TIOCSCTTY as _, 0) == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        Ok(())
    }
}

impl TerminalMaster {
    /// A single physical write, surrounded by durable CAS commits. `commit`
    /// must recheck the live task owner, grant and generation in its transaction.
    /// An error after the write witness commits must never trigger a replay.
    pub fn deliver_input(
        &mut self,
        receipt: &nola_domain::terminal_input::TerminalInputReceipt,
        identity: &nola_domain::terminal_input::TerminalInputIdentity,
        bytes: &[u8],
        commit: impl FnMut(&nola_domain::terminal_input::TerminalInputReceipt,
            &nola_domain::terminal_input::TerminalInputReceipt) -> Result<(), String>,
    ) -> Result<nola_domain::terminal_input::TerminalInputReceipt, String> {
        deliver_input(receipt, identity, bytes, commit, |suffix| self.write_input(suffix))
    }
    /// Nonblocking bounded read. WouldBlock is a live idle terminal, not EOF.
    /// OS errors (including Linux PTY EIO) remain errors; only the child owner
    /// may determine whether a process has exited.
    pub fn read_available(&mut self, max_bytes: usize) -> io::Result<Vec<u8>> {
        if max_bytes == 0 || max_bytes > MAX_IO_BYTES {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid terminal read budget"));
        }
        let mut buffer = vec![0; max_bytes];
        let size = self.file.read(&mut buffer)?;
        buffer.truncate(size);
        Ok(buffer)
    }

    /// Returns exactly the acknowledged byte count. Partial writes and
    /// WouldBlock must be handled by the input ledger, never replayed blindly.
    pub fn write_input(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.is_empty() || bytes.len() > MAX_IO_BYTES {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid terminal input budget"));
        }
        self.file.write(bytes)
    }

    pub fn resize(&self, size: TerminalSize) -> io::Result<()> {
        let dimensions = size.native()?;
        if unsafe { libc::ioctl(self.file.as_raw_fd(), libc::TIOCSWINSZ as _, &dimensions) } == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

fn deliver_input(
    receipt: &nola_domain::terminal_input::TerminalInputReceipt,
    identity: &nola_domain::terminal_input::TerminalInputIdentity,
    bytes: &[u8],
    mut commit: impl FnMut(&nola_domain::terminal_input::TerminalInputReceipt,
        &nola_domain::terminal_input::TerminalInputReceipt) -> Result<(), String>,
    mut write: impl FnMut(&[u8]) -> io::Result<usize>,
) -> Result<nola_domain::terminal_input::TerminalInputReceipt, String> {
    use nola_domain::terminal_input::TerminalInputPhase as Phase;
    if !receipt.matches_input(identity, bytes) {
        return Err("terminal input identity or content does not match its receipt".into());
    }
    if !matches!(receipt.phase, Phase::Prepared | Phase::Partial) {
        return Err("terminal input is not eligible for a physical write".into());
    }
    let mut writing = receipt.clone(); writing.phase = Phase::Writing;
    writing.follows(Some(receipt))?;
    commit(receipt, &writing)?;
    let mut next = writing.clone();
    match write(&bytes[writing.acknowledged_bytes..]) {
        Ok(count) if count <= bytes.len() - writing.acknowledged_bytes => {
            next.acknowledged_bytes += count;
            next.phase = if next.acknowledged_bytes == next.size_bytes { Phase::Delivered } else { Phase::Partial };
        },
        Err(error) if matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted) => {
            // No bytes accepted by this nonblocking write. A later attempt
            // still needs a new committed Writing witness and live ownership.
            next.phase = Phase::Partial;
        },
        _ => next.phase = Phase::Unknown,
    }
    next.follows(Some(&writing))?;
    commit(&writing, &next).map_err(|error| format!("terminal input settlement unknown; do not resend: {error}"))?;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input_receipt() -> nola_domain::terminal_input::TerminalInputReceipt {
        nola_domain::terminal_input::TerminalInputReceipt::prepare(
            nola_domain::terminal_input::TerminalInputIdentity { host_id:"host".into(),
                owner_session_id:"session".into(), task_id:"task".into(), attempt_id:"attempt".into(),
                worker_generation:"generation".into(), request_id:"request".into() }, b"run\n").unwrap()
    }

    #[test]
    fn physical_input_requires_committed_intent_and_cannot_replay() {
        use nola_domain::terminal_input::TerminalInputPhase as Phase;
        let receipt = input_receipt();
        let calls = std::cell::Cell::new(0);
        let denied = deliver_input(&receipt, &receipt.identity, b"run\n", |_, _| Err("stale owner".into()),
            |_| { calls.set(calls.get() + 1); Ok(4) });
        assert!(denied.is_err()); assert_eq!(calls.get(), 0);
        let mut committed = receipt.clone();
        let partial = deliver_input(&receipt, &receipt.identity, b"run\n", |old, next| {
            assert_eq!(old, &committed); committed = next.clone(); Ok(())
        }, |bytes| { assert_eq!(bytes, b"run\n"); Ok(2) }).unwrap();
        assert_eq!(partial.phase, Phase::Partial);
        let delivered = deliver_input(&partial, &receipt.identity, b"run\n", |old, next| {
            assert_eq!(old, &committed); committed = next.clone(); Ok(())
        }, |bytes| { assert_eq!(bytes, b"n\n"); Ok(2) }).unwrap();
        assert_eq!(delivered.phase, Phase::Delivered);
        assert!(deliver_input(&delivered, &receipt.identity, b"run\n", |_, _| panic!("must not commit"),
            |_| panic!("must not write")).is_err());
    }

    #[test]
    fn backpressure_preserves_offset_and_unknown_delivery_never_replays() {
        use nola_domain::terminal_input::TerminalInputPhase as Phase;
        let receipt = input_receipt();
        for kind in [io::ErrorKind::WouldBlock, io::ErrorKind::Interrupted] {
            let mut commits = Vec::new();
            let deferred = deliver_input(&receipt, &receipt.identity, b"run\n",
                |old, next| { commits.push((old.phase, next.phase)); Ok(()) },
                |_| Err(io::Error::from(kind))).unwrap();
            assert_eq!(deferred.phase, Phase::Partial);
            assert_eq!(deferred.acknowledged_bytes, 0);
            assert_eq!(commits, vec![(Phase::Prepared, Phase::Writing), (Phase::Writing, Phase::Partial)]);
            let completed = deliver_input(&deferred, &receipt.identity, b"run\n",
                |_, _| Ok(()), |bytes| { assert_eq!(bytes, b"run\n"); Ok(bytes.len()) }).unwrap();
            assert_eq!(completed.phase, Phase::Delivered);
        }
        for invalid_count in [false, true] {
            let unknown = deliver_input(&receipt, &receipt.identity, b"run\n",
                |_, _| Ok(()), |_| if invalid_count { Ok(5) } else { Err(io::Error::from(io::ErrorKind::BrokenPipe)) }).unwrap();
            assert_eq!(unknown.phase, Phase::Unknown);
            assert_eq!(unknown.acknowledged_bytes, 0);
            assert!(deliver_input(&unknown, &receipt.identity, b"run\n",
                |_, _| panic!("unknown delivery cannot commit a retry"),
                |_| panic!("unknown delivery cannot be replayed")).is_err());
        }
    }

    #[test]
    fn changed_content_or_worker_cannot_reach_physical_io() {
        let receipt = input_receipt();
        let mut other = receipt.identity.clone();
        other.worker_generation = "replacement".into();
        for (identity, bytes) in [(&other, b"run\n".as_slice()),
            (&receipt.identity, b"RUN\n".as_slice())] {
            assert!(deliver_input(&receipt, identity, bytes,
                |_, _| panic!("mismatch cannot commit"),
                |_| panic!("mismatch cannot write")).is_err());
        }
    }

    #[test]
    fn failed_post_write_commit_leaves_non_replayable_writing_receipt() {
        use nola_domain::terminal_input::TerminalInputPhase as Phase;
        let receipt = input_receipt();
        let mut committed = receipt.clone();
        let result = deliver_input(&receipt, &receipt.identity, b"run\n", |old, next| {
            if next.phase != Phase::Writing { return Err("lost database".into()); }
            assert_eq!(old, &committed); committed = next.clone(); Ok(())
        }, |_| Ok(4));
        assert!(result.unwrap_err().contains("do not resend"));
        assert_eq!(committed.phase, Phase::Writing);
        assert!(deliver_input(&committed, &receipt.identity, b"run\n", |_, _| panic!("no retry"),
            |_| panic!("no resend")).is_err());
    }

    #[test]
    fn rejects_unbounded_io_and_invalid_dimensions() {
        assert!(open(TerminalSize { rows: 0, columns: 80 }).is_err());
        let (mut master, _slave) = open(TerminalSize { rows: 24, columns: 80 }).unwrap();
        assert!(master.read_available(0).is_err());
        assert!(master.read_available(MAX_IO_BYTES + 1).is_err());
        assert!(master.write_input(&vec![0; MAX_IO_BYTES + 1]).is_err());
        assert!(master.resize(TerminalSize { rows: 24, columns: 0 }).is_err());
        master.resize(TerminalSize { rows: 40, columns: 120 }).unwrap();
    }

    #[test]
    fn real_terminal_delivers_receipted_input_and_reads_child_response() {
        struct ChildGuard(std::process::Child);
        impl Drop for ChildGuard {
            fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); }
        }
        let (mut master, slave) = open(TerminalSize { rows: 24, columns: 80 }).unwrap();
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "IFS= read -r value; printf 'received:%s\\n' \"$value\""])
            .env_clear();
        slave.attach(&mut command).unwrap();
        let mut child = ChildGuard(command.spawn().unwrap());
        // Drop Command's retained slave stdio so closure is observable.
        drop(command);
        let receipt = input_receipt();
        let mut durable = receipt.clone();
        let mut current = receipt.clone();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while current.phase != nola_domain::terminal_input::TerminalInputPhase::Delivered {
            assert!(std::time::Instant::now() < deadline, "terminal input timed out");
            current = master.deliver_input(&current, &receipt.identity, b"run\n", |old, next| {
                assert_eq!(old, &durable);
                durable = next.clone();
                Ok(())
            }).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let mut output = Vec::new();
        let mut exited = None;
        loop {
            match master.read_available(MAX_IO_BYTES) {
                Ok(bytes) => output.extend(bytes),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock
                    || error.raw_os_error() == Some(libc::EIO) => {},
                Err(error) => panic!("PTY read failed: {error}"),
            }
            if exited.is_none() { exited = child.0.try_wait().unwrap(); }
            if exited.is_some() && String::from_utf8_lossy(&output).contains("received:run") { break; }
            assert!(std::time::Instant::now() < deadline, "terminal response timed out: {output:?}");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(exited.unwrap().success());
        assert_eq!(durable.acknowledged_bytes, 4);
    }

    #[test]
    fn slave_supplies_a_controlling_terminal_only_to_the_child() {
        let (_master, slave) = open(TerminalSize { rows: 24, columns: 80 }).unwrap();
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "test -t 0 && test -t 1 && test -t 2"]);
        slave.attach(&mut command).unwrap();
        assert!(command.status().unwrap().success());
    }
}
