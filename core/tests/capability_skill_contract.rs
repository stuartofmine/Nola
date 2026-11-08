#[path = "support/model_identity.rs"]
mod model_identity;
use nola_core::agent::capability::*;
use nola_core::agent::host::{
    CapabilityAvailability, HostCancellationMode, HostCapabilityCatalog, HostCapabilityDescriptor,
    HostCapabilityExposure, HostCapabilityParallelism, HostCapabilityRoute, HostId,
    HostOutputPolicy, HostReplaySafety, HostRequest, HostRequestEnvelope, HostResponse,
    HostResponseEnvelope,
};
use nola_core::agent::recovery::CoreCheckpoint;
use nola_core::agent::{
    AgentCore, AgentCoreConfig, AgentEvent, AgentSubmission, AgentThreadState, SessionId,
};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

fn catalog(revision: u64, include_search: bool) -> CapabilityCatalog {
    let descriptor = |name: &str| McpToolDescriptor {
        name: name.to_string(),
        title: None,
        description: None,
        input_schema: json!({"type":"object"}),
        output_schema: None,
        read_only: true,
        destructive: false,
    };
    let mut tools = BTreeMap::from([("read_paper".to_string(), descriptor("read_paper"))]);
    if include_search {
        tools.insert("search_papers".to_string(), descriptor("search_papers"));
    }
    let server = McpServerDescriptor {
        server_id: "research".to_string(),
        display_name: "Research".to_string(),
        transport: McpTransportKind::StreamableHttp,
        connection_ref: "host-connection:research".to_string(),
        availability: McpServerAvailability::Available,
        protocol_version: Some("2025-06-18".to_string()),
        instructions: None,
        tools,
        resources: BTreeMap::from([(
            "paper-42".to_string(),
            McpResourceDescriptor {
                resource_id: "paper-42".to_string(),
                uri: "arxiv://paper/42".to_string(),
                name: "Paper 42".to_string(),
                description: None,
                mime_type: Some("application/pdf".to_string()),
                annotations: BTreeMap::new(),
            },
        )]),
        resource_templates: BTreeMap::new(),
        prompts: BTreeMap::from([(
            "review".to_string(),
            McpPromptDescriptor {
                name: "review".to_string(),
                title: Some("Review paper".to_string()),
                description: None,
                arguments: vec![McpPromptArgumentDescriptor {
                    name: "depth".to_string(),
                    description: None,
                    required: false,
                }],
            },
        )]),
    };
    CapabilityCatalog {
        revision,
        builtin_tools: BTreeSet::from(["read_file".to_string()]),
        mcp: McpCapabilityCatalog {
            revision,
            servers: BTreeMap::from([("research".to_string(), server)]),
        },
    }
}

fn skill(required_tools: Vec<CapabilityToolReference>) -> SkillDescriptor {
    let digest = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    SkillDescriptor {
        skill_id: "paper-review".to_string(),
        version: "1.0.0".to_string(),
        display_name: "Paper review".to_string(),
        instructions_ref: format!("skill://paper-review/1.0.0/SKILL.md#sha256={digest}"),
        instructions_digest: digest.to_string(),
        required_tools,
        default_mode: Some(SkillExecutionMode::Fork),
        execution_contract: SkillExecutionContract::default(),
    }
}

fn owner(mode: SkillExecutionMode, suffix: &str) -> SkillRunOwner {
    SkillRunOwner {
        session_id: "session-1".to_string(),
        parent_turn_id: "turn-1".to_string(),
        child_turn_id: (mode == SkillExecutionMode::Fork).then(|| format!("turn-1:{suffix}")),
    }
}

fn apply(
    state: &mut SkillRuntimeState,
    id: &str,
    command: SkillRuntimeCommand,
) -> Vec<SkillRuntimeEvent> {
    state
        .apply(SkillRuntimeCommandEnvelope {
            operation_id: id.to_string(),
            command,
        })
        .expect("transition")
        .events
        .into_iter()
        .map(|event| event.event)
        .collect()
}

