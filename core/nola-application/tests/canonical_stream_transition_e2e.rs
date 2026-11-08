// SPDX-License-Identifier: Apache-2.0
//! Real Application reduction/batching/receipt paths; no provider or network mock.
mod support;
#[path = "../../nola-store/tests/support/remote_submission_from.rs"]
mod remote_submission_fixture;
use remote_submission_fixture::RemoteSubmissionFixture;

use nola_application::{
    begin_assistant_turn, load_committed_host_requests, AcceptedHostResponse, ApplicationCommand,
    BeginAssistantTurn, ExecuteReceivedCommand, ReduceAndAcceptHostResponse,
};
use nola_agent_engine::agent::host::*;
use nola_agent_engine::agent::protocol::*;
use nola_domain::{CanonicalStreamHead, CanonicalStreamOwner, CanonicalTextState, PartBodyField};
use nola_store::NolaStore;
use nola_store_port::run_effect::RunEffectRepository;
use nola_store_port::assistant_journal::AssistantJournalRepository;
use nola_store_port::protocol_journal::ProjectionScopeKey;
use std::collections::BTreeMap;

fn config() -> AgentCoreConfig {
    let mut capabilities = BTreeMap::new();
    capabilities.insert(
        "model.stream".into(),
        HostCapabilityDescriptor {
            id: "model.stream".into(),
            title: "model.stream".into(),
            route: HostCapabilityRoute::Model,
            availability: CapabilityAvailability::Available,
            exposure: HostCapabilityExposure::Internal,
            permission_domain: None,
            input_schema: serde_json::json!({"type":"object"}),
            input_normalization: Default::default(),
            output_policy: HostOutputPolicy {
                preview_char_limit: 4000,
                persist_full_output: true,
                allow_artifacts: true,
            },
            supports_streaming: true,
            supports_argument_streaming: false,
            parallelism: HostCapabilityParallelism::Serial,
            cancellation: HostCancellationMode::Cooperative,
            replay_safety: HostReplaySafety::Recoverable,
        },
    );
    AgentCoreConfig {
        turn_identity: support::model_identity::identity(),
        session_id: Some(SessionId::from_string("c".into())),
        model: Some("fixture-model".into()),
        host_catalog: Some(HostCapabilityCatalog {
            host_id: HostId("fixture-host".into()),
            revision: 1,
            availability_epoch: 1,
            lease: None,
            capabilities,
        }),
        ..AgentCoreConfig::default()
    }
}
fn command(
    request: &HostRequestEnvelope,
    run: &str,
    digest: &str,
    sequence: u64,
    payload: HostResponse,
) -> ApplicationCommand {
    ApplicationCommand::ReduceAndAcceptHostResponse {
        input: ReduceAndAcceptHostResponse {
            conversation_id: "c".into(),
            run_id: run.into(),
            invocation_config_digest: digest.into(),
            received_at_ms: 2000 + sequence as i64,
            response: HostResponseEnvelope {
                attempt_id: request.attempt_id.clone(),
                delegated_invocation_id: request.delegated_invocation_id.clone(),
                request_id: request.request_id.clone(),
                effect_id: request.effect_id.clone(),
                host_id: request.host_id.clone(),
                lease_id: request.lease_id.clone(),
                session_id: request.session_id.clone(),
                turn_id: request.turn_id.clone(),
                catalog_revision: request.catalog_revision,
                availability_epoch: request.availability_epoch,
                delivery: request.delivery.clone(),
                sequence: sequence + request.resume_after_sequence.unwrap_or(0),
                payload,
            },
        },
    }
}
fn stream_head(store: &NolaStore, owner: &CanonicalStreamOwner) -> CanonicalStreamHead {
    match RunEffectRepository::load_canonical_text_state(store, owner).unwrap() {
        CanonicalTextState::Stream(head) => head,
        state => panic!("real model text must use canonical stream, got {state:?}"),
    }
}
fn original(store: &NolaStore, owner: &CanonicalStreamOwner) -> String {
    store
        .load_conversation("c")
        .unwrap()
        .unwrap()
        .messages
        .iter()
        .find(|message| message.id == owner.message_id)
        .unwrap()
        .parts
        .iter()
        .find(|part| part.id == owner.part_id)
        .unwrap()
        .content_text
        .clone()
}

fn begin(database: &std::path::Path) -> (NolaStore, String, String, HostRequestEnvelope, String) {
    let mut store = NolaStore::open(database).unwrap();
    store.register_source_submission(&nola_domain::remote_submission::RemoteSubmissionIdentity {
        source_host_id:"verified-host".into(),client_id:"fixture-client".into(),conversation_id:"c".into(),submission_id:"submission".into(),user_message_id:"u".into(),assistant_message_id:"submission:assistant".into(),
    },&serde_json::json!({"fixture_submission":"submission","user_text":"Stream a response"})).unwrap();
    let begun = begin_assistant_turn(
        &mut store,
        BeginAssistantTurn {
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            conversation_id: "c".into(),
            conversation_title: "stream".into(),
            user_message_id: "u".into(),
            user_part_id: "u:body".into(),
            user_text: "Stream a response".into(),
            user_content_json: None,
            assistant_message_id: "submission:assistant".into(),
            context_checkpoint_id: None,
            core_config: config(),
            created_at_ms: 1000,
            submission: AgentSubmission::UserInput {
                items: vec![AgentItem::user_message(
                    ItemId::from_string("u".into()),
                    "Stream a response",
                )],
                turn_context_items: vec![],
                turn_context_platform: Default::default(),
                turn_context_components: vec![],
                submission_id: Some("submission".into()),
                submitted_at_epoch_ms: Some(1000),
                delivery: AgentInputDelivery::CurrentTurn,
            },
        },
    )
    .unwrap();
    assert!(!begun.assistant_publications.is_empty(), "admission must expose its actual published owners");
    for cursor in &begun.assistant_publications {
        assert_eq!(store.assistant_journal_head(&cursor.identity).unwrap(), *cursor);
    }
    let run = begun.begin.run_id;
    let digest = begun.invocation_config_digest;
    let mut request = load_committed_host_requests(&mut store, &run)
        .unwrap()
        .requests
        .into_iter()
        .find(|request| matches!(request.payload, HostRequest::ModelStream { .. }))
        .unwrap();
    support::admit_model_sampling(&mut store, &run, &digest, &mut request, 1100);
    let part = match &request.payload {
        HostRequest::ModelStream { text_part_id, .. } => text_part_id.clone(),
        _ => unreachable!(),
    };
    (store, run, digest, request, part)
}

