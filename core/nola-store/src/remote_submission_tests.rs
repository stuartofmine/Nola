// SPDX-License-Identifier: Apache-2.0
use super::*;
#[path = "../tests/support/terminal_sources.rs"]
mod terminal_sources;
use nola_store_port::assistant_journal::AssistantJournalRepository;
use nola_store_port::run_message_admission::RunMessageAdmissionRepository;
use nola_domain::remote_submission::*;

fn input() -> PrepareRemoteSubmissionInput {
    PrepareRemoteSubmissionInput {
        source_host_id: "host".into(),
        client_id: "client".into(),
        conversation_id: "conversation".into(),
        conversation_title: "Remote".into(),
        submission_id: "run".into(),
        user_message_id: "user".into(),
        user_part_id: "draft-part".into(),
        user_text: "display-only draft".into(),
        user_content_json: Some("{\"draft\":true}".into()),
        assistant_message_id: "assistant".into(),
        assistant_part_id: "assistant:text".into(),
        context_checkpoint_id: None,
        created_at_ms: 1,
    }
}
fn request() -> serde_json::Value {
    serde_json::json!({"input_items":[{"text":"actual source input"}],"configuration":{"temperature":0.25}})
}

#[test]
fn room_remote_submission_rechecks_membership_but_preserves_bound_recovery() {
    use nola_domain::agent_room::{AgentRoom, RoomParticipant, CONTEXT_OWNER_FIELD};
    let mut store = NolaStore::open(":memory:").unwrap();
    store.create_conversation(&CreateConversationInput {
        id: "conversation".into(), title: "Member".into(), revision: 1,
        settings_json: Some(serde_json::json!({CONTEXT_OWNER_FIELD: {
            "room_id":"room", "participant_id":"member", "agent_id":"agent"
        }}).to_string()), created_at_ms: 1, updated_at_ms: 1,
    }).unwrap();
    let room = AgentRoom::new("room".into(), "Room".into(), vec![RoomParticipant {
        id: "member".into(), agent_id: "agent".to_string().try_into().unwrap(),
        agent_revision: 1, display_name: "Agent".into(), description: "Research".into(),
        context_conversation_id: "conversation".into(), active: true,
    }]).unwrap();
    let inactive = room.deactivate("member").unwrap();
    let put = |store: &mut NolaStore, room: &AgentRoom| {
        store.put_product_setting("agent_chat_room/room", &serde_json::to_string(room).unwrap(), 1).unwrap();
    };
    put(&mut store, &inactive);
    assert!(store.prepare_remote_submission(&input()).is_err());
    assert!(store.remote_submission_intent("conversation").unwrap().is_none());
    put(&mut store, &room);
    store.prepare_remote_submission(&input()).unwrap();
    put(&mut store, &inactive);
    assert!(store.bind_remote_submission_request(&input().identity(), &request()).is_err());
    assert_eq!(store.remote_submission_intent("conversation").unwrap().unwrap().phase, RemoteSubmissionPhase::Prepared);
    put(&mut store, &room);
    let bound = store.bind_remote_submission_request(&input().identity(), &request()).unwrap();
    put(&mut store, &inactive);
    assert_eq!(store.bind_remote_submission_request(&input().identity(), &request()).unwrap(), bound);
    assert_eq!(count(&store, "runs"), 0);
    assert_eq!(count(&store, "messages"), 0);
}
fn turn() -> BeginTurnInput {
    BeginTurnInput {
        user_part_kind: "text".into(),
        expected_configuration_revision: None,
        expected_parent_message_id: None,
        initial_events: Vec::new(),
        conversation_id: "conversation".into(),
        conversation_title: "Remote".into(),
        user_message_id: "user".into(),
        user_part_id: "source-part".into(),
        user_text: "actual source input".into(),
        user_content_json: Some("{\"source\":true}".into()),
        additional_user_parts: vec![],
        assistant_message_id: "assistant".into(),
        assistant_part_id: "assistant:text".into(),
        run_id: "run".into(),
        context_checkpoint_id: None,
        created_at_ms: 100,
        checkpoint_revision: 1,
        checkpoint_event_cursor: 0,
        checkpoint_json: "{}".into(),
        invocation_config_json: "{}".into(),
        invocation_config_digest: "fixture".into(),
        outbox_effects: vec![],
        initial_assistant_part_mutations: vec![],
        assistant_turn_bindings: vec![],
        memory_usage_entries: vec![],
        delegated_invocation_mutations: vec![],
        delegated_permission_mutations: vec![],
        model_attempt_mutations: vec![],
    }
}
fn count(store: &NolaStore, table: &str) -> i64 {
    assert!(matches!(
        table,
        "runs" | "messages" | "source_submission_receipts"
    ));
    store
        .connection
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}
fn accept(
    store: &mut NolaStore,
    receipt: &SourceSubmissionReceipt,
) -> StoreResult<RemoteSubmissionIntentView> {
    store.begin_write_unit_of_work()?;
    match store.accept_remote_submission_receipt(receipt) {
        Ok(value) => {
            store.commit_write_unit_of_work()?;
            Ok(value)
        }
        Err(error) => {
            store.rollback_write_unit_of_work()?;
            Err(error)
        }
    }
}
fn adopt(store: &mut NolaStore, receipt: &SourceSubmissionReceipt) -> StoreResult<()> {
    store.begin_write_unit_of_work()?;
    match store.adopt_observed_remote_run(receipt) {
        Ok(()) => store.commit_write_unit_of_work(),
        Err(error) => {
            store.rollback_write_unit_of_work()?;
            Err(error)
        }
    }
}
fn admitted(source: &mut NolaStore) -> SourceSubmissionReceipt {
    let first = source
        .register_source_submission(&input().identity(), &request())
        .unwrap();
    assert!(first.created);
    source.begin_turn(&turn()).unwrap();
    source
        .source_submission_receipt(&first.receipt.identity, &first.receipt.request_digest)
        .unwrap()
        .unwrap()
}

