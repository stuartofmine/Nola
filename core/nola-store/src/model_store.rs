// SPDX-License-Identifier: Apache-2.0
//! Revisioned model profile selection, parameter configuration, runtime reporting, and sync.

use super::*;

#[cfg(test)]
#[path = "model_write_admission_tests.rs"]
mod write_admission_tests;

impl NolaStore {
    /// Edit an existing endpoint without replacing the remote-model catalog.
    /// Membership and every revision are checked inside the same savepoint as
    /// writes and sync publication, including models added after the UI loaded.
    pub fn configure_model_endpoint(
        &mut self,
        endpoint_id: &str,
        expected_revisions: &std::collections::BTreeMap<String, i64>,
        profiles: &[StoredModelProfile],
    ) -> StoreResult<()> {
        require_id("model_endpoint.id", endpoint_id)?;
        let mut transaction = self.write_transaction()?;
        let result = transaction.with_store(|store| -> StoreResult<()> {
            let mut existing = std::collections::BTreeMap::new();
            for profile in store.list_model_profiles()? {
                if profile.runtime != "remote" || profile.deleted_at_ms.is_some() { continue; }
                let source: serde_json::Value = serde_json::from_str(&profile.source_json)
                    .map_err(|error| StoreError::Invariant(error.to_string()))?;
                if source["endpointID"].as_str() == Some(endpoint_id) {
                    existing.insert(profile.id.clone(), profile);
                }
            }
            let revisions = existing.iter().map(|(id, profile)| (id.clone(), profile.revision)).collect::<std::collections::BTreeMap<_, _>>();
            if existing.is_empty() || &revisions != expected_revisions {
                return Err(StoreError::Invariant("revision_conflict: endpoint membership or model revision changed".into()));
            }
            let mut seen = std::collections::BTreeSet::new();
            for profile in profiles {
                let previous = existing.get(&profile.id).ok_or_else(|| StoreError::Invariant("endpoint update contains foreign model".into()))?;
                let mut unchanged = profile.clone();
                unchanged.source_json = previous.source_json.clone();
                unchanged.updated_at_ms = previous.updated_at_ms;
                if &unchanged != previous || !seen.insert(profile.id.clone()) {
                    return Err(StoreError::Invariant("endpoint update cannot change model identity, parameters or selection".into()));
                }
            }
            if seen.len() != existing.len() {
                return Err(StoreError::Invariant("endpoint update must include every model".into()));
            }
            for profile in profiles { store.upsert_model_profile(profile)?; }
            Ok(())
        });
        match result {
            Ok(()) => transaction.commit(),
            Err(error) => { transaction.rollback()?; Err(error) }
        }
    }

    /// Replaces one provider/runtime configuration as a single mutation,
    /// including selection displacement and sync outbox records.
    pub fn replace_model_profiles(
        &mut self,
        provider: &str,
        runtime: &str,
        profiles: &[StoredModelProfile],
        updated_at_ms: i64,
        expected_revisions: &std::collections::BTreeMap<String, i64>,
    ) -> StoreResult<()> {
        require_id("model_profile.provider", provider)?;
        require_id("model_profile.runtime", runtime)?;
        let mut ids = std::collections::HashSet::new();
        for profile in profiles {
            if profile.provider != provider
                || profile.runtime != runtime
                || profile.deleted_at_ms.is_some()
                || (profile.selected && !profile.enabled)
                || !ids.insert(profile.id.as_str())
            {
                return Err(StoreError::Invariant(
                    "invalid model replacement scope or duplicate identity".into(),
                ));
            }
        }
        let enabled_count = profiles.iter().filter(|profile| profile.enabled).count();
        let selected_count = profiles.iter().filter(|profile| profile.selected).count();
        if selected_count != usize::from(enabled_count > 0) {
            return Err(StoreError::Invariant(
                "model replacement requires one enabled selection when enabled models exist".into(),
            ));
        }
        let mut transaction = self.write_transaction()?;
        let result = transaction.with_store(|store| -> StoreResult<()> {
            let previous = store.list_model_profiles()?;
            let actual = previous.iter()
                .filter(|profile| profile.provider == provider && profile.runtime == runtime
                    && profile.deleted_at_ms.is_none())
                .map(|profile| (profile.id.clone(), profile.revision))
                .collect::<std::collections::BTreeMap<_, _>>();
            if &actual != expected_revisions {
                return Err(StoreError::Invariant(
                    "revision_conflict: model catalog membership or profile revision changed".into(),
                ));
            }
            for profile in profiles {
                if let Some(existing) = store.model_profile(&profile.id)? {
                    if existing.provider != provider || existing.runtime != runtime {
                        return Err(StoreError::Invariant(
                            "model identity belongs to another configuration scope".into(),
                        ));
                    }
                }
                store.upsert_model_profile(profile)?;
            }
            for profile in previous {
                if profile.provider == provider
                    && profile.runtime == runtime
                    && !ids.contains(profile.id.as_str())
                {
                    store.delete_model_profile(&profile.id, updated_at_ms)?;
                }
            }
            Ok(())
        });
        match result {
            Ok(()) => transaction.commit(),
            Err(error) => { transaction.rollback()?; Err(error) }
        }
    }

