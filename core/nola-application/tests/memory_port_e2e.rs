// SPDX-License-Identifier: Apache-2.0

use nola_agent_engine::agent::{
    MemoryKind, MemoryNamespace, MemoryProvenance, MemoryProvenanceKind, MemoryQualityAssessment,
    MemoryRecord as SemanticMemoryRecord, MemoryRetention, MemoryScope,
};
use nola_application::{
    execute_application_command, ApplicationCommand, MemoryCandidate, MemoryCommand,
    MemoryCommitIntent, MemoryRecallQuery, MemoryRecord,
};
use nola_store::NolaStore;
use tempfile::tempdir;

fn memory(id: &str, content: &str, proposed_at_ms: i64) -> MemoryCandidate {
    MemoryCandidate {
        record: SemanticMemoryRecord {
            id: id.into(),
            owner: nola_domain::memory_owner::MemoryOwner::local_profile(),
            namespace: MemoryNamespace::Personal,
            scope: MemoryScope::Global,
            kind: MemoryKind::Preference,
            content: content.into(),
            summary: Some("Response style".into()),
            tags: vec!["communication".into()],
            confidence: 0.9,
            importance: 0.8,
            sensitivity: None,
            retention: MemoryRetention::UntilSuperseded,
            supersedes: Vec::new(),
            provenance: MemoryProvenance {
                kind: MemoryProvenanceKind::ExplicitUser,
                source: "memory-port-e2e".into(),
                session_id: Some("conversation-1".into()),
                turn_id: Some("message-1".into()),
                item_ids: vec!["message-1".into()],
                observed_at: None,
                ..MemoryProvenance::default()
            },
        },
        quality: None,
        proposed_at_ms,
    }
}

#[test]
fn memory_application_object_deduplicates_and_survives_cold_reopen() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("memory-application-port.db");

    let canonical_id = {
        let mut store = NolaStore::open(&database).unwrap();
        let first = execute_application_command(
            &mut store,
            ApplicationCommand::Memory {
                command: MemoryCommand::Commit {
                    input: memory("memory-first", "Prefer concise answers.", 20),
                    intent: MemoryCommitIntent::ExplicitUser,
                },
            },
        )
        .unwrap();
        let first: MemoryRecord = serde_json::from_value(first).unwrap();

        let second = execute_application_command(
            &mut store,
            ApplicationCommand::Memory {
                command: MemoryCommand::Commit {
                    input: memory("memory-duplicate", "Prefer concise answers.", 30),
                    intent: MemoryCommitIntent::ExplicitUser,
                },
            },
        )
        .unwrap();
        let second: MemoryRecord = serde_json::from_value(second).unwrap();
        assert_eq!(second.record.id, first.record.id);
        first.record.id
    };

    let mut reopened = NolaStore::open(&database).unwrap();
    let restored = execute_application_command(
        &mut reopened,
        ApplicationCommand::Memory {
            command: MemoryCommand::Get {
                id: canonical_id.clone(),
            },
        },
    )
    .unwrap();
    let restored: Option<MemoryRecord> = serde_json::from_value(restored).unwrap();
    assert_eq!(restored.unwrap().record.content, "Prefer concise answers.");

    let listed = execute_application_command(
        &mut reopened,
        ApplicationCommand::Memory {
            command: MemoryCommand::List { limit: 10 },
        },
    )
    .unwrap();
    let listed: Vec<MemoryRecord> = serde_json::from_value(listed).unwrap();
    assert_eq!(listed.len(), 1);
}