#[test]
fn deferred_progress_trace_survives_rejected_flush_then_commits_before_terminal() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("deferred-trace.sqlite");
    let (store, run, digest, request, _) = begin(&database);
    let initial_revision = store.load_checkpoint("c").unwrap().unwrap().revision;
    drop(store);
    let mut runtime = support::open_runtime(&database).unwrap();
    let progress: AcceptedHostResponse = serde_json::from_value(runtime.execute(command(
        &request, &run, &digest, 1, HostResponse::ToolProgress {
            title: "preparing response".into(), detail: None, metadata: serde_json::Value::Null,
        },
    )).unwrap()).unwrap();
    assert!(!progress.commit.applied, "progress must retain the bounded batching contract");
    assert!(!progress.traces.is_empty(), "fixture must exercise actual control traces");
    let trace_id = progress.traces[0].trace_id.clone();
    let event_ids: Vec<_> = progress.traces.iter().map(|event| event.event_id.clone()).collect();
    let observer = NolaStore::open(&database).unwrap();
    assert_eq!(observer.load_checkpoint("c").unwrap().unwrap().revision, initial_revision);
    assert!(observer.trace_journal_events_after(&trace_id, 0).unwrap().iter()
        .all(|event| !event_ids.contains(&event.event_id)), "uncommitted progress leaked to disk");
    drop(observer);
    let completion = || command(&request, &run, &digest, 2, HostResponse::ModelCompleted {
        items: vec![serde_json::to_value(AgentItem::assistant_message(ItemId::from_string("result".into()), "done")).unwrap()],
        tool_calls: vec![], usage: serde_json::json!({}),
    });
    let fault = rusqlite::Connection::open(&database).unwrap();
    fault.execute_batch("CREATE TRIGGER reject_terminal_trace BEFORE INSERT ON trace_journal_events
        WHEN NEW.kind='run.completed' BEGIN SELECT RAISE(ABORT, 'trace flush rejected'); END;").unwrap();
    let error = runtime.execute(completion()).unwrap_err();
    assert!(error.contains("trace flush rejected"), "{error}");
    let observer = NolaStore::open(&database).unwrap();
    assert!(observer.trace_journal_events_after(&trace_id, 0).unwrap().iter()
        .all(|event| !event_ids.contains(&event.event_id)), "rolled-back progress leaked");
    drop(observer);
    fault.execute_batch("DROP TRIGGER reject_terminal_trace").unwrap();
    runtime.execute(completion()).unwrap();
    drop(runtime);
    let observer = NolaStore::open(&database).unwrap();
    let stored = observer.trace_journal_events_after(&trace_id, 0).unwrap();
    for id in event_ids { assert_eq!(stored.iter().filter(|event| event.event_id == id).count(), 1); }
    assert_eq!(stored.last().unwrap().kind, "run.completed");
    assert!(stored.windows(2).all(|pair| pair[1].sequence == pair[0].sequence + 1));
}

#[test]
fn admission_preserves_actual_reasoning_and_text_arrival_order() {
    use nola_domain::assistant_journal::AssistantJournalIdentity;
    for reasoning_first in [true, false] {
        let directory = tempfile::tempdir().unwrap();
        let (mut store, run, digest, request, text_id) =
            begin(&directory.path().join("arrival-order.sqlite"));
        let identity = AssistantJournalIdentity {
            source_store_epoch: store.assistant_source_epoch().unwrap(),
            conversation_id: "c".into(), run_id: run.clone(),
            assistant_message_id: "submission:assistant".into(),
        };
        assert_eq!(store.assistant_journal_head(&identity).unwrap().sequence, 0);
        let reasoning_id = match &request.payload {
            HostRequest::ModelStream { reasoning_part_id, .. } => reasoning_part_id.clone(),
            _ => unreachable!(),
        };
        let reasoning = HostResponse::ModelDelta {
            channel: ModelDeltaChannel::Reasoning,
            part_id: reasoning_id.clone(), delta: "思考内容".into(),
        };
        let text = HostResponse::ModelCommittedDelta {
            part_id: text_id.clone(), delta: "你好".into(),
        };
        let updates = if reasoning_first { [reasoning, text] } else { [text, reasoning] };
        for (index, payload) in updates.into_iter().enumerate() {
            let ApplicationCommand::ReduceAndAcceptHostResponse { input } =
                command(&request, &run, &digest, index as u64 + 1, payload) else { panic!() };
            nola_application::reduce_and_accept_host_response(&mut store, input).unwrap();
        }
        let reasoning = store.assistant_journal_part(&identity, &reasoning_id).unwrap().unwrap();
        let text = store.assistant_journal_part(&identity, &text_id).unwrap().unwrap();
        assert_eq!(reasoning.order_key.position < text.order_key.position, reasoning_first,
            "display order follows real emitted items, never admission placeholders or kind sorting");
        let expected = if reasoning_first { vec![reasoning_id, text_id] }
            else { vec![text_id, reasoning_id] };
        store.begin_write_unit_of_work().unwrap();
        let snapshot = store.open_assistant_journal_snapshot(&identity).unwrap();
        store.commit_write_unit_of_work().unwrap();
        let page = store.read_assistant_snapshot_parts(&snapshot, None, 128, 262144).unwrap();
        assert_eq!(page.parts.into_iter().map(|part| part.part_id).collect::<Vec<_>>(), expected);
    }
}