#[test]
fn mcp_server_resource_prompt_and_tool_resolution_is_exact_and_typed() {
    let catalog = catalog(7, true);
    assert_eq!(
        catalog
            .mcp
            .resolve_tool(&McpToolReference {
                server_id: "research".to_string(),
                tool_name: "search_papers".to_string(),
            })
            .expect("tool")
            .name,
        "search_papers"
    );
    assert_eq!(
        catalog
            .mcp
            .resolve_resource(&McpResourceReference {
                server_id: "research".to_string(),
                resource_id: "paper-42".to_string(),
            })
            .expect("resource")
            .uri,
        "arxiv://paper/42"
    );
    assert_eq!(
        catalog
            .mcp
            .resolve_prompt(&McpPromptReference {
                server_id: "research".to_string(),
                prompt_name: "review".to_string(),
            })
            .expect("prompt")
            .arguments[0]
            .name,
        "depth"
    );
    let error = catalog
        .mcp
        .resolve_tool(&McpToolReference {
            server_id: "research".to_string(),
            tool_name: "search".to_string(),
        })
        .expect_err("partial names do not resolve");
    assert_eq!(error.kind, CapabilityLookupKind::Tool);
    assert_eq!(error.recovery, CapabilityRecovery::RefreshCatalogThenRetry);
    assert_eq!(error.catalog_revision, 7);
}

#[test]
fn unversioned_mcp_catalog_and_server_fail_with_recoverable_errors() {
    let reference = McpToolReference {
        server_id: "research".to_string(),
        tool_name: "read_paper".to_string(),
    };
    let mut unversioned_catalog = catalog(7, true).mcp;
    unversioned_catalog.revision = 0;
    let error = unversioned_catalog
        .resolve_tool(&reference)
        .expect_err("an unversioned catalog must not resolve resources");
    assert_eq!(error.code, "unversioned_mcp_catalog");
    assert_eq!(error.recovery, CapabilityRecovery::RefreshCatalogThenRetry);

    let mut unversioned_server = catalog(7, true).mcp;
    unversioned_server
        .servers
        .get_mut("research")
        .unwrap()
        .protocol_version = None;
    let error = unversioned_server
        .resolve_tool(&reference)
        .expect_err("an unversioned server must not resolve resources");
    assert_eq!(error.code, "unversioned_mcp_server");
    assert_eq!(error.catalog_revision, 7);
}

#[test]
fn catalog_content_cannot_change_without_a_revision_increase() {
    let mut state = SkillRuntimeState::default();
    apply(
        &mut state,
        "catalog-1",
        SkillRuntimeCommand::ReplaceCatalog {
            catalog: catalog(1, false),
        },
    );
    let error = state
        .apply(SkillRuntimeCommandEnvelope {
            operation_id: "catalog-1-conflict".to_string(),
            command: SkillRuntimeCommand::ReplaceCatalog {
                catalog: catalog(1, true),
            },
        })
        .expect_err("same revision must identify the same catalog content");
    assert_eq!(error.code, "catalog_revision_conflict");
}

#[test]
fn enclosing_and_mcp_catalog_revisions_must_match() {
    let mut state = SkillRuntimeState::default();
    let mut mismatched = catalog(3, true);
    mismatched.mcp.revision = 2;
    let error = state
        .apply(SkillRuntimeCommandEnvelope {
            operation_id: "catalog-mismatch".to_string(),
            command: SkillRuntimeCommand::ReplaceCatalog {
                catalog: mismatched,
            },
        })
        .expect_err("mixed catalog revisions must not enter runtime state");
    assert_eq!(error.code, "mcp_catalog_revision_mismatch");
}

