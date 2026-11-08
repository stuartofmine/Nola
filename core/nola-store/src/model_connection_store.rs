// SPDX-License-Identifier: Apache-2.0
//! Connection metadata and existing runtime profiles share one atomic writer.
use super::*;
use nola_domain::model_connection::*;
use std::collections::BTreeMap;

impl NolaStore {
    pub fn model_probe_attempt(&self, id: &str) -> StoreResult<Option<ModelProbeAttempt>> {
        validate_identity(id).map_err(StoreError::Invariant)?;
        self.product_setting(&format!("{PROBE_ATTEMPT_KEY_PREFIX}{id}"))?
            .map(|value| serde_json::from_str(&value.value_json).map_err(StoreError::from))
            .transpose()
    }
    pub fn reserve_model_probe(&mut self, record: &ModelProbeAttempt) -> StoreResult<()> {
        validate_identity(&record.request_id).map_err(StoreError::Invariant)?;
        validate_identity(&record.model_id).map_err(StoreError::Invariant)?;
        if record.finished_at_ms.is_some()
            || record.text_confirmed.is_some()
            || record.credential_binding.len() != 64
            || record.started_at_ms <= 0
        {
            return Err(StoreError::Invariant("invalid probe start".into()));
        }
        let mut transaction = self.write_transaction()?;
        let result = transaction.with_store(|store| -> StoreResult<()> {
            if store.model_probe_attempt(&record.request_id)?.is_some() {
                return Err(StoreError::Invariant(
                    "model_probe_confirmation_consumed".into(),
                ));
            }
            let connection = store
                .model_connection(&record.connection_id)?
                .ok_or_else(|| StoreError::Invariant("model_connection_not_found".into()))?;
            let account_revision = if let Some(id) = &record.account_id {
                Some(
                    store
                        .model_account(id)?
                        .ok_or_else(|| StoreError::Invariant("model_account_not_found".into()))?
                        .revision,
                )
            } else {
                None
            };
            if connection.revision != record.connection_revision
                || connection.document.connection.account_id != record.account_id
                || account_revision != record.account_revision
                || !connection.document.connection.enabled
                || !connection.document.catalog.as_ref().is_some_and(|catalog| {
                    catalog.models.iter().any(|model| {
                        model.model_id == record.model_id && model.protocol == record.protocol
                    })
                })
            {
                return Err(StoreError::Invariant(
                    "model_probe_confirmation_stale".into(),
                ));
            }
            store.put_product_setting(
                &format!("{PROBE_ATTEMPT_KEY_PREFIX}{}", record.request_id),
                &serde_json::to_string(record)?,
                record.started_at_ms,
            )?;
            Ok(())
        });
        match result {
            Ok(()) => transaction.commit(),
            Err(error) => {
                transaction.rollback()?;
                Err(error)
            }
        }
    }
    pub fn finish_model_probe(&mut self, id: &str, confirmed: bool, at: i64) -> StoreResult<()> {
        let mut transaction = self.write_transaction()?;
        let result = transaction.with_store(|store| -> StoreResult<()> {
            let mut record = store
                .model_probe_attempt(id)?
                .ok_or_else(|| StoreError::Invariant("model_probe_attempt_missing".into()))?;
            if record.finished_at_ms.is_some() || at < record.started_at_ms {
                return Err(StoreError::Invariant("model_probe_attempt_terminal".into()));
            }
            record.finished_at_ms = Some(at);
            record.text_confirmed = Some(confirmed);
            store.put_product_setting(
                &format!("{PROBE_ATTEMPT_KEY_PREFIX}{id}"),
                &serde_json::to_string(&record)?,
                at,
            )?;
            Ok(())
        });
        match result {
            Ok(()) => transaction.commit(),
            Err(error) => {
                transaction.rollback()?;
                Err(error)
            }
        }
    }

