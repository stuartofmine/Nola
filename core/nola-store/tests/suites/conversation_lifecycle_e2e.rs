// SPDX-License-Identifier: Apache-2.0
//! Consolidated conversation lifecycle suite merging `conversation_configuration_e2e`,
//! `conversation_metadata_patch_e2e`, `conversation_resource_e2e`, `conversation_deletion_e2e`,
//! and `conversation_sync_e2e` into dense multi-scenario Store tests.

use std::sync::{Arc, Barrier};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use crate::support::{conversation_deletion_fixture, outbox as outbox_fixture};
use nola_store::{BeginTurnInput, NolaStore, OutboxEffectInput, StoredConversation};
use nola_store_port::{
    conversation_deletion::ConversationDeletionRepository,
    conversation_profile::{
        CreateConversationInput, PatchConversationConfigurationInput,
        PatchConversationMetadataInput,
    },
    conversation_resource::{
        ConversationResourceRepository, PutConversationPdfResource, MAX_PDF_RESOURCE_BYTES,
    },
};
use rusqlite::{params, Connection};
use serde_json::{json, Value};

fn committed_config(
    result: nola_domain::conversation_configuration::ConfigurationCommitResult,
) -> Value {
    serde_json::to_value(result.into_committed().expect("configuration committed")).unwrap()
}

fn pdf_bytes() -> Vec<u8> {
    b"%PDF-1.4\n1 0 obj <<>> endobj\n%%EOF\n".to_vec()
}

fn pdf_input(
    conversation_id: &str,
    resource_id: String,
    bytes: &[u8],
) -> PutConversationPdfResource {
    PutConversationPdfResource {
        conversation_id: conversation_id.into(),
        resource_id,
        display_name: "报告.pdf".into(),
        mime_type: "application/pdf".into(),
        data_base64: STANDARD.encode(bytes),
        created_at_ms: 1,
    }
}