#[test]
fn validated_automatic_duplicate_preserves_user_fact_authority() {
    let mut store = NolaStore::open(":memory:").unwrap();
    let mut explicit = memory("user-fact", "Prefer concise answers.", 20);
    explicit.record.confidence = 1.0;
    let saved: MemoryRecord = serde_json::from_value(
        execute_application_command(
            &mut store,
            ApplicationCommand::Memory {
                command: MemoryCommand::Commit {
                    input: explicit,
                    intent: MemoryCommitIntent::ExplicitUser,
                },
            },
        )
        .unwrap(),
    )
    .unwrap();

    let mut inferred = memory("worker-candidate", "Prefer concise answers.", 30);
    inferred.record.confidence = 0.7;
    inferred.record.summary = Some("Automatic extraction".into());
    inferred.record.provenance.kind = MemoryProvenanceKind::AgentInference;
    inferred.record.provenance.source = "nola://session/conversation-1".into();
    inferred.record.provenance.turn_id = Some("turn-2".into());
    inferred.record.provenance.item_ids = vec!["turn-2".into()];
    inferred.quality = Some(MemoryQualityAssessment {
        applicable: true,
        durable: true,
        legible: true,
        rationale: "Stable response preference stated by the user".into(),
    });
    let deduplicated: MemoryRecord = serde_json::from_value(
        execute_application_command(
            &mut store,
            ApplicationCommand::Memory {
                command: MemoryCommand::Commit {
                    input: inferred,
                    intent: MemoryCommitIntent::ValidatedAutomatic,
                },
            },
        )
        .unwrap(),
    )
    .unwrap();

    assert_eq!(deduplicated.record.id, saved.record.id);
    assert_eq!(
        deduplicated.record.provenance.kind,
        MemoryProvenanceKind::ExplicitUser
    );
    assert_eq!(deduplicated.record.provenance.source, "memory-port-e2e");
    assert_eq!(deduplicated.record.confidence, 1.0);
    assert_eq!(deduplicated.updated_at_ms, 20);
}

#[test]
fn memory_command_rejects_unknown_fields() {
    let raw = serde_json::json!({
        "type": "memory",
        "command": {
            "type": "get",
            "id": "memory-first",
            "legacy_scope": "global"
        }
    });
    assert!(serde_json::from_value::<ApplicationCommand>(raw).is_err());
}

fn recall(store: &mut NolaStore, namespace: MemoryNamespace, now: &str) -> Vec<MemoryRecord> {
    let binding = nola_domain::memory_owner::MemoryExecutionBinding {
        workspace_id: Some("workspace".into()),
        ..nola_domain::memory_owner::MemoryExecutionBinding::local_profile()
    };
    serde_json::from_value(
        execute_application_command(
            store,
            ApplicationCommand::Memory {
                command: MemoryCommand::Recall {
                    input: MemoryRecallQuery {
                        binding,
                        namespace,
                        scope: MemoryScope::Global,
                        query: None,
                        kind: None,
                        now_iso: now.into(),
                        limit: 10,
                    },
                },
            },
        )
        .unwrap(),
    )
    .unwrap()
}

#[test]
fn recall_observes_timezone_expiry_namespace_and_deletion_after_cold_reopen() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("lifecycle.db");
    {
        let mut store = NolaStore::open(&database).unwrap();
        let mut expiring = memory("expiring", "Temporary preference", 1);
        expiring.record.retention = MemoryRetention::ExpiresAt {
            expires_at: "2026-09-07T08:00:00+08:00".into(),
        };
        let mut workspace = memory("workspace", "Workspace-only preference", 2);
        workspace.record.namespace = MemoryNamespace::Workspace;
        workspace.record.owner = nola_domain::memory_owner::MemoryOwner::Workspace {
            profile_id: "local".into(),
            workspace_id: "workspace".into(),
        };
        for candidate in [expiring, workspace] {
            execute_application_command(
                &mut store,
                ApplicationCommand::Memory {
                    command: MemoryCommand::Commit {
                        input: candidate,
                        intent: MemoryCommitIntent::ExplicitUser,
                    },
                },
            )
            .unwrap();
        }
    }
    let mut store = NolaStore::open(&database).unwrap();
    let before = recall(
        &mut store,
        MemoryNamespace::Personal,
        "2026-09-06T23:59:59Z",
    );
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].record.id, "expiring");
    for boundary in [
        "2026-09-07T00:00:00Z",
        "2026-09-06T20:00:00-04:00",
        "2026-09-07T00:00:01Z",
    ] {
        assert!(recall(&mut store, MemoryNamespace::Personal, boundary).is_empty());
    }
    let workspace = recall(
        &mut store,
        MemoryNamespace::Workspace,
        "2026-09-07T00:00:01Z",
    );
    assert_eq!(workspace.len(), 1);
    assert_eq!(workspace[0].record.id, "workspace");
    assert_eq!(workspace[0].record.provenance.item_ids, vec!["message-1"]);
    execute_application_command(
        &mut store,
        ApplicationCommand::Memory {
            command: MemoryCommand::Delete {
                id: "workspace".into(),
                deleted_at_ms: 3,
            },
        },
    )
    .unwrap();
    drop(store);
    let mut store = NolaStore::open(&database).unwrap();
    assert!(recall(
        &mut store,
        MemoryNamespace::Workspace,
        "2026-09-07T00:00:01Z"
    )
    .is_empty());
    let expired = execute_application_command(
        &mut store,
        ApplicationCommand::Memory {
            command: MemoryCommand::Get {
                id: "expiring".into(),
            },
        },
    )
    .unwrap();
    let expired: Option<MemoryRecord> = serde_json::from_value(expired).unwrap();
    assert_eq!(
        expired.unwrap().record.provenance.item_ids,
        vec!["message-1"]
    );
}