    /// An explicit catalog configuration may register a new model, but cannot
    /// overwrite an existing source/selection or resurrect a deleted model.
    pub fn configure_model_parameters(&mut self, input: &StoredModelProfile) -> StoreResult<()> {
        require_id("model_profile.id", &input.id)?;
        validate_json("model_profile.parameters_json", &input.parameters_json)?;
        if input.selected || !input.enabled || input.deleted_at_ms.is_some() {
            return Err(StoreError::Invariant(
                "parameter configuration cannot select or delete a model".into(),
            ));
        }
        let mut transaction = self.write_transaction()?;
        let result = transaction.with_store(|store| -> StoreResult<()> {
            let existing = store
                .connection
                .query_row(
                    &format!("SELECT {MODEL_PROFILE_COLUMNS} FROM model_profiles WHERE id=?1"),
                    params![input.id],
                    model_profile_row,
                )
                .optional()?;
            let mut profile = match existing {
                Some(existing) => {
                    if existing.deleted_at_ms.is_some() {
                        return Err(StoreError::Invariant(
                            "deleted model cannot be restored by parameter editing".into(),
                        ));
                    }
                    if existing.provider != input.provider || existing.runtime != input.runtime {
                        return Err(StoreError::Invariant(
                            "model identity belongs to another configuration scope".into(),
                        ));
                    }
                    existing
                }
                None => input.clone(),
            };
            profile.parameters_json = input.parameters_json.clone();
            profile.updated_at_ms = input.updated_at_ms;
            store.upsert_model_profile(&profile)?;
            Ok(())
        });
        match result {
            Ok(()) => transaction.commit(),
            Err(error) => { transaction.rollback()?; Err(error) }
        }
    }

    /// Updates parameters without trusting a client copy of model identity,
    /// selection, source, or lifecycle fields.
    pub fn reset_model_parameters(
        &mut self,
        profile_ids: &[String],
        parameters_json: &str,
        updated_at_ms: i64,
    ) -> StoreResult<()> {
        validate_json("model_profile.parameters_json", parameters_json)?;
        let mut seen = std::collections::HashSet::new();
        for id in profile_ids {
            require_id("model_profile.id", id)?;
            if !seen.insert(id) {
                return Err(StoreError::Invariant(
                    "duplicate model parameter reset identity".into(),
                ));
            }
        }
        let mut transaction = self.write_transaction()?;
        let result = transaction.with_store(|store| -> StoreResult<()> {
            for id in profile_ids {
                let mut profile = store.model_profile(id)?.ok_or_else(|| {
                    StoreError::Invariant(format!("model profile not found: {id}"))
                })?;
                profile.parameters_json = parameters_json.to_owned();
                profile.updated_at_ms = updated_at_ms;
                store.upsert_model_profile(&profile)?;
            }
            Ok(())
        });
        match result {
            Ok(()) => transaction.commit(),
            Err(error) => { transaction.rollback()?; Err(error) }
        }
    }

