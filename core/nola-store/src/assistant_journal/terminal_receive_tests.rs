// SPDX-License-Identifier: Apache-2.0
use super::*;
use nola_domain::assistant_run_terminal::*;
use nola_domain::run_message_admission::*;
use nola_store_port::run_message_admission::RunMessageAdmissionRepository;

fn admissions(root: &AssistantJournalIdentity) -> RunMessageAdmissionPage {
    let after = RunMessageAdmissionCursor::initial(root.clone()).unwrap();
    let mut next = after.clone();
    let mut records = vec![];
    for (message, ordinal) in [(root.assistant_message_id.as_str(), 1000), ("a2", 1010)] {
        let value = nola_domain::ReplicaMessageDeclaration {
            execution_ownership: nola_domain::ConversationExecutionOwnership::User,
            ordinal,
            input: nola_domain::TransitionMessageInput {
                id: message.into(),
                run_id: Some(root.run_id.clone()),
                role: "assistant".into(),
                status: "streaming".into(),
                created_at_ms: 1,
                completed_at_ms: None,
                initial_parts: vec![],
            },
        };
        let text = serde_json::to_string(&value).unwrap();
        let mut record = RunMessageAdmissionRecord {
            cursor: RunMessageAdmissionCursor {
                identity: root.clone(),
                sequence: JournalNumber(next.sequence.0 + 1),
                record_digest: next.record_digest,
            },
            previous_digest: next.record_digest,
            message_id: message.into(),
            offset: JournalNumber(0),
            total_utf8_bytes: JournalNumber(text.len() as u64),
            declaration_digest: admission_digest(text.as_bytes()),
            text,
        };
        record.cursor.record_digest = record.computed_digest().unwrap();
        next = record.cursor.clone();
        records.push(record);
    }
    RunMessageAdmissionPage {
        after,
        through: next.clone(),
        next,
        complete: true,
        records,
    }
}

fn record(
    id: &AssistantJournalIdentity,
    part: &str,
    position: u64,
    previous: AssistantJournalCursor,
) -> StoredAssistantJournalRecord {
    let mut batch = declaration(id);
    let AssistantJournalOperationV1::Declare(declare) = &mut batch.operations[0] else {
        panic!()
    };
    declare.part.part_id = part.into();
    declare.part.order_key.position = JournalNumber(position);
    declare.part.bodies =
        vec![
            empty_journal_content(id, part, JournalContentField::Markdown, JournalNumber(1))
                .unwrap(),
        ];
    let header = batch
        .seal(
            previous.sequence + 1,
            previous.record_digest,
            format!("declare-{part}"),
            previous.sequence + 1,
        )
        .unwrap();
    StoredAssistantJournalRecord { header, batch }
}
fn cursor(record: &StoredAssistantJournalRecord) -> AssistantJournalCursor {
    AssistantJournalCursor {
        identity: record.header.identity.clone(),
        sequence: record.header.sequence,
        record_digest: record.header.record_digest,
    }
}

#[test]
fn multi_owner_terminal_requires_durable_historical_frontiers_and_survives_reopen() {
    exercise_multi_owner_terminal(false);
}

#[test]
fn admitted_empty_segment_terminal_requires_same_run_admission_and_survives_reopen() {
    exercise_multi_owner_terminal(true);
}

