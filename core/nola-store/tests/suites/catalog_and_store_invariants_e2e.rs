// SPDX-License-Identifier: Apache-2.0
//! Consolidated Store invariant scenarios merging `agent_command_fifo_e2e`,
//! `agent_command_identity_e2e`, `auxiliary_transaction_e2e`, `checkpoint_e2e`,
//! `context_usage_read_model`, `data_source_e2e`, `host_outbox_e2e`,
//! `model_catalog_replacement_e2e`, `schema_epoch_e2e`, `skill_version_registration_e2e`,
//! `transcript_edit_port_e2e`, and `unit_of_work_e2e` (+ incompatible schema preservation).

use std::collections::BTreeMap;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use crate::support::outbox as outbox_fixture;
use nola_domain::{AuxiliaryInvocationOwner, AuxiliarySourceObservation};
use nola_store::{
    host::replay_pending_effects, BeginCommandReceiptInput, BeginCommandReceiptResult,
    BeginTurnInput, CompleteCommandReceiptOutcome, CreateConversationInput,
    EnqueueAgentCommandInput, NolaStore, OutboxEffectInput, SaveCheckpointInput, SkillSourceKind,
    StoredDataSourceRecord, StoredDataSourceUsage, StoredModelProfile, UpsertSkillInput,
    STORE_SCHEMA_EPOCH, STORE_SCHEMA_VERSION,
};
use nola_store_port::{
    auxiliary_invocation::AuxiliaryInvocationRepository,
    conversation_resource::{ConversationResourceRepository, PutConversationPdfResource},
    conversation_revert::ApplyConversationProjectionInput,
    transcript_write::{MessageInput, TranscriptEditRepository},
};
use rusqlite::Connection;
use serde_json::{json, Value};

#[test]
fn agent_command_fifo_ordering_lease_fencing_and_identity_deduplication_hold_across_reopen() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("agent-commands.sqlite");
    let mut first = NolaStore::open(&path).unwrap();

    // 1. Identity deduplication & payload immutability ("identity-session").
    let mut id_input = EnqueueAgentCommandInput {
        session_id: "identity-session".into(),
        command_id: "stable".into(),
        payload_json: "{\"operation\":1}".into(),
        created_at_ms: 100,
    };
    assert!(first.enqueue_agent_command(&id_input).unwrap());
    id_input.created_at_ms = 200;
    assert!(!first.enqueue_agent_command(&id_input).unwrap());
    id_input.payload_json = "{\"operation\":2}".into();
    assert!(first.enqueue_agent_command(&id_input).is_err());
    let claimed_id = first
        .claim_next_agent_command("identity-session", "consumer", 300, 400)
        .unwrap()
        .unwrap();
    assert_eq!(claimed_id.created_at_ms, 100);
    assert_eq!(claimed_id.payload_json, "{\"operation\":1}");

    // 2. FIFO head blocking across connections and cold reopen ("parent" & "other").
    for (session, id, time) in [
        ("parent", "waiting", 100),
        ("parent", "resume", 90),
        ("parent", "recover", 80),
        ("other", "independent", 70),
    ] {
        first
            .enqueue_agent_command(&EnqueueAgentCommandInput {
                session_id: session.into(),
                command_id: id.into(),
                payload_json: "{}".into(),
                created_at_ms: time,
            })
            .unwrap();
    }
    let head = first
        .claim_next_agent_command("parent", "worker-a", 200, 300)
        .unwrap()
        .unwrap();
    assert_eq!(head.command_id, "waiting");
    let mut second = NolaStore::open(&path).unwrap();
    assert!(second
        .claim_next_agent_command("parent", "worker-b", 210, 310)
        .unwrap()
        .is_none());
    assert!(second
        .claim_next_agent_command("other", "worker-b", 210, 310)
        .unwrap()
        .is_some());
    drop(first);
    drop(second);

    let mut reopened = NolaStore::open(&path).unwrap();
    assert!(reopened
        .claim_next_agent_command("parent", "worker-b", 299, 399)
        .unwrap()
        .is_none());
    let replay = reopened
        .claim_next_agent_command("parent", "worker-b", 300, 400)
        .unwrap()
        .unwrap();
    assert_eq!(replay.command_id, "waiting");
    assert!(!reopened
        .acknowledge_agent_command("parent", "waiting", "worker-a")
        .unwrap());
    assert!(reopened
        .claim_next_agent_command("parent", "worker-c", 301, 401)
        .unwrap()
        .is_none());
    assert!(reopened
        .acknowledge_agent_command("parent", "waiting", "worker-b")
        .unwrap());
    let resume = reopened
        .claim_next_agent_command("parent", "worker-b", 302, 402)
        .unwrap()
        .unwrap();
    assert_eq!(resume.command_id, "resume");
    assert!(reopened
        .release_agent_command_lease("parent", "resume", "worker-b")
        .unwrap());
    let resume = reopened
        .claim_next_agent_command("parent", "worker-c", 303, 403)
        .unwrap()
        .unwrap();
    assert_eq!(resume.command_id, "resume");
    assert!(reopened
        .acknowledge_agent_command("parent", "resume", "worker-c")
        .unwrap());
    assert_eq!(
        reopened
            .claim_next_agent_command("parent", "worker-c", 304, 404)
            .unwrap()
            .unwrap()
            .command_id,
        "recover"
    );
}