#[test]
fn skill_completion_requires_declared_progress_artifact_and_verification() {
    let mut state = SkillRuntimeState::default();
    let mut descriptor = skill(Vec::new());
    descriptor.execution_contract = SkillExecutionContract {
        schema_version: SKILL_EXECUTION_CONTRACT_SCHEMA_VERSION,
        progress_steps: vec![SkillProgressStep {
            id: "render".to_string(),
            title: "Render document".to_string(),
            required: true,
        }],
        artifacts: vec![SkillArtifactContract {
            id: "document".to_string(),
            media_type: "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                .to_string(),
            required: true,
            description: "Final Word document".to_string(),
        }],
        verifiers: vec![SkillVerifierContract {
            id: "render-check".to_string(),
            method: SkillVerificationMethod::ModelJudgment,
            description: "Rendered pages contain the expected content".to_string(),
            required: true,
        }],
        resume_points: vec!["after-render".to_string()],
    };
    apply(
        &mut state,
        "start-contract-run",
        SkillRuntimeCommand::Start {
            run_id: "contract-run".to_string(),
            owner: owner(SkillExecutionMode::Fork, "contract"),
            descriptor,
            mode: SkillExecutionMode::Fork,
            budget: SkillRunBudget::default(),
        },
    );

    let premature = state
        .apply(SkillRuntimeCommandEnvelope {
            operation_id: "complete-prematurely".to_string(),
            command: SkillRuntimeCommand::Complete {
                run_id: "contract-run".to_string(),
                result_ref: Some("artifact://document".to_string()),
            },
        })
        .expect_err("completion must be contract-gated");
    assert_eq!(premature.code, "skill_completion_contract_unsatisfied");

    apply(
        &mut state,
        "progress-render",
        SkillRuntimeCommand::RecordProgress {
            run_id: "contract-run".to_string(),
            step_id: "render".to_string(),
            completed: true,
        },
    );
    apply(
        &mut state,
        "artifact-document",
        SkillRuntimeCommand::RegisterArtifact {
            run_id: "contract-run".to_string(),
            artifact_id: "document".to_string(),
            artifact_ref: "artifact://document".to_string(),
        },
    );
    let duplicate = state
        .apply(SkillRuntimeCommandEnvelope {
            operation_id: "artifact-document".to_string(),
            command: SkillRuntimeCommand::RegisterArtifact {
                run_id: "contract-run".to_string(),
                artifact_id: "document".to_string(),
                artifact_ref: "artifact://document".to_string(),
            },
        })
        .unwrap();
    assert!(duplicate.duplicate);
    assert!(duplicate.events.is_empty());
    apply(
        &mut state,
        "verify-render",
        SkillRuntimeCommand::RecordVerification {
            run_id: "contract-run".to_string(),
            result: SkillVerificationResult {
                verifier_id: "render-check".to_string(),
                passed: true,
                evidence: SkillVerificationEvidence::ModelJudgment { sources: vec![] },
                replaces_evidence: None,
                summary: "3 pages inspected".to_string(),
            },
        },
    );
    apply(
        &mut state,
        "resume-after-render",
        SkillRuntimeCommand::SetResumePoint {
            run_id: "contract-run".to_string(),
            resume_point: "after-render".to_string(),
        },
    );
    apply(
        &mut state,
        "complete-contract-run",
        SkillRuntimeCommand::Complete {
            run_id: "contract-run".to_string(),
            result_ref: Some("artifact://document".to_string()),
        },
    );

    let run = &state.runs["contract-run"];
    assert_eq!(run.status, SkillRunStatus::Completed);
    assert_eq!(run.current_progress_step, None);
    assert_eq!(run.artifacts["document"], "artifact://document");
    assert!(run.verifications["render-check"].passed);
    assert_eq!(run.resume_point.as_deref(), Some("after-render"));
}

#[test]
fn failed_required_verification_preserves_evidence_and_blocks_skill_completion() {
    let mut state = SkillRuntimeState::default();
    let mut descriptor = skill(Vec::new());
    descriptor.execution_contract = SkillExecutionContract {
        schema_version: SKILL_EXECUTION_CONTRACT_SCHEMA_VERSION,
        progress_steps: vec![SkillProgressStep {
            id: "render".to_string(),
            title: "Render document".to_string(),
            required: true,
        }],
        artifacts: vec![SkillArtifactContract {
            id: "document".to_string(),
            media_type: "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                .to_string(),
            required: true,
            description: "Rendered document".to_string(),
        }],
        verifiers: vec![SkillVerifierContract {
            id: "render-check".to_string(),
            method: SkillVerificationMethod::ModelJudgment,
            description: "Every rendered page is readable".to_string(),
            required: true,
        }],
        resume_points: Vec::new(),
    };
    apply(
        &mut state,
        "failed-verifier-start",
        SkillRuntimeCommand::Start {
            run_id: "failed-verifier-run".to_string(),
            owner: owner(SkillExecutionMode::Inline, "unused"),
            descriptor,
            mode: SkillExecutionMode::Inline,
            budget: SkillRunBudget::default(),
        },
    );
    apply(
        &mut state,
        "failed-verifier-progress",
        SkillRuntimeCommand::RecordProgress {
            run_id: "failed-verifier-run".to_string(),
            step_id: "render".to_string(),
            completed: true,
        },
    );
    apply(
        &mut state,
        "failed-verifier-artifact",
        SkillRuntimeCommand::RegisterArtifact {
            run_id: "failed-verifier-run".to_string(),
            artifact_id: "document".to_string(),
            artifact_ref: "artifact://document/broken.docx".to_string(),
        },
    );
    apply(
        &mut state,
        "failed-verifier-result",
        SkillRuntimeCommand::RecordVerification {
            run_id: "failed-verifier-run".to_string(),
            result: SkillVerificationResult {
                verifier_id: "render-check".to_string(),
                passed: false,
                evidence: SkillVerificationEvidence::ModelJudgment { sources: vec![] },
                replaces_evidence: None,
                summary: "Page 2 is clipped".to_string(),
            },
        },
    );

    let error = state
        .apply(SkillRuntimeCommandEnvelope {
            operation_id: "failed-verifier-complete".to_string(),
            command: SkillRuntimeCommand::Complete {
                run_id: "failed-verifier-run".to_string(),
                result_ref: Some("artifact://document/broken.docx".to_string()),
            },
        })
        .expect_err("a failed required verifier must block completion");
    assert_eq!(error.code, "skill_completion_contract_unsatisfied");
    let run = &state.runs["failed-verifier-run"];
    assert_eq!(run.status, SkillRunStatus::Running);
    assert!(!run.verifications["render-check"].passed);
    assert_eq!(
        run.verifications["render-check"].evidence,
        SkillVerificationEvidence::ModelJudgment { sources: vec![] }
    );
}