    pub fn upsert_model_profile(&mut self, profile: &StoredModelProfile) -> StoreResult<()> {
        require_id("model_profile.id", &profile.id)?;
        require_id("model_profile.provider", &profile.provider)?;
        require_id("model_profile.model_name", &profile.model_name)?;
        require_id("model_profile.runtime", &profile.runtime)?;
        validate_json("model_profile.source_json", &profile.source_json)?;
        validate_json("model_profile.parameters_json", &profile.parameters_json)?;
        let transaction = self.write_transaction()?;

        let existing = transaction
            .query_row(
                &format!("SELECT {MODEL_PROFILE_COLUMNS} FROM model_profiles WHERE id=?1"),
                params![profile.id],
                model_profile_row,
            )
            .optional()?;

        // Stable model IDs retain their provider/runtime ownership, including
        // tombstones. Enforce this in the common writer so direct upserts and
        // scoped batch commands cannot bypass one another's identity checks.
        if existing.as_ref().is_some_and(|stored| {
            stored.provider != profile.provider || stored.runtime != profile.runtime
        }) {
            return Err(StoreError::Invariant(
                "model identity belongs to another configuration scope".into(),
            ));
        }

        if profile.selected && profile.enabled && profile.deleted_at_ms.is_none() {
            let mut statement = transaction.prepare(&format!(
                "SELECT {MODEL_PROFILE_COLUMNS} FROM model_profiles
                 WHERE selected=1 AND enabled=1 AND deleted_at_ms IS NULL AND id<>?1
                 AND provider=?2 AND runtime=?3"
            ))?;
            let displaced = statement
                .query_map(params![profile.id, profile.provider, profile.runtime], model_profile_row)?
                .collect::<Result<Vec<_>, _>>()?;
            drop(statement);
            for previous in displaced {
                let revision = previous.revision.saturating_add(1);
                transaction.execute(
                    "UPDATE model_profiles SET selected=0,revision=?2,updated_at_ms=?3 WHERE id=?1",
                    params![previous.id, revision, profile.updated_at_ms],
                )?;
                let stored = transaction.query_row(
                    &format!("SELECT {MODEL_PROFILE_COLUMNS} FROM model_profiles WHERE id=?1"),
                    params![previous.id],
                    model_profile_row,
                )?;
                let payload = serde_json::to_string(&stored)?;
                enqueue_sync_change(
                    &transaction,
                    "model_profile",
                    &stored.id,
                    stored.revision,
                    false,
                    &payload,
                    stored.updated_at_ms,
                )?;
            }
        }

        let unchanged = existing.as_ref().is_some_and(|stored| {
            stored.display_name == profile.display_name
                && stored.provider == profile.provider
                && stored.model_name == profile.model_name
                && stored.runtime == profile.runtime
                && stored.source_json == profile.source_json
                && stored.parameters_json == profile.parameters_json
                && stored.selected == profile.selected
                && stored.enabled == profile.enabled
                && stored.deleted_at_ms == profile.deleted_at_ms
        });
        if !unchanged {
            let revision = existing
                .as_ref()
                .map_or(1, |stored| stored.revision.saturating_add(1));
            let created_at_ms = existing
                .as_ref()
                .map_or(profile.created_at_ms, |stored| stored.created_at_ms);
            transaction.execute(
                "INSERT INTO model_profiles(id,display_name,provider,model_name,runtime,source_json,
                    parameters_json,selected,enabled,revision,created_at_ms,updated_at_ms,deleted_at_ms)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)
                 ON CONFLICT(id) DO UPDATE SET display_name=excluded.display_name,
                    provider=excluded.provider,model_name=excluded.model_name,runtime=excluded.runtime,
                    source_json=excluded.source_json,parameters_json=excluded.parameters_json,
                    selected=excluded.selected,enabled=excluded.enabled,revision=excluded.revision,
                    updated_at_ms=excluded.updated_at_ms,deleted_at_ms=excluded.deleted_at_ms",
                params![
                    profile.id,
                    profile.display_name,
                    profile.provider,
                    profile.model_name,
                    profile.runtime,
                    profile.source_json,
                    profile.parameters_json,
                    profile.selected,
                    profile.enabled,
                    revision,
                    created_at_ms,
                    profile.updated_at_ms,
                    profile.deleted_at_ms,
                ],
            )?;
            let stored = transaction.query_row(
                &format!("SELECT {MODEL_PROFILE_COLUMNS} FROM model_profiles WHERE id=?1"),
                params![profile.id],
                model_profile_row,
            )?;
            let payload = serde_json::to_string(&stored)?;
            enqueue_sync_change(
                &transaction,
                "model_profile",
                &stored.id,
                stored.revision,
                false,
                &payload,
                stored.updated_at_ms,
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn model_profile(&self, id: &str) -> StoreResult<Option<StoredModelProfile>> {
        require_id("model_profile.id", id)?;
        self.connection
            .query_row(
                &format!("SELECT {MODEL_PROFILE_COLUMNS} FROM model_profiles WHERE id=?1 AND deleted_at_ms IS NULL"),
                params![id],
                model_profile_row,
            )
            .optional()
            .map_err(StoreError::from)
    }

    /// Includes tombstones: deletion must not reset an entity's CAS identity.
    pub fn model_profile_revision(&self, id: &str) -> StoreResult<Option<i64>> {
        require_id("model_profile.id", id)?;
        self.connection
            .query_row(
                "SELECT revision FROM model_profiles WHERE id=?1",
                params![id],
                |row| row.get(0),
            )
            .optional()
            .map_err(StoreError::from)
    }

    pub fn list_model_profiles(&self) -> StoreResult<Vec<StoredModelProfile>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {MODEL_PROFILE_COLUMNS} FROM model_profiles WHERE deleted_at_ms IS NULL
             ORDER BY selected DESC,lower(display_name),id"
        ))?;
        let rows = statement.query_map([], model_profile_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    pub fn delete_model_profile(&mut self, id: &str, deleted_at_ms: i64) -> StoreResult<bool> {
        require_id("model_profile.id", id)?;
        let transaction = self.write_transaction()?;
        let changed = transaction.execute(
            "UPDATE model_profiles SET enabled=0,selected=0,deleted_at_ms=?2,
                updated_at_ms=?2,revision=revision+1 WHERE id=?1 AND deleted_at_ms IS NULL",
            params![id, deleted_at_ms],
        )?;
        transaction.execute(
            "UPDATE model_runtimes SET phase='stopped',stopped_at_ms=?2,updated_at_ms=?2,
                process_id=NULL,endpoint_json=NULL WHERE profile_id=?1 AND phase<>'stopped'",
            params![id, deleted_at_ms],
        )?;
        if changed == 1 {
            let revision: i64 = transaction.query_row(
                "SELECT revision FROM model_profiles WHERE id=?1",
                params![id],
                |row| row.get(0),
            )?;
            enqueue_sync_change(
                &transaction,
                "model_profile",
                id,
                revision,
                true,
                "null",
                deleted_at_ms,
            )?;
        }
        transaction.commit()?;
        Ok(changed == 1)
    }

    pub fn report_model_runtime(&mut self, runtime: &StoredModelRuntime) -> StoreResult<()> {
        require_id("model_runtime.host_id", &runtime.host_id)?;
        require_id("model_runtime.profile_id", &runtime.profile_id)?;
        if !matches!(
            runtime.phase.as_str(),
            "starting" | "ready" | "failed" | "stopping" | "stopped"
        ) {
            return Err(StoreError::Invariant(format!(
                "unsupported model runtime phase {}",
                runtime.phase
            )));
        }
        if let Some(value) = runtime.endpoint_json.as_deref() {
            validate_json("model_runtime.endpoint_json", value)?;
        }
        if runtime.phase == "ready" && runtime.endpoint_json.is_none() {
            return Err(StoreError::Invariant(
                "ready model runtime requires endpoint_json".to_string(),
            ));
        }
        self.connection.execute(
            "INSERT INTO model_runtimes(host_id,profile_id,phase,endpoint_json,process_id,error,
                started_at_ms,stopped_at_ms,updated_at_ms)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)
             ON CONFLICT(host_id) DO UPDATE SET profile_id=excluded.profile_id,
                phase=excluded.phase,endpoint_json=excluded.endpoint_json,
                process_id=excluded.process_id,error=excluded.error,
                started_at_ms=excluded.started_at_ms,stopped_at_ms=excluded.stopped_at_ms,
                updated_at_ms=excluded.updated_at_ms
             WHERE excluded.updated_at_ms>=model_runtimes.updated_at_ms",
            params![
                runtime.host_id,
                runtime.profile_id,
                runtime.phase,
                runtime.endpoint_json,
                runtime.process_id,
                runtime.error,
                runtime.started_at_ms,
                runtime.stopped_at_ms,
                runtime.updated_at_ms,
            ],
        )?;
        Ok(())
    }

    pub fn model_runtime(&self, host_id: &str) -> StoreResult<Option<StoredModelRuntime>> {
        require_id("model_runtime.host_id", host_id)?;
        self.connection
            .query_row(
                &format!("SELECT {MODEL_RUNTIME_COLUMNS} FROM model_runtimes WHERE host_id=?1"),
                params![host_id],
                model_runtime_row,
            )
            .optional()
            .map_err(StoreError::from)
    }
}
