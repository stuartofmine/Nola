// SPDX-License-Identifier: Apache-2.0
//! Validation-only JSONL port: real VT100 committed frames and read-only Run
//! receipts. Not linked into the product CLI and never starts a Service.
use base64::Engine;
use rusqlite::{Connection, OpenFlags};
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{self, BufRead, Read, Write};

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    Feed { bytes_base64: String },
    Resize { rows: u16, columns: u16 },
    Audit { database: String, live: bool },
}

struct Terminal {
    parser: vt100::Parser,
    pending: Vec<u8>,
    offset: usize,
    frame_id: usize,
}

impl Terminal {
    fn new() -> Self {
        Self { parser: vt100::Parser::new(24, 100, 2_000), pending: Vec::new(), offset: 0, frame_id: 0 }
    }
    fn feed(&mut self, bytes: &[u8]) -> Value {
        self.pending.extend_from_slice(bytes);
        let close = b"\x1b[?2026l";
        let mut frames = Vec::new();
        while let Some(index) = self.pending.windows(close.len()).position(|part| part == close) {
            let count = index + close.len();
            self.parser.process(&self.pending[..count]);
            self.pending.drain(..count);
            self.offset += count;
            self.frame_id += 1;
            let screen = self.parser.screen();
            let (row, column) = screen.cursor_position();
            frames.push(json!({"frameId":self.frame_id,"rawOffset":self.offset,
                "rows":screen.size().0,"columns":screen.size().1,"screen":screen.contents(),
                "cursorRow":row,"cursorColumn":column}));
        }
        // A huge uncommitted update is not evidence of a visible frame.
        // Fail closed instead of retaining unbounded malformed provider/TTY data.
        if self.pending.len() > 8 * 1024 * 1024 {
            return json!({"error":"uncommitted terminal update exceeded validation bound"});
        }
        json!({"frames":frames})
    }
}

#[cfg(unix)]
fn formal_seal(database: &str, owner: &str, id: &str)
    -> Result<Option<nola_domain::assistant_run_terminal::AssistantRunTerminalSeal>,String> {
    // Connect only to the already-owned product Service. Unlike the production
    // lifecycle client, this read-only validator must not start/recover one.
    let socket = nola_service_client::socket_path_for_database(std::path::Path::new(database));
    let mut stream = std::os::unix::net::UnixStream::connect(socket).map_err(|e|e.to_string())?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(2))).map_err(|e|e.to_string())?;
    stream.set_write_timeout(Some(std::time::Duration::from_secs(2))).map_err(|e|e.to_string())?;
    let envelope = nola_protocol::CommandEnvelope {
        protocol_version:nola_protocol::APPLICATION_SERVICE_PROTOCOL_VERSION,
        request_id:format!("terminal-seal-{id}"), client_id:format!("terminal-validator-{}",std::process::id()),
        expected_revision:None, command:nola_protocol::CommandPayload {
            command_type:"local_assistant_run_terminal_seal".into(),
            arguments:json!({"conversation_id":owner,"run_id":id}),
        },
    };
    envelope.validate().map_err(|e|e.to_string())?;
    let mut request = serde_json::to_vec(&envelope).map_err(|e|e.to_string())?;
    request.push(b'\n');
    stream.write_all(&request).and_then(|_|stream.flush()).map_err(|e|e.to_string())?;
    let limit = nola_protocol::APPLICATION_SERVICE_MAX_FRAME_BYTES;
    let mut response = Vec::new();
    io::BufReader::new(stream).take((limit+1) as u64).read_until(b'\n',&mut response).map_err(|e|e.to_string())?;
    if response.len() > limit || response.last() != Some(&b'\n') {
        return Err("invalid/oversize formal Service response".into());
    }
    let receipt:nola_protocol::CommandReceipt = serde_json::from_slice(&response).map_err(|e|e.to_string())?;
    receipt.validate().map_err(|e|e.to_string())?;
    if receipt.request_id != envelope.request_id || receipt.client_id != envelope.client_id {
        return Err("formal Service receipt identity mismatch".into());
    }
    match receipt.outcome {
        nola_protocol::CommandReceiptOutcome::Committed { result,.. } =>
            serde_json::from_value(result).map_err(|e|e.to_string()),
        _ => Err("formal Service read was not committed".into()),
    }
}

#[cfg(not(unix))]
fn formal_seal(_: &str, _: &str, _: &str)
    -> Result<Option<nola_domain::assistant_run_terminal::AssistantRunTerminalSeal>,String> {
    Err("real PTY validation requires Unix".into())
}