#[test]
fn intent_is_durable_without_canonical_execution_and_frozen_payload_cannot_change() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("intent.sqlite");
    let mut store = NolaStore::open(&path).unwrap();
    let prepared = store.prepare_remote_submission(&input()).unwrap();
    assert_eq!(prepared.phase, RemoteSubmissionPhase::Prepared);
    assert_eq!(count(&store, "runs"), 0);
    assert_eq!(count(&store, "messages"), 0);
    assert_eq!(prepared, store.prepare_remote_submission(&input()).unwrap());
    let bound = store
        .bind_remote_submission_request(&prepared.identity, &request())
        .unwrap();
    assert_eq!(
        bound,
        store
            .bind_remote_submission_request(&prepared.identity, &request())
            .unwrap()
    );
    assert!(store
        .bind_remote_submission_request(&prepared.identity, &serde_json::json!({"other":1}))
        .is_err());
    assert!(store
        .abandon_prepared_remote_submission(&prepared.identity)
        .is_err());
    drop(store);
    let store = NolaStore::open(&path).unwrap();
    assert_eq!(
        store.remote_submission_intent("conversation").unwrap(),
        Some(bound.clone())
    );
    assert_eq!(
        store
            .remote_submission_request(&bound.identity, bound.request_digest.as_deref().unwrap())
            .unwrap(),
        request()
    );
    assert_eq!(count(&store, "runs"), 0);
    assert_eq!(count(&store, "messages"), 0);
}

#[test]
fn first_receipt_establishes_content_base_atomically_after_reserved_conversation() {
    let dir = tempfile::tempdir().unwrap();
    let mut source = NolaStore::open(dir.path().join("source.sqlite")).unwrap();
    let receipt = admitted(&mut source);
    let path = dir.path().join("receiver.sqlite");
    let mut target = NolaStore::open(&path).unwrap();
    let prepared = target.prepare_remote_submission(&input()).unwrap();
    target
        .bind_remote_submission_request(&prepared.identity, &request())
        .unwrap();
    let base = |store: &NolaStore| -> Option<i64> {
        store.connection.query_row(
            "SELECT revision FROM sync_entity_revisions WHERE entity_type='conversation' AND entity_id='conversation'",
            [], |row| row.get(0),
        ).optional().unwrap()
    };
    assert_eq!(
        base(&target),
        None,
        "a pending intent publishes no transcript"
    );
    target
        .connection
        .execute_batch(
            "CREATE TRIGGER reject_initial_publication BEFORE INSERT ON sync_outbox
         WHEN NEW.entity_type='conversation'
         BEGIN SELECT RAISE(ABORT,'initial publication fault'); END;",
        )
        .unwrap();
    assert!(accept(&mut target, &receipt).is_err());
    assert_eq!(count(&target, "runs"), 0);
    assert_eq!(count(&target, "messages"), 0);
    assert_eq!(base(&target), None);
    assert_eq!(
        target
            .remote_submission_intent("conversation")
            .unwrap()
            .unwrap()
            .phase,
        RemoteSubmissionPhase::Bound
    );
    target
        .connection
        .execute_batch("DROP TRIGGER reject_initial_publication")
        .unwrap();
    let accepted = accept(&mut target, &receipt).unwrap();
    let publication_revision = base(&target).unwrap();
    assert_eq!(
        publication_revision, 2,
        "metadata revision 1 was not a content publication"
    );
    let loaded = target.load_conversation("conversation").unwrap().unwrap();
    assert!(
        loaded.messages[0].parts.is_empty(),
        "pending draft is never source content"
    );
    drop(target);
    let mut target = NolaStore::open(&path).unwrap();
    assert_eq!(accept(&mut target, &receipt).unwrap(), accepted);
    assert_eq!(base(&target), Some(publication_revision));
    assert_eq!(count(&target, "runs"), 1);
    assert_eq!(count(&target, "messages"), 2);
}

