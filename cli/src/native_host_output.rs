// SPDX-License-Identifier: Apache-2.0
// One ordered native protocol writer, shared by Core presentation and Host interactions.
use serde_json::Value;
use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::{Mutex, OnceLock};

enum Frame {
    Reserved(String),
    Ready(Vec<u8>),
}

struct NativeOutput<W> {
    writer: W,
    frames: VecDeque<Frame>,
    failed: bool,
    queued_bytes: usize,
}

impl<W: Write> NativeOutput<W> {
    fn new(writer: W) -> Self {
        Self { writer, frames: VecDeque::new(), failed: false, queued_bytes: 0 }
    }

    fn check(&mut self) -> io::Result<()> {
        if self.failed || self.frames.len() >= 1024 {
            self.failed = true;
            return Err(io::Error::other("native output unavailable or reservation queue exhausted"));
        }
        Ok(())
    }

    fn reserve(&mut self, event_id: &str) -> io::Result<()> {
        self.check()?;
        if event_id.is_empty() || self.frames.iter().any(|frame| matches!(frame, Frame::Reserved(id) if id == event_id)) {
            return Err(io::Error::other("invalid native challenge reservation"));
        }
        self.frames.push_back(Frame::Reserved(event_id.to_string()));
        Ok(())
    }

    fn emit(&mut self, value: Value, challenge: bool) -> io::Result<()> {
        self.check()?;
        let mut bytes = serde_json::to_vec(&value)?;
        bytes.push(b'\n');
        if self.queued_bytes.saturating_add(bytes.len()) > 32 * 1024 * 1024 {
            self.failed = true;
            return Err(io::Error::other("native output reservation byte limit exceeded"));
        }
        let byte_count = bytes.len();
        if challenge {
            let id = value.pointer("/trace/event_id").and_then(Value::as_str)
                .ok_or_else(|| io::Error::other("native challenge has no trace identity"))?;
            let slot = self.frames.iter_mut().find(|frame| matches!(frame, Frame::Reserved(reserved) if reserved == id))
                .ok_or_else(|| io::Error::other("native challenge has no reserved position"))?;
            *slot = Frame::Ready(bytes);
        } else {
            self.frames.push_back(Frame::Ready(bytes));
        }
        self.queued_bytes += byte_count;
        // Completing a reservation drains immediately, under the SAME lock as
        // the challenge write. Never wait for another Emitter event to flush.
        while matches!(self.frames.front(), Some(Frame::Ready(_))) {
            let Some(Frame::Ready(bytes)) = self.frames.pop_front() else { unreachable!() };
            self.queued_bytes -= bytes.len();
            if let Err(error) = self.writer.write_all(&bytes).and_then(|_| self.writer.flush()) {
                self.failed = true;
                return Err(error);
            }
        }
        if challenge && !self.frames.is_empty() {
            self.failed = true;
            return Err(io::Error::other("native interaction cannot wait behind an unresolved output reservation"));
        }
        Ok(())
    }
}

static OUTPUT: OnceLock<Mutex<NativeOutput<io::Stdout>>> = OnceLock::new();

fn with_output<T>(operation: impl FnOnce(&mut NativeOutput<io::Stdout>) -> io::Result<T>) -> io::Result<T> {
    let mut output = OUTPUT.get_or_init(|| Mutex::new(NativeOutput::new(io::stdout())))
        .lock().map_err(|_| io::Error::other("native output lock poisoned"))?;
    operation(&mut output)
}

pub fn reserve_challenge(event_id: &str) -> io::Result<()> {
    with_output(|output| output.reserve(event_id))
}

pub fn emit(value: Value) -> io::Result<()> {
    let challenge = value.get("type").and_then(Value::as_str) == Some("permission_request");
    let interaction = matches!(value.get("type").and_then(Value::as_str),
        Some("permission_request" | "user_input_wait_changed" | "device_tool_request"));
    with_output(|output| {
        output.emit(value, challenge)?;
        if interaction && !output.frames.is_empty() {
            output.failed = true;
            return Err(io::Error::other("native interaction was not delivered before wait"));
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn challenge_drains_projection_without_any_following_event() {
        let mut output = NativeOutput::new(Vec::new());
        output.reserve("permission-1").unwrap();
        output.emit(json!({"type":"core_permission_projection_updated"}), false).unwrap();
        assert!(output.writer.is_empty());
        output.emit(json!({"type":"permission_request","trace":{"event_id":"permission-1"}}), true).unwrap();
        let frames: Vec<Value> = output.writer.split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty()).map(|line| serde_json::from_slice(line).unwrap()).collect();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0]["type"], "permission_request");
        assert_eq!(frames[1]["type"], "core_permission_projection_updated");
        assert!(output.frames.is_empty());
    }

    #[test]
    fn challenge_before_projection_preserves_order() {
        let mut output = NativeOutput::new(Vec::new());
        output.reserve("p").unwrap();
        output.emit(json!({"trace":{"event_id":"p"}}), true).unwrap();
        output.emit(json!({"type":"projection"}), false).unwrap();
        assert_eq!(output.writer.iter().filter(|byte| **byte == b'\n').count(), 2);
    }

    #[test]
    fn unmatched_or_duplicate_challenge_cannot_enter_wait() {
        let mut output = NativeOutput::new(Vec::new());
        assert!(output.emit(json!({"trace":{"event_id":"p"}}), true).is_err());
        output.reserve("p").unwrap();
        output.emit(json!({"trace":{"event_id":"p"}}), true).unwrap();
        assert!(output.emit(json!({"trace":{"event_id":"p"}}), true).is_err());
    }

    #[test]
    fn write_failure_is_latched_and_never_reported_as_delivered() {
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> { Err(io::ErrorKind::BrokenPipe.into()) }
            fn flush(&mut self) -> io::Result<()> { Ok(()) }
        }
        let mut output = NativeOutput::new(Broken);
        output.reserve("p").unwrap();
        assert!(output.emit(json!({"trace":{"event_id":"p"}}), true).is_err());
        assert!(output.emit(json!({}), false).is_err());
    }
}