    pub fn list_gateway_attempts(
        &self,
        limit: usize,
        offset: usize,
    ) -> StoreResult<Vec<GatewayAttemptSnapshot>> {
        self.list_product_settings(GATEWAY_ATTEMPT_KEY_PREFIX, limit.min(256), offset)?
            .into_iter()
            .map(|setting| {
                let record: GatewayAttemptRecord = serde_json::from_str(&setting.value_json)
                    .map_err(|_| StoreError::Invariant("invalid stored gateway attempt".into()))?;
                record.validate().map_err(StoreError::Invariant)?;
                if setting.key != format!("{GATEWAY_ATTEMPT_KEY_PREFIX}{}", record.id) {
                    return Err(StoreError::Invariant(
                        "gateway attempt identity mismatch".into(),
                    ));
                }
                Ok(GatewayAttemptSnapshot {
                    revision: setting.revision,
                    record,
                })
            })
            .collect()
    }

    pub fn save_gateway_attempt(
        &mut self,
        record: &GatewayAttemptRecord,
        expected_revision: Option<i64>,
    ) -> StoreResult<GatewayAttemptSnapshot> {
        record.validate().map_err(StoreError::Invariant)?;
        let mut transaction = self.write_transaction()?;
        let result = transaction.with_store(|store| -> StoreResult<GatewayAttemptSnapshot> {
            let key = format!("{GATEWAY_ATTEMPT_KEY_PREFIX}{}", record.id);
            let previous = store.product_setting(&key)?;
            if previous.as_ref().map(|setting| setting.revision) != expected_revision {
                return Err(StoreError::Invariant(
                    "gateway_attempt_revision_conflict".into(),
                ));
            }
            if let Some(previous) = previous {
                let previous: GatewayAttemptRecord = serde_json::from_str(&previous.value_json)
                    .map_err(|_| StoreError::Invariant("invalid stored gateway attempt".into()))?;
                if previous.outcome != GatewayAttemptOutcome::InFlight
                    || !previous.same_identity(record)
                {
                    return Err(StoreError::Invariant(
                        "gateway attempt is terminal or identity changed".into(),
                    ));
                }
            } else if record.outcome != GatewayAttemptOutcome::InFlight
                || record.usage.is_some()
                || record.http_status.is_some()
            {
                return Err(StoreError::Invariant(
                    "gateway attempt must start before upstream submission".into(),
                ));
            }
            let setting = store.put_product_setting(
                &key,
                &serde_json::to_string(record)?,
                record.finished_at_ms.unwrap_or(record.started_at_ms),
            )?;
            Ok(GatewayAttemptSnapshot {
                revision: setting.revision,
                record: record.clone(),
            })
        });
        match result {
            Ok(snapshot) => {
                transaction.commit()?;
                Ok(snapshot)
            }
            Err(error) => {
                transaction.rollback()?;
                Err(error)
            }
        }
    }

    pub fn model_connection(&self, id: &str) -> StoreResult<Option<ModelConnectionSnapshot>> {
        validate_identity(id).map_err(StoreError::Invariant)?;
        self.product_setting(&format!("{CONNECTION_KEY_PREFIX}{id}"))?
            .map(|setting| {
                let document: ModelConnectionDocument = serde_json::from_str(&setting.value_json)
                    .map_err(|_| {
                    StoreError::Invariant("invalid stored model connection".into())
                })?;
                document.validate().map_err(StoreError::Invariant)?;
                if document.connection.id != id || setting.revision < 1 {
                    return Err(StoreError::Invariant(
                        "invalid stored model connection identity".into(),
                    ));
                }
                Ok(ModelConnectionSnapshot {
                    revision: setting.revision,
                    document,
                })
            })
            .transpose()
    }

    pub fn list_model_connections(&self) -> StoreResult<Vec<ModelConnectionSnapshot>> {
        let mut result = Vec::new();
        let mut offset = 0;
        loop {
            let page = self.list_product_settings(CONNECTION_KEY_PREFIX, 256, offset)?;
            for setting in &page {
                let id = &setting.key[CONNECTION_KEY_PREFIX.len()..];
                result.push(
                    self.model_connection(id)?.ok_or_else(|| {
                        StoreError::Invariant("model connection disappeared".into())
                    })?,
                );
            }
            if page.len() < 256 {
                break;
            }
            offset += page.len();
        }
        Ok(result)
    }