#[test]
fn first_local_turn_can_follow_an_abandoned_unpublished_intent() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(dir.path().join("local.sqlite")).unwrap();
    let pending = store.prepare_remote_submission(&input()).unwrap();
    store
        .abandon_prepared_remote_submission(&pending.identity)
        .unwrap();
    let mut local = turn();
    let observed = store.load_conversation_message_window("conversation", None, 1, 1).unwrap().unwrap();
    local.expected_configuration_revision = Some(observed.conversation.configuration_revision);
    local.expected_parent_message_id = observed.conversation.messages.last().map(|message| message.id.clone());
    local.run_id = "local-run".into();
    local.user_message_id = "local-user".into();
    local.assistant_message_id = "local-assistant".into();
    local.assistant_part_id = "local-assistant:text".into();
    store.begin_turn(&local).unwrap();
    let revision: i64 = store.connection.query_row(
        "SELECT revision FROM sync_entity_revisions WHERE entity_type='conversation' AND entity_id='conversation'",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(revision, 2);
    assert_eq!(count(&store, "runs"), 1);
    assert!(
        !conversation_append_sync::needs_initial_turn_publication(
            &store.connection,
            "conversation",
            1
        )
        .unwrap(),
        "never replace an existing content base"
    );
    assert!(
        !conversation_append_sync::needs_initial_turn_publication(
            &store.connection,
            "unpublished-history",
            3
        )
        .unwrap(),
        "missing history provenance is not a first turn"
    );
}

#[test]
fn abandon_is_only_for_unbound_intent_and_permanently_rejects_late_bind() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(dir.path().join("abandon.sqlite")).unwrap();
    let prepared = store.prepare_remote_submission(&input()).unwrap();
    let mut wrong = prepared.identity.clone();
    wrong.client_id = "other".into();
    assert!(store.abandon_prepared_remote_submission(&wrong).is_err());
    let cancelled = store
        .abandon_prepared_remote_submission(&prepared.identity)
        .unwrap();
    assert_eq!(cancelled.phase, RemoteSubmissionPhase::Cancelled);
    assert_eq!(
        cancelled,
        store
            .abandon_prepared_remote_submission(&prepared.identity)
            .unwrap()
    );
    assert!(store
        .bind_remote_submission_request(&prepared.identity, &request())
        .is_err());
    assert!(store
        .remote_submission_intent("conversation")
        .unwrap()
        .is_none());
    assert_eq!(count(&store, "runs"), 0);
}

#[test]
fn source_register_is_single_dispatch_and_cancel_tombstone_blocks_late_admission() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.sqlite");
    let mut source = NolaStore::open(&path).unwrap();
    let id = input().identity();
    let digest = submission_request_digest(&request()).unwrap();
    let cancelled = source.settle_source_submission(&id, &digest, None).unwrap();
    assert_eq!(cancelled.phase, RemoteSubmissionPhase::Cancelled);
    drop(source);
    let mut source = NolaStore::open(&path).unwrap();
    let late = source.register_source_submission(&id, &request()).unwrap();
    assert!(!late.created);
    assert_eq!(late.receipt, cancelled);
    assert!(source.begin_turn(&turn()).is_err());
    assert_eq!(count(&source, "runs"), 0);
    let mut wrong = id.clone();
    wrong.client_id = "other".into();
    assert!(source
        .register_source_submission(&wrong, &request())
        .is_err());
    assert!(source
        .register_source_submission(&id, &serde_json::json!({"different":true}))
        .is_err());
    let mut other = NolaStore::open(dir.path().join("pending.sqlite")).unwrap();
    assert!(
        other
            .register_source_submission(&id, &request())
            .unwrap()
            .created
    );
    assert!(
        !other
            .register_source_submission(&id, &request())
            .unwrap()
            .created
    );
    let mut bad_turn = turn();
    bad_turn.assistant_message_id = "wrong-root".into();
    assert!(other.begin_turn(&bad_turn).is_err());
    assert_eq!(count(&other, "runs"), 0);
    let rejected = other
        .settle_source_submission(&id, &digest, Some("configuration_invalid"))
        .unwrap();
    assert_eq!(rejected.phase, RemoteSubmissionPhase::Rejected);
    assert!(other.begin_turn(&turn()).is_err());
}