#[test]
fn publication_failure_rolls_back_real_host_delta_and_cold_retry_publishes_once() {
    use nola_domain::assistant_journal::AssistantJournalIdentity;
    use nola_store_port::assistant_journal::AssistantJournalRepository;

    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("publication-rollback.sqlite");
    let (mut store, run, digest, request, part) = begin(&database);
    let message_id = store.load_conversation("c").unwrap().unwrap().messages
        .into_iter().find(|message| message.parts.iter().any(|p| p.id == part))
        .unwrap().id;
    let owner = CanonicalStreamOwner {
        conversation_id: "c".into(), run_id: run.clone(), message_id: message_id.clone(),
        part_id: part.clone(), field: PartBodyField::ContentText,
    };
    let identity = AssistantJournalIdentity {
        source_store_epoch: store.assistant_source_epoch().unwrap(),
        conversation_id: "c".into(), run_id: run.clone(), assistant_message_id: message_id,
    };
    let checkpoint = store.load_checkpoint("c").unwrap().unwrap();
    let before_head = store.assistant_journal_head(&identity).unwrap();
    let before_part = store.assistant_journal_part(&identity, &part).unwrap();
    let before_text = original(&store, &owner);
    // Compare exact durable journal rows, including prepared content and order
    // reservations: a failed public projection must not leave invisible debris.
    let journal_rows = || {
        let connection = rusqlite::Connection::open(&database).unwrap();
        let mut names = connection.prepare(
            "SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'assistant_journal_%' ORDER BY name"
        ).unwrap();
        let tables: Vec<String> = names.query_map([], |row| row.get(0)).unwrap()
            .map(Result::unwrap).collect();
        tables.into_iter().map(|table| {
            let mut statement = connection.prepare(&format!("SELECT * FROM {table} ORDER BY rowid")).unwrap();
            let columns = statement.column_count();
            let rows: Vec<Vec<rusqlite::types::Value>> = statement.query_map([], |row| {
                (0..columns).map(|column| row.get(column)).collect()
            }).unwrap().map(Result::unwrap).collect();
            (table, rows)
        }).collect::<Vec<_>>()
    };
    let before_rows = journal_rows();
    let fault = rusqlite::Connection::open(&database).unwrap();
    fault.execute_batch("CREATE TRIGGER reject_test_journal BEFORE INSERT ON assistant_journal_records
        BEGIN SELECT RAISE(ABORT, 'injected journal publication failure'); END;").unwrap();
    let input = || match command(&request, &run, &digest, 1, HostResponse::ModelDelta {
        channel: ModelDeltaChannel::Text, part_id: part.clone(), delta: "提交后才可见🙂".into(),
    }) {
        ApplicationCommand::ReduceAndAcceptHostResponse { input } => input,
        _ => unreachable!(),
    };
    let error = nola_application::reduce_and_accept_host_response(&mut store, input()).unwrap_err();
    assert!(error.contains("injected journal publication failure"), "{error}");
    drop(store);

    let mut store = NolaStore::open(&database).unwrap();
    let restored = store.load_checkpoint("c").unwrap().unwrap();
    assert_eq!(restored.revision, checkpoint.revision);
    assert_eq!(restored.checkpoint_json, checkpoint.checkpoint_json);
    assert_eq!(original(&store, &owner), before_text);
    assert_eq!(store.assistant_journal_head(&identity).unwrap(), before_head);
    assert_eq!(store.assistant_journal_part(&identity, &part).unwrap(), before_part);
    assert_eq!(journal_rows(), before_rows);
    fault.execute_batch("DROP TRIGGER reject_test_journal").unwrap();
    let accepted = nola_application::reduce_and_accept_host_response(&mut store, input()).unwrap();
    assert!(accepted.commit.applied);
    assert_eq!(accepted.commit.checkpoint_revision, checkpoint.revision + 1);
    let published = store.assistant_journal_head(&identity).unwrap();
    assert!(accepted.commit.assistant_publications.contains(&published));
    assert_ne!(published, before_head);
    drop(store);
    let store = NolaStore::open(&database).unwrap();
    assert_eq!(original(&store, &owner), format!("{before_text}提交后才可见🙂"));
    assert_eq!(store.assistant_journal_head(&identity).unwrap(), published);
}

#[test]
fn two_store_application_ingestion_preserves_source_cursor_and_verified_body() {
    use base64::Engine;
    use nola_application::{AssistantJournalRecord,AssistantJournalChunk,AssistantJournalReceiveResult};
    let directory=tempfile::tempdir().unwrap();
    let source_path=directory.path().join("source.sqlite");
    let target_path=directory.path().join("target.sqlite");
    let (mut source,run,digest,request,part)=begin(&source_path);
    let ApplicationCommand::ReduceAndAcceptHostResponse {input}=command(&request,&run,&digest,1,
        HostResponse::ModelDelta {channel:ModelDeltaChannel::Text,part_id:part.clone(),delta:"跨端正文🙂".into()}) else {panic!()};
    let accepted=nola_application::reduce_and_accept_host_response(&mut source,input).unwrap();
    let cursor=accepted.commit.assistant_publications.iter().find(|cursor|
        source.assistant_journal_part(&cursor.identity,&part).unwrap().is_some()).unwrap().clone();
    let genesis=nola_domain::assistant_journal::AssistantJournalCursor::initial(cursor.identity.clone()).unwrap();
    let window=source.open_assistant_journal_read(&genesis).unwrap();
    let nola_store_port::assistant_journal::AssistantJournalPage::Records {records,..}=
        source.read_assistant_journal_page(&window,128,262144).unwrap() else {panic!()};
    source.begin_write_unit_of_work().unwrap();
    let snapshot=source.open_assistant_journal_snapshot_at(&cursor).unwrap();
    source.commit_write_unit_of_work().unwrap();
    let content=source.assistant_journal_part(&cursor.identity,&part).unwrap().unwrap().bodies[0].clone();
    let receipt=source.observed_source_submission_receipt("verified-host","fixture-client","c",&run,&cursor.identity.assistant_message_id).unwrap().unwrap();
    drop(source);
    let mut source_runtime=support::open_runtime(&source_path).unwrap();
    let wire_chunk:AssistantJournalChunk=serde_json::from_value(source_runtime.execute(
        ApplicationCommand::ReadAssistantJournalRecordChunk {
            snapshot:nola_application::AssistantJournalSnapshotHandle {snapshot_id:snapshot.snapshot_id,at:snapshot.at},
            part_id:part.clone(),content:content.clone(),
            chunk_index:nola_domain::assistant_journal_operations::JournalNumber(0),
        }).unwrap()).unwrap();
    assert_eq!(base64::engine::general_purpose::STANDARD.decode(&wire_chunk.data_base64).unwrap(),"跨端正文🙂".as_bytes());
    drop(source_runtime);
    let mut target=NolaStore::open(&target_path).unwrap();
    target.admit_remote_fixture_from(&nola_store_port::remote_session::AdmittedReplicaTurnInput {
        user_part_kind: "text".into(),
        source_host_id:"verified-host".into(),conversation_id:"c".into(),conversation_title:"Remote".into(),
        user_message_id:"u".into(),user_part_id:"u:body".into(),user_text:"Stream a response".into(),user_content_json:None,
        assistant_message_id:cursor.identity.assistant_message_id.clone(),assistant_part_id:"remote-placeholder".into(),
        run_id:run.clone(),context_checkpoint_id:None,created_at_ms:1,
    },receipt).unwrap();drop(target);
    let mut runtime=support::open_runtime(&target_path).unwrap();
    let window_scope = ProjectionScopeKey { scope_type: "conversation".into(), scope_id: "c".into() };
    let window_cursor_before = NolaStore::open(&target_path).unwrap()
        .projection_cursor(&window_scope).unwrap().map(|cursor| cursor.current_sequence);
    for record in records {
        let record=AssistantJournalRecord {header:record.header,batch:record.batch};
        let mut corrupt=wire_chunk.clone();corrupt.data_base64="AA==".into();
        assert!(runtime.execute(ApplicationCommand::ReceiveAssistantJournalRecord {
            source_host_id:"verified-host".into(),record:record.clone(),chunks:vec![corrupt],
        }).is_err());
        loop {
            let need:Option<nola_store_port::assistant_journal::AssistantJournalContentNeed>=serde_json::from_value(runtime.execute(
                ApplicationCommand::NextAssistantJournalContent {source_host_id:"verified-host".into(),record:record.clone()},
            ).unwrap()).unwrap();
            let Some(need)=need else {break};
            assert_eq!(need.expected_prefix.chunk_count.0,0);
            let prepared:nola_domain::assistant_journal_operations::JournalContentRef=serde_json::from_value(runtime.execute(
                ApplicationCommand::StageAssistantJournalContent {input:nola_application::StageAssistantJournalContent {
                    source_host_id:"verified-host".into(),record:record.clone(),part_id:need.part_id,
                    content:need.content.clone(),expected_prefix:need.expected_prefix,chunks:vec![wire_chunk.clone()],restart:false,
                }}).unwrap()).unwrap();
            assert_eq!(prepared,need.content);
            // Lose every in-memory download cursor before the next decision.
            drop(runtime); runtime=support::open_runtime(&target_path).unwrap();
        }
        let receive_command = ApplicationCommand::ReceiveAssistantJournalRecord {
            source_host_id: "verified-host".into(), record: record.clone(), chunks: vec![],
        };
        let accepted = runtime.execute_received_command(ExecuteReceivedCommand {
            client_id: "remote-stream-fixture".into(),
            request_id: format!("record-{}", record.header.sequence),
            command_type: "receive_assistant_journal_record".into(),
            command_json: serde_json::to_string(&receive_command).unwrap(),
            expected_revision: None,
            received_at: "2001".into(),
        }, receive_command).unwrap();
        let received: AssistantJournalReceiveResult = match accepted.outcome {
            nola_application::ApplicationCommandReceiptOutcome::Committed { result, .. } =>
                serde_json::from_value(result).unwrap(),
            outcome => panic!("expected committed imported journal receipt, got {outcome:?}"),
        };
        assert!(received.applied);
        assert_eq!(received.cursor.sequence,record.header.sequence);
        assert_eq!(received.cursor.record_digest,record.header.record_digest);
        assert_eq!(NolaStore::open(&target_path).unwrap().projection_cursor(&window_scope)
            .unwrap().map(|cursor| cursor.current_sequence), window_cursor_before,
            "imported journal records must not reload the existing conversation window");
    }
    drop(runtime);
    let mut runtime=support::open_runtime(&target_path).unwrap();
    let snapshot:nola_application::AssistantJournalSnapshotHandle=serde_json::from_value(runtime.execute(
        ApplicationCommand::OpenAssistantJournalSnapshot {at:cursor.clone()}).unwrap()).unwrap();
    let read:nola_application::AssistantJournalTextRange=serde_json::from_value(runtime.execute(
        ApplicationCommand::ReadAssistantJournalRange {snapshot,part_id:part,content,
            offset_bytes:nola_domain::assistant_journal_operations::JournalNumber(0),limit_bytes:32768,
        }).unwrap()).unwrap();
    assert_eq!(read.text,"跨端正文🙂");
}

#[test]
fn exact_old_publication_range_survives_new_output_and_cold_reopen_then_release() {
    use nola_application::{AssistantJournalSnapshotHandle, AssistantJournalTextRange};
    use nola_domain::assistant_journal_operations::JournalNumber;
    let directory=tempfile::tempdir().unwrap();
    let database=directory.path().join("old-publication-range.sqlite");
    let (mut store,run,digest,request,part)=begin(&database);
    let delta=|sequence,text:&str| match command(&request,&run,&digest,sequence,HostResponse::ModelDelta {
        channel:ModelDeltaChannel::Text,part_id:part.clone(),delta:text.into(),
    }) {
        ApplicationCommand::ReduceAndAcceptHostResponse {input}=>input,
        _=>unreachable!(),
    };
    let first=nola_application::reduce_and_accept_host_response(&mut store,delta(1,"中🙂x")).unwrap();
    let at=first.commit.assistant_publications.iter().find(|cursor|
        store.assistant_journal_part(&cursor.identity,&part).unwrap().is_some()).unwrap().clone();
    let old_content=store.assistant_journal_part(&at.identity,&part).unwrap().unwrap().bodies[0].clone();
    nola_application::reduce_and_accept_host_response(&mut store,delta(2,"后续正文")).unwrap();
    let current_content=store.assistant_journal_part(&at.identity,&part).unwrap().unwrap().bodies[0].clone();
    assert_ne!(old_content,current_content);
    let reasoning_part=match &request.payload {
        HostRequest::ModelStream {reasoning_part_id,..}=>reasoning_part_id.clone(),
        _=>unreachable!(),
    };
    let reasoning=match command(&request,&run,&digest,3,HostResponse::ModelDelta {
        channel:ModelDeltaChannel::Reasoning,part_id:reasoning_part,delta:"检查版本".into(),
    }) {
        ApplicationCommand::ReduceAndAcceptHostResponse {input}=>input,
        _=>unreachable!(),
    };
    nola_application::reduce_and_accept_host_response(&mut store,reasoning).unwrap();
    drop(store);
    let mut runtime=support::open_runtime(&database).unwrap();
    // Both directory entries postdate genesis. Empty pages must still advance
    // through their keys instead of silently claiming end of snapshot.
    let genesis:AssistantJournalSnapshotHandle=serde_json::from_value(runtime.execute(
        ApplicationCommand::OpenAssistantJournalSnapshot {
            at:nola_domain::assistant_journal::AssistantJournalCursor::initial(at.identity.clone()).unwrap(),
        }).unwrap()).unwrap();
    let empty:nola_application::AssistantJournalPartPage=serde_json::from_value(runtime.execute(
        ApplicationCommand::ReadAssistantJournalParts {
            snapshot:genesis.clone(),after_part_id:None,max_records:1,max_encoded_bytes:65536,
        }).unwrap()).unwrap();
    assert!(empty.parts.is_empty());
    assert!(empty.next_part_id.is_some(),"postdated directory entries are not end-of-snapshot");
    let last:nola_application::AssistantJournalPartPage=serde_json::from_value(runtime.execute(
        ApplicationCommand::ReadAssistantJournalParts {
            snapshot:genesis.clone(),after_part_id:empty.next_part_id,max_records:1,max_encoded_bytes:65536,
        }).unwrap()).unwrap();
    assert!(last.parts.is_empty());assert!(last.next_part_id.is_none());
    runtime.execute(ApplicationCommand::ReleaseAssistantJournalSnapshot {snapshot:genesis}).unwrap();
    let snapshot:AssistantJournalSnapshotHandle=serde_json::from_value(runtime.execute(
        ApplicationCommand::OpenAssistantJournalSnapshot {at:at.clone()}).unwrap()).unwrap();
    assert_eq!(snapshot.at,at,"pin the requested version, not the latest head");
    drop(runtime);
    let mut runtime=support::open_runtime(&database).unwrap();
    let read=|snapshot:AssistantJournalSnapshotHandle,content,offset|ApplicationCommand::ReadAssistantJournalRange {
        snapshot,part_id:part.clone(),content,offset_bytes:JournalNumber(offset),limit_bytes:4,
    };
    let mut assembled=String::new();
    let mut offset=0;
    loop {
        let page:AssistantJournalTextRange=serde_json::from_value(runtime.execute(
            read(snapshot.clone(),old_content.clone(),offset)).unwrap()).unwrap();
        assert_eq!(page.offset_bytes.0,offset);
        assert_eq!(page.total_utf8_bytes.0,"中🙂x".len() as u64);
        assert!(page.text.len()<=4);
        assembled.push_str(&page.text);
        if page.end_of_content {break;}
        assert!(page.next_offset_bytes.0>offset);
        offset=page.next_offset_bytes.0;
    }
    assert_eq!(assembled,"中🙂x");
    // Consume the same public manifest/chunk commands a remote client needs;
    // verify bytes and reconstruct the owner-bound content root, not just text.
    use base64::Engine;
    use nola_domain::assistant_journal_operations::{empty_journal_content,append_journal_content};
    let mut reconstructed=empty_journal_content(&at.identity,&part,old_content.field,old_content.content_generation).unwrap();
    let mut next_chunk=0;
    loop {
        let manifest:nola_application::AssistantJournalManifestPage=serde_json::from_value(runtime.execute(
            ApplicationCommand::ReadAssistantJournalManifest {
                snapshot:snapshot.clone(),part_id:part.clone(),content:old_content.clone(),
                after_chunk:JournalNumber(next_chunk),max_records:1,
            }).unwrap()).unwrap();
        for (index,reference) in manifest.chunks.iter().enumerate() {
            let chunk:nola_application::AssistantJournalChunk=serde_json::from_value(runtime.execute(
                ApplicationCommand::ReadAssistantJournalChunk {
                    snapshot:snapshot.clone(),part_id:part.clone(),content:old_content.clone(),
                    chunk_index:JournalNumber(next_chunk+index as u64),
                }).unwrap()).unwrap();
            let bytes=base64::engine::general_purpose::STANDARD.decode(chunk.data_base64).unwrap();
            let verified=nola_domain::assistant_journal::AssistantTextChunkV1::new(String::from_utf8(bytes).unwrap()).unwrap();
            assert_eq!(verified.digest(),reference.digest);
            assert_eq!(verified.digest(),chunk.digest);
            assert_eq!(verified.byte_length() as u64,reference.utf8_bytes.0);
        }
        if !manifest.chunks.is_empty() {
            reconstructed=append_journal_content(&reconstructed,&manifest.chunks).unwrap();
        }
        if manifest.complete {break;}
        assert!(manifest.next_chunk.0>next_chunk);next_chunk=manifest.next_chunk.0;
    }
    assert_eq!(reconstructed,old_content);
    assert!(runtime.execute(read(snapshot.clone(),current_content,0)).is_err(),
        "an old handle cannot read a newer content reference");
    let mut forged=snapshot.clone();forged.at.sequence+=1;
    assert!(runtime.execute(read(forged,old_content.clone(),0)).is_err());
    for _ in 0..2 {
        runtime.execute(ApplicationCommand::ReleaseAssistantJournalSnapshot {snapshot:snapshot.clone()}).unwrap();
    }
    drop(runtime);
    let mut runtime=support::open_runtime(&database).unwrap();
    assert!(runtime.execute(read(snapshot,old_content,0)).unwrap_err().contains("released"));
}

#[test]
fn real_batched_deltas_reopen_receipt_replay_and_close_preserve_one_original() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("canonical-stream.sqlite");
    let (store, run, digest, request, part) = begin(&database);
    let initial_revision = store.load_checkpoint("c").unwrap().unwrap().revision;
    let initial_message=store.load_conversation("c").unwrap().unwrap().messages.into_iter()
        .find(|message|message.parts.iter().any(|item|item.id==part)).unwrap().id;
    let initial_cursor=store.assistant_journal_head(&nola_domain::assistant_journal::AssistantJournalIdentity {
        source_store_epoch:store.assistant_source_epoch().unwrap(),conversation_id:"c".into(),
        run_id:run.clone(),assistant_message_id:initial_message,
    }).unwrap();
    drop(store);
    let delta = |text: &str| HostResponse::ModelDelta {
        channel: ModelDeltaChannel::Text,
        part_id: part.clone(),
        delta: text.into(),
    };
    let mut runtime = support::open_runtime(&database).unwrap();
    for (sequence, text) in [(1, "中"), (2, "🙂")] {
        let result: AcceptedHostResponse = serde_json::from_value(
            runtime
                .execute(command(&request, &run, &digest, sequence, delta(text)))
                .unwrap(),
        )
        .unwrap();
        assert!(
            !result.commit.applied,
            "small deltas must be in the same pending batch"
        );
        assert!(result.commit.assistant_publications.is_empty(), "buffered text has no committed watermark");
        let read:nola_application::AssistantJournalReadResult=serde_json::from_value(runtime.execute(
            ApplicationCommand::ReadAssistantJournal {
                after:initial_cursor.clone(),through:initial_cursor.clone(),
                max_records:1,max_encoded_bytes:65536,
            }).unwrap()).unwrap();
        assert!(matches!(read,nola_application::AssistantJournalReadResult::Records {
            records,complete:true,..
        } if records.is_empty()));
        let observer=NolaStore::open(&database).unwrap();
        assert_eq!(observer.load_checkpoint("c").unwrap().unwrap().revision,initial_revision,
            "journal polling cannot flush buffered provider text");
    }
    // Exercise the byte commit boundary without provoking Core's legitimate
    // structural-repetition recovery (a 70KiB repeated character is rejected).
    let suffix: String = (0..2048)
        .map(|index| format!("Record {index:06}: canonical UTF-8 segment {index:06}.\n"))
        .collect();
    assert!(suffix.len() > 64 * 1024);
    let batched: AcceptedHostResponse = serde_json::from_value(
        runtime
            .execute(command(&request, &run, &digest, 3, delta(&suffix)))
            .unwrap(),
    )
    .unwrap();
    assert!(batched.commit.applied);
    assert_eq!(batched.commit.checkpoint_revision, initial_revision + 1);
    drop(runtime);
    let store = NolaStore::open(&database).unwrap();
    // Resolve the actual Core-owned assistant rather than guessing from provider IDs.
    assert!(!batched.commit.assistant_publications.is_empty());
    for cursor in &batched.commit.assistant_publications {
        assert_eq!(store.assistant_journal_head(&cursor.identity).unwrap(), *cursor);
    }
    let conversation = store.load_conversation("c").unwrap().unwrap();
    let message = conversation
        .messages
        .iter()
        .find(|message| message.parts.iter().any(|p| p.id == part))
        .unwrap();
    let owner = CanonicalStreamOwner {
        conversation_id: "c".into(),
        run_id: run.clone(),
        message_id: message.id.clone(),
        part_id: part.clone(),
        field: PartBodyField::ContentText,
    };
    let before = stream_head(&store, &owner);
    let mut expected = format!("中🙂{suffix}");
    assert_eq!(original(&store, &owner), expected);
    assert_eq!(before.total_utf8_bytes, expected.len() as u64);
    assert_eq!(before.generation, 1);
    assert!(
        before.chunk_count >= 4,
        "large delta must be split without merging old prefixes"
    );
    assert!(!before.closed);
    drop(store);

    // Cold runtime must continue the persisted stream, not rebuild it from a UI prefix.
    let mut runtime = support::open_runtime(&database).unwrap();
    let serialized =
        serde_json::to_string(&command(&request, &run, &digest, 4, delta("尾"))).unwrap();
    let receipt = || ExecuteReceivedCommand {
        client_id: "stream-fixture".into(),
        request_id: "delta-4".into(),
        command_type: "reduce_and_accept_host_response".into(),
        command_json: serialized.clone(),
        expected_revision: None,
        received_at: "2004".into(),
    };
    let window_scope = ProjectionScopeKey { scope_type: "conversation".into(), scope_id: "c".into() };
    let window_cursor_before = NolaStore::open(&database).unwrap()
        .projection_cursor(&window_scope).unwrap().map(|cursor| cursor.current_sequence);
    let accepted = runtime
        .execute_received_command(receipt(), serde_json::from_str(&serialized).unwrap())
        .unwrap();
    assert!(!accepted.replayed);
    let receipt_result: AcceptedHostResponse = match &accepted.outcome {
        nola_application::ApplicationCommandReceiptOutcome::Committed { result, .. } =>
            serde_json::from_value(result.clone()).unwrap(),
        outcome => panic!("expected committed stream receipt, got {outcome:?}"),
    };
    assert!(!receipt_result.commit.assistant_publications.is_empty());
    drop(runtime);
    expected.push('尾');
    let store = NolaStore::open(&database).unwrap();
    let continued = stream_head(&store, &owner);
    assert_eq!(store.projection_cursor(&window_scope).unwrap().map(|cursor| cursor.current_sequence),
        window_cursor_before, "journal-only delta must not force a conversation window reload");
    for cursor in &receipt_result.commit.assistant_publications {
        assert_eq!(store.assistant_journal_head(&cursor.identity).unwrap(), *cursor);
    }
    assert_eq!(continued.generation, before.generation);
    assert_eq!(continued.chunk_count, before.chunk_count + 1);
    assert_eq!(original(&store, &owner), expected);
    drop(store);
    let mut runtime = support::open_runtime(&database).unwrap();
    let replay = runtime
        .execute_received_command(receipt(), serde_json::from_str(&serialized).unwrap())
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(accepted.outcome, replay.outcome);
    let observer = NolaStore::open(&database).unwrap();
    assert_eq!(stream_head(&observer, &owner), continued);
    drop(observer);
    let completion_command = command(
        &request,
        &run,
        &digest,
        5,
        HostResponse::ModelCompleted {
            items: vec![serde_json::to_value(AgentItem::assistant_message(
                ItemId::from_string("provider-result".into()),
                expected.clone(),
            ))
            .unwrap()],
            tool_calls: vec![],
            usage: serde_json::json!({}),
        },
    );
    let completion_receipt = runtime.execute_received_command(
        ExecuteReceivedCommand {
            client_id: "stream-fixture".into(),
            request_id: "completion-5".into(),
            command_type: "reduce_and_accept_host_response".into(),
            command_json: serde_json::to_string(&completion_command).unwrap(),
            expected_revision: None,
            received_at: "2005".into(),
        },
        completion_command,
    ).unwrap();
    let completion: AcceptedHostResponse = match completion_receipt.outcome {
        nola_application::ApplicationCommandReceiptOutcome::Committed { result, .. } =>
            serde_json::from_value(result).unwrap(),
        outcome => panic!("expected committed completion receipt, got {outcome:?}"),
    };
    drop(runtime);
    let store = NolaStore::open(&database).unwrap();
    let closed = stream_head(&store, &owner);
    let window_cursor_after = store.projection_cursor(&window_scope).unwrap()
        .map(|cursor| cursor.current_sequence);
    assert!(window_cursor_after > window_cursor_before,
        "terminal lifecycle must still invalidate the conversation window");
    let parts: Vec<_> = store
        .load_conversation("c")
        .unwrap()
        .unwrap()
        .messages
        .into_iter()
        .filter(|message| message.role == "assistant")
        .flat_map(|message| {
            message.parts.into_iter().map(move |part| {
                (
                    message.id.clone(),
                    part.id,
                    part.state,
                    part.content_text.len(),
                )
            })
        })
        .collect();
    assert!(closed.closed,"completion applied={} status={:?}, original head gen/rev={}/{}, parts(id/state/bytes)={parts:?}",
        completion.commit.applied,completion.control.as_ref().map(|control|control.turn_status),closed.generation,closed.revision);
    assert_eq!(closed.generation, continued.generation);
    assert_eq!(closed.total_utf8_bytes, expected.len() as u64);
    assert_eq!(original(&store, &owner), expected);
    assert!(store.active_run("c").unwrap().is_none());
}

