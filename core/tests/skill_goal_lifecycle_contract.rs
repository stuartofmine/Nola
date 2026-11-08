#[path = "support/model_identity.rs"]
mod model_identity;
use nola_core::agent::capability::{
    SkillArtifactContract, SkillDescriptor, SkillExecutionContract, SkillExecutionMode,
    SkillProgressStep, SkillRunBudget, SkillRunOwner, SkillRunStatus, SkillRuntimeCommand,
    SkillRuntimeCommandEnvelope, SkillRuntimeState, SkillVerificationResult, SkillVerifierContract,
    SkillVerificationEvidence, SkillVerificationMethod,
    SKILL_EXECUTION_CONTRACT_SCHEMA_VERSION,
};
use nola_core::agent::{
    AgentCore, AgentCoreConfig, AgentSubmission, GoalAuditFinding, GoalAuditReport,
    GoalAuditVerdict, GoalContractStatus, GoalEvidence, GoalExecutionReport, GoalRecordKind,
    GoalRecordStatus, GoalRequirement, GoalRuntimeCommand, GoalRuntimeCommandEnvelope, GoalStatus,
    GoalSubtaskContract, SessionId,
};

fn skill_command(state: &mut SkillRuntimeState, operation_id: &str, command: SkillRuntimeCommand) {
    state
        .apply(SkillRuntimeCommandEnvelope {
            operation_id: operation_id.to_string(),
            command,
        })
        .expect("skill transition");
}

fn goal_submission(operation_id: &str, command: GoalRuntimeCommand) -> AgentSubmission {
    AgentSubmission::GoalRuntime {
        envelope: GoalRuntimeCommandEnvelope {
            operation_id: operation_id.to_string(),
            command,
        },
    }
}

