// SPDX-License-Identifier: Apache-2.0

use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use nola_application::{
    ApplicationCommand, ConversationPlan, ConversationPlanCommand, ConversationPlanSopIdentity,
    ConversationPlanStepStatus,
};
use nola_service_client::ApplicationServiceClient;

fn start_service(database: &std::path::Path, socket: &std::path::Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_nola-service"))
        .args(["--database"])
        .arg(database)
        .args(["--socket"])
        .arg(socket)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start real Application Service")
}

fn ready(client: &ApplicationServiceClient, child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if client.health().is_ok() {
            return;
        }
        assert!(child.try_wait().expect("inspect service").is_none());
        thread::sleep(Duration::from_millis(20));
    }
    panic!("Application Service did not become ready");
}

fn sop() -> ConversationPlanSopIdentity {
    ConversationPlanSopIdentity {
        run_id: "service-run-1".to_string(),
        sop_id: "test.service.plan".to_string(),
        sop_version: 1,
    }
}

fn plan_steps(status: &str) -> serde_json::Value {
    serde_json::json!([{
        "id": "inspect",
        "content": "Inspect",
        "activeForm": "Inspecting",
        "status": status,
        "priority": "medium",
        "dependencyIds": [],
        "approvalStatus": "not_required",
        "capabilityRefs": ["builtin:read_file"],
        "expectedArtifactRefs": [],
        "successCriteria": ["Inspection completes"],
        "evidenceRefs": if status == "completed" { serde_json::json!(["evidence:inspection"]) } else { serde_json::json!([]) },
        "verificationStatus": if status == "completed" { "passed" } else { "pending" },
        "terminationReason": if status == "completed" { serde_json::Value::String("success".to_string()) } else { serde_json::Value::Null }
    }])
}

#[test]
fn conversation_plan_survives_real_service_cold_restart() {
    let directory = tempfile::tempdir().expect("temporary state");
    let database = directory.path().join("nola.db");
    let socket = directory.path().join("nola.sock");
    let client = ApplicationServiceClient::with_socket(&database, &socket);
    let mut first = start_service(&database, &socket);
    ready(&client, &mut first);

    let first_plan: ConversationPlan = client
        .execute(ApplicationCommand::ConversationPlan {
            command: ConversationPlanCommand::Save {
                expected_revision: None,
                conversation_id: "conversation-plan".to_string(),
                sop: sop(),
                todos: plan_steps("in_progress"),
                updated_at_ms: 100,
            },
        })
        .expect("save plan");
    assert_eq!(first_plan.revision, 1);
    let second_plan: ConversationPlan = client
        .execute(ApplicationCommand::ConversationPlan {
            command: ConversationPlanCommand::Save {
                expected_revision: Some(first_plan.revision),
                conversation_id: "conversation-plan".to_string(),
                sop: sop(),
                todos: plan_steps("completed"),
                updated_at_ms: 200,
            },
        })
        .expect("update plan");
    assert_eq!(second_plan.revision, 2);

    first.kill().expect("kill first service");
    first.wait().expect("reap first service");
    let _ = std::fs::remove_file(&socket);
    let mut second = start_service(&database, &socket);
    ready(&client, &mut second);
    let restored: Option<ConversationPlan> = client
        .execute(ApplicationCommand::ConversationPlan {
            command: ConversationPlanCommand::Get {
                conversation_id: "conversation-plan".to_string(),
            },
        })
        .expect("restore plan");
    let restored = restored.expect("stored plan");
    assert_eq!(restored.revision, 2);
    assert_eq!(
        restored.todos[0].status,
        ConversationPlanStepStatus::Completed
    );
    assert_eq!(restored.sop, sop());

    second.kill().expect("kill second service");
    second.wait().expect("reap second service");
}