#[test]
fn unit_of_work_and_auxiliary_transactions_enforce_nested_and_outer_rollback_atomicity() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("transactions.sqlite");
    let mut store = NolaStore::open(&path).unwrap();

    // 1. Unit of work rollback vs commit for command receipts.
    let receipt_input = |request_id: &str| BeginCommandReceiptInput {
        client_id: "unit-of-work-client".to_string(),
        request_id: request_id.to_string(),
        command_type: "list_conversations".to_string(),
        command_json: r#"{"type":"list_conversations","query":{"project_root":null,"non_empty":false,"limit":1024,"offset":0}}"#.to_string(),
        expected_revision: None,
        received_at: "2026-09-04T00:00:00Z".to_string(),
    };
    store.begin_write_unit_of_work().unwrap();
    assert!(matches!(
        store
            .begin_command_receipt(&receipt_input("rolled-back"))
            .unwrap(),
        BeginCommandReceiptResult::Started(_)
    ));
    store.rollback_write_unit_of_work().unwrap();
    assert!(store
        .command_receipt("unit-of-work-client", "rolled-back")
        .unwrap()
        .is_none());

    store.begin_write_unit_of_work().unwrap();
    store
        .begin_command_receipt(&receipt_input("committed"))
        .unwrap();
    store
        .complete_command_receipt(
            "unit-of-work-client",
            "committed",
            "2026-09-04T00:00:01Z",
            &CompleteCommandReceiptOutcome::Committed {
                committed_revision: Some(7),
                result_json: "{\"ok\":true}".to_string(),
            },
        )
        .unwrap();
    store.commit_write_unit_of_work().unwrap();
    let receipt = store
        .command_receipt("unit-of-work-client", "committed")
        .unwrap()
        .unwrap();
    assert_eq!(receipt.status, "committed");
    assert_eq!(receipt.committed_revision, Some(7));
    assert_eq!(receipt.result_json.as_deref(), Some("{\"ok\":true}"));

    // 2. Auxiliary source observations & invocation rollback boundaries.
    let inspection = Connection::open(&path).unwrap();
    inspection
        .execute_batch(
            "INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms)
             VALUES('owner-a','fixture',1,1,1),('owner-b','fixture',1,1,1),('other','other',1,1,1);",
        )
        .unwrap();
    let make_obs = |op_id: &str| -> AuxiliarySourceObservation {
        serde_json::from_value(json!({
            "source_host_id":"source-host", "operation_id":op_id, "attempt_id":"physical",
            "receipt":{"dispatch_permitted":false,
                "operation":{"id":op_id,"owner":{"kind":"source_authority","source_host_id":"source-host","authority_id":"epoch"},
                    "scenario":"context_summary","configuration":{"provider":"fixture","model":"fixture-model","configuration_revision":1,
                        "configuration_digest":"a".repeat(64),"input_digest":"b".repeat(64),"max_attempts":1,"maximum_duration_ms":1000,
                        "max_output_tokens_per_attempt":100,"total_output_token_budget":100},
                    "state":"succeeded","attempts_started":1,"active_attempt_id":null,"cancellation_requested_at_ms":null,"created_at_ms":1,"revision":3},
                "attempt":{"id":"physical","operation_id":op_id,"predecessor_id":null,"state":"succeeded",
                    "provider":"fixture","model":"fixture-model","usage":{"coverage":"unknown","provider_raw":null,"normalized":null},
                    "failure":null,"output":{"items":[]},"created_at_ms":2,"completed_at_ms":3,"revision":2}}
        }))
        .unwrap()
    };

    let owner_a = AuxiliaryInvocationOwner::Conversation {
        conversation_id: "owner-a".into(),
    };
    let obs_a = make_obs("summary-a");
    inspection
        .execute_batch(
            "CREATE TRIGGER reject_projection BEFORE UPDATE ON conversations
             WHEN NEW.id='owner-a' BEGIN SELECT RAISE(ABORT,'injected projection failure'); END;",
        )
        .unwrap();
    assert!(store
        .commit_auxiliary_source_observation(&owner_a, &obs_a)
        .is_err());
    assert!(store
        .auxiliary_source_observation(&owner_a, "source-host", "summary-a")
        .unwrap()
        .is_none());
    let rev_a: i64 = inspection
        .query_row(
            "SELECT revision FROM conversations WHERE id='owner-a'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(rev_a, 1);
    inspection.execute_batch("DROP TRIGGER reject_projection").unwrap();
    store
        .commit_auxiliary_source_observation(&owner_a, &obs_a)
        .unwrap();
    assert_eq!(
        store
            .auxiliary_source_observation(&owner_a, "source-host", "summary-a")
            .unwrap(),
        Some(obs_a)
    );
    assert!(inspection.is_autocommit());

    let owner_b = AuxiliaryInvocationOwner::Conversation {
        conversation_id: "owner-b".into(),
    };
    let foreign = AuxiliaryInvocationOwner::Conversation {
        conversation_id: "other".into(),
    };
    let obs_b = make_obs("summary-b");
    store.begin_write_unit_of_work().unwrap();
    store
        .commit_auxiliary_source_observation(&owner_b, &obs_b)
        .unwrap();
    assert!(store
        .commit_auxiliary_source_observation(&foreign, &obs_b)
        .is_err());
    assert_eq!(
        store
            .auxiliary_source_observation(&owner_b, "source-host", "summary-b")
            .unwrap(),
        Some(obs_b.clone())
    );
    store.rollback_write_unit_of_work().unwrap();
    assert!(store
        .auxiliary_source_observation(&owner_b, "source-host", "summary-b")
        .unwrap()
        .is_none());
    store
        .commit_auxiliary_source_observation(&owner_b, &obs_b)
        .unwrap();

    let inv_receipt = make_obs("summary-inv").receipt;
    let mut operation = inv_receipt.operation;
    operation.owner = owner_b;
    let mut bad_attempt = inv_receipt.attempt.unwrap();
    bad_attempt.operation_id = "foreign-operation".into();
    assert!(store
        .commit_auxiliary_invocation(&operation, None, Some(&bad_attempt), None)
        .is_err());
    assert!(store.auxiliary_operation("summary-inv").unwrap().is_none());
    assert!(store
        .auxiliary_attempt("summary-inv", "physical")
        .unwrap()
        .is_none());
    store.begin_write_unit_of_work().unwrap();
    store
        .commit_auxiliary_invocation(&operation, None, None, None)
        .unwrap();
    assert_eq!(
        store.auxiliary_operation("summary-inv").unwrap(),
        Some(operation.clone())
    );
    store.rollback_write_unit_of_work().unwrap();
    assert!(store.auxiliary_operation("summary-inv").unwrap().is_none());
    store
        .commit_auxiliary_invocation(&operation, None, None, None)
        .unwrap();
    assert_eq!(
        store.auxiliary_operation("summary-inv").unwrap(),
        Some(operation)
    );
}

