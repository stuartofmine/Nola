// SPDX-License-Identifier: Apache-2.0
//! Sync resolution, cursor persistence, and atomic remote page application.

use super::*;

impl NolaStore {
    pub fn resolve_sync_change(
        &mut self,
        operation_id: &str,
        status: &str,
        remote_cursor: Option<i64>,
        resolved_at_ms: i64,
    ) -> StoreResult<bool> {
        require_id("sync.operation_id", operation_id)?;
        if !matches!(status, "acknowledged" | "unchanged" | "conflict") {
            return Err(StoreError::Invariant(format!(
                "invalid sync status {status}"
            )));
        }
        Ok(self.connection.execute(
            "UPDATE sync_outbox SET status=?2,remote_cursor=?3,resolved_at_ms=?4
             WHERE operation_id=?1 AND status='pending'",
            params![operation_id, status, remote_cursor, resolved_at_ms],
        )? == 1)
    }

    pub fn sync_cursor(&self, replica_id: &str) -> StoreResult<Option<StoredSyncCursor>> {
        require_id("sync.replica_id", replica_id)?;
        self.connection
            .query_row(
                "SELECT replica_id,cursor,updated_at_ms FROM sync_cursors WHERE replica_id=?1",
                params![replica_id],
                |row| {
                    Ok(StoredSyncCursor {
                        replica_id: row.get(0)?,
                        cursor: row.get(1)?,
                        updated_at_ms: row.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(StoreError::from)
    }

    pub fn save_sync_cursor(
        &mut self,
        replica_id: &str,
        cursor: i64,
        updated_at_ms: i64,
    ) -> StoreResult<StoredSyncCursor> {
        require_id("sync.replica_id", replica_id)?;
        if cursor < 0 {
            return Err(StoreError::Invariant(
                "sync cursor must be non-negative".to_string(),
            ));
        }
        self.connection.execute(
            "INSERT INTO sync_cursors(replica_id,cursor,updated_at_ms) VALUES(?1,?2,?3)
             ON CONFLICT(replica_id) DO UPDATE SET
               cursor=MAX(sync_cursors.cursor,excluded.cursor),
               updated_at_ms=CASE WHEN excluded.cursor>=sync_cursors.cursor
                                  THEN excluded.updated_at_ms ELSE sync_cursors.updated_at_ms END",
            params![replica_id, cursor, updated_at_ms],
        )?;
        self.sync_cursor(replica_id)?.ok_or_else(|| {
            StoreError::Invariant("sync cursor disappeared after upsert".to_string())
        })
    }

    pub fn apply_remote_sync_page(
        &mut self,
        input: &ApplyRemoteSyncPageInput,
    ) -> StoreResult<ApplyRemoteSyncPageResult> {
        require_id("sync.replica_id", &input.replica_id)?;
        if input.next_cursor < 0 {
            return Err(StoreError::Invariant(
                "sync cursor must be non-negative".to_string(),
            ));
        }
        let transaction = self.write_transaction()?;
        let current_cursor = transaction
            .query_row(
                "SELECT cursor FROM sync_cursors WHERE replica_id=?1",
                params![input.replica_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .unwrap_or(0);
        if input.next_cursor < current_cursor {
            return Err(StoreError::Conflict(format!(
                "sync cursor {} would regress from {current_cursor}",
                input.next_cursor
            )));
        }

        let mut previous_cursor = current_cursor;
        let mut applied = 0usize;
        let mut skipped = 0usize;
        for change in &input.changes {
            require_id("sync.operation_id", &change.operation_id)?;
            require_id("sync.entity_type", &change.entity_type)?;
            require_id("sync.entity_id", &change.entity_id)?;
            if change.entity_type == "conversation_metadata"
                && change.payload_json.len() > 512 * 1024
            {
                return Err(StoreError::Invariant(
                    "conversation metadata envelope exceeds 512 KiB".into(),
                ));
            }
            validate_json("sync.payload_json", &change.payload_json)?;
            if change.cursor <= previous_cursor || change.cursor > input.next_cursor {
                return Err(StoreError::Invariant(format!(
                    "remote change cursor {} is outside ({previous_cursor}, {}]",
                    change.cursor, input.next_cursor
                )));
            }
            if change.revision <= 0 {
                return Err(StoreError::Invariant(
                    "remote revision must be positive".to_string(),
                ));
            }
            previous_cursor = change.cursor;

            let marker = transaction
                .query_row(
                    "SELECT revision,operation_id,deleted FROM sync_entity_revisions
                     WHERE entity_type=?1 AND entity_id=?2",
                    params![change.entity_type, change.entity_id],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, bool>(2)?,
                        ))
                    },
                )
                .optional()?;
            if let Some((revision, operation_id, deleted)) = marker {
                if change.entity_type == "conversation" && deleted && !change.deleted {
                    // Stable conversation identities are never resurrected by
                    // delayed live replicas, even with a larger revision.
                    skipped += 1;
                    continue;
                }
                if revision > change.revision
                    || (revision == change.revision
                        && operation_id.as_deref() == Some(change.operation_id.as_str()))
                {
                    if matches!(
                        change.entity_type.as_str(),
                        "canonical_text_chunk"
                            | "canonical_text_seal"
                            | "archive_restore_begin"
                            | "archive_restore_entry"
                            | "archive_restore_target"
                            | "archive_restore_commit"
                            | "part_derivation"
                            | "conversation_branch_lineage"
                    ) {
                        // Immutable content identity never authorizes changed
                        // replay bytes, even when its operation was seen before.
                        if change.entity_type == "part_derivation" {
                            part_derivation_sync::apply(&transaction, change)?;
                        } else if change.entity_type == "conversation_branch_lineage" {
                            conversation_branch::apply_lineage(&transaction, change)?;
                        } else if change.entity_type.starts_with("archive_restore_") {
                            archive_restore_sync::apply(&transaction, change)?;
                        } else {
                            canonical_text_replica::apply(&transaction, change)?;
                        }
                    }
                    skipped += 1;
                    continue;
                }
                if revision == change.revision {
                    return Err(StoreError::Conflict(format!(
                        "remote {} {} revision {} diverges from local operation",
                        change.entity_type, change.entity_id, change.revision
                    )));
                }
            }

            match change.entity_type.as_str() {
                "product_setting" => {
                    if change.entity_id.starts_with(nola_domain::edited_resend::EDITED_RESEND_PREFIX) {
                        return Err(StoreError::Invariant("edited_resend_source_records_are_not_replica_authority".into()));
                    }
                    if change.deleted {
                        transaction.execute(
                            "DELETE FROM product_settings WHERE key=?1",
                            params![change.entity_id],
                        )?;
                    } else {
                        let stored: StoredProductSetting =
                            serde_json::from_str(&change.payload_json)?;
                        if stored.key != change.entity_id || stored.revision != change.revision {
                            return Err(StoreError::Invariant(
                                "remote product setting identity/revision mismatch".to_string(),
                            ));
                        }
                        validate_json("product_setting.value_json", &stored.value_json)?;
                        transaction.execute(
                            "INSERT INTO product_settings(key,value_json,revision,updated_at_ms)
                             VALUES(?1,?2,?3,?4)
                             ON CONFLICT(key) DO UPDATE SET
                               value_json=excluded.value_json,
                               revision=excluded.revision,
                               updated_at_ms=excluded.updated_at_ms",
                            params![
                                stored.key,
                                stored.value_json,
                                stored.revision,
                                stored.updated_at_ms
                            ],
                        )?;
                    }
                }
                "part_derivation" => {
                    part_derivation_sync::apply(&transaction, change)?;
                }
                "canonical_text_chunk" | "canonical_text_seal" => {
                    canonical_text_replica::apply(&transaction, change)?;
                }
                "archive_restore_begin"
                | "archive_restore_entry"
                | "archive_restore_target"
                | "archive_restore_commit" => {
                    archive_restore_sync::apply(&transaction, change)?;
                }
                "conversation_branch_lineage" => {
                    conversation_branch::apply_lineage(&transaction, change)?;
                }
                "conversation_append" => {
                    if !conversation_append_sync::apply(&transaction, change)? {
                        skipped += 1;
                        continue;
                    }
                }
                "conversation_transition" => {
                    if !conversation_transition_sync::apply(&transaction, change)? {
                        skipped += 1;
                        continue;
                    }
                }
                "conversation_metadata" => {
                    if !conversation_metadata_sync::apply(&transaction, change)? {
                        skipped += 1;
                        continue;
                    }
                }
                "conversation" => {
                    if change.deleted {
                        conversation_deletion::require_inactive_conversation(
                            &transaction, &change.entity_id,
                        )?;
                        transaction.execute(
                            "DELETE FROM data_source_overrides WHERE conversation_id=?1",
                            params![change.entity_id],
                        )?;
                        transaction.execute(
                            "DELETE FROM conversations WHERE id=?1",
                            params![change.entity_id],
                        )?;
                    } else {
                        let mut stored: StoredConversation =
                            serde_json::from_str(&change.payload_json)?;
                        if stored.id != change.entity_id || stored.revision != change.revision {
                            return Err(StoreError::Invariant(
                                "remote conversation identity/revision mismatch".to_string(),
                            ));
                        }
                        conversation_metadata_sync::preserve_newer_header(
                            &transaction,
                            &mut stored,
                        )?;
                        replace_conversation_projection(&transaction, &stored)?;
                    }
                }
                "mcp_server" => {
                    if change.deleted {
                        transaction.execute(
                            "UPDATE mcp_servers SET enabled=0,deleted_at_ms=?2,updated_at_ms=?2,
                                revision=?3 WHERE id=?1",
                            params![change.entity_id, change.created_at_ms, change.revision],
                        )?;
                    } else {
                        let stored: StoredMcpServerRecord =
                            serde_json::from_str(&change.payload_json)?;
                        if stored.id != change.entity_id || stored.revision != change.revision {
                            return Err(StoreError::Invariant(
                                "remote MCP server identity/revision mismatch".to_string(),
                            ));
                        }
                        validate_mcp_server(&stored)?;
                        transaction.execute(
                            "INSERT INTO mcp_servers(id,display_name,enabled,configuration_json,revision,
                                created_at_ms,updated_at_ms,deleted_at_ms)
                             VALUES(?1,?2,?3,?4,?5,?6,?7,NULL)
                             ON CONFLICT(id) DO UPDATE SET display_name=excluded.display_name,
                                enabled=excluded.enabled,configuration_json=excluded.configuration_json,
                                revision=excluded.revision,updated_at_ms=excluded.updated_at_ms,
                                deleted_at_ms=NULL",
                            params![stored.id, stored.display_name, stored.enabled, stored.configuration_json,
                                stored.revision, stored.created_at_ms, stored.updated_at_ms],
                        )?;
                    }
                }
                "data_source" => {
                    if change.deleted {
                        transaction.execute(
                            "UPDATE data_sources SET enabled=0,deleted_at_ms=?2,updated_at_ms=?2,
                                revision=?3 WHERE id=?1",
                            params![change.entity_id, change.created_at_ms, change.revision],
                        )?;
                        transaction.execute(
                            "DELETE FROM data_source_overrides WHERE source_id=?1",
                            params![change.entity_id],
                        )?;
                    } else {
                        let stored: StoredDataSourceRecord =
                            serde_json::from_str(&change.payload_json)?;
                        if stored.id != change.entity_id || stored.revision != change.revision {
                            return Err(StoreError::Invariant(
                                "remote data source identity/revision mismatch".to_string(),
                            ));
                        }
                        validate_data_source(&stored)?;
                        transaction.execute(
                            "INSERT INTO data_sources(id,name,kind,location,enabled,access_policy,
                                validation_contract_json,capabilities_json,privacy,metadata_json,
                                revision,created_at_ms,updated_at_ms,deleted_at_ms)
                             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,NULL)
                             ON CONFLICT(id) DO UPDATE SET name=excluded.name,kind=excluded.kind,
                                location=excluded.location,enabled=excluded.enabled,
                                access_policy=excluded.access_policy,
                                validation_contract_json=excluded.validation_contract_json,
                                capabilities_json=excluded.capabilities_json,privacy=excluded.privacy,
                                metadata_json=excluded.metadata_json,revision=excluded.revision,
                                updated_at_ms=excluded.updated_at_ms,deleted_at_ms=NULL",
                            params![stored.id, stored.name, stored.kind, stored.location, stored.enabled,
                                stored.access_policy, stored.validation_contract_json,
                                stored.capabilities_json, stored.privacy, stored.metadata_json,
                                stored.revision, stored.created_at_ms, stored.updated_at_ms],
                        )?;
                    }
                }
                "memory" => {
                    if change.deleted {
                        transaction.execute(
                            "UPDATE memory_records SET deleted_at_ms=?2,updated_at_ms=?2,
                                revision=?3 WHERE id=?1",
                            params![change.entity_id, change.created_at_ms, change.revision],
                        )?;
                    } else {
                        let projection: MemorySyncProjection =
                            serde_json::from_str(&change.payload_json)?;
                        if projection.record.id != change.entity_id
                            || projection.revision != change.revision
                        {
                            return Err(StoreError::Invariant(
                                "remote memory identity/revision mismatch".to_string(),
                            ));
                        }
                        validate_memory_record(&projection.record)?;
                        replace_memory_projection(
                            &transaction,
                            &projection.record,
                            projection.revision,
                        )?;
                    }
                }
                "skill" => {
                    if change.deleted {
                        transaction.execute(
                            "UPDATE skill_definitions SET enabled=0,deleted_at_ms=?2,
                                updated_at_ms=?2,revision=?3 WHERE id=?1",
                            params![change.entity_id, change.created_at_ms, change.revision],
                        )?;
                    } else {
                        let projection: SkillSyncProjection =
                            serde_json::from_str(&change.payload_json)?;
                        if projection.record.id != change.entity_id
                            || projection.revision != change.revision
                        {
                            return Err(StoreError::Invariant(
                                "remote Skill identity/revision mismatch".to_string(),
                            ));
                        }
                        replace_skill_projection(
                            &transaction,
                            &projection.record,
                            projection.revision,
                        )?;
                    }
                }
                "automation" => {
                    if change.deleted {
                        transaction.execute(
                            "UPDATE automation_definitions SET enabled=0,deleted_at_ms=?2,
                                updated_at_ms=?2,revision=?3 WHERE id=?1",
                            params![change.entity_id, change.created_at_ms, change.revision],
                        )?;
                    } else {
                        let stored: StoredAutomationDefinition =
                            serde_json::from_str(&change.payload_json)?;
                        if stored.id != change.entity_id || stored.revision != change.revision {
                            return Err(StoreError::Invariant(
                                "remote automation identity/revision mismatch".to_string(),
                            ));
                        }
                        validate_automation_definition(&stored)?;
                        transaction.execute(
                            "INSERT INTO automation_definitions(id,name,schedule_format,enabled,revision,
                                metadata_json,created_at_ms,updated_at_ms,deleted_at_ms,execution_spec_json)
                             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,NULL,?9)
                             ON CONFLICT(id) DO UPDATE SET name=excluded.name,
                                schedule_format=excluded.schedule_format,enabled=excluded.enabled,
                                revision=excluded.revision,metadata_json=excluded.metadata_json,execution_spec_json=excluded.execution_spec_json,
                                updated_at_ms=excluded.updated_at_ms,deleted_at_ms=NULL",
                            params![stored.id, stored.name, stored.schedule_format, stored.enabled,
                                stored.revision, stored.metadata_json, stored.created_at_ms, stored.updated_at_ms,serde_json::to_string(&stored.execution_spec)?],
                        )?;
                    }
                }
                "model_profile" => {
                    if change.deleted {
                        transaction.execute(
                            "UPDATE model_profiles SET enabled=0,selected=0,deleted_at_ms=?2,
                                updated_at_ms=?2,revision=?3 WHERE id=?1",
                            params![change.entity_id, change.created_at_ms, change.revision],
                        )?;
                        transaction.execute(
                            "UPDATE model_runtimes SET phase='stopped',stopped_at_ms=?2,
                                updated_at_ms=?2,process_id=NULL,endpoint_json=NULL
                             WHERE profile_id=?1 AND phase<>'stopped'",
                            params![change.entity_id, change.created_at_ms],
                        )?;
                    } else {
                        let stored: StoredModelProfile =
                            serde_json::from_str(&change.payload_json)?;
                        if stored.id != change.entity_id || stored.revision != change.revision {
                            return Err(StoreError::Invariant(
                                "remote model profile identity/revision mismatch".to_string(),
                            ));
                        }
                        require_id("model_profile.provider", &stored.provider)?;
                        require_id("model_profile.model_name", &stored.model_name)?;
                        require_id("model_profile.runtime", &stored.runtime)?;
                        validate_json("model_profile.source_json", &stored.source_json)?;
                        validate_json("model_profile.parameters_json", &stored.parameters_json)?;
                        transaction.execute(
                            "INSERT INTO model_profiles(id,display_name,provider,model_name,runtime,
                                source_json,parameters_json,selected,enabled,revision,created_at_ms,
                                updated_at_ms,deleted_at_ms)
                             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,NULL)
                             ON CONFLICT(id) DO UPDATE SET display_name=excluded.display_name,
                                provider=excluded.provider,model_name=excluded.model_name,
                                runtime=excluded.runtime,source_json=excluded.source_json,
                                parameters_json=excluded.parameters_json,selected=excluded.selected,
                                enabled=excluded.enabled,revision=excluded.revision,
                                updated_at_ms=excluded.updated_at_ms,deleted_at_ms=NULL",
                            params![stored.id, stored.display_name, stored.provider, stored.model_name,
                                stored.runtime, stored.source_json, stored.parameters_json,
                                stored.selected, stored.enabled, stored.revision,
                                stored.created_at_ms, stored.updated_at_ms],
                        )?;
                    }
                }
                unsupported => {
                    return Err(StoreError::Invariant(format!(
                        "unsupported remote entity type {unsupported}"
                    )));
                }
            }
            transaction.execute(
                "INSERT INTO sync_entity_revisions(
                    entity_type,entity_id,revision,deleted,operation_id,updated_at_ms
                 ) VALUES(?1,?2,?3,?4,?5,?6)
                 ON CONFLICT(entity_type,entity_id) DO UPDATE SET
                    revision=excluded.revision,
                    deleted=excluded.deleted,
                    operation_id=excluded.operation_id,
                    updated_at_ms=excluded.updated_at_ms",
                params![
                    change.entity_type,
                    change.entity_id,
                    change.revision,
                    change.deleted,
                    change.operation_id,
                    change.created_at_ms
                ],
            )?;
            applied += 1;
        }
        if previous_cursor != input.next_cursor && !input.changes.is_empty() {
            return Err(StoreError::Invariant(format!(
                "page next cursor {} does not match last change cursor {previous_cursor}",
                input.next_cursor
            )));
        }
        transaction.execute(
            "INSERT INTO sync_cursors(replica_id,cursor,updated_at_ms) VALUES(?1,?2,?3)
             ON CONFLICT(replica_id) DO UPDATE SET
               cursor=MAX(sync_cursors.cursor,excluded.cursor),
               updated_at_ms=excluded.updated_at_ms",
            params![input.replica_id, input.next_cursor, input.applied_at_ms],
        )?;
        transaction.commit()?;
        Ok(ApplyRemoteSyncPageResult {
            applied,
            skipped,
            cursor: input.next_cursor,
        })
    }
}
