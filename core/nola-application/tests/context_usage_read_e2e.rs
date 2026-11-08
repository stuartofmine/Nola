// SPDX-License-Identifier: Apache-2.0
#[path = "support/committed_usage.rs"]
mod committed_usage;
mod support;
use nola_application::{execute_application_command, ApplicationCommand};
use nola_domain::committed_run_context_usage::CommittedRunContextUsage;
use nola_domain::run_message_admission::{RunMessageAdmissionCursor, RunMessageAdmissionPage};
use nola_store::NolaStore;
use serde_json::{json, Value};
use tempfile::tempdir;
fn query(store: &mut NolaStore) -> Result<Value, String> {
    execute_application_command(
        store,
        ApplicationCommand::LoadConversationContextUsageV1 {
            conversation_id: "usage".into(),
        },
    )
}
fn accept(store: &mut NolaStore, record: CommittedRunContextUsage) -> Result<Value, String> {
    execute_application_command(
        store,
        ApplicationCommand::AcceptRemoteRunContextUsage {
            source_host_id: "usage-host".into(),
            record,
        },
    )
}

fn admit_usage_run(source: &mut NolaStore, receiver: &mut NolaStore, receipt: nola_domain::remote_submission::SourceSubmissionReceipt) {
    execute_application_command(receiver, ApplicationCommand::AdoptObservedRemoteRun { receipt: receipt.clone() }).unwrap();
    let through = receipt.admission.unwrap();
    let mut after = RunMessageAdmissionCursor::initial(through.identity.clone()).unwrap();
    loop {
        let page: RunMessageAdmissionPage = serde_json::from_value(execute_application_command(source,
            ApplicationCommand::ReadRunMessageAdmissions { after, through: through.clone(), max_records: 2, max_encoded_bytes: 65536 }).unwrap()).unwrap();
        after = page.next.clone();
        let complete = page.complete;
        execute_application_command(receiver, ApplicationCommand::ReceiveRunMessageAdmissions { source_host_id: "usage-host".into(), page }).unwrap();
        if complete { break; }
    }
}

