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

    assert_eq!(nola_core_memory_selection_abi_version(), 4);
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