#[test]
fn recall_does_not_inject_labeled_sensitive_memory_without_typed_authorization() {
    let mut store = NolaStore::open(":memory:").unwrap();
    let ordinary = memory("ordinary", "Prefer concise answers.", 1);
    let mut sensitive = memory("sensitive", "Private health detail.", 2);
    sensitive.record.sensitivity = Some("health.private".into());
    for candidate in [ordinary, sensitive] {
        execute_application_command(
            &mut store,
            ApplicationCommand::Memory {
                command: MemoryCommand::Commit {
                    input: candidate,
                    intent: MemoryCommitIntent::ExplicitUser,
                },
            },
        )
        .unwrap();
    }

    let recalled = recall(
        &mut store,
        MemoryNamespace::Personal,
        "2026-09-10T00:00:00Z",
    );
    assert_eq!(recalled.len(), 1);
    assert_eq!(recalled[0].record.id, "ordinary");
    assert!(execute_application_command(
        &mut store,
        ApplicationCommand::Memory {
            command: MemoryCommand::Get {
                id: "sensitive".into(),
            },
        },
    )
    .unwrap()
    .is_object());
}

#[test]
fn invalid_expiry_and_recall_clock_are_rejected() {
    let mut store = NolaStore::open(":memory:").unwrap();
    let mut candidate = memory("invalid", "Temporary", 1);
    candidate.record.retention = MemoryRetention::ExpiresAt {
        expires_at: "tomorrow".into(),
    };
    assert!(execute_application_command(
        &mut store,
        ApplicationCommand::Memory {
            command: MemoryCommand::Commit {
                input: candidate,
                intent: MemoryCommitIntent::ExplicitUser
            },
        }
    )
    .is_err());
    assert!(execute_application_command(
        &mut store,
        ApplicationCommand::Memory {
            command: MemoryCommand::Recall {
                input: MemoryRecallQuery {
                    binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
                    namespace: MemoryNamespace::Personal,
                    scope: MemoryScope::Global,
                    query: None,
                    kind: None,
                    now_iso: "invalid".into(),
                    limit: 10,
                }
            },
        }
    )
    .is_err());
}

#[test]
fn recall_returns_only_current_confident_facts_within_the_core_budget() {
    let mut store = NolaStore::open(":memory:").unwrap();
    let old = memory("old", "Use the legacy release process.", 1);
    let mut correction = memory("correction", "Use the reviewed release process.", 2);
    correction.record.supersedes = vec!["old".into()];
    let mut weak = memory("weak", "Unverified inferred preference.", 3);
    weak.record.confidence = 0.2;
    let oversized = memory("oversized", &"x".repeat(40_000), 4);
    for candidate in [old, correction, weak, oversized] {
        execute_application_command(
            &mut store,
            ApplicationCommand::Memory {
                command: MemoryCommand::Commit {
                    input: candidate,
                    intent: MemoryCommitIntent::ExplicitUser,
                },
            },
        )
        .unwrap();
    }

    let recalled = recall(
        &mut store,
        MemoryNamespace::Personal,
        "2026-09-10T00:00:00Z",
    );
    assert_eq!(
        recalled
            .iter()
            .map(|memory| memory.record.id.as_str())
            .collect::<Vec<_>>(),
        vec!["correction"]
    );
}

