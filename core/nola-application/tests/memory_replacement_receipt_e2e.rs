// SPDX-License-Identifier: Apache-2.0
mod support;
use nola_application::{
    ApplicationCommand, ApplicationCommandReceiptOutcome, ExecuteReceivedCommand, MemoryCandidate,
    MemoryCommand, MemoryCommitIntent, MemoryRecord,
};
use serde_json::{json, Value};
use support::ApplicationRuntime;

fn candidate(id: &str, timestamp: i64) -> MemoryCandidate {
    serde_json::from_value(json!({
        "record": {
            "id": id, "owner": {"kind":"profile","profile_id":"local"}, "namespace": {"kind": "custom", "name": "fixture"},
            "scope": "global", "kind": "reference", "content": format!("Keep {id}"),
            "summary": null, "tags": ["fixture"], "confidence": 0.8, "importance": 0.9,
            "sensitivity": null, "retention": {"mode": "permanent"}, "supersedes": [],
            "provenance": {"kind": "explicit_user", "source": "test", "session_id": null,
                "turn_id": null, "item_ids": [], "observed_at": null}
        }, "quality": null, "proposed_at_ms": timestamp
    }))
    .unwrap()
}

fn submit(runtime: &mut ApplicationRuntime, id: &str, command: MemoryCommand) -> Option<Value> {
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
}

fn list(runtime: &mut ApplicationRuntime) -> Vec<MemoryRecord> {
    serde_json::from_value(
        runtime
            .execute(ApplicationCommand::Memory {
                command: MemoryCommand::List { limit: 100 },
            })
            .unwrap(),
    )
    .unwrap()
}

#[test]
fn unchanged_replacement_preserves_snapshot_and_replays_after_cold_reopen() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("memory.db");
    let mut runtime = support::open_runtime(&path).unwrap();
    submit(
        &mut runtime,
        "create",
        MemoryCommand::Commit {
            input: candidate("keep", 100),
            intent: MemoryCommitIntent::ExplicitUser,
        },
    )
    .unwrap();
    let before = list(&mut runtime);
    let replacement = MemoryCommand::Replace {
        expected: before.clone(),
        candidates: vec![candidate("keep", 900)],
        intent: MemoryCommitIntent::ExplicitUser,
        deleted_at_ms: 900,
    };
    let result = submit(&mut runtime, "retain", replacement.clone()).unwrap();
    assert_eq!(
        serde_json::from_value::<Vec<MemoryRecord>>(result.clone()).unwrap(),
        before
    );
    assert_eq!(list(&mut runtime), before);
    drop(runtime);
    let mut runtime = support::open_runtime(&path).unwrap();
    assert_eq!(submit(&mut runtime, "retain", replacement).unwrap(), result);
    assert_eq!(list(&mut runtime), before);
}

