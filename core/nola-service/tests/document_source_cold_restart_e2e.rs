// SPDX-License-Identifier: Apache-2.0

use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use nola_application::{
    ApplicationCommand, DocumentSource, DocumentSourceCommand, DocumentSourceSummary,
};
use nola_service_client::ApplicationServiceClient;

struct Service(Child);

impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start_service(database: &std::path::Path, socket: &std::path::Path) -> Service {
    Service(
        Command::new(env!("CARGO_BIN_EXE_nola-service"))
            .args(["--database"])
            .arg(database)
            .args(["--socket"])
            .arg(socket)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start real Application Service"),
    )
}

fn ready(client: &ApplicationServiceClient, child: &mut Service) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if client.health().is_ok() {
            return;
        }
        assert!(child.0.try_wait().expect("inspect service").is_none());
        thread::sleep(Duration::from_millis(20));
    }
    panic!("Application Service did not become ready");
}

#[test]
fn document_source_replace_and_search_survive_real_service_cold_restart() {
    let directory = tempfile::tempdir().expect("temporary state");
    let database = directory.path().join("nola.db");
    let socket = directory.path().join("nola.sock");
    let client = ApplicationServiceClient::with_socket(&database, &socket);
    let mut first = start_service(&database, &socket);
    ready(&client, &mut first);

    let input = DocumentSource {
        id: "web-doc-report".into(),
        scope: "web:local".into(),
        uri: "file:///report.txt".into(),
        kind: "uploaded_document".into(),
        mime_type: Some("text/plain".into()),
        sha256: None,
        byte_count: Some(25),
        title: Some("Quarterly report".into()),
        text_content: Some("revenue grew through cloud".into()),
        metadata: serde_json::json!({"id": "report", "name": "Quarterly report"}),
        created_at_ms: 100,
        updated_at_ms: 100,
    };
    let saved: usize = client
        .execute(ApplicationCommand::DocumentSource {
            command: DocumentSourceCommand::Replace {
                scope: "web:local".into(),
                kind: "uploaded_document".into(),
                inputs: vec![input],
            },
        })
        .expect("replace document sources");
    assert_eq!(saved, 1);

    drop(first);
    let _ = std::fs::remove_file(&socket);
    let mut second = start_service(&database, &socket);
    ready(&client, &mut second);
    let matches: Vec<DocumentSourceSummary> = client
        .execute(ApplicationCommand::DocumentSource {
            command: DocumentSourceCommand::Search {
                scope: "web:local".into(),
                query: Some("cloud".into()),
                kind: Some("uploaded_document".into()),
                limit: 20,
            },
        })
        .expect("search restored document sources");
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].title.as_deref(), Some("Quarterly report"));

    drop(second);
}
