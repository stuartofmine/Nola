// SPDX-License-Identifier: Apache-2.0
//! Consolidated memory ownership, consolidation identity & planning, bounded selection & C ABI,
//! typed CRUD/checkpoint, and runtime job lifecycle contracts.

pub mod ownership {
use nola_core::memory::{MemoryNamespace, MemoryScope};
use nola_domain::memory_owner::{MemoryExecutionBinding, MemoryOwner};

fn binding() -> MemoryExecutionBinding {
    MemoryExecutionBinding {
        profile_id: "profile-a".into(),
        workspace_id: Some("workspace-a".into()),
        conversation_id: Some("conversation-a".into()),
        turn_id: Some("turn-a".into()),
    }
}

#[test]
fn memory_ownership_visibility_validation_and_wire_boundaries_hold() {
    let original = binding();
    let owners = [
        original
            .owner_for(&MemoryNamespace::Personal, MemoryScope::Global)
            .unwrap(),
        original
            .owner_for(&MemoryNamespace::Workspace, MemoryScope::Global)
            .unwrap(),
        original
            .owner_for(&MemoryNamespace::Conversation, MemoryScope::Conversation)
            .unwrap(),
        original
            .owner_for(&MemoryNamespace::Conversation, MemoryScope::Turn)
            .unwrap(),
    ];
    let mut other_profile = original.clone();
    other_profile.profile_id = "profile-b".into();
    let mut other_workspace = original.clone();
    other_workspace.workspace_id = Some("workspace-b".into());
    let mut other_conversation = original.clone();
    other_conversation.conversation_id = Some("conversation-b".into());
    let mut other_turn = original.clone();
    other_turn.turn_id = Some("turn-b".into());
    for (context, expected) in [
        (original, [true, true, true, true]),
        (other_profile, [false, false, false, false]),
        (other_workspace, [true, false, true, true]),
        (other_conversation, [true, true, false, false]),
        (other_turn, [true, true, true, false]),
    ] {
        assert_eq!(
            owners.each_ref().map(|owner| owner.is_visible_to(&context)),
            expected
        );
    }


    let empty = MemoryExecutionBinding {
        profile_id: "profile-a".into(),
        workspace_id: None,
        conversation_id: None,
        turn_id: None,
    };
    for (namespace, scope) in [
        (MemoryNamespace::Workspace, MemoryScope::Global),
        (MemoryNamespace::Conversation, MemoryScope::Conversation),
        (MemoryNamespace::Conversation, MemoryScope::Turn),
        (MemoryNamespace::Conversation, MemoryScope::Global),
    ] {
        assert!(empty.owner_for(&namespace, scope).is_err());
    }
    let orphan_turn = MemoryExecutionBinding {
        turn_id: Some("turn".into()),
        ..empty
    };
    assert!(orphan_turn.validate().is_err());


    let owner = binding()
        .owner_for(&MemoryNamespace::Conversation, MemoryScope::Turn)
        .unwrap();
    let encoded = serde_json::to_value(&owner).unwrap();
    assert_eq!(
        serde_json::from_value::<MemoryOwner>(encoded.clone()).unwrap(),
        owner
    );
    let mut extra = encoded;
    extra["source"] = "conversation-b".into();
    assert!(serde_json::from_value::<MemoryOwner>(extra).is_err());
    assert!(serde_json::from_str::<MemoryOwner>(
        r#"{"kind":"turn","profile_id":"a","turn_id":"b"}"#
    )
    .is_err());
    assert!(!MemoryOwner::Profile {
        profile_id: " ".into()
    }
    .is_visible_to(&binding()));
}

}

