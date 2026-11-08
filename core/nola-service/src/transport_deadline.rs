// SPDX-License-Identifier: Apache-2.0
//! Per-frame I/O budgets; these do not limit Application command execution.

use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

pub(super) struct DeadlineStream<'a> {
    stream: &'a UnixStream,
    deadline: Instant,
}

impl<'a> DeadlineStream<'a> {
    pub(super) fn new(stream: &'a UnixStream, budget: Duration) -> Self {
        Self {
            stream,
            deadline: Instant::now() + budget,
        }
    }

    fn remaining(&self) -> io::Result<Duration> {
        self.deadline
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::TimedOut, "service frame deadline exceeded")
            })
    }
}

impl Read for DeadlineStream<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.stream.set_read_timeout(Some(self.remaining()?))?;
        self.stream.read(bytes)
    }
}

impl Write for DeadlineStream<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.stream.set_write_timeout(Some(self.remaining()?))?;
        self.stream.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.set_write_timeout(Some(self.remaining()?))?;
        self.stream.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};

    #[test]
    fn trickling_request_does_not_reset_frame_budget() {
        let (server, mut client) = UnixStream::pair().unwrap();
        let peer = std::thread::spawn(move || {
            client
                .set_write_timeout(Some(Duration::from_millis(100)))
                .unwrap();
            for _ in 0..40 {
                if client.write_all(b" ").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        let start = Instant::now();
        let mut frame = Vec::new();
        let result = BufReader::new(DeadlineStream::new(&server, Duration::from_millis(100)))
            .read_until(b'\n', &mut frame);
        let elapsed = start.elapsed();
        drop(server);
        peer.join().unwrap();
        assert!(result.is_err());
        assert!(!frame.is_empty(), "fixture must deliver partial data");
        assert!(
            elapsed < Duration::from_millis(600),
            "read took {elapsed:?}"
        );
    }

    #[test]
    fn nonreading_client_cannot_hold_response_writer_indefinitely() {
        let (server, _client) = UnixStream::pair().unwrap();
        let mut writer = DeadlineStream::new(&server, Duration::from_millis(100));
        let frame = vec![b'x'; 8 * 1024 * 1024];
        let start = Instant::now();
        assert!(writer.write_all(&frame).is_err());
        assert!(start.elapsed() < Duration::from_millis(600));
    }

    #[test]
    fn complete_request_and_response_keep_their_bytes() {
        let (server, mut client) = UnixStream::pair().unwrap();
        client.write_all(b"request\n").unwrap();
        let mut stream = DeadlineStream::new(&server, Duration::from_secs(1));
        let mut request = [0; 8];
        stream.read_exact(&mut request).unwrap();
        assert_eq!(&request, b"request\n");
        stream.write_all(b"response\n").unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let mut response = [0; 9];
        client.read_exact(&mut response).unwrap();
        assert_eq!(&response, b"response\n");
    }
}