fn create_conv(store: &mut NolaStore, id: &str) {
    store
        .create_conversation(&CreateConversationInput {
            id: id.into(),
            title: id.into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
}

#[test]
fn conversation_configuration_and_metadata_patches_enforce_sparse_cas_validation_and_reopen() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("conversation-config.sqlite");
    let mut store = NolaStore::open(&database).unwrap();

    // 1. Shared knowledge sparse patch, CAS conflict, native field preservation, and clear.
    let initial_knowledge = json!({
        "isPinned": true, "permissionModeRawValue": "cautious", "nativeField": "keep",
        "modelOverrides": {"configured:m": {"temperature": 0.5}},
        "knowledge": {"keyPointsMarkdown": "old facts", "wikiMarkdown": "# Keep\n", "updatedAt": 0},
        "conversation": {"knowledgePoints": "preserve owned data"}
    });
    store
        .create_conversation(&CreateConversationInput {
            id: "knowledge".into(),
            title: "Knowledge".into(),
            revision: 1,
            settings_json: Some(initial_knowledge.to_string()),
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    let kp_input = PatchConversationConfigurationInput {
        conversation_id: "knowledge".into(),
        expected_revision: 1,
        patch_json: json!({"knowledge":{"keyPointsMarkdown":"  事实\n\n"}})
            .to_string()
            .parse()
            .unwrap(),
        updated_at_ms: 978_307_201_500,
    };
    let kp_receipt = committed_config(store.patch_conversation_configuration(&kp_input).unwrap());
    assert_eq!(kp_receipt["configurationRevision"], 2);
    assert_eq!(
        kp_receipt["knowledge"],
        json!({"keyPointsMarkdown":"  事实\n\n", "wikiMarkdown":"# Keep\n", "updatedAt":1.5})
    );
    assert!(store
        .patch_conversation_configuration(&kp_input)
        .unwrap()
        .is_conflict());
    let settings: Value =
        serde_json::from_str(&store.load_conversation_settings("knowledge").unwrap().unwrap())
            .unwrap();
    for key in [
        "isPinned",
        "permissionModeRawValue",
        "nativeField",
        "modelOverrides",
        "conversation",
    ] {
        assert_eq!(settings[key], initial_knowledge[key]);
    }
    let kp_clear = PatchConversationConfigurationInput {
        expected_revision: 2,
        patch_json: json!({"knowledge":{"wikiMarkdown":""}})
            .to_string()
            .parse()
            .unwrap(),
        updated_at_ms: 978_307_202_000,
        ..kp_input
    };
    assert!(store
        .patch_conversation_configuration(&kp_clear)
        .unwrap()
        .is_committed());

    // 2. Invalid knowledge patch rollback + absent document initialization + malformed stored knowledge guard.
    store
        .create_conversation(&CreateConversationInput {
            id: "knowledge-empty".into(),
            title: "Knowledge".into(),
            revision: 1,
            settings_json: Some("{}".into()),
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    for bad in [
        json!({}),
        Value::Null,
        json!({"wikiMarkdown":null}),
        json!({"wikiMarkdown":42}),
        json!({"knowledgePoints":"alias"}),
        json!({"wikiMarkdown":"text", "updatedAt":1}),
    ] {
        assert!(json!({"knowledge":bad})
            .to_string()
            .parse::<nola_domain::configuration_patch::ConversationConfigurationPatch>()
            .is_err());
        assert_eq!(
            store
                .load_conversation_settings("knowledge-empty")
                .unwrap()
                .unwrap(),
            "{}"
        );
    }
    let init_kp = PatchConversationConfigurationInput {
        conversation_id: "knowledge-empty".into(),
        expected_revision: 1,
        patch_json: json!({"knowledge":{"wikiMarkdown":"new"}})
            .to_string()
            .parse()
            .unwrap(),
        updated_at_ms: 978_307_200_000,
    };
    let init_receipt = committed_config(store.patch_conversation_configuration(&init_kp).unwrap());
    assert_eq!(
        init_receipt["knowledge"],
        json!({"keyPointsMarkdown":"", "wikiMarkdown":"new", "updatedAt":0.0})
    );

    for (index, malformed) in [
        json!("not a document"),
        json!({"keyPointsMarkdown":"missing wiki"}),
        json!({"keyPointsMarkdown":"facts", "wikiMarkdown":"wiki", "updatedAt":"wrong timestamp"}),
    ]
    .into_iter()
    .enumerate()
    {
        let id = format!("invalid-{index}");
        let raw = json!({"knowledge":malformed,"isPinned":true}).to_string();
        store
            .create_conversation(&CreateConversationInput {
                id: id.clone(),
                title: "Invalid knowledge".into(),
                revision: 1,
                settings_json: Some(raw.clone()),
                created_at_ms: 1,
                updated_at_ms: 1,
            })
            .unwrap();
        let patch = PatchConversationConfigurationInput {
            conversation_id: id.clone(),
            expected_revision: 1,
            patch_json: json!({"knowledge":{"wikiMarkdown":"must not replace"}, "isPinned":false})
                .to_string()
                .parse()
                .unwrap(),
            updated_at_ms: 2,
        };
        assert!(store.patch_conversation_configuration(&patch).is_err());
        assert_eq!(store.load_conversation_settings(&id).unwrap().unwrap(), raw);
        let unrelated = PatchConversationConfigurationInput {
            patch_json: json!({"isPinned":false}).to_string().parse().unwrap(),
            ..patch
        };
        assert!(store
            .patch_conversation_configuration(&unrelated)
            .unwrap()
            .is_committed());
    }

    // 3. Memory & retrieval overrides, pinning, seed intent, and session modelOverrides sparse patch.
    for (idx, key) in ["memoryEnabledOverride", "retrievalEnabledOverride"]
        .into_iter()
        .enumerate()
    {
        let conv_id = format!("memory-{idx}");
        store
            .create_conversation(&CreateConversationInput {
                id: conv_id.clone(),
                title: "Memory".into(),
                revision: 1,
                settings_json: Some(json!({"isPinned":true,"nativeField":"keep"}).to_string()),
                created_at_ms: 1,
                updated_at_ms: 1,
            })
            .unwrap();
        for (step, value) in [json!(true), json!(false), Value::Null]
            .into_iter()
            .enumerate()
        {
            let rev = step as i64 + 1;
            let patch = PatchConversationConfigurationInput {
                conversation_id: conv_id.clone(),
                expected_revision: rev,
                patch_json: json!({(key):value}).to_string().parse().unwrap(),
                updated_at_ms: rev + 1,
            };
            let receipt = committed_config(store.patch_conversation_configuration(&patch).unwrap());
            assert_eq!(receipt[key], value);
            assert_eq!(receipt["configurationRevision"], rev + 1);
            assert!(store
                .patch_conversation_configuration(&patch)
                .unwrap()
                .is_conflict());
            let saved: Value = serde_json::from_str(
                &store.load_conversation_settings(&conv_id).unwrap().unwrap(),
            )
            .unwrap();
            assert_eq!(saved["nativeField"], "keep");
            assert_eq!(saved["isPinned"], true);
            assert_eq!(saved.get(key).is_none(), value.is_null());
        }
        assert!(json!({(key):"true"})
            .to_string()
            .parse::<nola_domain::configuration_patch::ConversationConfigurationPatch>()
            .is_err());
    }

    store
        .create_conversation(&CreateConversationInput {
            id: "pin".into(),
            title: "Pinned".into(),
            revision: 1,
            settings_json: Some(
                json!({
                    "permissionModeRawValue":"cautious",
                    "modelOverrides":{"configured:m":{"temperature":0.5}},
                    "nativeField":"keep"
                })
                .to_string(),
            ),
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    for (rev, pinned) in [(1, true), (2, false)] {
        let patch = PatchConversationConfigurationInput {
            conversation_id: "pin".into(),
            expected_revision: rev,
            patch_json: json!({"isPinned":pinned}).to_string().parse().unwrap(),
            updated_at_ms: rev + 1,
        };
        let receipt = committed_config(store.patch_conversation_configuration(&patch).unwrap());
        assert_eq!(receipt["isPinned"], pinned);
        assert!(store
            .patch_conversation_configuration(&patch)
            .unwrap()
            .is_conflict());
    }

    store
        .create_conversation(&CreateConversationInput {
            id: "seed".into(),
            title: "Seed".into(),
            revision: 1,
            settings_json: Some("{}".into()),
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    for (index, seed) in [
        json!({"kind":"value","value":"9223372036854775807"}),
        json!({"kind":"clear"}),
        json!(null),
    ]
    .into_iter()
    .enumerate()
    {
        let rev = index as i64 + 1;
        let patch = PatchConversationConfigurationInput {
            conversation_id: "seed".into(),
            expected_revision: rev,
            patch_json: json!({"modelOverrides":{"configured:model":{"seed":seed}}})
                .to_string()
                .parse()
                .unwrap(),
            updated_at_ms: rev + 1,
        };
        assert!(store
            .patch_conversation_configuration(&patch)
            .unwrap()
            .is_committed());
        assert!(store
            .patch_conversation_configuration(&patch)
            .unwrap()
            .is_conflict());
        let saved: Value =
            serde_json::from_str(&store.load_conversation_settings("seed").unwrap().unwrap())
                .unwrap();
        let fields = &saved["modelOverrides"]["configured:model"];
        if seed.is_null() {
            assert!(fields.get("seed").is_none());
        } else {
            assert_eq!(fields["seed"], seed);
        }
    }

    store
        .create_conversation(&CreateConversationInput {
            id: "c".into(),
            title: "Conversation".into(),
            revision: 1,
            settings_json: Some(
                json!({
                    "nativeOnly":{"keep":true},
                    "modelOverrides":{"configured:a":{"temperature":0.7},"configured:b":{"seed":7}},
                    "owner":"web","conversation":{"conversationId":"c"}
                })
                .to_string(),
            ),
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    let session_patch = PatchConversationConfigurationInput {
        conversation_id: "c".into(),
        expected_revision: 1,
        patch_json: json!({
            "permissionProfile":"cautious",
            "modelOverrides":{"configured:a":{"temperature":0,"topP":0.2}}
        })
        .to_string()
        .parse()
        .unwrap(),
        updated_at_ms: 2,
    };
    let receipt = committed_config(store.patch_conversation_configuration(&session_patch).unwrap());
    assert_eq!(receipt["configurationRevision"], 2);
    assert_eq!(receipt["modelOverrides"]["configured:a"]["temperature"], 0);
    assert_eq!(receipt["modelOverrides"]["configured:b"]["seed"], 7);
    let outbox_len = store.pending_sync_changes(100).unwrap().len();
    assert!(store
        .patch_conversation_configuration(&session_patch)
        .unwrap()
        .is_conflict());
    assert_eq!(store.pending_sync_changes(100).unwrap().len(), outbox_len);
    store.rename_conversation("c", "Renamed", None, 3).unwrap();
    let clear_overrides = PatchConversationConfigurationInput {
        expected_revision: 2,
        patch_json: json!({
            "permissionProfile":null,
            "modelOverrides":{"configured:a":{"temperature":null},"configured:b":null}
        })
        .to_string()
        .parse()
        .unwrap(),
        ..session_patch
    };
    let receipt =
        committed_config(store.patch_conversation_configuration(&clear_overrides).unwrap());
    assert!(receipt["permissionProfile"].is_null());
    assert!(receipt["modelOverrides"]["configured:a"]
        .get("temperature")
        .is_none());
    assert_eq!(receipt["modelOverrides"]["configured:a"]["topP"], 0.2);
    assert!(receipt["modelOverrides"].get("configured:b").is_none());

    // 4. Cross-connection metadata patch CAS & invalid metadata rejection.
    store
        .create_conversation(&CreateConversationInput {
            id: "shared".into(),
            title: "Shared".into(),
            revision: 1,
            settings_json: Some(
                json!({
                    "isPinned": false, "permissionModeRawValue": "cautious",
                    "knowledge": {"wikiMarkdown": "old"}, "conversation": {"keep": true}
                })
                .to_string(),
            ),
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    store
        .create_conversation(&CreateConversationInput {
            id: "meta-invalid".into(),
            title: "Invalid".into(),
            revision: 1,
            settings_json: Some("{}".into()),
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    let mut bad_edit = PatchConversationMetadataInput {
        conversation_id: "meta-invalid".into(),
        expected_revision: 1,
        patch_json: "{}".into(),
        updated_at_ms: 2,
    };
    for raw in [
        "{}",
        "null",
        "[]",
        "false",
        "broken",
        "{\"modelOverrides\":null}",
        "{\"permissionModeRawValue\":\"allow_all\"}",
        "{\"sessionMemory\":\"text\"}",
    ] {
        bad_edit.patch_json = raw.into();
        assert!(store.patch_conversation_metadata(&bad_edit).is_err());
        assert_eq!(
            store
                .load_conversation_settings("meta-invalid")
                .unwrap()
                .unwrap(),
            "{}"
        );
    }
    bad_edit.patch_json = json!({"isPinned": true}).to_string();
    bad_edit.expected_revision = 0;
    assert!(store.patch_conversation_metadata(&bad_edit).is_err());
    bad_edit.expected_revision = 1;
    assert_eq!(store.patch_conversation_metadata(&bad_edit).unwrap(), Some(2));
    assert!(serde_json::from_value::<PatchConversationMetadataInput>(json!({
        "conversation_id": "meta-invalid", "expected_revision": 2, "patch_json": "{}",
        "updated_at_ms": 3, "legacy": true
    }))
    .is_err());

    let mut web = NolaStore::open(&database).unwrap();
    let mut edit = PatchConversationMetadataInput {
        conversation_id: "shared".into(),
        expected_revision: 1,
        patch_json: json!({"clientNote": "新知识\n"}).to_string(),
        updated_at_ms: 2,
    };
    assert_eq!(web.patch_conversation_metadata(&edit).unwrap(), Some(2));
    edit.patch_json = json!({"isPinned": true}).to_string();
    assert_eq!(store.patch_conversation_metadata(&edit).unwrap(), None);
    edit.expected_revision = 2;
    assert_eq!(store.patch_conversation_metadata(&edit).unwrap(), Some(3));
    edit.conversation_id = "deleted".into();
    assert_eq!(web.patch_conversation_metadata(&edit).unwrap(), None);

    drop(web);
    drop(store);
    let reopened = NolaStore::open(&database).unwrap();
    let kp_saved: Value =
        serde_json::from_str(&reopened.load_conversation_settings("knowledge").unwrap().unwrap())
            .unwrap();
    assert_eq!(kp_saved["knowledge"]["keyPointsMarkdown"], "  事实\n\n");
    assert_eq!(kp_saved["knowledge"]["wikiMarkdown"], "");
    assert_eq!(kp_saved["knowledge"]["updatedAt"], 2.0);
    let shared_saved: Value =
        serde_json::from_str(&reopened.load_conversation_settings("shared").unwrap().unwrap())
            .unwrap();
    assert_eq!(shared_saved["isPinned"], true);
    assert_eq!(shared_saved["permissionModeRawValue"], "cautious");
    assert_eq!(shared_saved["knowledge"], json!({"wikiMarkdown": "old"}));
    assert_eq!(shared_saved["clientNote"], "新知识\n");
    assert_eq!(shared_saved["conversation"], json!({"keep": true}));
}

#[test]
fn conversation_pdf_resources_enforce_immutability_validation_cascade_and_concurrent_quota() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("resources.sqlite");
    let mut store = NolaStore::open(&database).unwrap();
    for id in ["one", "two", "quota"] {
        create_conv(&mut store, id);
    }

    // 1. Immutability, conversation scoping, and cascade delete on "one" / "two".
    let valid_pdf = pdf_input("one", uuid::Uuid::new_v4().to_string(), &pdf_bytes());
    let descriptor = store
        .put_conversation_pdf_resource(&valid_pdf)
        .unwrap()
        .unwrap();
    assert_eq!(descriptor.byte_count, pdf_bytes().len() as u64);
    assert_eq!(
        store
            .put_conversation_pdf_resource(&valid_pdf)
            .unwrap()
            .unwrap(),
        descriptor
    );
    assert!(store
        .read_conversation_resource("two", &valid_pdf.resource_id)
        .unwrap()
        .is_none());
    assert!(!store
        .delete_conversation_resource("two", &valid_pdf.resource_id)
        .unwrap());
    let read = store
        .read_conversation_resource("one", &valid_pdf.resource_id)
        .unwrap()
        .unwrap();
    assert_eq!(STANDARD.decode(read.data_base64).unwrap(), pdf_bytes());
    assert_eq!(
        store.list_conversation_resources("one").unwrap().unwrap(),
        vec![descriptor]
    );
    assert!(store
        .list_conversation_resources("two")
        .unwrap()
        .unwrap()
        .is_empty());
    let mut changed = valid_pdf.clone();
    changed.data_base64 = STANDARD.encode(b"%PDF-1.4\nother\n%%EOF\n");
    assert!(store.put_conversation_pdf_resource(&changed).is_err());

    // 2. Spoofing, oversize, path traversal, wrong MIME, and missing owner rejection on "two".
    let bad_id = uuid::Uuid::new_v4().to_string();
    for bytes in [
        b"not a PDF\n%%EOF\n".to_vec(),
        b"%PDF-1.4\nmissing trailer".to_vec(),
        vec![b'x'; MAX_PDF_RESOURCE_BYTES + 1],
    ] {
        assert!(store
            .put_conversation_pdf_resource(&pdf_input("two", bad_id.clone(), &bytes))
            .is_err());
    }
    let mut bad_meta = pdf_input("two", bad_id.clone(), &pdf_bytes());
    bad_meta.display_name = "../report.pdf".into();
    assert!(store.put_conversation_pdf_resource(&bad_meta).is_err());
    bad_meta.display_name = "report.pdf".into();
    bad_meta.mime_type = "text/html".into();
    assert!(store.put_conversation_pdf_resource(&bad_meta).is_err());
    assert!(store
        .put_conversation_pdf_resource(&pdf_input("missing", bad_id, &pdf_bytes()))
        .unwrap()
        .is_none());

    assert!(store.delete_conversation("one").unwrap());
    assert!(store
        .read_conversation_resource("one", &valid_pdf.resource_id)
        .unwrap()
        .is_none());
    drop(store);

    // 3. Concurrent 12-worker PDF upload enforces transactional cap of 10 on "quota".
    let barrier = Arc::new(Barrier::new(12));
    let workers = (0..12)
        .map(|_| {
            let barrier = Arc::clone(&barrier);
            let db = database.clone();
            std::thread::spawn(move || {
                let mut conn = NolaStore::open(db).unwrap();
                let item = pdf_input("quota", uuid::Uuid::new_v4().to_string(), &pdf_bytes());
                barrier.wait();
                conn.put_conversation_pdf_resource(&item).is_ok()
            })
        })
        .collect::<Vec<_>>();
    let successes = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .filter(|ok| *ok)
        .count();
    assert_eq!(successes, 10);
    let connection = Connection::open(&database).unwrap();
    let count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM conversation_pdf_resources WHERE conversation_id='quota'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 10);
}

#[test]
fn conversation_deletion_guards_tombstones_and_sync_projections_survive_cold_reopen() {
    use nola_domain::agent_room::{AgentRoom, RoomParticipant};

    fn publications_for(
        connection: &Connection,
        entity_id: &str,
    ) -> Vec<(String, i64, bool, String, String)> {
        let mut statement = connection
            .prepare(
                "SELECT operation_id,revision,deleted,payload_json,status
                 FROM sync_outbox WHERE entity_type='conversation' AND entity_id=?1
                 ORDER BY local_sequence",
            )
            .unwrap();
        statement
            .query_map([entity_id], |row| {
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

    fn assert_tombstone_for(connection: &Connection, entity_id: &str) {
        let rows = publications_for(connection, entity_id);
        let pending: Vec<_> = rows.iter().filter(|row| row.4 == "pending").collect();
        assert_eq!(pending.len(), 1);
        let row = pending[0];
        assert!(!row.0.is_empty());
        assert_eq!((row.1, row.2, row.3.as_str()), (2, true, "null"));
        let tracked: (i64, bool, String) = connection
            .query_row(
                "SELECT revision,deleted,operation_id FROM sync_entity_revisions
                 WHERE entity_type='conversation' AND entity_id=?1",
                [entity_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(tracked, (2, true, row.0.clone()));
    }

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("deletion-and-sync.db");
    let mut store = NolaStore::open(&path).unwrap();
    let fixture = Connection::open(&path).unwrap();

    // 1. Room membership prevents generic deletion even after deactivation.
    create_conv(&mut store, "room-conv");
    let room = AgentRoom::new(
        "room".into(),
        "Shared".into(),
        vec![RoomParticipant {
            id: "member".into(),
            agent_id: "agent".to_string().try_into().unwrap(),
            agent_revision: 1,
            display_name: "Agent".into(),
            description: "Research".into(),
            context_conversation_id: "room-conv".into(),
            active: true,
        }],
    )
    .unwrap();
    for room_state in [room.clone(), room.deactivate("member").unwrap()] {
        store
            .put_product_setting(
                "agent_chat_room/room",
                &serde_json::to_string(&room_state).unwrap(),
                1,
            )
            .unwrap();
        let published = publications_for(&fixture, "room-conv");
        assert!(store
            .delete_conversation("room-conv")
            .unwrap_err()
            .to_string()
            .contains("room participant context"));
        assert_eq!(publications_for(&fixture, "room-conv"), published);
        assert!(store.load_conversation("room-conv").unwrap().is_some());
    }

    // 2. Run status matrix: only "sealed" permits deletion; all non-sealed statuses reject!
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
        let conv_id = format!("run-{status}");
        let run_id = format!("r-{status}");
        create_conv(&mut store, &conv_id);
        fixture
            .execute(
                "INSERT INTO runs(id,conversation_id,kind,status,next_event_sequence,created_at_ms,updated_at_ms)
                 VALUES(?1,?2,'maintenance',?3,1,1,1)",
                params![run_id, conv_id, status],
            )
            .unwrap();
        let before_pubs = publications_for(&fixture, &conv_id);
        let result = store
            .conversation_write_repository()
            .delete_conversation(&conv_id);
        if status == "sealed" {
            assert!(result.unwrap(), "{status}");
            assert_tombstone_for(&fixture, &conv_id);
            let committed = publications_for(&fixture, &conv_id);
            assert!(!store.delete_conversation(&conv_id).unwrap());
            assert_eq!(publications_for(&fixture, &conv_id), committed);
        } else {
            assert!(
                result.unwrap_err().to_string().contains("Run is active"),
                "{status}"
            );
            assert_eq!(publications_for(&fixture, &conv_id), before_pubs);
        }
    }

    // 3. Acknowledged sync retention + revision overflow protection + trigger rollback & retry.
    create_conv(&mut store, "ack-conv");
    fixture
        .execute(
            "UPDATE sync_outbox SET status='acknowledged' WHERE entity_id='ack-conv'",
            [],
        )
        .unwrap();
    let acked = publications_for(&fixture, "ack-conv");
    assert!(store.delete_conversation("ack-conv").unwrap());
    assert_eq!(publications_for(&fixture, "ack-conv")[0], acked[0]);
    assert_tombstone_for(&fixture, "ack-conv");

    create_conv(&mut store, "overflow-conv");
    fixture
        .execute(
            "UPDATE conversations SET revision=?1 WHERE id='overflow-conv'",
            params![i64::MAX],
        )
        .unwrap();
    let before_overflow = publications_for(&fixture, "overflow-conv");
    assert!(store
        .delete_conversation("overflow-conv")
        .unwrap_err()
        .to_string()
        .contains("revision overflow"));
    assert_eq!(publications_for(&fixture, "overflow-conv"), before_overflow);

    create_conv(&mut store, "rollback-conv");
    let before_rollback = publications_for(&fixture, "rollback-conv");
    fixture
        .execute_batch(
            "CREATE TRIGGER fixture_reject_delete
             BEFORE INSERT ON sync_outbox WHEN NEW.deleted=1 AND NEW.entity_id='rollback-conv'
             BEGIN SELECT RAISE(ABORT,'fixture publication unavailable'); END;",
        )
        .unwrap();
    assert!(store
        .delete_conversation("rollback-conv")
        .unwrap_err()
        .to_string()
        .contains("fixture publication unavailable"));
    assert_eq!(publications_for(&fixture, "rollback-conv"), before_rollback);
    fixture
        .execute_batch("DROP TRIGGER fixture_reject_delete")
        .unwrap();
    drop(store);

    let mut reopened = NolaStore::open(&path).unwrap();
    assert!(reopened.delete_conversation("rollback-conv").unwrap());
    assert_tombstone_for(&fixture, "rollback-conv");
    drop(reopened);
    drop(fixture);

    // 4. Full active-turn sync projection -> rename metadata -> cancel_before_delete -> tombstone across cold reopens.
    let sync_db = directory.path().join("conversation-sync.db");
    let begin_input = BeginTurnInput {
        user_part_kind: "text".into(),
        expected_configuration_revision: None,
        expected_parent_message_id: None,
        initial_events: Vec::new(),
        initial_assistant_part_mutations: vec![],
        memory_usage_entries: vec![],
        delegated_invocation_mutations: vec![],
        delegated_permission_mutations: vec![],
        model_attempt_mutations: vec![],
        conversation_id: "conversation-sync-1".into(),
        conversation_title: "Durable conversation".into(),
        user_message_id: "message-user-1".into(),
        user_part_id: "part-user-1".into(),
        user_text: "Preserve this complete request".into(),
        user_content_json: None,
        additional_user_parts: Vec::new(),
        assistant_message_id: "message-assistant-1".into(),
        assistant_part_id: "part-assistant-1".into(),
        assistant_turn_bindings: vec![],
        run_id: "run-1".into(),
        context_checkpoint_id: None,
        checkpoint_revision: 1,
        checkpoint_event_cursor: 0,
        checkpoint_json: json!({
            "session_id": "fixture-core:run-1", "revision": 1,
            "state": {"session_id": "fixture-core:run-1"}
        })
        .to_string(),
        invocation_config_json: "{}".into(),
        invocation_config_digest: "conversation-config-digest".into(),
        outbox_effects: vec![OutboxEffectInput {
            delegated_invocation_id: None,
            model_attempt_id: None,
            effect_id: "effect-model-1".into(),
            request_id: "request-model-1".into(),
            effect_kind: "model_request".into(),
            write: outbox_fixture::model(
                "effect-model-1",
                "request-model-1",
                "text",
                "reasoning",
            ),
        }],
        created_at_ms: 1_000,
    };
    {
        let mut sync_store = NolaStore::open(&sync_db).unwrap();
        sync_store.begin_turn(&begin_input).unwrap();
        let changes = sync_store.pending_sync_changes(20).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].entity_type, "conversation");
        assert_eq!(changes[0].entity_id, "conversation-sync-1");
        assert_eq!(changes[0].revision, 1);
        let projection: StoredConversation =
            serde_json::from_str(&changes[0].payload_json).unwrap();
        assert_eq!(projection.messages.len(), 2);
        assert_eq!(
            projection.messages[0].parts[0].content_text,
            "Preserve this complete request"
        );
        assert!(sync_store
            .rename_conversation("conversation-sync-1", "Renamed", None, 2_000)
            .unwrap());
    }
    {
        let mut sync_store = NolaStore::open(&sync_db).unwrap();
        let changes = sync_store.pending_sync_changes(20).unwrap();
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[1].entity_type, "conversation_metadata");
        assert_eq!(changes[1].revision, 2);
        let metadata: nola_domain::ConversationMetadataReplica =
            serde_json::from_str(&changes[1].payload_json).unwrap();
        assert_eq!(metadata.title(), "Renamed");
        assert_eq!(metadata.base_content_revision(), 1);
        assert!(changes[1].payload_json.len() < 1024);
        conversation_deletion_fixture::cancel_before_delete(&mut sync_store, &begin_input);
        assert!(sync_store.delete_conversation("conversation-sync-1").unwrap());
    }
    {
        let sync_store = NolaStore::open(&sync_db).unwrap();
        assert!(sync_store
            .load_conversation("conversation-sync-1")
            .unwrap()
            .is_none());
        let changes = sync_store.pending_sync_changes(20).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].revision, 4);
        assert!(changes[0].deleted);
        assert_eq!(changes[0].payload_json, "null");
    }
}
