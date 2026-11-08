// SPDX-License-Identifier: Apache-2.0
//! Query orchestration for an already admitted language-service process.
//! No configuration lookup, file reads, command launch or authorization here.

use std::path::Path;
use std::time::Instant;
use serde_json::{json, Value};
use super::language_service_document::Document;
use super::language_service_protocol::{Incoming, Protocol, Query};
use super::language_service_transport::{Cancellation, Transport};
use crate::task_runtime::OwnedShellChild;

pub(super) struct Session {
    protocol: Protocol,
    transport: Transport,
    document: Document,
    supporting_documents: Vec<Document>,
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use std::process::{Command, Stdio};
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    #[ignore = "requires an explicit NOLA_TEST_CLANGD executable; runs only against a temporary project"]
    fn real_language_server_queries_snapshot_and_shuts_down() {
        let executable = std::env::var_os("NOLA_TEST_CLANGD").expect("set NOLA_TEST_CLANGD");
        assert!(Path::new(&executable).is_absolute());
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().canonicalize().unwrap();
        let path = root.join("example.cpp");
        let header = root.join("shared.hpp");
        let header_text = "inline int shared_answer() { return 19; }\n";
        std::fs::write(&header, header_text).unwrap();
        let text = "int answer() { return 42; }\nint main() { return answer(); }\n#include \"shared.hpp\"\nint cross_file() { return shared_answer(); }\n";
        std::fs::write(&path, text).unwrap();
        let mut command = Command::new(executable);
        command.args(["--background-index=false", "--clang-tidy=false", "--log=error"])
            .env_clear().env("HOME", &root).current_dir(&root)
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        let child = OwnedShellChild::spawn_owned(&mut command).unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut session = Session::connect(child, &root, &path, "cpp", text.into(), deadline,
            Arc::new(|| false)).unwrap();
        // Exercise a cold workspace query first: earlier document-scoped
        // requests must not accidentally hide the didOpen/index race.
        let workspace = session.query(Query::WorkspaceSymbols, 1, 1, Some("answer"), deadline,
            || Ok(text.into())).unwrap();
        assert!(workspace["result"].as_array().unwrap().iter().any(|symbol| symbol["name"] == "answer"),
            "cold workspace query missed the opened document: {workspace}");
        let symbols = session.query(Query::DocumentSymbols, 1, 1, None, deadline,
            || Ok(std::fs::read_to_string(&path).unwrap())).unwrap();
        assert!(symbols["result"].as_array().unwrap().iter().any(|symbol| symbol["name"] == "answer"));
        assert_eq!(symbols["code_evidence"]["backend"], "language_service");
        assert_eq!(symbols["workspace_snapshot_verified"], false);
        let definition = session.query(Query::Definition, 2, 22, None, deadline,
            || Ok(text.into())).unwrap();
        assert!(!definition["result"].is_null());
        assert_ne!(definition["result"], json!([]));
        session.open_supporting(&header, "cpp", header_text.into(), deadline).unwrap();
        let references = session.query(Query::References, 4, 27, None, deadline, || Ok(text.into())).unwrap();
        let targets = references["result"].as_array().unwrap();
        let header_uri = url::Url::from_file_path(&header).unwrap().to_string();
        assert!(targets.iter().any(|item| item["uri"] == header_uri && item["range"]["start"]["line"] == 0), "{references}");
        assert!(targets.iter().any(|item| item["uri"] == url::Url::from_file_path(&path).unwrap().as_str()
            && item["range"]["start"]["line"] == 3), "{references}");
        assert_eq!(references["opened_documents"].as_array().unwrap().len(), 2);
        assert_eq!(references["workspace_snapshot_verified"], false);
        let stale = session.query(Query::DocumentSymbols, 1, 1, None, deadline,
            || Ok("changed after submission".into())).unwrap_err();
        assert!(stale.contains("stale"));
        session.close(deadline).unwrap();
    }
}

impl Session {
    pub(super) fn connect(
        child: OwnedShellChild,
        root: &Path,
        path: &Path,
        language_id: &str,
        text: String,
        deadline: Instant,
        cancelled: Cancellation,
    ) -> Result<Self, String> {
        let root_uri = url::Url::from_directory_path(root).map_err(|_| "language service root must be absolute")?;
        let (document, open) = Document::open(path, language_id, text)?;
        let mut session = Self {
            protocol: Protocol::default(),
            transport: Transport::attach(child, deadline, cancelled).map_err(|error| error.to_string())?,
            document,
            supporting_documents: Vec::new(),
        };
        let initialize = session.protocol.initialize(root_uri.as_str())?;
        session.send(&initialize, deadline)?;
        let Incoming::Initialized(notification) = session.await_response(deadline)? else {
            return Err("language service did not initialize".into());
        };
        session.send(&notification, deadline)?;
        session.protocol.require_document_sync()?;
        session.send(&open, deadline)?;
        Ok(session)
    }

    fn send(&mut self, message: &Value, deadline: Instant) -> Result<(), String> {
        self.transport.send(message, deadline).map_err(|error| error.to_string())
    }