#[test]
fn checkpoint_cas_presentation_read_model_and_transcript_edit_ownership_hold_across_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("checkpoints-and-edits.db");
    let mut first = NolaStore::open(&database).unwrap();
    for id in ["conversation-1", "local", "target", "other"] {
        first
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

    // 1. Checkpoint CAS across connections & stale rejection.
    first
        .save_checkpoint(&SaveCheckpointInput {
            conversation_id: "conversation-1".into(),
            expected_revision: 0,
            revision: 1,
            event_cursor: 7,
            checkpoint_json: "{\"state\":\"first\"}".into(),
            updated_at_ms: 100,
        })
        .unwrap();
    let mut second = NolaStore::open(&database).unwrap();
    second
        .save_checkpoint(&SaveCheckpointInput {
            conversation_id: "conversation-1".into(),
            expected_revision: 1,
            revision: 2,
            event_cursor: 8,
            checkpoint_json: "{\"state\":\"second\"}".into(),
            updated_at_ms: 200,
        })
        .unwrap();
    assert!(first
        .save_checkpoint(&SaveCheckpointInput {
            conversation_id: "conversation-1".into(),
            expected_revision: 1,
            revision: 2,
            event_cursor: 9,
            checkpoint_json: "{\"state\":\"stale\"}".into(),
            updated_at_ms: 300,
        })
        .is_err());

    // 2. Local presentation inputs omit 10MB model_working_set, private memory, and unverified token estimates.
    let presentation_cp = json!({
        "session_id": "local", "last_event_cursor": {"sequence": 2},
        "state": {
            "session_id": "local",
            "conversation": {
                "projection": {"revision": 1, "order": [], "parts": {}},
                "model_working_set": "x".repeat(10_000_000),
                "context_window": {
                    "config": {}, "used_tokens": 123, "usage_origin": "estimated"
                }
            },
            "memory_projection": {"private": "not part of the timeline"},
            "task_projection": {"revision": 0, "order": [], "tasks": {}},
            "skill_runtime": {"revision": 0, "catalog": {"revision": 0}, "runs": {}}
        }
    });
    first
        .save_checkpoint(&SaveCheckpointInput {
            conversation_id: "local".into(),
            expected_revision: 0,
            revision: 1,
            event_cursor: 2,
            checkpoint_json: presentation_cp.to_string(),
            updated_at_ms: 2,
        })
        .unwrap();
    let inputs = first
        .load_local_presentation_inputs("local", "[]", None)
        .unwrap()
        .unwrap();
    assert!(inputs.len() < 2048);
    assert!(!inputs.contains("model_working_set"));
    assert!(!inputs.contains("not part of the timeline"));
    let parsed_inputs: Value = serde_json::from_str(&inputs).unwrap();
    assert_eq!(parsed_inputs["cursor"], 2);
    assert!(parsed_inputs.get("used_tokens").is_none());
    assert_eq!(parsed_inputs["projection"]["revision"], 1);

    // 3. Transcript edit port rechecks active run & foreign run ownership in commit transaction.
    let fixture = Connection::open(&database).unwrap();
    fixture
        .execute_batch(
            "INSERT INTO runs(id,conversation_id,kind,status,next_event_sequence,created_at_ms,updated_at_ms)
             VALUES('active','target','conversation','running',1,1,1),('foreign','other','conversation','running',1,1,1);",
        )
        .unwrap();
    let mut edit_input = ApplyConversationProjectionInput {
        operation_id: "edit".into(),
        conversation_id: "target".into(),
        expected_revision: 1,
        operation_kind: "edit".into(),
        parent_message_id: None,
        mutations: vec![],
        appended_messages: vec![],
        created_at_ms: 2,
    };
    assert!(first
        .conversation_write_repository()
        .apply_conversation_projection(&edit_input)
        .unwrap_err()
        .to_string()
        .contains("Run is active"));
    fixture
        .execute("DELETE FROM runs WHERE id='active'", [])
        .unwrap();
    edit_input.appended_messages.push(MessageInput {
        id: "replacement".into(),
        run_id: Some("foreign".into()),
        role: "user".into(),
        status: "completed".into(),
        ordinal: 1,
        created_at_ms: 2,
        completed_at_ms: Some(2),
        parts: vec![],
    });
    assert!(first
        .conversation_write_repository()
        .apply_conversation_projection(&edit_input)
        .unwrap_err()
        .to_string()
        .contains("another conversation"));
    edit_input.appended_messages[0].run_id = Some("active".into());
    assert!(
        first
            .conversation_write_repository()
            .apply_conversation_projection(&edit_input)
            .unwrap()
            .applied
    );

    drop(fixture);
    drop(second);
    drop(first);

    let mut reopened = NolaStore::open(&database).unwrap();
    let cp = reopened.load_checkpoint("conversation-1").unwrap().unwrap();
    assert_eq!(cp.revision, 2);
    assert_eq!(cp.event_cursor, 8);
    assert_eq!(cp.checkpoint_json, "{\"state\":\"second\"}");
    assert!(
        !reopened
            .conversation_write_repository()
            .apply_conversation_projection(&edit_input)
            .unwrap()
            .applied
    );
}