fn exercise_multi_owner_terminal(empty_segment: bool) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("replica.sqlite");
    let mut store = NolaStore::open_archive_mode(&path, false).unwrap();
    store
        .admit_remote_submission_replica(&nola_store_port::remote_session::AdmittedReplicaTurnInput {
            user_part_kind: "text".into(),
            source_host_id: "host".into(),
            conversation_id: "conversation".into(),
            conversation_title: "Replica".into(),
            user_message_id: "u".into(),
            user_part_id: "up".into(),
            user_text: "Work".into(),
            user_content_json: None,
            assistant_message_id: "a".into(),
            assistant_part_id: "placeholder".into(),
            run_id: "run".into(),
            context_checkpoint_id: None,
            created_at_ms: 1,
        })
        .unwrap();
    let root = AssistantJournalIdentity {
        source_store_epoch: "source-epoch".into(),
        conversation_id: "conversation".into(),
        run_id: "run".into(),
        assistant_message_id: "a".into(),
    };
    let mut second = root.clone();
    second.assistant_message_id = "a2".into();
    let first = record(
        &root,
        "p1",
        0,
        AssistantJournalCursor::initial(root.clone()).unwrap(),
    );
    let extra = record(
        &second,
        "p2",
        0,
        AssistantJournalCursor::initial(second.clone()).unwrap(),
    );
    let ordinal: i64 = store
        .connection
        .query_row("SELECT ordinal FROM messages WHERE id='a'", [], |r| {
            r.get(0)
        })
        .unwrap();
    store.connection.execute("INSERT INTO messages(id,conversation_id,run_id,role,status,ordinal,created_at_ms) VALUES('local-note','conversation',NULL,'user','completed',?1,1)",[ordinal+1]).unwrap();
    let existing_order: Vec<(String, i64)> = store
        .connection
        .prepare("SELECT id,ordinal FROM messages ORDER BY ordinal")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let entries = vec![
        AssistantRunTerminalManifestEntry::Assistant {
            through: cursor(&first),
            ordinal: 1000,
            created_at_ms: 1,
            state: AssistantRunTerminalStatus::Completed,
            completed_at_ms: 4,
        },
        AssistantRunTerminalManifestEntry::Assistant {
            through: if empty_segment { AssistantJournalCursor::initial(second.clone()).unwrap() } else { cursor(&extra) },
            ordinal: 1010,
            created_at_ms: 2,
            state: AssistantRunTerminalStatus::Failed,
            completed_at_ms: 10,
        },
    ];
    let digest = terminal_manifest_digest(&entries).unwrap();
    for field in ["state", "completed_at_ms"] {
        let mut old = serde_json::to_value(&entries[0]).unwrap();
        old.as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<AssistantRunTerminalManifestEntry>(old).is_err());
    }
    let mut changed = entries.clone();
    if let AssistantRunTerminalManifestEntry::Assistant { state, .. } = &mut changed[0] {
        *state = AssistantRunTerminalStatus::Failed;
    }
    assert_ne!(digest, terminal_manifest_digest(&changed).unwrap());
    let mut changed = entries.clone();
    if let AssistantRunTerminalManifestEntry::Assistant {
        completed_at_ms, ..
    } = &mut changed[0]
    {
        *completed_at_ms += 1;
    }
    assert_ne!(digest, terminal_manifest_digest(&changed).unwrap());
    let result = nola_domain::run_result::RunResult {
        result_id: "failure-result".into(), run_id: root.run_id.clone(),
        conversation_id: root.conversation_id.clone(), originating_submission_id: root.run_id.clone(),
        input: nola_domain::run_input_commitment::RunInputCommitment {
            watermark: JournalNumber(0),
            applications_digest:nola_domain::run_input_commitment::RunInputCommitment::empty_applications_digest(),
        },
        decided_at_revision:JournalNumber(4),decision_event_cursor:JournalNumber(7),
        requirement_revision:JournalNumber(1),evidence_frontier:JournalNumber(0),
        value:nola_domain::run_result::RunResultValue::Failed {
            error_ref:"source-error".into(),observed_effect_refs:vec![],unknown_effect_refs:vec![],
        },
    };
    let mut seal = AssistantRunTerminalSeal {
        protocol_version: 1,
        identity: root.clone(),
        core_session_id: "core-not-conversation".into(),
        transition_id: "terminal".into(),
        checkpoint_revision: JournalNumber(4),
        core_revision: JournalNumber(4),
        event_cursor: JournalNumber(7),
        result_digest:result.computed_digest().unwrap(),
        result,
        completed_at_ms: 10,
        admission: admissions(&root).through,
        entry_count: JournalNumber(2),
        heads_count: JournalNumber(2),
        child_receipt_count: JournalNumber(0),
        manifest_digest: digest,
        seal_digest: digest,
    };
    seal.seal_digest = seal.computed_digest().unwrap();
    let page = |start: usize, end: usize| AssistantRunTerminalManifestPage {
        seal: seal.clone(),
        after_entry: JournalNumber(start as u64),
        next_entry: JournalNumber(end as u64),
        complete: end == 2,
        entries: entries[start..end].to_vec(),
    };
    store.begin_write_unit_of_work().unwrap();
    assert!(store
        .begin_receive_assistant_run_terminal("wrong-host", &seal)
        .is_err());
    store
        .begin_receive_assistant_run_terminal("host", &seal)
        .unwrap();
    assert!(store
        .stage_assistant_run_terminal_manifest("host", &page(1, 2))
        .is_err());
    store
        .stage_assistant_run_terminal_manifest("host", &page(0, 1))
        .unwrap();
    assert!(
        store
            .receive_assistant_journal_record("host", &extra, &[])
            .is_err(),
        "partial manifest cannot admit owners"
    );
    store.commit_write_unit_of_work().unwrap();
    drop(store);
    let mut store = NolaStore::open_archive_mode(&path, false).unwrap();
    store.begin_write_unit_of_work().unwrap();
    assert_eq!(
        store
            .begin_receive_assistant_run_terminal("host", &seal)
            .unwrap()
            .next_entry
            .0,
        1
    );
    let mut bad = page(1, 2);
    if let AssistantRunTerminalManifestEntry::Assistant { ordinal, .. } = &mut bad.entries[0] {
        *ordinal += 1;
    }
    assert!(store
        .stage_assistant_run_terminal_manifest("host", &bad)
        .is_err());
    assert_eq!(
        store
            .begin_receive_assistant_run_terminal("host", &seal)
            .unwrap()
            .next_entry
            .0,
        1,
        "failed final hash rolls back page and owners"
    );
    // A valid manifest cannot take over an existing row owned by another Run.
    store.connection.execute("INSERT INTO messages(id,conversation_id,run_id,role,status,ordinal,created_at_ms) VALUES('a2','conversation','other-run','assistant','running',?1,1)",[ordinal+2]).unwrap();
    assert!(store
        .receive_run_message_admissions("host", &admissions(&root))
        .is_err());
    assert!(store
        .stage_assistant_run_terminal_manifest("host", &page(1, 2))
        .is_err());
    store
        .connection
        .execute(
            "DELETE FROM messages WHERE id='a2' AND run_id='other-run'",
            [],
        )
        .unwrap();
    store
        .receive_run_message_admissions("host", &admissions(&root))
        .unwrap();
    assert!(
        store
            .stage_assistant_run_terminal_manifest("host", &page(1, 2))
            .unwrap()
            .manifest_complete
    );
    for (id, order) in existing_order {
        assert_eq!(
            store
                .connection
                .query_row("SELECT ordinal FROM messages WHERE id=?1", [id], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            order
        );
    }
    assert_eq!(
        store
            .connection
            .query_row("SELECT ordinal FROM messages WHERE id='a2'", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        ordinal + 2
    );
    assert!(store
        .commit_received_assistant_run_terminal("host", &seal)
        .is_err());
    store
        .receive_assistant_journal_record("host", &first, &[])
        .unwrap();
    if !empty_segment {
        assert!(store.commit_received_assistant_run_terminal("host", &seal).is_err(),
            "root alone cannot finish a nonempty second owner");
        store.receive_assistant_journal_record("host", &extra, &[]).unwrap();
    }
    let future = record(&root, "background", 1, cursor(&first));
    store
        .receive_assistant_journal_record("host", &future, &[])
        .unwrap();
    store.connection.execute_batch("CREATE TEMP TRIGGER fail_terminal BEFORE UPDATE OF status ON runs WHEN NEW.status='sealed' BEGIN SELECT RAISE(ABORT,'terminal fail'); END;").unwrap();
    assert!(store
        .commit_received_assistant_run_terminal("host", &seal)
        .is_err());
    assert!(
        !store
            .begin_receive_assistant_run_terminal("host", &seal)
            .unwrap()
            .committed
    );
    store
        .connection
        .execute_batch("DROP TRIGGER fail_terminal")
        .unwrap();
    assert!(
        store
            .commit_received_assistant_run_terminal("host", &seal)
            .unwrap()
            .committed,
        "newer durable head still proves historical sealed frontier"
    );
    store.commit_write_unit_of_work().unwrap();
    drop(store);
    let mut store = NolaStore::open_archive_mode(&path, false).unwrap();
    store.begin_write_unit_of_work().unwrap();
    assert!(
        store
            .commit_received_assistant_run_terminal("host", &seal)
            .unwrap()
            .committed
    );
    assert_eq!(
        store
            .assistant_journal_reception_state("host", &cursor(&first))
            .unwrap()
            .after,
        cursor(&future)
    );
    let later = record(&second, "later-card", 1,
        if empty_segment { AssistantJournalCursor::initial(second.clone()).unwrap() } else { cursor(&extra) });
    store
        .receive_assistant_journal_record("host", &later, &[])
        .unwrap();
    assert!(
        store
            .commit_received_assistant_run_terminal("host", &seal)
            .unwrap()
            .committed
    );
    store.commit_write_unit_of_work().unwrap();
    assert!(store.active_run("conversation").unwrap().is_none());
    let terminal = store.canonical_run_terminal("conversation", &seal.identity.run_id)
        .unwrap().expect("committed remote seal remains readable after cold reopen and replay");
    assert_eq!(terminal.result, seal.result);
    assert_eq!(terminal.result_digest, seal.result_digest);
    assert_eq!(terminal.assistant_message_id, seal.identity.assistant_message_id);
    let states: Vec<(String, String, i64)> = store.connection
        .prepare("SELECT id,status,completed_at_ms FROM messages WHERE role='assistant' ORDER BY ordinal")
        .unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap().collect::<Result<_, _>>().unwrap();
    assert_eq!(
        states,
        vec![
            ("a".into(), "completed".into(), 4),
            ("a2".into(), "failed".into(), 10)
        ]
    );
    let run_status: String = store
        .connection
        .query_row("SELECT status FROM runs WHERE id=?1", [&root.run_id], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(run_status, "sealed");
}