#[test]
fn source_admission_and_receipt_are_one_atomic_transaction() {
    let dir = tempfile::tempdir().unwrap();
    let mut source = NolaStore::open(dir.path().join("atomic.sqlite")).unwrap();
    let first = source
        .register_source_submission(&input().identity(), &request())
        .unwrap();
    source.connection.execute_batch("CREATE TRIGGER reject_receipt_update BEFORE UPDATE ON source_submission_receipts BEGIN SELECT RAISE(ABORT,'injected receipt failure'); END;").unwrap();
    assert!(source.begin_turn(&turn()).is_err());
    assert_eq!(count(&source, "runs"), 0);
    assert_eq!(count(&source, "messages"), 0);
    assert_eq!(
        source
            .source_submission_receipt(&first.receipt.identity, &first.receipt.request_digest)
            .unwrap()
            .unwrap()
            .phase,
        RemoteSubmissionPhase::Prepared
    );
    source
        .connection
        .execute_batch("DROP TRIGGER reject_receipt_update;")
        .unwrap();
    source.begin_turn(&turn()).unwrap();
    let receipt = source
        .source_submission_receipt(&first.receipt.identity, &first.receipt.request_digest)
        .unwrap()
        .unwrap();
    assert_eq!(receipt.phase, RemoteSubmissionPhase::Admitted);
    assert_eq!(
        receipt.admission,
        Some(
            source
                .local_run_message_admission_head("conversation", "run")
                .unwrap()
        )
    );
    assert_eq!(
        source
            .settle_source_submission(&receipt.identity, &receipt.request_digest, None)
            .unwrap()
            .phase,
        RemoteSubmissionPhase::Admitted
    );
    assert!(
        !source
            .register_source_submission(&receipt.identity, &request())
            .unwrap()
            .created
    );
    assert!(source
        .observed_source_submission_receipt(
            "host",
            "wrong-client",
            "conversation",
            "run",
            "assistant"
        )
        .is_err());
    assert_eq!(
        source
            .observed_source_submission_receipt(
                "host",
                "client",
                "conversation",
                "run",
                "assistant"
            )
            .unwrap(),
        Some(receipt)
    );
}

