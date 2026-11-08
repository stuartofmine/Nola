// SPDX-License-Identifier: Apache-2.0
//! Durable automation definitions, scheduling claims, heartbeats, retries, and completion.

use super::*;

impl NolaStore {
    /// Update one live definition only if the exact reviewed revision still exists.
    /// The revision check, mutation, and sync event share a single transaction.
    pub fn update_automation_definition_if_revision(
        &mut self,
        definition: &StoredAutomationDefinition,
        expected_revision: i64,
    ) -> StoreResult<Option<StoredAutomationDefinition>> {
        validate_automation_definition(definition)?;
        if expected_revision < 1 { return Err(StoreError::Invariant("expected automation revision must be positive".into())); }
        let transaction = self.write_transaction()?;
        let changed = transaction.execute(
            "UPDATE automation_definitions SET name=?2,enabled=?3,metadata_json=?4,updated_at_ms=?5,revision=revision+1,execution_spec_json=?8
             WHERE id=?1 AND revision=?6 AND deleted_at_ms IS NULL AND schedule_format=?7",
            params![definition.id, definition.name, definition.enabled, definition.metadata_json, definition.updated_at_ms, expected_revision, definition.schedule_format, serde_json::to_string(&definition.execution_spec)?],
        )?;
        if changed == 0 { transaction.commit()?; return Ok(None); }
        let stored = transaction.query_row(
            &format!("SELECT {AUTOMATION_COLUMNS} FROM automation_definitions WHERE id=?1"),
            params![definition.id], automation_definition_row,
        )?;
        enqueue_sync_change(&transaction, "automation", &stored.id, stored.revision, false,
            &serde_json::to_string(&stored)?, stored.updated_at_ms)?;
        transaction.commit()?;
        Ok(Some(stored))
    }

