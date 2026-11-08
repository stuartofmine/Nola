// SPDX-License-Identifier: Apache-2.0
use super::*;
use crate::{AppendMessagesInput, CreateConversationInput, MessageInput, MessagePartInput};
use nola_domain::{CanonicalBranchIntent, CanonicalBranchTarget};

const ORIGINAL: &str =
    " { \"id\": \"p\", \"message_id\": \"m\", \"turn_id\": \"root-run\", \"data\": [1, 2] } ";
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
                    kind: "tool_result".into(),
                    ordinal: 1,
                    state: "completed".into(),
                    content_text: "evidence".into(),
                    content_json: Some(ORIGINAL.into()),
                    content_digest: Some("not-a-source-hash".into()),
                    created_at_ms: 1,
                    updated_at_ms: 2,
                }],
            }],
        })
        .unwrap();
    store
}
fn owner(store: &NolaStore, conversation: &str) -> CanonicalPartOwner {
    let (message, part): (String, String) = store.connection.query_row(
        "SELECT m.id,p.id FROM messages m JOIN message_parts p ON p.message_id=m.id WHERE m.conversation_id=?1",
        params![conversation], |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
    CanonicalPartOwner::new(conversation.into(), message, part).unwrap()
}
fn intent(store: &NolaStore, source: &str, target: &str) -> CanonicalBranchIntent {
    let source_owner = owner(store, source);
    let revision = store
        .connection
        .query_row(
            "SELECT revision FROM conversations WHERE id=?1",
            params![source],
            |row| row.get(0),
        )
        .unwrap();
    CanonicalBranchIntent {
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
    }
}
fn count(store: &NolaStore, table: &str) -> i64 {
    store
        .connection
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}

#[test]
fn two_generations_keep_exact_original_json_and_root_after_sources_are_deleted() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db.sqlite");
    let mut store = seed(&path);
    let root = owner(&store, "root");
    let first = intent(&store, "root", "first");
    store.create_conversation_branch(&first).unwrap();
    let first_owner = owner(&store, "first");
    let first_record = store.part_derivation_record(&first_owner).map(|record| record.map(|record| record.derivation)).unwrap().unwrap();
    assert_eq!(first_record.immediate_source.owner, root);
    assert_eq!(first_record.root_source.as_ref().unwrap().owner, root);
    assert_eq!(
        first_record.root_source.as_ref().unwrap().run_id.as_deref(),
        Some("root-run")
    );
    assert_eq!(first_record.target.run_id, None);
    let fingerprint = first_record.root_source.as_ref().unwrap().fields[1]
        .as_ref()
        .unwrap();
    assert_eq!(
        fingerprint.sha256,
        format!("{:x}", Sha256::digest(ORIGINAL.as_bytes()))
    );
    assert_eq!(fingerprint.total_utf8_bytes, ORIGINAL.len() as u64);
    store.delete_conversation("root").unwrap();
    assert!(store.part_derivation_record(&first_owner).map(|record| record.map(|record| record.derivation)).unwrap().is_some());
    let second = intent(&store, "first", "second");
    store.create_conversation_branch(&second).unwrap();
    let second_owner = owner(&store, "second");
    let record = store.part_derivation_record(&second_owner).map(|record| record.map(|record| record.derivation)).unwrap().unwrap();
    assert_eq!(record.immediate_source.owner, first_owner);
    assert_eq!(record.immediate_source.run_id, None);
    assert_eq!(record.root_source, first_record.root_source);
    store.delete_conversation("first").unwrap();
    drop(store);
    let mut reopened = NolaStore::open(&path).unwrap();
    assert_eq!(
        reopened.part_derivation_record(&second_owner).map(|record| record.map(|record| record.derivation)).unwrap(),
        Some(record)
    );
    let raw: String = reopened
        .connection
        .query_row(
            "SELECT content_json FROM message_parts WHERE id=?1",
            params![second_owner.part_id()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(raw.as_bytes(), ORIGINAL.as_bytes());
    assert_eq!(count(&reopened, "message_part_derivations"), 1);
    reopened.delete_conversation("second").unwrap();
    assert_eq!(count(&reopened, "message_part_derivations"), 0);
}

#[test]
fn derivation_failure_rolls_back_entire_branch_and_same_intent_can_retry() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = seed(&directory.path().join("db.sqlite"));
    let input = intent(&store, "root", "first");
    let before = [
        count(&store, "conversations"),
        count(&store, "messages"),
        count(&store, "sync_outbox"),
    ];
    store.connection.execute_batch("CREATE TRIGGER fail_after_derivation BEFORE INSERT ON conversation_branch_operations BEGIN SELECT RAISE(ABORT,'final failure'); END").unwrap();
    assert!(store.create_conversation_branch(&input).is_err());
    assert_eq!(
        before,
        [
            count(&store, "conversations"),
            count(&store, "messages"),
            count(&store, "sync_outbox")
        ]
    );
    assert_eq!(count(&store, "message_part_derivations"), 0);
    store
        .connection
        .execute_batch("DROP TRIGGER fail_after_derivation")
        .unwrap();
    store.create_conversation_branch(&input).unwrap();
    let record = store
        .part_derivation_record(&owner(&store, "first")).map(|record| record.map(|record| record.derivation))
        .unwrap()
        .unwrap();
    assert!(!store.create_conversation_branch(&input).unwrap().created);
    assert_eq!(
        store.part_derivation_record(&owner(&store, "first")).map(|record| record.map(|record| record.derivation)).unwrap(),
        Some(record)
    );
    assert_eq!(count(&store, "message_part_derivations"), 1);
}

#[test]
fn changed_source_bytes_kind_and_run_cannot_inherit_prior_root_proof() {
    for alteration in ["json", "kind", "run"] {
        let directory = tempfile::tempdir().unwrap();
        let mut store = seed(&directory.path().join("db.sqlite"));
        let first = intent(&store, "root", "first");
        store.create_conversation_branch(&first).unwrap();
        let current = owner(&store, "first");
        let next = intent(&store, "first", "second");
        match alteration {
            "json" => {
                store
                    .connection
                    .execute(
                        "UPDATE message_parts SET content_json=?2 WHERE id=?1",
                        params![current.part_id(), ORIGINAL.replace("[1, 2]", "[2, 1]")],
                    )
                    .unwrap();
            }
            "kind" => {
                store
                    .connection
                    .execute(
                        "UPDATE message_parts SET kind='reasoning' WHERE id=?1",
                        params![current.part_id()],
                    )
                    .unwrap();
            }
            _ => {
                store
                    .connection
                    .execute(
                        "UPDATE messages SET run_id='different-run' WHERE id=?1",
                        params![current.message_id()],
                    )
                    .unwrap();
            }
        }
        assert!(store.part_derivation_record(&current).map(|record| record.map(|record| record.derivation)).is_err(), "{alteration}");
        assert!(
            store.create_conversation_branch(&next).is_err(),
            "{alteration}"
        );
        assert_eq!(count(&store, "conversations"), 2);
        assert_eq!(count(&store, "message_part_derivations"), 1);
    }
}

#[test]
fn caller_owner_cannot_write_or_relabel_derivation() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = seed(&directory.path().join("db.sqlite"));
    let first = intent(&store, "root", "first");
    store.create_conversation_branch(&first).unwrap();
    let current = owner(&store, "first");
    let foreign = CanonicalPartOwner::new(
        "root".into(),
        current.message_id().into(),
        current.part_id().into(),
    )
    .unwrap();
    assert!(store.part_derivation_record(&foreign).map(|record| record.map(|record| record.derivation)).is_err());
    assert!(store.connection.execute("UPDATE message_part_derivations SET derivation_json=json_set(derivation_json,'$.root_source.owner.conversation_id','forged') WHERE target_part_id=?1",
        params![current.part_id()]).is_err());
    assert!(store.part_derivation_record(&current).map(|record| record.map(|record| record.derivation)).unwrap().is_some());
    assert_eq!(count(&store, "message_part_derivations"), 1);
}

