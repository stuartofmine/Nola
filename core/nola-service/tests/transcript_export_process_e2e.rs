// SPDX-License-Identifier: Apache-2.0
//! Actual Service IPC archive reads after reopening a canonical reference fixture.
use nola_domain::{
    ActivityState, CanonicalBodyReference, CapabilityId, MessagePartId, MessagePartView,
    StructuredValue, ToolCallId,
};
use nola_protocol::{ExportBodyField, ExportCommand, ExportPage, ExportRange};
use nola_service_client::ApplicationServiceClient;
use std::{
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

struct Service(Child);

fn historical_tool_raw(
    part_id: &str,
    tool_call_id: String,
    input: StructuredValue,
    output: StructuredValue,
) -> String {
    serde_json::to_string(&MessagePartView::Tool {
        execution_source: Default::default(),
        part_id: MessagePartId(part_id.into()),
        revision: 1,
        tool_call_id: ToolCallId(tool_call_id),
        capability_id: CapabilityId("file.read".into()),
        state: ActivityState::Completed,
        display_key: None,
        title: "Historical tool".into(),
        summary: None,
        origin: Default::default(),
        permission: None,
        input,
        output: Some(output),
        output_preview: None,
        full_output: None,
        artifacts: vec![],
        started_at_ms: None,
        finished_at_ms: None,
        affected_resources: vec![],
    }).unwrap()
}

impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn start(database: &Path, socket: &Path) -> (Service, ApplicationServiceClient) {
    let mut service = Service(
        Command::new(env!("CARGO_BIN_EXE_nola-service"))
            .arg("--database")
            .arg(database)
            .arg("--socket")
            .arg(socket)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let client = ApplicationServiceClient::with_socket(database, socket);
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if client.health().is_ok() {
            return (service, client);
        }
        assert!(service.0.try_wait().unwrap().is_none());
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("export service startup timed out");
}

#[test]
fn original_export_crosses_only_bounded_frames_and_survives_service_restart_without_receipts() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("export.db");
    let socket = root.path().join("service.sock");
    // Fixture creation is deliberately offline; this test covers real export
    // IPC, not admission/publication (which has separate transaction coverage).
    drop(nola_store::NolaStore::open(&database).unwrap());
    let mut connection = rusqlite::Connection::open(&database).unwrap();
    let transaction = connection.transaction().unwrap();
    transaction.execute_batch("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','Large original',1,1,2);
        INSERT INTO messages VALUES('m','c',NULL,'user','completed',1,1,2);
        INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,created_at_ms,updated_at_ms)
        VALUES('p','m','text',1,'completed','',1,2);").unwrap();
    let total = 10_000_000_u64;
    let reference = CanonicalBodyReference::for_field(
        "body".into(),
        "c".into(),
        "m".into(),
        "p".into(),
        nola_domain::PartBodyField::ContentText,
        total,
        total.div_ceil(32768) as u32,
    )
    .unwrap();
    let encoded = serde_json::to_string(&reference).unwrap();
    transaction
        .execute(
            "INSERT INTO canonical_text_objects VALUES('body',?1)",
            [&encoded],
        )
        .unwrap();
    transaction
        .execute(
            "INSERT INTO message_part_body_references(part_id,field,reference_json) VALUES('p','content_text',?1)",
            [&encoded],
        )
        .unwrap();
    for index in 0..reference.chunk_count() {
        let offset = u64::from(index) * 32768;
        transaction
            .execute(
                "INSERT INTO canonical_text_chunks VALUES('body',?1,?2)",
                rusqlite::params![
                    offset as i64,
                    "é".repeat(((total - offset).min(32768) / 2) as usize)
                ],
            )
            .unwrap();
    }
    transaction.commit().unwrap();
    let mut branch_store = nola_store::NolaStore::open(&database).unwrap();
    branch_store
        .create_conversation_branch(&nola_domain::CanonicalBranchIntent {
            operation_id: "export-proof".into(),
            source_conversation_id: "c".into(),
            expected_source_revision: 1,
            through_message_id: "m".into(),
            conversation: nola_domain::CanonicalBranchTarget {
                id: "branch".into(),
                title: "Derived".into(),
                settings_json: None,
                created_at_ms: 3,
                updated_at_ms: 3,
            },
            committed_at_ms: 3,
        })
        .unwrap();
    drop(branch_store);

    let receipts_before: i64 = connection
        .query_row("SELECT COUNT(*) FROM command_receipts", [], |row| {
            row.get(0)
        })
        .unwrap();
    let started = Instant::now();
    let (mut service, mut client) = start(&database, &socket);
    let page: Option<ExportPage> = client
        .execute(ExportCommand::LoadConversationExportPage {
            conversation_id: "c".into(),
            after_ordinal: None,
            expected_revision: None,
        })
        .unwrap();
    let page = page.unwrap();
    assert_eq!(page.messages[0].parts[0].content_text_utf8_bytes, total);
    assert!(serde_json::to_vec(&page).unwrap().len() < 2048);
    let branch_page: Option<ExportPage> = client
        .execute(ExportCommand::LoadConversationExportPage {
            conversation_id: "branch".into(),
            after_ordinal: None,
            expected_revision: None,
        })
        .unwrap();
    let branch_page = branch_page.unwrap();
    assert!(branch_page.messages[0].parts[0].has_derivation);
    assert!(!page.messages[0].parts[0].has_derivation);
    let owner = nola_domain::CanonicalPartOwner::new(
        "branch".into(),
        branch_page.messages[0].id.clone(),
        branch_page.messages[0].parts[0].id.clone(),
    )
    .unwrap();
    let proof_command = ExportCommand::ReadConversationPartDerivation {
        owner: owner.clone(),
        expected_conversation_revision: branch_page.conversation.revision,
    };
    let proof: Option<nola_domain::StoredPartDerivation> = client
        .execute_with_request_id("proof-query", proof_command.clone())
        .unwrap();
    let proof = proof.unwrap();
    assert_eq!(
        proof.provenance,
        nola_domain::PartDerivationProvenance::LocalBranch
    );
    assert_eq!(
        proof.derivation.target.fields[0]
            .as_ref()
            .unwrap()
            .total_utf8_bytes,
        total
    );
    assert!(serde_json::to_vec(&proof).unwrap().len() <= 24576);
    assert!(client.command_receipt("proof-query").unwrap().is_none());
    let missing: Option<nola_domain::StoredPartDerivation> = client
        .execute(ExportCommand::ReadConversationPartDerivation {
            owner: nola_domain::CanonicalPartOwner::new("c".into(), "m".into(), "p".into())
                .unwrap(),
            expected_conversation_revision: 1,
        })
        .unwrap();
    assert!(missing.is_none());
    assert!(client
        .execute::<serde_json::Value, _>(ExportCommand::ReadConversationPartDerivation {
            owner: owner.clone(),
            expected_conversation_revision: branch_page.conversation.revision + 1,
        })
        .is_err());
    assert!(client
        .execute::<serde_json::Value, _>(ExportCommand::ReadConversationPartDerivation {
            owner: nola_domain::CanonicalPartOwner::new(
                "c".into(),
                owner.message_id().into(),
                owner.part_id().into()
            )
            .unwrap(),
            expected_conversation_revision: 1,
        })
        .is_err());
    let mut offset = 0;
    let mut largest = 0;
    let mut reads = 0;
    let mut restarted = false;
    while offset < total {
        if offset > total / 2 && !restarted {
            drop(service);
            (service, client) = start(&database, &socket);
            restarted = true;
        }
        let range: ExportRange = client
            .execute(ExportCommand::ReadConversationExportRange {
                conversation_id: "c".into(),
                message_id: "m".into(),
                part_id: "p".into(),
                field: ExportBodyField::ContentText,
                expected_revision: 1,
                offset_bytes: offset,
                limit_bytes: 32768,
            })
            .unwrap();
        assert_eq!(range.text, "é".repeat(range.text.len() / 2));
        assert_eq!(range.offset_bytes, offset);
        assert_eq!(range.total_utf8_bytes, total);
        assert_eq!(range.end_of_content, range.next_offset_bytes == total);
        assert!(range.next_offset_bytes > offset && range.text.len() <= 32768);
        largest = largest.max(serde_json::to_vec(&range).unwrap().len());
        offset = range.next_offset_bytes;
        reads += 1;
    }
    assert_eq!(offset, total);
    assert!(reads > 300 && largest < 200_000);
    assert!(client
        .execute::<ExportRange, _>(ExportCommand::ReadConversationExportRange {
            conversation_id: "c".into(),
            message_id: "m".into(),
            part_id: "p".into(),
            field: ExportBodyField::ContentText,
            expected_revision: 2,
            offset_bytes: 0,
            limit_bytes: 32768,
        })
        .is_err());
    drop(service);
    // Deleted source identities survive in immutable claims; only target
    // visibility/bytes are necessary to validate an export after reopening.
    connection
        .execute("DELETE FROM conversations WHERE id='c'", [])
        .unwrap();
    (service, client) = start(&database, &socket);
    let reopened: Option<nola_domain::StoredPartDerivation> = client
        .execute_with_request_id("proof-query", proof_command.clone())
        .unwrap();
    assert_eq!(reopened, Some(proof.clone()));
    drop(service);
    connection.execute("INSERT INTO conversation_projection_operations VALUES('hide-proof','branch','hide',NULL,99,1,5)", []).unwrap();
    connection.execute("INSERT INTO message_projection_events VALUES('hidden-proof','hide-proof','branch',?1,0,'fixture',5)", [owner.message_id()]).unwrap();
    (service, client) = start(&database, &socket);
    assert!(client
        .execute::<serde_json::Value, _>(proof_command.clone())
        .is_err());
    drop(service);
    connection
        .execute(
            "DELETE FROM conversation_projection_operations WHERE operation_id='hide-proof'",
            [],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE message_parts SET state='failed' WHERE id=?1",
            [owner.part_id()],
        )
        .unwrap();
    (service, client) = start(&database, &socket);
    assert!(
        client
            .execute::<serde_json::Value, _>(proof_command)
            .is_err(),
        "current target metadata must match claim"
    );
    drop(service);
    let receipts_after: i64 = connection
        .query_row("SELECT COUNT(*) FROM command_receipts", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(
        receipts_before, receipts_after,
        "bounded archive queries must not journal entire bodies"
    );
    eprintln!("export original_bytes={total} range_reads={reads} largest_result_bytes={largest} elapsed_ms={} restart=true",started.elapsed().as_millis());
}

#[test]
fn mixed_part_metadata_round_trips_export_to_atomic_archive_restore_across_service_restarts() {
    use nola_protocol::archive_restore::*;
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("metadata.db");
    let socket = root.path().join("metadata.sock");
    drop(nola_store::NolaStore::open(&database).unwrap());
    let db = rusqlite::Connection::open(&database).unwrap();
    db.execute_batch("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','Mixed originals',1,100,900);
        INSERT INTO messages VALUES('m','c',NULL,'assistant','completed',1,110,890);
        INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,content_json,content_digest,created_at_ms,updated_at_ms) VALUES
        ('text','m','text',1,'completed','Original text',NULL,'text-digest-exact',120,180),
        ('semantic','m','tool_result',2,'completed','Source result','{\"source_uri\":\"file:///evidence\",\"result\":\"中😀\"}','semantic-digest-exact',300,800)").unwrap();
    drop(db);
    let (service, client) = start(&database, &socket);
    let original: ExportPage = client
        .execute::<Option<ExportPage>, _>(ExportCommand::LoadConversationExportPage {
            conversation_id: "c".into(),
            after_ordinal: None,
            expected_revision: None,
        })
        .unwrap()
        .unwrap();
    let mut parts = Vec::new();
    for part in &original.messages[0].parts {
        let read = |field| {
            client
                .execute::<ExportRange, _>(ExportCommand::ReadConversationExportRange {
                    conversation_id: "c".into(),
                    message_id: "m".into(),
                    part_id: part.id.clone(),
                    field,
                    expected_revision: original.conversation.revision,
                    offset_bytes: 0,
                    limit_bytes: 32768,
                })
                .unwrap()
                .text
        };
        parts.push(nola_domain::MessagePartInput {
            id: part.id.clone(),
            kind: part.kind.clone(),
            ordinal: part.ordinal,
            state: part.state.clone(),
            content_text: read(ExportBodyField::ContentText),
            content_json: part
                .content_json_utf8_bytes
                .map(|_| read(ExportBodyField::ContentJson)),
            content_digest: part.content_digest.clone(),
            created_at_ms: part.created_at_ms,
            updated_at_ms: part.updated_at_ms,
        });
    }
    assert_eq!((parts[0].created_at_ms, parts[0].updated_at_ms), (120, 180));
    assert_eq!((parts[1].created_at_ms, parts[1].updated_at_ms), (300, 800));
    assert_eq!(
        parts[1].content_digest.as_deref(),
        Some("semantic-digest-exact")
    );
    let revision: u64 = client
        .execute(ArchiveRestoreCommand::ArchiveDirectoryRevision {})
        .unwrap();
    let _: ArchiveRestoreProgress = client
        .execute(ArchiveRestoreCommand::BeginArchiveRestore {
            input: BeginArchiveRestore {
                restore_id: "metadata".into(),
                expected_directory_revision: revision,
                created_at_ms: 1000,
            },
        })
        .unwrap();
    let header = &original.conversation;
    let _: ArchiveRestoreProgress = client
        .execute(ArchiveRestoreCommand::StageArchiveManifest {
            input: StageArchiveManifest {
                restore_id: "metadata".into(),
                sequence: 0,
                entry: ArchiveRestoreManifestEntry::Profile {
                    profile: ArchiveConversationProfile {
                        id: header.id.clone(),
                        title: header.title.clone(),
                        settings_json: header.settings_json.clone(),
                        created_at_ms: header.created_at_ms,
                        updated_at_ms: header.updated_at_ms,
                    },
                },
            },
        })
        .unwrap();
    let message = &original.messages[0];
    let progress: ArchiveRestoreProgress = client
        .execute(ArchiveRestoreCommand::StageArchiveManifest {
            input: StageArchiveManifest {
                restore_id: "metadata".into(),
                sequence: 1,
                entry: ArchiveRestoreManifestEntry::Message {
                    conversation_id: "c".into(),
                    message: ArchiveMessage {
                        id: message.id.clone(),
                        run_id: message.run_id.clone(),
                        role: message.role.clone(),
                        status: message.status.clone(),
                        ordinal: message.ordinal,
                        created_at_ms: message.created_at_ms,
                        completed_at_ms: message.completed_at_ms,
                        parts: parts.clone(),
                    },
                    uploads: vec![],
                },
            },
        })
        .unwrap();
    drop(service);
    let (service, client) = start(&database, &socket);
    let _: ArchiveRestoreResult = client
        .execute(ArchiveRestoreCommand::CommitArchiveRestore {
            input: CommitArchiveRestore {
                restore_id: "metadata".into(),
                expected_directory_revision: revision,
                expected_entry_count: progress.next_sequence,
                manifest_digest: progress.manifest_digest,
                committed_at_ms: 1000,
            },
        })
        .unwrap();
    drop(service);
    let (_service, client) = start(&database, &socket);
    let restored: ExportPage = client
        .execute::<Option<ExportPage>, _>(ExportCommand::LoadConversationExportPage {
            conversation_id: "c".into(),
            after_ordinal: None,
            expected_revision: None,
        })
        .unwrap()
        .unwrap();
    assert_eq!(restored.messages, original.messages);
    assert!(restored.conversation.revision > original.conversation.revision);
    for part in &parts {
        for (field, body) in [
            (ExportBodyField::ContentText, Some(&part.content_text)),
            (ExportBodyField::ContentJson, part.content_json.as_ref()),
        ] {
            if let Some(body) = body {
                let range: ExportRange = client
                    .execute(ExportCommand::ReadConversationExportRange {
                        conversation_id: "c".into(),
                        message_id: "m".into(),
                        part_id: part.id.clone(),
                        field,
                        expected_revision: restored.conversation.revision,
                        offset_bytes: 0,
                        limit_bytes: 32768,
                    })
                    .unwrap();
                assert_eq!(&range.text, body);
            }
        }
    }
}

#[test]
fn historical_ten_megabyte_json_window_is_bounded_and_stable_after_service_restart() {
    use nola_application::{ApplicationCommand, ConversationProjectionWindowV1};
    use nola_domain::{HistoricalProjectionStatus, MessagePartView, SemanticValue};
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("historical.db");
    let socket = root.path().join("history.sock");
    drop(nola_store::NolaStore::open(&database).unwrap());
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection.execute_batch("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','History',1,1,1);
        INSERT INTO messages VALUES('m','c',NULL,'assistant','completed',1,1,1);
        INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,content_json,created_at_ms,updated_at_ms)
        VALUES('p','m','tool',1,'completed','',NULL,1,1);").unwrap();
    let raw = historical_tool_raw(
        "p",
        "call".into(),
        StructuredValue::Object(Default::default()),
        StructuredValue::String("x".repeat(10_000_000)),
    );
    connection
        .execute("UPDATE message_parts SET content_json=?1", [&raw])
        .unwrap();
    drop(connection);
    let query = || ApplicationCommand::LoadConversationProjectionWindowV1 {
        conversation_id: "c".into(),
        before_ordinal: None,
        limit: 32,
        content_byte_limit: 1024,
    };
    let (service, client) = start(&database, &socket);
    let window: ConversationProjectionWindowV1 = client.execute(query()).unwrap();
    let encoded = serde_json::to_vec(&window).unwrap();
    assert!(encoded.len() < 16384);
    let MessagePartView::Historical {
        part_id,
        projection,
        ..
    } = &window.projection.messages[0].parts[0]
    else {
        panic!("historical JSON became a live tool")
    };
    assert_eq!(part_id.0, "p");
    assert_eq!(projection.status, HistoricalProjectionStatus::Deferred);
    let Some(SemanticValue::Deferred { locator }) = projection.fields.get("output") else {
        panic!("missing raw locator")
    };
    let original: ExportRange = client
        .execute(ExportCommand::ReadConversationExportRange {
            conversation_id: "c".into(),
            message_id: "m".into(),
            part_id: "p".into(),
            field: ExportBodyField::ContentJson,
            expected_revision: 1,
            offset_bytes: locator.end_bytes() - 8,
            limit_bytes: 4,
        })
        .unwrap();
    assert_eq!(
        original.text,
        &raw[locator.end_bytes() as usize - 8..locator.end_bytes() as usize - 4]
    );
    drop(service);
    let (_service, client) = start(&database, &socket);
    let reopened: ConversationProjectionWindowV1 = client.execute(query()).unwrap();
    assert_eq!(serde_json::to_vec(&reopened).unwrap(), encoded);
    let connection = rusqlite::Connection::open(&database).unwrap();
    let receipts: i64 = connection
        .query_row("SELECT COUNT(*) FROM command_receipts", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(receipts, 0);
}

#[test]
fn deferred_history_is_loaded_individually_over_ipc_without_mutating_the_window_or_receipts() {
    use nola_application::{ApplicationCommand, ConversationProjectionWindowV1};
    use nola_domain::{
        CanonicalPartOwner, HistoricalPartProjection, HistoricalProjectionStatus, MessagePartView,
    };
    use nola_protocol::HistoricalProjectionCommand;
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("history-budget.db");
    let socket = root.path().join("history-budget.sock");
    drop(nola_store::NolaStore::open(&database).unwrap());
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection
        .execute_batch(
            "INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','History',1,1,1);
        INSERT INTO messages VALUES('m','c',NULL,'assistant','completed',1,1,1);",
        )
        .unwrap();
    for index in 0..64 {
        let id = format!("p{index}");
        let raw = historical_tool_raw(
            &id,
            format!("call{index}"),
            StructuredValue::Object(std::collections::BTreeMap::from([
                ("path".into(), StructuredValue::String("x".repeat(1024))),
            ])),
            StructuredValue::Object(std::collections::BTreeMap::from([
                ("ok".into(), StructuredValue::Bool(true)),
            ])),
        );
        connection.execute("INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,content_json,created_at_ms,updated_at_ms)
            VALUES(?1,'m','tool',?2,'completed','',?3,1,1)",rusqlite::params![id,index+1,raw]).unwrap();
    }
    drop(connection);
    let query = || ApplicationCommand::LoadConversationProjectionWindowV1 {
        conversation_id: "c".into(),
        before_ordinal: None,
        limit: 32,
        content_byte_limit: 1024,
    };
    let (service, client) = start(&database, &socket);
    let window: ConversationProjectionWindowV1 = client.execute(query()).unwrap();
    assert_eq!(window.projection.messages[0].parts.len(), 64);
    let part = window.projection.messages[0]
        .parts
        .iter()
        .find_map(|part| match part {
            MessagePartView::Historical { projection, .. }
                if matches!(
                    projection.status,
                    HistoricalProjectionStatus::NotLoaded { .. }
                ) =>
            {
                Some(projection)
            }
            _ => None,
        })
        .expect("shared selection budget must produce an explicit not-loaded Part");
    assert!(part.source.is_none() && part.binding.is_none() && part.fields.is_empty());
    let read =
        |owner: CanonicalPartOwner, revision| HistoricalProjectionCommand::ReadHistoricalPart {
            owner,
            expected_conversation_revision: revision,
            maximum_output_bytes: 8192,
        };
    let loaded: Option<HistoricalPartProjection> =
        client.execute(read(part.owner.clone(), 1)).unwrap();
    let loaded = loaded.unwrap();
    assert_eq!(loaded.status, HistoricalProjectionStatus::Supported);
    assert_eq!(loaded.owner, part.owner);
    assert!(serde_json::to_vec(&loaded).unwrap().len() <= 8192);
    assert!(client
        .execute::<Option<HistoricalPartProjection>, _>(read(part.owner.clone(), 2))
        .is_err());
    let wrong =
        CanonicalPartOwner::new("c".into(), "foreign".into(), part.owner.part_id().into()).unwrap();
    assert!(client
        .execute::<Option<HistoricalPartProjection>, _>(read(wrong, 1))
        .is_err());
    let after: ConversationProjectionWindowV1 = client.execute(query()).unwrap();
    assert_eq!(
        serde_json::to_value(&after).unwrap(),
        serde_json::to_value(&window).unwrap()
    );
    drop(service);
    let (_service, client) = start(&database, &socket);
    let reopened: Option<HistoricalPartProjection> =
        client.execute(read(part.owner.clone(), 1)).unwrap();
    assert_eq!(reopened, Some(loaded));
    let connection = rusqlite::Connection::open(&database).unwrap();
    let receipts: i64 = connection
        .query_row("SELECT COUNT(*) FROM command_receipts", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(receipts, 0);
}
