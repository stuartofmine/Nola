// SPDX-License-Identifier: Apache-2.0

use nola_application::ApplicationCommand;
#[path = "../../nola-application/tests/support/committed_usage.rs"]
mod committed_usage;
use nola_service_client::ApplicationServiceClient;
use nola_store::NolaStore;
use serde_json::Value;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Service(Child);

impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start(database: &Path, socket: &Path, client: &ApplicationServiceClient) -> Service {
    let mut service = Service(
        Command::new(env!("CARGO_BIN_EXE_nola-service"))
            .arg("--database")
            .arg(database)
            .arg("--socket")
            .arg(socket)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if client.health().is_ok() {
            return service;
        }
        assert!(
            service.0.try_wait().unwrap().is_none(),
            "service exited during startup"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("service startup timed out");
}

#[test]
fn context_usage_query_survives_real_service_restart_without_rewriting_checkpoint() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("nola.db");
    let socket = root.path().join("nola.sock");
    let mut store = NolaStore::open(&database).unwrap();
    committed_usage::seed(&mut store, 8192);
    let sql = rusqlite::Connection::open(&database).unwrap();
    sql.execute("UPDATE core_checkpoint_bodies SET checkpoint_json=json_set(checkpoint_json,'$.unrelated_history',?1) WHERE conversation_id='usage'",
        ["x".repeat(10_000_000)]).unwrap();
    drop(sql);
    let original = store.load_checkpoint("usage").unwrap().unwrap();
    drop(store);
    let client = ApplicationServiceClient::with_socket(&database, &socket);
    let query = || -> Value {
        client
            .execute(ApplicationCommand::LoadConversationContextUsageV1 {
                conversation_id: "usage".into(),
            })
            .unwrap()
    };
    let first = start(&database, &socket, &client);
    let usage = query();
    assert_eq!(usage["used_tokens"], 8_192);
    assert_eq!(usage["context_window_tokens"], 65_536);
    assert_eq!(usage["prompt_token_limit"], 57_344);
    assert_eq!(usage["estimated"], false);
    assert!(usage.to_string().len() < 1024);
    drop(first);
    let _ = std::fs::remove_file(&socket);
    let second = start(&database, &socket, &client);
    assert_eq!(query(), usage);
    drop(second);
    let store = NolaStore::open(&database).unwrap();
    assert_eq!(store.load_checkpoint("usage").unwrap().unwrap(), original);
}