#[test]
fn skill_versions_model_catalog_scoping_and_data_sources_persist_across_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("catalogs.sqlite");
    let mut store = NolaStore::open(&path).unwrap();

    // 1. Skill version registration validation & non-selecting update.
    let skill_pkg = |version: &str| UpsertSkillInput {
        id: "word".into(),
        source: SkillSourceKind::BuiltIn,
        version: version.into(),
        display_name: "Word".into(),
        summary: "Document authoring".into(),
        category: "documents".into(),
        instructions_ref: format!("skill://word/{version}/SKILL.md"),
        instructions_text: "hello".into(),
        required_tools_json: None,
        default_mode: None,
        enabled: true,
        created_at_ms: 1,
        updated_at_ms: 2,
    };
    assert!(store.register_skill_version(&skill_pkg("v1")).is_err());
    store.upsert_skill(&skill_pkg("v1")).unwrap();
    store.set_skill_enabled("word", false, 3).unwrap();
    let skill_baseline = store.skill_editor_snapshot("word").unwrap();
    let mut wrong_source = skill_pkg("v2");
    wrong_source.source = SkillSourceKind::User;
    assert!(store.register_skill_version(&wrong_source).is_err());
    let mut empty_instructions = skill_pkg("v2");
    empty_instructions.instructions_text = "   ".into();
    assert!(store.register_skill_version(&empty_instructions).is_err());
    let next_skill = skill_pkg("v2");
    store.register_skill_version(&next_skill).unwrap();
    store.register_skill_version(&next_skill).unwrap();
    assert_eq!(store.skill_editor_snapshot("word").unwrap(), skill_baseline);

    // 2. Model catalog replacement & provider/runtime-scoped selection displacement.
    let model_profile = |id: &str, selected: bool, enabled: bool| StoredModelProfile {
        id: id.into(),
        display_name: id.into(),
        provider: "openai-compatible".into(),
        model_name: id.into(),
        runtime: "remote".into(),
        source_json: r#"{"schemaVersion":1,"endpointID":"fixture","baseURL":"https://example.invalid/v1"}"#.into(),
        parameters_json: "{}".into(),
        selected,
        enabled,
        revision: 1,
        created_at_ms: 1,
        updated_at_ms: 1,
        deleted_at_ms: None,
    };
    store
        .replace_model_profiles(
            "openai-compatible",
            "remote",
            &[model_profile("disabled", false, false)],
            1,
            &BTreeMap::new(),
        )
        .unwrap();
    assert!(!store.list_model_profiles().unwrap()[0].selected);
    let initial_revs: BTreeMap<_, _> = store
        .list_model_profiles()
        .unwrap()
        .into_iter()
        .map(|m| (m.id, m.revision))
        .collect();
    let selected = model_profile("selected", true, true);
    let disabled = model_profile("disabled", false, false);
    store
        .replace_model_profiles(
            "openai-compatible",
            "remote",
            &[selected.clone(), disabled],
            2,
            &initial_revs,
        )
        .unwrap();
    let current_revs: BTreeMap<_, _> = store
        .list_model_profiles()
        .unwrap()
        .into_iter()
        .map(|m| (m.id, m.revision))
        .collect();
    assert!(store
        .replace_model_profiles(
            "openai-compatible",
            "remote",
            &[selected, model_profile("disabled", true, false)],
            3,
            &current_revs,
        )
        .is_err());

    let mut local = model_profile("local", true, true);
    local.provider = "mlx".into();
    local.runtime = "local".into();
    store.upsert_model_profile(&local).unwrap();
    let default_json = r#"{"version":1,"selection":{"kind":"profile","profile_id":"local"}}"#;
    let default_before = store
        .put_product_setting("chat.default_model", default_json, 1)
        .unwrap();
    let local_before = store
        .list_model_profiles()
        .unwrap()
        .into_iter()
        .find(|p| p.id == "local")
        .unwrap();
    store
        .upsert_model_profile(&model_profile("remote-a", true, true))
        .unwrap();
    store
        .upsert_model_profile(&model_profile("remote-b", true, true))
        .unwrap();
    let profiles = store.list_model_profiles().unwrap();
    let local_after = profiles.iter().find(|p| p.id == "local").unwrap();
    assert!(local_after.selected);
    assert_eq!(local_before.revision, local_after.revision);
    assert!(!profiles.iter().find(|p| p.id == "remote-a").unwrap().selected);
    let remote_before = profiles.iter().find(|p| p.id == "remote-b").unwrap();
    let mut another_local = local.clone();
    another_local.id = "local-next".into();
    store.upsert_model_profile(&another_local).unwrap();
    let profiles = store.list_model_profiles().unwrap();
    let remote_after = profiles.iter().find(|p| p.id == "remote-b").unwrap();
    assert!(remote_after.selected);
    assert_eq!(remote_before.revision, remote_after.revision);
    let default_after = store.product_setting("chat.default_model").unwrap().unwrap();
    assert_eq!(default_before.revision, default_after.revision);
    store.delete_model_profile("local", 10).unwrap();
    assert_eq!(
        store
            .product_setting("chat.default_model")
            .unwrap()
            .unwrap()
            .value_json,
        default_json
    );

    // 3. Verified data source validation, override scoping, usage idempotency, and deletion cascade.
    let make_source = |updated_at_ms: i64| StoredDataSourceRecord {
        id: "sales".into(),
        name: "Sales data".into(),
        kind: "localFile".into(),
        location: "/data/sales.csv".into(),
        enabled: true,
        access_policy: "conversationOptIn".into(),
        validation_contract_json:
            r#"{"readRecipe":{"method":"localFileRead"},"redactedSample":"CSV · 2 rows"}"#.into(),
        capabilities_json: r#"["read","context"]"#.into(),
        privacy: "local-first".into(),
        metadata_json: r#"{"maximumBytes":262144}"#.into(),
        revision: 0,
        created_at_ms: 100,
        updated_at_ms,
        deleted_at_ms: None,
    };
    let mut invalid_src = make_source(100);
    invalid_src.validation_contract_json = "raw secret, not JSON".into();
    assert!(store.upsert_data_source(&invalid_src).is_err());
    assert!(store.list_data_sources().unwrap().is_empty());

    let first_src = store.upsert_data_source(&make_source(100)).unwrap();
    assert_eq!(first_src.revision, 1);
    let unchanged_src = store.upsert_data_source(&make_source(200)).unwrap();
    assert_eq!(unchanged_src.revision, 1);
    assert_eq!(unchanged_src.updated_at_ms, 100);
    assert!(store
        .set_data_source_override("missing", "sales", Some(true), 101)
        .is_err());
    for conv_id in ["conversation-keep", "conversation-temp"] {
        store
            .create_conversation(&CreateConversationInput {
                id: conv_id.into(),
                title: "Source test".into(),
                revision: 1,
                settings_json: None,
                created_at_ms: 100,
                updated_at_ms: 100,
            })
            .unwrap();
        store
            .set_data_source_override(conv_id, "sales", Some(true), 300)
            .unwrap();
    }
    assert!(store.delete_conversation("conversation-temp").unwrap());
    assert_eq!(
        store
            .data_source_override("conversation-temp", "sales")
            .unwrap(),
        None
    );
    assert!(store
        .set_data_source_override("conversation-temp", "sales", None, 301)
        .is_err());
    let usage = StoredDataSourceUsage {
        id: "usage-1".into(),
        source_id: "sales".into(),
        source_title: "Sales data".into(),
        consumer: "chat".into(),
        purpose: "conversation_context".into(),
        conversation_id: Some("conversation-keep".into()),
        disclosed_to_cloud: None,
        metadata_json: r#"{"model":"cloud-model"}"#.into(),
        used_at_ms: 400,
    };
    store.append_data_source_usage(&usage).unwrap();
    store.append_data_source_usage(&usage).unwrap();
    drop(store);

    let mut reopened = NolaStore::open(&path).unwrap();
    let mut conflicting_skill = next_skill.clone();
    conflicting_skill.instructions_ref = "skill://word/different/SKILL.md".into();
    assert!(reopened.register_skill_version(&conflicting_skill).is_err());
    assert!(reopened.upsert_skill(&conflicting_skill).is_err());
    assert_eq!(
        reopened.skill_editor_snapshot("word").unwrap(),
        skill_baseline
    );
    let selected_skill = reopened.upsert_skill(&next_skill).unwrap();
    assert_eq!(selected_skill.version, "v2");
    assert!(!selected_skill.enabled);

    assert_eq!(reopened.list_data_sources().unwrap().len(), 1);
    assert_eq!(
        reopened
            .data_source_override("conversation-keep", "sales")
            .unwrap(),
        Some(true)
    );
    assert_eq!(reopened.list_data_source_usage(20).unwrap(), vec![usage]);
    assert!(reopened.delete_data_source("sales", 500).unwrap());
    assert!(reopened.data_source("sales").unwrap().is_none());
    assert_eq!(
        reopened
            .data_source_override("conversation-keep", "sales")
            .unwrap(),
        None
    );
    assert_eq!(reopened.list_data_source_usage(20).unwrap().len(), 1);
}

