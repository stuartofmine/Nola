// SPDX-License-Identifier: Apache-2.0

use nola_application::{ApplicationCommand, ProductSetting};
use nola_service_client::ApplicationServiceClient;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

fn start(database: &std::path::Path, socket: &std::path::Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_nola-service"))
        .args(["--database"])
        .arg(database)
        .args(["--socket"])
        .arg(socket)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

fn ready(client: &ApplicationServiceClient, child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if client.health().is_ok() {
            return;
        }
        assert!(child.try_wait().unwrap().is_none());
        thread::sleep(Duration::from_millis(20));
    }
    panic!("Application Service did not become ready");
}

#[test]
fn product_setting_revision_and_delete_survive_real_service_restarts() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("nola.db");
    let socket = directory.path().join("nola.sock");
    let client = ApplicationServiceClient::with_socket(&database, &socket);

    let mut first = start(&database, &socket);
    ready(&client, &mut first);
    let initial: ProductSetting = client
        .execute(ApplicationCommand::PutProductSetting {
            key: "web.agent.settings".into(),
            value_json: r#"{"model":"first"}"#.into(),
            updated_at_ms: 10,
        })
        .unwrap();
    assert_eq!(initial.revision, 1);
    first.kill().unwrap();
    first.wait().unwrap();
    let _ = std::fs::remove_file(&socket);

    let mut second = start(&database, &socket);
    ready(&client, &mut second);
    let restored: Option<ProductSetting> = client
        .execute(ApplicationCommand::GetProductSetting {
            key: "web.agent.settings".into(),
        })
        .unwrap();
    assert_eq!(restored.unwrap().value_json, r#"{"model":"first"}"#);
    let revised: ProductSetting = client
        .execute(ApplicationCommand::PutProductSetting {
            key: "web.agent.settings".into(),
            value_json: r#"{"model":"second"}"#.into(),
            updated_at_ms: 20,
        })
        .unwrap();
    assert_eq!(revised.revision, 2);
    let deleted: bool = client
        .execute(ApplicationCommand::DeleteProductSetting {
            key: "web.agent.settings".into(),
        })
        .unwrap();
    assert!(deleted);
    second.kill().unwrap();
    second.wait().unwrap();
    let _ = std::fs::remove_file(&socket);

    let mut third = start(&database, &socket);
    ready(&client, &mut third);
    let missing: Option<ProductSetting> = client
        .execute(ApplicationCommand::GetProductSetting {
            key: "web.agent.settings".into(),
        })
        .unwrap();
    assert!(missing.is_none());
    third.kill().unwrap();
    third.wait().unwrap();
}