#[test]
fn exact_receipt_admits_identity_then_source_declarations_supply_real_user_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let mut source = NolaStore::open(dir.path().join("source.sqlite")).unwrap();
    let receipt = admitted(&mut source);
    let path = dir.path().join("receiver.sqlite");
    let mut target = NolaStore::open(&path).unwrap();
    let prepared = target.prepare_remote_submission(&input()).unwrap();
    assert!(accept(&mut target, &receipt).is_err());
    assert_eq!(count(&target, "runs"), 0);
    target
        .bind_remote_submission_request(&prepared.identity, &request())
        .unwrap();
    let mut wrong_digest = receipt.clone();
    wrong_digest.request_digest = "0".repeat(64);
    assert!(accept(&mut target, &wrong_digest).is_err());
    for field in ["source_host_id", "client_id", "user_message_id"] {
        let mut wrong = serde_json::to_value(&receipt).unwrap();
        wrong["identity"][field] = serde_json::json!("other");
        assert!(accept(&mut target, &serde_json::from_value(wrong).unwrap()).is_err());
    }
    let accepted = accept(&mut target, &receipt).unwrap();
    assert_eq!(target.admitted_remote_run_identity("run").unwrap(), Some(receipt.identity.clone()));
    assert!(target.observed_source_submission_receipt("host", "client", "conversation", "run", "assistant").unwrap().is_none(),
        "a replica must not need a source-owned receipt row");
    assert_eq!(accept(&mut target, &receipt).unwrap(), accepted);
    let mut wrong_epoch = receipt.clone();
    wrong_epoch
        .admission
        .as_mut()
        .unwrap()
        .identity
        .source_store_epoch = "other-source".into();
    assert!(accept(&mut target, &wrong_epoch).is_err());
    assert_eq!(count(&target, "runs"), 1);
    let shell = target.load_conversation("conversation").unwrap().unwrap();
    assert!(
        shell.messages[0].parts.is_empty(),
        "draft content must not become source authority"
    );
    let through = receipt.admission.clone().unwrap();
    let mut after = target
        .run_message_admission_reception_state("host", &through)
        .unwrap();
    while after.sequence.0 < through.sequence.0 {
        let page = source
            .read_run_message_admissions(&after, &through, 1, 262144)
            .unwrap();
        target.begin_write_unit_of_work().unwrap();
        after = target
            .receive_run_message_admissions("host", &page)
            .unwrap();
        target.commit_write_unit_of_work().unwrap();
    }
    drop(target);
    let target = NolaStore::open(&path).unwrap();
    let loaded = target.load_conversation("conversation").unwrap().unwrap();
    assert_eq!(loaded.messages[0].parts[0].id, "source-part");
    assert_eq!(
        loaded.messages[0].parts[0].content_text,
        "actual source input"
    );
    assert_eq!(
        loaded.messages[0].parts[0].content_json,
        Some("{\"source\":true}".into())
    );
    assert_eq!(loaded.messages[0].created_at_ms, 100);
}

#[test]
fn observed_adoption_cannot_bypass_originating_intent_or_change_client_epoch() {
    let dir = tempfile::tempdir().unwrap();
    let mut source = NolaStore::open(dir.path().join("source.sqlite")).unwrap();
    let receipt = admitted(&mut source);
    let mut target = NolaStore::open(dir.path().join("observed.sqlite")).unwrap();
    adopt(&mut target, &receipt).unwrap();
    adopt(&mut target, &receipt).unwrap();
    assert_eq!(target.admitted_remote_run_identity("run").unwrap(), Some(receipt.identity.clone()));
    assert!(target.admitted_remote_run_identity("unknown").unwrap().is_none());
    assert_eq!(count(&target, "runs"), 1);
    let mut wrong = receipt.clone();
    wrong.identity.client_id = "other".into();
    assert!(adopt(&mut target, &wrong).is_err());
    let mut wrong = receipt.clone();
    wrong
        .admission
        .as_mut()
        .unwrap()
        .identity
        .source_store_epoch = "other-epoch".into();
    assert!(adopt(&mut target, &wrong).is_err());
    let mut pending = NolaStore::open(dir.path().join("pending.sqlite")).unwrap();
    pending.prepare_remote_submission(&input()).unwrap();
    assert!(adopt(&mut pending, &receipt).is_err());
    assert!(pending.admitted_remote_run_identity("run").is_err());
    assert_eq!(count(&pending, "runs"), 0);
}