#[test]
fn unknown_tool_waits_for_catalog_refresh_and_resumes() {
    let required = CapabilityToolReference::Mcp {
        server_id: "research".to_string(),
        tool_name: "search_papers".to_string(),
    };
    let mut state = SkillRuntimeState::default();
    apply(
        &mut state,
        "catalog-1",
        SkillRuntimeCommand::ReplaceCatalog {
            catalog: catalog(1, false),
        },
    );
    let events = apply(
        &mut state,
        "start",
        SkillRuntimeCommand::Start {
            run_id: "run".to_string(),
            owner: owner(SkillExecutionMode::Inline, "unused"),
            descriptor: skill(vec![required]),
            mode: SkillExecutionMode::Inline,
            budget: SkillRunBudget::default(),
        },
    );
    assert_eq!(state.runs["run"].status, SkillRunStatus::WaitingForCatalog);
    assert!(
        matches!(&events[1], SkillRuntimeEvent::CapabilityRefreshRequested { errors, .. }
        if errors[0].requested_id == "search_papers")
    );
    let resumed = apply(
        &mut state,
        "catalog-2",
        SkillRuntimeCommand::ReplaceCatalog {
            catalog: catalog(2, true),
        },
    );
    assert_eq!(state.runs["run"].status, SkillRunStatus::Running);
    assert!(resumed.iter().any(|event| matches!(event,
        SkillRuntimeEvent::Resumed { run_id, catalog_revision: 2 } if run_id == "run")));
}

#[test]
fn start_rejects_a_skill_reference_that_does_not_attest_to_loaded_content() {
    let mut state = SkillRuntimeState::default();
    let mut descriptor = skill(Vec::new());
    descriptor.instructions_digest =
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string();
    let error = state
        .apply(SkillRuntimeCommandEnvelope {
            operation_id: "start-invalid-content".to_string(),
            command: SkillRuntimeCommand::Start {
                run_id: "invalid-content".to_string(),
                owner: owner(SkillExecutionMode::Inline, "unused"),
                descriptor,
                mode: SkillExecutionMode::Inline,
                budget: SkillRunBudget::default(),
            },
        })
        .expect_err("mismatched content identity must not start");
    assert_eq!(error.code, "invalid_skill_content_identity");
    assert!(state.runs.is_empty());
}

