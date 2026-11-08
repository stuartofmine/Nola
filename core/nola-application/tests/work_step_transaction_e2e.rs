use nola_application::{
    execute_application_command, ApplicationCommand, ConversationPlan, ConversationPlanCommand,
    ConversationPlanRevision, ConversationPlanSopIdentity, WorkStepTransaction,
    WorkStepTransactionCommand, WorkStepTransactionPhase, WorkStepTransactionPreparation,
};
use nola_store::NolaStore;
use serde_json::{json, Value};
use tempfile::tempdir;

fn execute(store: &mut NolaStore, command: WorkStepTransactionCommand) -> Result<Value, String> {
    execute_application_command(store, ApplicationCommand::WorkStepTransaction { command })
}

fn prepare_command(id: &str, tool_call_id: &str) -> WorkStepTransactionCommand {
    WorkStepTransactionCommand::Prepare {
        id: id.to_string(),
        work_id: "goal:goal-1".to_string(),
        step_id: "write-result".to_string(),
        tool_call_id: tool_call_id.to_string(),
        root: "/workspace".to_string(),
        effect_kind: "workspace_write".to_string(),
        workspace_checkpoint_id: None,
        request: json!({"path":"result.txt","content":"verified output"}),
        created_at_ms: 100,
    }
}

#[test]
fn competing_recovery_finalizes_observed_work_once_without_failing_the_loser() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("competing-recovery.db");
    let mut store = NolaStore::open(&database).unwrap();
    let checkpointed = prepare_and_checkpoint(&mut store, "observed", "observed-call");
    let observed: WorkStepTransaction = serde_json::from_value(execute(&mut store,
        WorkStepTransactionCommand::Advance {
            id: "observed".into(), expected_revision: checkpointed.revision,
            phase: WorkStepTransactionPhase::EffectObserved, workspace_checkpoint_id: None,
            result: Some(json!({"error":false,"text":"original durable result"})),
            result_ref: None, error: None, updated_at_ms: 200,
        }).unwrap()).unwrap();
    let uncertain = prepare_and_checkpoint(&mut store, "uncertain", "uncertain-call");
    drop(store);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let workers = (0..2).map(|_| {
        let database = database.clone();
        let barrier = barrier.clone();
        let revision = observed.revision;
        std::thread::spawn(move || {
            let mut store = NolaStore::open(database).unwrap();
            barrier.wait();
            execute(&mut store, WorkStepTransactionCommand::ReconcileObserved {
                id: "observed".into(), expected_revision: revision, updated_at_ms: 300,
            }).unwrap()
        })
    }).collect::<Vec<_>>();
    let outcomes = workers.into_iter().map(|worker| worker.join().unwrap()).collect::<Vec<_>>();
    assert_eq!(outcomes.iter().filter(|value| !value.is_null()).count(), 1);
    let mut store = NolaStore::open(database).unwrap();
    let committed: WorkStepTransaction = serde_json::from_value(execute(&mut store,
        WorkStepTransactionCommand::Get { id: "observed".into() }).unwrap()).unwrap();
    assert_eq!(committed.phase, WorkStepTransactionPhase::Committed);
    assert_eq!(committed.revision, observed.revision + 1);
    assert_eq!(committed.result, observed.result);
    assert!(execute(&mut store, WorkStepTransactionCommand::ReconcileObserved {
        id: "uncertain".into(), expected_revision: uncertain.revision, updated_at_ms: 400,
    }).unwrap().is_null());
    let still_uncertain: WorkStepTransaction = serde_json::from_value(execute(&mut store,
        WorkStepTransactionCommand::Get { id: "uncertain".into() }).unwrap()).unwrap();
    assert_eq!(still_uncertain, uncertain);
}

fn linked_plan_steps() -> Value {
    json!([{
        "id": "write-result",
        "content": "Write the result",
        "activeForm": "Writing the result",
        "status": "in_progress",
        "priority": "high",
        "dependencyIds": [],
        "approvalStatus": "not_required",
        "capabilityRefs": ["builtin:write_file"],
        "expectedArtifactRefs": ["file:/workspace/result.txt"],
        "artifactRefs": [],
        "workStepTransactionRefs": [],
        "attempt": 1,
        "executorRef": "host:test",
        "resumeKey": "write-result:prepared",
        "successCriteria": ["The result exists"],
        "evidenceRefs": [],
        "verificationStatus": "pending"
    }])
}