#[test]
fn on_demand_recall_returns_an_explicitly_normal_memory_after_commit() {
    let mut store = NolaStore::open(":memory:").unwrap();
    let mut candidate = memory(
        "ordinary-preference",
        "The user prefers concise release notes.",
        1,
    );
    candidate.record.sensitivity = Some("normal".into());
    execute_application_command(
        &mut store,
        ApplicationCommand::Memory {
            command: MemoryCommand::Commit {
                input: candidate,
                intent: MemoryCommitIntent::ExplicitUser,
            },
        },
    )
    .unwrap();

    let mut sensitive = memory(
        "restricted-detail",
        "Concise release notes contain a private launch code.",
        2,
    );
    sensitive.record.sensitivity = Some("sensitive".into());
    execute_application_command(
        &mut store,
        ApplicationCommand::Memory {
            command: MemoryCommand::Commit {
                input: sensitive,
                intent: MemoryCommitIntent::ExplicitUser,
            },
        },
    )
    .unwrap();

    let recalled: Vec<MemoryRecord> = serde_json::from_value(
        execute_application_command(
            &mut store,
            ApplicationCommand::Memory {
                command: MemoryCommand::Recall {
                    input: MemoryRecallQuery {
                        binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
                        namespace: MemoryNamespace::Personal,
                        scope: MemoryScope::Global,
                        query: Some("concise release notes".into()),
                        kind: None,
                        now_iso: "2026-09-12T00:00:00Z".into(),
                        limit: 10,
                    },
                },
            },
        )
        .unwrap(),
    )
    .unwrap();

    assert_eq!(recalled.len(), 1);
    assert_eq!(recalled[0].record.id, "ordinary-preference");
    assert_eq!(recalled[0].record.sensitivity.as_deref(), Some("normal"));
}

#[test]
fn literal_recall_includes_differently_worded_superseding_fact() {
    let mut store = NolaStore::open(":memory:").unwrap();
    let mut old = memory("old-process", "Use the legacy release process.", 1);
    old.record.importance = 1.0;
    let mut correction = memory(
        "reviewed-process",
        "Require two reviewers before publishing.",
        2,
    );
    correction.record.importance = 0.6;
    correction.record.supersedes = vec!["old-process".into()];
    for candidate in [old, correction] {
        execute_application_command(
            &mut store,
            ApplicationCommand::Memory {
                command: MemoryCommand::Commit {
                    input: candidate,
                    intent: MemoryCommitIntent::ExplicitUser,
                },
            },
        )
        .unwrap();
    }

    let recalled: Vec<MemoryRecord> = serde_json::from_value(
        execute_application_command(
            &mut store,
            ApplicationCommand::Memory {
                command: MemoryCommand::Recall {
                    input: MemoryRecallQuery {
                        binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
                        namespace: MemoryNamespace::Personal,
                        scope: MemoryScope::Global,
                        query: Some("legacy".into()),
                        kind: None,
                        now_iso: "2026-09-10T00:00:00Z".into(),
                        limit: 10,
                    },
                },
            },
        )
        .unwrap(),
    )
    .unwrap();

    assert_eq!(recalled.len(), 1);
    assert_eq!(recalled[0].record.id, "reviewed-process");
    assert_eq!(
        recalled[0].record.content,
        "Require two reviewers before publishing."
    );
}

#[test]
fn recall_matches_reordered_terms_without_requiring_the_exact_query_phrase() {
    let mut store = NolaStore::open(":memory:").unwrap();
    execute_application_command(
        &mut store,
        ApplicationCommand::Memory {
            command: MemoryCommand::Commit {
                input: memory(
                    "reviewed-process",
                    "Use the reviewed release process for production.",
                    1,
                ),
                intent: MemoryCommitIntent::ExplicitUser,
            },
        },
    )
    .unwrap();

    let recalled: Vec<MemoryRecord> = serde_json::from_value(
        execute_application_command(
            &mut store,
            ApplicationCommand::Memory {
                command: MemoryCommand::Recall {
                    input: MemoryRecallQuery {
                        binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
                        namespace: MemoryNamespace::Personal,
                        scope: MemoryScope::Global,
                        query: Some("release reviewed process".into()),
                        kind: None,
                        now_iso: "2026-09-10T00:00:00Z".into(),
                        limit: 10,
                    },
                },
            },
        )
        .unwrap(),
    )
    .unwrap();

    assert_eq!(recalled.len(), 1);
    assert_eq!(recalled[0].record.id, "reviewed-process");
}

