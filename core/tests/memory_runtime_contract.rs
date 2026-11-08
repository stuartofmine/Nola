// SPDX-License-Identifier: Apache-2.0

use nola_core::agent::{
    ExtractedMemory, MemoryCitation, MemoryJobKind, MemoryJobOutput, MemoryJobState, MemoryKind,
    MemoryNamespace, MemoryProvenance, MemoryProvenanceKind, MemoryQualityAssessment, MemoryRecord,
    MemoryRetention, MemoryRuntimeCommand, MemoryRuntimeCommandEnvelope, MemoryRuntimeEvent,
    MemoryRuntimeProjection, MemoryScope, MemorySourceSnapshot, MemoryUsageLedgerEntry,
};

#[test]
fn claimable_jobs_are_ordered_and_respect_retry_deadlines() {
    let mut runtime = MemoryRuntimeProjection::default();
    runtime
        .apply(MemoryRuntimeCommandEnvelope {
            operation_id: "create-a".to_string(),
            command: MemoryRuntimeCommand::CreateExtraction {
                job_id: "a".to_string(),
                input_watermark: "w-a".to_string(),
                sources: vec![source()],
                now_epoch_ms: 10,
            },
        })
        .unwrap();
    runtime
        .apply(MemoryRuntimeCommandEnvelope {
            operation_id: "create-b".to_string(),
            command: MemoryRuntimeCommand::CreateExtraction {
                job_id: "b".to_string(),
                input_watermark: "w-b".to_string(),
                sources: vec![source()],
                now_epoch_ms: 20,
            },
        })
        .unwrap();
    runtime
        .apply(MemoryRuntimeCommandEnvelope {
            operation_id: "claim-a".to_string(),
            command: MemoryRuntimeCommand::Claim {
                job_id: "a".to_string(),
                worker_id: "worker".to_string(),
                ownership_token: "lease-a".to_string(),
                now_epoch_ms: 30,
                lease_until_epoch_ms: 100,
            },
        })
        .unwrap();

    let ready = runtime.claimable_jobs(40);
    assert_eq!(
        ready.iter().map(|job| job.id.as_str()).collect::<Vec<_>>(),
        vec!["b"]
    );
}

fn apply(
    runtime: &mut MemoryRuntimeProjection,
    operation_id: &str,
    command: MemoryRuntimeCommand,
) -> nola_core::agent::MemoryRuntimeApplyResult {
    runtime
        .apply(MemoryRuntimeCommandEnvelope {
            operation_id: operation_id.to_string(),
            command,
        })
        .expect("memory runtime command")
}

fn source() -> MemorySourceSnapshot {
    MemorySourceSnapshot {
        session_id: "session-1".to_string(),
        turn_id: Some("turn-1".to_string()),
        item_ids: vec!["user-1".to_string(), "assistant-1".to_string()],
        transcript_digest: "sha256:transcript".to_string(),
        source_watermark: "message:2".to_string(),
        captured_at_epoch_ms: 10,
    }
}