#[test]
fn host_outbox_replays_failed_effects_and_skips_acknowledged_prefixes_across_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("outbox.db");
    let make_turn = |conv_id: &str, run_id: &str, effects: Vec<OutboxEffectInput>| BeginTurnInput {
        user_part_kind: "text".into(),
        expected_configuration_revision: None,
        expected_parent_message_id: None,
        initial_events: Vec::new(),
        initial_assistant_part_mutations: vec![],
        memory_usage_entries: vec![],
        delegated_invocation_mutations: vec![],
        delegated_permission_mutations: vec![],
        model_attempt_mutations: vec![],
        conversation_id: conv_id.into(),
        conversation_title: "Outbox E2E".into(),
        user_message_id: format!("u-{run_id}"),
        user_part_id: format!("up-{run_id}"),
        user_text: "run the durable effect".into(),
        user_content_json: None,
        additional_user_parts: Vec::new(),
        assistant_message_id: format!("a-{run_id}"),
        assistant_part_id: format!("ap-{run_id}"),
        assistant_turn_bindings: vec![],
        run_id: run_id.into(),
        context_checkpoint_id: None,
        checkpoint_revision: 0,
        checkpoint_event_cursor: 0,
        checkpoint_json: "{\"state\":\"running\"}".into(),
        invocation_config_json: "{}".into(),
        invocation_config_digest: "conversation-config-digest".into(),
        outbox_effects: effects,
        created_at_ms: 1_700_000_000_000,
    };

    {
        let mut store = NolaStore::open(&database).unwrap();
        store
            .begin_turn(&make_turn(
                "conv-single",
                "run-single",
                vec![OutboxEffectInput {
                    delegated_invocation_id: None,
                    model_attempt_id: None,
                    effect_id: "effect-single".into(),
                    request_id: "request-single".into(),
                    effect_kind: "tool.execute".into(),
                    write: outbox_fixture::tool(
                        "effect-single",
                        "request-single",
                        json!({"tool":"read_file"}),
                    ),
                }],
            ))
            .unwrap();
        let report_single =
            replay_pending_effects(&mut store, "run-single", 10, 100, |_| {
                Err("host unavailable".into())
            })
            .unwrap();
        assert_eq!(report_single.dispatched, 1);
        assert_eq!(report_single.acknowledged, 0);
        assert!(report_single.stopped_after_failure);

        store
            .begin_turn(&make_turn(
                "conv-prefix",
                "run-prefix",
                vec![
                    OutboxEffectInput {
                        delegated_invocation_id: None,
                        model_attempt_id: None,
                        effect_id: "effect-1".into(),
                        request_id: "request-1".into(),
                        effect_kind: "tool.execute".into(),
                        write: outbox_fixture::tool(
                            "effect-1",
                            "request-1",
                            json!({"tool":"read_file"}),
                        ),
                    },
                    OutboxEffectInput {
                        delegated_invocation_id: None,
                        model_attempt_id: None,
                        effect_id: "effect-2".into(),
                        request_id: "request-2".into(),
                        effect_kind: "artifact.publish".into(),
                        write: outbox_fixture::tool(
                            "effect-2",
                            "request-2",
                            json!({"artifact":"report.docx"}),
                        ),
                    },
                ],
            ))
            .unwrap();
        let mut first_pass = Vec::new();
        let report_prefix =
            replay_pending_effects(&mut store, "run-prefix", 10, 100, |effect| {
                first_pass.push(effect.effect_id.clone());
                if effect.effect_id == "effect-2" {
                    Err("publisher unavailable".into())
                } else {
                    Ok(())
                }
            })
            .unwrap();
        assert_eq!(first_pass, vec!["effect-1", "effect-2"]);
        assert_eq!(report_prefix.acknowledged, 1);
        assert!(report_prefix.stopped_after_failure);
    }

    let mut reopened = NolaStore::open(&database).unwrap();
    let report_single =
        replay_pending_effects(&mut reopened, "run-single", 10, 200, |_| Ok(())).unwrap();
    assert_eq!(report_single.acknowledged, 1);
    assert!(reopened.pending_effects("run-single").unwrap().is_empty());

    let mut replayed = Vec::new();
    let report_prefix = replay_pending_effects(&mut reopened, "run-prefix", 10, 200, |effect| {
        replayed.push(effect.effect_id.clone());
        Ok(())
    })
    .unwrap();
    assert_eq!(replayed, vec!["effect-2"]);
    assert_eq!(report_prefix.acknowledged, 1);
    assert!(reopened.pending_effects("run-prefix").unwrap().is_empty());
}

