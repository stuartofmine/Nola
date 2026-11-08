#[path = "support/model_identity.rs"]
mod model_identity;
use nola_core::agent::recovery::CoreCheckpoint;
use nola_core::agent::{
    AgentCore, AgentCoreConfig, AgentSubmission, AgentThreadState, GoalAuditFinding,
    GoalAuditReport, GoalAuditVerdict, GoalContractStatus, GoalEvidence, GoalExecutionReport,
    GoalRecordKind, GoalRecordStatus, GoalRequirement, GoalRuntimeCommand,
    GoalRuntimeCommandEnvelope, GoalStatus, GoalSubtaskContract, SessionId,
};

fn core_config() -> AgentCoreConfig {
    use nola_core::agent::host::*;
    let mut catalog = HostCapabilityCatalog::empty("goal-fixture-host");
    catalog.capabilities.insert("model.stream".into(), HostCapabilityDescriptor {
        id:"model.stream".into(), title:"Deterministic fixture model".into(), route:HostCapabilityRoute::Model,
        availability:CapabilityAvailability::Available, exposure:HostCapabilityExposure::Visible,
        permission_domain:None, input_schema:serde_json::json!({"type":"object"}), input_normalization:Default::default(),
        output_policy:HostOutputPolicy {preview_char_limit:4_000,persist_full_output:true,allow_artifacts:true},
        supports_streaming:true,supports_argument_streaming:false,parallelism:HostCapabilityParallelism::Serial,
        cancellation:HostCancellationMode::Cooperative,replay_safety:HostReplaySafety::Recoverable,
    });
    AgentCoreConfig {
        turn_identity: model_identity::host_identity(),
        session_id: Some(SessionId::from_string("goal-session-1".to_string())),
        model:Some("fixture-model".into()), host_catalog:Some(catalog),
        ..AgentCoreConfig::default()
    }
}

fn requirement() -> GoalRequirement {
    GoalRequirement {
        acceptance_policy: nola_core::agent::goal::GoalAcceptancePolicy::ModelJudgment,
        semantic_revision: 1, id: "requirement-1".to_string(),
        description: "The durable output is independently observed".to_string(),
        dependencies: Vec::new(),
        status: GoalRecordStatus::Completed,
        evidence_refs: vec!["untrusted-executor-claim".to_string()],
    }
}

fn submission(operation_id: &str, command: GoalRuntimeCommand) -> AgentSubmission {
    AgentSubmission::GoalRuntime {
        envelope: GoalRuntimeCommandEnvelope {
            operation_id: operation_id.to_string(),
            command,
        },
    }
}

fn start_submission(operation_id: &str) -> AgentSubmission {
    submission(
        operation_id,
        GoalRuntimeCommand::Start {
            id: "goal-1".to_string(),
            objective: "Complete a recoverable long-running task".to_string(),
            constraints: vec!["Only audited evidence may advance completion".to_string()],
            requirements: vec![requirement()],
            now_epoch_ms: 100,
        },
    )
}

#[test]
fn goal_runtime_survives_checkpoint_restore_and_replay_is_idempotent() {
    let config = core_config();
    let mut core = AgentCore::try_new(config.clone()).expect("create Core");

    core.submit(start_submission("goal-start-1"))
        .expect("start Goal");
    let scheduled = core
        .submit(submission(
            "goal-schedule-1",
            GoalRuntimeCommand::ScheduleResume {
                wake_at_epoch_ms: 86_400_000,
                now_epoch_ms: 200,
            },
        ))
        .expect("schedule Goal resume");

    assert_eq!(scheduled.snapshot.goal.revision, 2);
    assert_eq!(
        scheduled
            .snapshot
            .goal
            .active
            .as_ref()
            .map(|goal| goal.status),
        Some(GoalStatus::Paused)
    );

    let checkpoint: CoreCheckpoint<AgentThreadState> = core.checkpoint(
        scheduled
            .durability
            .as_ref()
            .expect("Goal runtime mutation must request durability")
            .checkpoint_reason,
    );
    let mut restored = AgentCore::restore(config, checkpoint).expect("restore Goal checkpoint");
    assert_eq!(
        restored
            .snapshot()
            .goal
            .active
            .as_ref()
            .and_then(|goal| goal.next_wake_at_epoch_ms),
        Some(86_400_000)
    );

    let replay = restored
        .submit(submission(
            "goal-schedule-1",
            GoalRuntimeCommand::ScheduleResume {
                wake_at_epoch_ms: 86_400_000,
                now_epoch_ms: 200,
            },
        ))
        .expect("replay accepted operation");
    assert_eq!(replay.snapshot.goal.revision, 2);
    assert!(replay.event_envelopes.is_empty());
    assert!(replay.host_requests.is_empty());
    assert_eq!(replay.snapshot.revision, scheduled.snapshot.revision);
    let before = restored.snapshot().goal;
    assert!(restored.submit(submission("goal-schedule-1", GoalRuntimeCommand::ScheduleResume {
        wake_at_epoch_ms:86_400_000,now_epoch_ms:201,
    })).is_err());
    assert_eq!(restored.snapshot().goal,before);
}