/// Exercise the same immutable terminal/admission/journal protocol as a real
/// receiver. A usage observation must never itself finish the active Run.
fn receive_usage_run_terminal(source: &mut NolaStore, receiver: &mut NolaStore, run: &str) {
    use nola_application::{AssistantJournalChunk, AssistantJournalReadResult, AssistantJournalSnapshotHandle, StageAssistantJournalContent};
    use nola_domain::assistant_journal::AssistantJournalCursor;
    use nola_domain::assistant_journal_operations::JournalNumber;
    use nola_domain::assistant_run_terminal::{AssistantRunTerminalManifestEntry, AssistantRunTerminalManifestPage, AssistantRunTerminalSeal};
    let seal: AssistantRunTerminalSeal = serde_json::from_value(execute_application_command(source,
        ApplicationCommand::LocalAssistantRunTerminalSeal { conversation_id: "usage".into(), run_id: run.into() }).unwrap()).unwrap();
    execute_application_command(receiver, ApplicationCommand::BeginReceiveAssistantRunTerminal {
        source_host_id: "usage-host".into(), seal: seal.clone(),
    }).unwrap();
    let mut entry = JournalNumber(0);
    loop {
        let page: AssistantRunTerminalManifestPage = serde_json::from_value(execute_application_command(source,
            ApplicationCommand::ReadLocalAssistantRunTerminalManifest { seal: seal.clone(), after_entry: entry, max_records: 2, max_encoded_bytes: 65536 }).unwrap()).unwrap();
        execute_application_command(receiver, ApplicationCommand::StageAssistantRunTerminalManifest {
            source_host_id: "usage-host".into(), page: page.clone(),
        }).unwrap();
        for item in &page.entries {
            let AssistantRunTerminalManifestEntry::Assistant { through, .. } = item else { continue };
            let mut after = AssistantJournalCursor::initial(through.identity.clone()).unwrap();
            loop {
                let records: AssistantJournalReadResult = serde_json::from_value(execute_application_command(source,
                    ApplicationCommand::ReadAssistantJournal { after, through: through.clone(), max_records: 2, max_encoded_bytes: 65536 }).unwrap()).unwrap();
                let AssistantJournalReadResult::Records { records, next, complete, .. } = records else { panic!("fresh fixture journal must be replayable") };
                for record in records {
                    let at = AssistantJournalCursor { identity: record.header.identity.clone(), sequence: record.header.sequence, record_digest: record.header.record_digest.clone() };
                    let snapshot: AssistantJournalSnapshotHandle = serde_json::from_value(execute_application_command(source,
                        ApplicationCommand::OpenAssistantJournalSnapshot { at }).unwrap()).unwrap();
                    loop {
                        let need: Option<nola_store_port::assistant_journal::AssistantJournalContentNeed> = serde_json::from_value(execute_application_command(receiver,
                            ApplicationCommand::NextAssistantJournalContent { source_host_id: "usage-host".into(), record: record.clone() }).unwrap()).unwrap();
                        let Some(need) = need else { break };
                        let chunk: AssistantJournalChunk = serde_json::from_value(execute_application_command(source,
                            ApplicationCommand::ReadAssistantJournalRecordChunk { snapshot: snapshot.clone(), part_id: need.part_id.clone(), content: need.content.clone(), chunk_index: need.expected_prefix.chunk_count }).unwrap()).unwrap();
                        execute_application_command(receiver, ApplicationCommand::StageAssistantJournalContent {
                            input: StageAssistantJournalContent { source_host_id: "usage-host".into(), record: record.clone(), part_id: need.part_id,
                                content: need.content, expected_prefix: need.expected_prefix, chunks: vec![chunk], restart: false },
                        }).unwrap();
                    }
                    execute_application_command(receiver, ApplicationCommand::ReceiveAssistantJournalRecord {
                        source_host_id: "usage-host".into(), record, chunks: vec![],
                    }).unwrap();
                    execute_application_command(source, ApplicationCommand::ReleaseAssistantJournalSnapshot { snapshot }).unwrap();
                }
                after = next;
                if complete { break; }
            }
        }
        entry = page.next_entry;
        if page.complete { break; }
    }
    execute_application_command(receiver, ApplicationCommand::CommitReceivedAssistantRunTerminal {
        source_host_id: "usage-host".into(), seal,
    }).unwrap();
    assert!(receiver.active_run("usage").unwrap().is_none());
}