    pub fn upsert_automation_definition(
        &mut self,
        definition: &StoredAutomationDefinition,
    ) -> StoreResult<StoredAutomationDefinition> {
        validate_automation_definition(definition)?;
        let transaction = self.write_transaction()?;
        let previous_revision = transaction
            .query_row(
                "SELECT revision FROM automation_definitions WHERE id=?1",
                params![definition.id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        transaction.execute(
            "INSERT INTO automation_definitions(id,name,schedule_format,enabled,revision,metadata_json,
                created_at_ms,updated_at_ms,deleted_at_ms,execution_spec_json)
             VALUES(?1,?2,?3,?4,1,?5,?6,?7,NULL,?8)
             ON CONFLICT(id) DO UPDATE SET name=excluded.name,schedule_format=excluded.schedule_format,
                enabled=excluded.enabled,
                revision=CASE WHEN automation_definitions.name<>excluded.name
                    OR automation_definitions.schedule_format<>excluded.schedule_format
                    OR automation_definitions.enabled<>excluded.enabled
                    OR automation_definitions.metadata_json<>excluded.metadata_json
                    OR automation_definitions.execution_spec_json<>excluded.execution_spec_json
                    OR automation_definitions.deleted_at_ms IS NOT NULL
                    THEN automation_definitions.revision+1 ELSE automation_definitions.revision END,
                metadata_json=excluded.metadata_json,execution_spec_json=excluded.execution_spec_json,
                updated_at_ms=CASE WHEN automation_definitions.name<>excluded.name
                    OR automation_definitions.schedule_format<>excluded.schedule_format
                    OR automation_definitions.enabled<>excluded.enabled
                    OR automation_definitions.metadata_json<>excluded.metadata_json
                    OR automation_definitions.execution_spec_json<>excluded.execution_spec_json
                    OR automation_definitions.deleted_at_ms IS NOT NULL
                    THEN excluded.updated_at_ms ELSE automation_definitions.updated_at_ms END,
                deleted_at_ms=NULL",
            params![definition.id,definition.name,definition.schedule_format,definition.enabled,
                definition.metadata_json,definition.created_at_ms,definition.updated_at_ms,serde_json::to_string(&definition.execution_spec)?],
        )?;
        let stored = transaction.query_row(
            &format!("SELECT {AUTOMATION_COLUMNS} FROM automation_definitions WHERE id=?1 AND deleted_at_ms IS NULL"),
            params![definition.id], automation_definition_row,
        )?;
        if previous_revision != Some(stored.revision) {
            let payload = serde_json::to_string(&stored)?;
            enqueue_sync_change(
                &transaction,
                "automation",
                &stored.id,
                stored.revision,
                false,
                &payload,
                stored.updated_at_ms,
            )?;
        }
        transaction.commit()?;
        Ok(stored)
    }

    pub fn reconcile_automation_definitions(
        &mut self,
        definitions: &[StoredAutomationDefinition],
        schedule_formats: &[String],
        reconciled_at_ms: i64,
    ) -> StoreResult<Vec<StoredAutomationDefinition>> {
        if schedule_formats.is_empty() || schedule_formats.iter().any(|format| format.trim().is_empty()) {
            return Err(StoreError::Invariant("automation reconciliation requires an explicit format scope".into()));
        }
        let mut ids = std::collections::BTreeSet::new();
        for definition in definitions {
            validate_automation_definition(definition)?;
            if !schedule_formats.contains(&definition.schedule_format) {
                return Err(StoreError::Invariant("automation is outside the reconciliation scope".into()));
            }
            if !ids.insert(definition.id.clone()) {
                return Err(StoreError::Invariant(format!(
                    "duplicate automation id {}",
                    definition.id
                )));
            }
        }
        let transaction = self.write_transaction()?;
        let active = {
            let mut statement = transaction.prepare(
                "SELECT id,revision,schedule_format FROM automation_definitions WHERE deleted_at_ms IS NULL",
            )?;
            let collected = statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?))
                })?
                .collect::<Result<Vec<_>, _>>()?
                .into_iter().filter(|(_, _, format)| schedule_formats.contains(format))
                .map(|(id, revision, _)| (id, revision)).collect::<Vec<_>>();
            collected
        };
        let previous_revisions = active
            .iter()
            .cloned()
            .collect::<std::collections::BTreeMap<_, _>>();
        for definition in definitions {
            let existing_format: Option<String> = transaction.query_row(
                "SELECT schedule_format FROM automation_definitions WHERE id=?1",
                params![definition.id], |row| row.get(0),
            ).optional()?;
            if existing_format.as_ref().is_some_and(|format| !schedule_formats.contains(format)) {
                return Err(StoreError::Invariant("automation identity belongs to another format scope".into()));
            }
            transaction.execute(
                "INSERT INTO automation_definitions(id,name,schedule_format,enabled,revision,metadata_json,
                    created_at_ms,updated_at_ms,deleted_at_ms,execution_spec_json)
                 VALUES(?1,?2,?3,?4,1,?5,?6,?7,NULL,?8)
                 ON CONFLICT(id) DO UPDATE SET name=excluded.name,schedule_format=excluded.schedule_format,
                    enabled=excluded.enabled,
                    revision=CASE WHEN automation_definitions.name<>excluded.name
                        OR automation_definitions.schedule_format<>excluded.schedule_format
                        OR automation_definitions.enabled<>excluded.enabled
                        OR automation_definitions.metadata_json<>excluded.metadata_json
                        OR automation_definitions.execution_spec_json<>excluded.execution_spec_json
                        OR automation_definitions.deleted_at_ms IS NOT NULL
                        THEN automation_definitions.revision+1 ELSE automation_definitions.revision END,
                    metadata_json=excluded.metadata_json,execution_spec_json=excluded.execution_spec_json,
                    updated_at_ms=CASE WHEN automation_definitions.name<>excluded.name
                        OR automation_definitions.schedule_format<>excluded.schedule_format
                        OR automation_definitions.enabled<>excluded.enabled
                        OR automation_definitions.metadata_json<>excluded.metadata_json
                        OR automation_definitions.execution_spec_json<>excluded.execution_spec_json
                        OR automation_definitions.deleted_at_ms IS NOT NULL
                        THEN excluded.updated_at_ms ELSE automation_definitions.updated_at_ms END,
                    deleted_at_ms=NULL",
                params![definition.id,definition.name,definition.schedule_format,definition.enabled,
                    definition.metadata_json,definition.created_at_ms,definition.updated_at_ms,serde_json::to_string(&definition.execution_spec)?],
            )?;
            let stored = transaction.query_row(
                &format!("SELECT {AUTOMATION_COLUMNS} FROM automation_definitions WHERE id=?1 AND deleted_at_ms IS NULL"),
                params![definition.id], automation_definition_row,
            )?;
            if previous_revisions.get(&stored.id).copied() != Some(stored.revision) {
                let payload = serde_json::to_string(&stored)?;
                enqueue_sync_change(
                    &transaction,
                    "automation",
                    &stored.id,
                    stored.revision,
                    false,
                    &payload,
                    stored.updated_at_ms,
                )?;
            }
        }
        for (id, _) in active {
            if !ids.contains(&id) {
                let changed = transaction.execute(
                    "UPDATE automation_definitions SET deleted_at_ms=?2,updated_at_ms=?2,
                        revision=revision+1 WHERE id=?1 AND deleted_at_ms IS NULL",
                    params![id, reconciled_at_ms],
                )?;
                if changed == 1 {
                    let revision: i64 = transaction.query_row(
                        "SELECT revision FROM automation_definitions WHERE id=?1",
                        params![id],
                        |row| row.get(0),
                    )?;
                    enqueue_sync_change(
                        &transaction,
                        "automation",
                        &id,
                        revision,
                        true,
                        "null",
                        reconciled_at_ms,
                    )?;
                }
            }
        }
        transaction.commit()?;
        Ok(self.list_automation_definitions()?.into_iter()
            .filter(|definition| schedule_formats.contains(&definition.schedule_format)).collect())
    }

    pub fn automation_definition(
        &self,
        id: &str,
    ) -> StoreResult<Option<StoredAutomationDefinition>> {
        self.connection.query_row(
            &format!("SELECT {AUTOMATION_COLUMNS} FROM automation_definitions WHERE id=?1 AND deleted_at_ms IS NULL"),
            params![id], automation_definition_row,
        ).optional().map_err(StoreError::from)
    }

    pub fn list_automation_definitions(&self) -> StoreResult<Vec<StoredAutomationDefinition>> {
        let mut statement =
            self.connection.prepare(&format!(
            "SELECT {AUTOMATION_COLUMNS} FROM automation_definitions WHERE deleted_at_ms IS NULL
             ORDER BY updated_at_ms DESC,id ASC"))?;
        let rows = statement.query_map([], automation_definition_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    pub fn delete_automation_definition(
        &mut self,
        id: &str,
        deleted_at_ms: i64,
    ) -> StoreResult<bool> {
        require_id("automation_definition.id", id)?;
        let transaction = self.write_transaction()?;
        let changed = transaction.execute(
            "UPDATE automation_definitions SET enabled=0,deleted_at_ms=?2,
                updated_at_ms=?2,revision=revision+1
             WHERE id=?1 AND deleted_at_ms IS NULL",
            params![id, deleted_at_ms],
        )? == 1;
        if changed {
            let revision: i64 = transaction.query_row(
                "SELECT revision FROM automation_definitions WHERE id=?1",
                params![id],
                |row| row.get(0),
            )?;
            enqueue_sync_change(
                &transaction,
                "automation",
                id,
                revision,
                true,
                "null",
                deleted_at_ms,
            )?;
        }
        transaction.commit()?;
        Ok(changed)
    }

    pub fn enqueue_automation_run(
        &mut self,
        run: &StoredAutomationRun,
    ) -> StoreResult<StoredAutomationRun> {
        validate_automation_run(run)?;
        if let Some(existing) = self.connection.query_row(
            &format!("SELECT {AUTOMATION_RUN_COLUMNS} FROM automation_runs WHERE automation_id=?1 AND occurrence_key=?2"),
            params![run.automation_id, run.occurrence_key], automation_run_row,
        ).optional()? {
            if existing.definition_revision != run.definition_revision {
                return Err(StoreError::Conflict("automation occurrence is bound to another definition revision".into()));
            }
            return Ok(existing);
        }
        self.automation_definition(&run.automation_id)?
            .ok_or_else(|| StoreError::Invariant("automation definition missing".into()))?
            .execution_spec.agent().map_err(StoreError::Invariant)?;
        if run.status != "queued" || run.attempt != 0 {
            return Err(StoreError::Invariant(
                "new automation run must be queued with attempt 0".into(),
            ));
        }
        self.connection.execute(
            "INSERT INTO automation_runs(
                id,automation_id,occurrence_key,trigger,status,attempt,worker_id,lease_token,
                lease_until_ms,scheduled_for_ms,started_at_ms,finished_at_ms,next_retry_at_ms,
                result_json,error_json,created_at_ms,updated_at_ms,definition_revision
             ) SELECT ?1,?2,?3,?4,'queued',0,NULL,NULL,NULL,?5,NULL,NULL,NULL,NULL,NULL,?6,?7,?8
             WHERE EXISTS(SELECT 1 FROM automation_definitions WHERE id=?2
                AND revision=?8 AND enabled=1 AND deleted_at_ms IS NULL
                AND json_extract(execution_spec_json,'$.kind')='agent')
             ON CONFLICT(automation_id,occurrence_key) DO NOTHING",
            params![
                run.id,
                run.automation_id,
                run.occurrence_key,
                run.trigger,
                run.scheduled_for_ms,
                run.created_at_ms,
                run.updated_at_ms,
                run.definition_revision
            ],
        )?;
        let committed = self.connection
            .query_row(
                &format!(
                    "SELECT {AUTOMATION_RUN_COLUMNS} FROM automation_runs
                WHERE automation_id=?1 AND occurrence_key=?2"
                ),
                params![run.automation_id, run.occurrence_key],
                automation_run_row,
            )
            .map_err(StoreError::from)?;
        if committed.definition_revision != run.definition_revision {
            return Err(StoreError::Conflict("automation occurrence is bound to another definition revision".into()));
        }
        Ok(committed)
    }

    pub fn claim_automation_run(
        &mut self,
        id: &str,
        worker_id: &str,
        lease_token: &str,
        now_ms: i64,
        lease_until_ms: i64,
    ) -> StoreResult<bool> {
        require_id("automation_run.id", id)?;
        let occurrence = self.automation_run(id)?
            .ok_or_else(|| StoreError::Invariant("automation occurrence missing".into()))?;
        self.automation_definition(&occurrence.automation_id)?
            .ok_or_else(|| StoreError::Invariant("automation definition missing".into()))?
            .execution_spec.agent().map_err(StoreError::Invariant)?;
        require_id("automation_run.worker_id", worker_id)?;
        require_id("automation_run.lease_token", lease_token)?;
        if lease_until_ms <= now_ms {
            return Err(StoreError::Invariant(
                "automation run lease must be in the future".into(),
            ));
        }
        Ok(self.connection.execute(
            "UPDATE automation_runs SET status='running',attempt=attempt+1,worker_id=?2,
                lease_token=?3,lease_until_ms=?5,started_at_ms=COALESCE(started_at_ms,?4),
                updated_at_ms=?4
             WHERE id=?1 AND EXISTS(SELECT 1 FROM automation_definitions d
                WHERE d.id=automation_runs.automation_id AND d.revision=automation_runs.definition_revision
                AND d.enabled=1 AND d.deleted_at_ms IS NULL AND json_extract(d.execution_spec_json,'$.kind')='agent')
             AND NOT EXISTS(SELECT 1 FROM automation_execution_bindings b WHERE b.occurrence_id=automation_runs.id) AND (
                (status IN ('queued','retry_wait') AND (next_retry_at_ms IS NULL OR next_retry_at_ms<=?4))
             )",
            params![id, worker_id, lease_token, now_ms, lease_until_ms],
        )? == 1)
    }

    pub fn heartbeat_automation_run(
        &mut self,
        id: &str,
        lease_token: &str,
        now_ms: i64,
        lease_until_ms: i64,
    ) -> StoreResult<bool> {
        if lease_until_ms <= now_ms {
            return Err(StoreError::Invariant(
                "automation run heartbeat must extend its lease".into(),
            ));
        }
        Ok(self.connection.execute(
            "UPDATE automation_runs SET lease_until_ms=?4,updated_at_ms=?3
             WHERE id=?1 AND status='running' AND lease_token=?2 AND lease_until_ms>?3",
            params![id, lease_token, now_ms, lease_until_ms],
        )? == 1)
    }

    pub fn retry_automation_run(
        &mut self,
        id: &str,
        lease_token: &str,
        next_retry_at_ms: i64,
        error_json: &str,
        updated_at_ms: i64,
    ) -> StoreResult<bool> {
        validate_json("automation_run.error_json", error_json)?;
        if next_retry_at_ms <= updated_at_ms {
            return Err(StoreError::Invariant(
                "automation retry must be scheduled in the future".into(),
            ));
        }
        Ok(self.connection.execute(
            "UPDATE automation_runs SET status='retry_wait',worker_id=NULL,lease_token=NULL,
                lease_until_ms=NULL,next_retry_at_ms=?3,error_json=?4,updated_at_ms=?5
             WHERE id=?1 AND status='running' AND lease_token=?2 AND lease_until_ms>?5
             AND NOT EXISTS(SELECT 1 FROM automation_execution_bindings b WHERE b.occurrence_id=automation_runs.id)
             AND NOT EXISTS(SELECT 1 FROM automation_worker_observations o WHERE o.occurrence_id=automation_runs.id
                 AND json_extract(o.observation_json,'$.kind')='admission_requested')",
            params![id, lease_token, next_retry_at_ms, error_json, updated_at_ms],
        )? == 1)
    }


    pub fn automation_run(&self, id: &str) -> StoreResult<Option<StoredAutomationRun>> {
        self.connection
            .query_row(
                &format!("SELECT {AUTOMATION_RUN_COLUMNS} FROM automation_runs WHERE id=?1"),
                params![id],
                automation_run_row,
            )
            .optional()
            .map_err(StoreError::from)
    }

    pub fn list_automation_runs(
        &self,
        automation_id: Option<&str>,
        limit: usize,
    ) -> StoreResult<Vec<StoredAutomationRun>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {AUTOMATION_RUN_COLUMNS} FROM automation_runs
             WHERE (?1 IS NULL OR automation_id=?1)
             ORDER BY scheduled_for_ms DESC,id DESC LIMIT ?2"
        ))?;
        let rows = statement.query_map(
            params![automation_id, limit.clamp(1, 2_000) as i64],
            automation_run_row,
        )?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }
}