pub mod consolidation_identity {
use nola_core::agent::memory_runtime::consolidation_input_digest;
use nola_core::memory::MemoryRecord;

fn record(id: &str) -> MemoryRecord {
    serde_json::from_value(serde_json::json!({
        "id": id, "owner": {"kind":"profile","profile_id":"local"}, "namespace": {"kind": "personal"}, "scope": "global",
        "kind": "preference", "content": "Prefer concise answers",
        "summary": "Response style", "tags": [], "confidence": 0.9,
        "importance": 0.8, "sensitivity": null,
        "retention": {"mode": "until_superseded"}, "supersedes": [],
        "provenance": {"kind": "explicit_user", "source": "identity-test",
            "session_id": null, "turn_id": null, "item_ids": [], "observed_at": null}
    }))
    .unwrap()
}

#[test]
fn consolidation_digest_tracks_semantic_edits_owners_and_rejects_invalid_inputs() {
    let a = record("a");
    let b = record("b");
    let initial = consolidation_input_digest(&[a.clone(), b.clone()]).unwrap();
    assert_eq!(
        initial,
        consolidation_input_digest(&[b.clone(), a.clone()]).unwrap()
    );
    let mut changed = a.clone();
    changed.content = "Prefer detailed explanations".into();
    assert_ne!(
        initial,
        consolidation_input_digest(&[changed, b.clone()]).unwrap()
    );
    let mut changed = a.clone();
    changed.summary = Some("Corrected summary".into());
    assert_ne!(
        initial,
        consolidation_input_digest(&[changed, b.clone()]).unwrap()
    );
    let mut changed = a;
    changed.provenance.source = "another-source".into();
    assert_ne!(initial, consolidation_input_digest(&[changed, b]).unwrap());

    assert!(consolidation_input_digest(&[]).is_err());
    assert!(consolidation_input_digest(&[record("same"), record("same")]).is_err());
}

#[test]
fn ten_million_character_selection_only_copies_the_selected_records() {
    use nola_core::agent::memory_runtime::MemoryRuntimeProjection;
    let records = (0..1000)
        .map(|index| {
            let mut item = record(&format!("memory-{index:04}"));
            item.content = "x".repeat(10_000);
            item
        })
        .collect::<Vec<_>>();
    let runtime = MemoryRuntimeProjection::default();
    let (candidates, allocated) = crate::measure_allocated_bytes(|| runtime.consolidation_candidates(&records));
    assert_eq!(candidates.len(), 40);
    assert_eq!(candidates[0].id, "memory-0000");
    assert_eq!(candidates[39].id, "memory-0039");
    assert_eq!(
        candidates
            .iter()
            .map(|record| record.content.len())
            .sum::<usize>(),
        400_000
    );
    eprintln!("10M memory selection allocated {allocated} bytes for 40 records");
    assert!(
        allocated < 1_000_000,
        "selection copied unselected content: {allocated}"
    );
}

#[test]
fn owner_changes_invalidate_the_frozen_semantic_input_digest() {
    let original = record("owner-bound");
    let digest = consolidation_input_digest(&[original.clone()]).unwrap();
    let mut changed = original;
    changed.owner = nola_domain::memory_owner::MemoryOwner::Profile { profile_id: "foreign".into() };
    assert_ne!(digest, consolidation_input_digest(&[changed]).unwrap());
}

}

pub mod consolidation_planning {
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

}

