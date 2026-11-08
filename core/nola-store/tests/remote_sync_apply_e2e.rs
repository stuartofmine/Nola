use nola_store::{
    ApplyRemoteSyncPageInput, BeginTurnInput, NolaStore, OutboxEffectInput, RemoteSyncChangeInput,
    SkillSourceKind, StoredAutomationDefinition, StoredDataSourceRecord, StoredMcpServerRecord,
    StoredMemoryRecord, StoredModelProfile, UpsertSkillInput,
};
use tempfile::tempdir;

#[path = "support/conversation_deletion_fixture.rs"]
mod conversation_deletion_fixture;

fn remote_change(cursor: i64, change: &nola_store::StoredSyncChange) -> RemoteSyncChangeInput {
    RemoteSyncChangeInput {
        cursor,
        operation_id: change.operation_id.clone(),
        entity_type: change.entity_type.clone(),
        entity_id: change.entity_id.clone(),
        revision: change.revision,
        deleted: change.deleted,
        payload_json: change.payload_json.clone(),
        created_at_ms: change.created_at_ms,
    }
}

fn conversation_input() -> BeginTurnInput {
    BeginTurnInput {
        user_part_kind: "text".into(),
        expected_configuration_revision: None,
        expected_parent_message_id: None,
        initial_events: Vec::new(),
        initial_assistant_part_mutations: vec![],
        memory_usage_entries: vec![],
        delegated_invocation_mutations: vec![],
        delegated_permission_mutations: vec![],
        model_attempt_mutations: vec![],
        conversation_id: "shared-conversation".into(),
        conversation_title: "Shared".into(),
        user_message_id: "shared-user".into(),
        user_part_id: "shared-user-part".into(),
        user_text: "Complete transcript".into(),
        user_content_json: Some(r#"{"attachments":[]}"#.into()),
        additional_user_parts: Vec::new(),
        assistant_message_id: "shared-assistant".into(),
        assistant_part_id: "shared-assistant-part".into(),
        assistant_turn_bindings: vec![],
        run_id: "shared-run".into(),
        context_checkpoint_id: None,
        checkpoint_revision: 1,
        checkpoint_event_cursor: 0,
        // The later cancellation is a submission in this admitted Core session.
        checkpoint_json: serde_json::json!({
            "session_id": "fixture-core:shared-run", "revision": 1,
            "state": {"session_id": "fixture-core:shared-run"}
        }).to_string(),
        invocation_config_json: "{}".into(),
        invocation_config_digest: "conversation-config-digest".into(),
        outbox_effects: vec![OutboxEffectInput {
            delegated_invocation_id: None,
            model_attempt_id: None,
            effect_id: "shared-effect".into(),
            request_id: "shared-request".into(),
            effect_kind: "model_request".into(),
            write: outbox_fixture::model("shared-effect", "shared-request", "text", "reasoning"),
        }],
        created_at_ms: 1_000,
    }
}

#[test]
fn two_devices_apply_complete_conversation_projection_and_tombstone() {
    let directory = tempdir().unwrap();
    let source_path = directory.path().join("conversation-source.db");
    let target_path = directory.path().join("conversation-target.db");
    let created = {
        let mut source = NolaStore::open(&source_path).unwrap();
        source.begin_turn(&conversation_input()).unwrap();
        source.pending_sync_changes(10).unwrap().remove(0)
    };
    {
        let mut target = NolaStore::open(&target_path).unwrap();
        target
            .apply_remote_sync_page(&ApplyRemoteSyncPageInput {
                replica_id: "conversation-replica".into(),
                changes: vec![remote_change(1, &created)],
                next_cursor: 1,
                applied_at_ms: 1_100,
            })
            .unwrap();
        let projection = target
            .load_conversation("shared-conversation")
            .unwrap()
            .unwrap();
        assert_eq!(projection.messages.len(), 2);
        assert_eq!(
            projection.messages[0].parts[0].content_text,
            "Complete transcript"
        );
        assert!(target.pending_sync_changes(10).unwrap().is_empty());
        assert!(target
            .load_checkpoint("shared-conversation")
            .unwrap()
            .is_none());
        target.upsert_data_source(&StoredDataSourceRecord {
            id: "local-source".into(), name: "Local source".into(), kind: "localFile".into(),
            location: "/fixture/source".into(), enabled: true, access_policy: "conversationOptIn".into(),
            validation_contract_json: "{}".into(), capabilities_json: "[]".into(),
            privacy: "local-first".into(), metadata_json: "{}".into(), revision: 0,
            created_at_ms: 1_100, updated_at_ms: 1_100, deleted_at_ms: None,
        }).unwrap();
        target.set_data_source_override("shared-conversation", "local-source", Some(true), 1_101).unwrap();
    }
    let renamed = {
        let mut source = NolaStore::open(&source_path).unwrap();
        source
            .rename_conversation("shared-conversation", "Renamed remotely", None, 2_000)
            .unwrap();
        source
            .pending_sync_changes(10)
            .unwrap()
            .into_iter()
            .find(|change| change.revision == 2)
            .unwrap()
    };
    {
        let mut target = NolaStore::open(&target_path).unwrap();
        target
            .apply_remote_sync_page(&ApplyRemoteSyncPageInput {
                replica_id: "conversation-replica".into(),
                changes: vec![remote_change(2, &renamed)],
                next_cursor: 2,
                applied_at_ms: 2_100,
            })
            .unwrap();
        assert_eq!(
            target
                .load_conversation("shared-conversation")
                .unwrap()
                .unwrap()
                .title,
            "Renamed remotely"
        );
        assert_eq!(target.data_source_override("shared-conversation", "local-source").unwrap(), Some(true),
            "remote projection replacement must not clear local source selection");
    }
    let deleted = {
        let mut source = NolaStore::open(&source_path).unwrap();
        conversation_deletion_fixture::cancel_before_delete(&mut source, &conversation_input());
        source.delete_conversation("shared-conversation").unwrap();
        source
            .pending_sync_changes(10)
            .unwrap()
            .into_iter()
            .find(|change| change.deleted)
            .unwrap()
    };
    {
        let mut target = NolaStore::open(&target_path).unwrap();
        target
            .apply_remote_sync_page(&ApplyRemoteSyncPageInput {
                replica_id: "conversation-replica".into(),
                changes: vec![remote_change(3, &deleted)],
                next_cursor: 3,
                applied_at_ms: 3_100,
            })
            .unwrap();
    }
    let reopened = NolaStore::open(&target_path).unwrap();
    assert!(reopened
        .load_conversation("shared-conversation")
        .unwrap()
        .is_none());
    assert_eq!(reopened.data_source_override("shared-conversation", "local-source").unwrap(), None,
        "remote deletion must remove the local source selection");
    assert_eq!(
        reopened
            .sync_cursor("conversation-replica")
            .unwrap()
            .unwrap()
            .cursor,
        3
    );
}

#[test]
fn two_devices_replicate_mcp_configuration_and_tombstone_without_echo() {
    let directory = tempdir().unwrap();
    let source_path = directory.path().join("mcp-source.db");
    let target_path = directory.path().join("mcp-target.db");
    let first = {
        let mut source = NolaStore::open(&source_path).unwrap();
        source
            .upsert_mcp_server(&StoredMcpServerRecord {
                id: "filesystem".into(),
                display_name: "Filesystem".into(),
                enabled: true,
                configuration_json: r#"{"transport":"stdio","command":"mcp-filesystem"}"#.into(),
                revision: 0,
                created_at_ms: 1_000,
                updated_at_ms: 1_000,
                deleted_at_ms: None,
            })
            .unwrap();
        source.pending_sync_changes(10).unwrap().remove(0)
    };
    {
        let mut target = NolaStore::open(&target_path).unwrap();
        target
            .apply_remote_sync_page(&ApplyRemoteSyncPageInput {
                replica_id: "mcp-replica".into(),
                changes: vec![remote_change(1, &first)],
                next_cursor: 1,
                applied_at_ms: 1_100,
            })
            .unwrap();
        let server = target.mcp_server("filesystem").unwrap().unwrap();
        assert_eq!(server.revision, 1);
        assert!(server.enabled);
        assert!(server.configuration_json.contains("mcp-filesystem"));
        assert!(target.pending_sync_changes(10).unwrap().is_empty());
    }
    let tombstone = {
        let mut source = NolaStore::open(&source_path).unwrap();
        source.delete_mcp_server("filesystem", 2_000).unwrap();
        source
            .pending_sync_changes(10)
            .unwrap()
            .into_iter()
            .find(|change| change.deleted)
            .unwrap()
    };
    {
        let mut target = NolaStore::open(&target_path).unwrap();
        target
            .apply_remote_sync_page(&ApplyRemoteSyncPageInput {
                replica_id: "mcp-replica".into(),
                changes: vec![remote_change(2, &tombstone)],
                next_cursor: 2,
                applied_at_ms: 2_100,
            })
            .unwrap();
    }
    let target = NolaStore::open(&target_path).unwrap();
    assert!(target.mcp_server("filesystem").unwrap().is_none());
    assert_eq!(
        target.sync_cursor("mcp-replica").unwrap().unwrap().cursor,
        2
    );
}

#[test]
fn two_devices_replicate_verified_data_source_and_delete_overrides() {
    let directory = tempdir().unwrap();
    let source_path = directory.path().join("source-catalog.db");
    let target_path = directory.path().join("target-catalog.db");
    let record = StoredDataSourceRecord {
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
        updated_at_ms: 100,
        deleted_at_ms: None,
    };
    let created = {
        let mut source = NolaStore::open(&source_path).unwrap();
        source.upsert_data_source(&record).unwrap();
        source.upsert_data_source(&record).unwrap();
        let mut pending = source.pending_sync_changes(10).unwrap();
        assert_eq!(
            pending.len(),
            1,
            "no-op upsert must not manufacture a same-revision operation"
        );
        pending.remove(0)
    };
    {
        let mut target = NolaStore::open(&target_path).unwrap();
        target
            .apply_remote_sync_page(&ApplyRemoteSyncPageInput {
                replica_id: "source-replica".into(),
                changes: vec![remote_change(1, &created)],
                next_cursor: 1,
                applied_at_ms: 200,
            })
            .unwrap();
        let saved = target.data_source("sales").unwrap().unwrap();
        assert_eq!(saved.revision, 1);
        assert!(saved.validation_contract_json.contains("readRecipe"));
        assert!(target.pending_sync_changes(10).unwrap().is_empty());
    }
    let deleted = {
        let mut source = NolaStore::open(&source_path).unwrap();
        source.delete_data_source("sales", 300).unwrap();
        source
            .pending_sync_changes(10)
            .unwrap()
            .into_iter()
            .find(|change| change.deleted)
            .unwrap()
    };
    {
        let mut target = NolaStore::open(&target_path).unwrap();
        target
            .apply_remote_sync_page(&ApplyRemoteSyncPageInput {
                replica_id: "source-replica".into(),
                changes: vec![remote_change(2, &deleted)],
                next_cursor: 2,
                applied_at_ms: 400,
            })
            .unwrap();
    }
    let target = NolaStore::open(&target_path).unwrap();
    assert!(target.data_source("sales").unwrap().is_none());
    assert_eq!(
        target
            .sync_cursor("source-replica")
            .unwrap()
            .unwrap()
            .cursor,
        2
    );
}

fn automation(id: &str, name: &str, updated_at_ms: i64) -> StoredAutomationDefinition {
    StoredAutomationDefinition {
        execution_spec: serde_json::from_value(serde_json::json!({"kind":"agent","spec":{"prompt":"Execute this task once","execution_scope":{"kind":"local_workspace","root":"/fixture"},"permission_profile":"cautious","model_selection":"use_current_default_at_admission","context_refs":[],"skill_refs":[]}})).unwrap(),
        id: id.into(),
        name: name.into(),
        schedule_format: "cron".into(),
        enabled: true,
        revision: 0,
        metadata_json: format!(r#"{{"id":"{id}","format":"cron"}}"#),
        created_at_ms: 100,
        updated_at_ms,
        deleted_at_ms: None,
    }
}

fn model_profile(
    id: &str,
    selected: bool,
    temperature: f64,
    updated_at_ms: i64,
) -> StoredModelProfile {
    StoredModelProfile {
        id: id.into(),
        display_name: id.into(),
        provider: "local".into(),
        model_name: id.into(),
        runtime: "mlx".into(),
        source_json: "{}".into(),
        parameters_json: format!(r#"{{"temperature":{temperature}}}"#),
        selected,
        enabled: true,
        revision: 0,
        created_at_ms: 100,
        updated_at_ms,
        deleted_at_ms: None,
    }
}

fn memory(id: &str, summary: &str, updated_at_ms: i64) -> StoredMemoryRecord {
    StoredMemoryRecord {
        id: id.into(),
        owner_json: serde_json::to_string(&nola_domain::memory_owner::MemoryOwner::local_profile()).unwrap(),
        namespace_json: r#"{"kind":"personal"}"#.into(),
        scope: "global".into(),
        kind: "preference".into(),
        content: "Prefer concise answers".into(),
        summary: Some(summary.into()),
        tags_json: "[]".into(),
        confidence: 0.9,
        importance: 0.8,
        sensitivity: None,
        retention_json: r#"{"mode":"durable"}"#.into(),
        expires_at: None,
        supersedes_json: "[]".into(),
        provenance_json: "{}".into(),
        created_at_ms: 100,
        updated_at_ms,
        last_used_at_ms: None,
        deleted_at_ms: None,
    }
}

fn skill(version: &str, updated_at_ms: i64) -> UpsertSkillInput {
    UpsertSkillInput {
        id: "document-author".into(),
        source: SkillSourceKind::User,
        version: version.into(),
        display_name: "Document Author".into(),
        summary: "Create verified documents".into(),
        category: "documents".into(),
        instructions_ref: format!("nola-skill://document-author/{version}"),
        instructions_digest: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
            .into(),
        instructions_text: "hello".into(),
        required_tools_json: Some("[]".into()),
        default_mode: Some("fork".into()),
        enabled: true,
        created_at_ms: 100,
        updated_at_ms,
    }
}

#[test]
fn skills_replicate_current_version_enabled_state_and_tombstone_without_echo() {
    let directory = tempdir().unwrap();
    let source_path = directory.path().join("skill-source.db");
    let target_path = directory.path().join("skill-target.db");
    let changes = {
        let mut source = NolaStore::open(&source_path).unwrap();
        source.upsert_skill(&skill("1", 100)).unwrap();
        source
            .set_skill_enabled("document-author", false, 200)
            .unwrap();
        source.upsert_skill(&skill("2", 300)).unwrap();
        source.upsert_skill(&skill("2", 400)).unwrap();
        let pending = source.pending_sync_changes(10).unwrap();
        assert_eq!(pending.len(), 3, "same immutable version must be a no-op");
        pending
    };
    {
        let mut target = NolaStore::open(&target_path).unwrap();
        target
            .apply_remote_sync_page(&ApplyRemoteSyncPageInput {
                replica_id: "skill-replica".into(),
                changes: changes
                    .iter()
                    .enumerate()
                    .map(|(index, change)| remote_change(index as i64 + 1, change))
                    .collect(),
                next_cursor: 3,
                applied_at_ms: 500,
            })
            .unwrap();
        let saved = target.skill("document-author").unwrap().unwrap();
        assert_eq!(saved.version, "2");
        assert!(
            !saved.enabled,
            "installing a new version must preserve the user's disabled state"
        );
        assert!(target.pending_sync_changes(10).unwrap().is_empty());
    }
    let tombstone = {
        let mut source = NolaStore::open(&source_path).unwrap();
        source.delete_user_skill("document-author", 600).unwrap();
        source
            .pending_sync_changes(10)
            .unwrap()
            .into_iter()
            .find(|change| change.deleted)
            .unwrap()
    };
    {
        let mut target = NolaStore::open(&target_path).unwrap();
        target
            .apply_remote_sync_page(&ApplyRemoteSyncPageInput {
                replica_id: "skill-replica".into(),
                changes: vec![remote_change(4, &tombstone)],
                next_cursor: 4,
                applied_at_ms: 700,
            })
            .unwrap();
    }
    let target = NolaStore::open(&target_path).unwrap();
    assert!(target
        .skill("document-author")
        .unwrap()
        .unwrap()
        .deleted_at_ms
        .is_some());
    assert_eq!(
        target.sync_cursor("skill-replica").unwrap().unwrap().cursor,
        4
    );
}

#[test]
fn memories_replicate_deduplicated_projection_and_tombstone_without_echo() {
    let directory = tempdir().unwrap();
    let source_path = directory.path().join("memory-source.db");
    let target_path = directory.path().join("memory-target.db");
    let changes = {
        let mut source = NolaStore::open(&source_path).unwrap();
        source
            .save_memory_record_deduplicated(&memory("first", "initial", 100))
            .unwrap();
        source
            .save_memory_record_deduplicated(&memory("different-id", "refined", 200))
            .unwrap();
        source
            .save_memory_record_deduplicated(&memory("third-id", "refined", 300))
            .unwrap();
        let pending = source.pending_sync_changes(10).unwrap();
        assert_eq!(
            pending.len(),
            2,
            "deduplication and no-op save must preserve one causal identity"
        );
        assert!(pending.iter().all(|change| change.entity_id == "first"));
        pending
    };
    {
        let mut target = NolaStore::open(&target_path).unwrap();
        target
            .apply_remote_sync_page(&ApplyRemoteSyncPageInput {
                replica_id: "memory-replica".into(),
                changes: changes
                    .iter()
                    .enumerate()
                    .map(|(index, change)| remote_change(index as i64 + 1, change))
                    .collect(),
                next_cursor: 2,
                applied_at_ms: 400,
            })
            .unwrap();
        assert_eq!(
            target
                .memory_record("first")
                .unwrap()
                .unwrap()
                .summary
                .as_deref(),
            Some("refined")
        );
        assert!(target.pending_sync_changes(10).unwrap().is_empty());
    }
    let tombstone = {
        let mut source = NolaStore::open(&source_path).unwrap();
        source.delete_memory_record("first", 500).unwrap();
        source
            .pending_sync_changes(10)
            .unwrap()
            .into_iter()
            .find(|change| change.deleted)
            .unwrap()
    };
    {
        let mut target = NolaStore::open(&target_path).unwrap();
        target
            .apply_remote_sync_page(&ApplyRemoteSyncPageInput {
                replica_id: "memory-replica".into(),
                changes: vec![remote_change(3, &tombstone)],
                next_cursor: 3,
                applied_at_ms: 600,
            })
            .unwrap();
    }
    let target = NolaStore::open(&target_path).unwrap();
    assert!(target.memory_record("first").unwrap().is_none());
    assert_eq!(
        target
            .sync_cursor("memory-replica")
            .unwrap()
            .unwrap()
            .cursor,
        3
    );
}

#[test]
fn model_profiles_replicate_parameters_selection_and_tombstone_without_echo() {
    let directory = tempdir().unwrap();
    let source_path = directory.path().join("model-source.db");
    let target_path = directory.path().join("model-target.db");
    let changes = {
        let mut source = NolaStore::open(&source_path).unwrap();
        source
            .upsert_model_profile(&model_profile("small", true, 0.2, 100))
            .unwrap();
        source
            .upsert_model_profile(&model_profile("large", false, 0.4, 110))
            .unwrap();
        source
            .upsert_model_profile(&model_profile("small", true, 0.2, 120))
            .unwrap();
        assert_eq!(
            source.pending_sync_changes(20).unwrap().len(),
            2,
            "no-op profile save must not allocate another revision"
        );
        source
            .upsert_model_profile(&model_profile("large", true, 0.7, 200))
            .unwrap();
        source.pending_sync_changes(20).unwrap()
    };
    assert_eq!(
        changes.len(),
        4,
        "selection change must version both affected profiles"
    );
    {
        let mut target = NolaStore::open(&target_path).unwrap();
        target
            .apply_remote_sync_page(&ApplyRemoteSyncPageInput {
                replica_id: "model-replica".into(),
                changes: changes
                    .iter()
                    .enumerate()
                    .map(|(index, change)| remote_change(index as i64 + 1, change))
                    .collect(),
                next_cursor: 4,
                applied_at_ms: 300,
            })
            .unwrap();
        let profiles = target.list_model_profiles().unwrap();
        assert_eq!(
            profiles.iter().filter(|profile| profile.selected).count(),
            1
        );
        let selected = profiles.iter().find(|profile| profile.selected).unwrap();
        assert_eq!(selected.id, "large");
        assert!(selected.parameters_json.contains("0.7"));
        assert!(target.pending_sync_changes(10).unwrap().is_empty());
    }
    let tombstone = {
        let mut source = NolaStore::open(&source_path).unwrap();
        source.delete_model_profile("large", 400).unwrap();
        source
            .pending_sync_changes(20)
            .unwrap()
            .into_iter()
            .find(|change| change.entity_id == "large" && change.deleted)
            .unwrap()
    };
    {
        let mut target = NolaStore::open(&target_path).unwrap();
        target
            .apply_remote_sync_page(&ApplyRemoteSyncPageInput {
                replica_id: "model-replica".into(),
                changes: vec![remote_change(5, &tombstone)],
                next_cursor: 5,
                applied_at_ms: 500,
            })
            .unwrap();
    }
    let target = NolaStore::open(&target_path).unwrap();
    assert!(target.model_profile("large").unwrap().is_none());
    assert_eq!(
        target.sync_cursor("model-replica").unwrap().unwrap().cursor,
        5
    );
}

#[test]
fn automation_reconcile_replicates_upserts_and_tombstones() {
    let directory = tempdir().unwrap();
    let source_path = directory.path().join("automation-source.db");
    let target_path = directory.path().join("automation-target.db");
    let initial = {
        let mut source = NolaStore::open(&source_path).unwrap();
        source
            .reconcile_automation_definitions(
                &[
                    automation("daily", "Daily summary", 100),
                    automation("weekly", "Weekly summary", 100),
                ],
                &["cron".into()],
                100,
            )
            .unwrap();
        source.pending_sync_changes(10).unwrap()
    };
    assert_eq!(initial.len(), 2);
    {
        let mut target = NolaStore::open(&target_path).unwrap();
        target
            .apply_remote_sync_page(&ApplyRemoteSyncPageInput {
                replica_id: "automation-replica".into(),
                changes: initial
                    .iter()
                    .enumerate()
                    .map(|(index, change)| remote_change(index as i64 + 1, change))
                    .collect(),
                next_cursor: 2,
                applied_at_ms: 200,
            })
            .unwrap();
        assert_eq!(target.list_automation_definitions().unwrap().len(), 2);
        assert!(target.pending_sync_changes(10).unwrap().is_empty());
    }
    let removal = {
        let mut source = NolaStore::open(&source_path).unwrap();
        source
            .reconcile_automation_definitions(&[automation("daily", "Daily summary", 300)], &["cron".into()], 300)
            .unwrap();
        let pending = source.pending_sync_changes(20).unwrap();
        assert_eq!(
            pending.iter().filter(|change| change.revision == 1).count(),
            2
        );
        pending
            .into_iter()
            .find(|change| change.entity_id == "weekly" && change.deleted)
            .unwrap()
    };
    {
        let mut target = NolaStore::open(&target_path).unwrap();
        target
            .apply_remote_sync_page(&ApplyRemoteSyncPageInput {
                replica_id: "automation-replica".into(),
                changes: vec![remote_change(3, &removal)],
                next_cursor: 3,
                applied_at_ms: 400,
            })
            .unwrap();
    }
    let target = NolaStore::open(&target_path).unwrap();
    assert!(target.automation_definition("weekly").unwrap().is_none());
    assert_eq!(target.list_automation_definitions().unwrap().len(), 1);
}

#[test]
fn two_cold_restarted_stores_replicate_setting_tombstone_and_next_revision() {
    let directory = tempdir().unwrap();
    let source_path = directory.path().join("source.db");
    let target_path = directory.path().join("target.db");

    let first = {
        let mut source = NolaStore::open(&source_path).unwrap();
        source
            .put_product_setting("assistant.memory.enabled", "true", 1_000)
            .unwrap();
        source.pending_sync_changes(10).unwrap().remove(0)
    };

    {
        let mut target = NolaStore::open(&target_path).unwrap();
        let result = target
            .apply_remote_sync_page(&ApplyRemoteSyncPageInput {
                replica_id: "account-replica".into(),
                changes: vec![remote_change(1, &first)],
                next_cursor: 1,
                applied_at_ms: 1_100,
            })
            .unwrap();
        assert_eq!(result.applied, 1);
        assert_eq!(
            target
                .product_setting("assistant.memory.enabled")
                .unwrap()
                .unwrap()
                .value_json,
            "true"
        );
    }

    let tombstone = {
        let mut source = NolaStore::open(&source_path).unwrap();
        source
            .delete_product_setting("assistant.memory.enabled")
            .unwrap();
        source
            .pending_sync_changes(10)
            .unwrap()
            .into_iter()
            .find(|change| change.deleted)
            .unwrap()
    };

    {
        let mut target = NolaStore::open(&target_path).unwrap();
        target
            .apply_remote_sync_page(&ApplyRemoteSyncPageInput {
                replica_id: "account-replica".into(),
                changes: vec![remote_change(2, &tombstone)],
                next_cursor: 2,
                applied_at_ms: 2_100,
            })
            .unwrap();
        assert!(target
            .product_setting("assistant.memory.enabled")
            .unwrap()
            .is_none());
        assert_eq!(
            target
                .sync_cursor("account-replica")
                .unwrap()
                .unwrap()
                .cursor,
            2
        );
    }

    {
        let mut target = NolaStore::open(&target_path).unwrap();
        let recreated = target
            .put_product_setting("assistant.memory.enabled", "false", 3_000)
            .unwrap();
        assert_eq!(recreated.revision, 3);
    }
}

#[test]
fn divergent_same_revision_rolls_back_entity_and_cursor() {
    let directory = tempdir().unwrap();
    let local_path = directory.path().join("local.db");
    let remote_path = directory.path().join("remote.db");
    let remote = {
        let mut store = NolaStore::open(&remote_path).unwrap();
        store
            .put_product_setting("theme", r#""remote""#, 1_000)
            .unwrap();
        store.pending_sync_changes(1).unwrap().remove(0)
    };
    let mut local = NolaStore::open(&local_path).unwrap();
    local
        .put_product_setting("theme", r#""local""#, 1_000)
        .unwrap();
    let error = local
        .apply_remote_sync_page(&ApplyRemoteSyncPageInput {
            replica_id: "account-replica".into(),
            changes: vec![remote_change(1, &remote)],
            next_cursor: 1,
            applied_at_ms: 2_000,
        })
        .unwrap_err();
    assert!(error.to_string().contains("diverges"));
    assert!(local.sync_cursor("account-replica").unwrap().is_none());
    assert_eq!(
        local.product_setting("theme").unwrap().unwrap().value_json,
        r#""local""#
    );
}
#[path = "support/outbox.rs"]
mod outbox_fixture;

#[test]
fn replica_cannot_reassign_memory_owner_and_rejects_the_whole_page() {
    let mut source = NolaStore::open(":memory:").unwrap();
    let mut target = NolaStore::open(":memory:").unwrap();
    source.upsert_memory_record(&memory("owned", "original", 100)).unwrap();
    let created = source.pending_sync_changes(10).unwrap().remove(0);
    target.apply_remote_sync_page(&ApplyRemoteSyncPageInput {
        replica_id: "owner-replica".into(), changes: vec![remote_change(1, &created)],
        next_cursor: 1, applied_at_ms: 100,
    }).unwrap();
    let original = target.memory_record("owned").unwrap().unwrap();
    let mut update = memory("owned", "updated", 200);
    update.content = "New content".into();
    source.upsert_memory_record(&update).unwrap();
    let updated = source.pending_sync_changes(10).unwrap().into_iter().find(|change| change.revision == 2).unwrap();
    let mut forged = remote_change(2, &updated);
    let mut payload: serde_json::Value = serde_json::from_str(&forged.payload_json).unwrap();
    payload["record"]["owner_json"] = serde_json::json!(r#"{"kind":"profile","profile_id":"foreign"}"#);
    forged.payload_json = payload.to_string();
    let error = target.apply_remote_sync_page(&ApplyRemoteSyncPageInput {
        replica_id: "owner-replica".into(), changes: vec![forged], next_cursor: 2, applied_at_ms: 200,
    }).unwrap_err();
    assert!(error.to_string().contains("owner_conflict"), "{error}");
    assert_eq!(target.memory_record("owned").unwrap().unwrap(), original);
    assert_eq!(target.sync_cursor("owner-replica").unwrap().unwrap().cursor, 1);
    assert!(target.pending_sync_changes(10).unwrap().is_empty());
}
