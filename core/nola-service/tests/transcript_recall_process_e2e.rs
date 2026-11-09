// SPDX-License-Identifier: Apache-2.0
#[path = "../../nola-application/tests/support/model_identity.rs"]
mod fixture_model_identity;
use nola_application::{ApplicationCommand, CreateConversation};
use nola_application_bootstrap::ApplicationRuntime;
use nola_protocol::{ConversationTranscriptImportSource, ImportConversationTranscript};
use nola_protocol::{
    TranscriptMessageInput as MessageInput, TranscriptPartInput as MessagePartInput,
};
use nola_service_client::ApplicationServiceClient;
use serde_json::Value;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

fn resident_bytes(pid: u32) -> u64 {
    let result = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .expect("sample service RSS");
    assert!(result.status.success(), "service RSS query failed");
    String::from_utf8(result.stdout)
        .unwrap()
        .trim()
        .parse::<u64>()
        .unwrap()
        * 1024
}

struct MemorySampler {
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<(u64, usize)>>,
}

impl MemorySampler {
    fn start(pid: u32) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let worker = std::thread::spawn(move || {
            let mut peak = 0;
            let mut samples = 0;
            while !stopped.load(Ordering::Relaxed) {
                peak = peak.max(resident_bytes(pid));
                samples += 1;
                std::thread::sleep(Duration::from_millis(25));
            }
            (peak, samples)
        });
        Self {
            stop,
            worker: Some(worker),
        }
    }

    fn finish(mut self) -> (u64, usize) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker
            .take()
            .unwrap()
            .join()
            .expect("service RSS sampler panicked")
    }
}

impl Drop for MemorySampler {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct Service(Child);
impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start(db: &Path, socket: &Path, client: &ApplicationServiceClient) -> Service {
    let mut service = Service(
        Command::new(env!("CARGO_BIN_EXE_nola-service"))
            .arg("--database")
            .arg(db)
            .arg("--socket")
            .arg(socket)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if client.health().is_ok() {
            return service;
        }
        assert!(service.0.try_wait().unwrap().is_none());
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("recall service startup timed out");
}

fn seed(runtime: &mut ApplicationRuntime, id: &str, text: String) {
    runtime
        .execute(ApplicationCommand::CreateConversation {
            input: CreateConversation {
                id: id.into(),
                title: id.into(),
                settings_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
            },
        })
        .unwrap();
    runtime
        .execute(ApplicationCommand::ImportConversationTranscript {
            input: ImportConversationTranscript {
                import_id: format!("import-{id}"),
                source: ConversationTranscriptImportSource::ExternalArchive,
                conversation_id: id.into(),
                expected_message_count: 0,
                imported_at_ms: 2,
                messages: vec![MessageInput {
                    id: format!("message-{id}"),
                    run_id: None,
                    role: "user".into(),
                    status: "completed".into(),
                    ordinal: 1,
                    created_at_ms: 1,
                    completed_at_ms: Some(1),
                    parts: vec![MessagePartInput {
                        id: format!("part-{id}"),
                        kind: "text".into(),
                        ordinal: 1,
                        state: "completed".into(),
                        content_text: text,
                        content_json: None,
                        content_digest: None,
                        created_at_ms: 1,
                        updated_at_ms: 1,
                    }],
                }],
            },
        })
        .unwrap();
}

fn document_fixture(id: &str, scope: &str, text: String) -> nola_application::DocumentSource {
    nola_application::DocumentSource {
        id: id.into(),
        scope: scope.into(),
        uri: format!("fixture://{id}"),
        kind: "uploaded_document".into(),
        mime_type: Some("text/plain".into()),
        sha256: None,
        byte_count: Some(text.len() as i64),
        title: Some(id.into()),
        text_content: Some(text),
        metadata: serde_json::json!({"name":id,"labels":["reference"]}),
        created_at_ms: 1,
        updated_at_ms: 1,
    }
}

fn replace_documents(
    client: &ApplicationServiceClient,
    scope: &str,
    documents: Vec<nola_application::DocumentSource>,
) -> Result<Value, String> {
    client
        .execute(ApplicationCommand::DocumentSource {
            command: nola_application::DocumentSourceCommand::Replace {
                scope: scope.into(),
                kind: "uploaded_document".into(),
                inputs: documents,
            },
        })
        .map_err(|error| error.to_string())
}

#[test]
fn document_body_round_trips_real_service_but_search_returns_only_a_summary() {
    use nola_application::{DocumentSource, DocumentSourceCommand, DocumentSourceSummary};
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("documents.db");
    let socket = root.path().join("documents.sock");
    let client = ApplicationServiceClient::with_socket(&database, &socket);
    let service = start(&database, &socket, &client);
    let baseline = resident_bytes(service.0.id());
    let sampler = MemorySampler::start(service.0.id());
    let started = Instant::now();
    // New canonical writes carry the body once, not again in metadata. This
    // tests the Service contract; CLI conversion has its own round-trip tests.
    let marker = "DOCUMENT-TAIL-unique";
    let mut text = "x".repeat(2_000_000 - marker.len());
    text.push_str(marker);
    let document = document_fixture("large-document", "web:local", text);
    let written = replace_documents(&client, "web:local", vec![document.clone()]).unwrap();
    assert_eq!(written, serde_json::json!(1));
    assert!(
        serde_json::to_vec(&written).unwrap().len() <= 16,
        "mutation echoed document bodies"
    );
    replace_documents(
        &client,
        "conversation:private",
        vec![document_fixture(
            "private-document",
            "conversation:private",
            "foreign-secret".into(),
        )],
    )
    .unwrap();
    let summaries: Vec<DocumentSourceSummary> = client
        .execute(ApplicationCommand::DocumentSource {
            command: DocumentSourceCommand::Search {
                scope: "web:local".into(),
                query: Some(marker.into()),
                kind: None,
                limit: 10,
            },
        })
        .unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].id, document.id);
    assert!(summaries[0].metadata.get("content").is_none());
    let response_bytes = serde_json::to_vec(&summaries).unwrap().len();
    assert!(
        response_bytes < 8192,
        "summary carried the complete document: {response_bytes}"
    );
    let foreign: Vec<DocumentSourceSummary> = client
        .execute(ApplicationCommand::DocumentSource {
            command: DocumentSourceCommand::Search {
                scope: "web:local".into(),
                query: Some("foreign-secret".into()),
                kind: None,
                limit: 10,
            },
        })
        .unwrap();
    assert!(foreign.is_empty());
    let (peak, samples) = sampler.finish();
    eprintln!("document_ipc_2m: elapsed={:?} baseline={baseline} peak={peak} samples={samples} summary_bytes={response_bytes}", started.elapsed());
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(peak <= baseline + 128 * 1024 * 1024);
    drop(service);
    let _restarted = start(&database, &socket, &client);
    let loaded: Vec<DocumentSource> = client
        .execute(ApplicationCommand::DocumentSource {
            command: DocumentSourceCommand::List {
                scope: "web:local".into(),
                kind: None,
            },
        })
        .unwrap();
    assert_eq!(loaded, vec![document]);
}

