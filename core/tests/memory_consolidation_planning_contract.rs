use nola_core::agent::{
    MemoryConsolidationOutput, MemoryJobOutput, MemoryKind, MemoryNamespace, MemoryProvenance,
    MemoryProvenanceKind, MemoryQualityAssessment, MemoryRecord, MemoryRetention,
    MemoryRuntimeCommand, MemoryRuntimeCommandEnvelope, MemoryRuntimeProjection, MemoryScope,
    MemoryUsageLedgerEntry,
};

fn record(index: usize) -> MemoryRecord {
    MemoryRecord {
        id: format!("memory-{index}"),
        owner: nola_domain::memory_owner::MemoryOwner::local_profile(),
        namespace: MemoryNamespace::Personal,
        scope: MemoryScope::Global,
        kind: MemoryKind::Preference,
        content: format!("Durable preference {index}"),
        summary: None,
        tags: Vec::new(),
        confidence: 0.9,
        importance: 0.7,
        sensitivity: None,
        retention: MemoryRetention::UntilSuperseded,
        supersedes: Vec::new(),
        provenance: MemoryProvenance {
            kind: MemoryProvenanceKind::AgentInference,
            source: "nola://test".to_string(),
            session_id: Some("session-source".to_string()),
            turn_id: Some("turn-source".to_string()),
            item_ids: vec!["item-source".to_string()],
            observed_at: None,
            ..MemoryProvenance::default()
        },
    }
}

#[test]
fn consolidation_selection_is_usage_aware_and_single_flight() {
    let mut projection = MemoryRuntimeProjection::default();
    projection
        .apply(MemoryRuntimeCommandEnvelope {
            operation_id: "record-usage".to_string(),
            command: MemoryRuntimeCommand::RecordUsage {
                entry: MemoryUsageLedgerEntry {
                    usage_id: "usage-7".to_string(),
                    memory_id: "memory-7".to_string(),
                    session_id: "session-a".to_string(),
                    turn_id: "turn-a".to_string(),
                    used_at_epoch_ms: 100,
                    context_payload_digest: "sha256:context".to_string(),
                    citation_ids: vec!["citation-7".to_string()],
                },
            },
        })
        .unwrap();

    let records = (0..8).map(record).collect::<Vec<_>>();
    let selected = projection.consolidation_candidates(&records);
    assert_eq!(selected.len(), 8);
    assert_eq!(selected[0].id, "memory-7");

    projection
        .apply(MemoryRuntimeCommandEnvelope {
            operation_id: "create-consolidation".to_string(),
            command: MemoryRuntimeCommand::CreateConsolidation {
                job_id: "consolidation-1".to_string(),
                input_watermark: "watermark-1".to_string(),
                input_digest: "sha256:records".to_string(),
                selected_memory_ids: selected.into_iter().map(|record| record.id).collect(),
                now_epoch_ms: 101,
            },
        })
        .unwrap();
    assert!(projection.consolidation_candidates(&records).is_empty());
}

#[test]
fn consolidation_selection_uses_durable_cross_session_usage() {
    let projection = MemoryRuntimeProjection::default();
    let records = (0..8).map(record).collect::<Vec<_>>();
    let usage = [("memory-6".to_string(), 4_u64)].into_iter().collect();

    let selected = projection.consolidation_candidates_with_usage(&records, &usage);

    assert_eq!(selected[0].id, "memory-6");
}

#[test]
fn consolidation_selection_excludes_superseded_records() {
    let projection = MemoryRuntimeProjection::default();
    let mut records = (0..9).map(record).collect::<Vec<_>>();
    records[8].supersedes = vec!["memory-0".to_string()];
    let selected = projection.consolidation_candidates(&records);
    assert_eq!(selected.len(), 8);
    assert!(!selected.iter().any(|record| record.id == "memory-0"));
}