#[test]
fn no_op_candidates_do_not_bypass_conflicts_or_automatic_quality_validation() {
    let root = tempfile::tempdir().unwrap();
    let mut runtime = support::open_runtime(root.path().join("memory.db")).unwrap();
    submit(
        &mut runtime,
        "create",
        MemoryCommand::Commit {
            input: candidate("keep", 100),
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
}

#[test]
fn replacement_preserves_distinct_ids_even_when_contents_become_equal() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("memory.db");
    let mut runtime = support::open_runtime(&path).unwrap();
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
    let before = list(&mut runtime);
    let mut edited = candidate("a", 900);
    edited.record.content = candidate("b", 100).record.content;
    let replacement = MemoryCommand::Replace {
        expected: before.clone(),
        candidates: vec![edited.clone(), candidate("b", 100)],
        intent: MemoryCommitIntent::ExplicitUser,
        deleted_at_ms: 900,
    };
    let result = submit(&mut runtime, "same-content", replacement.clone()).unwrap();
    let saved: Vec<MemoryRecord> = serde_json::from_value(result.clone()).unwrap();
    assert_eq!(saved.len(), 2);
    assert_eq!(saved[0].record, edited.record);
    assert_eq!(
        &saved[1],
        before.iter().find(|item| item.record.id == "b").unwrap()
    );
    let canonical = list(&mut runtime);
    for record in &saved {
        assert!(canonical.contains(record));
    }
    drop(runtime);
    let mut runtime = support::open_runtime(&path).unwrap();
    assert_eq!(list(&mut runtime), canonical);
    assert_eq!(
        submit(&mut runtime, "same-content", replacement).unwrap(),
        result
    );
    assert_eq!(list(&mut runtime), canonical);
}

#[test]
fn replacement_can_swap_content_without_reassigning_identity() {
    let root = tempfile::tempdir().unwrap();
    let mut runtime = support::open_runtime(root.path().join("memory.db")).unwrap();
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
    let before = list(&mut runtime);
    let mut a = candidate("a", 200);
    let mut b = candidate("b", 200);
    std::mem::swap(&mut a.record.content, &mut b.record.content);
    let result = submit(
        &mut runtime,
        "swap",
        MemoryCommand::Replace {
            expected: before,
            candidates: vec![a.clone(), b.clone()],
            intent: MemoryCommitIntent::ExplicitUser,
            deleted_at_ms: 200,
        },
    )
    .unwrap();
    let saved: Vec<MemoryRecord> = serde_json::from_value(result).unwrap();
    assert_eq!(
        saved
            .iter()
            .map(|item| item.record.clone())
            .collect::<Vec<_>>(),
        vec![a.record, b.record]
    );
    let canonical = list(&mut runtime);
    assert_eq!(canonical.len(), 2);
    for record in saved {
        assert!(canonical.contains(&record));
    }
}

#[test]
fn late_candidates_cannot_overwrite_newer_canonical_memory_after_restart() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("late-memory.db");
    let mut runtime = support::open_runtime(&path).unwrap();
    submit(
        &mut runtime,
        "new",
        MemoryCommand::Commit {
            input: candidate("keep", 300),
            intent: MemoryCommitIntent::ExplicitUser,
        },
    )
    .unwrap();
    let before = list(&mut runtime);
    drop(runtime);
    let mut runtime = support::open_runtime(&path).unwrap();
    let mut old = candidate("keep", 100);
    old.record.summary = Some("stale summary".into());
    for (request, id) in [("old-same-id", "keep"), ("old-content-duplicate", "other")] {
        old.record.id = id.into();
        assert!(submit(
            &mut runtime,
            request,
            MemoryCommand::Commit {
                input: old.clone(),
                intent: MemoryCommitIntent::ExplicitUser,
            }
        )
        .is_none());
        assert_eq!(list(&mut runtime), before);
    }
}

#[test]
fn ordinary_commit_cannot_resurrect_deleted_memory_identity() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("deleted-memory.db");
    let mut runtime = support::open_runtime(&path).unwrap();
    submit(
        &mut runtime,
        "create",
        MemoryCommand::Commit {
            input: candidate("removed", 100),
            intent: MemoryCommitIntent::ExplicitUser,
        },
    )
    .unwrap();
    submit(
        &mut runtime,
        "delete",
        MemoryCommand::Delete {
            id: "removed".into(),
            deleted_at_ms: 200,
        },
    )
    .unwrap();
    drop(runtime);
    let mut runtime = support::open_runtime(&path).unwrap();
    for time in [100, 300] {
        assert!(submit(
            &mut runtime,
            &format!("late-{time}"),
            MemoryCommand::Commit {
                input: candidate("removed", time),
                intent: MemoryCommitIntent::ExplicitUser,
            }
        )
        .is_none());
    }
    assert!(list(&mut runtime).is_empty());
}

#[test]
fn ordinary_commit_cannot_move_an_existing_identity_between_scopes() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("immutable-memory-scope.db");
    let mut runtime = support::open_runtime(&path).unwrap();
    submit(
        &mut runtime,
        "create",
        MemoryCommand::Commit {
            input: candidate("keep", 100),
            intent: MemoryCommitIntent::ExplicitUser,
        },
    )
    .unwrap();
    let before = list(&mut runtime);
    drop(runtime);
    let mut runtime = support::open_runtime(&path).unwrap();
    let mut namespace_change = candidate("keep", 200);
    namespace_change.record.namespace = nola_agent_engine::agent::MemoryNamespace::Personal;
    let mut scope_change = candidate("keep", 200);
    scope_change.record.scope = nola_agent_engine::agent::MemoryScope::Conversation;
    for (request, input) in [
        ("change-namespace", namespace_change),
        ("change-scope", scope_change),
    ] {
        let command = MemoryCommand::Commit {
            input,
            intent: MemoryCommitIntent::ExplicitUser,
        };
        assert!(submit(&mut runtime, request, command.clone()).is_none());
        assert!(submit(&mut runtime, request, command).is_none());
        assert_eq!(list(&mut runtime), before);
    }
}