#[test]
fn document_replacement_cannot_take_an_existing_identity_from_another_scope() {
    use nola_application::{DocumentSource, DocumentSourceCommand};
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("document-identity.db");
    let socket = root.path().join("document-identity.sock");
    let client = ApplicationServiceClient::with_socket(&database, &socket);
    let service = start(&database, &socket, &client);
    let first = document_fixture("one", "conversation:1", "first original".into());
    let second = document_fixture("two", "conversation:2", "second original".into());
    for document in [&first, &second] {
        replace_documents(&client, &document.scope, vec![document.clone()]).unwrap();
    }
    let mut collision = first.clone();
    collision.id = second.id.clone();
    assert!(
        replace_documents(&client, &first.scope, vec![collision]).is_err(),
        "replacement stole another scope's document identity"
    );
    drop(service);
    let _restarted = start(&database, &socket, &client);
    for expected in [first, second] {
        let loaded: Vec<DocumentSource> = client
            .execute(ApplicationCommand::DocumentSource {
                command: DocumentSourceCommand::List {
                    scope: expected.scope.clone(),
                    kind: None,
                },
            })
            .unwrap();
        assert_eq!(loaded, vec![expected]);
    }
}

#[test]
fn oversized_document_metadata_does_not_escape_through_search_ipc() {
    use nola_application::{DocumentSource, DocumentSourceCommand, DocumentSourceSummary};
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("document-metadata.db");
    let socket = root.path().join("document-metadata.sock");
    let client = ApplicationServiceClient::with_socket(&database, &socket);
    let service = start(&database, &socket, &client);
    let mut source = document_fixture("oversized", "web:local", "original document".into());
    source.metadata = serde_json::json!({"large":"x".repeat(64 * 1024)});
    replace_documents(&client, "web:local", vec![source.clone()]).unwrap();
    drop(service);
    let _restarted = start(&database, &socket, &client);
    let summaries: Vec<DocumentSourceSummary> = client
        .execute(ApplicationCommand::DocumentSource {
            command: DocumentSourceCommand::Search {
                scope: "web:local".into(),
                query: None,
                kind: None,
                limit: 1,
            },
        })
        .unwrap();
    assert_eq!(summaries.len(), 1);
    assert!(summaries[0].metadata_truncated);
    assert_eq!(summaries[0].metadata, serde_json::json!({}));
    assert!(serde_json::to_vec(&summaries).unwrap().len() < 2048);
    let originals: Vec<DocumentSource> = client
        .execute(ApplicationCommand::DocumentSource {
            command: DocumentSourceCommand::List {
                scope: "web:local".into(),
                kind: None,
            },
        })
        .unwrap();
    assert_eq!(originals, vec![source]);
}

#[test]
fn checkpoint_count_does_not_load_ten_million_bytes_of_components() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("context-count.db");
    let socket = root.path().join("context-count.sock");
    let mut runtime = nola_application_bootstrap::open(&database).unwrap();
    for id in ["c", "other"] {
        runtime
            .execute(ApplicationCommand::CreateConversation {
                input: CreateConversation {
                    id: id.into(),
                    title: id.into(),
                    settings_json: None,
                    created_at_ms: 1,
                    updated_at_ms: 1,
                },
            })
            .unwrap();
    }
    drop(runtime);
    // Isolated fixture: seed large component bodies before starting the service.
    let mut connection = rusqlite::Connection::open(&database).unwrap();
    let tx = connection.transaction().unwrap();
    let content =
        serde_json::json!({"type":"inline_preview","text":"x".repeat(50_000),"truncated":false})
            .to_string();
    for index in 0..201 {
        let id = format!("cp-{index}");
        tx.execute("INSERT INTO context_checkpoints(id,conversation_id,model_profile_id,policy_version,estimated_tokens,context_window_tokens,prompt_limit_tokens,payload_digest,created_at_ms)
            VALUES(?1,?2,'fixture','v2',100,64000,60000,'fixture',1)",
            rusqlite::params![id, if index == 200 { "other" } else { "c" }]).unwrap();
        tx.execute("INSERT INTO context_components(id,checkpoint_id,kind,authorization,freshness,content_reference_json,content_digest,estimated_tokens,provenance_json)
            VALUES(?1,?1,'data_source','authorized','fresh',?2,'fixture',100,'{}')",
            rusqlite::params![id, content]).unwrap();
    }
    tx.commit().unwrap();
    drop(connection);
    let client = ApplicationServiceClient::with_socket(&database, &socket);
    let service = start(&database, &socket, &client);
    let baseline = resident_bytes(service.0.id());
    let sampler = MemorySampler::start(service.0.id());
    let started = Instant::now();
    for _ in 0..64 {
        let count: u64 = client
            .execute(ApplicationCommand::Context {
                command: nola_application::ContextCommand::CountCheckpoints {
                    conversation_id: "c".into(),
                },
            })
            .unwrap();
        assert_eq!(count, 200);
    }
    let (peak, samples) = sampler.finish();
    eprintln!("context_count_10m: queries=64 elapsed={:?} baseline={baseline} peak={peak} samples={samples}", started.elapsed());
    assert!(
        peak <= baseline + 8 * 1024 * 1024,
        "count loaded component bodies"
    );
    assert!(started.elapsed() < Duration::from_secs(5));
    drop(service);
    let _restarted = start(&database, &socket, &client);
    for (id, expected) in [("c", 200_u64), ("other", 1), ("missing", 0)] {
        let count: u64 = client
            .execute(ApplicationCommand::Context {
                command: nola_application::ContextCommand::CountCheckpoints {
                    conversation_id: id.into(),
                },
            })
            .unwrap();
        assert_eq!(count, expected);
    }
}