#[test]
fn inline_and_fork_runs_have_independent_budgets_and_turns() {
    let mut state = SkillRuntimeState::default();
    apply(
        &mut state,
        "catalog",
        SkillRuntimeCommand::ReplaceCatalog {
            catalog: catalog(1, true),
        },
    );
    let budget = SkillRunBudget {
        max_model_turns: 1,
        max_tool_calls: 1,
        max_elapsed_millis: Some(100),
    };
    for (id, mode) in [
        ("inline", SkillExecutionMode::Inline),
        ("fork", SkillExecutionMode::Fork),
    ] {
        apply(
            &mut state,
            &format!("start-{id}"),
            SkillRuntimeCommand::Start {
                run_id: id.to_string(),
                owner: owner(mode, id),
                descriptor: skill(Vec::new()),
                mode,
                budget: budget.clone(),
            },
        );
    }
    apply(
        &mut state,
        "inline-turn-1",
        SkillRuntimeCommand::RecordModelTurn {
            run_id: "inline".to_string(),
        },
    );
    let exceeded = apply(
        &mut state,
        "inline-turn-2",
        SkillRuntimeCommand::RecordModelTurn {
            run_id: "inline".to_string(),
        },
    );
    assert!(matches!(
        exceeded[0],
        SkillRuntimeEvent::BudgetExceeded {
            dimension: SkillBudgetDimension::ModelTurns,
            ..
        }
    ));
    assert_eq!(state.runs["inline"].status, SkillRunStatus::Failed);
    assert_eq!(state.runs["fork"].status, SkillRunStatus::Running);
    assert_eq!(state.runs["fork"].usage.model_turns, 0);
    assert_eq!(
        state.runs["fork"].owner.child_turn_id.as_deref(),
        Some("turn-1:fork")
    );
}

#[test]
fn checkpoint_replay_is_deterministic_and_operations_are_exactly_once() {
    let mut original = SkillRuntimeState::default();
    apply(
        &mut original,
        "catalog",
        SkillRuntimeCommand::ReplaceCatalog {
            catalog: catalog(3, true),
        },
    );
    apply(
        &mut original,
        "start",
        SkillRuntimeCommand::Start {
            run_id: "fork".to_string(),
            owner: owner(SkillExecutionMode::Fork, "fork"),
            descriptor: skill(Vec::new()),
            mode: SkillExecutionMode::Fork,
            budget: SkillRunBudget::default(),
        },
    );
    let bytes = serde_json::to_vec(&original).expect("checkpoint");
    let mut restored: SkillRuntimeState = serde_json::from_slice(&bytes).expect("restore");
    let command = SkillRuntimeCommandEnvelope {
        operation_id: "tool-1".to_string(),
        command: SkillRuntimeCommand::RecordToolCall {
            run_id: "fork".to_string(),
            tool: CapabilityToolReference::Builtin {
                tool_name: "read_file".to_string(),
            },
        },
    };
    assert_eq!(
        original.apply(command.clone()).expect("original"),
        restored.apply(command.clone()).expect("restored")
    );
    assert_eq!(original, restored);
    let duplicate = restored.apply(command).expect("duplicate");
    assert!(duplicate.duplicate && duplicate.events.is_empty());
    assert_eq!(restored.runs["fork"].usage.tool_calls, 1);
}

#[test]
fn completed_run_absorbs_late_terminal_updates() {
    let mut state = SkillRuntimeState::default();
    apply(
        &mut state,
        "catalog",
        SkillRuntimeCommand::ReplaceCatalog {
            catalog: catalog(1, true),
        },
    );
    apply(
        &mut state,
        "start",
        SkillRuntimeCommand::Start {
            run_id: "inline".to_string(),
            owner: owner(SkillExecutionMode::Inline, "unused"),
            descriptor: skill(Vec::new()),
            mode: SkillExecutionMode::Inline,
            budget: SkillRunBudget::default(),
        },
    );
    apply(
        &mut state,
        "complete",
        SkillRuntimeCommand::Complete {
            run_id: "inline".to_string(),
            result_ref: Some("artifact://report.docx".to_string()),
        },
    );
    assert!(apply(
        &mut state,
        "late",
        SkillRuntimeCommand::Fail {
            run_id: "inline".to_string(),
            error: "late transport error".to_string(),
        }
    )
    .is_empty());
    assert_eq!(state.runs["inline"].status, SkillRunStatus::Completed);
}

#[test]
fn multilingual_or_ambiguous_user_text_cannot_route_capabilities() {
    let catalog = catalog(9, true);
    for text in [
        "search current tools and permissions",
        "搜索当前技能与权限",
        "analyse les outils actuels",
        "The word search is only being discussed.",
    ] {
        assert!(!text.is_empty());
        assert!(catalog
            .resolve_tool(&CapabilityToolReference::Builtin {
                tool_name: "read_file".to_string(),
            })
            .is_ok());
        assert!(catalog
            .resolve_tool(&CapabilityToolReference::Builtin {
                tool_name: "search".to_string(),
            })
            .is_err());
    }
}

