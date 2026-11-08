// SPDX-License-Identifier: Apache-2.0

#[path = "../../nola-application/tests/support/model_identity.rs"]
mod model_identity;
#[path = "../../nola-application/tests/support/goal_execution.rs"]
mod goal_execution_fixture;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use nola_application::{
    AcceptedIdleSubmission, ApplicationCommand, CreateConversation, ReduceAndCommitIdleSubmission,
    SessionProjection,
};
use nola_core::agent::protocol::{AgentCoreConfig, AgentSubmission, SessionId};
use nola_core::agent::{
    GoalRecordStatus, GoalRequirement, GoalRuntimeCommand, GoalRuntimeCommandEnvelope, GoalStatus,
};
use nola_service_client::ApplicationServiceClient;

fn start_service(database: &std::path::Path, socket: &std::path::Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_nola-service"))
        .arg("--database")
        .arg(database)
        .arg("--socket")
        .arg(socket)
        .args(["--parent-pid", &std::process::id().to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start real Application Service")
}

fn wait_until_ready(client: &ApplicationServiceClient, child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if client.health().is_ok() {
            return;
        }
        if let Some(status) = child.try_wait().expect("inspect Application Service") {
            let stderr = child
                .stderr
                .take()
                .map(|mut pipe| {
                    use std::io::Read as _;
                    let mut value = String::new();
                    let _ = pipe.read_to_string(&mut value);
                    value
                })
                .unwrap_or_default();
            panic!("Application Service exited with {status}: {stderr}");
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("Application Service did not become ready");
}

fn goal_submission(operation_id: &str, command: GoalRuntimeCommand) -> AgentSubmission {
    AgentSubmission::GoalRuntime {
        envelope: GoalRuntimeCommandEnvelope {
            operation_id: operation_id.to_string(),
            command,
        },
    }
}

fn commit(
    client: &ApplicationServiceClient,
    conversation_id: &str,
    operation_id: &str,
    command: GoalRuntimeCommand,
    now: i64,
) -> AcceptedIdleSubmission {
    if let GoalRuntimeCommand::Start {id,..}=&command {
        let mut config=goal_execution_fixture::config();
        config.session_id=Some(SessionId::from_string(conversation_id.to_string()));
        return client.execute(ApplicationCommand::StartGoalExecution {
            input:nola_application::goal_execution::StartGoalExecution {
                identity:nola_domain::goal_execution::GoalExecutionBindingIdentity {
                    conversation_id:conversation_id.into(),goal_id:id.clone(),operation_id:operation_id.into(),source_host_id:"host".into(),
                },
                template:nola_application::goal_execution::GoalExecutionTemplate {
                    model_selection:nola_domain::goal_execution::GoalModelSelection::DirectRoute,core_config:config,
                },
                envelope:GoalRuntimeCommandEnvelope {operation_id:operation_id.into(),command},
            },
        }).expect("atomic Goal Start through real service");
    }
    client
        .execute(ApplicationCommand::ReduceAndCommitIdleSubmission {
            input: ReduceAndCommitIdleSubmission {
                conversation_id: conversation_id.to_string(),
                core_config: AgentCoreConfig {
                    turn_identity: model_identity::identity(),
                    session_id: Some(SessionId::from_string(conversation_id.to_string())),
                    ..AgentCoreConfig::default()
                },
                submission: goal_submission(operation_id, command),
                committed_at_ms: now,
            },
        })
        .expect("commit idle Goal transition through real service")
}

fn read_goal_status(client: &ApplicationServiceClient, conversation_id: &str) -> GoalStatus {
    let projection: Option<SessionProjection> = client.execute(ApplicationCommand::LoadSessionProjection {
        conversation_id: conversation_id.to_owned(),
    }).expect("read committed Goal projection independently from control acknowledgement");
    projection.unwrap().goal_projection.active.unwrap().status
}

// Observe only: opening a NolaStore here would start another publisher and
// conceal a missing Service wake/recovery path. A commit ACK proves durable
// database state, not synchronous publication of the derived files.
fn publication_frontier(database: &std::path::Path) -> (i64, i64) {
    let db = rusqlite::Connection::open_with_flags(
        database, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ).unwrap();
    db.query_row(
        "SELECT target_generation,published_generation FROM work_archive_publication WHERE singleton=1",
        [], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap()
}

fn await_publication(database: &std::path::Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let (target, published) = publication_frontier(database);
        if target == published { return; }
        assert!(Instant::now() < deadline,
            "Service archive publisher did not settle: target={target}, published={published}");
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn goal_projection_survives_real_service_cold_restart() {
    let directory = tempfile::tempdir().expect("temporary service state");
    let database = directory.path().join("nola.db");
    let socket = directory.path().join("nola.sock");
    let conversation_id = "goal-cold-restart";
    let client = ApplicationServiceClient::with_socket(&database, &socket);

    let mut first_service = start_service(&database, &socket);
    wait_until_ready(&client, &mut first_service);
    let _: serde_json::Value = client
        .execute(ApplicationCommand::CreateConversation {
            input: CreateConversation {
                id: conversation_id.to_string(),
                title: "Cold restart Goal".to_string(),
                settings_json: None,
                created_at_ms: 100,
                updated_at_ms: 100,
            },
        })
        .expect("create canonical conversation");
    let started = commit(
        &client,
        conversation_id,
        "goal:start",
        GoalRuntimeCommand::Start {
            id: "goal-1".to_string(),
            objective: "finish the release".to_string(),
            constraints: vec!["preserve canonical history".to_string()],
            requirements: vec![GoalRequirement {
                acceptance_policy: nola_core::agent::goal::GoalAcceptancePolicy::ModelJudgment,
                semantic_revision: 1, id: "requirement-1".to_string(),
                description: "cold restart succeeds".to_string(),
                dependencies: Vec::new(),
                status: GoalRecordStatus::Pending,
                evidence_refs: Vec::new(),
            }],
            now_epoch_ms: 200,
        },
        200,
    );
    assert_eq!(
        read_goal_status(&client, conversation_id),
        GoalStatus::Running
    );
    commit(&client, conversation_id, "goal:work", GoalRuntimeCommand::UpdateWork {
        goal_id: "goal-1".into(), expected_revision: 1, now_epoch_ms: 250,
        entries: vec![serde_json::from_value(serde_json::json!({
            "id":"next-validation","kind":"next_action","origin":"model",
            "source_ref":"turn:checkpoint","content":"Verify the preserved data after restart",
            "requirement_ids":["requirement-1"],"task_ids":[],"evidence_refs":[],
            "progress":null,"supersedes":null
        })).unwrap()],
    }, 250);
    let paused = commit(
        &client,
        conversation_id,
        "goal:pause",
        GoalRuntimeCommand::Pause {
            status: GoalStatus::Paused,
            wake_at_epoch_ms: None,
            now_epoch_ms: 300,
        },
        300,
    );
    assert_eq!(
        read_goal_status(&client, conversation_id),
        GoalStatus::Paused
    );
    await_publication(&database);
    first_service.kill().expect("stop first service process");
    first_service.wait().expect("reap first service process");
    // Read files directly before any Store reopen can retry an export.
    let archive_root = std::path::PathBuf::from(format!("{}.work",database.display()));
    let archive_dir = std::fs::read_dir(&archive_root).unwrap().map(|entry|entry.unwrap().path())
        .find(|path|path.is_dir()).expect("service published a work archive");
    let archive = nola_store::work_archive::read_verified(&archive_dir).unwrap();
    let archived: serde_json::Value = serde_json::from_str(&archive.recovery.checkpoint.checkpoint_json).unwrap();
    assert_eq!(archived["state"]["goal_projection"]["active"]["status"],"paused");
    assert_eq!(archived["state"]["goal_projection"]["active"]["stop_reason"],"suspended");
    assert!(String::from_utf8_lossy(&archive.files["goal.md"]).contains("Verify the preserved data after restart"));
    let _ = std::fs::remove_file(&socket);

    let mut second_service = start_service(&database, &socket);
    wait_until_ready(&client, &mut second_service);
    let before_claim: Option<SessionProjection> = client.execute(
        ApplicationCommand::LoadSessionProjection { conversation_id: conversation_id.into() }
    ).expect("read paused Goal after restart");
    let before_goal = before_claim.unwrap().goal_projection;
    // New transport request/time after a real service restart: semantic replay
    // must return the current projection, not execute the old Pause again.
    let replay = commit(&client, conversation_id, "goal:pause", GoalRuntimeCommand::Pause {
        status:GoalStatus::Paused,wake_at_epoch_ms:None,now_epoch_ms:300,
    }, 340);
    let after_replay: Option<SessionProjection> = client.execute(
        ApplicationCommand::LoadSessionProjection { conversation_id: conversation_id.into() }
    ).expect("read complete Goal projection after semantic replay");
    assert_eq!(after_replay.unwrap().goal_projection, before_goal);
    assert!(replay.events.is_empty());
    let rejected: Result<AcceptedIdleSubmission, _> = client.execute(
        ApplicationCommand::ReduceAndCommitIdleSubmission {
            input: ReduceAndCommitIdleSubmission {
                conversation_id: conversation_id.into(),
                core_config: AgentCoreConfig {
                    turn_identity: model_identity::identity(),
                    session_id: Some(SessionId::from_string(conversation_id.into())),
                    ..AgentCoreConfig::default()
                },
                submission: goal_submission("goal:claim-without-resume", GoalRuntimeCommand::Claim {
                    owner: "scheduled-worker".into(), lease_until_epoch_ms: 1_000,
                    now_epoch_ms: 350,
                }),
                committed_at_ms: 350,
            },
        }
    );
    let error = rejected.err().expect("a worker cannot silently resume a suspended Goal");
    assert!(error.to_string().contains("Goal requires explicit resume before claim"));
    let after_claim: Option<SessionProjection> = client.execute(
        ApplicationCommand::LoadSessionProjection { conversation_id: conversation_id.into() }
    ).expect("read canonical Goal after rejected claim");
    assert_eq!(after_claim.unwrap().goal_projection, before_goal,
        "a rejected claim must not mutate the persisted Goal or its revision");
    // Deliberately prevent publication while the second process commits. The
    // third process must recover the durable backlog without another mutation.
    let publication_lock = std::fs::OpenOptions::new().read(true).write(true)
        .open(archive_root.join("publish.lock")).unwrap();
    publication_lock.lock().unwrap();
    let resumed = commit(
        &client,
        conversation_id,
        "goal:resume",
        GoalRuntimeCommand::Resume { now_epoch_ms: 400 },
        400,
    );
    assert_eq!(
        read_goal_status(&client, conversation_id),
        GoalStatus::Running
    );
    assert!(resumed.execution.control.revision > paused.execution.control.revision);
    assert!(started.execution.has_active_goal);

    let projection: Option<SessionProjection> = client
        .execute(ApplicationCommand::LoadSessionProjection {
            conversation_id: conversation_id.to_string(),
        })
        .expect("load canonical projection after cold restart");
    let goal = projection
        .expect("session projection")
        .goal_projection
        .active
        .expect("active Goal projection");
    assert_eq!(goal.status, GoalStatus::Running);
    assert_eq!(goal.objective, "finish the release");
    assert_eq!(goal.work.current().len(), 1);
    assert_eq!(goal.work.current()[0].content, "Verify the preserved data after restart");
    assert_eq!(goal.constraints, vec!["preserve canonical history"]);

    let (target, published) = publication_frontier(&database);
    assert!(target > published, "the held lock must leave a durable unpublished generation");

    second_service.kill().expect("stop second service process");
    second_service.wait().expect("reap second service process");
    drop(publication_lock);
    let _ = std::fs::remove_file(&socket);
    let mut third_service = start_service(&database, &socket);
    wait_until_ready(&client, &mut third_service);
    await_publication(&database);
    let restored = nola_store::work_archive::read_verified(&archive_dir).unwrap();
    let restored_checkpoint: serde_json::Value =
        serde_json::from_str(&restored.recovery.checkpoint.checkpoint_json).unwrap();
    let restored_goal = &restored_checkpoint["state"]["goal_projection"]["active"];
    assert_eq!(restored_goal["status"], "running");
    assert_eq!(restored_goal["objective"], goal.objective);
    assert_eq!(restored_goal["constraints"], serde_json::json!(goal.constraints));
    assert!(String::from_utf8_lossy(&restored.files["goal.md"])
        .contains("Verify the preserved data after restart"));
    assert_eq!(publication_frontier(&database), (target, target),
        "recovery publishes the original generation without a new Goal command");
    third_service.kill().expect("stop third service process");
    third_service.wait().expect("reap third service process");
}