fn audit(database: &str, live: bool) -> Result<Value, String> {
    let connection = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| e.to_string())?;
    connection.execute_batch("BEGIN").map_err(|e| e.to_string())?;
    let mut query = connection.prepare("SELECT r.id,r.conversation_id,r.status,s.seal_json,v.result_json
        FROM runs r LEFT JOIN assistant_run_terminal_seals s ON s.run_id=r.id
        LEFT JOIN run_results v ON v.run_id=r.id
        WHERE NOT EXISTS(SELECT 1 FROM runtime_task_run_bindings b WHERE b.run_id=r.id)
        ORDER BY r.created_at_ms,r.id").map_err(|e| e.to_string())?;
    let records = query.query_map([], |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,
        row.get::<_,String>(2)?,row.get::<_,Option<String>>(3)?,row.get::<_,Option<String>>(4)?)))
        .map_err(|e| e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e| e.to_string())?;
    let mut runs = Vec::new();
    for (id, owner, status, seal_json, result_json) in records {
        if status != "sealed" {
            if seal_json.is_some() { return Err("unsealed Run has a terminal seal".into()); }
            runs.push(json!({"runId":id,"conversationId":owner,"status":status,"kind":null}));
            continue;
        }
        let seal: nola_domain::assistant_run_terminal::AssistantRunTerminalSeal =
            serde_json::from_str(seal_json.as_deref().ok_or("sealed Run has no seal")?).map_err(|e| e.to_string())?;
        let result: nola_domain::run_result::RunResult =
            serde_json::from_str(result_json.as_deref().ok_or("sealed Run has no result")?).map_err(|e| e.to_string())?;
        seal.validate()?;
        result.validate()?;
        if seal.identity.run_id != id || seal.identity.conversation_id != owner || seal.result != result
            || seal.result_digest != result.computed_digest()? {
            return Err("Run terminal receipt ownership/result mismatch".into());
        }
        if live {
            let formal = formal_seal(database,&owner,&id)?;
            if formal.as_ref() != Some(&seal) { return Err("Service terminal read disagrees with durable receipt".into()); }
        }
        let encoded = serde_json::to_value(&result).map_err(|e| e.to_string())?;
        runs.push(json!({"runId":id,"conversationId":owner,"status":status,
            "kind":encoded["value"]["kind"],"sealDigest":seal.seal_digest,"resultDigest":seal.result_digest}));
    }
    Ok(json!({"runs":runs,"formalServiceRead":live}))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut terminal = Terminal::new();
    let mut output = io::BufWriter::new(io::stdout().lock());
    for line in io::stdin().lock().lines() {
        let result = (|| -> Result<Value, String> {
            let request: Request = serde_json::from_str(&line.map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
            match request {
                Request::Feed { bytes_base64 } => Ok(terminal.feed(&base64::engine::general_purpose::STANDARD
                    .decode(bytes_base64).map_err(|e|e.to_string())?)),
                Request::Resize { rows, columns } => {
                    if rows == 0 || columns == 0 || rows > 200 || columns > 500 {
                        return Err("invalid terminal dimensions".into());
                    }
                    terminal.parser.set_size(rows, columns);
                    Ok(json!({"resized":true}))
                }
                Request::Audit { database, live } => audit(&database, live),
            }
        })();
        serde_json::to_writer(&mut output, &result.unwrap_or_else(|error|json!({"error":error})))?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn erased_text_inside_a_sync_frame_is_not_a_visible_witness() {
        let mut terminal = Terminal::new();
        let result = terminal.feed(b"\x1b[?1049h\x1b[?2026hFAKE-SUCCESS\r\x1b[2Kactual\x1b[?2026l");
        assert_eq!(result["frames"].as_array().unwrap().len(),1);
        assert_eq!(result["frames"][0]["screen"],"actual");
    }

    #[test]
    fn split_commit_retains_utf8_without_admitting_an_open_frame() {
        let mut terminal = Terminal::new();
        let mut first = b"\x1b[?2026h".to_vec();
        first.extend_from_slice("中文".as_bytes());
        first.extend_from_slice(b"\x1b[?20");
        assert!(terminal.feed(&first)["frames"].as_array().unwrap().is_empty());
        let result = terminal.feed(b"26l\x1b[?2026hUNCOMMITTED");
        assert_eq!(result["frames"].as_array().unwrap().len(),1);
        assert_eq!(result["frames"][0]["screen"],"中文");
    }
}