    fn await_response(&mut self, deadline: Instant) -> Result<Incoming, String> {
        // Server notifications may be unsolicited. Bound their total per
        // request, not only each frame's size; never inject them into prompts.
        for _ in 0..4096 {
            let message = self.transport.receive(deadline).map_err(|error| error.to_string())?;
            match self.protocol.receive(message)? {
                Incoming::Reply(reply) => self.send(&reply, deadline)?,
                Incoming::Notification => {},
                Incoming::Failure(error) => return Err(format!("language service query failed: {error}")),
                response => return Ok(response),
            }
        }
        self.protocol.abort();
        Err("language service unsolicited-message limit exceeded".into())
    }

    pub(super) fn update(&mut self, text: String, deadline: Instant) -> Result<(), String> {
        if let Some(message) = self.document.change(text)? {
            self.send(&message, deadline)?;
        }
        Ok(())
    }

    /// Open only caller-authorized snapshots. Synchronize each document before
    /// querying an index; no inference about unopened workspace files is made.
    pub(super) fn open_supporting(&mut self, path: &Path, language: &str, text: String,
        deadline: Instant) -> Result<(), String> {
        let (document, open) = Document::open(path, language, text)?;
        if !self.protocol.supports(Query::DocumentSymbols) {
            return Err("language service cannot synchronize supporting documents".into());
        }
        let uri = document.identity().uri;
        if uri == self.document.identity().uri || self.supporting_documents.iter().any(|item| item.identity().uri == uri) {
            return Err("duplicate language service document".into());
        }
        self.send(&open, deadline)?;
        self.supporting_documents.push(document);
        let barrier = self.protocol.query(Query::DocumentSymbols, json!({"textDocument":{"uri":uri}}))?;
        self.send(&barrier, deadline)?;
        if !matches!(self.await_response(deadline)?, Incoming::Result(_)) {
            return Err("unexpected supporting document synchronization response".into());
        }
        Ok(())
    }

    /// The caller supplies a fresh authorized read after the response so
    /// filesystem changes during the request cannot silently appear current.
    pub(super) fn query(
        &mut self,
        operation: Query,
        line: u32,
        character: u32,
        symbol_query: Option<&str>,
        deadline: Instant,
        current_text: impl FnOnce() -> Result<String, String>,
    ) -> Result<Value, String> {
        let snapshot = self.document.identity();
        // Workspace queries may read the index immediately, unlike document
        // queries which wait for the opened document's parse. Synchronize that
        // document first when supported; this is not a whole-workspace indexing
        // guarantee and must not replace the actual workspace query result.
        if operation == Query::WorkspaceSymbols && self.protocol.supports(Query::DocumentSymbols) {
            let barrier = self.protocol.query(Query::DocumentSymbols,
                json!({"textDocument":{"uri":snapshot.uri}}))?;
            self.send(&barrier, deadline)?;
            if !matches!(self.await_response(deadline)?, Incoming::Result(_)) {
                return Err("unexpected document synchronization response phase".into());
            }
        }
        let mut params = match operation {
            Query::WorkspaceSymbols => json!({"query":symbol_query.ok_or("workspace symbol query is required")?}),
            Query::DocumentSymbols => json!({"textDocument":{"uri":snapshot.uri}}),
            _ => json!({"textDocument":{"uri":snapshot.uri}, "position":self.document.position(line, character)?}),
        };
        if operation == Query::References { params["context"] = json!({"includeDeclaration":true}); }
        let request = self.protocol.query(operation, params)?;
        self.send(&request, deadline)?;
        let Incoming::Result(result) = self.await_response(deadline)? else {
            return Err("unexpected language service response phase".into());
        };
        if !self.document.matches(&snapshot, &current_text()?) {
            return Err("language service response is stale: queried document changed".into());
        }
        let mut documents = vec![self.document.evidence()];
        documents.extend(self.supporting_documents.iter().map(Document::evidence));
        Ok(json!({"code_evidence":self.document.evidence(), "result":result,
            "opened_documents":documents,"index_scope":"server_index_plus_explicit_documents",
            "workspace_snapshot_verified":false}))
    }

    pub(super) fn close(&mut self, deadline: Instant) -> Result<(), String> {
        let protocol_result = (|| {
            for index in 0..self.supporting_documents.len() {
                self.send(&self.supporting_documents[index].close(), deadline)?;
            }
            self.send(&self.document.close(), deadline)?;
            let shutdown = self.protocol.shutdown()?;
            self.send(&shutdown, deadline)?;
            if !matches!(self.await_response(deadline)?, Incoming::Shutdown) {
                return Err("language service shutdown response mismatch".into());
            }
            self.send(&json!({"jsonrpc":"2.0", "method":"exit"}), deadline)
        })();
        // Always stop/reap, even when the graceful protocol failed. A failed
        // stop confirmation must remain explicit rather than claiming success.
        let stopped = self.transport.stop();
        match (protocol_result, stopped) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), Ok(())) => Err(error),
            (result, Err(error)) => Err(format!("language service stop unconfirmed: {error}; protocol: {result:?}")),
        }
    }
}