#[test]
fn cumulative_remote_usage_is_per_run_frontier_guarded_and_replay_safe() {
    let root = tempdir().unwrap();
    let source_path = root.path().join("source.db");
    let receiver_path = root.path().join("receiver.db");
    let mut source = NolaStore::open(&source_path).unwrap();
    let mut receiver = NolaStore::open(&receiver_path).unwrap();
    let (first, receipt) = committed_usage::seed(&mut source, 1000);
    admit_usage_run(&mut source, &mut receiver, receipt);
    accept(&mut receiver, first.clone()).unwrap();
    let sql = rusqlite::Connection::open(&source_path).unwrap();
    // Two further physical attempts commit without a context_updated event.
    // One reports telemetry, one has actually started but reports none.
    for (id, usage) in [("second", json!({"cache_usage": nola_domain::PromptCacheUsage::measured(Some(3000),Some(300),Some(0),false)})),
        ("unreported", Value::Null)] {
        let seq: i64 = sql.query_row("SELECT MAX(sequence)+1 FROM run_events WHERE run_id=?1", [&first.identity.run_id], |r|r.get(0)).unwrap();
        sql.execute("INSERT INTO run_events(event_id,run_id,sequence,event_kind,payload_json,created_at_ms) VALUES(?1,?2,?3,'model_attempt_changed',?4,2000)",
            rusqlite::params![format!("extra:{id}"),first.identity.run_id,seq,json!({"id":id,"state":"succeeded","started_at_ms":1000,"usage":usage}).to_string()]).unwrap();
    }
    let next = source.local_run_context_usage(&first.identity).unwrap().unwrap();
    assert_eq!(next.event_sequence, first.event_sequence);
    assert!(next.attempt_sequence > first.attempt_sequence);
    assert_eq!(next.run_cache_usage.unwrap().reported_requests, 2);
    assert_eq!(next.run_cache_usage.unwrap().unreported_requests, 1);
    for _ in 0..2 { accept(&mut receiver, next.clone()).unwrap(); }
    assert_eq!(serde_json::from_value::<CommittedRunContextUsage>(accept(&mut receiver, first.clone()).unwrap()).unwrap(), next);
    let mut fork = next.clone();
    fork.run_cache_usage.as_mut().unwrap().unreported_requests += 1;
    fork.record_digest = fork.computed_digest().unwrap();
    assert!(accept(&mut receiver, fork).is_err());
    let mut crossed = next.clone();
    crossed.event_sequence.0 += 1;
    crossed.attempt_sequence = first.attempt_sequence;
    crossed.record_digest = crossed.computed_digest().unwrap();
    assert!(accept(&mut receiver, crossed).is_err());
    assert_eq!(query(&mut receiver).unwrap(), query(&mut source).unwrap());

    assert!(receiver.active_run("usage").unwrap().is_some(), "usage alone is not terminal authority");
    receive_usage_run_terminal(&mut source, &mut receiver, &first.identity.run_id);
    let (second_run, receipt) = committed_usage::seed_another_run(&mut source, 2000);
    assert_ne!(first.identity.run_id, second_run.identity.run_id);
    admit_usage_run(&mut source, &mut receiver, receipt);
    accept(&mut receiver, second_run.clone()).unwrap();
    accept(&mut receiver, second_run).unwrap();
    // A local projection copy is not a second physical execution ledger.
    let replica_sql = rusqlite::Connection::open(&receiver_path).unwrap();
    replica_sql.execute("INSERT INTO run_events(event_id,run_id,sequence,event_kind,payload_json,created_at_ms) VALUES('copy',?1,(SELECT COALESCE(MAX(sequence),0)+1 FROM run_events WHERE run_id=?1),'model_attempt_changed',?2,2000)",
        rusqlite::params![first.identity.run_id,json!({"id":"copy","state":"succeeded","usage":{"cache_usage":nola_domain::PromptCacheUsage::measured(Some(9999),Some(9999),Some(0),false)}}).to_string()]).unwrap();
    drop(receiver);
    let mut receiver = NolaStore::open(&receiver_path).unwrap();
    assert_eq!(query(&mut receiver).unwrap(), query(&mut source).unwrap());
    let totals = query(&mut receiver).unwrap()["conversation_cache_usage"].clone();
    assert_eq!(totals["input_tokens"], 6000);
    assert_eq!(totals["read_tokens"], 1800);
    assert_eq!(totals["hit_rate_bps"], 3000);
    assert_eq!(totals["reported_requests"], 3);
    assert_eq!(totals["unreported_requests"], 1);
}
#[test]
fn cold_usage_reads_committed_event_without_reading_or_rewriting_large_checkpoint() {
    let root = tempdir().unwrap();
    let path = root.path().join("nola.db");
    let mut store = NolaStore::open(&path).unwrap();
    let (record, _) = committed_usage::seed(&mut store, 5432);
    assert_eq!(record.usage.used_tokens.0, 5432);
    assert!(!record.usage.estimated);
    // Corrupt execution state must not poison an independently committed control fact.
    let sql = rusqlite::Connection::open(&path).unwrap();
    let large =
        json!({"unrelated_history":"x".repeat(10_000_000),"session_id":"foreign"}).to_string();
    sql.execute(
        "UPDATE core_checkpoint_bodies SET checkpoint_json=?1 WHERE conversation_id='usage'",
        [&large],
    )
    .unwrap();
    drop(store);
    let mut store = NolaStore::open(&path).unwrap();
    let usage = query(&mut store).unwrap();
    assert_eq!(usage["used_tokens"], 5432);
    assert_eq!(usage["context_window_tokens"], 65536);
    assert_eq!(usage["prompt_token_limit"], 57344);
    assert_eq!(usage["estimated"], false);
    assert_eq!(usage["cache_usage"]["hit_rate_bps"], 5000);
    assert_eq!(usage["cache_usage"]["input_tokens"], 5432);
    assert_eq!(usage["cache_usage"]["read_tokens"], 2716);
    assert!(usage.to_string().len() < 1024);
    assert_eq!(
        store
            .load_checkpoint("usage")
            .unwrap()
            .unwrap()
            .checkpoint_json,
        large
    );
    let plan:String=sql.query_row("EXPLAIN QUERY PLAN SELECT sequence,payload_json FROM run_events WHERE run_id='usage-run' AND event_kind='context_updated' ORDER BY sequence DESC LIMIT 1",[],|r|r.get(3)).unwrap();
    assert!(plan.contains("run_context_usage_by_sequence"), "{plan}");
}
#[test]
fn usage_wire_requires_decimal_integers_and_exact_identity() {
    let root = tempdir().unwrap();
    let mut store = NolaStore::open(root.path().join("nola.db")).unwrap();
    let (record, _) = committed_usage::seed(&mut store, 5432);
    for bad in [
        json!(true),
        json!(false),
        json!(5432),
        json!("-1"),
        json!("5.5"),
        Value::Null,
    ] {
        let mut value = serde_json::to_value(&record).unwrap();
        value["usage"]["used_tokens"] = bad;
        assert!(serde_json::from_value::<CommittedRunContextUsage>(value).is_err());
    }
    let mut foreign = record.identity.clone();
    foreign.source_store_epoch.push_str("-foreign");
    assert!(store.local_run_context_usage(&foreign).is_err());
    let mut foreign = record.identity.clone();
    foreign.assistant_message_id = "foreign".into();
    assert!(store.local_run_context_usage(&foreign).is_err());
    let mut invalid = record.clone();
    invalid.usage.used_tokens.0 += 1;
    assert!(invalid.validate().is_err());
    let mut obsolete = record.clone();
    obsolete.protocol_version = 1;
    obsolete.record_digest = obsolete.computed_digest().unwrap();
    assert!(obsolete.validate().is_err());
    let mut missing_frontier = serde_json::to_value(record).unwrap();
    missing_frontier.as_object_mut().unwrap().remove("attempt_sequence");
    assert!(serde_json::from_value::<CommittedRunContextUsage>(missing_frontier).is_err());
}
#[test]
fn usage_receiver_requires_real_admission_and_survives_cold_reopen() {
    let root = tempdir().unwrap();
    let mut source = NolaStore::open(root.path().join("source.db")).unwrap();
    let (record, receipt) = committed_usage::seed(&mut source, 5432);
    let path = root.path().join("receiver.db");
    let mut receiver = NolaStore::open(&path).unwrap();
    assert!(accept(&mut receiver, record.clone()).is_err());
    execute_application_command(
        &mut receiver,
        ApplicationCommand::AdoptObservedRemoteRun {
            receipt: receipt.clone(),
        },
    )
    .unwrap();
    assert!(
        accept(&mut receiver, record.clone()).is_err(),
        "shell alone is not received admission"
    );
    let through = receipt.admission.unwrap();
    let mut after = RunMessageAdmissionCursor::initial(through.identity.clone()).unwrap();
    loop {
        let page: RunMessageAdmissionPage = serde_json::from_value(
            execute_application_command(
                &mut source,
                ApplicationCommand::ReadRunMessageAdmissions {
                    after,
                    through: through.clone(),
                    max_records: 2,
                    max_encoded_bytes: 65536,
                },
            )
            .unwrap(),
        )
        .unwrap();
        after = page.next.clone();
        let complete = page.complete;
        execute_application_command(
            &mut receiver,
            ApplicationCommand::ReceiveRunMessageAdmissions {
                source_host_id: "usage-host".into(),
                page,
            },
        )
        .unwrap();
        if complete {
            break;
        }
    }
    accept(&mut receiver, record.clone()).unwrap();
    assert_eq!(query(&mut receiver).unwrap()["cache_usage"]["hit_rate_bps"], 5000);
    let mut fork = record.clone();
    fork.usage.used_tokens.0 += 1;
    fork.record_digest = fork.computed_digest().unwrap();
    assert!(accept(&mut receiver, fork).is_err());
    let mut stale = record.clone();
    stale.event_sequence.0 -= 1;
    stale.record_digest = stale.computed_digest().unwrap();
    assert_eq!(
        serde_json::from_value::<CommittedRunContextUsage>(accept(&mut receiver, stale).unwrap())
            .unwrap(),
        record
    );
    assert!(execute_application_command(
        &mut receiver,
        ApplicationCommand::AcceptRemoteRunContextUsage {
            source_host_id: "foreign-host".into(),
            record: record.clone()
        }
    )
    .is_err());
    let mut epoch = record.clone();
    epoch.identity.source_store_epoch.push_str("-foreign");
    epoch.admission.identity = epoch.identity.clone();
    epoch.record_digest = epoch.computed_digest().unwrap();
    assert!(accept(&mut receiver, epoch).is_err());
    drop(receiver);
    let mut receiver = NolaStore::open(&path).unwrap();
    assert_eq!(query(&mut receiver).unwrap(), query(&mut source).unwrap());
    let full = nola_application::load_conversation_projection_v1(&receiver, "usage").unwrap();
    assert_eq!(
        serde_json::to_value(full.context_usage).unwrap(),
        query(&mut receiver).unwrap()
    );
    let window = nola_application::load_conversation_projection_window_v1(
        &receiver, "usage", None, 2, 32768,
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(window.projection.context_usage).unwrap(),
        query(&mut receiver).unwrap()
    );
    let sql = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        sql.query_row("SELECT count(*) FROM remote_run_context_usage", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        1
    );
}
#[test]
fn absent_committed_usage_is_unknown_not_summary_tokens() {
    let root = tempdir().unwrap();
    let mut store = NolaStore::open(root.path().join("nola.db")).unwrap();
    let usage = query(&mut store).unwrap();
    assert_eq!(usage["used_tokens"], 0);
    assert_eq!(usage["estimated"], true);
}