fn host_capability(id: &str, route: HostCapabilityRoute) -> HostCapabilityDescriptor {
    HostCapabilityDescriptor {
        id: id.to_string(),
        title: id.to_string(),
        route,
        availability: CapabilityAvailability::Available,
        exposure: HostCapabilityExposure::Visible,
        permission_domain: None,
        input_schema: json!({"type":"object"}),
        input_normalization: Default::default(),
        output_policy: HostOutputPolicy {
            preview_char_limit: 4_000,
            persist_full_output: true,
            allow_artifacts: false,
        },
        supports_streaming: false,
        supports_argument_streaming: false,
        parallelism: HostCapabilityParallelism::Serial,
        cancellation: HostCancellationMode::Cooperative,
        replay_safety: HostReplaySafety::Recoverable,
    }
}

fn core_config() -> AgentCoreConfig {
    let capabilities = BTreeMap::from([(
        "capability.catalog.refresh".to_string(),
        host_capability("capability.catalog.refresh", HostCapabilityRoute::Tool),
    )]);
    AgentCoreConfig {
        turn_identity: model_identity::host_identity(),
        session_id: Some(SessionId::from_string("skill-core-contract".to_string())),
        host_catalog: Some(HostCapabilityCatalog {
            host_id: HostId("skill-host".to_string()),
            revision: 1,
            availability_epoch: 1,
            lease: None,
            capabilities,
        }),
        ..AgentCoreConfig::default()
    }
}

fn find_request(
    requests: &[HostRequestEnvelope],
    predicate: impl Fn(&HostRequest) -> bool,
) -> HostRequestEnvelope {
    requests
        .iter()
        .find(|request| predicate(&request.payload))
        .cloned()
        .expect("Host request")
}

fn host_response(request: &HostRequestEnvelope, payload: HostResponse) -> HostResponseEnvelope {
    HostResponseEnvelope {
        delegated_invocation_id: request.delegated_invocation_id.clone(),
        attempt_id: None,
        request_id: request.request_id.clone(),
        effect_id: request.effect_id.clone(),
        host_id: request.host_id.clone(),
        lease_id: request.lease_id.clone(),
        session_id: request.session_id.clone(),
        turn_id: request.turn_id.clone(),
        catalog_revision: request.catalog_revision,
        availability_epoch: request.availability_epoch,
        delivery: request.delivery.clone(),
        sequence: 1,
        payload,
    }
}

fn checkpoint(core: &AgentCore) -> CoreCheckpoint<AgentThreadState> {
    core.checkpoint(nola_core::agent::recovery::CheckpointReason::Manual)
}

fn core_skill_submission(id: &str, command: SkillRuntimeCommand) -> AgentSubmission {
    AgentSubmission::SkillRuntime {
        envelope: SkillRuntimeCommandEnvelope {
            operation_id: id.to_string(),
            command,
        },
    }
}