fn linked_sop() -> ConversationPlanSopIdentity {
    ConversationPlanSopIdentity {
        run_id: "linked-run-1".to_string(),
        sop_id: "test.linked-plan".to_string(),
        sop_version: 1,
    }
}

fn prepare_and_checkpoint(
    store: &mut NolaStore,
    id: &str,
    tool_call_id: &str,
) -> WorkStepTransaction {
    let prepared: WorkStepTransactionPreparation =
        serde_json::from_value(execute(store, prepare_command(id, tool_call_id)).unwrap()).unwrap();
    serde_json::from_value(
        execute(
            store,
            WorkStepTransactionCommand::Advance {
                id: id.to_string(),
                expected_revision: prepared.transaction.revision,
                phase: WorkStepTransactionPhase::Checkpointed,
                workspace_checkpoint_id: None,
                result: None,
                result_ref: None,
                error: None,
                updated_at_ms: 150,
            },
        )
        .unwrap(),
    )
    .unwrap()
}

#[test]
fn side_effect_phase_and_result_commit_survive_process_reconstruction() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("work-step-transaction.db");

    {
        let mut store = NolaStore::open(&database).unwrap();
        let prepared: WorkStepTransactionPreparation =
            serde_json::from_value(execute(&mut store, prepare_command("tx-1", "call-1")).unwrap())
                .unwrap();
        assert!(prepared.inserted);
        assert_eq!(
            prepared.transaction.phase,
            WorkStepTransactionPhase::Prepared
        );
        assert_eq!(prepared.transaction.revision, 1);

        let duplicate: WorkStepTransactionPreparation =
            serde_json::from_value(execute(&mut store, prepare_command("tx-1", "call-1")).unwrap())
                .unwrap();
        assert!(!duplicate.inserted);
        assert_eq!(duplicate.transaction, prepared.transaction);

        let conflict = execute(&mut store, prepare_command("tx-other", "call-1")).unwrap_err();
        assert!(conflict.contains("different work step transaction"));

        let skipped_observation = execute(
            &mut store,
            WorkStepTransactionCommand::Advance {
                id: "tx-1".to_string(),
                expected_revision: 1,
                phase: WorkStepTransactionPhase::Committed,
                workspace_checkpoint_id: None,
                result: Some(json!({"written":true})),
                result_ref: None,
                error: None,
                updated_at_ms: 200,
            },
        )
        .unwrap_err();
        assert!(skipped_observation.contains("prepared -> committed"));
    }

    {
        let mut store = NolaStore::open(&database).unwrap();
        let incomplete: Vec<WorkStepTransaction> = serde_json::from_value(
            execute(
                &mut store,
                WorkStepTransactionCommand::ListIncomplete {
                    work_id: Some("goal:goal-1".to_string()),
                },
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(incomplete.len(), 1);
        assert_eq!(incomplete[0].phase, WorkStepTransactionPhase::Prepared);

        let checkpointed: WorkStepTransaction = serde_json::from_value(
            execute(
                &mut store,
                WorkStepTransactionCommand::Advance {
                    id: "tx-1".to_string(),
                    expected_revision: 1,
                    phase: WorkStepTransactionPhase::Checkpointed,
                    workspace_checkpoint_id: None,
                    result: None,
                    result_ref: None,
                    error: None,
                    updated_at_ms: 250,
                },
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(checkpointed.phase, WorkStepTransactionPhase::Checkpointed);
        assert_eq!(checkpointed.revision, 2);

        let observed: WorkStepTransaction = serde_json::from_value(
            execute(
                &mut store,
                WorkStepTransactionCommand::Advance {
                    id: "tx-1".to_string(),
                    expected_revision: 2,
                    phase: WorkStepTransactionPhase::EffectObserved,
                    workspace_checkpoint_id: None,
                    result: Some(json!({"path":"result.txt","bytes":15})),
                    result_ref: Some("file:/workspace/result.txt".to_string()),
                    error: None,
                    updated_at_ms: 300,
                },
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(observed.phase, WorkStepTransactionPhase::EffectObserved);
        assert_eq!(observed.revision, 3);
    }

    let mut store = NolaStore::open(&database).unwrap();
    let incomplete: Vec<WorkStepTransaction> = serde_json::from_value(
        execute(
            &mut store,
            WorkStepTransactionCommand::ListIncomplete { work_id: None },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(incomplete.len(), 1);
    assert_eq!(
        incomplete[0].phase,
        WorkStepTransactionPhase::EffectObserved
    );
    assert_eq!(
        incomplete[0].result_ref.as_deref(),
        Some("file:/workspace/result.txt")
    );

    let stale = execute(
        &mut store,
        WorkStepTransactionCommand::Advance {
            id: "tx-1".to_string(),
            expected_revision: 2,
            phase: WorkStepTransactionPhase::Committed,
            workspace_checkpoint_id: None,
            result: Some(json!({"verified":true})),
            result_ref: None,
            error: None,
            updated_at_ms: 400,
        },
    )
    .unwrap_err();
    assert!(stale.contains("changed before transition"));

    let committed: WorkStepTransaction = serde_json::from_value(
        execute(
            &mut store,
            WorkStepTransactionCommand::Advance {
                id: "tx-1".to_string(),
                expected_revision: 3,
                phase: WorkStepTransactionPhase::Committed,
                workspace_checkpoint_id: None,
                result: Some(json!({"verified":true,"path":"result.txt"})),
                result_ref: Some("file:/workspace/result.txt".to_string()),
                error: None,
                updated_at_ms: 500,
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(committed.phase, WorkStepTransactionPhase::Committed);
    assert_eq!(committed.revision, 4);

    let incomplete: Vec<WorkStepTransaction> = serde_json::from_value(
        execute(
            &mut store,
            WorkStepTransactionCommand::ListIncomplete { work_id: None },
        )
        .unwrap(),
    )
    .unwrap();
    assert!(incomplete.is_empty());
}

#[test]
fn free_text_work_step_resolution_is_not_an_execution_receipt() {
    let directory = tempdir().unwrap();
    let mut store = NolaStore::open(directory.path().join("no-asserted-success.db")).unwrap();
    let original = prepare_and_checkpoint(&mut store, "unknown", "call-unknown");
    for resolution in ["applied", "not_applied", "abandon"] {
        assert!(serde_json::from_value::<WorkStepTransactionCommand>(json!({
            "type":"resolve_uncertain", "id":original.id, "expected_revision":original.revision,
            "resolution":resolution,"note":"A human assertion is not a Host receipt","updated_at_ms":200
        })).is_err());
    }
    let current: Option<WorkStepTransaction> = serde_json::from_value(execute(
        &mut store, WorkStepTransactionCommand::Get { id: original.id.clone() }
    ).unwrap()).unwrap();
    assert_eq!(current.unwrap().phase, WorkStepTransactionPhase::Checkpointed);
}

#[test]
fn terminal_effect_atomically_links_transaction_and_artifact_to_the_plan_stage() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("linked-plan-transaction.db");
    {
        let mut store = NolaStore::open(&database).unwrap();
        let _: ConversationPlan = serde_json::from_value(
            execute_application_command(
                &mut store,
                ApplicationCommand::ConversationPlan {
                    command: ConversationPlanCommand::Save {
                        expected_revision: None,
                        conversation_id: "conversation-linked".to_string(),
                        sop: linked_sop(),
                        todos: linked_plan_steps(),
                        updated_at_ms: 100,
                    },
                },
            )
            .unwrap(),
        )
        .unwrap();

        let prepared: WorkStepTransactionPreparation = serde_json::from_value(
            execute(
                &mut store,
                WorkStepTransactionCommand::Prepare {
                    id: "tx-linked".to_string(),
                    work_id: "conversation-linked".to_string(),
                    step_id: "write-result".to_string(),
                    tool_call_id: "call-linked".to_string(),
                    root: "/workspace".to_string(),
                    effect_kind: "workspace_write".to_string(),
                    workspace_checkpoint_id: None,
                    request: json!({"path":"result.txt"}),
                    created_at_ms: 110,
                },
            )
            .unwrap(),
        )
        .unwrap();
        let checkpointed: WorkStepTransaction = serde_json::from_value(
            execute(
                &mut store,
                WorkStepTransactionCommand::Advance {
                    id: "tx-linked".to_string(),
                    expected_revision: prepared.transaction.revision,
                    phase: WorkStepTransactionPhase::Checkpointed,
                    workspace_checkpoint_id: None,
                    result: None,
                    result_ref: None,
                    error: None,
                    updated_at_ms: 120,
                },
            )
            .unwrap(),
        )
        .unwrap();
        let observed: WorkStepTransaction = serde_json::from_value(
            execute(
                &mut store,
                WorkStepTransactionCommand::Advance {
                    id: "tx-linked".to_string(),
                    expected_revision: checkpointed.revision,
                    phase: WorkStepTransactionPhase::EffectObserved,
                    workspace_checkpoint_id: None,
                    result: Some(json!({
                        "path":"result.txt",
                        "error":false,
                        "artifacts":[{
                            "uri":"file:///workspace/result.txt",
                            "name":"result.txt",
                            "mime_type":"text/plain",
                            "operation":"write"
                        }]
                    })),
                    result_ref: Some("tool-output://tx-linked".to_string()),
                    error: None,
                    updated_at_ms: 130,
                },
            )
            .unwrap(),
        )
        .unwrap();
        let committed: WorkStepTransaction = serde_json::from_value(
            execute(
                &mut store,
                WorkStepTransactionCommand::Advance {
                    id: "tx-linked".to_string(),
                    expected_revision: observed.revision,
                    phase: WorkStepTransactionPhase::Committed,
                    workspace_checkpoint_id: None,
                    result: Some(json!({
                        "path":"result.txt",
                        "error":false,
                        "artifacts":[{
                            "uri":"file:///workspace/result.txt",
                            "name":"result.txt",
                            "mime_type":"text/plain",
                            "operation":"write"
                        }]
                    })),
                    result_ref: Some("tool-output://tx-linked".to_string()),
                    error: None,
                    updated_at_ms: 140,
                },
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(committed.phase, WorkStepTransactionPhase::Committed);

        let plan: Option<ConversationPlan> = serde_json::from_value(
            execute_application_command(
                &mut store,
                ApplicationCommand::ConversationPlan {
                    command: ConversationPlanCommand::Get {
                        conversation_id: "conversation-linked".to_string(),
                    },
                },
            )
            .unwrap(),
        )
        .unwrap();
        let plan = plan.unwrap();
        assert_eq!(plan.revision, 2);
        assert_eq!(plan.todos[0].work_step_transaction_refs, vec!["tx-linked"]);
        assert_eq!(
            plan.todos[0].artifact_refs,
            vec!["file:///workspace/result.txt"]
        );
        let history: Vec<ConversationPlanRevision> = serde_json::from_value(
            execute_application_command(
                &mut store,
                ApplicationCommand::ConversationPlan {
                    command: ConversationPlanCommand::History {
                        conversation_id: "conversation-linked".to_string(),
                    },
                },
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(history[1].change_kind, "execution");
        assert_eq!(history[1].sop, linked_sop());
    }

    let mut reopened = NolaStore::open(&database).unwrap();
    let restored: Option<ConversationPlan> = serde_json::from_value(
        execute_application_command(
            &mut reopened,
            ApplicationCommand::ConversationPlan {
                command: ConversationPlanCommand::Get {
                    conversation_id: "conversation-linked".to_string(),
                },
            },
        )
        .unwrap(),
    )
    .unwrap();
    let restored = restored.unwrap();
    assert_eq!(restored.sop, linked_sop());
    assert_eq!(
        restored.todos[0].work_step_transaction_refs,
        vec!["tx-linked"]
    );
    assert_eq!(
        restored.todos[0].artifact_refs,
        vec!["file:///workspace/result.txt"]
    );
}

#[test]
fn invalid_host_artifact_rejects_terminal_commit_and_rolls_back_the_plan_link() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("invalid-linked-artifact.db");
    let mut store = NolaStore::open(&database).unwrap();
    let _: ConversationPlan = serde_json::from_value(
        execute_application_command(
            &mut store,
            ApplicationCommand::ConversationPlan {
                command: ConversationPlanCommand::Save {
                    expected_revision: None,
                    conversation_id: "conversation-invalid-artifact".to_string(),
                    sop: linked_sop(),
                    todos: linked_plan_steps(),
                    updated_at_ms: 100,
                },
            },
        )
        .unwrap(),
    )
    .unwrap();
    let prepared: WorkStepTransactionPreparation = serde_json::from_value(
        execute(
            &mut store,
            WorkStepTransactionCommand::Prepare {
                id: "tx-invalid-artifact".to_string(),
                work_id: "conversation-invalid-artifact".to_string(),
                step_id: "write-result".to_string(),
                tool_call_id: "call-invalid-artifact".to_string(),
                root: "/workspace".to_string(),
                effect_kind: "workspace_write".to_string(),
                workspace_checkpoint_id: None,
                request: json!({"path":"invalid.txt"}),
                created_at_ms: 110,
            },
        )
        .unwrap(),
    )
    .unwrap();
    let checkpointed: WorkStepTransaction = serde_json::from_value(
        execute(
            &mut store,
            WorkStepTransactionCommand::Advance {
                id: "tx-invalid-artifact".to_string(),
                expected_revision: prepared.transaction.revision,
                phase: WorkStepTransactionPhase::Checkpointed,
                workspace_checkpoint_id: None,
                result: None,
                result_ref: None,
                error: None,
                updated_at_ms: 120,
            },
        )
        .unwrap(),
    )
    .unwrap();
    let invalid_result = json!({
        "error": false,
        "artifacts": [{
            "uri": "file:///workspace/not encoded name.txt",
            "name": "not encoded name.txt",
            "mime_type": "text/plain",
            "operation": "write"
        }]
    });
    let observed: WorkStepTransaction = serde_json::from_value(
        execute(
            &mut store,
            WorkStepTransactionCommand::Advance {
                id: "tx-invalid-artifact".to_string(),
                expected_revision: checkpointed.revision,
                phase: WorkStepTransactionPhase::EffectObserved,
                workspace_checkpoint_id: None,
                result: Some(invalid_result.clone()),
                result_ref: None,
                error: None,
                updated_at_ms: 130,
            },
        )
        .unwrap(),
    )
    .unwrap();
    let failure = execute(
        &mut store,
        WorkStepTransactionCommand::Advance {
            id: "tx-invalid-artifact".to_string(),
            expected_revision: observed.revision,
            phase: WorkStepTransactionPhase::Committed,
            workspace_checkpoint_id: None,
            result: Some(invalid_result),
            result_ref: None,
            error: None,
            updated_at_ms: 140,
        },
    )
    .unwrap_err();
    assert!(
        failure.contains("normalized and percent-encoded"),
        "{failure}"
    );

    let persisted: WorkStepTransaction = serde_json::from_value(
        execute(
            &mut store,
            WorkStepTransactionCommand::Get {
                id: "tx-invalid-artifact".to_string(),
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(persisted.phase, WorkStepTransactionPhase::EffectObserved);
    let plan: Option<ConversationPlan> = serde_json::from_value(
        execute_application_command(
            &mut store,
            ApplicationCommand::ConversationPlan {
                command: ConversationPlanCommand::Get {
                    conversation_id: "conversation-invalid-artifact".to_string(),
                },
            },
        )
        .unwrap(),
    )
    .unwrap();
    let plan = plan.unwrap();
    assert_eq!(plan.revision, 1);
    assert!(plan.todos[0].artifact_refs.is_empty());
    assert!(plan.todos[0].work_step_transaction_refs.is_empty());
}