#[test]
fn cold_observation_of_terminal_originating_intent_reuses_exact_receipt_acceptance() {
    let dir = tempfile::tempdir().unwrap();
    let mut source = NolaStore::open(dir.path().join("source.sqlite")).unwrap();
    let receipt = admitted(&mut source);
    let path = dir.path().join("receiver.sqlite");
    let mut target = NolaStore::open(&path).unwrap();
    let prepared = target.prepare_remote_submission(&input()).unwrap();
    target
        .bind_remote_submission_request(&prepared.identity, &request())
        .unwrap();
    // An originating bound intent must use its own digest, even via observation.
    let mut wrong = receipt.clone();
    wrong.request_digest = "f".repeat(64);
    assert!(adopt(&mut target, &wrong).is_err());
    assert_eq!(count(&target, "runs"), 0);
    adopt(&mut target, &receipt).unwrap();
    terminal_sources::settle_fixture(&mut source, "conversation", "run", "run", "user");
    let seal = source.local_assistant_run_terminal_seal("conversation", "run").unwrap().unwrap();
    // A source receipt admits the Run identity, not the complete ordered
    // message declaration frontier required by its eventual terminal seal.
    let mut after = target.run_message_admission_reception_state("host", &seal.admission).unwrap();
    while after.sequence.0 < seal.admission.sequence.0 {
        let page = source.read_run_message_admissions(&after, &seal.admission, 64, 262144).unwrap();
        target.begin_write_unit_of_work().unwrap();
        after = target.receive_run_message_admissions("host", &page).unwrap();
        target.commit_write_unit_of_work().unwrap();
    }
    let window = source.open_assistant_journal_read(
        &nola_domain::assistant_journal::AssistantJournalCursor::initial(seal.identity.clone()).unwrap()
    ).unwrap();
    let nola_store_port::assistant_journal::AssistantJournalPage::Records { records, complete, .. } =
        source.read_assistant_journal_page(&window, 128, 262144).unwrap() else { panic!("expected journal records") };
    assert!(complete);
    let manifest = source.read_local_assistant_run_terminal_manifest(&seal, nola_domain::assistant_journal_operations::JournalNumber(0), 64, 262144).unwrap();
    assert!(manifest.complete);
    target.begin_write_unit_of_work().unwrap();
    for record in records {
        target.receive_assistant_journal_record("host", &record, &[]).unwrap();
    }
    target.begin_receive_assistant_run_terminal("host", &seal).unwrap();
    target.stage_assistant_run_terminal_manifest("host", &manifest).unwrap();
    target.commit_received_assistant_run_terminal("host", &seal).unwrap();
    target.commit_write_unit_of_work().unwrap();
    assert!(target
        .remote_submission_intent("conversation")
        .unwrap()
        .is_none());
    drop(target);
    let mut target = NolaStore::open(&path).unwrap();
    adopt(&mut target, &receipt).unwrap();
    adopt(&mut target, &receipt).unwrap();
    assert_eq!(count(&target, "runs"), 1);
    let status: String = target
        .connection
        .query_row("SELECT status FROM runs WHERE id='run'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(status, "sealed");
    for mutation in 0..3 {
        let mut wrong = receipt.clone();
        match mutation {
            0 => wrong.identity.client_id = "other-client".into(),
            1 => wrong.request_digest = "e".repeat(64),
            _ => {
                wrong
                    .admission
                    .as_mut()
                    .unwrap()
                    .identity
                    .source_store_epoch = "other-epoch".into()
            }
        }
        assert!(adopt(&mut target, &wrong).is_err());
    }
    assert_eq!(count(&target, "runs"), 1);
}

#[test]
fn rejection_or_cancellation_retires_bound_intent_without_fabricating_terminal_run() {
    let dir = tempfile::tempdir().unwrap();
    for (name, reason) in [
        ("rejected", Some("invalid_configuration")),
        ("cancelled", None),
    ] {
        let mut source = NolaStore::open(dir.path().join(format!("{name}-source.sqlite"))).unwrap();
        let mut target =
            NolaStore::open(dir.path().join(format!("{name}-receiver.sqlite"))).unwrap();
        let prepared = target.prepare_remote_submission(&input()).unwrap();
        assert!(target.exact_bound_remote_submission(&prepared.identity, &"a".repeat(64)).is_err());
        let bound = target
            .bind_remote_submission_request(&prepared.identity, &request())
            .unwrap();
        let digest = bound.request_digest.as_deref().unwrap();
        assert_eq!(target.exact_bound_remote_submission(&bound.identity, digest).unwrap().phase, bound.phase);
        let receipt = source
            .settle_source_submission(
                &bound.identity,
                bound.request_digest.as_deref().unwrap(),
                reason,
            )
            .unwrap();
        assert_eq!(accept(&mut target, &receipt).unwrap().phase, receipt.phase);
        assert_eq!(accept(&mut target, &receipt).unwrap().phase, receipt.phase);
        // Room recovery must read the accepted, exact-bound client intent:
        // the source receipt table lives on a different database/Host.
        assert_eq!(target.exact_bound_remote_submission(&bound.identity, digest).unwrap().phase, receipt.phase);
        assert!(target.source_submission_receipt(&bound.identity, digest).unwrap().is_none());
        assert!(target.exact_bound_remote_submission(&bound.identity, &"e".repeat(64)).is_err());
        let mut wrong_owner = bound.identity.clone();
        wrong_owner.client_id = "other-client".into();
        assert!(target.exact_bound_remote_submission(&wrong_owner, digest).is_err());
        assert!(target
            .remote_submission_intent("conversation")
            .unwrap()
            .is_none());
        assert_eq!(count(&target, "runs"), 0);
        assert_eq!(count(&target, "messages"), 0);
        assert!(
            !source
                .register_source_submission(&bound.identity, &request())
                .unwrap()
                .created
        );
    }
}
