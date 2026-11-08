// SPDX-License-Identifier: Apache-2.0
use super::*;

fn effect(padding: usize) -> OutboxEffectInput {
    OutboxEffectInput {
        effect_id: "effect".into(),
        request_id: "request".into(),
        effect_kind: "tool.execute".into(),
        model_attempt_id: None,
        delegated_invocation_id: None,
        write: crate::outbox_fixture::tool(
            "effect",
            "request",
            serde_json::json!({"original":"中🙂","large": "x".repeat(padding)}),
        ),
    }
}
fn seed(store: &mut NolaStore, padding: usize) {
    let tx = store.write_transaction().unwrap();
    tx.execute_batch("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','outbox',1,1,1);
        INSERT INTO runs(id,conversation_id,kind,status,next_event_sequence,created_at_ms,updated_at_ms)
        VALUES('r','c','maintenance','running',1,1,1)").unwrap();
    write(&tx, "r", &effect(padding), 1).unwrap();
    tx.commit().unwrap();
}

// Seed the real checkpoint writer tables: their production trigger derives
// the current delivery witness, rather than constructing a GC-only assertion.
fn seed_delivery_checkpoint(store: &mut NolaStore, pending: bool) {
    let effects = if pending {
        serde_json::json!({"effect": {
            "effect_id":"effect", "request_id":"request", "session_id":"session",
            "turn_id":"turn", "host_id":"host", "catalog_revision":1,
            "availability_epoch":1, "replay_safety":"read_only"
        }})
    } else {
        serde_json::json!({})
    };
    let checkpoint = serde_json::json!({
        "session_id":"session", "revision":1,
        "state": {"session_id":"session", "execution":{"permissions":{"pending":[]}},
            "host":{"pending_host_effects":{"effects":effects}}}
    });
    let tx = store.write_transaction().unwrap();
    tx.execute("INSERT INTO core_checkpoints VALUES('c',1,1,1)", [])
        .unwrap();
    tx.execute(
        "INSERT INTO core_checkpoint_bodies VALUES('c',?1)",
        [checkpoint.to_string()],
    )
    .unwrap();
    tx.commit().unwrap();
}
fn rebound(store: &NolaStore) -> OutboxEffectInput {
    let old = identity(&store.connection, "effect").unwrap().unwrap();
    let mut binding = old.binding.clone();
    binding.request_id = "next-request".into();
    binding.attempt_id = Some("next-attempt".into());
    OutboxEffectInput {
        effect_id: "effect".into(),
        request_id: binding.request_id.clone(),
        effect_kind: old.effect_kind,
        model_attempt_id: None,
        delegated_invocation_id: None,
        write: OutboxEffectWrite::Rebind {
            expected_request_id: old.request_id,
            expected_attempt_id: old.binding.attempt_id,
            binding,
        },
    }
}

#[test]
fn large_immutable_request_rebind_ack_and_cold_read_are_bounded_and_exact() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("outbox.db");
    let mut store = NolaStore::open_archive_mode(&path, false).unwrap();
    seed(&mut store, 4 * 1024 * 1024);
    let raw: String = store
        .connection
        .query_row("SELECT payload_json FROM effect_outbox_payloads", [], |r| {
            r.get(0)
        })
        .unwrap();
    store.connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);
        CREATE TRIGGER immutable_outbox_fixture BEFORE UPDATE ON effect_outbox_payloads BEGIN SELECT RAISE(ABORT,'immutable body rewritten'); END;
        PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
    let update = rebound(&store);
    let tx = store.write_transaction().unwrap();
    write(&tx, "r", &update, 2).unwrap();
    tx.commit().unwrap();
    let wal = std::fs::metadata(path.with_file_name("outbox.db-wal"))
        .unwrap()
        .len();
    assert!(
        wal < 256 * 1024,
        "binding+ack wrote {wal} bytes for immutable {} byte request",
        raw.len()
    );
    drop(store);
    let mut store = NolaStore::open_archive_mode(&path, false).unwrap();
    let unchanged: String = store
        .connection
        .query_row("SELECT payload_json FROM effect_outbox_payloads", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(unchanged, raw);
    let restored = store.outbox_effect("effect").unwrap().unwrap();
    assert_eq!(restored.request_id, "next-request");
    assert_eq!(restored.status, "pending");
    let request: HostRequestEnvelope = serde_json::from_str(&restored.payload_json).unwrap();
    assert_eq!(request.attempt_id.as_deref(), Some("next-attempt"));
    let HostRequest::ExecuteTool { input, .. } = request.payload else {
        panic!("wrong immutable payload")
    };
    assert_eq!(input["original"], "中🙂");
    assert_eq!(input["large"].as_str().unwrap().len(), 4 * 1024 * 1024);
    store
        .connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .unwrap();
    store.acknowledge_effect("effect", 3).unwrap();
    let wal = std::fs::metadata(path.with_file_name("outbox.db-wal"))
        .unwrap()
        .len();
    assert!(wal < 256 * 1024, "ack rewrote {wal} bytes");
    seed_delivery_checkpoint(&mut store, false);
    drop(store);
    // Exercise normal foreground-open retention, not the archive worker's
    // deliberately non-pruning connection mode.
    let store = NolaStore::open(&path).unwrap();
    assert!(store.outbox_effect("effect").unwrap().is_none());
    assert_eq!(
        store
            .connection
            .query_row("SELECT count(*) FROM effect_outbox_payloads", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn acknowledged_requests_survive_reopen_without_proof_core_consumed_them() {
    for mode in ["pending", "missing", "stale_revision", "stale_cursor"] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("retained-outbox.db");
        let mut store = NolaStore::open(&path).unwrap();
        seed(&mut store, 16);
        if mode != "missing" {
            seed_delivery_checkpoint(&mut store, mode == "pending");
            if mode == "stale_revision" {
                store
                    .connection
                    .execute("UPDATE core_checkpoints SET revision=2", [])
                    .unwrap();
            } else if mode == "stale_cursor" {
                store
                    .connection
                    .execute("UPDATE core_checkpoints SET event_cursor=2", [])
                    .unwrap();
            }
        }
        store.acknowledge_effect("effect", 3).unwrap();
        drop(store);
        let store = NolaStore::open(&path).unwrap();
        let retained = store
            .outbox_effect("effect")
            .unwrap()
            .unwrap_or_else(|| panic!("request pruned without consumption proof: {mode}"));
        assert_eq!(retained.status, "acknowledged", "{mode}");
        assert_eq!(retained.request_id, "request", "{mode}");
    }
}

#[test]
fn declaration_and_binding_rollback_and_owner_cas_remain_atomic() {
    let mut store = NolaStore::open(":memory:").unwrap();
    seed(&mut store, 32);
    let original = store.outbox_effect("effect").unwrap().unwrap();
    let update = rebound(&store);
    {
        let tx = store.write_transaction().unwrap();
        write(&tx, "r", &update, 2).unwrap();
        tx.rollback().unwrap();
    }
    assert_eq!(store.outbox_effect("effect").unwrap().unwrap(), original);
    for mode in [
        "foreign_run",
        "foreign_session",
        "stale_request",
        "stale_attempt",
    ] {
        let mut update = rebound(&store);
        if let OutboxEffectWrite::Rebind {
            expected_request_id,
            expected_attempt_id,
            binding,
        } = &mut update.write
        {
            match mode {
                "foreign_session" => binding.session_id = "foreign".into(),
                "stale_request" => *expected_request_id = "old".into(),
                "stale_attempt" => *expected_attempt_id = Some("old".into()),
                _ => {}
            }
        }
        let tx = store.write_transaction().unwrap();
        assert!(write(
            &tx,
            if mode == "foreign_run" {
                "foreign"
            } else {
                "r"
            },
            &update,
            2
        )
        .is_err());
        tx.rollback().unwrap();
    }
    store.connection.execute_batch("CREATE TRIGGER reject_immutable_body BEFORE INSERT ON effect_outbox_payloads BEGIN SELECT RAISE(ABORT,'body write rejected'); END;").unwrap();
    let tx = store.write_transaction().unwrap();
    let mut next = effect(0);
    next.effect_id = "new-effect".into();
    next.request_id = "new-request".into();
    next.write = crate::outbox_fixture::tool("new-effect", "new-request", serde_json::json!({}));
    assert!(write(&tx, "r", &next, 2).is_err());
    drop(tx);
    assert!(identity(&store.connection, "new-effect").unwrap().is_none());
    store
        .connection
        .execute(
            "DELETE FROM effect_outbox_payloads WHERE effect_id='effect'",
            [],
        )
        .unwrap();
    assert!(store.outbox_effect("effect").is_err());
    let tx = store.write_transaction().unwrap();
    assert!(write(&tx, "r", &update, 4).is_err());
}


