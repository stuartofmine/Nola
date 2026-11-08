use nola_application::{
    execute_application_command, ApplicationCommand, ConversationPlan, ConversationPlanCommand,
    ConversationPlanDiff, ConversationPlanRevision, ConversationPlanSopIdentity,
    ConversationPlanStepEdit,
};
use nola_store::NolaStore;
use serde_json::{json, Value};
use tempfile::tempdir;

fn steps(inspect_content: &str, write_content: &str) -> Value {
    json!([
        {
            "id": "inspect",
            "content": inspect_content,
            "activeForm": "Inspecting the workspace",
            "status": "pending",
            "priority": "high",
            "dependencyIds": [],
            "approvalStatus": "not_required",
            "capabilityRefs": ["builtin:read_file"],
            "expectedArtifactRefs": [],
            "successCriteria": ["Relevant files are identified"],
            "evidenceRefs": [],
            "verificationStatus": "pending"
        },
        {
            "id": "write",
            "content": write_content,
            "activeForm": "Writing the result",
            "status": "pending",
            "priority": "medium",
            "dependencyIds": ["inspect"],
            "approvalStatus": "not_required",
            "capabilityRefs": ["builtin:write_file"],
            "expectedArtifactRefs": ["file:result"],
            "successCriteria": ["The result exists"],
            "evidenceRefs": [],
            "verificationStatus": "pending"
        }
    ])
}

fn execute(store: &mut NolaStore, command: ConversationPlanCommand) -> Result<Value, String> {
    execute_application_command(store, ApplicationCommand::ConversationPlan { command })
}

fn sop(run_id: &str, version: u32) -> ConversationPlanSopIdentity {
    ConversationPlanSopIdentity {
        run_id: run_id.to_string(),
        sop_id: "test.plan".to_string(),
        sop_version: version,
    }
}