#[test]
fn repetitive_completion_closes_discarded_stream_without_completing_run() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("discarded-stream.sqlite");
    let (store, run, digest, request, part) = begin(&database);
    drop(store);
    // The real Core guard checks repetitive 160-scalar windows from 8KiB.
    let repeated = "z".repeat(nola_agent_engine::agent::loop_guard::DELEGATED_OUTPUT_GUARD_MIN_CHARS);
    let mut runtime = support::open_runtime(&database).unwrap();
    let delta_command = command(
        &request,
        &run,
        &digest,
        1,
        HostResponse::ModelDelta {
            channel: ModelDeltaChannel::Text,
            part_id: part.clone(),
            delta: repeated.clone(),
        },
    );
    let command_json = serde_json::to_string(&delta_command).unwrap();
    runtime
        .execute_received_command(
            ExecuteReceivedCommand {
                client_id: "discard-fixture".into(),
                request_id: "delta-1".into(),
                command_type: "reduce_and_accept_host_response".into(),
                command_json,
                expected_revision: None,
                received_at: "2001".into(),
            },
            delta_command,
        )
        .unwrap();
    let observer = NolaStore::open(&database).unwrap();
    let conversation = observer.load_conversation("c").unwrap().unwrap();
    let message = conversation
        .messages
        .iter()
        .find(|m| m.parts.iter().any(|p| p.id == part))
        .unwrap();
    let owner = CanonicalStreamOwner {
        conversation_id: "c".into(),
        run_id: run.clone(),
        message_id: message.id.clone(),
        part_id: part,
        field: PartBodyField::ContentText,
    };
    let before = stream_head(&observer, &owner);
    assert!(!before.closed);
    assert_eq!(original(&observer, &owner), repeated);
    drop(observer);
    let completed: AcceptedHostResponse = serde_json::from_value(
        runtime
            .execute(command(
                &request,
                &run,
                &digest,
                2,
                HostResponse::ModelCompleted {
                    items: vec![serde_json::to_value(AgentItem::assistant_message(
                        ItemId::from_string("repetitive-result".into()),
                        repeated,
                    ))
                    .unwrap()],
                    tool_calls: vec![],
                    usage: serde_json::json!({}),
                },
            ))
            .unwrap(),
    )
    .unwrap();
    assert!(completed.commit.applied);
    assert_eq!(
        completed.control.as_ref().unwrap().turn_status,
        nola_agent_engine::agent::AgentTurnStatus::AwaitingModel
    );
    drop(runtime);
    let store = NolaStore::open(&database).unwrap();
    let discarded = stream_head(&store, &owner);
    assert!(
        discarded.closed,
        "discard must Close after replacing original text with empty content"
    );
    assert_eq!(discarded.generation, before.generation + 1);
    assert_eq!((discarded.total_utf8_bytes, discarded.chunk_count), (0, 0));
    assert_eq!(original(&store, &owner), "");
    assert_eq!(store.active_run("c").unwrap().unwrap().run_id, run);
    assert!(
        !store.pending_effects(&run).unwrap().is_empty(),
        "Core requests another model effect, not a fabricated terminal"
    );
}