#[test]
fn missing_current_run_fact_does_not_reuse_previous_run_or_checkpoint_usage() {
    let root = tempdir().unwrap();
    let path = root.path().join("nola.db");
    let mut store = NolaStore::open(&path).unwrap();
    committed_usage::seed(&mut store, 5432);
    let sql = rusqlite::Connection::open(&path).unwrap();
    // A distinct canonical Run with no committed usage must not borrow its
    // predecessor's measured tokens. This fixture adds only metadata, no event.
    sql.execute("INSERT INTO runs(id,conversation_id,kind,status,assistant_message_id,next_event_sequence,created_at_ms,updated_at_ms) VALUES('new-run','usage','conversation','running',NULL,1,2000,2000)",[]).unwrap();
    let usage = query(&mut store).unwrap();
    assert_eq!(usage["used_tokens"], 0);
    assert_eq!(usage["estimated"], true);
}
#[test]
fn source_estimate_remains_explicit_until_provider_usage_is_committed() {
    let root = tempdir().unwrap();
    let database = root.path().join("nola.db");
    let mut store = NolaStore::open(&database).unwrap();
    let (record, _) = committed_usage::seed_with_usage(&mut store, None);
    assert!(record.usage.estimated);
    assert_eq!(query(&mut store).unwrap()["estimated"], true);
    drop(store);
    let mut reopened = NolaStore::open(&database).unwrap();
    assert_eq!(reopened.local_run_context_usage(&record.identity).unwrap(), Some(record.clone()));
    assert_eq!(query(&mut reopened).unwrap()["used_tokens"], record.usage.used_tokens.0);
    let sql = rusqlite::Connection::open(&database).unwrap();
    let (event_id, payload): (String,String) = sql.query_row(
        "SELECT event_id,payload_json FROM run_events WHERE event_kind='turn_started' ORDER BY sequence LIMIT 1",
        [], |row| Ok((row.get(0)?,row.get(1)?)),
    ).unwrap();
    let payload: Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(payload["session_id"], payload["core_envelope"]["session_id"]);
    assert_eq!(payload["turn_id"], payload["core_envelope"]["turn_id"]);
    assert_eq!(event_id, format!("core:{}:{}", record.identity.run_id, payload["core_envelope"]["cursor"]["sequence"].as_u64().unwrap()));
}