#[test]
fn consolidation_never_mixes_or_cross_suppresses_namespace_owners() {
    let projection = MemoryRuntimeProjection::default();
    let personal = (0..9).map(record).collect::<Vec<_>>();
    let mut workspace = (0..8)
        .map(|index| {
            let mut item = record(index + 100);
            item.namespace = MemoryNamespace::Workspace;
            item.owner = nola_domain::memory_owner::MemoryOwner::Workspace { profile_id: "local".into(), workspace_id: "workspace-fixture".into() };
            item
        })
        .collect::<Vec<_>>();
    workspace[0].supersedes = vec!["memory-0".to_string()];
    let records = personal
        .iter()
        .chain(workspace.iter())
        .cloned()
        .collect::<Vec<_>>();

    let selected = projection.consolidation_candidates(&records);

    assert_eq!(selected.len(), 9);
    assert!(selected
        .iter()
        .all(|record| record.namespace == MemoryNamespace::Personal));
    assert!(selected.iter().any(|record| record.id == "memory-0"));
}

#[test]
fn consolidation_rejects_supersession_without_matching_output_lineage() {
    let mut projection = MemoryRuntimeProjection::default();
    projection
        .apply(MemoryRuntimeCommandEnvelope {
            operation_id: "create-lineage-job".to_string(),
            command: MemoryRuntimeCommand::CreateConsolidation {
                job_id: "lineage-job".to_string(),
                input_watermark: "lineage-watermark".to_string(),
                input_digest: "sha256:lineage-input".to_string(),
                selected_memory_ids: vec!["memory-0".to_string(), "memory-1".to_string()],
                now_epoch_ms: 100,
            },
        })
        .unwrap();

    let mut consolidated = record(100);
    consolidated.id = "memory-consolidated".to_string();
    consolidated.content = "One durable consolidated preference".to_string();
    consolidated.supersedes = vec!["memory-1".to_string()];
    consolidated.provenance = MemoryProvenance {
        kind: MemoryProvenanceKind::AgentInference,
        source: "nola://memory/consolidation/lineage-job".to_string(),
        session_id: None,
        turn_id: None,
        item_ids: vec!["memory-1".to_string()],
        observed_at: None,
        ..MemoryProvenance::default()
    };
    let quality = MemoryQualityAssessment {
        applicable: true,
        durable: true,
        legible: true,
        rationale: "Preserves the selected source fact".to_string(),
    };
    let mut output = MemoryJobOutput::Consolidation(MemoryConsolidationOutput {
        records: vec![consolidated],
        summary: "Consolidated one preference".to_string(),
        output_digest: "sha256:lineage-output".to_string(),
        citations: Vec::new(),
        // This claims memory-0 was retired even though no accepted output
        // carries that supersession edge. Recall would otherwise keep the old
        // row active while the job reports a successful replacement.
        superseded_memory_ids: vec!["memory-0".to_string()],
        quality_by_memory_id: [("memory-consolidated".to_string(), quality)]
            .into_iter()
            .collect(),
    });

    let error = projection
        .validate_job_output("lineage-job", &output)
        .expect_err("unmaterialized supersession must fail closed");
    assert!(
        error.contains("exactly match output record lineage"),
        "{error}"
    );

    let MemoryJobOutput::Consolidation(corrected) = &mut output else {
        unreachable!()
    };
    corrected.superseded_memory_ids = vec!["memory-1".to_string()];
    projection
        .validate_job_output("lineage-job", &output)
        .expect("matching supersession lineage remains valid");
}

#[test]
fn consolidation_groups_concrete_owners_within_the_same_namespace() {
    let projection = MemoryRuntimeProjection::default();
    let mut records = (0..9).map(record).collect::<Vec<_>>();
    for index in 100..108 {
        let mut foreign = record(index);
        foreign.owner = nola_domain::memory_owner::MemoryOwner::Profile { profile_id: "foreign".into() };
        foreign.supersedes = vec!["memory-0".into()];
        records.push(foreign);
    }
    let selected = projection.consolidation_candidates(&records);
    assert_eq!(selected.len(), 9);
    assert!(selected.iter().all(|record| record.owner == nola_domain::memory_owner::MemoryOwner::local_profile()));
    assert!(selected.iter().any(|record| record.id == "memory-0"));
}
