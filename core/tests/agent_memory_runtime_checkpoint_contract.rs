#[path = "support/model_identity.rs"]
mod model_identity;
use nola_core::agent::recovery::CoreCheckpoint;
use nola_core::agent::{
    AgentCore, AgentCoreConfig, AgentSubmission, AgentThreadState, MemoryJobState,
    MemoryRuntimeCommand, MemoryRuntimeCommandEnvelope, MemorySourceSnapshot,
    MemoryUsageLedgerEntry, SessionId,
};

fn core_config() -> AgentCoreConfig {
    AgentCoreConfig {
        turn_identity: model_identity::host_identity(),
        session_id: Some(SessionId::from_string("session-1".to_string())),
        ..AgentCoreConfig::default()
    }
}

fn extraction_submission(operation_id: &str) -> AgentSubmission {
    AgentSubmission::MemoryRuntime {
        envelope: MemoryRuntimeCommandEnvelope {
            operation_id: operation_id.to_string(),
            command: MemoryRuntimeCommand::CreateExtraction {
                job_id: "memory-extract-session-1".to_string(),
                input_watermark: "turn-9".to_string(),
                sources: vec![MemorySourceSnapshot {
                    session_id: "session-1".to_string(),
                    turn_id: Some("turn-9".to_string()),
                    item_ids: vec!["user-9".to_string(), "assistant-9".to_string()],
                    transcript_digest: "sha256:transcript".to_string(),
                    source_watermark: "turn-9".to_string(),
                    captured_at_epoch_ms: 100,
                }],
                now_epoch_ms: 100,
            },
        },
    }
}

#[test]
fn memory_runtime_is_part_of_the_authoritative_session_checkpoint() {
    let config = core_config();
    let mut core = AgentCore::try_new(config.clone()).expect("create Core");

    let transition = core
        .submit(extraction_submission("memory-op-1"))
        .expect("create extraction job");
    let job = transition
        .snapshot
        .memory_runtime
        .job("memory-extract-session-1")
        .expect("job in snapshot");
    assert_eq!(job.state, MemoryJobState::Queued);

    let checkpoint: CoreCheckpoint<AgentThreadState> = core.checkpoint(
        transition
            .durability
            .as_ref()
            .expect("memory runtime mutation must request durability")
            .checkpoint_reason,
    );
    let restored = AgentCore::restore(config, checkpoint).expect("restore checkpoint");
    assert_eq!(
        restored
            .snapshot()
            .memory_runtime
            .job("memory-extract-session-1")
            .map(|job| job.input_watermark.as_str()),
        Some("turn-9")
    );
}

#[test]
fn replayed_memory_operation_is_idempotent_inside_the_session() {
    let mut core = AgentCore::try_new(core_config()).expect("create Core");

    core.submit(extraction_submission("memory-op-1"))
        .expect("first operation");
    let replay = core
        .submit(extraction_submission("memory-op-1"))
        .expect("replayed operation");

    assert_eq!(replay.snapshot.memory_runtime.job_order.len(), 1);
    assert_eq!(replay.snapshot.memory_runtime.revision, 1);
}

#[test]
fn usage_feedback_cannot_mutate_a_different_session() {
    let mut core = AgentCore::try_new(core_config()).expect("create Core");
    let submission = AgentSubmission::MemoryRuntime {
        envelope: MemoryRuntimeCommandEnvelope {
            operation_id: "usage-op-cross-session".to_string(),
            command: MemoryRuntimeCommand::RecordUsage {
                entry: MemoryUsageLedgerEntry {
                    usage_id: "usage-cross-session".to_string(),
                    memory_id: "memory-owned-by-session-2".to_string(),
                    session_id: "session-2".to_string(),
                    turn_id: "turn-1".to_string(),
                    used_at_epoch_ms: 1_785_600_000_123,
                    context_payload_digest: "sha256:context-session-2".to_string(),
                    citation_ids: Vec::new(),
                },
            },
        },
    };

    let error = core
        .submit(submission)
        .expect_err("cross-session usage feedback must fail closed");
    assert!(error.contains("does not match active session"));
    assert!(core.snapshot().memory_runtime.usage_ledger.is_empty());
    assert!(core.snapshot().memory_runtime.usage_stats.is_empty());
    assert_eq!(core.snapshot().memory_runtime.revision, 0);

    let corrected = core
        .submit(AgentSubmission::MemoryRuntime {
            envelope: MemoryRuntimeCommandEnvelope {
                operation_id: "usage-op-cross-session".to_string(),
                command: MemoryRuntimeCommand::RecordUsage {
                    entry: MemoryUsageLedgerEntry {
                        usage_id: "usage-session-1".to_string(),
                        memory_id: "memory-owned-by-session-1".to_string(),
                        session_id: "session-1".to_string(),
                        turn_id: "turn-1".to_string(),
                        used_at_epoch_ms: 1_785_600_000_123,
                        context_payload_digest: "sha256:context-session-1".to_string(),
                        citation_ids: Vec::new(),
                    },
                },
            },
        })
        .expect("same-session usage feedback remains admissible");
    assert_eq!(corrected.snapshot.memory_runtime.usage_ledger.len(), 1);
    assert_eq!(
        corrected.snapshot.memory_runtime.usage_stats["memory-owned-by-session-1"].usage_count,
        1
    );
}