#[test]
fn initial_context_fact_failure_rolls_back_the_entire_admission() {
    let root = tempdir().unwrap();
    let database = root.path().join("initial-fact-rollback.db");
    let mut store = NolaStore::open(&database).unwrap();
    let sql = rusqlite::Connection::open(&database).unwrap();
    sql.execute_batch("CREATE TRIGGER reject_initial_context BEFORE INSERT ON run_events WHEN NEW.event_kind='context_updated' BEGIN SELECT RAISE(ABORT,'initial fact fault'); END").unwrap();
    // The shared real-model-delivery fixture unwraps begin's Result; catching
    // that expected fixture panic lets us inspect the same failed transaction.
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        committed_usage::seed_with_usage(&mut store, None)
    }));
    assert!(failed.is_err());
    drop(store);
    let reopened = NolaStore::open(&database).unwrap();
    assert!(reopened.active_run("usage").unwrap().is_none());
    for table in ["runs", "run_events", "messages", "effect_outbox", "assistant_journal_heads", "run_invocation_payloads"] {
        let count: i64 = sql.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| row.get(0)).unwrap();
        assert_eq!(count, 0, "initial fact failure leaked {table}");
    }
}

#[test]
fn imported_history_without_usage_never_requires_phantom_admission() {
    use nola_application::CreateConversation;
    use nola_protocol::{
        ConversationTranscriptImportSource, ImportConversationTranscript, TranscriptMessageInput,
        TranscriptPartInput,
    };
    for persisted_run in [false, true] {
        let root = tempdir().unwrap();
        let path = root.path().join("history.db");
        let mut store = NolaStore::open(&path).unwrap();
        execute_application_command(
            &mut store,
            ApplicationCommand::CreateConversation {
                input: CreateConversation {
                    id: "usage".into(),
                    title: "Imported".into(),
                    settings_json: None,
                    created_at_ms: 1,
                    updated_at_ms: 1,
                },
            },
        )
        .unwrap();
        let sql = rusqlite::Connection::open(&path).unwrap();
        if persisted_run {
            // Fault injection: claimed sealed metadata has no source admission
            // or result seal and must not authorize the usage read below.
            sql.execute("INSERT INTO runs(id,conversation_id,kind,status,assistant_message_id,next_event_sequence,created_at_ms,updated_at_ms,completed_at_ms) VALUES('archived-run','usage','conversation','sealed',NULL,1,1,2,2)",[]).unwrap();
        }
        execute_application_command(
            &mut store,
            ApplicationCommand::ImportConversationTranscript {
                input: ImportConversationTranscript {
                    import_id: "archive".into(),
                    source: ConversationTranscriptImportSource::ExternalArchive,
                    conversation_id: "usage".into(),
                    expected_message_count: 0,
                    imported_at_ms: 3,
                    messages: vec![TranscriptMessageInput {
                        id: "archived-root".into(),
                        run_id: Some("archived-run".into()),
                        role: "assistant".into(),
                        status: "completed".into(),
                        ordinal: 1,
                        created_at_ms: 1,
                        completed_at_ms: Some(2),
                        parts: vec![TranscriptPartInput {
                            id: "archived-text".into(),
                            kind: "text".into(),
                            ordinal: 1,
                            state: "completed".into(),
                            content_text: "Historical answer".into(),
                            content_json: None,
                            content_digest: None,
                            created_at_ms: 1,
                            updated_at_ms: 2,
                        }],
                    }],
                },
            },
        )
        .unwrap();
        if persisted_run {
            sql.execute("UPDATE runs SET assistant_message_id='archived-root' WHERE id='archived-run'",[]).unwrap();
        }
        // Missing facts are unknown, not a synthesized local journal owner.
        assert!(store
            .local_run_context_usage(&nola_domain::assistant_journal::AssistantJournalIdentity {
                source_store_epoch: "archive-is-not-a-source-epoch".into(),
                conversation_id: "usage".into(),
                run_id: "archived-run".into(),
                assistant_message_id: "archived-root".into(),
            })
            .unwrap()
            .is_none());
        let full = nola_application::load_conversation_projection_v1(&store, "usage").unwrap();
        let page =
            nola_application::load_conversation_projection_page_v1(&store, "usage", 0, 10).unwrap();
        let window = nola_application::load_conversation_projection_window_v1(
            &store, "usage", None, 10, 32768,
        )
        .unwrap();
        for projection in [full, page, window.projection] {
            assert_eq!(projection.messages.len(), 1);
            assert_eq!(projection.messages[0].message_id.0, "archived-root");
            assert_eq!(projection.context_usage.used_tokens, 0);
            assert!(projection.context_usage.estimated);
        }
        assert!(window.assistant_journal_heads.is_empty());
        assert_eq!(
            sql.query_row(
                "SELECT count(*) FROM run_message_admission_records",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            sql.query_row(
                "SELECT count(*) FROM run_events WHERE event_kind='context_updated'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    }
}
#[test]
fn receipted_usage_query_replays_without_mutating_conversation_facts() {
    let root = tempdir().unwrap();
    let database = root.path().join("nola.db");
    let mut store = NolaStore::open(&database).unwrap();
    committed_usage::seed(&mut store, 5432);
    let before = store.load_checkpoint("usage").unwrap();
    let scope = nola_store::ProjectionScopeKey {
        scope_type: "conversation".into(),
        scope_id: "usage".into(),
    };
    let before_cursor = store.projection_cursor(&scope).unwrap();
    drop(store);

    let command_json = serde_json::to_string(&ApplicationCommand::LoadConversationContextUsageV1 {
        conversation_id: "usage".into(),
    })
    .unwrap();
    let receipt = || nola_application::ExecuteReceivedCommand {
        client_id: "usage-client".into(),
        request_id: "usage-request".into(),
        command_type: "load_conversation_context_usage_v1".into(),
        command_json: command_json.clone(),
        expected_revision: None,
        received_at: "2026-09-05T00:00:00Z".into(),
    };
    let mut runtime = support::open_runtime(&database).unwrap();
    let first = runtime
        .execute_received_command(receipt(), serde_json::from_str(&command_json).unwrap())
        .unwrap();
    assert!(!first.replayed);
    assert!(matches!(
        first.outcome,
        nola_application::ApplicationCommandReceiptOutcome::Committed { .. }
    ));
    drop(runtime);
    let mut runtime = support::open_runtime(&database).unwrap();
    let replay = runtime
        .execute_received_command(receipt(), serde_json::from_str(&command_json).unwrap())
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(first.outcome, replay.outcome);
    drop(runtime);
    let store = NolaStore::open(&database).unwrap();
    assert_eq!(store.load_checkpoint("usage").unwrap(), before);
    assert_eq!(store.projection_cursor(&scope).unwrap(), before_cursor);
}
