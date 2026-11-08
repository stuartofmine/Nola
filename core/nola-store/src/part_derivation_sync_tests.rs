// SPDX-License-Identifier: Apache-2.0
use super::*;
use crate::{
    AppendMessagesInput, ApplyRemoteSyncPageInput, CreateConversationInput, MessageInput,
    MessagePartInput, NolaStore, StoredSyncChange,
};
use nola_domain::{CanonicalBranchIntent, CanonicalBranchTarget, CanonicalPartOwner};
use nola_store_port::part_derivation::PartDerivationProvenance;

fn owner(store: &NolaStore, conversation: &str) -> CanonicalPartOwner {
    let (message, part): (String, String) = store.connection.query_row(
        "SELECT m.id,p.id FROM messages m JOIN message_parts p ON p.message_id=m.id WHERE m.conversation_id=?1",
        params![conversation], |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
    CanonicalPartOwner::new(conversation.into(), message, part).unwrap()
}
fn seed(path: &std::path::Path) -> NolaStore {
    let mut store = NolaStore::open(path).unwrap();
    store
        .create_conversation(&CreateConversationInput {
            id: "root".into(),
            title: "Root".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    store
        .append_messages(&AppendMessagesInput {
            conversation_id: "root".into(),
            expected_message_count: 0,
            updated_at_ms: 2,
            messages: vec![MessageInput {
                id: "m".into(),
                run_id: Some("root-run".into()),
                role: "assistant".into(),
                status: "completed".into(),
                ordinal: 1,
                created_at_ms: 1,
                completed_at_ms: Some(2),
                parts: vec![MessagePartInput {
                    id: "p".into(),
                    kind: "text".into(),
                    ordinal: 1,
                    state: "completed".into(),
                    content_text: "original".into(),
                    content_json: Some(" {\"id\":\"p\", \"value\":1} ".into()),
                    content_digest: Some("business-digest".into()),
                    created_at_ms: 1,
                    updated_at_ms: 2,
                }],
            }],
        })
        .unwrap();
    store
}
fn branch(store: &mut NolaStore, source: &str, target: &str) {
    let source_owner = owner(store, source);
    let revision = store
        .connection
        .query_row(
            "SELECT revision FROM conversations WHERE id=?1",
            params![source],
            |row| row.get(0),
        )
        .unwrap();
    store
        .create_conversation_branch(&CanonicalBranchIntent {
            operation_id: format!("op-{target}"),
            source_conversation_id: source.into(),
            expected_source_revision: revision,
            through_message_id: source_owner.message_id().into(),
            conversation: CanonicalBranchTarget {
                id: target.into(),
                title: target.into(),
                settings_json: None,
                created_at_ms: 3,
                updated_at_ms: 3,
            },
            committed_at_ms: 3,
        })
        .unwrap();
}
fn wire(entries: &[StoredSyncChange], cursor: i64) -> ApplyRemoteSyncPageInput {
    ApplyRemoteSyncPageInput {
        replica_id: "deriv-peer".into(),
        next_cursor: cursor + entries.len() as i64,
        applied_at_ms: 4,
        changes: entries
            .iter()
            .enumerate()
            .map(|(index, entry)| RemoteSyncChangeInput {
                cursor: cursor + index as i64 + 1,
                operation_id: entry.operation_id.clone(),
                entity_type: entry.entity_type.clone(),
                entity_id: entry.entity_id.clone(),
                revision: entry.revision,
                deleted: entry.deleted,
                payload_json: entry.payload_json.clone(),
                created_at_ms: entry.created_at_ms,
            })
            .collect(),
    }
}
fn drain(source: &std::path::Path, target: &std::path::Path) -> Vec<StoredSyncChange> {
    let mut all = Vec::new();
    for _ in 0..100 {
        let mut sender = NolaStore::open(source).unwrap();
        let pending = sender.pending_sync_changes(2).unwrap();
        if pending.is_empty() {
            return all;
        }
        let mut receiver = NolaStore::open(target).unwrap();
        let cursor = receiver
            .sync_cursor("deriv-peer")
            .unwrap()
            .map_or(0, |value| value.cursor);
        let page = wire(&pending, cursor);
        receiver.apply_remote_sync_page(&page).unwrap();
        for entry in &pending {
            sender
                .resolve_sync_change(
                    &entry.operation_id,
                    "acknowledged",
                    Some(page.next_cursor),
                    4,
                )
                .unwrap();
        }
        all.extend(pending);
    }
    panic!("branch replica did not drain")
}

#[test]
fn source_deleted_claims_cross_databases_and_replica_origin_is_never_upgraded() {
    let directory = tempfile::tempdir().unwrap();
    let left = directory.path().join("left.db");
    let right = directory.path().join("right.db");
    let mut source = seed(&left);
    branch(&mut source, "root", "first");
    let first_owner = owner(&source, "first");
    let local = source
        .part_derivation_record(&first_owner)
        .unwrap()
        .unwrap();
    assert_eq!(local.provenance, PartDerivationProvenance::LocalBranch);
    source.delete_conversation("root").unwrap();
    drop(source);
    let transferred = drain(&left, &right);
    let proof = transferred
        .iter()
        .position(|fact| fact.entity_type == "part_derivation")
        .unwrap();
    let lineage = transferred
        .iter()
        .position(|fact| fact.entity_type == "conversation_branch_lineage")
        .unwrap();
    assert!(proof < lineage);
    let mut destination = NolaStore::open(&right).unwrap();
    let received = destination
        .part_derivation_record(&first_owner)
        .unwrap()
        .unwrap();
    assert_eq!(received.derivation, local.derivation);
    assert_eq!(received.provenance, PartDerivationProvenance::Replica);
    assert!(destination.pending_sync_changes(10).unwrap().is_empty());
    branch(&mut destination, "first", "second");
    let second_owner = owner(&destination, "second");
    assert_eq!(
        destination
            .part_derivation_record(&second_owner)
            .unwrap()
            .unwrap()
            .provenance,
        PartDerivationProvenance::Replica
    );
    drop(destination);
    drain(&right, &left);
    let mut source = NolaStore::open(&left).unwrap();
    source.delete_conversation("first").unwrap();
    let second = source
        .part_derivation_record(&second_owner)
        .unwrap()
        .unwrap();
    assert_eq!(second.provenance, PartDerivationProvenance::Replica);
    assert_eq!(
        second
            .derivation
            .root_source
            .unwrap()
            .owner
            .conversation_id(),
        "root"
    );
}

#[test]
fn proof_ack_survives_restart_and_final_lineage_cannot_omit_proofs() {
    let directory = tempfile::tempdir().unwrap();
    let left = directory.path().join("left.db");
    let right = directory.path().join("right.db");
    let mut source = seed(&left);
    branch(&mut source, "root", "first");
    let pending = source.pending_sync_changes(100).unwrap();
    assert!(pending
        .iter()
        .any(|fact| fact.entity_type == "part_derivation"));
    assert!(pending
        .iter()
        .all(|fact| fact.entity_type != "conversation_branch_lineage"));
    let encoded: String = source
        .connection
        .query_row(
            "SELECT payload_json FROM sync_outbox WHERE entity_type='conversation_branch_lineage'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!dependencies_acknowledged(&source.connection, &encoded).unwrap());
    let mut target = NolaStore::open(&right).unwrap();
    let without: Vec<_> = pending
        .iter()
        .filter(|fact| fact.entity_type != "part_derivation")
        .cloned()
        .collect();
    target.apply_remote_sync_page(&wire(&without, 0)).unwrap();
    let change = RemoteSyncChangeInput {
        cursor: 1,
        operation_id: "attempt-lineage".into(),
        entity_type: "conversation_branch_lineage".into(),
        entity_id: "op-first".into(),
        revision: 1,
        deleted: false,
        payload_json: encoded.clone(),
        created_at_ms: 1,
    };
    let cursor = without.len() as i64;
    let page = ApplyRemoteSyncPageInput {
        replica_id: "deriv-peer".into(),
        next_cursor: cursor + 1,
        applied_at_ms: 4,
        changes: vec![RemoteSyncChangeInput {
            cursor: cursor + 1,
            ..change
        }],
    };
    assert!(target.apply_remote_sync_page(&page).is_err());
    assert_eq!(
        target.sync_cursor("deriv-peer").unwrap().unwrap().cursor,
        cursor
    );
    // Local outbox eligibility requires exact proof ACK, independently of target state.
    for fact in &pending {
        source
            .resolve_sync_change(&fact.operation_id, "acknowledged", Some(1), 4)
            .unwrap();
    }
    drop(source);
    let source = NolaStore::open(&left).unwrap();
    assert!(dependencies_acknowledged(&source.connection, &encoded).unwrap());
    assert_eq!(source.pending_sync_changes(10).unwrap().len(), 1);
}

#[test]
fn invalid_target_proof_rolls_back_page_and_changed_same_operation_is_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let left = directory.path().join("left.db");
    let right = directory.path().join("right.db");
    let mut source = seed(&left);
    branch(&mut source, "root", "first");
    let pending = source.pending_sync_changes(100).unwrap();
    let mut page = wire(&pending, 0);
    let proof = page
        .changes
        .iter_mut()
        .find(|fact| fact.entity_type == "part_derivation")
        .unwrap();
    let mut record: PartDerivation = serde_json::from_str(&proof.payload_json).unwrap();
    record.target.run_id = Some("injected".into());
    proof.payload_json = serde_json::to_string(&record).unwrap();
    let mut target = NolaStore::open(&right).unwrap();
    assert!(target.apply_remote_sync_page(&page).is_err());
    assert!(target.sync_cursor("deriv-peer").unwrap().is_none());
    let count: i64 = target
        .connection
        .query_row("SELECT COUNT(*) FROM conversations", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0);
    target.apply_remote_sync_page(&wire(&pending, 0)).unwrap();
    let mut changed = pending
        .iter()
        .find(|fact| fact.entity_type == "part_derivation")
        .unwrap()
        .clone();
    let mut record: PartDerivation = serde_json::from_str(&changed.payload_json).unwrap();
    record.root_source.as_mut().unwrap().run_id = Some("forged-root".into());
    changed.payload_json = serde_json::to_string(&record).unwrap();
    assert!(target
        .apply_remote_sync_page(&wire(&[changed], pending.len() as i64))
        .is_err());
}

#[test]
fn exact_receipts_survive_target_edit_delete_and_local_echo_without_recreation() {
    let directory = tempfile::tempdir().unwrap();
    let left = directory.path().join("left.db");
    let right = directory.path().join("right.db");
    let mut source = seed(&left);
    branch(&mut source, "root", "first");
    drop(source);
    let all = drain(&left, &right);
    let facts: Vec<_> = all
        .iter()
        .filter(|fact| {
            matches!(
                fact.entity_type.as_str(),
                "part_derivation" | "conversation_branch_lineage"
            )
        })
        .cloned()
        .collect();
    for path in [&left, &right] {
        let mut store = NolaStore::open(path).unwrap();
        let target = owner(&store, "first");
        store
            .connection
            .execute(
                "UPDATE message_parts SET content_text='later edit' WHERE id=?1",
                params![target.part_id()],
            )
            .unwrap();
        let cursor = store
            .sync_cursor("deriv-peer")
            .unwrap()
            .map_or(0, |v| v.cursor);
        store.apply_remote_sync_page(&wire(&facts, cursor)).unwrap();
        store.delete_conversation("first").unwrap();
        drop(store);
        let mut store = NolaStore::open(path).unwrap();
        let cursor = store.sync_cursor("deriv-peer").unwrap().unwrap().cursor;
        let mut equivalent = facts.clone();
        for fact in &mut equivalent {
            let value: serde_json::Value = serde_json::from_str(&fact.payload_json).unwrap();
            fact.payload_json = serde_json::to_string_pretty(&value).unwrap();
        }
        store
            .apply_remote_sync_page(&wire(&equivalent, cursor))
            .unwrap();
        let count: i64 = store
            .connection
            .query_row("SELECT COUNT(*) FROM message_part_derivations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
        let mut changed = facts[0].clone();
        let mut record: PartDerivation = serde_json::from_str(&changed.payload_json).unwrap();
        record.root_source = None;
        changed.payload_json = serde_json::to_string(&record).unwrap();
        assert!(store
            .apply_remote_sync_page(&wire(&[changed], cursor + facts.len() as i64))
            .is_err());
    }
}

#[test]
fn first_obsolete_facts_after_target_tombstone_are_receipted_without_resurrection() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("source.db");
    let mut source = seed(&source_path);
    branch(&mut source, "root", "first");
    let pending = source.pending_sync_changes(100).unwrap();
    for item in &pending {
        source
            .resolve_sync_change(&item.operation_id, "acknowledged", Some(1), 4)
            .unwrap();
    }
    let lineage = source.pending_sync_changes(100).unwrap().remove(0);
    let proof = pending
        .iter()
        .find(|item| item.entity_type == "part_derivation")
        .unwrap()
        .clone();
    let facts = vec![proof, lineage];
    let mut target = NolaStore::open(directory.path().join("target.db")).unwrap();
    assert!(
        target.apply_remote_sync_page(&wire(&facts, 0)).is_err(),
        "missing target without tombstone must fail"
    );
    assert!(target.sync_cursor("deriv-peer").unwrap().is_none());
    target
        .create_conversation(&CreateConversationInput {
            id: "first".into(),
            title: "temporary".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    target.delete_conversation("first").unwrap();
    target.apply_remote_sync_page(&wire(&facts, 0)).unwrap();
    assert_eq!(target.sync_cursor("deriv-peer").unwrap().unwrap().cursor, 2);
    for table in [
        "conversations",
        "message_parts",
        "message_part_derivations",
        "conversation_branch_operations",
    ] {
        let count: i64 = target
            .connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
    }
    let receipts: i64 = target
        .connection
        .query_row("SELECT COUNT(*) FROM branch_fact_receipts", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(receipts, 2);
    target.apply_remote_sync_page(&wire(&facts, 2)).unwrap();
    let mut changed = facts[1].clone();
    let mut value: serde_json::Value = serde_json::from_str(&changed.payload_json).unwrap();
    value["derivation_count"] = 2.into();
    changed.payload_json = value.to_string();
    assert!(target.apply_remote_sync_page(&wire(&[changed], 4)).is_err());
}