#[test]
fn missing_legacy_branch_proof_stays_unknown_root_across_later_generations() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = seed(&directory.path().join("db.sqlite"));
    let first = intent(&store, "root", "first");
    store.create_conversation_branch(&first).unwrap();
    // Model the existing remote/archive representation: conversation lineage,
    // original Part fields, and no new local Part derivation proof.
    store
        .connection
        .execute("DELETE FROM message_part_derivations", [])
        .unwrap();
    let second = intent(&store, "first", "second");
    store.create_conversation_branch(&second).unwrap();
    let second_record = store
        .part_derivation_record(&owner(&store, "second")).map(|record| record.map(|record| record.derivation))
        .unwrap()
        .unwrap();
    assert_eq!(second_record.root_source, None);
    assert_eq!(
        second_record.immediate_source.owner.conversation_id(),
        "first"
    );
    let third = intent(&store, "second", "third");
    store.create_conversation_branch(&third).unwrap();
    let third_record = store
        .part_derivation_record(&owner(&store, "third")).map(|record| record.map(|record| record.derivation))
        .unwrap()
        .unwrap();
    assert_eq!(third_record.root_source, None);
    assert_eq!(
        third_record.immediate_source.owner.conversation_id(),
        "second"
    );
    assert_eq!(third_record.target.fields, second_record.target.fields);
}