#[test]
fn edit_freeze_replan_history_and_diff_survive_cold_reopen() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("plan-lifecycle.db");

    {
        let mut store = NolaStore::open(&database).unwrap();
        let saved: ConversationPlan = serde_json::from_value(
            execute(
                &mut store,
                ConversationPlanCommand::Save {
                    expected_revision: None,
                    conversation_id: "conversation-1".to_string(),
                    sop: sop("run-1", 1),
                    todos: steps("Inspect the workspace", "Write the result"),
                    updated_at_ms: 100,
                },
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(saved.revision, 1);
        assert_eq!(saved.frozen_revision, None);

        let edited: ConversationPlan = serde_json::from_value(
            execute(
                &mut store,
                ConversationPlanCommand::EditStep {
                    conversation_id: "conversation-1".to_string(),
                    expected_revision: 1,
                    step_id: "inspect".to_string(),
                    edit: ConversationPlanStepEdit {
                        content: Some("Inspect the authoritative workspace".to_string()),
                        ..ConversationPlanStepEdit::default()
                    },
                    updated_at_ms: 200,
                },
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(edited.revision, 2);
        assert_eq!(
            edited.todos[0].content,
            "Inspect the authoritative workspace"
        );

        let frozen: ConversationPlan = serde_json::from_value(
            execute(
                &mut store,
                ConversationPlanCommand::Freeze {
                    conversation_id: "conversation-1".to_string(),
                    expected_revision: 2,
                    updated_at_ms: 300,
                },
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(frozen.revision, 3);
        assert_eq!(frozen.frozen_revision, Some(2));

        let implicit_edit = execute(
            &mut store,
            ConversationPlanCommand::EditStep {
                conversation_id: "conversation-1".to_string(),
                expected_revision: 3,
                step_id: "write".to_string(),
                edit: ConversationPlanStepEdit {
                    content: Some("Silently replace the frozen contract".to_string()),
                    ..ConversationPlanStepEdit::default()
                },
                updated_at_ms: 400,
            },
        )
        .unwrap_err();
        assert!(implicit_edit.contains("frozen"));

        let implicit_save = execute(
            &mut store,
            ConversationPlanCommand::Save {
                expected_revision: Some(3),
                conversation_id: "conversation-1".to_string(),
                sop: sop("run-1", 1),
                todos: steps("Inspect something else", "Write something else"),
                updated_at_ms: 500,
            },
        )
        .unwrap_err();
        assert!(implicit_save.contains("explicit replan"));

        let identity_swap = execute(
            &mut store,
            ConversationPlanCommand::Save {
                expected_revision: Some(3),
                conversation_id: "conversation-1".to_string(),
                sop: sop("run-silent-swap", 2),
                todos: steps("Inspect the authoritative workspace", "Write the result"),
                updated_at_ms: 550,
            },
        )
        .unwrap_err();
        assert!(identity_swap.contains("SOP identity is immutable"));

        let stale_replan = execute(
            &mut store,
            ConversationPlanCommand::Replan {
                conversation_id: "conversation-1".to_string(),
                expected_revision: 2,
                sop: sop("run-2", 2),
                todos: steps(
                    "Inspect the authoritative workspace",
                    "Write the revised result",
                ),
                reason: "New evidence changes the output contract".to_string(),
                updated_at_ms: 600,
            },
        )
        .unwrap_err();
        assert!(stale_replan.contains("revision conflict"));

        let reused_run = execute(
            &mut store,
            ConversationPlanCommand::Replan {
                conversation_id: "conversation-1".to_string(),
                expected_revision: 3,
                sop: sop("run-1", 2),
                todos: steps(
                    "Inspect the authoritative workspace",
                    "Write the revised result",
                ),
                reason: "Attempt to reuse the prior run".to_string(),
                updated_at_ms: 650,
            },
        )
        .unwrap_err();
        assert!(reused_run.contains("new SOP run id"));

        let replanned: ConversationPlan = serde_json::from_value(
            execute(
                &mut store,
                ConversationPlanCommand::Replan {
                    conversation_id: "conversation-1".to_string(),
                    expected_revision: 3,
                    sop: sop("run-2", 2),
                    todos: steps(
                        "Inspect the authoritative workspace",
                        "Write the revised result",
                    ),
                    reason: "New evidence changes the output contract".to_string(),
                    updated_at_ms: 700,
                },
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(replanned.revision, 4);
        assert_eq!(replanned.sop, sop("run-2", 2));
        assert_eq!(replanned.frozen_revision, None);
        assert_eq!(replanned.todos[1].content, "Write the revised result");

        let diff: ConversationPlanDiff = serde_json::from_value(
            execute(
                &mut store,
                ConversationPlanCommand::Diff {
                    conversation_id: "conversation-1".to_string(),
                    from_revision: 2,
                    to_revision: 4,
                },
            )
            .unwrap(),
        )
        .unwrap();
        assert!(diff.added.is_empty());
        assert!(diff.removed.is_empty());
        assert_eq!(diff.changed.len(), 1);
        assert_eq!(diff.changed[0].id, "write");
        assert_eq!(diff.from_sop, sop("run-1", 1));
        assert_eq!(diff.to_sop, sop("run-2", 2));
    }

    let mut reopened = NolaStore::open(&database).unwrap();
    let loaded: Option<ConversationPlan> = serde_json::from_value(
        execute(
            &mut reopened,
            ConversationPlanCommand::Get {
                conversation_id: "conversation-1".to_string(),
            },
        )
        .unwrap(),
    )
    .unwrap();
    let loaded = loaded.unwrap();
    assert_eq!(loaded.revision, 4);
    assert_eq!(loaded.sop, sop("run-2", 2));
    assert_eq!(loaded.frozen_revision, None);

    let history: Vec<ConversationPlanRevision> = serde_json::from_value(
        execute(
            &mut reopened,
            ConversationPlanCommand::History {
                conversation_id: "conversation-1".to_string(),
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(history.len(), 4);
    assert_eq!(
        history
            .iter()
            .map(|revision| revision.change_kind.as_str())
            .collect::<Vec<_>>(),
        vec!["save", "edit", "freeze", "replan"]
    );
    assert_eq!(
        history[3].change_summary,
        "New evidence changes the output contract"
    );
    assert_eq!(history[0].sop, sop("run-1", 1));
    assert_eq!(history[3].sop, sop("run-2", 2));
}

#[test]
fn stage_artifacts_transactions_and_resume_point_survive_cold_reopen() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("plan-runtime.db");
    let runtime_steps = json!([
        {
            "id": "inspect",
            "content": "Inspect the workspace",
            "activeForm": "Inspecting the workspace",
            "status": "completed",
            "priority": "high",
            "dependencyIds": [],
            "approvalStatus": "not_required",
            "capabilityRefs": ["builtin:read_file"],
            "expectedArtifactRefs": ["artifact:inventory"],
            "artifactRefs": ["artifact:inventory"],
            "workStepTransactionRefs": ["work-step:inspect:1"],
            "attempt": 1,
            "executorRef": "host:cli",
            "resumeKey": "checkpoint:inspect:complete",
            "successCriteria": ["Relevant files are identified"],
            "evidenceRefs": ["evidence:inventory-verified"],
            "verificationStatus": "passed",
            "terminationReason": "success"
        },
        {
            "id": "write",
            "content": "Write the result",
            "activeForm": "Writing the result",
            "status": "in_progress",
            "priority": "medium",
            "dependencyIds": ["inspect"],
            "approvalStatus": "not_required",
            "capabilityRefs": ["builtin:write_file"],
            "expectedArtifactRefs": ["file:result"],
            "artifactRefs": [],
            "workStepTransactionRefs": ["work-step:write:1"],
            "attempt": 1,
            "executorRef": "host:cli",
            "resumeKey": "checkpoint:write:prepared",
            "successCriteria": ["The result exists"],
            "evidenceRefs": [],
            "verificationStatus": "pending"
        }
    ]);

    {
        let mut store = NolaStore::open(&database).unwrap();
        let saved: ConversationPlan = serde_json::from_value(
            execute(
                &mut store,
                ConversationPlanCommand::Save {
                    expected_revision: None,
                    conversation_id: "runtime-conversation".to_string(),
                    sop: sop("runtime-run-1", 1),
                    todos: runtime_steps,
                    updated_at_ms: 100,
                },
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(saved.todos[0].artifact_refs, vec!["artifact:inventory"]);
        assert_eq!(saved.todos[1].attempt, 1);
    }

    let mut reopened = NolaStore::open(&database).unwrap();
    let loaded: Option<ConversationPlan> = serde_json::from_value(
        execute(
            &mut reopened,
            ConversationPlanCommand::Get {
                conversation_id: "runtime-conversation".to_string(),
            },
        )
        .unwrap(),
    )
    .unwrap();
    let loaded = loaded.unwrap();
    assert_eq!(loaded.sop, sop("runtime-run-1", 1));
    assert_eq!(loaded.todos[0].artifact_refs, vec!["artifact:inventory"]);
    assert_eq!(
        loaded.todos[0].work_step_transaction_refs,
        vec!["work-step:inspect:1"]
    );
    assert_eq!(loaded.todos[1].executor_ref.as_deref(), Some("host:cli"));
    assert_eq!(
        loaded.todos[1].resume_key.as_deref(),
        Some("checkpoint:write:prepared")
    );
    assert_eq!(loaded.todos[1].attempt, 1);
}
