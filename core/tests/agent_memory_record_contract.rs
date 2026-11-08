#[path = "support/model_identity.rs"]
mod model_identity;
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
