// SPDX-License-Identifier: Apache-2.0
//! Consolidated Memory Lifecycle suite:
//! deduplication, explicit-user authority over automatic inference, literal SQL
//! metacharacter & unsegmented Unicode recall, timezone expiry, sensitivity
//! filtering, supersession, 70-row concrete multi-owner isolation, receipted
//! replacement/swap/tombstone/immutability, and consolidation input conflict guards.

use crate::support;
use crate::support::model_identity as fixture_model_identity;
use nola_agent_engine::agent::memory_runtime::{
    consolidation_input_digest, MemoryConsolidationOutput, MemoryJobOutput,
    MemoryRuntimeCommandEnvelope,
};
use nola_agent_engine::agent::{
    AgentCoreConfig, AgentSubmission, MemoryKind, MemoryNamespace, MemoryProvenance,
    MemoryProvenanceKind, MemoryQualityAssessment, MemoryRecord as SemanticMemoryRecord,
    MemoryRetention, MemoryRuntimeCommand, MemoryScope,
};
use nola_application::{
    execute_application_command, reduce_and_commit_idle_submission, ApplicationCommand,
    ApplicationCommandReceiptOutcome, ExecuteReceivedCommand, MemoryCandidate, MemoryCommand,
    MemoryCommitIntent, MemoryRecallQuery, MemoryRecord, ReduceAndCommitIdleSubmission,
};
use nola_domain::memory_owner::{MemoryExecutionBinding, MemoryOwner};
use nola_store::{CreateConversationInput, NolaStore};
use serde_json::{json, Value};
use support::ApplicationRuntime;
use tempfile::tempdir;

