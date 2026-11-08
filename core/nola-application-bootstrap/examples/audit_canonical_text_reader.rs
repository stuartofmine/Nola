// SPDX-License-Identifier: Apache-2.0
//! Audit-only bridge to the production Application canonical text range query.
//! The Python auditor supplies its sole-owned, consistent SQLite backup, never
//! the live product database. No service, effect worker, or model is started.

use std::io::{self, BufRead, Read, Write};
use std::path::{Path, PathBuf};

use nola_application::ApplicationCommand;
use nola_protocol::{ConversationRecallCommand, ConversationTextRange};

const MAXIMUM_REQUEST_BYTES: u64 = 4096;
const MAXIMUM_RESPONSE_BYTES: usize = 262144;

fn snapshot_path(arguments: impl Iterator<Item = String>) -> Result<PathBuf, &'static str> {
    let mut arguments = arguments;
    if arguments.next().as_deref() != Some("--snapshot-database") {
        return Err("audit reader requires --snapshot-database");
    }
    let path = PathBuf::from(arguments.next().ok_or("audit snapshot path is missing")?);
    if arguments.next().is_some()
        || path.file_name().and_then(|value| value.to_str()) != Some("snapshot.db")
        || !path
            .parent()
            .and_then(Path::file_name)
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.starts_with("nola-canonical-audit-"))
        || !path.is_file()
    {
        return Err("audit reader accepts only an existing private audit snapshot");
    }
    Ok(path)
}

fn read_query(
    application: &mut nola_application_bootstrap::ApplicationRuntime,
    request: ConversationRecallCommand,
) -> Result<ConversationTextRange, &'static str> {
    if !matches!(
        &request,
        ConversationRecallCommand::ReadConversationTextRange { .. }
    ) {
        return Err("audit reader accepts only canonical text range queries");
    }
    request
        .validate()
        .map_err(|_| "invalid canonical range request")?;
    let value = application
        .execute(ApplicationCommand::from(request.clone()))
        .map_err(|_| "Application canonical range rejected")?;
    let range: ConversationTextRange = serde_json::from_value(value)
        .map_err(|_| "invalid Application canonical range response")?;
    range
        .validate_for(&request)
        .map_err(|_| "Application canonical range response changed identity or bounds")?;
    Ok(range)
}

fn run() -> Result<(), &'static str> {
    let database = snapshot_path(std::env::args().skip(1))?;
    // This directory is created by the auditor with one owner and one process.
    // Opening the copied schema can never migrate or write the source database.
    let mut application = nola_application_bootstrap::open(&database)
        .map_err(|_| "cannot open current-schema audit snapshot")?;
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let stdout = io::stdout();
    let mut output = stdout.lock();
    loop {
        let mut frame = Vec::new();
        let length = (&mut input)
            .take(MAXIMUM_REQUEST_BYTES + 1)
            .read_until(b'\n', &mut frame)
            .map_err(|_| "cannot read canonical range query")?;
        if length == 0 {
            return Ok(());
        }
        if length as u64 > MAXIMUM_REQUEST_BYTES || frame.last() != Some(&b'\n') {
            return Err("canonical range request exceeds the frame budget");
        }
        let response = match serde_json::from_slice::<ConversationRecallCommand>(&frame)
            .map_err(|_| "invalid canonical range query")
            .and_then(|request| read_query(&mut application, request))
        {
            Ok(range) => serde_json::json!({"ok": true, "range": range}),
            Err(error) => serde_json::json!({"ok": false, "error": error}),
        };
        let mut encoded =
            serde_json::to_vec(&response).map_err(|_| "cannot encode canonical range response")?;
        if encoded.len() + 1 > MAXIMUM_RESPONSE_BYTES {
            return Err("canonical range response exceeds the frame budget");
        }
        encoded.push(b'\n');
        output
            .write_all(&encoded)
            .map_err(|_| "cannot write canonical range response")?;
        output
            .flush()
            .map_err(|_| "cannot flush canonical range response")?;
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("audit canonical reader: {error}");
        std::process::exit(2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_or_unscoped_database_is_never_opened() {
        for path in [
            "/tmp/nola-v4.db",
            "/tmp/snapshot.db",
            "/Users/example/.nola/nola-v4.db",
        ] {
            assert!(
                snapshot_path(["--snapshot-database".into(), path.into()].into_iter()).is_err()
            );
        }
    }

    #[test]
    fn canonical_range_query_is_strict_and_owner_bound() {
        let valid = r#"{"type":"read_conversation_text_range","conversation_id":"c","message_id":"m","part_id":"p","offset_bytes":0,"limit_bytes":32768}"#;
        let request: ConversationRecallCommand = serde_json::from_str(valid).unwrap();
        request.validate().unwrap();
        let invalid = valid.replace(
            "\"limit_bytes\":32768",
            "\"limit_bytes\":32768,\"readOnly\":true",
        );
        assert!(serde_json::from_str::<ConversationRecallCommand>(&invalid).is_err());
    }
}
