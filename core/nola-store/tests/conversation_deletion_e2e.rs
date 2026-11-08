use nola_store::{CreateConversationInput, NolaStore};
use nola_store_port::conversation_deletion::ConversationDeletionRepository;
use rusqlite::{params, Connection};

fn create(store: &mut NolaStore) {
    store
        .create_conversation(&CreateConversationInput {
            id: "c".into(),
            title: "Keep history".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
}

#[test]
fn room_membership_prevents_generic_deletion_even_after_exit_or_missing_reverse_owner() {
    use nola_domain::agent_room::{AgentRoom, RoomParticipant};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("room-owned.db");
    let mut store = NolaStore::open(&path).unwrap();
    create(&mut store);
    let fixture = Connection::open(&path).unwrap();
    let room = AgentRoom::new("room".into(), "Shared".into(), vec![RoomParticipant {
        id: "member".into(), agent_id: "agent".to_string().try_into().unwrap(),
        agent_revision: 1, display_name: "Agent".into(), description: "Research".into(), context_conversation_id: "c".into(), active: true,
    }]).unwrap();
    for room in [room.clone(), room.deactivate("member").unwrap()] {
        store.put_product_setting("agent_chat_room/room", &serde_json::to_string(&room).unwrap(), 1).unwrap();
        let before = snapshot(&fixture);
        let published = publications(&fixture);
        assert!(store.delete_conversation("c").unwrap_err().to_string().contains("room participant context"));
        assert_eq!(snapshot(&fixture), before);
        assert_eq!(publications(&fixture), published);
        assert!(store.load_conversation("c").unwrap().is_some());
    }
}

fn snapshot(connection: &Connection) -> (i64, i64, i64) {
    connection
        .query_row(
            "SELECT (SELECT COUNT(*) FROM conversations),
        (SELECT COUNT(*) FROM runs),(SELECT COUNT(*) FROM sync_outbox)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap()
}

fn publications(connection: &Connection) -> Vec<(String, i64, bool, String, String)> {
    let mut statement = connection
        .prepare(
            "SELECT operation_id,revision,deleted,payload_json,status
        FROM sync_outbox WHERE entity_type='conversation' AND entity_id='c'
        ORDER BY local_sequence",
        )
        .unwrap();
    statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

fn assert_tombstone(connection: &Connection) {
    let rows = publications(connection);
    let pending: Vec<_> = rows.iter().filter(|row| row.4 == "pending").collect();
    assert_eq!(pending.len(), 1);
    let row = pending[0];
    assert!(!row.0.is_empty());
    assert_eq!((row.1, row.2, row.3.as_str()), (2, true, "null"));
    let tracked: (i64, bool, String) = connection
        .query_row(
            "SELECT revision,deleted,operation_id FROM sync_entity_revisions
         WHERE entity_type='conversation' AND entity_id='c'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(tracked, (2, true, row.0.clone()));
}

#[test]
fn deletion_rechecks_new_runs_at_commit_without_requiring_assistant_parts() {
    for status in [
        "queued",
        "running",
        "waiting_permission",
        "suspended",
        "unknown",
        "completed",
        "failed",
        "cancelled",
        "sealed",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("delete.db");
        let mut store = NolaStore::open(&path).unwrap();
        create(&mut store);
        let fixture = Connection::open(&path).unwrap();
        // Model a Run committed by another writer after an earlier idle observation.
        fixture
            .execute(
                "INSERT INTO runs(id,conversation_id,kind,status,next_event_sequence,
            created_at_ms,updated_at_ms) VALUES('r','c','maintenance',?1,1,1,1)",
                params![status],
            )
            .unwrap();
        let before = snapshot(&fixture);
        let before_publications = publications(&fixture);
        let result = store
            .conversation_write_repository()
            .delete_conversation("c");
        if status == "sealed" {
            assert!(result.unwrap(), "{status}");
            assert_eq!(snapshot(&fixture), (0, 0, 1));
            assert_tombstone(&fixture);
            let committed = publications(&fixture);
            assert!(!store.delete_conversation("c").unwrap());
            assert_eq!(publications(&fixture), committed);
        } else {
            assert!(
                result.unwrap_err().to_string().contains("Run is active"),
                "{status}"
            );
            assert_eq!(snapshot(&fixture), before);
            assert_eq!(publications(&fixture), before_publications);
        }
    }
}

#[test]
fn failed_tombstone_rolls_back_and_can_be_retried_after_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("rollback.db");
    let mut store = NolaStore::open(&path).unwrap();
    create(&mut store);
    let fixture = Connection::open(&path).unwrap();
    let before = snapshot(&fixture);
    let before_publications = publications(&fixture);
    fixture
        .execute_batch(
            "CREATE TRIGGER fixture_reject_delete
        BEFORE INSERT ON sync_outbox WHEN NEW.deleted=1
        BEGIN SELECT RAISE(ABORT,'fixture publication unavailable'); END;",
        )
        .unwrap();
    assert!(store
        .delete_conversation("c")
        .unwrap_err()
        .to_string()
        .contains("fixture publication unavailable"));
    assert_eq!(snapshot(&fixture), before);
    // enqueue_sync_change removed the pending creation before the trigger
    // rejected its replacement. Rollback must restore that exact publication.
    assert_eq!(publications(&fixture), before_publications);
    let revision: i64 = fixture
        .query_row(
            "SELECT revision FROM conversations WHERE id='c'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(revision, 1);
    fixture
        .execute_batch("DROP TRIGGER fixture_reject_delete")
        .unwrap();
    drop(store);
    let mut reopened = NolaStore::open(&path).unwrap();
    assert!(reopened.delete_conversation("c").unwrap());
    assert_eq!(snapshot(&fixture), (0, 0, 1));
    assert_tombstone(&fixture);
}

#[test]
fn deletion_retains_acknowledged_sync_history() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("acknowledged.db");
    let mut store = NolaStore::open(&path).unwrap();
    create(&mut store);
    let fixture = Connection::open(&path).unwrap();
    fixture
        .execute("UPDATE sync_outbox SET status='acknowledged'", [])
        .unwrap();
    let acknowledged = publications(&fixture);
    assert!(store.delete_conversation("c").unwrap());
    assert_eq!(snapshot(&fixture), (0, 0, 2));
    assert_eq!(publications(&fixture)[0], acknowledged[0]);
    assert_tombstone(&fixture);
}

#[test]
fn revision_overflow_does_not_remove_history_or_publications() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("overflow.db");
    let mut store = NolaStore::open(&path).unwrap();
    create(&mut store);
    let fixture = Connection::open(&path).unwrap();
    fixture
        .execute(
            "UPDATE conversations SET revision=?1 WHERE id='c'",
            params![i64::MAX],
        )
        .unwrap();
    let before = publications(&fixture);
    assert!(store
        .delete_conversation("c")
        .unwrap_err()
        .to_string()
        .contains("revision overflow"));
    assert_eq!(publications(&fixture), before);
    let revision: i64 = fixture
        .query_row(
            "SELECT revision FROM conversations WHERE id='c'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(revision, i64::MAX);
}