    pub fn commit_model_connection(
        &mut self,
        document: &ModelConnectionDocument,
        expected_revision: Option<i64>,
        profiles: &[StoredModelProfile],
        expected_profile_revisions: &BTreeMap<String, i64>,
        updated_at_ms: i64,
    ) -> StoreResult<ModelConnectionSnapshot> {
        document.validate().map_err(StoreError::Invariant)?;
        if updated_at_ms < 0 || expected_revision.is_some_and(|r| r < 1) {
            return Err(StoreError::Invariant(
                "invalid model connection revision or time".into(),
            ));
        }
        let id = &document.connection.id;
        let mut transaction = self.write_transaction()?;
        let result = transaction.with_store(|store| -> StoreResult<ModelConnectionSnapshot> {
            let previous = store.model_connection(id)?;
            if previous.as_ref().map(|s| s.revision) != expected_revision {
                return Err(StoreError::Invariant(
                    "model_connection_revision_conflict".into(),
                ));
            }
            if previous.is_some_and(|s| {
                s.document.connection.source != document.connection.source
                    || s.document.connection.source_id != document.connection.source_id
            }) {
                return Err(StoreError::Invariant(
                    "model connection source identity cannot change".into(),
                ));
            }
            for snapshot in store.list_model_connections()? {
                let other = snapshot.document.connection;
                if other.id == *id {
                    continue;
                }
                if other
                    .display_name
                    .trim()
                    .eq_ignore_ascii_case(document.connection.display_name.trim())
                {
                    return Err(StoreError::Invariant(
                        "model_connection_name_conflict".into(),
                    ));
                }
                if other.source == document.connection.source
                    && other.source_id.is_some()
                    && other.source_id == document.connection.source_id
                {
                    return Err(StoreError::Invariant(
                        "model_connection_source_already_imported".into(),
                    ));
                }
            }
            if let Some(account_id) = &document.connection.account_id {
                if store.model_account(account_id)?.is_none() {
                    return Err(StoreError::Invariant("model account does not exist".into()));
                }
            }
            let existing = store
                .list_model_profiles()?
                .into_iter()
                .filter(|profile| {
                    serde_json::from_str::<serde_json::Value>(&profile.source_json)
                        .ok()
                        .is_some_and(|source| source["connectionID"].as_str() == Some(id.as_str()))
                        && profile.deleted_at_ms.is_none()
                })
                .map(|profile| (profile.id.clone(), profile.revision))
                .collect::<BTreeMap<_, _>>();
            if &existing != expected_profile_revisions {
                return Err(StoreError::Invariant(
                    "model_connection_profiles_revision_conflict".into(),
                ));
            }
            let mut seen = std::collections::BTreeSet::new();
            let mut ordered: Vec<_> = profiles.iter().collect();
            ordered.sort_by_key(|profile| profile.selected);
            for profile in ordered {
                let source: serde_json::Value = serde_json::from_str(&profile.source_json)
                    .map_err(|_| StoreError::Invariant("invalid connection model source".into()))?;
                if profile.runtime != "remote"
                    || profile.deleted_at_ms.is_some()
                    || source["connectionID"].as_str() != Some(id.as_str())
                    || source["endpointID"].as_str() != Some(id.as_str())
                    || source["baseURL"].as_str() != Some(document.connection.base_url.as_str())
                    || source["accountID"] != serde_json::json!(document.connection.account_id)
                    || !seen.insert(profile.id.as_str())
                {
                    return Err(StoreError::Invariant(
                        "invalid connection profile ownership".into(),
                    ));
                }
                if let Some(previous) = store.model_profile(&profile.id)? {
                    if !existing.contains_key(&previous.id) {
                        return Err(StoreError::Invariant(
                            "connection model identity already owned".into(),
                        ));
                    }
                }
                store.upsert_model_profile(profile)?;
            }
            // Omitting an existing profile is never an implicit deletion.
            if existing.keys().any(|id| !seen.contains(id.as_str())) {
                return Err(StoreError::Invariant(
                    "connection write must retain existing models".into(),
                ));
            }
            let setting = store.put_product_setting(
                &format!("{CONNECTION_KEY_PREFIX}{id}"),
                &serde_json::to_string(document)?,
                updated_at_ms,
            )?;
            Ok(ModelConnectionSnapshot {
                revision: setting.revision,
                document: document.clone(),
            })
        });
        match result {
            Ok(snapshot) => {
                transaction.commit()?;
                Ok(snapshot)
            }
            Err(error) => {
                transaction.rollback()?;
                Err(error)
            }
        }
    }