#[test]
fn agent_core_checkpoints_unknown_tool_effect_and_resumes_after_host_catalog_refresh() {
    let config = core_config();
    let mut core = AgentCore::try_new(config.clone()).expect("Core");
    core.submit(core_skill_submission(
        "catalog-1",
        SkillRuntimeCommand::ReplaceCatalog {
            catalog: catalog(1, false),
        },
    ))
    .expect("initial catalog");

    let started = core
        .submit(core_skill_submission(
            "start-waiting",
            SkillRuntimeCommand::Start {
                run_id: "waiting-fork".to_string(),
                owner: owner(SkillExecutionMode::Fork, "waiting-fork"),
                descriptor: skill(vec![CapabilityToolReference::Mcp {
                    server_id: "research".to_string(),
                    tool_name: "search_papers".to_string(),
                }]),
                mode: SkillExecutionMode::Fork,
                budget: SkillRunBudget::default(),
            },
        ))
        .expect("start waiting Skill");
    assert!(started.event_envelopes.iter().any(|envelope| matches!(
        envelope.event,
        AgentEvent::SkillRuntimeUpdated {
            update: SkillRuntimeEvent::CapabilityRefreshRequested { .. },
            ..
        }
    )));
    let refresh = find_request(&started.host_requests, |payload| {
        matches!(
            payload,
            HostRequest::RefreshCapabilityCatalog { run_id, catalog_revision: 1, .. }
                if run_id == "waiting-fork"
        )
    });
    let durable = checkpoint(&core);
    assert_eq!(
        durable.state.skill_runtime.runs["waiting-fork"].status,
        SkillRunStatus::WaitingForCatalog
    );
    assert!(durable.state.host.pending_effects.iter().any(|effect| {
        effect.effect_id == refresh.effect_id.0
            && effect.capability_id == "capability.catalog.refresh"
    }));

    let mut restored = AgentCore::restore(config, durable).expect("restore");
    let replayed = restored
        .submit(AgentSubmission::ResumeHostEffect {
            effect_id: refresh.effect_id.0.clone(),
        })
        .expect("explicitly resume catalog refresh");
    let replayed_refresh = find_request(&replayed.host_requests, |payload| {
        matches!(
            payload,
            HostRequest::RefreshCapabilityCatalog { run_id, .. } if run_id == "waiting-fork"
        )
    });
    assert_eq!(replayed_refresh.effect_id, refresh.effect_id);
    let mut refreshed_host = checkpoint(&restored).state.host.catalog.clone();
    refreshed_host.revision += 1;
    let refreshed_host_revision = refreshed_host.revision;
    let resumed = restored
        .submit(AgentSubmission::HostResponse {
            response: host_response(
                &replayed_refresh,
                HostResponse::CapabilityCatalogRefreshed {
                    catalog: nola_core::agent::capability::CapabilityCatalogRefresh {
                        catalog: catalog(2, true), host_catalog: refreshed_host,
                    },
                },
            ),
        })
        .expect("catalog response");
    assert_eq!(checkpoint(&restored).state.host.catalog.revision, refreshed_host_revision);
    assert!(resumed.event_envelopes.iter().any(|envelope| matches!(
        &envelope.event,
        AgentEvent::SkillRuntimeUpdated {
            update: SkillRuntimeEvent::Resumed { run_id, catalog_revision: 2 },
            ..
        } if run_id == "waiting-fork"
    )));
    assert_eq!(
        checkpoint(&restored).state.skill_runtime.runs["waiting-fork"].status,
        SkillRunStatus::Running
    );
}

#[test]
fn agent_core_inline_and_fork_terminal_updates_are_exactly_once() {
    let mut core = AgentCore::try_new(core_config()).expect("Core");
    core.submit(core_skill_submission(
        "catalog",
        SkillRuntimeCommand::ReplaceCatalog {
            catalog: catalog(1, true),
        },
    ))
    .expect("catalog");
    for (run_id, mode) in [
        ("inline", SkillExecutionMode::Inline),
        ("fork", SkillExecutionMode::Fork),
    ] {
        core.submit(core_skill_submission(
            &format!("start-{run_id}"),
            SkillRuntimeCommand::Start {
                run_id: run_id.to_string(),
                owner: owner(mode, run_id),
                descriptor: skill(Vec::new()),
                mode,
                budget: SkillRunBudget::default(),
            },
        ))
        .expect("start");
    }
    let complete = core_skill_submission(
        "complete-fork",
        SkillRuntimeCommand::Complete {
            run_id: "fork".to_string(),
            result_ref: Some("artifact://paper.md".to_string()),
        },
    );
    let completed = core.submit(complete.clone()).expect("complete");
    assert_eq!(
        completed
            .event_envelopes
            .iter()
            .filter(|envelope| matches!(
                envelope.event,
                AgentEvent::SkillRuntimeUpdated {
                    update: SkillRuntimeEvent::Completed { .. },
                    ..
                }
            ))
            .count(),
        1
    );
    let duplicate = core.submit(complete).expect("duplicate absorbed");
    assert!(duplicate.event_envelopes.is_empty() && duplicate.host_requests.is_empty());
    let late = core
        .submit(core_skill_submission(
            "late-fork-failure",
            SkillRuntimeCommand::Fail {
                run_id: "fork".to_string(),
                error: "late".to_string(),
            },
        ))
        .expect("late terminal absorbed");
    assert!(!late.event_envelopes.iter().any(|envelope| matches!(
        envelope.event,
        AgentEvent::SkillRuntimeUpdated {
            update: SkillRuntimeEvent::Failed { .. },
            ..
        }
    )));
    let _cancelled = core
        .submit(core_skill_submission(
            "cancel-inline",
            SkillRuntimeCommand::Cancel {
                run_id: "inline".to_string(),
                reason: Some("user".to_string()),
            },
        ))
        .expect("cancel inline");
    let durable = checkpoint(&core);
    assert_eq!(
        durable.state.skill_runtime.runs["fork"].status,
        SkillRunStatus::Completed
    );
    assert_eq!(
        durable.state.skill_runtime.runs["inline"].status,
        SkillRunStatus::Cancelled
    );
}