#[test]
fn beginning_a_turn_does_not_republish_ten_million_characters() {
    use nola_core::agent::host::*;
    use nola_core::agent::protocol::{
        AgentCoreConfig, AgentInputDelivery, AgentItem, AgentSubmission, ItemId, SessionId,
    };
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("begin-long.db");
    let socket = root.path().join("begin-long.sock");
    let mut runtime = nola_application_bootstrap::open(&database).unwrap();
    seed(&mut runtime, "begin-long", "x".repeat(10_000_000));
    drop(runtime);
    let client = ApplicationServiceClient::with_socket(&database, &socket);
    let service = start(&database, &socket, &client);
    let tail: nola_application::ConversationTailSnapshot = client
        .execute::<Option<nola_application::ConversationTailSnapshot>, _>(
            ApplicationCommand::LoadConversationTailSnapshotV1 {
                conversation_id: "begin-long".into(),
                message_limit: 1,
                part_text_limit: 1,
                part_json_limit: 128,
            },
        )
        .unwrap()
        .unwrap();
    let configuration_revision = tail.conversation.configuration_revision;
    let parent_message_id = tail
        .conversation
        .messages
        .last()
        .map(|message| message.id.clone());
    let config = AgentCoreConfig {
        turn_identity: fixture_model_identity::identity(),
        session_id: Some(SessionId::from_string("begin-long".into())),
        model: Some("fixture-model".into()),
        host_catalog: Some(HostCapabilityCatalog {
            host_id: HostId("fixture-host".into()),
            revision: 1,
            availability_epoch: 1,
            lease: None,
            capabilities: std::collections::BTreeMap::from([(
                "model.stream".into(),
                HostCapabilityDescriptor {
                    id: "model.stream".into(),
                    title: "Fixture model".into(),
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
            )]),
        }),
        ..Default::default()
    };
    let input = nola_application::BeginAssistantTurn {
        expected_configuration_revision: Some(configuration_revision),
        expected_parent_message_id: parent_message_id,
        conversation_id: "begin-long".into(),
        conversation_title: "Long history".into(),
        user_message_id: "next-user".into(),
        user_part_id: "next-user:text".into(),
        user_text: "Continue".into(),
        user_content_json: None,
        assistant_message_id: "next-assistant".into(),
        context_checkpoint_id: None,
        core_config: config,
        submission: AgentSubmission::UserInput {
            items: vec![AgentItem::user_message(
                ItemId::from_string("next-user".into()),
                "Continue",
            )],
            turn_context_items: Vec::new(),
            turn_context_platform: Default::default(),
            turn_context_components: Vec::new(),
            submission_id: Some("next-submission".into()),
            submitted_at_epoch_ms: Some(1000),
            delivery: AgentInputDelivery::CurrentTurn,
        },
        created_at_ms: 1000,
    };
    // This guards growth caused by history, not one-time mapping/initialization
    // of Core code and schemas. Warm the same admission path on an empty owner;
    // cold-start history growth is covered separately by the four-history test.
    let mut warm = input.clone();
    warm.expected_configuration_revision = None;
    warm.expected_parent_message_id = None;
    warm.conversation_id = "warm-empty".into();
    warm.user_message_id = "warm-user".into();
    warm.user_part_id = "warm-user:text".into();
    warm.assistant_message_id = "warm-assistant".into();
    warm.core_config.session_id = Some(SessionId::from_string("warm-empty".into()));
    warm.submission = AgentSubmission::UserInput {
        items: vec![AgentItem::user_message(ItemId::from_string("warm-user".into()), "Continue")],
        turn_context_items: Vec::new(), turn_context_platform: Default::default(),
        turn_context_components: Vec::new(), submission_id: Some("warm-submission".into()),
        submitted_at_epoch_ms: Some(1000), delivery: AgentInputDelivery::CurrentTurn,
    };
    let _: Value = client.execute(ApplicationCommand::BeginAssistantTurn { input: warm }).unwrap();
    let baseline = resident_bytes(service.0.id());
    let sampler = MemorySampler::start(service.0.id());
    std::thread::sleep(Duration::from_millis(30));
    let started = Instant::now();
    let result: Value = client.execute(ApplicationCommand::BeginAssistantTurn { input }).unwrap();
    let elapsed = started.elapsed();
    let (peak, samples) = sampler.finish();
    assert!(result["begin"]["run_id"].as_str().is_some());
    let connection = rusqlite::Connection::open_with_flags(
        &database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let published_bytes: i64 = connection.query_row(
        "SELECT COALESCE(SUM(length(CAST(payload_json AS BLOB))),0) FROM sync_outbox WHERE entity_id='begin-long' AND revision>2",
        [], |row| row.get(0),
    ).unwrap();
    let message_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE conversation_id='begin-long'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    eprintln!("begin_turn_10m: elapsed={elapsed:?} baseline_rss={baseline} peak_rss={peak} samples={samples} published_bytes={published_bytes}");
    assert_eq!(
        message_count, 3,
        "old message, new user, and immediate assistant placeholder must survive"
    );
    assert!(
        published_bytes > 0 && published_bytes < 65_536,
        "short turn republished old transcript: {published_bytes} bytes"
    );
    assert!(samples > 0);
    assert!(
        peak.saturating_sub(baseline) < 8 * 1024 * 1024,
        "begin turn hydrated old history"
    );
    assert!(
        elapsed < Duration::from_secs(1),
        "short turn admission exceeded budget"
    );
}

#[test]
fn renaming_ten_million_character_history_is_bounded_and_replay_safe() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("rename.db");
    let socket = root.path().join("rename.sock");
    let mut runtime = nola_application_bootstrap::open(&database).unwrap();
    seed(&mut runtime, "rename", "x".repeat(10_000_000));
    drop(runtime);
    let client = ApplicationServiceClient::with_socket(&database, &socket);
    let service = start(&database, &socket, &client);
    let baseline = resident_bytes(service.0.id());
    let sampler = MemorySampler::start(service.0.id());
    std::thread::sleep(Duration::from_millis(30));
    let started = Instant::now();
    for index in 0..8_u64 {
        let result: bool = client
            .execute_with_revision(
                format!("rename-{index}"),
                index + 2,
                ApplicationCommand::RenameConversation {
                    title_source: None,
                    conversation_id: "rename".into(),
                    title: format!("Renamed {index}"),
                    updated_at_ms: index as i64 + 10,
                },
            )
            .unwrap();
        assert!(result);
    }
    let elapsed = started.elapsed();
    let (peak, samples) = sampler.finish();
    eprintln!("rename_10m: operations=8 elapsed={elapsed:?} baseline_rss={baseline} peak_rss={peak} samples={samples}");
    assert!(samples > 0);
    assert!(
        peak.saturating_sub(baseline) < 16 * 1024 * 1024,
        "rename hydrated long transcript"
    );
    assert!(
        elapsed < Duration::from_secs(3),
        "metadata-only rename loop exceeded budget"
    );
    let connection = rusqlite::Connection::open_with_flags(
        &database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let bytes: i64 = connection.query_row(
        "SELECT COALESCE(SUM(length(CAST(payload_json AS BLOB))),0) FROM sync_outbox WHERE entity_id='rename' AND revision>2",
        [], |row| row.get(0),
    ).unwrap();
    assert!(
        bytes > 0 && bytes < 16_384,
        "rename republished transcript bytes"
    );
    drop(connection);
    let committed = durable_counts(&database);
    drop(service);
    let _service = start(&database, &socket, &client);
    let replay: bool = client
        .execute_with_revision(
            "rename-7",
            9,
            ApplicationCommand::RenameConversation {
                title_source: None,
                conversation_id: "rename".into(),
                title: "Renamed 7".into(),
                updated_at_ms: 17,
            },
        )
        .unwrap();
    assert!(replay);
    assert_eq!(durable_counts(&database), committed);
    let stale = client
        .submit_with_revision(
            "stale-rename",
            Some(2),
            ApplicationCommand::RenameConversation {
                title_source: None,
                conversation_id: "rename".into(),
                title: "Stale".into(),
                updated_at_ms: 20,
            },
        )
        .unwrap();
    assert!(matches!(
        stale.outcome,
        nola_protocol::CommandReceiptOutcome::Conflict { .. }
    ));
    let window: Value = client
        .execute(ApplicationCommand::LoadConversationProjectionWindowV1 {
            conversation_id: "rename".into(),
            before_ordinal: None,
            limit: 4,
            content_byte_limit: 4096,
        })
        .unwrap();
    assert_eq!(window["projection"]["title"], "Renamed 7");
    assert_eq!(window["event_sequence"], 8);
    assert_eq!(
        window["deferred_text_parts"][0]["total_utf8_bytes"],
        10_000_000
    );
}

fn durable_counts(db: &Path) -> (i64, i64, i64) {
    let connection =
        rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    connection
        .query_row(
            "SELECT (SELECT COUNT(*) FROM command_receipts),
        (SELECT COUNT(*) FROM projection_events),(SELECT SUM(revision) FROM conversations)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap()
}

#[test]
fn importing_into_ten_million_character_history_refreshes_a_bounded_window() {
    let root = tempfile::tempdir().unwrap();
    let db = root.path().join("import-long-history.db");
    let socket = root.path().join("import-long-history.sock");
    let mut runtime = nola_application_bootstrap::open(&db).unwrap();
    seed(&mut runtime, "a", "x".repeat(10_000_000));
    drop(runtime);
    let client = ApplicationServiceClient::with_socket(&db, &socket);
    let service = start(&db, &socket, &client);
    let baseline = resident_bytes(service.0.id());
    let sampler = MemorySampler::start(service.0.id());
    let command = || nola_protocol::TranscriptCommand::ImportConversationTranscript {
        input: ImportConversationTranscript {
            import_id: "additional-archive".into(),
            source: ConversationTranscriptImportSource::ExternalArchive,
            conversation_id: "a".into(),
            expected_message_count: 1,
            imported_at_ms: 3,
            messages: vec![MessageInput {
                id: "imported-message".into(),
                run_id: None,
                role: "assistant".into(),
                status: "completed".into(),
                ordinal: 2,
                created_at_ms: 2,
                completed_at_ms: Some(2),
                parts: vec![MessagePartInput {
                    id: "imported-part".into(),
                    kind: "text".into(),
                    ordinal: 1,
                    state: "completed".into(),
                    content_text: "new archive evidence".into(),
                    content_json: None,
                    content_digest: None,
                    created_at_ms: 2,
                    updated_at_ms: 2,
                }],
            }],
        },
    };
    let update = |after_sequence| ApplicationCommand::LoadConversationProjectionWindowUpdateV1 {
        conversation_id: "a".into(),
        before_ordinal: None,
        limit: 2,
        content_byte_limit: 4096,
        after_sequence,
    };
    let started = Instant::now();
    let first: Value = client
        .execute_with_request_id("durable-import", command())
        .unwrap();
    let window: Value = client.execute(update(0)).unwrap();
    assert!(!window.is_null());
    assert_eq!(
        window["projection"]["messages"].as_array().unwrap().len(),
        1
    );
    assert_eq!(
        window["projection"]["messages"][0]["message_id"],
        "imported-message"
    );
    assert_eq!(window["has_more"], true);
    assert_eq!(window["total_message_count"], 2);
    let before_ordinal = window["before_ordinal"]
        .as_i64()
        .expect("history continuation cursor");
    assert_eq!(before_ordinal, 2);
    let older: Value = client
        .execute(ApplicationCommand::LoadConversationProjectionWindowV1 {
            conversation_id: "a".into(),
            before_ordinal: Some(before_ordinal),
            limit: 2,
            content_byte_limit: 4096,
        })
        .unwrap();
    assert_eq!(older["projection"]["messages"].as_array().unwrap().len(), 1);
    assert_eq!(
        older["projection"]["messages"][0]["message_id"],
        "message-a"
    );
    assert_eq!(older["has_more"], false);
    assert!(
        older.to_string().len() < 16_384,
        "older page must remain bounded too"
    );
    assert_eq!(older["deferred_text_parts"][0]["part_id"], "part-a");
    assert_eq!(
        older["deferred_text_parts"][0]["total_utf8_bytes"],
        10_000_000
    );
    assert!(
        older["deferred_text_parts"][0]["preview_utf8_bytes"]
            .as_u64()
            .unwrap()
            <= 4096
    );
    let original: Value = client
        .execute(ApplicationCommand::ReadConversationTextRange {
            conversation_id: "a".into(),
            message_id: "message-a".into(),
            part_id: "part-a".into(),
            offset_bytes: 9_999_990,
            limit_bytes: 32,
        })
        .unwrap();
    assert_eq!(original["text"], "xxxxxxxxxx");
    assert_eq!(original["end_of_content"], true);
    assert!(
        window.to_string().len() < 16_384,
        "window must not contain the full old message"
    );
    let sequence = window["event_sequence"].as_u64().unwrap();
    assert_eq!(sequence, 1);
    let committed = durable_counts(&db);
    for _ in 0..12 {
        let caught_up: Value = client.execute(update(sequence)).unwrap();
        assert!(caught_up.is_null());
    }
    assert_eq!(durable_counts(&db), committed);
    let ending = resident_bytes(service.0.id());
    let (sampled, samples) = sampler.finish();
    let peak = sampled.max(baseline).max(ending);
    eprintln!("import_into_10m elapsed={:?} baseline_rss={baseline} peak_rss={peak} samples={samples} window_bytes={}",
        started.elapsed(),window.to_string().len());
    assert!(
        peak.saturating_sub(baseline) < 32 * 1024 * 1024,
        "import loaded too much old history: baseline={baseline}, peak={peak}"
    );
    drop(service);
    let service = start(&db, &socket, &client);
    let replay: Value = client
        .execute_with_request_id("durable-import", command())
        .unwrap();
    assert_eq!(replay, first);
    assert_eq!(durable_counts(&db), committed);
    let restored: Value = client.execute(update(0)).unwrap();
    assert_eq!(restored, window);
    drop(service);
    let runtime = nola_application_bootstrap::open(&db).unwrap();
    let events = runtime
        .projection_event_suffix(
            nola_application::ApplicationProjectionScope {
                scope_type: "conversation".into(),
                scope_id: "a".into(),
            },
            0,
            10,
        )
        .unwrap();
    assert_eq!(events.events.len(), 1);
    let payload = serde_json::to_string(&events.events[0].event).unwrap();
    assert!(payload.len() < 1024);
    assert!(!payload.contains("new archive evidence"));
    assert!(matches!(
        events.events[0].event.mutation,
        nola_domain::ConversationProjectionMutation::InvalidateProjection { revision: 3 }
    ));
}

#[test]
fn cancellation_survives_real_service_restart_without_touching_canonical_history() {
    use nola_protocol::content_upload::{
        AppendContentChunk, BeginContentUpload, ContentUploadOwner, ContentUploadRequest,
        ReadContentUploadChunk,
    };
    let root = tempfile::tempdir().unwrap();
    let db = root.path().join("cancel.db");
    let socket = root.path().join("cancel.sock");
    let mut runtime = nola_application_bootstrap::open(&db).unwrap();
    seed(&mut runtime, "a", "canonical-evidence-survives".into());
    drop(runtime);
    let before = durable_counts(&db);
    let client = ApplicationServiceClient::with_socket(&db, &socket);
    let owner = ContentUploadOwner {
        conversation_id: "a".into(),
        import_id: "cancel-import".into(),
        part_id: "cancel-part".into(),
    };
    let request = ContentUploadRequest {
        upload_id: "cancel-upload".into(),
        owner: owner.clone(),
    };
    let begin = || ApplicationCommand::BeginContentUpload {
        input: BeginContentUpload {
            upload_id: request.upload_id.clone(),
            owner: owner.clone(),
            expected_utf8_bytes: 10,
        },
    };
    let append = || ApplicationCommand::AppendContentChunk {
        input: AppendContentChunk {
            upload_id: request.upload_id.clone(),
            owner: owner.clone(),
            offset_bytes: 0,
            text: "中😀".into(),
        },
    };
    let service = start(&db, &socket, &client);
    let _: Value = client.execute(begin()).unwrap();
    let _: Value = client.execute(append()).unwrap();
    let mut foreign = request.clone();
    foreign.owner.part_id = "another-part".into();
    assert!(client
        .execute::<Value, _>(ApplicationCommand::CancelContentUpload { input: foreign })
        .is_err());
    let progress: Value = client
        .execute(ApplicationCommand::ContentUploadProgress {
            input: request.clone(),
        })
        .unwrap();
    assert_eq!(progress["committed_utf8_bytes"], 7);
    let cancelled: Value = client
        .execute(ApplicationCommand::CancelContentUpload {
            input: request.clone(),
        })
        .unwrap();
    assert_eq!(cancelled["state"], "cancelled");
    assert_eq!(cancelled["committed_utf8_bytes"], 0);
    drop(service);

    let service = start(&db, &socket, &client);
    let progress: Value = client
        .execute(ApplicationCommand::ContentUploadProgress {
            input: request.clone(),
        })
        .unwrap();
    assert_eq!(progress, cancelled);
    let repeated: Value = client
        .execute(ApplicationCommand::CancelContentUpload {
            input: request.clone(),
        })
        .unwrap();
    assert_eq!(repeated, cancelled);
    assert!(client.execute::<Value, _>(append()).is_err());
    assert!(client.execute::<Value, _>(begin()).is_err());
    assert!(client
        .execute::<Value, _>(ApplicationCommand::SealContentUpload {
            input: request.clone()
        })
        .is_err());
    assert!(client
        .execute::<Value, _>(ApplicationCommand::ReadContentUploadChunk {
            input: ReadContentUploadChunk {
                upload_id: request.upload_id.clone(),
                owner: owner.clone(),
                offset_bytes: 0,
            }
        })
        .is_err());
    let evidence: Value = client
        .execute(ApplicationCommand::SearchConversationText {
            conversation_id: "a".into(),
            query: "canonical-evidence-survives".into(),
            limit: 1,
        })
        .unwrap();
    assert_eq!(evidence.as_array().unwrap().len(), 1);
    assert_eq!(evidence[0]["message_id"], "message-a");
    let after = durable_counts(&db);
    assert_eq!(
        (after.1, after.2),
        (before.1, before.2),
        "staging must not change canonical events or revision"
    );
    let canonical =
        rusqlite::Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let messages: i64 = canonical
        .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
        .unwrap();
    assert_eq!(messages, 1);
    let staging = rusqlite::Connection::open_with_flags(
        root.path().join("cancel.db.uploads.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let chunks: i64 = staging
        .query_row("SELECT COUNT(*) FROM upload_chunks", [], |r| r.get(0))
        .unwrap();
    assert_eq!(chunks, 0, "cancelled body chunks must be removed");
    drop(service);
}

#[test]
fn ten_million_byte_upload_crosses_real_ipc_and_restart_with_bounded_reads() {
    use nola_protocol::content_upload::{
        AppendContentChunk, BeginContentUpload, ContentUploadOwner, ContentUploadRequest,
        ReadContentUploadChunk, CONTENT_CHUNK_MAX_BYTES,
    };
    let root = tempfile::tempdir().unwrap();
    let db = root.path().join("large-upload.db");
    let socket = root.path().join("large-upload.sock");
    let mut runtime = nola_application_bootstrap::open(&db).unwrap();
    runtime
        .execute(ApplicationCommand::CreateConversation {
            input: CreateConversation {
                id: "a".into(),
                title: "Large upload".into(),
                settings_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
            },
        })
        .unwrap();
    drop(runtime);
    let client = ApplicationServiceClient::with_socket(&db, &socket);
    let owner = ContentUploadOwner {
        conversation_id: "a".into(),
        import_id: "large-import".into(),
        part_id: "large-part".into(),
    };
    let request = ContentUploadRequest {
        upload_id: "large".into(),
        owner: owner.clone(),
    };
    let mut service = start(&db, &socket, &client);
    let _: Value = client
        .execute(ApplicationCommand::BeginContentUpload {
            input: BeginContentUpload {
                upload_id: "large".into(),
                owner: owner.clone(),
                expected_utf8_bytes: 10_000_000,
            },
        })
        .unwrap();
    let mut sampler = MemorySampler::start(service.0.id());
    let mut offset = 0_u64;
    let mut restarted = false;
    let mut peak = 0_u64;
    let started = Instant::now();
    while offset < 10_000_000 {
        let length = (10_000_000 - offset).min(CONTENT_CHUNK_MAX_BYTES as u64) as usize;
        let text = if offset + length as u64 == 10_000_000 {
            "x".repeat(length - 9) + "TAIL-DONE"
        } else {
            "x".repeat(length)
        };
        let response: Value = client
            .execute(ApplicationCommand::AppendContentChunk {
                input: AppendContentChunk {
                    upload_id: "large".into(),
                    owner: owner.clone(),
                    offset_bytes: offset,
                    text,
                },
            })
            .unwrap();
        offset += length as u64;
        assert_eq!(response["committed_utf8_bytes"], offset);
        assert!(response.to_string().len() < 4096);
        if !restarted && offset >= 5_000_000 {
            peak = peak.max(sampler.finish().0);
            drop(service);
            service = start(&db, &socket, &client);
            sampler = MemorySampler::start(service.0.id());
            let progress: Value = client
                .execute(ApplicationCommand::ContentUploadProgress {
                    input: request.clone(),
                })
                .unwrap();
            assert_eq!(progress["committed_utf8_bytes"], offset);
            restarted = true;
        }
    }
    let _: Value = client
        .execute(ApplicationCommand::SealContentUpload { input: request })
        .unwrap();
    let before = durable_counts(&db);
    let mut read_offset = 0_u64;
    while read_offset < 10_000_000 {
        let response: Value = client
            .execute(ApplicationCommand::ReadContentUploadChunk {
                input: ReadContentUploadChunk {
                    upload_id: "large".into(),
                    owner: owner.clone(),
                    offset_bytes: read_offset,
                },
            })
            .unwrap();
        let text = response["text"].as_str().unwrap();
        assert!(text.len() <= CONTENT_CHUNK_MAX_BYTES);
        let next = response["next_offset_bytes"].as_u64().unwrap();
        assert_eq!(next, read_offset + text.len() as u64);
        if next == 10_000_000 {
            assert!(text.ends_with("TAIL-DONE"));
        } else {
            assert!(text.bytes().all(|byte| byte == b'x'));
        }
        read_offset = next;
    }
    assert_eq!(durable_counts(&db), before);
    peak = peak.max(sampler.finish().0);
    eprintln!(
        "real_upload_10m elapsed={:?} sampled_peak_rss_bytes={peak}",
        started.elapsed()
    );
    assert!(
        peak < 96 * 1024 * 1024,
        "upload memory exceeded 96 MiB: {peak}"
    );
    assert!(restarted);
    drop(service);
}

#[test]
fn upload_progress_is_fresh_read_only_and_resumes_after_service_restart() {
    use nola_protocol::content_upload::{
        AppendContentChunk, BeginContentUpload, ContentUploadOwner, ContentUploadRequest,
        ReadContentUploadChunk,
    };
    let root = tempfile::tempdir().unwrap();
    let db = root.path().join("upload-test.db");
    let socket = root.path().join("upload.sock");
    let mut runtime = nola_application_bootstrap::open(&db).unwrap();
    runtime
        .execute(ApplicationCommand::CreateConversation {
            input: CreateConversation {
                id: "a".into(),
                title: "A".into(),
                settings_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
            },
        })
        .unwrap();
    drop(runtime);
    let client = ApplicationServiceClient::with_socket(&db, &socket);
    let owner = ContentUploadOwner {
        conversation_id: "a".into(),
        import_id: "import".into(),
        part_id: "part".into(),
    };
    let request = ContentUploadRequest {
        upload_id: "upload".into(),
        owner: owner.clone(),
    };
    let service = start(&db, &socket, &client);
    let before = durable_counts(&db);
    assert!(client
        .execute::<Value, _>(ApplicationCommand::ContentUploadProgress {
            input: request.clone()
        })
        .is_err());
    assert_eq!(durable_counts(&db), before);
    assert!(!root.path().join("upload-test.db.uploads.sqlite").exists());
    let _: Value = client
        .execute(ApplicationCommand::BeginContentUpload {
            input: BeginContentUpload {
                upload_id: "upload".into(),
                owner: owner.clone(),
                expected_utf8_bytes: 10,
            },
        })
        .unwrap();
    let append = || ApplicationCommand::AppendContentChunk {
        input: AppendContentChunk {
            upload_id: "upload".into(),
            owner: owner.clone(),
            offset_bytes: 0,
            text: "中😀".into(),
        },
    };
    let _: Value = client.execute(append()).unwrap();
    let _: Value = client.execute(append()).unwrap();
    let read = |offset_bytes| ApplicationCommand::ReadContentUploadChunk {
        input: ReadContentUploadChunk {
            upload_id: "upload".into(),
            owner: owner.clone(),
            offset_bytes,
        },
    };
    assert!(client.execute::<Value, _>(read(0)).is_err());
    drop(service);
    let service = start(&db, &socket, &client);
    let before = durable_counts(&db);
    for _ in 0..6 {
        let progress: Value = client
            .execute(ApplicationCommand::ContentUploadProgress {
                input: request.clone(),
            })
            .unwrap();
        assert_eq!(progress["committed_utf8_bytes"], 7);
        assert_eq!(progress["state"], "receiving");
    }
    assert_eq!(durable_counts(&db), before);
    let _: Value = client
        .execute(ApplicationCommand::AppendContentChunk {
            input: AppendContentChunk {
                upload_id: "upload".into(),
                owner: owner.clone(),
                offset_bytes: 7,
                text: "end".into(),
            },
        })
        .unwrap();
    let _: Value = client
        .execute(ApplicationCommand::SealContentUpload {
            input: request.clone(),
        })
        .unwrap();
    let progress: Value = client
        .execute(ApplicationCommand::ContentUploadProgress { input: request })
        .unwrap();
    assert_eq!(progress["committed_utf8_bytes"], 10);
    assert_eq!(progress["state"], "sealed");
    let before = durable_counts(&db);
    let first: Value = client.execute(read(0)).unwrap();
    assert_eq!(first["text"], "中😀");
    assert_eq!(first["next_offset_bytes"], 7);
    assert_eq!(first["end_of_content"], false);
    let last: Value = client.execute(read(7)).unwrap();
    assert_eq!(last["text"], "end");
    assert_eq!(last["next_offset_bytes"], 10);
    assert_eq!(last["end_of_content"], true);
    let end: Value = client.execute(read(10)).unwrap();
    assert_eq!(end["text"], "");
    assert_eq!(end["end_of_content"], true);
    assert!(client.execute::<Value, _>(read(1)).is_err());
    assert!(client.execute::<Value, _>(read(u64::MAX)).is_err());
    assert_eq!(durable_counts(&db), before);
    let connection =
        rusqlite::Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let count: i64 = connection
        .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        count, 0,
        "staging must never publish partial canonical messages"
    );
    drop(service);
}

#[test]
fn real_service_recall_is_bounded_read_only_and_survives_restart() {
    let root = tempfile::tempdir().unwrap();
    let db = root.path().join("recall.db");
    let socket = root.path().join("recall.sock");
    let marker = "历史标识 100%_CODE-7319";
    let mut text = "x".repeat(10_000_000 - marker.chars().count());
    text.push_str(marker);
    let mut runtime = nola_application_bootstrap::open(&db).unwrap();
    seed(&mut runtime, "a", text);
    seed(&mut runtime, "b", "foreign-private-marker".into());
    drop(runtime);
    let before = durable_counts(&db);
    let client = ApplicationServiceClient::with_socket(&db, &socket);
    let request = || ApplicationCommand::SearchConversationText {
        conversation_id: "a".into(),
        query: "历史标识".into(),
        limit: 20,
    };
    let service = start(&db, &socket, &client);
    let baseline_rss = resident_bytes(service.0.id());
    let sampler = MemorySampler::start(service.0.id());
    let started = Instant::now();
    let expected: Value = client.execute(request()).unwrap();
    eprintln!(
        "real service 10M recall {:?}, response bytes={}",
        started.elapsed(),
        expected.to_string().len()
    );
    assert_eq!(expected.as_array().unwrap().len(), 1);
    assert_eq!(expected[0]["message_id"], "message-a");
    assert!(expected[0]["snippet"].as_str().unwrap().ends_with(marker));
    assert!(expected.to_string().len() < 8_000);
    for _ in 0..12 {
        let actual: Value = client.execute(request()).unwrap();
        assert_eq!(actual, expected);
    }
    let foreign: Value = client
        .execute(ApplicationCommand::SearchConversationText {
            conversation_id: "a".into(),
            query: "foreign-private-marker".into(),
            limit: 20,
        })
        .unwrap();
    assert!(foreign.as_array().unwrap().is_empty());
    let ending_rss = resident_bytes(service.0.id());
    let (sampled_peak_rss, samples) = sampler.finish();
    assert!(
        samples > 0,
        "query interval must include service RSS samples"
    );
    eprintln!(
        "service_recall_memory {}",
        serde_json::json!({
            "conversation_characters": 10_000_000,
            "baseline_rss_bytes": baseline_rss,
            "sampled_peak_rss_bytes": sampled_peak_rss.max(baseline_rss).max(ending_rss),
            "ending_rss_bytes": ending_rss,
            "samples": samples,
            "sampling_interval_ms": 25,
            "query_count": 14,
        })
    );
    drop(service);
    assert_eq!(durable_counts(&db), before);
    let _ = std::fs::remove_file(&socket);
    let service = start(&db, &socket, &client);
    let restored: Value = client.execute(request()).unwrap();
    assert_eq!(restored, expected);
    drop(service);
    assert_eq!(durable_counts(&db), before);
}

#[test]
fn repeated_recall_across_four_large_conversations_keeps_service_memory_bounded() {
    let root = tempfile::tempdir().unwrap();
    let db = root.path().join("multi-recall.db");
    let socket = root.path().join("multi-recall.sock");
    let mut runtime = nola_application_bootstrap::open(&db).unwrap();
    for index in 0..4 {
        let marker = format!("unique-history-marker-{index}");
        let mut text = "x".repeat(10_000_000 - marker.len());
        text.push_str(&marker);
        seed(&mut runtime, &format!("conversation-{index}"), text);
    }
    drop(runtime);
    let before = durable_counts(&db);
    let client = ApplicationServiceClient::with_socket(&db, &socket);
    let service = start(&db, &socket, &client);
    let baseline = resident_bytes(service.0.id());
    let sampler = MemorySampler::start(service.0.id());
    let mut durations = Vec::new();
    let mut phase_rss = Vec::new();
    let empty_root = tempfile::tempdir().unwrap();
    let empty_database = empty_root.path().join("empty.db");
    let empty_socket = empty_root.path().join("empty.sock");
    drop(nola_application_bootstrap::open(&empty_database).unwrap());
    let empty_client = ApplicationServiceClient::with_socket(&empty_database, &empty_socket);
    let empty_service = start(&empty_database, &empty_socket, &empty_client);
    let empty_rss = resident_bytes(empty_service.0.id());
    drop(empty_service);
    let cold_growth = baseline.saturating_sub(empty_rss);
    eprintln!("four-history cold start: empty_rss={empty_rss}, history_rss={baseline}, growth={cold_growth}");
    assert!(
        cold_growth < 32 * 1024 * 1024,
        "cold startup loaded history or pending content: empty={empty_rss}, history={baseline}"
    );
    for phase in 0..5 {
        // First visit warms each conversation. Later phases alternate owners
        // repeatedly instead of measuring the same cached answer.
        let iterations = if phase == 0 { 4 } else { 16 };
        for iteration in 0..iterations {
            let index = iteration % 4;
            let started = Instant::now();
            let response: Value = client
                .execute(ApplicationCommand::SearchConversationText {
                    conversation_id: format!("conversation-{index}"),
                    query: "unique-history-marker".into(),
                    limit: 20,
                })
                .unwrap();
            durations.push(started.elapsed().as_secs_f64());
            assert_eq!(response.as_array().unwrap().len(), 1);
            assert_eq!(
                response[0]["message_id"],
                format!("message-conversation-{index}")
            );
            assert!(response[0]["snippet"]
                .as_str()
                .unwrap()
                .ends_with(&format!("unique-history-marker-{index}")));
            assert!(response.to_string().len() < 8_000);
        }
        phase_rss.push(resident_bytes(service.0.id()));
    }
    let (peak, samples) = sampler.finish();
    durations.sort_by(f64::total_cmp);
    let warm = phase_rss[0];
    let post_warm_growth = phase_rss
        .iter()
        .copied()
        .max()
        .unwrap()
        .saturating_sub(warm);
    eprintln!(
        "multi_conversation_recall {}",
        serde_json::json!({
            "conversation_count": 4, "characters_per_conversation": 10_000_000,
            "query_count": durations.len(), "baseline_rss_bytes": baseline,
            "empty_service_rss_bytes": empty_rss, "cold_start_growth_bytes": cold_growth,
            "phase_rss_bytes": phase_rss, "sampled_peak_rss_bytes": peak,
            "post_warm_growth_bytes": post_warm_growth, "samples": samples,
            "p95_query_seconds": durations[(durations.len() * 95 / 100).min(durations.len() - 1)],
        })
    );
    assert!(samples > 0);
    assert!(
        post_warm_growth < 32 * 1024 * 1024,
        "repeated bounded recall grew by {post_warm_growth} bytes after warming every conversation"
    );
    drop(service);
    assert_eq!(durable_counts(&db), before);
}

#[test]
fn protocol_configuration_persists_replays_and_rejects_stale_version_after_restart() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("settings.db");
    let socket = root.path().join("settings.sock");
    let mut runtime = nola_application_bootstrap::open(&database).unwrap();
    seed(&mut runtime, "settings", "x".repeat(10_000_000));
    drop(runtime);
    let client = ApplicationServiceClient::with_socket(&database, &socket);
    let service = start(&database, &socket, &client);
    let command = || ApplicationCommand::PatchConversationConfiguration {
        input: nola_store_port::conversation_profile::PatchConversationConfigurationInput {
            conversation_id: "settings".into(),
            expected_revision: 1,
            patch_json: r#"{"memoryEnabledOverride":true}"#.parse().unwrap(),
            updated_at_ms: 3,
        },
    };
    let before = durable_counts(&database);
    let rejected = client
        .submit_with_revision("unsupported-configuration-envelope", Some(2), command())
        .unwrap();
    match rejected.outcome {
        nola_protocol::CommandReceiptOutcome::Rejected { error, .. } => {
            assert_eq!(error.code, "unsupported_revision_precondition");
        }
        other => panic!("configuration CAS belongs to the command body, not envelope: {other:?}"),
    }
    assert_eq!(durable_counts(&database), before);
    assert!(client
        .command_receipt("unsupported-configuration-envelope")
        .unwrap()
        .is_none());
    let changed: Value = client
        .execute_with_request_id("configuration-request", command())
        .unwrap();
    assert_eq!(changed["status"], "committed");
    assert_eq!(changed["snapshot"]["configurationRevision"], 2);
    let committed = durable_counts(&database);
    drop(service);
    let _service = start(&database, &socket, &client);
    let replay: Value = client
        .execute_with_request_id("configuration-request", command())
        .unwrap();
    assert_eq!(replay, changed);
    assert_eq!(durable_counts(&database), committed);
    let stale: Value = client
        .execute(ApplicationCommand::PatchConversationConfiguration {
            input: nola_store_port::conversation_profile::PatchConversationConfigurationInput {
                conversation_id: "settings".into(),
                expected_revision: 1,
                patch_json: r#"{"memoryEnabledOverride":false}"#.parse().unwrap(),
                updated_at_ms: 4,
            },
        })
        .unwrap();
    assert_eq!(
        stale,
        serde_json::json!({"status":"conflict","conversation_id":"settings","expected_revision":1,"actual_revision":2})
    );
    let window: Value = client
        .execute(ApplicationCommand::LoadConversationProjectionWindowV1 {
            conversation_id: "settings".into(),
            before_ordinal: None,
            limit: 4,
            content_byte_limit: 4096,
        })
        .unwrap();
    assert!(serde_json::to_vec(&window).unwrap().len() < 16_384);
    assert_eq!(
        window["deferred_text_parts"][0]["total_utf8_bytes"],
        10_000_000
    );
}