#[test]
fn recall_matches_unsegmented_unicode_text_without_a_language_dictionary() {
    let mut store = NolaStore::open(":memory:").unwrap();
    execute_application_command(
        &mut store,
        ApplicationCommand::Memory {
            command: MemoryCommand::Commit {
                input: memory("color", "用户最喜欢蓝色。", 1),
                intent: MemoryCommitIntent::ExplicitUser,
            },
        },
    )
    .unwrap();

    let recalled: Vec<MemoryRecord> = serde_json::from_value(
        execute_application_command(
            &mut store,
            ApplicationCommand::Memory {
                command: MemoryCommand::Recall {
                    input: MemoryRecallQuery {
                        binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
                        namespace: MemoryNamespace::Personal,
                        scope: MemoryScope::Global,
                        query: Some("用户喜欢什么颜色".into()),
                        kind: None,
                        now_iso: "2026-09-10T00:00:00Z".into(),
                        limit: 10,
                    },
                },
            },
        )
        .unwrap(),
    )
    .unwrap();

    assert_eq!(recalled.len(), 1);
    assert_eq!(recalled[0].record.id, "color");
}

#[test]
fn typed_literal_recall_uses_cross_kind_correction_only_to_suppress_old_fact() {
    let mut store = NolaStore::open(":memory:").unwrap();
    let old = memory("old-preference", "Use the legacy release process.", 1);
    let mut correction = memory("replacement-decision", "Require two reviewers.", 2);
    correction.record.kind = MemoryKind::Decision;
    correction.record.supersedes = vec!["old-preference".into()];
    for candidate in [old, correction] {
        execute_application_command(
            &mut store,
            ApplicationCommand::Memory {
                command: MemoryCommand::Commit {
                    input: candidate,
                    intent: MemoryCommitIntent::ExplicitUser,
                },
            },
        )
        .unwrap();
    }

    let recalled: Vec<MemoryRecord> = serde_json::from_value(
        execute_application_command(
            &mut store,
            ApplicationCommand::Memory {
                command: MemoryCommand::Recall {
                    input: MemoryRecallQuery {
                        binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
                        namespace: MemoryNamespace::Personal,
                        scope: MemoryScope::Global,
                        query: Some("legacy".into()),
                        kind: Some(MemoryKind::Preference),
                        now_iso: "2026-09-10T00:00:00Z".into(),
                        limit: 10,
                    },
                },
            },
        )
        .unwrap(),
    )
    .unwrap();

    assert!(recalled.is_empty());
}

