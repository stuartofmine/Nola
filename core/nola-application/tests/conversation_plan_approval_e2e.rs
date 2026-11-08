use nola_application::{
    execute_application_command, ApplicationCommand, ConversationPlan,
    ConversationPlanApprovalStatus, ConversationPlanCommand, ConversationPlanSopIdentity,
    ConversationPlanStepStatus,
};
use nola_store::NolaStore;
use serde_json::json;
use tempfile::tempdir;

fn plan_steps() -> serde_json::Value {
    json!([
        {
            "id": "inspect",
            "content": "Inspect the workspace",
            "activeForm": "Inspecting the workspace",
            "status": "pending",
            "priority": "high",
            "dependencyIds": [],
            "approvalStatus": "pending",
            "capabilityRefs": ["builtin:read_file"],
            "expectedArtifactRefs": [],
            "successCriteria": ["Relevant files are identified"],
            "evidenceRefs": [],
            "verificationStatus": "pending"
        },
        {
            "id": "write",
            "content": "Write the result",
            "activeForm": "Writing the result",
            "status": "pending",
            "priority": "medium",
            "dependencyIds": ["inspect"],
            "approvalStatus": "pending",
            "capabilityRefs": ["builtin:write_file"],
            "expectedArtifactRefs": ["file:result"],
            "successCriteria": ["The result exists"],
            "evidenceRefs": [],
            "verificationStatus": "pending"
        }
    ])
}

fn sop(run_id: &str) -> ConversationPlanSopIdentity {
    ConversationPlanSopIdentity {
        run_id: run_id.to_string(),
        sop_id: "test.plan".to_string(),
        sop_version: 1,
    }
}

#[test]
fn user_approval_is_revision_guarded_and_survives_cold_reopen() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("plan-approval.db");
    {
        let mut store = NolaStore::open(&database).unwrap();
        let saved = execute_application_command(
            &mut store,
            ApplicationCommand::ConversationPlan {
                command: ConversationPlanCommand::Save {
                    expected_revision: None,
                    conversation_id: "conversation-1".to_string(),
                    sop: sop("run-1"),
                    todos: plan_steps(),
                    updated_at_ms: 100,
                },
            },
        )
        .unwrap();
        let saved: ConversationPlan = serde_json::from_value(saved).unwrap();
        assert_eq!(saved.revision, 1);

        let approved = execute_application_command(
            &mut store,
            ApplicationCommand::ConversationPlan {
                command: ConversationPlanCommand::SetApproval {
                    conversation_id: "conversation-1".to_string(),
                    expected_revision: 1,
                    step_ids: vec!["inspect".to_string()],
                    decision: ConversationPlanApprovalStatus::Approved,
                    updated_at_ms: 200,
                },
            },
        )
        .unwrap();
        let approved: ConversationPlan = serde_json::from_value(approved).unwrap();
        assert_eq!(approved.revision, 2);
        assert_eq!(
            approved.todos[0].approval_status,
            ConversationPlanApprovalStatus::Approved
        );

        let stale = execute_application_command(
            &mut store,
            ApplicationCommand::ConversationPlan {
                command: ConversationPlanCommand::SetApproval {
                    conversation_id: "conversation-1".to_string(),
                    expected_revision: 1,
                    step_ids: vec!["write".to_string()],
                    decision: ConversationPlanApprovalStatus::Rejected,
                    updated_at_ms: 300,
                },
            },
        )
        .unwrap_err();
        assert!(stale.contains("revision conflict"));
    }

    let mut reopened = NolaStore::open(&database).unwrap();
    let loaded = execute_application_command(
        &mut reopened,
        ApplicationCommand::ConversationPlan {
            command: ConversationPlanCommand::Get {
                conversation_id: "conversation-1".to_string(),
            },
        },
    )
    .unwrap();
    let loaded: Option<ConversationPlan> = serde_json::from_value(loaded).unwrap();
    let loaded = loaded.unwrap();
    assert_eq!(loaded.revision, 2);
    assert_eq!(loaded.todos[0].status, ConversationPlanStepStatus::Pending);
    assert_eq!(
        loaded.todos[0].approval_status,
        ConversationPlanApprovalStatus::Approved
    );
    assert_eq!(
        loaded.todos[1].approval_status,
        ConversationPlanApprovalStatus::Pending
    );
}