    pub fn model_account(&self, id: &str) -> StoreResult<Option<ModelAccountSnapshot>> {
        validate_identity(id).map_err(StoreError::Invariant)?;
        self.product_setting(&format!("{ACCOUNT_KEY_PREFIX}{id}"))?
            .map(|setting| {
                #[derive(serde::Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Document {
                    format_version: u32,
                    account: ModelAccount,
                }
                let document: Document = serde_json::from_str(&setting.value_json)
                    .map_err(|_| StoreError::Invariant("invalid stored model account".into()))?;
                document.account.validate().map_err(StoreError::Invariant)?;
                if document.format_version != 1 || document.account.id != id || setting.revision < 1
                {
                    return Err(StoreError::Invariant(
                        "unsupported model account format or identity".into(),
                    ));
                }
                Ok(ModelAccountSnapshot {
                    revision: setting.revision,
                    account: document.account,
                })
            })
            .transpose()
    }

    pub fn list_model_accounts(&self) -> StoreResult<Vec<ModelAccountSnapshot>> {
        let mut result = Vec::new();
        let mut offset = 0;
        loop {
            let page = self.list_product_settings(ACCOUNT_KEY_PREFIX, 256, offset)?;
            for setting in &page {
                result.push(
                    self.model_account(&setting.key[ACCOUNT_KEY_PREFIX.len()..])?
                        .ok_or_else(|| StoreError::Invariant("model account disappeared".into()))?,
                );
            }
            if page.len() < 256 {
                break;
            }
            offset += page.len();
        }
        Ok(result)
    }

    pub fn save_model_account(
        &mut self,
        account: &ModelAccount,
        expected_revision: Option<i64>,
        updated_at_ms: i64,
    ) -> StoreResult<ModelAccountSnapshot> {
        account.validate().map_err(StoreError::Invariant)?;
        if updated_at_ms < 0 || expected_revision.is_some_and(|r| r < 1) {
            return Err(StoreError::Invariant(
                "invalid model account revision or time".into(),
            ));
        }
        let mut transaction = self.write_transaction()?;
        let result = transaction.with_store(|store| -> StoreResult<ModelAccountSnapshot> {
            let previous = store.model_account(&account.id)?;
            if previous.as_ref().map(|s| s.revision) != expected_revision {
                return Err(StoreError::Invariant(
                    "model_account_revision_conflict".into(),
                ));
            }
            if previous.is_some_and(|s| s.account.source != account.source) {
                return Err(StoreError::Invariant(
                    "model account source identity cannot change".into(),
                ));
            }
            let setting = store.put_product_setting(
                &format!("{ACCOUNT_KEY_PREFIX}{}", account.id),
                &serde_json::json!({"format_version":1,"account":account}).to_string(),
                updated_at_ms,
            )?;
            Ok(ModelAccountSnapshot {
                revision: setting.revision,
                account: account.clone(),
            })
        });
        match result {
            Ok(snapshot) => {
                transaction.commit()?;
                Ok(snapshot)
            }
            Err(error) => {
                transaction.rollback()?;
                Err(error)
            }
        }
    }
}
