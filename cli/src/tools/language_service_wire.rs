// SPDX-License-Identifier: Apache-2.0
//! Bounded LSP stdio framing. Transport only: no process launch, permissions,
//! fallback scanning or claims about semantic correctness.

use std::io::{self, BufRead, Write};
use serde_json::Value;

const MAX_HEADER_BYTES: usize = 8 * 1024;
pub(super) const MAX_MESSAGE_BYTES: usize = 8 * 1024 * 1024;

pub(super) fn read_message(reader: &mut impl BufRead) -> io::Result<Value> {
    let invalid = |message| io::Error::new(io::ErrorKind::InvalidData, message);
    let mut header = Vec::new();
    // Do not use unbounded read_until: a broken server could omit CRLF.
    loop {
        let available = reader.fill_buf()?;
        let Some(&byte) = available.first() else {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "language service header incomplete"));
        };
        header.push(byte);
        reader.consume(1);
        if header.ends_with(b"\r\n\r\n") { break; }
        if header.len() >= MAX_HEADER_BYTES { return Err(invalid("language service header exceeds limit")); }
    }
    let header = std::str::from_utf8(&header).map_err(|_| invalid("language service header is not UTF-8"))?;
    let mut length = None;
    for line in header[..header.len() - 4].split("\r\n") {
        let (key, value) = line.split_once(':').ok_or_else(|| invalid("malformed language service header"))?;
        if key.eq_ignore_ascii_case("Content-Length") {
            if length.is_some() { return Err(invalid("duplicate Content-Length")); }
            let value = value.trim();
            if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(invalid("invalid Content-Length"));
            }
            let bytes = value.parse::<usize>().map_err(|_| invalid("Content-Length overflow"))?;
            if bytes == 0 || bytes > MAX_MESSAGE_BYTES { return Err(invalid("language service message exceeds limit")); }
            length = Some(bytes);
        }
    }
    let mut body = vec![0; length.ok_or_else(|| invalid("missing Content-Length"))?];
    reader.read_exact(&mut body)?;
    let message: Value = serde_json::from_slice(&body).map_err(|_| invalid("invalid language service JSON"))?;
    validate_message(&message)?;
    Ok(message)
}

pub(super) fn write_message(writer: &mut impl Write, message: &Value) -> io::Result<()> {
    validate_message(message)?;
    let body = serde_json::to_vec(message)?;
    if body.len() > MAX_MESSAGE_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "language service message exceeds limit"));
    }
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(&body)?;
    writer.flush()
}

fn validate_message(message: &Value) -> io::Result<()> {
    if !message.is_object() || message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "expected JSON-RPC 2.0 object"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use serde_json::json;

    #[test]
    fn preserves_utf8_byte_lengths_and_message_boundaries() {
        let first = json!({"jsonrpc":"2.0", "id":1,"result":"中文🦀"});
        let second = json!({"jsonrpc":"2.0", "method":"initialized", "params":{}});
        let mut bytes = Vec::new();
        write_message(&mut bytes, &first).unwrap();
        write_message(&mut bytes, &second).unwrap();
        let mut reader = Cursor::new(bytes);
        assert_eq!(read_message(&mut reader).unwrap(), first);
        assert_eq!(read_message(&mut reader).unwrap(), second);
        assert_eq!(read_message(&mut reader).unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn rejects_unbounded_ambiguous_and_incomplete_frames() {
        for bytes in [
            b"Content-Length: 2\r\nContent-Length: 2\r\n\r\n{}".to_vec(),
            b"Content-Length: 999999999\r\n\r\n".to_vec(),
            b"Content-Length: -1\r\n\r\n".to_vec(),
            b"Content-Length: 30\r\n\r\n{}".to_vec(),
            b"Content-Length: 2\r\n\r\n{}".to_vec(),
            b"X: y\r\n\r\n".to_vec(),
            vec![b'x'; MAX_HEADER_BYTES + 1],
        ] { assert!(read_message(&mut Cursor::new(bytes)).is_err()); }
    }
}