#[test]
fn schema_epoch_baseline_and_incompatible_formats_fail_closed_without_rewriting_bytes() {
    assert_eq!(STORE_SCHEMA_VERSION, 1);
    assert_eq!(STORE_SCHEMA_EPOCH, 1);
    let directory = tempfile::tempdir().unwrap();

    // 1. Fresh store records schema_epoch == 1; previous epoch is rejected without rewrite.
    let current_db = directory.path().join("current.db");
    drop(NolaStore::open(&current_db).unwrap());
    let conn = Connection::open(&current_db).unwrap();
    let epoch: i64 = conn
        .query_row(
            "SELECT CAST(value AS INTEGER) FROM store_metadata WHERE key='schema_epoch'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(epoch, 1);
    let prev_epoch = STORE_SCHEMA_EPOCH - 1;
    conn.execute(
        "UPDATE store_metadata SET value=?1 WHERE key='schema_epoch'",
        [prev_epoch.to_string()],
    )
    .unwrap();
    drop(conn);
    let err = NolaStore::open(&current_db).err().expect("previous epoch must be rejected");
    assert!(err.is_schema_incompatible());
    let conn = Connection::open(&current_db).unwrap();
    let preserved_epoch: String = conn
        .query_row(
            "SELECT value FROM store_metadata WHERE key='schema_epoch'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(preserved_epoch, prev_epoch.to_string());
    drop(conn);

    // 2. Unsupported legacy development user_version=83 preserves exact bytes.
    let dev_db = directory.path().join("unsupported-dev.sqlite");
    let conn = Connection::open(&dev_db).unwrap();
    conn.execute_batch(
        "PRAGMA application_id=1313823809; PRAGMA user_version=83; CREATE TABLE valuable(content TEXT); INSERT INTO valuable VALUES('keep this data');",
    )
    .unwrap();
    drop(conn);
    let dev_before = std::fs::read(&dev_db).unwrap();
    assert!(NolaStore::open(&dev_db).is_err());
    assert_eq!(std::fs::read(&dev_db).unwrap(), dev_before);

    // 3. Incompatible user_version (version-1, version+1, and 60 with legacy model index) preserves PDF & model bytes.
    let pdf = b"%PDF-1.4\n1 0 obj <<>> endobj\n%%EOF\n";
    for version in [STORE_SCHEMA_VERSION - 1, STORE_SCHEMA_VERSION + 1, 60] {
        let path = directory.path().join(format!("incompatible-{version}.sqlite"));
        let mut store = NolaStore::open(&path).unwrap();
        store
            .create_conversation(&CreateConversationInput {
                id: "existing".into(),
                title: "existing".into(),
                revision: 1,
                settings_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
            })
            .unwrap();
        let resource_id = uuid::Uuid::new_v4().to_string();
        store
            .put_conversation_pdf_resource(&PutConversationPdfResource {
                conversation_id: "existing".into(),
                resource_id: resource_id.clone(),
                display_name: "报告.pdf".into(),
                mime_type: "application/pdf".into(),
                data_base64: STANDARD.encode(pdf),
                created_at_ms: 1,
            })
            .unwrap()
            .unwrap();
        store
            .upsert_model_profile(&StoredModelProfile {
                id: "remote".into(),
                display_name: "remote".into(),
                provider: "openai-compatible".into(),
                model_name: "remote".into(),
                runtime: "remote".into(),
                source_json: "{}".into(),
                parameters_json: "{}".into(),
                selected: true,
                enabled: true,
                revision: 1,
                created_at_ms: 1,
                updated_at_ms: 1,
                deleted_at_ms: None,
            })
            .unwrap();
        drop(store);
        let sql = Connection::open(&path).unwrap();
        if version == 60 {
            sql.execute_batch(
                "DROP INDEX idx_model_profiles_selected;
                 CREATE UNIQUE INDEX idx_model_profiles_selected ON model_profiles(selected)
                 WHERE selected=1 AND enabled=1 AND deleted_at_ms IS NULL;",
            )
            .unwrap();
        }
        sql.pragma_update(None, "user_version", version).unwrap();
        sql.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
        drop(sql);
        let before = std::fs::read(&path).unwrap();
        assert!(matches!(
            NolaStore::open(&path),
            Err(nola_store::StoreError::IncompatibleSchema)
        ));
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let sql = Connection::open(&path).unwrap();
        let body: Vec<u8> = sql
            .query_row(
                "SELECT body FROM conversation_pdf_resources WHERE resource_id=?1",
                [&resource_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(body, pdf);
    }
}