#[test]
fn goal_operations_do_not_share_id_only_input_deduplication() {
    use nola_core::agent::{AgentItem, ItemId};
    use nola_core::agent::protocol::AgentInputDelivery;
    let input = || AgentSubmission::UserInput {
        items:vec![AgentItem::user_message(ItemId::from_string("input-item".into()),"Hello")],
        turn_context_items:vec![],turn_context_platform:Default::default(),turn_context_components:vec![],
        submission_id:Some("shared-id".into()),submitted_at_epoch_ms:None,delivery:AgentInputDelivery::CurrentTurn,
    };
    for input_first in [true,false] {
        let mut core = AgentCore::try_new(core_config()).unwrap();
        if input_first { core.submit(input()).unwrap(); }
        let goal = core.submit(start_submission("shared-id")).unwrap();
        assert_eq!(goal.snapshot.goal.active.as_ref().unwrap().id,"goal-1");
        if !input_first {
            let accepted = core.submit(input()).unwrap();
            assert!(!accepted.event_envelopes.is_empty(),"Goal ID cannot swallow an unrelated input");
        }
        let before = core.snapshot().goal;
        let changed = submission("shared-id",GoalRuntimeCommand::Cancel { now_epoch_ms:101 });
        assert!(core.submit(changed).is_err(),"Changed Goal command must reach content-bound validation");
        assert_eq!(core.snapshot().goal,before);
    }
}

#[test]
fn public_runtime_rejects_executor_self_certification_and_accepts_independent_audit() {
    let mut core = AgentCore::try_new(core_config()).expect("create Core");
    core.submit(start_submission("goal-start-1"))
        .expect("start Goal");
    core.submit(submission(
        "goal-contract-1",
        GoalRuntimeCommand::PlanContract {
            contract: GoalSubtaskContract { execution_run_id: None,
                requirement_versions: std::collections::BTreeMap::from([("requirement-1".into(), 1)]),
                id: "contract-1".to_string(),
                round: 1,
                objective: "Create the durable output".to_string(),
                proposed_checks: vec!["A read-only verifier observes the output".to_string()],
                additional_execution_constraints: vec!["Do not treat executor claims as evidence".to_string()],
                relevant_record_ids: vec!["requirement-1".to_string()],
                status: GoalContractStatus::Audited,
                created_at_epoch_ms: 200,
            },
            now_epoch_ms: 200,
        },
    ))
    .expect("plan contract");
    core.submit(submission(
        "goal-execution-start-1",
        GoalRuntimeCommand::BeginExecution { run_id: "contract-run".into(),
            contract_id: "contract-1".to_string(),
            now_epoch_ms: 300,
        },
    ))
    .expect("begin execution");
    core.submit(submission(
        "goal-execution-report-1",
        GoalRuntimeCommand::RecordExecution {
            report: GoalExecutionReport {
                contract_id: "contract-1".to_string(),
                summary: "Executor claims the output is ready".to_string(),
                claimed_record_ids: vec!["requirement-1".to_string()],
                artifact_refs: Vec::new(),
                blockers: Vec::new(),
                reported_at_epoch_ms: 400,
            },
        },
    ))
    .expect("record executor report");

    assert!(core
        .submit(submission(
            "goal-premature-completion-1",
            GoalRuntimeCommand::Complete { now_epoch_ms: 450 },
        ))
        .is_err());

    core.submit(submission(
        "goal-audit-1",
        GoalRuntimeCommand::ApplyAudit {
            report: GoalAuditReport {
                assessor: nola_core::agent::goal::GoalAssessor::RuntimeOperation { operation_id: "untrusted-external-audit".into() }, expected_goal_revision: core.snapshot().goal.revision,
                id: "audit-1".to_string(),
                contract_id: "contract-1".to_string(),
                verdict: GoalAuditVerdict::Passed,
                summary: "The read-only verifier observed the durable output".to_string(),
                findings: vec![GoalAuditFinding {
                    requirement_revision: Some(1), assessment_method: nola_core::agent::goal::GoalAssessmentMethod::EvidenceInspection,
                    record_kind: GoalRecordKind::Requirement,
                    record_id: "requirement-1".to_string(),
                    status: GoalRecordStatus::Completed,
                    evidence_refs: vec!["evidence-1".to_string()],
                    summary: "Observed expected output".to_string(),
                }],
                unmet_requirement_ids: Vec::new(),
                integrity_violations: Vec::new(),
                evidence: vec![GoalEvidence {
                    id: "evidence-1".to_string(),
                    source: "read-only environment verifier".to_string(),
                    summary: "Output exists and satisfies its acceptance criteria".to_string(),
                    observed_at_epoch_ms: 500,
                    source_verified: true,
                }],
                audited_at_epoch_ms: 500,
            },
        },
    ))
    .expect_err("self-declared inspection cannot certify Goal completion");
    core
        .submit(submission(
            "goal-complete-1",
            GoalRuntimeCommand::Complete { now_epoch_ms: 600 },
        ))
        .expect_err("rejected inspection did not create acceptance");
    assert_ne!(core.snapshot().goal.active.as_ref().unwrap().status, GoalStatus::Completed);
}