fn durable_extracted_memory() -> ExtractedMemory {
    ExtractedMemory {
        record: MemoryRecord {
            id: "memory-1".to_string(),
            owner: nola_domain::memory_owner::MemoryOwner::local_profile(),
            namespace: MemoryNamespace::Personal,
            scope: MemoryScope::Global,
            kind: MemoryKind::Feedback,
            content: "Use real end-to-end evidence for routing changes.".to_string(),
            summary: Some("Prefer real E2E evidence".to_string()),
            tags: vec!["testing".to_string()],
            confidence: 0.95,
            importance: 0.9,
            sensitivity: None,
            retention: MemoryRetention::UntilSuperseded,
            supersedes: Vec::new(),
            provenance: MemoryProvenance {
                kind: MemoryProvenanceKind::AgentInference,
                source: "nola://session/session-1".to_string(),
                session_id: Some("session-1".to_string()),
                turn_id: Some("turn-1".to_string()),
                item_ids: vec!["user-1".to_string()],
                observed_at: None,
                ..MemoryProvenance::default()
            },
        },
        quality: MemoryQualityAssessment {
            applicable: true,
            durable: true,
            legible: true,
            rationale: "The correction governs future routing work.".to_string(),
        },
        citations: vec![MemoryCitation {
            citation_id: "citation-1".to_string(),
            memory_id: "memory-1".to_string(),
            uri: "nola://session/session-1/items/user-1".to_string(),
            title: None,
            note: None,
            line_start: None,
            line_end: None,
            session_id: Some("session-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            item_ids: vec!["user-1".to_string()],
        }],
    }
}

#[test]
fn extraction_claim_retry_and_completion_preserve_frozen_input() {
    let mut runtime = MemoryRuntimeProjection::default();
    apply(
        &mut runtime,
        "create-1",
        MemoryRuntimeCommand::CreateExtraction {
            job_id: "extract-1".to_string(),
            input_watermark: "message:2".to_string(),
            sources: vec![source()],
            now_epoch_ms: 10,
        },
    );
    apply(
        &mut runtime,
        "claim-1",
        MemoryRuntimeCommand::Claim {
            job_id: "extract-1".to_string(),
            worker_id: "host-1".to_string(),
            ownership_token: "lease-1".to_string(),
            now_epoch_ms: 11,
            lease_until_epoch_ms: 100,
        },
    );
    apply(
        &mut runtime,
        "retry-1",
        MemoryRuntimeCommand::ScheduleRetry {
            job_id: "extract-1".to_string(),
            ownership_token: "lease-1".to_string(),
            now_epoch_ms: 20,
            retry_after_ms: Some(5),
            max_retry_count: 3,
            error: "memory worker exceeded its Core-owned lease deadline".to_string(),
        },
    );
    let waiting = runtime.job("extract-1").expect("retrying extraction");
    assert_eq!(waiting.state, MemoryJobState::RetryWaiting);
    assert_eq!(waiting.input_watermark, "message:2");
    assert!(waiting.output.is_none());
    assert!(waiting.ownership_token.is_none());

    apply(
        &mut runtime,
        "claim-2",
        MemoryRuntimeCommand::Claim {
            job_id: "extract-1".to_string(),
            worker_id: "host-2".to_string(),
            ownership_token: "lease-2".to_string(),
            now_epoch_ms: 25,
            lease_until_epoch_ms: 125,
        },
    );
    let wrong_watermark = runtime.apply(MemoryRuntimeCommandEnvelope {
        operation_id: "complete-wrong".to_string(),
        command: MemoryRuntimeCommand::Complete {
            job_id: "extract-1".to_string(),
            ownership_token: "lease-2".to_string(),
            now_epoch_ms: 30,
            completion_watermark: "message:3".to_string(),
            output: MemoryJobOutput::Extraction(nola_core::agent::MemoryExtractionOutput {
                memories: Vec::new(),
                rollout_summary: "No durable signal".to_string(),
                output_digest: "sha256:output".to_string(),
                redacted_secret_count: 0,
            }),
        },
    });
    assert!(wrong_watermark.is_err());

    apply(
        &mut runtime,
        "complete-1",
        MemoryRuntimeCommand::Complete {
            job_id: "extract-1".to_string(),
            ownership_token: "lease-2".to_string(),
            now_epoch_ms: 31,
            completion_watermark: "message:2".to_string(),
            output: MemoryJobOutput::Extraction(nola_core::agent::MemoryExtractionOutput {
                memories: Vec::new(),
                rollout_summary: "No durable signal".to_string(),
                output_digest: "sha256:output".to_string(),
                redacted_secret_count: 0,
            }),
        },
    );
    let completed = runtime.job("extract-1").expect("completed extraction");
    assert_eq!(completed.kind, MemoryJobKind::Extraction);
    assert_eq!(completed.state, MemoryJobState::Completed);
    assert_eq!(completed.completion_watermark.as_deref(), Some("message:2"));
}

#[test]
fn operation_replay_is_idempotent_and_global_consolidation_is_single_flight() {
    let mut runtime = MemoryRuntimeProjection::default();
    let command = MemoryRuntimeCommand::CreateConsolidation {
        job_id: "consolidate-1".to_string(),
        input_watermark: "memory-revision:7".to_string(),
        input_digest: "sha256:memory-set".to_string(),
        selected_memory_ids: vec!["memory-1".to_string()],
        now_epoch_ms: 10,
    };
    let first = apply(&mut runtime, "create-consolidation", command.clone());
    let replay = apply(&mut runtime, "create-consolidation", command);
    assert_eq!(first.revision, 1);
    assert!(replay.duplicate);
    assert_eq!(runtime.jobs.len(), 1);

    let checkpoint = serde_json::to_string(&runtime).expect("encode memory runtime checkpoint");
    let mut restored: MemoryRuntimeProjection =
        serde_json::from_str(&checkpoint).expect("restore memory runtime checkpoint");
    let restored_replay = restored
        .apply(MemoryRuntimeCommandEnvelope {
            operation_id: "create-consolidation".to_string(),
            command: MemoryRuntimeCommand::CreateConsolidation {
                job_id: "consolidate-1".to_string(),
                input_watermark: "memory-revision:7".to_string(),
                input_digest: "sha256:memory-set".to_string(),
                selected_memory_ids: vec!["memory-1".to_string()],
                now_epoch_ms: 10,
            },
        })
        .expect("same command identity remains idempotent after restore");
    assert!(restored_replay.duplicate);
    let conflict = restored
        .apply(MemoryRuntimeCommandEnvelope {
            operation_id: "create-consolidation".to_string(),
            command: MemoryRuntimeCommand::CreateConsolidation {
                job_id: "consolidate-conflict".to_string(),
                input_watermark: "memory-revision:conflict".to_string(),
                input_digest: "sha256:conflict".to_string(),
                selected_memory_ids: vec!["memory-conflict".to_string()],
                now_epoch_ms: 12,
            },
        })
        .expect_err("same operation id with a different command must be rejected");
    assert!(conflict.contains("replayed with a different command"));
    assert!(restored.job("consolidate-conflict").is_none());

    let mut legacy_checkpoint: serde_json::Value =
        serde_json::from_str(&checkpoint).expect("decode memory runtime checkpoint value");
    legacy_checkpoint
        .as_object_mut()
        .expect("memory runtime checkpoint object")
        .remove("processed_operation_digests");
    let mut legacy_restored: MemoryRuntimeProjection = serde_json::from_value(legacy_checkpoint)
        .expect("restore checkpoint without operation identities");
    let unavailable = legacy_restored
        .apply(MemoryRuntimeCommandEnvelope {
            operation_id: "create-consolidation".to_string(),
            command: MemoryRuntimeCommand::CreateConsolidation {
                job_id: "consolidate-1".to_string(),
                input_watermark: "memory-revision:7".to_string(),
                input_digest: "sha256:memory-set".to_string(),
                selected_memory_ids: vec!["memory-1".to_string()],
                now_epoch_ms: 10,
            },
        })
        .expect_err("legacy receipt without an identity must fail closed");
    assert!(unavailable.contains("replay identity is unavailable"));

    let second = runtime.apply(MemoryRuntimeCommandEnvelope {
        operation_id: "create-consolidation-2".to_string(),
        command: MemoryRuntimeCommand::CreateConsolidation {
            job_id: "consolidate-2".to_string(),
            input_watermark: "memory-revision:8".to_string(),
            input_digest: "sha256:memory-set-2".to_string(),
            selected_memory_ids: vec!["memory-2".to_string()],
            now_epoch_ms: 11,
        },
    });
    assert!(second.is_err());
}

#[test]
fn lease_sweep_requeues_work_and_usage_ledger_tracks_actual_context_use() {
    let mut runtime = MemoryRuntimeProjection::default();
    apply(
        &mut runtime,
        "create-1",
        MemoryRuntimeCommand::CreateExtraction {
            job_id: "extract-1".to_string(),
            input_watermark: "message:2".to_string(),
            sources: vec![source()],
            now_epoch_ms: 10,
        },
    );
    apply(
        &mut runtime,
        "claim-1",
        MemoryRuntimeCommand::Claim {
            job_id: "extract-1".to_string(),
            worker_id: "host-1".to_string(),
            ownership_token: "lease-1".to_string(),
            now_epoch_ms: 11,
            lease_until_epoch_ms: 20,
        },
    );
    let sweep = apply(
        &mut runtime,
        "sweep-1",
        MemoryRuntimeCommand::SweepExpiredLeases {
            now_epoch_ms: 20,
            max_retry_count: 2,
        },
    );
    assert!(matches!(
        sweep.events.as_slice(),
        [MemoryRuntimeEvent::JobRetryScheduled { .. }]
    ));

    apply(
        &mut runtime,
        "usage-1",
        MemoryRuntimeCommand::RecordUsage {
            entry: MemoryUsageLedgerEntry {
                usage_id: "usage-1".to_string(),
                memory_id: "memory-1".to_string(),
                session_id: "session-2".to_string(),
                turn_id: "turn-8".to_string(),
                used_at_epoch_ms: 50,
                context_payload_digest: "sha256:context".to_string(),
                citation_ids: vec!["citation-1".to_string()],
            },
        },
    );
    assert_eq!(runtime.usage_stats["memory-1"].usage_count, 1);
    assert_eq!(
        runtime.usage_stats["memory-1"].last_used_at_epoch_ms,
        Some(50)
    );
}

#[test]
fn extraction_rejects_unattested_or_session_only_memory() {
    let mut runtime = MemoryRuntimeProjection::default();
    apply(
        &mut runtime,
        "create-quality",
        MemoryRuntimeCommand::CreateExtraction {
            job_id: "extract-quality".to_string(),
            input_watermark: "message:2".to_string(),
            sources: vec![source()],
            now_epoch_ms: 10,
        },
    );
    apply(
        &mut runtime,
        "claim-quality",
        MemoryRuntimeCommand::Claim {
            job_id: "extract-quality".to_string(),
            worker_id: "host-1".to_string(),
            ownership_token: "lease-quality".to_string(),
            now_epoch_ms: 11,
            lease_until_epoch_ms: 100,
        },
    );

    let mut weak = durable_extracted_memory();
    weak.quality.durable = false;
    let weak_result = runtime.apply(MemoryRuntimeCommandEnvelope {
        operation_id: "complete-weak".to_string(),
        command: MemoryRuntimeCommand::Complete {
            job_id: "extract-quality".to_string(),
            ownership_token: "lease-quality".to_string(),
            now_epoch_ms: 20,
            completion_watermark: "message:2".to_string(),
            output: MemoryJobOutput::Extraction(nola_core::agent::MemoryExtractionOutput {
                memories: vec![weak],
                rollout_summary: "One candidate".to_string(),
                output_digest: "sha256:weak".to_string(),
                redacted_secret_count: 0,
            }),
        },
    });
    assert!(weak_result.is_err());
    assert_eq!(
        runtime.job("extract-quality").map(|job| job.state),
        Some(MemoryJobState::Running)
    );

    let mut outside_snapshot = durable_extracted_memory();
    outside_snapshot.record.provenance.item_ids = vec!["unknown-item".to_string()];
    let outside_result = runtime.apply(MemoryRuntimeCommandEnvelope {
        operation_id: "complete-outside".to_string(),
        command: MemoryRuntimeCommand::Complete {
            job_id: "extract-quality".to_string(),
            ownership_token: "lease-quality".to_string(),
            now_epoch_ms: 21,
            completion_watermark: "message:2".to_string(),
            output: MemoryJobOutput::Extraction(nola_core::agent::MemoryExtractionOutput {
                memories: vec![outside_snapshot],
                rollout_summary: "One candidate".to_string(),
                output_digest: "sha256:outside".to_string(),
                redacted_secret_count: 0,
            }),
        },
    });
    assert!(outside_result.is_err());

    apply(
        &mut runtime,
        "complete-quality",
        MemoryRuntimeCommand::Complete {
            job_id: "extract-quality".to_string(),
            ownership_token: "lease-quality".to_string(),
            now_epoch_ms: 22,
            completion_watermark: "message:2".to_string(),
            output: MemoryJobOutput::Extraction(nola_core::agent::MemoryExtractionOutput {
                memories: vec![durable_extracted_memory()],
                rollout_summary: "Saved one durable correction".to_string(),
                output_digest: "sha256:quality".to_string(),
                redacted_secret_count: 0,
            }),
        },
    );
    assert_eq!(
        runtime.job("extract-quality").map(|job| job.state),
        Some(MemoryJobState::Completed)
    );
}

#[test]
fn extraction_rejects_duplicate_semantic_identity_before_checkpoint_commit() {
    let mut runtime = MemoryRuntimeProjection::default();
    apply(
        &mut runtime,
        "create-duplicate-content",
        MemoryRuntimeCommand::CreateExtraction {
            job_id: "extract-duplicate-content".to_string(),
            input_watermark: "message:2".to_string(),
            sources: vec![source()],
            now_epoch_ms: 10,
        },
    );
    apply(
        &mut runtime,
        "claim-duplicate-content",
        MemoryRuntimeCommand::Claim {
            job_id: "extract-duplicate-content".to_string(),
            worker_id: "host-1".to_string(),
            ownership_token: "lease-duplicate-content".to_string(),
            now_epoch_ms: 11,
            lease_until_epoch_ms: 100,
        },
    );

    let first = durable_extracted_memory();
    let mut duplicate = first.clone();
    duplicate.record.id = "memory-duplicate-id".to_string();
    duplicate.citations[0].citation_id = "citation-duplicate-id".to_string();
    duplicate.citations[0].memory_id = duplicate.record.id.clone();

    let rejected = runtime.apply(MemoryRuntimeCommandEnvelope {
        operation_id: "complete-duplicate-content".to_string(),
        command: MemoryRuntimeCommand::Complete {
            job_id: "extract-duplicate-content".to_string(),
            ownership_token: "lease-duplicate-content".to_string(),
            now_epoch_ms: 20,
            completion_watermark: "message:2".to_string(),
            output: MemoryJobOutput::Extraction(nola_core::agent::MemoryExtractionOutput {
                memories: vec![first, duplicate],
                rollout_summary: "Duplicated fact".to_string(),
                output_digest: "sha256:duplicate-content".to_string(),
                redacted_secret_count: 0,
            }),
        },
    });

    assert!(rejected
        .unwrap_err()
        .contains("duplicate semantic identities"));
    assert_eq!(
        runtime
            .job("extract-duplicate-content")
            .map(|job| job.state),
        Some(MemoryJobState::Running)
    );
}