#[test]
fn failed_and_cancelled_stream_windows_report_actual_original_range_eligibility() {
    for terminal in ["failed", "cancelled"] {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("terminal-reader.sqlite");
        let (store, run, digest, request, part) = begin(&database);
        drop(store);
        let text: String = (0..768)
            .map(|i| format!("Record {i}: retained 中文🙂 original segment.\n"))
            .collect();
        let mut runtime = support::open_runtime(&database).unwrap();
        let delta = command(
            &request,
            &run,
            &digest,
            1,
            HostResponse::ModelDelta {
                channel: ModelDeltaChannel::Text,
                part_id: part.clone(),
                delta: text.clone(),
            },
        );
        runtime
            .execute_received_command(
                ExecuteReceivedCommand {
                    client_id: "terminal-reader".into(),
                    request_id: "first-delta".into(),
                    command_type: "reduce_and_accept_host_response".into(),
                    command_json: serde_json::to_string(&delta).unwrap(),
                    expected_revision: None,
                    received_at: "2001".into(),
                },
                delta,
            )
            .unwrap();
        let observer = NolaStore::open(&database).unwrap();
        let before = observer
            .load_conversation_message_window("c", None, 1, 512)
            .unwrap()
            .unwrap();
        let reference = before
            .deferred_text_parts
            .iter()
            .find(|p| p.part_id == part)
            .unwrap();
        assert!(!reference.is_final && !reference.can_read_original);
        let message = before
            .conversation
            .messages
            .iter()
            .find(|m| m.parts.iter().any(|p| p.id == part))
            .unwrap();
        let owner = CanonicalStreamOwner {
            conversation_id: "c".into(),
            run_id: run.clone(),
            message_id: message.id.clone(),
            part_id: part.clone(),
            field: PartBodyField::ContentText,
        };
        assert_eq!(original(&observer, &owner), text);
        drop(observer);
        if terminal == "failed" {
            runtime
                .execute(command(
                    &request,
                    &run,
                    &digest,
                    2,
                    HostResponse::Failed {
                        error: HostError {
                            code: "provider_failure".into(),
                            message: "Fixture failed".into(),
                            retryable: false,
                            http_status: Some(400),
                        },
                    },
                ))
                .unwrap();
        } else {
            let cancellation = ApplicationCommand::RequestRunCancellation {
                input: nola_application::RequestRunCancellation {
                    conversation_id: "c".into(),
                    expected_run_id: run.clone(),
                    assistant_message_id: owner.message_id.clone(),
                    cancellation_id: "cancel-reader".into(),
                    reason: Some("User stopped".into()),
                    requested_at_ms: 3000,
                },
            };
            runtime
                .execute_received_command(
                    ExecuteReceivedCommand {
                        client_id: "terminal-reader".into(),
                        request_id: "cancel-reader".into(),
                        command_type: "request_run_cancellation".into(),
                        command_json: serde_json::to_string(&cancellation).unwrap(),
                        expected_revision: None,
                        received_at: "3000".into(),
                    },
                    cancellation,
                )
                .unwrap();
            let claims: nola_application::CommittedHostRequests = serde_json::from_value(
                runtime
                    .execute(ApplicationCommand::ClaimCommittedHostRequests {
                        input: nola_application::ClaimCommittedHostRequests {
                            run_id: run.clone(),
                            worker_generation: "cancel-reader".into(),
                            lease_owner: "cancel-reader".into(),
                            claimed_at_ms: 3100,
                            lease_expires_at_ms: 10_000,
                            limit: 64,
                        },
                    })
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(claims.requests.len(), 1);
            let cancel = &claims.requests[0];
            let HostRequest::Cancel { target_effect_id, .. } = &cancel.payload else {
                panic!("expected cancellation");
            };
            for (sequence, payload) in [
                (1, HostResponse::ExecutionStarted),
                (
                    2,
                    HostResponse::CancellationConfirmed {
                        target_effect_id: target_effect_id.clone(),
                    },
                ),
            ] {
                runtime
                    .execute(command(cancel, &run, &digest, sequence, payload))
                    .unwrap();
            }
        }
        drop(runtime);
        for _ in 0..2 {
            let store = NolaStore::open(&database).unwrap();
            let window = store
                .load_conversation_message_window("c", None, 1, 512)
                .unwrap()
                .unwrap();
            let message = window
                .conversation
                .messages
                .iter()
                .find(|m| m.id == owner.message_id)
                .unwrap();
            assert_eq!(message.status, terminal);
            let reference = window
                .deferred_text_parts
                .iter()
                .find(|p| p.part_id == part)
                .unwrap();
            assert!(reference.is_final);
            assert!(
                !reference.can_read_original,
                "Finality cannot override the original range gate"
            );
            assert!(stream_head(&store, &owner).closed);
            assert_eq!(original(&store, &owner), text);
            assert!(store
                .read_conversation_text_range("c", &owner.message_id, &part, 0, 32)
                .is_err());
            assert!(store.active_run("c").unwrap().is_none());
        }
    }
}

#[test]
fn exact_trace_replay_cold_pages_pin_frontier_and_reject_foreign_owners_or_gaps() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("exact-trace-replay.sqlite");
    let (store, run, digest, request, _) = begin(&database);
    drop(store);
    let mut runtime = support::open_runtime(&database).unwrap();
    let replay = |after, through, limit| ApplicationCommand::LoadExactRunTraceReplay {
        conversation_id: "c".into(), run_id: run.clone(), assistant_message_id: "submission:assistant".into(),
        after_sequence: after, through_sequence: through, limit,
    };
    let first = runtime.execute(replay(0,None,1)).unwrap();
    let frontier = first["through_sequence"].as_i64().unwrap();
    assert!(frontier >= 1);
    assert_eq!(first["next_sequence"],1);
    runtime.execute(command(&request,&run,&digest,1,HostResponse::ModelCompleted {
        items:vec![serde_json::to_value(AgentItem::assistant_message(ItemId::from_string("result".into()),"done")).unwrap()],
        tool_calls:vec![],usage:serde_json::json!({}),
    })).unwrap();
    drop(runtime);
    // The second page is served by a fresh Application after more events were
    // committed. It must stop at the original immutable journal frontier.
    let mut runtime = support::open_runtime(&database).unwrap();
    let mut sequence = 1;
    let mut all = first["events"].as_array().unwrap().clone();
    while sequence < frontier {
        let page = runtime.execute(replay(sequence,Some(frontier),1)).unwrap();
        assert_eq!(page["through_sequence"],frontier);
        sequence = page["next_sequence"].as_i64().unwrap();
        all.extend(page["events"].as_array().unwrap().clone());
    }
    let live = runtime.execute(replay(frontier,None,256)).unwrap();
    assert!(live["through_sequence"].as_i64().unwrap()>frontier);
    all.extend(live["events"].as_array().unwrap().clone());
    for (index,event) in all.iter().enumerate() { assert_eq!(event["sequence"],index as u64+1); }
    assert_eq!(all.last().unwrap()["kind"],"run.completed");
    for (conversation,assistant) in [("foreign","submission:assistant"),("c","foreign")] {
        assert!(runtime.execute(ApplicationCommand::LoadExactRunTraceReplay {
            conversation_id:conversation.into(),run_id:run.clone(),assistant_message_id:assistant.into(),
            after_sequence:0,through_sequence:None,limit:256,
        }).is_err());
    }
    let db = rusqlite::Connection::open(&database).unwrap();
    let original: String = db.query_row("SELECT event_json FROM trace_journal_events WHERE sequence=1",[],|row|row.get(0)).unwrap();
    db.execute("UPDATE trace_journal_events SET event_json=json_set(event_json,'$.run_id','foreign') WHERE sequence=1",[]).unwrap();
    assert!(runtime.execute(replay(0,None,256)).unwrap_err().contains("payload identity"));
    db.execute("UPDATE trace_journal_events SET event_json=?1 WHERE sequence=1",[original]).unwrap();
    db.execute("DELETE FROM trace_journal_events WHERE sequence=2",[]).unwrap();
    assert!(runtime.execute(replay(0,None,256)).unwrap_err().contains("gap"));
}