#[test]
fn recovered_skill_output_requires_independent_goal_audit_before_completion() {
    let digest = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let descriptor = SkillDescriptor {
        skill_id: "document-build".to_string(),
        version: "1.0.0".to_string(),
        display_name: "Document build".to_string(),
        instructions_ref: format!("skill://document-build/1.0.0/SKILL.md#sha256={digest}"),
        instructions_digest: digest.to_string(),
        required_tools: Vec::new(),
        default_mode: Some(SkillExecutionMode::Fork),
        execution_contract: SkillExecutionContract {
            schema_version: SKILL_EXECUTION_CONTRACT_SCHEMA_VERSION,
            progress_steps: vec![SkillProgressStep {
                id: "render".to_string(),
                title: "Render the document".to_string(),
                required: true,
            }],
            artifacts: vec![SkillArtifactContract {
                id: "document".to_string(),
                media_type:
                    "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                        .to_string(),
                required: true,
                description: "Final document".to_string(),
            }],
            verifiers: vec![SkillVerifierContract {
                id: "render-check".to_string(),
                method: SkillVerificationMethod::ModelJudgment,
                description: "Rendered pages contain the requested content".to_string(),
                required: true,
            }],
            resume_points: vec!["artifact-written".to_string()],
        },
    };
    let mut skill_state = SkillRuntimeState::default();
    skill_command(
        &mut skill_state,
        "skill-start",
        SkillRuntimeCommand::Start {
            run_id: "skill-run-1".to_string(),
            owner: SkillRunOwner {
                session_id: "session-1".to_string(),
                parent_turn_id: "turn-1".to_string(),
                child_turn_id: Some("turn-1:document-build".to_string()),
            },
            descriptor,
            mode: SkillExecutionMode::Fork,
            budget: SkillRunBudget::default(),
        },
    );
    skill_command(
        &mut skill_state,
        "skill-progress",
        SkillRuntimeCommand::RecordProgress {
            run_id: "skill-run-1".to_string(),
            step_id: "render".to_string(),
            completed: true,
        },
    );
    skill_command(
        &mut skill_state,
        "skill-artifact",
        SkillRuntimeCommand::RegisterArtifact {
            run_id: "skill-run-1".to_string(),
            artifact_id: "document".to_string(),
            artifact_ref: "artifact://document/final.docx".to_string(),
        },
    );
    skill_command(
        &mut skill_state,
        "skill-resume-point",
        SkillRuntimeCommand::SetResumePoint {
            run_id: "skill-run-1".to_string(),
            resume_point: "artifact-written".to_string(),
        },
    );

    let checkpoint = serde_json::to_vec(&skill_state).expect("serialize skill checkpoint");
    let mut recovered_skill: SkillRuntimeState =
        serde_json::from_slice(&checkpoint).expect("restore skill checkpoint");
    assert_eq!(
        recovered_skill.runs["skill-run-1"].resume_point.as_deref(),
        Some("artifact-written")
    );
    assert!(recovered_skill
        .apply(SkillRuntimeCommandEnvelope {
            operation_id: "skill-premature-complete".to_string(),
            command: SkillRuntimeCommand::Complete {
                run_id: "skill-run-1".to_string(),
                result_ref: Some("artifact://document/final.docx".to_string()),
            },
        })
        .is_err());
    skill_command(
        &mut recovered_skill,
        "skill-verification",
        SkillRuntimeCommand::RecordVerification {
            run_id: "skill-run-1".to_string(),
            result: SkillVerificationResult {
                verifier_id: "render-check".to_string(),
                passed: true,
                evidence: SkillVerificationEvidence::ModelJudgment { sources: vec![] },
                replaces_evidence: None,
                summary: "Model assessment of rendered output".to_string(),
            },
        },
    );
    skill_command(
        &mut recovered_skill,
        "skill-complete",
        SkillRuntimeCommand::Complete {
            run_id: "skill-run-1".to_string(),
            result_ref: Some("artifact://document/final.docx".to_string()),
        },
    );
    assert_eq!(
        recovered_skill.runs["skill-run-1"].status,
        SkillRunStatus::Completed
    );

    let mut core = AgentCore::try_new(AgentCoreConfig {
        turn_identity: model_identity::host_identity(),
        session_id: Some(SessionId::from_string("session-1".to_string())),
        ..AgentCoreConfig::default()
    })
    .expect("create Core");
    core.submit(goal_submission(
        "goal-start",
        GoalRuntimeCommand::Start {
            id: "goal-1".to_string(),
            objective: "Produce a verified document".to_string(),
            constraints: vec!["Executor output cannot self-certify completion".to_string()],
            requirements: vec![GoalRequirement {
                acceptance_policy: nola_core::agent::goal::GoalAcceptancePolicy::ModelJudgment,
                semantic_revision: 1, id: "document-verified".to_string(),
                description: "The rendered document is independently verified".to_string(),
                dependencies: Vec::new(),
                status: GoalRecordStatus::Pending,
                evidence_refs: Vec::new(),
            }],
            now_epoch_ms: 100,
        },
    ))
    .expect("start Goal");
    core.submit(goal_submission(
        "goal-contract",
        GoalRuntimeCommand::PlanContract {
            contract: GoalSubtaskContract { execution_run_id: None,
                requirement_versions: std::collections::BTreeMap::from([("document-verified".into(), 1)]),
                id: "contract-1".to_string(),
                round: 1,
                objective: "Build and verify the document".to_string(),
                proposed_checks: vec!["render-check passes".to_string()],
                additional_execution_constraints: vec!["Use independent verifier evidence".to_string()],
                relevant_record_ids: vec!["document-verified".to_string()],
                status: GoalContractStatus::Audited,
                created_at_epoch_ms: 200,
            },
            now_epoch_ms: 200,
        },
    ))
    .expect("plan Goal contract");
    core.submit(goal_submission(
        "goal-execution-start",
        GoalRuntimeCommand::BeginExecution { run_id: "contract-run".into(),
            contract_id: "contract-1".to_string(),
            now_epoch_ms: 300,
        },
    ))
    .expect("begin Goal execution");
    core.submit(goal_submission(
        "goal-execution-report",
        GoalRuntimeCommand::RecordExecution {
            report: GoalExecutionReport {
                contract_id: "contract-1".to_string(),
                summary: "Skill reports a completed document".to_string(),
                claimed_record_ids: vec!["document-verified".to_string()],
                artifact_refs: vec!["artifact://document/final.docx".to_string()],
                blockers: Vec::new(),
                reported_at_epoch_ms: 400,
            },
        },
    ))
    .expect("record execution report");
    assert!(core
        .submit(goal_submission(
            "goal-self-certified-complete",
            GoalRuntimeCommand::Complete { now_epoch_ms: 450 },
        ))
        .is_err());

    core.submit(goal_submission(
        "goal-independent-audit",
        GoalRuntimeCommand::ApplyAudit {
            report: GoalAuditReport {
                assessor: nola_core::agent::goal::GoalAssessor::RuntimeOperation { operation_id: "untrusted-external-audit".into() }, expected_goal_revision: core.snapshot().goal.revision,
                id: "audit-1".to_string(),
                contract_id: "contract-1".to_string(),
                verdict: GoalAuditVerdict::Passed,
                summary: "Verifier evidence confirms the rendered output".to_string(),
                findings: vec![GoalAuditFinding {
                    requirement_revision: Some(1), assessment_method: nola_core::agent::goal::GoalAssessmentMethod::EvidenceInspection,
                    record_kind: GoalRecordKind::Requirement,
                    record_id: "document-verified".to_string(),
                    status: GoalRecordStatus::Completed,
                    evidence_refs: vec!["evidence://render-check/1".to_string()],
                    summary: "Rendered pages satisfy acceptance criteria".to_string(),
                }],
                unmet_requirement_ids: Vec::new(),
                integrity_violations: Vec::new(),
                evidence: vec![GoalEvidence {
                    id: "evidence://render-check/1".to_string(),
                    source: "skill verifier render-check".to_string(),
                    summary: "Rendered output inspected independently".to_string(),
                    observed_at_epoch_ms: 500,
                    source_verified: true,
                }],
                audited_at_epoch_ms: 500,
            },
        },
    ))
    .expect_err("self-declared inspection is not canonical acceptance");
    core
        .submit(goal_submission(
            "goal-complete",
            GoalRuntimeCommand::Complete { now_epoch_ms: 600 },
        ))
        .expect_err("Skill reporting does not create Goal acceptance");
    let goal = core.snapshot().goal.active.expect("active Goal");
    assert_ne!(goal.status, GoalStatus::Completed);
}