fn memory(id: &str, content: &str, proposed_at_ms: i64) -> MemoryCandidate {
    MemoryCandidate {
        record: SemanticMemoryRecord {
            id: id.into(),
            owner: MemoryOwner::local_profile(),
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

fn recall(store: &mut NolaStore, namespace: MemoryNamespace, now: &str) -> Vec<MemoryRecord> {
    let binding = MemoryExecutionBinding {
        workspace_id: Some("workspace".into()),
        ..MemoryExecutionBinding::local_profile()
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
fn memory_commit_deduplication_expiry_sensitivity_supersession_and_literal_recall() {
    // 1. Strict wire schema validation: unknown fields, missing owner, and missing binding.
    assert!(serde_json::from_value::<ApplicationCommand>(json!({
        "type": "memory",
        "command": {"type": "get", "id": "memory-first", "legacy_scope": "global"}
    }))
    .is_err());
    let mut missing_owner = serde_json::to_value(memory("strict", "strict record", 1)).unwrap();
    missing_owner["record"]
        .as_object_mut()
        .unwrap()
        .remove("owner");
    assert!(serde_json::from_value::<MemoryCandidate>(missing_owner).is_err());
    assert!(serde_json::from_value::<MemoryRecallQuery>(json!({
        "namespace":{"kind":"personal"},"scope":"global","query":null,
        "kind":null,"now_iso":"2026-11-08T00:00:00Z","limit":10
    }))
    .is_err());

    // 2. Deduplication, automatic duplicate authority preservation, SQL metacharacters, and timezone expiry across cold reopen.
    let directory = tempdir().unwrap();
    let database = directory.path().join("memory-lifecycle.db");
    let canonical_id = {
        let mut store = NolaStore::open(&database).unwrap();
        let mut explicit = memory("memory-first", "Prefer concise answers.", 20);
        explicit.record.confidence = 1.0;
        let first: MemoryRecord = serde_json::from_value(
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

        let second: MemoryRecord = serde_json::from_value(
            execute_application_command(
                &mut store,
                ApplicationCommand::Memory {
                    command: MemoryCommand::Commit {
                        input: memory("memory-duplicate", "Prefer concise answers.", 30),
                        intent: MemoryCommitIntent::ExplicitUser,
                    },
                },
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(second.record.id, first.record.id);

        let mut inferred = memory("worker-candidate", "Prefer concise answers.", 35);
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
        assert_eq!(deduplicated.record.id, first.record.id);
        assert_eq!(
            deduplicated.record.provenance.kind,
            MemoryProvenanceKind::ExplicitUser
        );
        assert_eq!(deduplicated.record.provenance.source, "memory-port-e2e");
        assert_eq!((deduplicated.record.confidence, deduplicated.updated_at_ms), (0.9, 30));

        // Literal SQL metacharacters and custom namespace isolation.
        for (id, content, ns) in [
            ("percent", "100% complete", "fixture"),
            ("underscore", "snake_case", "fixture"),
            ("slash", r"path\name", "fixture"),
            ("unicode", "预算约束", "fixture"),
            ("plain", "ordinary content", "fixture"),
            ("foreign", "100% complete", "foreign"),
        ] {
            let cmd: MemoryCommand = serde_json::from_value(json!({
                "type": "commit", "intent": "explicit_user", "input": {
                    "record": {
                        "id": id, "owner": {"kind":"profile","profile_id":"local"},
                        "namespace": {"kind": "custom", "name": ns},
                        "scope": "global", "kind": "reference", "content": content,
                        "summary": null, "tags": [], "confidence": 0.8, "importance": 0.9,
                        "sensitivity": null, "retention": {"mode": "permanent"}, "supersedes": [],
                        "provenance": {"kind": "explicit_user", "source": "test", "session_id": null,
                            "turn_id": null, "item_ids": [], "observed_at": null}
                    }, "quality": null, "proposed_at_ms": 1
                }
            }))
            .unwrap();
            execute_application_command(&mut store, ApplicationCommand::Memory { command: cmd })
                .unwrap();
        }

        // Workspace & expiring records in custom/workspace namespaces.
        let mut workspace = memory("workspace", "Workspace-only preference", 2);
        workspace.record.namespace = MemoryNamespace::Workspace;
        workspace.record.owner = MemoryOwner::Workspace {
            profile_id: "local".into(),
            workspace_id: "workspace".into(),
        };
        execute_application_command(
            &mut store,
            ApplicationCommand::Memory {
                command: MemoryCommand::Commit {
                    input: workspace,
                    intent: MemoryCommitIntent::ExplicitUser,
                },
            },
        )
        .unwrap();

        first.record.id
    };

    let mut reopened = NolaStore::open(&database).unwrap();
    let restored: Option<MemoryRecord> = serde_json::from_value(
        execute_application_command(
            &mut reopened,
            ApplicationCommand::Memory {
                command: MemoryCommand::Get { id: canonical_id.clone() },
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(restored.unwrap().record.content, "Prefer concise answers.");

    for (query, id) in [
        ("%", "percent"),
        ("_", "underscore"),
        (r"\", "slash"),
        ("预算", "unicode"),
    ] {
        let cmd: MemoryCommand = serde_json::from_value(json!({
            "type": "recall", "input": {
                "binding": {"profile_id":"local"},
                "namespace": {"kind": "custom", "name": "fixture"}, "scope": "global",
                "query": query, "kind": null, "now_iso": "2026-09-08T00:00:00Z", "limit": 20
            }
        }))
        .unwrap();
        let result =
            execute_application_command(&mut reopened, ApplicationCommand::Memory { command: cmd })
                .unwrap();
        assert_eq!(result.as_array().unwrap().len(), 1, "query={query}");
        assert_eq!(result[0]["record"]["id"], id);
    }

    let ws_before = recall(&mut reopened, MemoryNamespace::Workspace, "2026-09-07T00:00:01Z");
    assert_eq!(ws_before.len(), 1);
    assert_eq!(ws_before[0].record.id, "workspace");
    assert_eq!(ws_before[0].record.provenance.item_ids, vec!["message-1"]);
    execute_application_command(
        &mut reopened,
        ApplicationCommand::Memory {
            command: MemoryCommand::Delete {
                id: "workspace".into(),
                deleted_at_ms: 3,
            },
        },
    )
    .unwrap();
    drop(reopened);
    let mut reopened = NolaStore::open(&database).unwrap();
    assert!(recall(&mut reopened, MemoryNamespace::Workspace, "2026-09-07T00:00:01Z").is_empty());

    // 3. Timezone expiry, invalid clock/retention, sensitivity, budget, supersession, reordered & CJK recall.
    let mut expiring_store = NolaStore::open(":memory:").unwrap();
    let mut invalid_exp = memory("invalid", "Temporary", 1);
    invalid_exp.record.retention = MemoryRetention::ExpiresAt {
        expires_at: "tomorrow".into(),
    };
    assert!(execute_application_command(
        &mut expiring_store,
        ApplicationCommand::Memory {
            command: MemoryCommand::Commit {
                input: invalid_exp,
                intent: MemoryCommitIntent::ExplicitUser,
            },
        }
    )
    .is_err());
    assert!(execute_application_command(
        &mut expiring_store,
        ApplicationCommand::Memory {
            command: MemoryCommand::Recall {
                input: MemoryRecallQuery {
                    binding: MemoryExecutionBinding::local_profile(),
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

    let mut expiring = memory("expiring", "Temporary preference", 1);
    expiring.record.retention = MemoryRetention::ExpiresAt {
        expires_at: "2026-09-07T08:00:00+08:00".into(),
    };
    execute_application_command(
        &mut expiring_store,
        ApplicationCommand::Memory {
            command: MemoryCommand::Commit {
                input: expiring,
                intent: MemoryCommitIntent::ExplicitUser,
            },
        },
    )
    .unwrap();
    assert_eq!(
        recall(
            &mut expiring_store,
            MemoryNamespace::Personal,
            "2026-09-06T23:59:59Z"
        )[0]
        .record
        .id,
        "expiring"
    );
    for boundary in [
        "2026-09-07T00:00:00Z",
        "2026-09-06T20:00:00-04:00",
        "2026-09-07T00:00:01Z",
    ] {
        assert!(recall(&mut expiring_store, MemoryNamespace::Personal, boundary).is_empty());
    }

    // Sensitivity labels, confidence, Core budget, supersession, reordered terms, CJK, and cross-kind supersession.
    let mut recall_store = NolaStore::open(":memory:").unwrap();
    let mut normal = memory("ordinary-preference", "The user prefers concise release notes.", 1);
    normal.record.sensitivity = Some("normal".into());
    let mut private_health = memory("sensitive", "Private health detail.", 2);
    private_health.record.sensitivity = Some("health.private".into());
    let mut private_code = memory(
        "restricted-detail",
        "Concise release notes contain a private launch code.",
        3,
    );
    private_code.record.sensitivity = Some("sensitive".into());
    let mut old_proc = memory("old-process", "Use the legacy release process.", 4);
    old_proc.record.importance = 1.0;
    let mut reviewed_proc = memory(
        "reviewed-process",
        "Require two reviewers before publishing the reviewed release process.",
        5,
    );
    reviewed_proc.record.importance = 0.6;
    reviewed_proc.record.supersedes = vec!["old-process".into()];
    let mut weak = memory("weak", "Unverified inferred preference.", 6);
    weak.record.confidence = 0.2;
    let oversized = memory("oversized", &"x".repeat(40_000), 7);
    let cjk = memory("color", "用户最喜欢蓝色。", 8);
    let cross_old = memory("old-pref-cross", "Use archaic deployment script.", 9);
    let mut cross_new = memory("replacement-decision", "Require automated rollout.", 10);
    cross_new.record.kind = MemoryKind::Decision;
    cross_new.record.supersedes = vec!["old-pref-cross".into()];

    for candidate in [
        normal,
        private_health,
        private_code,
        old_proc,
        reviewed_proc,
        weak,
        oversized,
        cjk,
        cross_old,
        cross_new,
    ] {
        execute_application_command(
            &mut recall_store,
            ApplicationCommand::Memory {
                command: MemoryCommand::Commit {
                    input: candidate,
                    intent: MemoryCommitIntent::ExplicitUser,
                },
            },
        )
        .unwrap();
    }

    let default_recalled = recall(
        &mut recall_store,
        MemoryNamespace::Personal,
        "2026-09-12T00:00:00Z",
    );
    let ids: Vec<&str> = default_recalled
        .iter()
        .map(|m| m.record.id.as_str())
        .collect();
    assert!(ids.contains(&"ordinary-preference"));
    assert!(ids.contains(&"reviewed-process"));
    assert!(!ids.contains(&"sensitive") && !ids.contains(&"restricted-detail"));
    assert!(!ids.contains(&"old-process") && !ids.contains(&"weak") && !ids.contains(&"oversized"));
    assert!(execute_application_command(
        &mut recall_store,
        ApplicationCommand::Memory {
            command: MemoryCommand::Get {
                id: "sensitive".into()
            }
        }
    )
    .unwrap()
    .is_object());

    let query_recall = |store: &mut NolaStore, q: &str, kind: Option<MemoryKind>| -> Vec<MemoryRecord> {
        serde_json::from_value(
            execute_application_command(
                store,
                ApplicationCommand::Memory {
                    command: MemoryCommand::Recall {
                        input: MemoryRecallQuery {
                            binding: MemoryExecutionBinding::local_profile(),
                            namespace: MemoryNamespace::Personal,
                            scope: MemoryScope::Global,
                            query: Some(q.into()),
                            kind,
                            now_iso: "2026-09-12T00:00:00Z".into(),
                            limit: 10,
                        },
                    },
                },
            )
            .unwrap(),
        )
        .unwrap()
    };

    let notes = query_recall(&mut recall_store, "concise release notes", None);
    assert_eq!(
        notes.iter().map(|r| r.record.id.as_str()).collect::<Vec<_>>(),
        vec!["ordinary-preference", "reviewed-process"]
    );

    let superseded_by_legacy = query_recall(&mut recall_store, "legacy", None);
    assert_eq!(superseded_by_legacy.len(), 1);
    assert_eq!(superseded_by_legacy[0].record.id, "reviewed-process");

    let reordered = query_recall(&mut recall_store, "release reviewed process", None);
    assert_eq!(reordered[0].record.id, "reviewed-process");

    let cjk_res = query_recall(&mut recall_store, "用户喜欢什么颜色", None);
    assert_eq!(cjk_res.len(), 1);
    assert_eq!(cjk_res[0].record.id, "color");

    let cross_filtered =
        query_recall(&mut recall_store, "archaic", Some(MemoryKind::Preference));
    assert!(cross_filtered.is_empty());
}

#[test]
fn concrete_owners_isolate_recall_deduplication_and_supersession_after_restart() {
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
                make(id, owner, MemoryNamespace::Conversation, MemoryScope::Conversation),
            )
            .unwrap();
            assert_eq!(result["record"]["id"], id);
        }
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
            ("turn-a", a.clone(), MemoryNamespace::Conversation, MemoryScope::Turn),
            (
                "turn-next",
                binding("conversation-a", "turn-next", "/workspace/a"),
                MemoryNamespace::Conversation,
                MemoryScope::Turn,
            ),
            ("workspace-a", a.clone(), MemoryNamespace::Workspace, MemoryScope::Global),
            ("workspace-b", b.clone(), MemoryNamespace::Workspace, MemoryScope::Global),
        ] {
            commit(&mut store, make(id, &owner, namespace, scope)).unwrap();
        }
        let mut changed_owner =
            make("a", &b, MemoryNamespace::Conversation, MemoryScope::Conversation);
        changed_owner.proposed_at_ms = 200;
        assert!(commit(&mut store, changed_owner)
            .unwrap_err()
            .contains("scope_conflict"));
        let mut cross_owner =
            make("cross", &a, MemoryNamespace::Conversation, MemoryScope::Conversation);
        cross_owner.record.supersedes = vec!["b".into()];
        assert!(commit(&mut store, cross_owner)
            .unwrap_err()
            .contains("owner_conflict"));
        assert!(store.memory_record("cross").unwrap().is_none());
    }
    let mut store = NolaStore::open(&database).unwrap();
    let recall_owned = |store: &mut NolaStore, binding: MemoryExecutionBinding, namespace, scope| {
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
        (MemoryNamespace::Conversation, MemoryScope::Conversation, "a"),
        (MemoryNamespace::Conversation, MemoryScope::Turn, "turn-a"),
        (MemoryNamespace::Workspace, MemoryScope::Global, "workspace-a"),
    ] {
        let records = recall_owned(&mut store, a.clone(), namespace, scope).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].record.id, expected);
        assert!(records[0].record.owner.is_visible_to(&a));
        assert!(!records[0].record.owner.is_visible_to(&b));
    }
    assert!(recall_owned(
        &mut store,
        MemoryExecutionBinding::local_profile(),
        MemoryNamespace::Conversation,
        MemoryScope::Conversation
    )
    .unwrap_err()
    .contains("bound conversation"));
    assert!(recall_owned(
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
fn memory_replacement_receipt_and_identity_immutability_lifecycle() {
    let candidate = |id: &str, timestamp: i64| -> MemoryCandidate {
        serde_json::from_value(json!({
            "record": {
                "id": id,
                "owner": {"kind":"profile","profile_id":"local"},
                "namespace": {"kind": "custom", "name": "fixture"},
                "scope": "global",
                "kind": "reference",
                "content": format!("Keep {id}"),
                "summary": null,
                "tags": ["fixture"],
                "confidence": 0.8,
                "importance": 0.9,
                "sensitivity": null,
                "retention": {"mode": "permanent"},
                "supersedes": [],
                "provenance": {
                    "kind": "explicit_user", "source": "test", "session_id": null,
                    "turn_id": null, "item_ids": [], "observed_at": null
                }
            },
            "quality": null,
            "proposed_at_ms": timestamp
        }))
        .unwrap()
    };
    let submit =
        |runtime: &mut ApplicationRuntime, id: &str, command: MemoryCommand| -> Option<Value> {
            let command = ApplicationCommand::Memory { command };
            let receipt = runtime
                .execute_received_command(
                    ExecuteReceivedCommand {
                        client_id: "memory-editor".into(),
                        request_id: id.into(),
                        command_type: "memory".into(),
                        command_json: serde_json::to_string(&command).unwrap(),
                        expected_revision: None,
                        received_at: "2026-09-07T00:00:00Z".into(),
                    },
                    command,
                )
                .unwrap();
            match receipt.outcome {
                ApplicationCommandReceiptOutcome::Committed { result, .. } => Some(result),
                _ => None,
            }
        };
    let list = |runtime: &mut ApplicationRuntime| -> Vec<MemoryRecord> {
        serde_json::from_value(
            runtime
                .execute(ApplicationCommand::Memory {
                    command: MemoryCommand::List { limit: 100 },
                })
                .unwrap(),
        )
        .unwrap()
    };

    let root = tempdir().unwrap();
    let path = root.path().join("replacement.db");
    let mut runtime = support::open_runtime(&path).unwrap();

    // 1. Create "keep" at t=300; test stale expected, unassessed automatic, unchanged Replace,
    //    cold reopen replay, late candidate rejection, scope/namespace move rejection, and tombstone non-resurrection.
    submit(
        &mut runtime,
        "create-keep",
        MemoryCommand::Commit {
            input: candidate("keep", 300),
            intent: MemoryCommitIntent::ExplicitUser,
        },
    )
    .unwrap();
    let before = list(&mut runtime);
    assert!(submit(
        &mut runtime,
        "stale",
        MemoryCommand::Replace {
            expected: vec![],
            candidates: vec![candidate("keep", 900)],
            intent: MemoryCommitIntent::ExplicitUser,
            deleted_at_ms: 900,
        }
    )
    .is_none());
    assert!(submit(
        &mut runtime,
        "unassessed",
        MemoryCommand::Replace {
            expected: before.clone(),
            candidates: vec![candidate("keep", 900)],
            intent: MemoryCommitIntent::ValidatedAutomatic,
            deleted_at_ms: 900,
        }
    )
    .is_none());
    assert_eq!(list(&mut runtime), before);

    let retain_cmd = MemoryCommand::Replace {
        expected: before.clone(),
        candidates: vec![candidate("keep", 900)],
        intent: MemoryCommitIntent::ExplicitUser,
        deleted_at_ms: 900,
    };
    let retain_result = submit(&mut runtime, "retain", retain_cmd.clone()).unwrap();
    assert_eq!(
        serde_json::from_value::<Vec<MemoryRecord>>(retain_result.clone()).unwrap(),
        before
    );
    drop(runtime);
    let mut runtime = support::open_runtime(&path).unwrap();
    assert_eq!(
        submit(&mut runtime, "retain", retain_cmd).unwrap(),
        retain_result
    );
    assert_eq!(list(&mut runtime), before);

    // Late candidates cannot overwrite newer canonical memory.
    let mut old = candidate("keep", 100);
    old.record.summary = Some("stale summary".into());
    for (req, id) in [("old-same-id", "keep"), ("old-content-duplicate", "other")] {
        old.record.id = id.into();
        assert!(submit(
            &mut runtime,
            req,
            MemoryCommand::Commit {
                input: old.clone(),
                intent: MemoryCommitIntent::ExplicitUser,
            }
        )
        .is_none());
        assert_eq!(list(&mut runtime), before);
    }

    // Existing identity cannot move between namespaces or scopes.
    let mut ns_change = candidate("keep", 400);
    ns_change.record.namespace = MemoryNamespace::Personal;
    let mut scope_change = candidate("keep", 400);
    scope_change.record.scope = MemoryScope::Conversation;
    for (req, inp) in [("change-namespace", ns_change), ("change-scope", scope_change)] {
        let cmd = MemoryCommand::Commit {
            input: inp,
            intent: MemoryCommitIntent::ExplicitUser,
        };
        assert!(submit(&mut runtime, req, cmd.clone()).is_none());
        assert!(submit(&mut runtime, req, cmd).is_none());
        assert_eq!(list(&mut runtime), before);
    }

    // Deleted identity cannot be resurrected.
    submit(
        &mut runtime,
        "delete-keep",
        MemoryCommand::Delete {
            id: "keep".into(),
            deleted_at_ms: 500,
        },
    )
    .unwrap();
    drop(runtime);
    let mut runtime = support::open_runtime(&path).unwrap();
    for time in [100, 600] {
        assert!(submit(
            &mut runtime,
            &format!("late-resurrect-{time}"),
            MemoryCommand::Commit {
                input: candidate("keep", time),
                intent: MemoryCommitIntent::ExplicitUser,
            }
        )
        .is_none());
    }
    assert!(list(&mut runtime).is_empty());

    // 2. Replacement swap and equal-content distinct-ID preservation across cold reopen.
    for id in ["a", "b"] {
        submit(
            &mut runtime,
            id,
            MemoryCommand::Commit {
                input: candidate(id, 100),
                intent: MemoryCommitIntent::ExplicitUser,
            },
        )
        .unwrap();
    }
    let ab_before = list(&mut runtime);
    let mut swap_a = candidate("a", 200);
    let mut swap_b = candidate("b", 200);
    std::mem::swap(&mut swap_a.record.content, &mut swap_b.record.content);
    let swapped: Vec<MemoryRecord> = serde_json::from_value(
        submit(
            &mut runtime,
            "swap",
            MemoryCommand::Replace {
                expected: ab_before,
                candidates: vec![swap_a.clone(), swap_b.clone()],
                intent: MemoryCommitIntent::ExplicitUser,
                deleted_at_ms: 200,
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        swapped.iter().map(|i| i.record.clone()).collect::<Vec<_>>(),
        vec![swap_a.record.clone(), swap_b.record.clone()]
    );

    let after_swap = list(&mut runtime);
    let mut same_a = swap_a;
    same_a.proposed_at_ms = 900;
    same_a.record.content = swap_b.record.content.clone();
    let same_cmd = MemoryCommand::Replace {
        expected: after_swap.clone(),
        candidates: vec![same_a.clone(), swap_b],
        intent: MemoryCommitIntent::ExplicitUser,
        deleted_at_ms: 900,
    };
    let same_res = submit(&mut runtime, "same-content", same_cmd.clone()).unwrap();
    let same_saved: Vec<MemoryRecord> = serde_json::from_value(same_res.clone()).unwrap();
    assert_eq!(same_saved.len(), 2);
    assert_eq!(same_saved[0].record, same_a.record);
    assert_eq!(
        &same_saved[1],
        after_swap.iter().find(|item| item.record.id == "b").unwrap()
    );
    let canonical = list(&mut runtime);
    drop(runtime);
    let mut runtime = support::open_runtime(&path).unwrap();
    assert_eq!(list(&mut runtime), canonical);
    assert_eq!(
        submit(&mut runtime, "same-content", same_cmd).unwrap(),
        same_res
    );
}

#[test]
fn memory_maintenance_consolidation_accepts_unchanged_and_rejects_edited_or_deleted_inputs() {
    let candidate = |content: &str, time: i64| -> MemoryCandidate {
        serde_json::from_value(json!({
            "record": {
                "id": "input",
                "owner": {"kind":"profile","profile_id":"local"},
                "namespace": {"kind": "personal"},
                "scope": "global",
                "kind": "preference",
                "content": content,
                "summary": "Response style",
                "tags": [],
                "confidence": 0.9,
                "importance": 0.8,
                "sensitivity": null,
                "retention": {"mode": "until_superseded"},
                "supersedes": [],
                "provenance": {
                    "kind": "explicit_user", "source": "maintenance-test",
                    "session_id": null, "turn_id": null, "item_ids": [], "observed_at": null
                }
            },
            "quality": null,
            "proposed_at_ms": time
        }))
        .unwrap()
    };
    let submit_idle = |store: &mut NolaStore, submission: AgentSubmission| -> Result<(), String> {
        reduce_and_commit_idle_submission(
            store,
            ReduceAndCommitIdleSubmission {
                conversation_id: "maintenance".into(),
                core_config: AgentCoreConfig {
                    turn_identity: fixture_model_identity::identity(),
                    session_id: Some(nola_agent_engine::agent::SessionId::from_string(
                        "maintenance".into(),
                    )),
                    ..AgentCoreConfig::default()
                },
                submission,
                committed_at_ms: 500,
            },
        )
        .map(|_| ())
    };
    let setup = |store: &mut NolaStore| {
        store
            .create_conversation(&CreateConversationInput {
                id: "maintenance".into(),
                title: "Maintenance".into(),
                revision: 1,
                settings_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
            })
            .unwrap();
        execute_application_command(
            store,
            ApplicationCommand::Memory {
                command: MemoryCommand::Commit {
                    input: candidate("Prefer concise replies", 100),
                    intent: MemoryCommitIntent::ExplicitUser,
                },
            },
        )
        .unwrap();
        let record = candidate("Prefer concise replies", 100).record;
        let digest = consolidation_input_digest(&[record.clone()]).unwrap();
        submit_idle(store, AgentSubmission::CreateMemory { record }).unwrap();
        submit_idle(
            store,
            AgentSubmission::MemoryRuntime {
                envelope: MemoryRuntimeCommandEnvelope {
                    operation_id: "create-job".into(),
                    command: MemoryRuntimeCommand::CreateConsolidation {
                        job_id: "job".into(),
                        input_watermark: "input-v1".into(),
                        input_digest: digest,
                        selected_memory_ids: vec!["input".into()],
                        now_epoch_ms: 200,
                    },
                },
            },
        )
        .unwrap();
        submit_idle(
            store,
            AgentSubmission::ClaimMemoryWorker {
                operation_id: "claim-job".into(),
                job_id: "job".into(),
                worker_id: "worker".into(),
                ownership_token: "lease".into(),
                now_epoch_ms: 201,
                lease_until_epoch_ms: 1000,
            },
        )
        .unwrap();
    };
    let completion = |direct: bool| -> AgentSubmission {
        let output = MemoryJobOutput::Consolidation(MemoryConsolidationOutput {
            records: vec![],
            summary: "No additional durable facts".into(),
            output_digest: "result-v1".into(),
            citations: vec![],
            superseded_memory_ids: vec![],
            quality_by_memory_id: Default::default(),
        });
        if direct {
            AgentSubmission::MemoryRuntime {
                envelope: MemoryRuntimeCommandEnvelope {
                    operation_id: "complete-job".into(),
                    command: MemoryRuntimeCommand::Complete {
                        job_id: "job".into(),
                        ownership_token: "lease".into(),
                        now_epoch_ms: 500,
                        completion_watermark: "input-v1".into(),
                        output,
                    },
                },
            }
        } else {
            AgentSubmission::MemoryWorkerCompleted {
                operation_id: "complete-job".into(),
                job_id: "job".into(),
                ownership_token: "lease".into(),
                now_epoch_ms: 500,
                completion_watermark: "input-v1".into(),
                output: serde_json::to_value(output).unwrap(),
            }
        }
    };

    for direct in [false, true] {
        let mut store = NolaStore::open(":memory:").unwrap();
        setup(&mut store);
        submit_idle(&mut store, completion(direct)).unwrap();
        let checkpoint = store.load_checkpoint("maintenance").unwrap().unwrap();
        let val: Value = serde_json::from_str(&checkpoint.checkpoint_json).unwrap();
        assert_eq!(
            val.pointer("/state/memory_runtime/jobs/job/state").unwrap(),
            "completed"
        );

        for deleted in [false, true] {
            let temp = tempdir().unwrap();
            let path = temp.path().join("maintenance.db");
            let mut store = NolaStore::open(&path).unwrap();
            setup(&mut store);
            if deleted {
                execute_application_command(
                    &mut store,
                    ApplicationCommand::Memory {
                        command: MemoryCommand::Delete {
                            id: "input".into(),
                            deleted_at_ms: 300,
                        },
                    },
                )
                .unwrap();
            } else {
                execute_application_command(
                    &mut store,
                    ApplicationCommand::Memory {
                        command: MemoryCommand::Commit {
                            input: candidate("Prefer detailed replies", 300),
                            intent: MemoryCommitIntent::ExplicitUser,
                        },
                    },
                )
                .unwrap();
            }
            let before = store.load_checkpoint("maintenance").unwrap().unwrap();
            drop(store);
            let mut store = NolaStore::open(&path).unwrap();
            let error = submit_idle(&mut store, completion(direct)).unwrap_err();
            assert!(error.contains("memory_input_conflict"), "{error}");
            drop(store);
            let store = NolaStore::open(&path).unwrap();
            let after = store.load_checkpoint("maintenance").unwrap().unwrap();
            assert_eq!(
                (before.revision, &before.checkpoint_json),
                (after.revision, &after.checkpoint_json)
            );
            let record = store.memory_record("input").unwrap();
            if deleted {
                assert!(record.is_none());
            } else {
                assert_eq!(record.unwrap().content, "Prefer detailed replies");
            }
        }
    }
}