pub mod selection {
use nola_core::agent::{
    select_memories, MemoryKind, MemoryNamespace, MemoryPolicy, MemoryProvenance,
    MemoryProvenanceKind, MemoryRecallMode, MemoryRecord, MemoryRetention, MemoryScope,
    MemorySelectionInput, MemorySelectionPlan, MemorySelectionReason, MemorySelectionRejectReason,
    MemoryTokenAccounting,
};
use nola_core::{
    nola_core_free_c_string, nola_core_memory_select_json, nola_core_memory_selection_abi_version,
};
use std::collections::BTreeMap;
use std::ffi::CStr;

fn record(id: &str, confidence: f32) -> MemoryRecord {
    MemoryRecord {
        id: id.to_string(),
        owner: nola_domain::memory_owner::MemoryOwner::local_profile(),
        namespace: MemoryNamespace::Personal,
        scope: MemoryScope::Global,
        kind: MemoryKind::Preference,
        content: format!("memory {id}"),
        summary: None,
        tags: Vec::new(),
        confidence,
        importance: 0.8,
        sensitivity: None,
        retention: MemoryRetention::UntilSuperseded,
        supersedes: Vec::new(),
        provenance: MemoryProvenance {
            kind: MemoryProvenanceKind::ExplicitUser,
            source: format!("source-{id}"),
            session_id: None,
            turn_id: None,
            item_ids: Vec::new(),
            observed_at: Some("2026-07-20T00:00:00Z".to_string()),
            ..MemoryProvenance::default()
        },
    }
}

fn input(records: Vec<MemoryRecord>) -> MemorySelectionInput {
    MemorySelectionInput {
        policy: MemoryPolicy {
            search_limit: 2,
            minimum_confidence: 0.7,
            maximum_age_days: Some(30),
            ..MemoryPolicy::default()
        },
        records,
        mode: MemoryRecallMode::OnDemand,
        allowed_namespaces: vec![MemoryNamespace::Personal],
        include_session_retention: false,
        now: Some("2026-08-02T00:00:00Z".to_string()),
        maximum_input_tokens: None,
        token_accounting: MemoryTokenAccounting::HostExact,
        token_counts: BTreeMap::new(),
    }
}

#[test]
fn core_estimate_accounts_for_budget_without_host_tokenizer() {
    let mut selection_input = input(vec![record("estimated", 0.9)]);
    selection_input.mode = MemoryRecallMode::ResidentProfile;
    selection_input.maximum_input_tokens = Some(1_024);
    selection_input.token_accounting = MemoryTokenAccounting::CoreEstimate;

    let plan = select_memories(selection_input);
    assert_eq!(plan.selected.len(), 1);
    assert!(plan.selected[0].token_count.unwrap_or_default() > 0);
    assert!(plan.selected[0].token_count_is_estimated);
    assert!(plan.token_counts_are_estimated);
    assert!(plan.used_input_tokens > 0);
    assert_eq!(
        plan.remaining_input_tokens,
        Some(1_024 - plan.used_input_tokens)
    );
}

#[test]
fn selection_applies_policy_supersession_and_rank_limit() {
    let old = record("old", 0.95);
    let low = record("low", 0.4);
    let mut current = record("current", 0.95);
    current.supersedes = vec!["old".to_string()];
    let overflow = record("overflow", 0.9);
    let overflow_two = record("overflow-two", 0.9);

    let plan = select_memories(input(vec![old, low, current, overflow, overflow_two]));
    assert_eq!(
        plan.selected
            .iter()
            .map(|entry| entry.record.id.as_str())
            .collect::<Vec<_>>(),
        vec!["current", "overflow"]
    );
    assert!(plan.rejected.iter().any(|entry| {
        entry.memory_id == "old" && entry.reason == MemorySelectionRejectReason::Superseded
    }));
    assert!(plan.rejected.iter().any(|entry| {
        entry.memory_id == "low" && entry.reason == MemorySelectionRejectReason::BelowConfidence
    }));
    assert!(plan.rejected.iter().any(|entry| {
        entry.memory_id == "overflow-two"
            && entry.reason == MemorySelectionRejectReason::LimitExceeded
    }));
}

#[test]
fn supersession_cannot_cross_namespace_or_scope_ownership() {
    let personal = record("personal-fact", 0.95);

    let mut workspace_claim = record("workspace-claim", 0.95);
    workspace_claim.namespace = MemoryNamespace::Workspace;
    workspace_claim.owner = nola_domain::memory_owner::MemoryOwner::Workspace { profile_id: "local".into(), workspace_id: "workspace-fixture".into() };
    workspace_claim.supersedes = vec![personal.id.clone()];

    let mut conversation_claim = record("conversation-claim", 0.95);
    conversation_claim.scope = MemoryScope::Conversation;
    conversation_claim.owner = nola_domain::memory_owner::MemoryOwner::Conversation { profile_id: "local".into(), conversation_id: "conversation-fixture".into() };
    conversation_claim.supersedes = vec![personal.id.clone()];

    let mut selection_input = input(vec![
        personal.clone(),
        workspace_claim.clone(),
        conversation_claim.clone(),
    ]);
    selection_input.policy.search_limit = 3;
    selection_input.allowed_namespaces =
        vec![MemoryNamespace::Personal, MemoryNamespace::Workspace];

    let plan = select_memories(selection_input);
    assert_eq!(
        plan.selected
            .iter()
            .map(|selected| selected.record.id.as_str())
            .collect::<Vec<_>>(),
        vec![
            personal.id.as_str(),
            workspace_claim.id.as_str(),
            conversation_claim.id.as_str()
        ]
    );
    assert!(!plan.rejected.iter().any(|rejected| {
        rejected.memory_id == personal.id
            && rejected.reason == MemorySelectionRejectReason::Superseded
    }));
}

#[test]
fn supersession_still_suppresses_a_fact_with_the_same_owner() {
    let old = record("old-same-owner", 0.95);
    let mut correction = record("correction-same-owner", 0.95);
    correction.supersedes = vec![old.id.clone()];

    let plan = select_memories(input(vec![old.clone(), correction.clone()]));
    assert_eq!(plan.selected.len(), 1);
    assert_eq!(plan.selected[0].record.id, correction.id);
    assert!(plan.rejected.iter().any(|rejected| {
        rejected.memory_id == old.id && rejected.reason == MemorySelectionRejectReason::Superseded
    }));
}

#[test]
fn omitted_budget_uses_a_bounded_core_estimate() {
    let mut oversized = record("oversized", 0.95);
    oversized.content = "x".repeat(40_000);
    let plan = select_memories(input(vec![oversized]));

    assert!(plan.selected.is_empty());
    assert_eq!(plan.remaining_input_tokens, Some(2_048));
    assert!(plan.token_counts_are_estimated);
    assert!(plan.rejected.iter().any(|entry| {
        entry.memory_id == "oversized"
            && entry.reason == MemorySelectionRejectReason::TokenBudgetExceeded
    }));
}

#[test]
fn selection_rejects_stale_expired_session_and_wrong_namespace_records() {
    let mut stale = record("stale", 0.9);
    stale.provenance.observed_at = Some("2026-01-01T00:00:00Z".to_string());
    let mut expired = record("expired", 0.9);
    expired.retention = MemoryRetention::ExpiresAt {
        expires_at: "2026-08-01T00:00:00Z".to_string(),
    };
    let mut session = record("session", 0.9);
    session.retention = MemoryRetention::Session;
    let mut workspace = record("workspace", 0.9);
    workspace.namespace = MemoryNamespace::Workspace;
    workspace.owner = nola_domain::memory_owner::MemoryOwner::Workspace { profile_id: "local".into(), workspace_id: "workspace-fixture".into() };

    let plan = select_memories(input(vec![stale, expired, session, workspace]));
    assert!(plan.selected.is_empty());
    for (id, reason) in [
        ("stale", MemorySelectionRejectReason::TooOld),
        ("expired", MemorySelectionRejectReason::Expired),
        ("session", MemorySelectionRejectReason::SessionOnly),
        (
            "workspace",
            MemorySelectionRejectReason::NamespaceNotAllowed,
        ),
    ] {
        assert!(plan
            .rejected
            .iter()
            .any(|entry| entry.memory_id == id && entry.reason == reason));
    }
}

#[test]
fn age_policy_rejects_missing_or_future_observation_provenance() {
    let mut missing = record("missing-observation", 0.9);
    missing.provenance.observed_at = None;
    let mut future = record("future-observation", 0.9);
    future.provenance.observed_at = Some("2099-01-01T00:00:00Z".to_string());

    let denied = select_memories(input(vec![missing.clone(), future]));
    assert!(denied.selected.is_empty());
    for id in ["missing-observation", "future-observation"] {
        assert!(denied.rejected.iter().any(|entry| {
            entry.memory_id == id && entry.reason == MemorySelectionRejectReason::InvalidTimestamp
        }));
    }

    let mut no_age_policy = input(vec![missing]);
    no_age_policy.policy.maximum_age_days = None;
    assert_eq!(select_memories(no_age_policy).selected.len(), 1);
}

#[test]
fn sensitivity_requires_an_exact_typed_policy_allowlist() {
    let mut sensitive = record("sensitive", 0.95);
    sensitive.sensitivity = Some("health.private".to_string());

    let denied = select_memories(input(vec![sensitive.clone()]));
    assert!(denied.selected.is_empty());
    assert!(denied.rejected.iter().any(|entry| {
        entry.memory_id == sensitive.id
            && entry.reason == MemorySelectionRejectReason::SensitivityNotAllowed
    }));

    let mut allowed_input = input(vec![sensitive.clone()]);
    allowed_input.policy.allowed_sensitivities = vec!["health.private".to_string()];
    let allowed = select_memories(allowed_input);
    assert_eq!(allowed.selected.len(), 1);
    assert_eq!(allowed.selected[0].record.id, sensitive.id);

    let mut different_label = input(vec![sensitive]);
    different_label.policy.allowed_sensitivities = vec!["health.summary".to_string()];
    assert!(select_memories(different_label).selected.is_empty());
}

#[test]
fn mutable_snapshots_require_verification_but_explicit_preferences_do_not() {
    let preference = record("preference", 0.9);
    let mut project = record("project", 0.9);
    project.kind = MemoryKind::Project;
    project.provenance.kind = MemoryProvenanceKind::ToolResult;

    let plan = select_memories(input(vec![preference, project]));
    assert!(!plan.selected[0].requires_current_state_verification);
    assert!(plan.selected[1].requires_current_state_verification);
}

#[test]
fn resident_profile_is_boundary_safe_and_token_budgeted() {
    let short = record("short", 0.9);
    let long = record("long", 0.9);
    let mut project = record("project", 0.9);
    project.kind = MemoryKind::Project;
    let mut workspace = record("workspace", 0.9);
    workspace.namespace = MemoryNamespace::Workspace;
    workspace.owner = nola_domain::memory_owner::MemoryOwner::Workspace { profile_id: "local".into(), workspace_id: "workspace-fixture".into() };

    let mut selection_input = input(vec![short, project, long, workspace]);
    selection_input.mode = MemoryRecallMode::ResidentProfile;
    selection_input.maximum_input_tokens = Some(10);
    selection_input.token_counts = BTreeMap::from([
        ("short".to_string(), 4),
        ("project".to_string(), 2),
        ("long".to_string(), 8),
        ("workspace".to_string(), 2),
    ]);

    let plan = select_memories(selection_input);
    assert_eq!(plan.mode, MemoryRecallMode::ResidentProfile);
    assert_eq!(plan.selected.len(), 1);
    assert_eq!(plan.selected[0].record.id, "short");
    assert_eq!(plan.selected[0].token_count, Some(4));
    assert_eq!(
        plan.selected[0].selection_reason,
        MemorySelectionReason::ResidentProfile
    );
    assert_eq!(plan.used_input_tokens, 4);
    assert_eq!(plan.remaining_input_tokens, Some(6));
    assert!(plan.plan_digest.starts_with("sha256:"));
    assert!(plan.rejected.iter().any(|entry| {
        entry.memory_id == "long"
            && entry.reason == MemorySelectionRejectReason::TokenBudgetExceeded
    }));
    for id in ["project", "workspace"] {
        assert!(plan.rejected.iter().any(|entry| {
            entry.memory_id == id && entry.reason == MemorySelectionRejectReason::NotResidentProfile
        }));
    }
}

#[test]
fn explicit_reference_can_be_part_of_the_resident_profile() {
    let mut reference = record("reference", 0.9);
    reference.kind = MemoryKind::Reference;

    let mut selection_input = input(vec![reference]);
    selection_input.mode = MemoryRecallMode::ResidentProfile;
    selection_input.maximum_input_tokens = Some(1_024);
    selection_input.token_accounting = MemoryTokenAccounting::CoreEstimate;

    let plan = select_memories(selection_input);
    assert_eq!(plan.selected.len(), 1);
    assert_eq!(plan.selected[0].record.id, "reference");
    assert!(plan.selected[0].requires_current_state_verification);
}

#[test]
fn json_c_abi_matches_native_memory_selection_plan() {
    let selection_input = input(vec![record("eligible", 0.9), record("low", 0.2)]);
    let expected = select_memories(selection_input.clone());
    let json = serde_json::to_vec(&selection_input).expect("serialize selection input");

    assert_eq!(nola_core_memory_selection_abi_version(), 1);
    let output = nola_core_memory_select_json(json.as_ptr(), json.len());
    assert!(!output.is_null());
    let output_json = unsafe { CStr::from_ptr(output) }.to_bytes();
    let actual: MemorySelectionPlan =
        serde_json::from_slice(output_json).expect("decode selection plan");
    nola_core_free_c_string(output);

    assert_eq!(actual, expected);
}

#[test]
fn a_foreign_owner_cannot_suppress_a_fact_by_its_id_in_a_combined_selection() {
    let original = record("local", 0.9);
    let mut foreign = record("foreign", 0.9);
    foreign.owner = nola_domain::memory_owner::MemoryOwner::Profile { profile_id: "foreign".into() };
    foreign.supersedes = vec!["local".into()];
    let plan = select_memories(input(vec![original, foreign]));
    assert!(plan.selected.iter().any(|selected| selected.record.id == "local"));
}

}

