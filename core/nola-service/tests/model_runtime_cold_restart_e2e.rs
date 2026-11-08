// SPDX-License-Identifier: Apache-2.0

use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use nola_application::{
    ApplicationCommand, ModelCommand, ModelProfile, ModelRuntime, ModelRuntimePhase,
};
use nola_service_client::ApplicationServiceClient;

struct OwnedService(Child);

impl std::ops::Deref for OwnedService {
    type Target = Child;
    fn deref(&self) -> &Child {
        &self.0
    }
}

impl std::ops::DerefMut for OwnedService {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.0
    }
}

impl Drop for OwnedService {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start_service(database: &std::path::Path, socket: &std::path::Path) -> OwnedService {
    OwnedService(
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

fn ready(client: &ApplicationServiceClient, child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if client.health().is_ok() {
            return;
        }
        assert!(
            child.try_wait().expect("inspect service").is_none(),
            "Application Service exited before readiness"
        );
        thread::sleep(Duration::from_millis(20));
    }
    panic!("Application Service did not become ready");
}

fn profile(id: &str, selected: bool, revision: i64, now: i64) -> ModelProfile {
    ModelProfile {
        id: id.to_string(),
        display_name: id.to_string(),
        provider: "local".to_string(),
        model_name: id.to_string(),
        runtime: "llama.cpp".to_string(),
        source_json: r#"{"repository":"publisher/model","revision":"immutable"}"#.to_string(),
        parameters_json: r#"{"contextTokens":65536,"temperature":0.7}"#.to_string(),
        selected,
        enabled: true,
        revision,
        created_at_ms: now,
        updated_at_ms: now,
        deleted_at_ms: None,
    }
}

#[test]
fn parameter_commands_preserve_canonical_state_and_rejections_across_real_service_restart() {
    let directory = tempfile::tempdir().expect("temporary state");
    let database = directory.path().join("nola.db");
    let socket = directory.path().join("nola.sock");
    let client = ApplicationServiceClient::with_socket(&database, &socket);
    let other_client = ApplicationServiceClient::with_socket(&database, &socket);
    let mut service = start_service(&database, &socket);
    ready(&client, &mut service);
    let execute =
        |command| client.execute::<serde_json::Value, _>(ApplicationCommand::Model { command });
    let stale = profile("model", false, 0, 100);
    execute(ModelCommand::UpsertProfile {
        input: stale.clone(),
    })
    .unwrap();
    let mut current = stale.clone();
    current.selected = true;
    current.display_name = "Updated by another client".into();
    current.source_json = r#"{"repository":"canonical/latest"}"#.into();
    current.updated_at_ms = 200;
    let _: serde_json::Value = other_client
        .execute(ApplicationCommand::Model {
            command: ModelCommand::UpsertProfile {
                input: current.clone(),
            },
        })
        .unwrap();
    let mut edit = stale;
    edit.parameters_json = r#"{"contextTokens":131072,"temperature":0.42}"#.into();
    edit.updated_at_ms = 300;
    execute(ModelCommand::ConfigureParameters {
        input: edit.clone(),
    })
    .unwrap();
    let configured: ModelProfile =
        serde_json::from_value(execute(ModelCommand::GetProfile { id: "model".into() }).unwrap())
            .unwrap();
    assert!(configured.selected);
    assert_eq!(configured.source_json, current.source_json);
    assert_eq!(configured.display_name, current.display_name);
    assert_eq!(configured.created_at_ms, 100);
    assert_eq!(configured.parameters_json, edit.parameters_json);
    assert!(configured.revision > current.revision);
    let snapshot = execute(ModelCommand::ListProfiles).unwrap();
    assert!(execute(ModelCommand::ResetParameters {
        profile_ids: vec!["model".into(), "missing".into()],
        parameters_json: "{}".into(),
        updated_at_ms: 400,
    })
    .is_err());
    assert_eq!(execute(ModelCommand::ListProfiles).unwrap(), snapshot);
    let mut foreign = profile("model", true, 0, 500);
    foreign.provider = "other-provider".into();
    assert!(execute(ModelCommand::UpsertProfile { input: foreign }).is_err());
    assert_eq!(execute(ModelCommand::ListProfiles).unwrap(), snapshot);
    drop(service);
    let _ = std::fs::remove_file(&socket);
    let mut service = start_service(&database, &socket);
    ready(&client, &mut service);
    assert_eq!(execute(ModelCommand::ListProfiles).unwrap(), snapshot);
    execute(ModelCommand::DeleteProfile {
        id: "model".into(),
        deleted_at_ms: 600,
    })
    .unwrap();
    assert!(execute(ModelCommand::ConfigureParameters { input: edit }).is_err());
    assert_eq!(
        execute(ModelCommand::ListProfiles).unwrap(),
        serde_json::json!([])
    );
}

#[test]
fn model_selection_and_runtime_reports_survive_service_restart_without_claiming_process_liveness() {
    let directory = tempfile::tempdir().expect("temporary state");
    let database = directory.path().join("nola.db");
    let socket = directory.path().join("nola.sock");
    let client = ApplicationServiceClient::with_socket(&database, &socket);
    let mut first = start_service(&database, &socket);
    ready(&client, &mut first);

    let _: serde_json::Value = client
        .execute(ApplicationCommand::Model {
            command: ModelCommand::UpsertProfile {
                input: profile("model-small", true, 1, 100),
            },
        })
        .expect("save first profile");
    let _: serde_json::Value = client
        .execute(ApplicationCommand::Model {
            command: ModelCommand::UpsertProfile {
                input: profile("model-large", true, 1, 200),
            },
        })
        .expect("select second profile");
    let profiles: Vec<ModelProfile> = client
        .execute(ApplicationCommand::Model {
            command: ModelCommand::ListProfiles,
        })
        .expect("list profiles");
    assert_eq!(
        profiles.iter().filter(|profile| profile.selected).count(),
        1
    );
    assert_eq!(profiles[0].id, "model-large");

    let _: serde_json::Value = client
        .execute(ApplicationCommand::Model {
            command: ModelCommand::ReportRuntime {
                input: ModelRuntime {
                    host_id: "host-a".to_string(),
                    profile_id: "model-large".to_string(),
                    phase: ModelRuntimePhase::Ready,
                    endpoint_json: Some(r#"{"baseUrl":"http://127.0.0.1:43123/v1"}"#.to_string()),
                    process_id: Some(1234),
                    error: None,
                    started_at_ms: Some(300),
                    stopped_at_ms: None,
                    updated_at_ms: 300,
                },
            },
        })
        .expect("report ready runtime");
    first.kill().expect("kill first service");
    first.wait().expect("reap first service");
    let _ = std::fs::remove_file(&socket);

    let mut second = start_service(&database, &socket);
    ready(&client, &mut second);
    let restored: Option<ModelRuntime> = client
        .execute(ApplicationCommand::Model {
            command: ModelCommand::GetRuntime {
                host_id: "host-a".to_string(),
            },
        })
        .expect("load runtime recovery hint");
    let restored = restored.expect("runtime record");
    assert_eq!(restored.phase, ModelRuntimePhase::Ready);
    assert_eq!(restored.process_id, Some(1234));

    let _: serde_json::Value = client
        .execute(ApplicationCommand::Model {
            command: ModelCommand::ReportRuntime {
                input: ModelRuntime {
                    host_id: "host-a".to_string(),
                    profile_id: "model-large".to_string(),
                    phase: ModelRuntimePhase::Stopped,
                    endpoint_json: None,
                    process_id: None,
                    error: Some("cold-start probe did not find the prior process".to_string()),
                    started_at_ms: Some(300),
                    stopped_at_ms: Some(400),
                    updated_at_ms: 400,
                },
            },
        })
        .expect("commit stopped result after Host probe");
    let stopped: Option<ModelRuntime> = client
        .execute(ApplicationCommand::Model {
            command: ModelCommand::GetRuntime {
                host_id: "host-a".to_string(),
            },
        })
        .expect("reload stopped runtime");
    assert_eq!(stopped.unwrap().phase, ModelRuntimePhase::Stopped);

    second.kill().expect("kill second service");
    second.wait().expect("reap second service");
}
