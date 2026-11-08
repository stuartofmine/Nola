// SPDX-License-Identifier: Apache-2.0
//! Durable outbox dependency tests using the actual canonical database schema.
use super::*;
use nola_domain::ConversationMetadataReplica;

fn publish(store: &NolaStore, kind: &str, id: &str, revision: i64, deleted: bool, payload: &str) {
    enqueue_sync_change(
        &store.connection,
        kind,
        id,
        revision,
        deleted,
        payload,
        revision,
    )
    .unwrap();
}

fn metadata(id: &str, revision: u64, base: u64) -> String {
    serde_json::to_string(
        &ConversationMetadataReplica::new(
            id.into(),
            revision,
            base,
            "Title".into(),
            None,
            revision as i64,
        )
        .unwrap(),
    )
    .unwrap()
}

fn pending(store: &NolaStore) -> Vec<(String, i64)> {
    store
        .pending_sync_changes(100)
        .unwrap()
        .into_iter()
        .map(|entry| (entry.entity_type, entry.revision))
        .collect()
}

#[test]
fn newer_content_keeps_pending_metadata_base_and_order_after_restart() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("outbox.db");
    let store = NolaStore::open(&path).unwrap();
    publish(&store, "conversation", "c", 2, false, "{}");
    let original_id = store.pending_sync_changes(10).unwrap()[0]
        .operation_id
        .clone();
    publish(
        &store,
        "conversation_metadata",
        "c",
        4,
        false,
        &metadata("c", 4, 2),
    );
    publish(&store, "conversation", "c", 5, false, "{}");
    drop(store);
    let store = NolaStore::open(&path).unwrap();
    assert_eq!(
        pending(&store),
        vec![
            ("conversation".into(), 2),
            ("conversation_metadata".into(), 4),
            ("conversation".into(), 5),
        ]
    );
    assert_eq!(
        store.pending_sync_changes(10).unwrap()[0].operation_id,
        original_id
    );
}

#[test]
fn acknowledged_metadata_releases_its_base_for_subsequent_coalescing() {
    let root = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(root.path().join("outbox.db")).unwrap();
    publish(&store, "conversation", "c", 2, false, "{}");
    publish(
        &store,
        "conversation_metadata",
        "c",
        4,
        false,
        &metadata("c", 4, 2),
    );
    publish(&store, "conversation", "c", 5, false, "{}");
    let operation = store
        .pending_sync_changes(10)
        .unwrap()
        .into_iter()
        .find(|entry| entry.entity_type == "conversation_metadata")
        .unwrap()
        .operation_id;
    store
        .resolve_sync_change(&operation, "acknowledged", Some(2), 6)
        .unwrap();
    publish(&store, "conversation", "c", 7, false, "{}");
    assert_eq!(pending(&store), vec![("conversation".into(), 7)]);
}

#[test]
fn tombstone_clears_pending_metadata_and_bases_only_for_its_owner() {
    let root = tempfile::tempdir().unwrap();
    let store = NolaStore::open(root.path().join("outbox.db")).unwrap();
    for id in ["c", "other"] {
        publish(&store, "conversation", id, 2, false, "{}");
        publish(
            &store,
            "conversation_metadata",
            id,
            4,
            false,
            &metadata(id, 4, 2),
        );
    }
    publish(&store, "conversation", "c", 5, true, "null");
    let entries = store.pending_sync_changes(10).unwrap();
    let own: Vec<_> = entries
        .iter()
        .filter(|entry| entry.entity_id == "c")
        .collect();
    assert_eq!(own.len(), 1);
    assert!(own[0].deleted);
    assert_eq!(own[0].revision, 5);
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.entity_id == "other")
            .count(),
        2
    );
}

#[test]
fn unreferenced_snapshots_still_coalesce_without_cross_conversation_pins() {
    let root = tempfile::tempdir().unwrap();
    let store = NolaStore::open(root.path().join("outbox.db")).unwrap();
    publish(&store, "conversation", "c", 2, false, "{}");
    publish(&store, "conversation", "other", 2, false, "{}");
    publish(
        &store,
        "conversation_metadata",
        "other",
        4,
        false,
        &metadata("other", 4, 2),
    );
    publish(&store, "conversation", "c", 5, false, "{}");
    let entries = store.pending_sync_changes(10).unwrap();
    assert!(!entries
        .iter()
        .any(|entry| entry.entity_id == "c" && entry.revision == 2));
    assert!(entries
        .iter()
        .any(|entry| entry.entity_id == "other" && entry.revision == 2));
}
