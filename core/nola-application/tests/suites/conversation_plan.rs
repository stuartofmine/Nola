// SPDX-License-Identifier: Apache-2.0
//! Consolidated Conversation Plan & Work-Step Transaction suite:
//! Save CAS, user approval, edit/freeze/replan/diff/history, runtime metadata,
//! work-step phases, invalid-artifact rollback, receipt atomicity, and competing recovery.

use crate::support;
use nola_application::{
    execute_application_command, ApplicationCommand, ApplicationCommandReceiptOutcome,
    ConversationPlan, ConversationPlanApprovalStatus, ConversationPlanCommand,
    ConversationPlanDiff, ConversationPlanRevision, ConversationPlanSopIdentity,
    ConversationPlanStepEdit, ConversationPlanStepStatus, ExecuteReceivedCommand,
    WorkStepTransaction, WorkStepTransactionCommand, WorkStepTransactionPhase,
    WorkStepTransactionPreparation,
};
use nola_protocol::CommandMetadata;
use nola_store::NolaStore;
use serde_json::{json, Value};
use tempfile::tempdir;

fn plan_execute(store: &mut NolaStore, command: ConversationPlanCommand) -> Result<Value, String> {
    execute_application_command(store, ApplicationCommand::ConversationPlan { command })
}

fn step_execute(
    store: &mut NolaStore,
    command: WorkStepTransactionCommand,
) -> Result<Value, String> {
    execute_application_command(store, ApplicationCommand::WorkStepTransaction { command })
}

fn sop(run_id: &str, version: u32) -> ConversationPlanSopIdentity {
    ConversationPlanSopIdentity {
        run_id: run_id.to_string(),
        sop_id: "test.plan".to_string(),
        sop_version: version,
    }
}

fn steps(inspect_content: &str, write_content: &str, approval: &str) -> Value {
    json!([
        {
            "id": "inspect",
            "content": inspect_content,
            "activeForm": "Inspecting the workspace",
            "status": "pending",
            "priority": "high",
            "dependencyIds": [],
            "approvalStatus": approval,
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
            "approvalStatus": approval,
            "capabilityRefs": ["builtin:write_file"],
            "expectedArtifactRefs": ["file:result"],
            "successCriteria": ["The result exists"],
            "evidenceRefs": [],
            "verificationStatus": "pending"
        }
    ])
}