pub mod runtime_checkpoint {
use crate::support::model_identity;
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
fn memory_runtime_checkpoint_idempotent_replay_and_session_usage_fencing_hold() {
    {

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
    {

    let mut core = AgentCore::try_new(core_config()).expect("create Core");

    core.submit(extraction_submission("memory-op-1"))
        .expect("first operation");
    let replay = core
        .submit(extraction_submission("memory-op-1"))
        .expect("replayed operation");

    assert_eq!(replay.snapshot.memory_runtime.job_order.len(), 1);
    assert_eq!(replay.snapshot.memory_runtime.revision, 1);
}
    {

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
}

}

pub mod record_and_turn_context {
use crate::support::model_identity;
// SPDX-License-Identifier: Apache-2.0

use std::collections::BTreeMap;

use nola_core::agent::host::{
    CapabilityAvailability, HostCancellationMode, HostCapabilityCatalog, HostCapabilityDescriptor,
    HostCapabilityExposure, HostCapabilityParallelism, HostCapabilityRoute, HostId, HostLease,
    HostOutputPolicy, HostReplaySafety, HostRequest,
};
use nola_core::agent::recovery::{CheckpointReason, CoreCheckpoint};
use nola_core::agent::{
    AgentCore, AgentCoreConfig, AgentEvent, AgentInputDelivery, AgentItem, AgentSubmission,
    AgentThreadState, ContextPlatform, ItemId, MemoryKind, MemoryNamespace, MemoryProvenance,
    MemoryProvenanceKind, MemoryRecord, MemoryRetention, MemoryScope, MemoryUpdate, SessionId,
};
use nola_core::context_engine::{ContextComponent, ContextComponentKind};
use serde_json::json;

fn capability(id: &str, route: HostCapabilityRoute) -> HostCapabilityDescriptor {
    HostCapabilityDescriptor {
        id: id.to_string(),
        title: id.to_string(),
        route,
        availability: CapabilityAvailability::Available,
        exposure: HostCapabilityExposure::Visible,
        permission_domain: None,
        input_schema: json!({"type": "object"}),
        input_normalization: Default::default(),
        output_policy: HostOutputPolicy {
            preview_char_limit: 4_000,
            persist_full_output: true,
            allow_artifacts: true,
        },
        supports_streaming: id == "model.stream",
        supports_argument_streaming: false,
        parallelism: HostCapabilityParallelism::Serial,
        cancellation: HostCancellationMode::Cooperative,
        replay_safety: HostReplaySafety::Recoverable,
    }
}

fn config() -> AgentCoreConfig {
    let mut capabilities = BTreeMap::new();
    for (id, route) in [("model.stream", HostCapabilityRoute::Model)] {
        capabilities.insert(id.to_string(), capability(id, route));
    }
    AgentCoreConfig {
        turn_identity: model_identity::host_identity(),
        session_id: Some(SessionId::from_string(
            "memory-contract-session".to_string(),
        )),
        model: Some("test-model".to_string()),
        host_catalog: Some(HostCapabilityCatalog {
            host_id: HostId("memory-host".to_string()),
            revision: 1,
            availability_epoch: 1,
            lease: Some(HostLease {
                lease_id: "memory-lease".to_string(),
                issued_at_ms: 1,
                last_heartbeat_ms: 1,
                expires_at_ms: u64::MAX,
            }),
            capabilities,
        }),
        ..AgentCoreConfig::default()
    }
}

fn record() -> MemoryRecord {
    MemoryRecord {
        id: "memory-preference-1".to_string(),
        owner: nola_domain::memory_owner::MemoryOwner::Workspace { profile_id: "local".into(), workspace_id: "workspace-fixture".into() },
        namespace: MemoryNamespace::Workspace,
        scope: MemoryScope::Global,
        kind: MemoryKind::Preference,
        content: "Prefer typed Core contracts".to_string(),
        summary: Some("Typed contracts".to_string()),
        tags: vec!["architecture".to_string()],
        confidence: 1.0,
        importance: 0.8,
        sensitivity: None,
        retention: MemoryRetention::UntilSuperseded,
        supersedes: Vec::new(),
        provenance: MemoryProvenance {
            kind: MemoryProvenanceKind::ExplicitUser,
            source: "message://user-1".to_string(),
            session_id: Some("memory-contract-session".to_string()),
            turn_id: Some("turn-1".to_string()),
            item_ids: vec!["user-1".to_string()],
            observed_at: Some("2026-08-02T00:00:00Z".to_string()),
            ..MemoryProvenance::default()
        },
    }
}

#[test]
fn typed_memory_crud_emits_projection_events_without_host_permission() {
    let mut core = AgentCore::try_new(config()).expect("create Core");
    let created = core
        .submit(AgentSubmission::CreateMemory { record: record() })
        .expect("create memory");
    assert_eq!(created.snapshot.memories.revision, 1);
    assert_eq!(created.snapshot.memories.order, ["memory-preference-1"]);
    assert!(created.event_envelopes.iter().any(|envelope| matches!(
        &envelope.event,
        AgentEvent::MemoryUpdated {
            memory_projection_revision: 1,
            update: MemoryUpdate::Created { record },
        } if record.id == "memory-preference-1"
    )));
    assert!(!created
        .host_requests
        .iter()
        .any(|request| matches!(request.payload, HostRequest::RequestPermission { .. })));

    let duplicate = core.submit(AgentSubmission::CreateMemory { record: record() });
    assert!(duplicate.is_err());
    assert_eq!(
        core.snapshot().memories.revision,
        1,
        "failed CRUD must roll back"
    );

    let mut updated = record();
    updated.content = "Prefer typed, checkpointed Core contracts".to_string();
    let changed = core
        .submit(AgentSubmission::UpdateMemory {
            record: updated.clone(),
        })
        .expect("update memory");
    assert_eq!(changed.snapshot.memories.revision, 2);
    assert_eq!(changed.snapshot.memories.get(&updated.id), Some(&updated));

    let deleted = core
        .submit(AgentSubmission::DeleteMemory {
            memory_id: updated.id.clone(),
        })
        .expect("delete memory");
    assert_eq!(deleted.snapshot.memories.revision, 3);
    assert!(deleted.snapshot.memories.is_empty());
}

#[test]
fn memory_projection_round_trips_through_core_checkpoint() {
    let mut core = AgentCore::try_new(config()).expect("create Core");
    let transition = core
        .submit(AgentSubmission::CreateMemory { record: record() })
        .expect("create memory");
    assert!(
        transition.durability.is_some(),
        "memory mutation must request durability"
    );
    let checkpoint: CoreCheckpoint<AgentThreadState> = core.checkpoint(CheckpointReason::Manual);
    let restored = AgentCore::restore(config(), checkpoint).expect("restore Core");
    assert_eq!(restored.snapshot().memories, transition.snapshot.memories);
    assert_eq!(
        restored
            .snapshot()
            .memories
            .records_in_namespace(&MemoryNamespace::Workspace),
        vec![transition
            .snapshot
            .memories
            .get("memory-preference-1")
            .unwrap()]
    );
}

#[test]
fn ordinary_user_text_never_routes_to_memory_crud() {
    let mut core = AgentCore::try_new(config()).expect("create Core");
    let transition = core
        .submit(AgentSubmission::UserInput {
            items: vec![AgentItem::user_message(
                ItemId::from_string("user-keywords".to_string()),
                "remember memory_save this preference forever",
            )],
            turn_context_items: Vec::new(),
            turn_context_platform: ContextPlatform::Unknown,
            turn_context_components: Vec::new(),
            submission_id: Some("memory-keyword-probe".to_string()),
            submitted_at_epoch_ms: None,
            delivery: AgentInputDelivery::CurrentTurn,
        })
        .expect("submit ordinary user text");
    assert!(transition.snapshot.memories.is_empty());
    assert!(!transition
        .event_envelopes
        .iter()
        .any(|envelope| matches!(envelope.event, AgentEvent::MemoryUpdated { .. })));
}

#[test]
fn typed_memory_injected_into_a_turn_records_usage_once_with_host_time() {
    const USED_AT_EPOCH_MS: i64 = 1_785_600_000_123;
    let memory = record();
    let mut core = AgentCore::try_new(config()).expect("create Core");
    core.submit(AgentSubmission::CreateMemory {
        record: memory.clone(),
    })
    .expect("create typed memory");

    let transition = core
        .submit(AgentSubmission::UserInput {
            items: vec![AgentItem::user_message(
                ItemId::from_string("memory-context-user".to_string()),
                "Use my saved preference",
            )],
            turn_context_items: Vec::new(),
            turn_context_platform: ContextPlatform::Unknown,
            turn_context_components: vec![
                ContextComponent {
                    kind: ContextComponentKind::Memory,
                    id: "memory-context-primary".to_string(),
                    content: serde_json::to_value(&memory).expect("encode memory"),
                    source: None,
                },
                ContextComponent {
                    kind: ContextComponentKind::Memory,
                    id: "memory-context-duplicate".to_string(),
                    content: serde_json::to_value(&memory).expect("encode duplicate memory"),
                    source: None,
                },
            ],
            submission_id: Some("memory-context-submission".to_string()),
            submitted_at_epoch_ms: Some(USED_AT_EPOCH_MS),
            delivery: AgentInputDelivery::CurrentTurn,
        })
        .expect("start turn with typed memory");

    let usage = transition.snapshot.memory_runtime.usage_ledger.as_slice();
    assert_eq!(usage.len(), 1);
    assert_eq!(usage[0].memory_id, memory.id);
    assert_eq!(usage[0].used_at_epoch_ms, USED_AT_EPOCH_MS);
    assert_eq!(
        transition
            .snapshot
            .memory_runtime
            .usage_stats
            .get(&memory.id)
            .map(|stats| (stats.usage_count, stats.last_used_at_epoch_ms)),
        Some((1, Some(USED_AT_EPOCH_MS)))
    );
}

#[test]
fn frozen_memory_policy_admits_typed_context_before_durable_turn_and_model_prompt() {
    for enabled in [false, true] {
        let mut configuration = config();
        configuration.memory_policy.long_term_enabled = enabled;
        let mut core = AgentCore::try_new(configuration).unwrap();
        let components = [
            (ContextComponentKind::Memory, "ordinary-id", "CROSS_SESSION_SECRET"),
            (ContextComponentKind::SessionMemory, "memory_save", "SESSION_SUMMARY memory search 记忆"),
            (ContextComponentKind::UserPreference, "memory_search", "USER_PREFERENCE"),
            (ContextComponentKind::PersonalIntelligence, "long-term-memory", "PERSONAL_CONTEXT"),
        ].into_iter().map(|(kind, id, text)| ContextComponent {
            kind, id: id.into(), content: json!(text), source: None,
        }).collect();
        let transition = core.submit(AgentSubmission::UserInput {
            items: vec![AgentItem::user_message(ItemId::from_string("user".into()), "继续 search memory")],
            turn_context_items: Vec::new(),
            turn_context_platform: ContextPlatform::Unknown,
            turn_context_components: components,
            submission_id: Some("context-policy".into()),
            submitted_at_epoch_ms: None,
            delivery: AgentInputDelivery::CurrentTurn,
        }).unwrap();
        let request = transition.host_requests.iter().find_map(|request| {
            if let HostRequest::ModelStream { messages, .. } = &request.payload { Some(messages) } else { None }
        }).expect("real model request");
        let prompt = serde_json::to_string(request).unwrap();
        let checkpoint = core.checkpoint(CheckpointReason::Manual);
        let context = &checkpoint.state.execution.active_turn.as_ref().unwrap().context;
        assert_eq!(context.components.iter().any(|component| component.kind == ContextComponentKind::Memory), enabled);
        assert_eq!(prompt.contains("CROSS_SESSION_SECRET"), enabled);
        assert_eq!(serde_json::to_string(context).unwrap().contains("CROSS_SESSION_SECRET"), enabled);
        for text in ["SESSION_SUMMARY", "USER_PREFERENCE", "PERSONAL_CONTEXT"] {
            assert!(prompt.contains(text), "missing {text}, enabled={enabled}");
        }
        assert_eq!(serde_json::to_value(ContextComponentKind::SessionMemory).unwrap(), json!("session_memory"));
    }
}

#[test]
fn semantic_projection_rejects_owner_reassignment_without_changing_its_revision() {
    let mut projection = nola_domain::memory_projection::MemoryProjection::default();
    let original = record();
    projection.apply(MemoryUpdate::Created { record: original.clone() }).unwrap();
    let before = projection.clone();
    let mut changed = original;
    changed.owner = nola_domain::memory_owner::MemoryOwner::Workspace {
        profile_id: "local".into(), workspace_id: "foreign".into(),
    };
    assert!(projection.apply(MemoryUpdate::Updated { record: changed }).is_err());
    assert_eq!(projection, before);
}

}

pub mod runtime_jobs {
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

}