#[test]
fn concrete_owners_isolate_recall_deduplication_and_supersession_after_restart() {
    use nola_domain::memory_owner::{MemoryExecutionBinding, MemoryOwner};
    let directory = tempdir().unwrap();
    let database = directory.path().join("owned-memory.db");
    let binding = |conversation: &str, turn: &str, workspace: &str| MemoryExecutionBinding {
        profile_id: "local".into(),
        workspace_id: Some(workspace.into()),
        conversation_id: Some(conversation.into()),
        turn_id: Some(turn.into()),
    };
    let a = binding("conversation-a", "turn-a", "/workspace/a");
    let b = binding("conversation-b", "turn-b", "/workspace/b");
    let make = |id: &str,
                binding: &MemoryExecutionBinding,
                namespace: MemoryNamespace,
                scope: MemoryScope| {
        let mut candidate = memory(id, "private isolation marker", 100);
        candidate.record.owner = binding.owner_for(&namespace, scope).unwrap();
        candidate.record.namespace = namespace;
        candidate.record.scope = scope;
        candidate
    };
    let commit = |store: &mut NolaStore, candidate: MemoryCandidate| {
        execute_application_command(
            store,
            ApplicationCommand::Memory {
                command: MemoryCommand::Commit {
                    input: candidate,
                    intent: MemoryCommitIntent::ExplicitUser,
                },
            },
        )
    };
    {
        let mut store = NolaStore::open(&database).unwrap();
        for (id, owner) in [("a", &a), ("b", &b)] {
            let result = commit(
                &mut store,
                make(
                    id,
                    owner,
                    MemoryNamespace::Conversation,
                    MemoryScope::Conversation,
                ),
            )
            .unwrap();
            assert_eq!(result["record"]["id"], id);
        }
        // Foreign high-ranked rows must be excluded in SQL before its bounded page.
        for index in 0..70 {
            let mut foreign = make(
                &format!("foreign-{index}"),
                &b,
                MemoryNamespace::Conversation,
                MemoryScope::Conversation,
            );
            foreign.record.content = format!("private isolation marker {index}");
            foreign.record.importance = 1.0;
            commit(&mut store, foreign).unwrap();
        }
        for (id, owner, namespace, scope) in [
            (
                "turn-a",
                a.clone(),
                MemoryNamespace::Conversation,
                MemoryScope::Turn,
            ),
            (
                "turn-next",
                binding("conversation-a", "turn-next", "/workspace/a"),
                MemoryNamespace::Conversation,
                MemoryScope::Turn,
            ),
            (
                "workspace-a",
                a.clone(),
                MemoryNamespace::Workspace,
                MemoryScope::Global,
            ),
            (
                "workspace-b",
                b.clone(),
                MemoryNamespace::Workspace,
                MemoryScope::Global,
            ),
        ] {
            commit(&mut store, make(id, &owner, namespace, scope)).unwrap();
        }
        let mut changed_owner = make(
            "a",
            &b,
            MemoryNamespace::Conversation,
            MemoryScope::Conversation,
        );
        changed_owner.proposed_at_ms = 200;
        assert!(commit(&mut store, changed_owner)
            .unwrap_err()
            .contains("scope_conflict"));
        let mut cross_owner = make(
            "cross",
            &a,
            MemoryNamespace::Conversation,
            MemoryScope::Conversation,
        );
        cross_owner.record.supersedes = vec!["b".into()];
        assert!(commit(&mut store, cross_owner)
            .unwrap_err()
            .contains("owner_conflict"));
        assert!(store.memory_record("cross").unwrap().is_none());
    }
    let mut store = NolaStore::open(&database).unwrap();
    let recall = |store: &mut NolaStore, binding: MemoryExecutionBinding, namespace, scope| {
        let value = execute_application_command(
            store,
            ApplicationCommand::Memory {
                command: MemoryCommand::Recall {
                    input: MemoryRecallQuery {
                        binding,
                        namespace,
                        scope,
                        query: Some("private isolation marker".into()),
                        kind: None,
                        now_iso: "2026-11-08T00:00:00Z".into(),
                        limit: 1,
                    },
                },
            },
        )?;
        serde_json::from_value::<Vec<MemoryRecord>>(value).map_err(|error| error.to_string())
    };
    for (namespace, scope, expected) in [
        (
            MemoryNamespace::Conversation,
            MemoryScope::Conversation,
            "a",
        ),
        (MemoryNamespace::Conversation, MemoryScope::Turn, "turn-a"),
        (
            MemoryNamespace::Workspace,
            MemoryScope::Global,
            "workspace-a",
        ),
    ] {
        let records = recall(&mut store, a.clone(), namespace, scope).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].record.id, expected);
        assert!(records[0].record.owner.is_visible_to(&a));
        assert!(!records[0].record.owner.is_visible_to(&b));
    }
    assert!(recall(
        &mut store,
        MemoryExecutionBinding::local_profile(),
        MemoryNamespace::Conversation,
        MemoryScope::Conversation
    )
    .unwrap_err()
    .contains("bound conversation"));
    assert!(recall(
        &mut store,
        MemoryExecutionBinding::local_profile(),
        MemoryNamespace::Workspace,
        MemoryScope::Global
    )
    .unwrap_err()
    .contains("bound workspace"));
    assert_eq!(
        store.memory_record("a").unwrap().unwrap().owner_json,
        serde_json::to_string(&MemoryOwner::Conversation {
            profile_id: "local".into(),
            conversation_id: "conversation-a".into()
        })
        .unwrap()
    );
}

#[test]
fn current_memory_wire_requires_owner_and_execution_binding() {
    let candidate = memory("strict", "strict record", 1);
    let mut value = serde_json::to_value(&candidate).unwrap();
    value["record"].as_object_mut().unwrap().remove("owner");
    assert!(serde_json::from_value::<MemoryCandidate>(value).is_err());
    let value = serde_json::json!({
        "namespace":{"kind":"personal"},"scope":"global","query":null,
        "kind":null,"now_iso":"2026-11-08T00:00:00Z","limit":10
    });
    assert!(serde_json::from_value::<MemoryRecallQuery>(value).is_err());
}