#[test]
fn plan_cas_approval_lifecycle_and_runtime_fields_survive_cold_reopen() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("plan-suite.db");

    {
        // 1. Cross-connection Save CAS and create-only absence semantics ("cas").
        let mut first = NolaStore::open(&database).unwrap();
        let mut second = NolaStore::open(&database).unwrap();
        let save_cas = |expected_revision, content: &str| ApplicationCommand::ConversationPlan {
            command: ConversationPlanCommand::Save {
                conversation_id: "cas".into(),
                expected_revision,
                sop: ConversationPlanSopIdentity {
                    run_id: "r".into(),
                    sop_id: "s".into(),
                    sop_version: 1,
                },
                todos: json!([{
                    "id": "step",
                    "content": content,
                    "status": "pending",
                    "priority": "medium",
                    "dependencyIds": [],
                    "approvalStatus": "not_required",
                    "capabilityRefs": [],
                    "expectedArtifactRefs": [],
                    "successCriteria": ["done"],
                    "verificationStatus": "pending"
                }]),
                updated_at_ms: 1,
            },
        };
        let created = execute_application_command(&mut first, save_cas(None, "original")).unwrap();
        assert_eq!(created["revision"], 1);
        assert!(execute_application_command(&mut second, save_cas(None, "duplicate create")).is_err());
        let winner = execute_application_command(&mut first, save_cas(Some(1), "winner")).unwrap();
        assert_eq!(winner["revision"], 2);
        assert!(execute_application_command(&mut second, save_cas(Some(1), "stale edit"))
            .unwrap_err()
            .contains("revision conflict"));
        let current =
            plan_execute(&mut second, ConversationPlanCommand::Get { conversation_id: "cas".into() })
                .unwrap();
        assert_eq!(current["todos"][0]["content"], "winner");
        drop(second);

        // 2. Revision-guarded user approval ("approval-conversation").
        let saved_approval: ConversationPlan = serde_json::from_value(
            plan_execute(
                &mut first,
                ConversationPlanCommand::Save {
                    expected_revision: None,
                    conversation_id: "approval-conversation".into(),
                    sop: sop("run-1", 1),
                    todos: steps("Inspect the workspace", "Write the result", "pending"),
                    updated_at_ms: 100,
                },
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(saved_approval.revision, 1);
        let approved: ConversationPlan = serde_json::from_value(
            plan_execute(
                &mut first,
                ConversationPlanCommand::SetApproval {
                    conversation_id: "approval-conversation".into(),
                    expected_revision: 1,
                    step_ids: vec!["inspect".into()],
                    decision: ConversationPlanApprovalStatus::Approved,
                    updated_at_ms: 200,
                },
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(approved.revision, 2);
        assert_eq!(
            approved.todos[0].approval_status,
            ConversationPlanApprovalStatus::Approved
        );
        let stale_approval = plan_execute(
            &mut first,
            ConversationPlanCommand::SetApproval {
                conversation_id: "approval-conversation".into(),
                expected_revision: 1,
                step_ids: vec!["write".into()],
                decision: ConversationPlanApprovalStatus::Rejected,
                updated_at_ms: 300,
            },
        )
        .unwrap_err();
        assert!(stale_approval.contains("revision conflict"));

        // 3. Edit -> Freeze -> frozen mutation guards -> Replan -> Diff ("lifecycle-conversation").
        let saved: ConversationPlan = serde_json::from_value(
            plan_execute(
                &mut first,
                ConversationPlanCommand::Save {
                    expected_revision: None,
                    conversation_id: "lifecycle-conversation".into(),
                    sop: sop("lifecycle-run-1", 1),
                    todos: steps("Inspect the workspace", "Write the result", "not_required"),
                    updated_at_ms: 100,
                },
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!((saved.revision, saved.frozen_revision), (1, None));

        let edited: ConversationPlan = serde_json::from_value(
            plan_execute(
                &mut first,
                ConversationPlanCommand::EditStep {
                    conversation_id: "lifecycle-conversation".into(),
                    expected_revision: 1,
                    step_id: "inspect".into(),
                    edit: ConversationPlanStepEdit {
                        content: Some("Inspect the authoritative workspace".into()),
                        ..ConversationPlanStepEdit::default()
                    },
                    updated_at_ms: 200,
                },
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(edited.revision, 2);
        assert_eq!(edited.todos[0].content, "Inspect the authoritative workspace");

        let frozen: ConversationPlan = serde_json::from_value(
            plan_execute(
                &mut first,
                ConversationPlanCommand::Freeze {
                    conversation_id: "lifecycle-conversation".into(),
                    expected_revision: 2,
                    updated_at_ms: 300,
                },
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!((frozen.revision, frozen.frozen_revision), (3, Some(2)));

        assert!(plan_execute(
            &mut first,
            ConversationPlanCommand::EditStep {
                conversation_id: "lifecycle-conversation".into(),
                expected_revision: 3,
                step_id: "write".into(),
                edit: ConversationPlanStepEdit {
                    content: Some("Silently replace the frozen contract".into()),
                    ..ConversationPlanStepEdit::default()
                },
                updated_at_ms: 400,
            },
        )
        .unwrap_err()
        .contains("frozen"));

        assert!(plan_execute(
            &mut first,
            ConversationPlanCommand::Save {
                expected_revision: Some(3),
                conversation_id: "lifecycle-conversation".into(),
                sop: sop("lifecycle-run-1", 1),
                todos: steps("Inspect something else", "Write something else", "not_required"),
                updated_at_ms: 500,
            },
        )
        .unwrap_err()
        .contains("explicit replan"));

        assert!(plan_execute(
            &mut first,
            ConversationPlanCommand::Save {
                expected_revision: Some(3),
                conversation_id: "lifecycle-conversation".into(),
                sop: sop("run-silent-swap", 2),
                todos: steps(
                    "Inspect the authoritative workspace",
                    "Write the result",
                    "not_required"
                ),
                updated_at_ms: 550,
            },
        )
        .unwrap_err()
        .contains("SOP identity is immutable"));

        assert!(plan_execute(
            &mut first,
            ConversationPlanCommand::Replan {
                conversation_id: "lifecycle-conversation".into(),
                expected_revision: 2,
                sop: sop("lifecycle-run-2", 2),
                todos: steps(
                    "Inspect the authoritative workspace",
                    "Write the revised result",
                    "not_required"
                ),
                reason: "New evidence changes the output contract".into(),
                updated_at_ms: 600,
            },
        )
        .unwrap_err()
        .contains("revision conflict"));

        assert!(plan_execute(
            &mut first,
            ConversationPlanCommand::Replan {
                conversation_id: "lifecycle-conversation".into(),
                expected_revision: 3,
                sop: sop("lifecycle-run-1", 2),
                todos: steps(
                    "Inspect the authoritative workspace",
                    "Write the revised result",
                    "not_required"
                ),
                reason: "Attempt to reuse the prior run".into(),
                updated_at_ms: 650,
            },
        )
        .unwrap_err()
        .contains("new SOP run id"));

        let replanned: ConversationPlan = serde_json::from_value(
            plan_execute(
                &mut first,
                ConversationPlanCommand::Replan {
                    conversation_id: "lifecycle-conversation".into(),
                    expected_revision: 3,
                    sop: sop("lifecycle-run-2", 2),
                    todos: steps(
                        "Inspect the authoritative workspace",
                        "Write the revised result",
                        "not_required",
                    ),
                    reason: "New evidence changes the output contract".into(),
                    updated_at_ms: 700,
                },
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(replanned.revision, 4);
        assert_eq!(replanned.sop, sop("lifecycle-run-2", 2));
        assert_eq!(replanned.frozen_revision, None);
        assert_eq!(replanned.todos[1].content, "Write the revised result");

        let diff: ConversationPlanDiff = serde_json::from_value(
            plan_execute(
                &mut first,
                ConversationPlanCommand::Diff {
                    conversation_id: "lifecycle-conversation".into(),
                    from_revision: 2,
                    to_revision: 4,
                },
            )
            .unwrap(),
        )
        .unwrap();
        assert!(diff.added.is_empty() && diff.removed.is_empty());
        assert_eq!(diff.changed.len(), 1);
        assert_eq!(diff.changed[0].id, "write");
        assert_eq!((diff.from_sop, diff.to_sop), (sop("lifecycle-run-1", 1), sop("lifecycle-run-2", 2)));

        // 4. Stage artifacts, transaction refs, executor ref, and resume key ("runtime-conversation").
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
        let saved_runtime: ConversationPlan = serde_json::from_value(
            plan_execute(
                &mut first,
                ConversationPlanCommand::Save {
                    expected_revision: None,
                    conversation_id: "runtime-conversation".into(),
                    sop: sop("runtime-run-1", 1),
                    todos: runtime_steps,
                    updated_at_ms: 100,
                },
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(saved_runtime.todos[0].artifact_refs, vec!["artifact:inventory"]);
        assert_eq!(saved_runtime.todos[1].attempt, 1);
    }

    // 5. Cold reopen verification for all persisted plans & revision histories.
    let mut reopened = NolaStore::open(&database).unwrap();

    let approval_loaded: ConversationPlan = serde_json::from_value::<Option<ConversationPlan>>(
        plan_execute(
            &mut reopened,
            ConversationPlanCommand::Get {
                conversation_id: "approval-conversation".into(),
            },
        )
        .unwrap(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(approval_loaded.revision, 2);
    assert_eq!(
        approval_loaded.todos[0].status,
        ConversationPlanStepStatus::Pending
    );
    assert_eq!(
        approval_loaded.todos[0].approval_status,
        ConversationPlanApprovalStatus::Approved
    );
    assert_eq!(
        approval_loaded.todos[1].approval_status,
        ConversationPlanApprovalStatus::Pending
    );

    let lifecycle_loaded: ConversationPlan = serde_json::from_value::<Option<ConversationPlan>>(
        plan_execute(
            &mut reopened,
            ConversationPlanCommand::Get {
                conversation_id: "lifecycle-conversation".into(),
            },
        )
        .unwrap(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(lifecycle_loaded.revision, 4);
    assert_eq!(lifecycle_loaded.sop, sop("lifecycle-run-2", 2));
    assert_eq!(lifecycle_loaded.frozen_revision, None);

    let history: Vec<ConversationPlanRevision> = serde_json::from_value(
        plan_execute(
            &mut reopened,
            ConversationPlanCommand::History {
                conversation_id: "lifecycle-conversation".into(),
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        history
            .iter()
            .map(|r| r.change_kind.as_str())
            .collect::<Vec<_>>(),
        vec!["save", "edit", "freeze", "replan"]
    );
    assert_eq!(
        history[3].change_summary,
        "New evidence changes the output contract"
    );
    assert_eq!(
        (&history[0].sop, &history[3].sop),
        (&sop("lifecycle-run-1", 1), &sop("lifecycle-run-2", 2))
    );

    let runtime_loaded: ConversationPlan = serde_json::from_value::<Option<ConversationPlan>>(
        plan_execute(
            &mut reopened,
            ConversationPlanCommand::Get {
                conversation_id: "runtime-conversation".into(),
            },
        )
        .unwrap(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(runtime_loaded.sop, sop("runtime-run-1", 1));
    assert_eq!(runtime_loaded.todos[0].artifact_refs, vec!["artifact:inventory"]);
    assert_eq!(
        runtime_loaded.todos[0].work_step_transaction_refs,
        vec!["work-step:inspect:1"]
    );
    assert_eq!(runtime_loaded.todos[1].executor_ref.as_deref(), Some("host:cli"));
    assert_eq!(
        runtime_loaded.todos[1].resume_key.as_deref(),
        Some("checkpoint:write:prepared")
    );
    assert_eq!(runtime_loaded.todos[1].attempt, 1);
}

fn prepare_command(work_id: &str, id: &str, tool_call_id: &str) -> WorkStepTransactionCommand {
    WorkStepTransactionCommand::Prepare {
        id: id.to_string(),
        work_id: work_id.to_string(),
        step_id: "write-result".to_string(),
        tool_call_id: tool_call_id.to_string(),
        root: "/workspace".to_string(),
        effect_kind: "workspace_write".to_string(),
        workspace_checkpoint_id: None,
        request: json!({"path":"result.txt","content":"verified output"}),
        created_at_ms: 100,
    }
}

fn prepare_and_checkpoint(
    store: &mut NolaStore,
    work_id: &str,
    id: &str,
    tool_call_id: &str,
) -> WorkStepTransaction {
    let prepared: WorkStepTransactionPreparation =
        serde_json::from_value(step_execute(store, prepare_command(work_id, id, tool_call_id)).unwrap())
            .unwrap();
    serde_json::from_value(
        step_execute(
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
fn work_step_lifecycle_plan_linkage_receipt_rollback_and_competing_recovery() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("work-step-suite.db");

    // 1. Prepare idempotency, call_id conflict, phase transition ordering, and reopen reconstruction.
    {
        let mut store = NolaStore::open(&database).unwrap();
        let prepared: WorkStepTransactionPreparation = serde_json::from_value(
            step_execute(&mut store, prepare_command("goal:goal-1", "tx-1", "call-1")).unwrap(),
        )
        .unwrap();
        assert!(prepared.inserted);
        assert_eq!(prepared.transaction.phase, WorkStepTransactionPhase::Prepared);
        assert_eq!(prepared.transaction.revision, 1);

        let duplicate: WorkStepTransactionPreparation = serde_json::from_value(
            step_execute(&mut store, prepare_command("goal:goal-1", "tx-1", "call-1")).unwrap(),
        )
        .unwrap();
        assert!(!duplicate.inserted);
        assert_eq!(duplicate.transaction, prepared.transaction);

        assert!(step_execute(&mut store, prepare_command("goal:goal-1", "tx-other", "call-1"))
            .unwrap_err()
            .contains("different work step transaction"));

        assert!(step_execute(
            &mut store,
            WorkStepTransactionCommand::Advance {
                id: "tx-1".into(),
                expected_revision: 1,
                phase: WorkStepTransactionPhase::Committed,
                workspace_checkpoint_id: None,
                result: Some(json!({"written":true})),
                result_ref: None,
                error: None,
                updated_at_ms: 200,
            },
        )
        .unwrap_err()
        .contains("prepared -> committed"));
    }

    {
        let mut store = NolaStore::open(&database).unwrap();
        let incomplete: Vec<WorkStepTransaction> = serde_json::from_value(
            step_execute(
                &mut store,
                WorkStepTransactionCommand::ListIncomplete {
                    work_id: Some("goal:goal-1".into()),
                },
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(incomplete.len(), 1);
        assert_eq!(incomplete[0].phase, WorkStepTransactionPhase::Prepared);

        let checkpointed: WorkStepTransaction = serde_json::from_value(
            step_execute(
                &mut store,
                WorkStepTransactionCommand::Advance {
                    id: "tx-1".into(),
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
        assert_eq!(
            (checkpointed.phase, checkpointed.revision),
            (WorkStepTransactionPhase::Checkpointed, 2)
        );

        let observed: WorkStepTransaction = serde_json::from_value(
            step_execute(
                &mut store,
                WorkStepTransactionCommand::Advance {
                    id: "tx-1".into(),
                    expected_revision: 2,
                    phase: WorkStepTransactionPhase::EffectObserved,
                    workspace_checkpoint_id: None,
                    result: Some(json!({"path":"result.txt","bytes":15})),
                    result_ref: Some("file:/workspace/result.txt".into()),
                    error: None,
                    updated_at_ms: 300,
                },
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            (observed.phase, observed.revision),
            (WorkStepTransactionPhase::EffectObserved, 3)
        );

        assert!(step_execute(
            &mut store,
            WorkStepTransactionCommand::Advance {
                id: "tx-1".into(),
                expected_revision: 2,
                phase: WorkStepTransactionPhase::Committed,
                workspace_checkpoint_id: None,
                result: Some(json!({"verified":true})),
                result_ref: None,
                error: None,
                updated_at_ms: 400,
            },
        )
        .unwrap_err()
        .contains("changed before transition"));

        let committed: WorkStepTransaction = serde_json::from_value(
            step_execute(
                &mut store,
                WorkStepTransactionCommand::Advance {
                    id: "tx-1".into(),
                    expected_revision: 3,
                    phase: WorkStepTransactionPhase::Committed,
                    workspace_checkpoint_id: None,
                    result: Some(json!({"verified":true,"path":"result.txt"})),
                    result_ref: Some("file:/workspace/result.txt".into()),
                    error: None,
                    updated_at_ms: 500,
                },
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            (committed.phase, committed.revision),
            (WorkStepTransactionPhase::Committed, 4)
        );
        let incomplete: Vec<WorkStepTransaction> = serde_json::from_value(
            step_execute(
                &mut store,
                WorkStepTransactionCommand::ListIncomplete { work_id: None },
            )
            .unwrap(),
        )
        .unwrap();
        assert!(incomplete.is_empty());

        // 2. Free-text work-step resolution wire commands are rejected.
        let unknown =
            prepare_and_checkpoint(&mut store, "goal:goal-1", "unknown", "call-unknown");
        for resolution in ["applied", "not_applied", "abandon"] {
            assert!(serde_json::from_value::<WorkStepTransactionCommand>(json!({
                "type":"resolve_uncertain",
                "id": unknown.id,
                "expected_revision": unknown.revision,
                "resolution": resolution,
                "note": "A human assertion is not a Host receipt",
                "updated_at_ms": 200
            }))
            .is_err());
        }
        let current: Option<WorkStepTransaction> = serde_json::from_value(
            step_execute(
                &mut store,
                WorkStepTransactionCommand::Get {
                    id: unknown.id.clone(),
                },
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(current.unwrap().phase, WorkStepTransactionPhase::Checkpointed);
    }

    // 3. Linked plan stage: invalid artifact rollback + receipt failure rollback + atomic commit + replay.
    let linked_sop = ConversationPlanSopIdentity {
        run_id: "linked-run-1".into(),
        sop_id: "test.linked-plan".into(),
        sop_version: 1,
    };
    let linked_steps = json!([{
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
    }]);

    let before_linked = {
        let mut store = NolaStore::open(&database).unwrap();
        plan_execute(
            &mut store,
            ConversationPlanCommand::Save {
                expected_revision: None,
                conversation_id: "conversation-linked".into(),
                sop: linked_sop.clone(),
                todos: linked_steps,
                updated_at_ms: 100,
            },
        )
        .unwrap();
        let checkpointed = prepare_and_checkpoint(
            &mut store,
            "conversation-linked",
            "tx-linked",
            "call-linked",
        );
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
            step_execute(
                &mut store,
                WorkStepTransactionCommand::Advance {
                    id: "tx-linked".into(),
                    expected_revision: checkpointed.revision,
                    phase: WorkStepTransactionPhase::EffectObserved,
                    workspace_checkpoint_id: None,
                    result: Some(invalid_result.clone()),
                    result_ref: Some("tool-output://tx-linked".into()),
                    error: None,
                    updated_at_ms: 130,
                },
            )
            .unwrap(),
        )
        .unwrap();
        let failure = step_execute(
            &mut store,
            WorkStepTransactionCommand::Advance {
                id: "tx-linked".into(),
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
        assert!(failure.contains("normalized and percent-encoded"), "{failure}");
        let plan_unchanged: ConversationPlan = serde_json::from_value::<Option<ConversationPlan>>(
            plan_execute(
                &mut store,
                ConversationPlanCommand::Get {
                    conversation_id: "conversation-linked".into(),
                },
            )
            .unwrap(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(plan_unchanged.revision, 1);
        assert!(plan_unchanged.todos[0].artifact_refs.is_empty());
        assert!(plan_unchanged.todos[0].work_step_transaction_refs.is_empty());

        (
            store.conversation_plan("conversation-linked").unwrap(),
            store.work_step_transaction("tx-linked").unwrap(),
            store.conversation_plan_revisions("conversation-linked").unwrap(),
            observed.revision,
        )
    };

    let valid_commit = || ApplicationCommand::WorkStepTransaction {
        command: WorkStepTransactionCommand::Advance {
            id: "tx-linked".into(),
            expected_revision: before_linked.3,
            phase: WorkStepTransactionPhase::Committed,
            workspace_checkpoint_id: None,
            result: Some(json!({
                "path": "result.txt",
                "error": false,
                "artifacts": [{
                    "uri": "file:///workspace/result.txt",
                    "name": "result.txt",
                    "mime_type": "text/plain",
                    "operation": "write"
                }]
            })),
            result_ref: Some("tool-output://tx-linked".into()),
            error: None,
            updated_at_ms: 145,
        },
    };
    let admission = |id: &str, cmd: &ApplicationCommand| ExecuteReceivedCommand {
        client_id: "plan-port".into(),
        request_id: id.into(),
        command_type: cmd.wire_command_name().into(),
        command_json: serde_json::to_string(cmd).unwrap(),
        expected_revision: None,
        received_at: "2026-09-08T00:00:00Z".into(),
    };
    let db = rusqlite::Connection::open(&database).unwrap();
    let outbox_and_events = || {
        db.query_row(
            "SELECT (SELECT COUNT(*) FROM sync_outbox)+(SELECT COUNT(*) FROM projection_events)",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
    };
    let initial_count = outbox_and_events();
    db.execute_batch(
        "CREATE TRIGGER receipt_failure BEFORE UPDATE ON command_receipts WHEN NEW.status='committed'
         BEGIN SELECT RAISE(ABORT,'injected receipt failure'); END",
    )
    .unwrap();
    let mut runtime = support::open_runtime(&database).unwrap();
    let commit_cmd = valid_commit();
    let failed = runtime.execute_received_command(
        admission("failed-linked", &commit_cmd),
        commit_cmd,
    );
    if let Ok(receipt) = failed {
        assert!(matches!(
            receipt.outcome,
            ApplicationCommandReceiptOutcome::Rejected { .. }
        ));
    }
    {
        let store = NolaStore::open(&database).unwrap();
        assert_eq!(
            (
                store.conversation_plan("conversation-linked").unwrap(),
                store.work_step_transaction("tx-linked").unwrap(),
                store.conversation_plan_revisions("conversation-linked").unwrap(),
            ),
            (before_linked.0, before_linked.1, before_linked.2)
        );
    }
    assert_eq!(outbox_and_events(), initial_count);
    db.execute_batch("DROP TRIGGER receipt_failure").unwrap();
    drop(runtime);

    let mut runtime = support::open_runtime(&database).unwrap();
    let commit_cmd = valid_commit();
    let received = admission("success-linked", &commit_cmd);
    let receipt = runtime
        .execute_received_command(received.clone(), commit_cmd)
        .unwrap();
    assert!(matches!(
        receipt.outcome,
        ApplicationCommandReceiptOutcome::Committed { .. }
    ));
    let after_count = outbox_and_events();
    let replay = runtime
        .execute_received_command(received, valid_commit())
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(
        replay.into_protocol_receipt(),
        receipt.into_protocol_receipt()
    );
    assert_eq!(outbox_and_events(), after_count);
    drop(runtime);
    drop(db);

    {
        let mut reopened = NolaStore::open(&database).unwrap();
        assert_eq!(
            reopened
                .work_step_transaction("tx-linked")
                .unwrap()
                .unwrap()
                .phase,
            "committed"
        );
        let restored: ConversationPlan = serde_json::from_value::<Option<ConversationPlan>>(
            plan_execute(
                &mut reopened,
                ConversationPlanCommand::Get {
                    conversation_id: "conversation-linked".into(),
                },
            )
            .unwrap(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(restored.revision, 2);
        assert_eq!(restored.sop, linked_sop);
        assert_eq!(restored.todos[0].work_step_transaction_refs, vec!["tx-linked"]);
        assert_eq!(
            restored.todos[0].artifact_refs,
            vec!["file:///workspace/result.txt"]
        );
        let history: Vec<ConversationPlanRevision> = serde_json::from_value(
            plan_execute(
                &mut reopened,
                ConversationPlanCommand::History {
                    conversation_id: "conversation-linked".into(),
                },
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(history[1].change_kind, "execution");
        assert_eq!(history[1].sop, linked_sop);
    }

    // 4. Competing recovery finalizes observed work once without failing the loser.
    let (observed, uncertain) = {
        let mut store = NolaStore::open(&database).unwrap();
        let checkpointed =
            prepare_and_checkpoint(&mut store, "goal:recovery", "observed", "observed-call");
        let observed: WorkStepTransaction = serde_json::from_value(
            step_execute(
                &mut store,
                WorkStepTransactionCommand::Advance {
                    id: "observed".into(),
                    expected_revision: checkpointed.revision,
                    phase: WorkStepTransactionPhase::EffectObserved,
                    workspace_checkpoint_id: None,
                    result: Some(json!({"error":false,"text":"original durable result"})),
                    result_ref: None,
                    error: None,
                    updated_at_ms: 200,
                },
            )
            .unwrap(),
        )
        .unwrap();
        let uncertain =
            prepare_and_checkpoint(&mut store, "goal:recovery", "uncertain", "uncertain-call");
        (observed, uncertain)
    };
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let workers = (0..2)
        .map(|_| {
            let db_path = database.clone();
            let barrier = barrier.clone();
            let revision = observed.revision;
            std::thread::spawn(move || {
                let mut store = NolaStore::open(db_path).unwrap();
                barrier.wait();
                step_execute(
                    &mut store,
                    WorkStepTransactionCommand::ReconcileObserved {
                        id: "observed".into(),
                        expected_revision: revision,
                        updated_at_ms: 300,
                    },
                )
                .unwrap()
            })
        })
        .collect::<Vec<_>>();
    let outcomes = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(outcomes.iter().filter(|v| !v.is_null()).count(), 1);

    let mut store = NolaStore::open(&database).unwrap();
    let committed: WorkStepTransaction = serde_json::from_value(
        step_execute(
            &mut store,
            WorkStepTransactionCommand::Get {
                id: "observed".into(),
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(committed.phase, WorkStepTransactionPhase::Committed);
    assert_eq!(committed.revision, observed.revision + 1);
    assert_eq!(committed.result, observed.result);
    assert!(step_execute(
        &mut store,
        WorkStepTransactionCommand::ReconcileObserved {
            id: "uncertain".into(),
            expected_revision: uncertain.revision,
            updated_at_ms: 400,
        }
    )
    .unwrap()
    .is_null());
    let still_uncertain: WorkStepTransaction = serde_json::from_value(
        step_execute(
            &mut store,
            WorkStepTransactionCommand::Get {
                id: "uncertain".into(),
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(still_uncertain, uncertain);
}
