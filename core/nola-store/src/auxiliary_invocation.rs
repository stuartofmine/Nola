// SPDX-License-Identifier: Apache-2.0
//! Auxiliary operations use the same run_attempts physical ledger.
use super::*;
use nola_domain::{
    AuxiliaryInvocationOwner, AuxiliaryModelAttempt, AuxiliaryOperation, AuxiliarySourceObservation,
};
use nola_store_port::auxiliary_invocation::AuxiliaryInvocationRepository;

impl AuxiliaryInvocationRepository for NolaStore {
    type Error = StoreError;
    fn conversation_auxiliary_sources(
        &self,
        conversation_id: &str,
        after: Option<&nola_domain::AuxiliarySourceCursor>,
        limit: usize,
    ) -> StoreResult<nola_domain::ConversationAuxiliarySourcePage> {
        require_id("auxiliary.conversation", conversation_id)?;
        if !(1..=16).contains(&limit) {
            return Err(StoreError::Invariant(
                "auxiliary source page limit must be 1...16".into(),
            ));
        }
        let mut statement = self.connection.prepare(
            "SELECT source_host_id,operation_id,json_extract(record_json,'$.attempt_id') FROM auxiliary_source_observations WHERE json_extract(local_owner_json,'$.kind')='conversation' AND json_extract(local_owner_json,'$.conversation_id')=?1 AND (json_extract(record_json,'$.receipt.operation.state') IN ('ready','running') OR COALESCE(json_extract(record_json,'$.receipt.attempt.usage.coverage'),'unknown')<>'known') AND (?2 IS NULL OR (source_host_id,operation_id)>(?2,?3)) ORDER BY source_host_id,operation_id LIMIT ?4")?;
        let mut references = statement
            .query_map(
                params![
                    conversation_id,
                    after.map(|cursor| cursor.source_host_id.as_str()),
                    after.map(|cursor| cursor.operation_id.as_str()),
                    (limit + 1) as i64
                ],
                |row| {
                    Ok(nola_domain::AuxiliarySourceReference {
                        source_host_id: row.get(0)?,
                        operation_id: row.get(1)?,
                        attempt_id: row.get(2)?,
                    })
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;
        let has_more = references.len() > limit;
        references.truncate(limit);
        let next_cursor = if has_more {
            references
                .last()
                .map(|reference| nola_domain::AuxiliarySourceCursor {
                    source_host_id: reference.source_host_id.clone(),
                    operation_id: reference.operation_id.clone(),
                })
        } else {
            None
        };
        Ok(nola_domain::ConversationAuxiliarySourcePage {
            references,
            next_cursor,
        })
    }
    fn auxiliary_source_slot(
        &self,
        owner: &AuxiliaryInvocationOwner,
        scenario: &nola_domain::AuxiliaryInvocationScenario,
    ) -> StoreResult<Option<AuxiliarySourceObservation>> {
        let owner = serde_json::to_string(owner)?;
        let scenario = serde_json::to_value(scenario)?;
        let raw: Option<String> = self.connection.query_row(
            "SELECT record_json FROM auxiliary_source_observations WHERE local_owner_json=?1 AND json_extract(record_json,'$.receipt.operation.scenario')=?2 AND json_extract(record_json,'$.receipt.operation.state') IN ('ready','running')",
            params![owner, scenario.as_str().unwrap_or("")], |row| row.get(0)).optional()?;
        raw.map(|raw| serde_json::from_str(&raw).map_err(StoreError::from))
            .transpose()
    }
    fn auxiliary_source_observation(
        &self,
        owner: &AuxiliaryInvocationOwner,
        host: &str,
        operation: &str,
    ) -> StoreResult<Option<AuxiliarySourceObservation>> {
        let owner = serde_json::to_string(owner)?;
        let raw: Option<String> = self.connection.query_row(
            "SELECT record_json FROM auxiliary_source_observations WHERE local_owner_json=?1 AND source_host_id=?2 AND operation_id=?3",
            params![owner, host, operation], |row| row.get(0)).optional()?;
        raw.map(|raw| serde_json::from_str(&raw).map_err(StoreError::from))
            .transpose()
    }
    fn commit_auxiliary_source_observation(
        &mut self,
        owner: &AuxiliaryInvocationOwner,
        observation: &AuxiliarySourceObservation,
    ) -> StoreResult<()> {
        // Reserve the writer before reading the receipt/CAS frontier. A bare
        // SAVEPOINT opens a deferred transaction and can become BUSY_SNAPSHOT
        // when the archive worker commits between validation and mutation.
        let mut transaction = self.write_transaction()?;
        transaction.with_store(|store| {
            let owner_json = serde_json::to_string(owner)?;
            let bound_owner: Option<String> = store.connection.query_row(
                "SELECT local_owner_json FROM auxiliary_source_observations WHERE source_host_id=?1 AND operation_id=?2",
                params![observation.source_host_id, observation.operation_id], |row| row.get(0)).optional()?;
            if bound_owner.is_some_and(|bound| bound != owner_json) {
                return Err(StoreError::Conflict(
                    "source physical operation already belongs to another local product owner"
                        .into(),
                ));
            }
            if store
                .auxiliary_operation(&observation.operation_id)?
                .is_some_and(|operation| &operation.owner == owner)
            {
                return Err(StoreError::Conflict(
                    "local physical operation cannot also be counted as a source observation"
                        .into(),
                ));
            }
            let previous = store.auxiliary_source_observation(
                owner,
                &observation.source_host_id,
                &observation.operation_id,
            )?;
            if let Some(previous) = &previous {
                if previous == observation {
                    return Ok(());
                }
                let old = &previous.receipt;
                let new = &observation.receipt;
                let attempt_regressed = match (&old.attempt, &new.attempt) {
                    (Some(old_attempt), Some(new_attempt)) => {
                        new_attempt.revision < old_attempt.revision
                            || (old_attempt.state.is_terminal()
                                && old_attempt.state != new_attempt.state)
                            || (old_attempt.output.is_some()
                                && old_attempt.output != new_attempt.output)
                            || (old_attempt.usage["coverage"] == "known"
                                && new_attempt.usage["coverage"] != "known")
                    }
                    (Some(_), None) => true,
                    _ => false,
                };
                if old.operation.owner != new.operation.owner
                    || old.operation.scenario != new.operation.scenario
                    || old.operation.configuration != new.operation.configuration
                    || old.operation.created_at_ms != new.operation.created_at_ms
                    || previous.attempt_id != observation.attempt_id
                    || new.operation.revision <= old.operation.revision
                    || new.operation.attempts_started < old.operation.attempts_started
                    || (old.operation.cancellation_requested_at_ms.is_some()
                        && old.operation.cancellation_requested_at_ms
                            != new.operation.cancellation_requested_at_ms)
                    || (matches!(
                        old.operation.state,
                        nola_domain::AuxiliaryOperationState::Succeeded
                            | nola_domain::AuxiliaryOperationState::Failed
                            | nola_domain::AuxiliaryOperationState::Cancelled
                    ) && old.operation.state != new.operation.state)
                    || attempt_regressed
                {
                    return Err(StoreError::Conflict(
                        "source auxiliary receipt regressed or changed immutable identity".into(),
                    ));
                }
            } else {
                let exists = match owner {
                    AuxiliaryInvocationOwner::Room { room_id } => {
                        require_id("auxiliary.room", room_id)?;
                        store.connection.query_row("SELECT EXISTS(SELECT 1 FROM product_settings WHERE key=?1)",
                            [format!("{}{room_id}", nola_domain::agent_room::ROOM_KEY_PREFIX)], |row| row.get::<_, bool>(0))?
                    }
                    AuxiliaryInvocationOwner::Conversation { conversation_id } => {
                        store.connection.query_row(
                            "SELECT EXISTS(SELECT 1 FROM conversations WHERE id=?1)",
                            [conversation_id],
                            |row| row.get::<_, bool>(0),
                        )?
                    }
                    AuxiliaryInvocationOwner::Task { task_id } => store.connection.query_row(
                        "SELECT EXISTS(SELECT 1 FROM durable_tasks WHERE id=?1)",
                        [task_id],
                        |row| row.get::<_, bool>(0),
                    )?,
                    AuxiliaryInvocationOwner::Account { account_id } => {
                        require_id("auxiliary.account", account_id)?;
                        true
                    }
                    AuxiliaryInvocationOwner::SourceAuthority { .. } => false,
                };
                if !exists {
                    return Err(StoreError::Conflict(
                        "local auxiliary observation owner missing".into(),
                    ));
                }
            }
            let raw = serde_json::to_string(observation)?;
            let revision = observation.receipt.operation.revision;
            let changed = if let Some(previous) = previous {
                store.connection.execute("UPDATE auxiliary_source_observations SET record_json=?4,revision=?5 WHERE local_owner_json=?1 AND source_host_id=?2 AND operation_id=?3 AND revision=?6",
                    params![owner_json, observation.source_host_id, observation.operation_id, raw, revision, previous.receipt.operation.revision])?
            } else {
                store.connection.execute("INSERT INTO auxiliary_source_observations(local_owner_json,source_host_id,operation_id,record_json,revision) VALUES(?1,?2,?3,?4,?5)",
                    params![owner_json, observation.source_host_id, observation.operation_id, raw, revision])?
            };
            if changed != 1 {
                return Err(StoreError::Conflict(
                    "source auxiliary receipt concurrently changed".into(),
                ));
            }
            bump_auxiliary_owner_projection(&store.connection, owner)?;
            Ok(())
        })?;
        transaction.commit()
    }
    fn auxiliary_operation(&self, id: &str) -> StoreResult<Option<AuxiliaryOperation>> {
        let raw: Option<String> = self
            .connection
            .query_row(
                "SELECT record_json FROM auxiliary_operations WHERE id=?1",
                [id],
                |row| row.get(0),
            )
            .optional()?;
        raw.map(|raw| serde_json::from_str(&raw).map_err(StoreError::from))
            .transpose()
    }
    fn auxiliary_attempt(
        &self,
        operation_id: &str,
        id: &str,
    ) -> StoreResult<Option<AuxiliaryModelAttempt>> {
        let raw: Option<String> = self.connection.query_row("SELECT record_json FROM run_attempts WHERE id=?1 AND operation_id=?2 AND run_id IS NULL", params![id, operation_id], |row| row.get(0)).optional()?;
        raw.map(|raw| serde_json::from_str(&raw).map_err(StoreError::from))
            .transpose()
    }
    fn auxiliary_attempt_tail(
        &self,
        operation_id: &str,
    ) -> StoreResult<Option<AuxiliaryModelAttempt>> {
        let raw: Option<String> = self.connection.query_row("SELECT a.record_json FROM run_attempts a WHERE operation_id=?1 AND NOT EXISTS(SELECT 1 FROM run_attempts child WHERE child.predecessor_id=a.id)", [operation_id], |row| row.get(0)).optional()?;
        raw.map(|raw| serde_json::from_str(&raw).map_err(StoreError::from))
            .transpose()
    }
    fn commit_auxiliary_invocation(
        &mut self,
        operation: &AuxiliaryOperation,
        expected_revision: Option<i64>,
        attempt: Option<&AuxiliaryModelAttempt>,
        expected_attempt_revision: Option<i64>,
    ) -> StoreResult<()> {
        let mut transaction = self.write_transaction()?;
        transaction.with_store(|store| {
            // Deleted owners cannot admit new work, but physical receipts and
            // late usage retain their original immutable operation identity.
            if expected_revision.is_none()
                || (attempt.is_some() && expected_attempt_revision.is_none())
            {
                match &operation.owner {
                    AuxiliaryInvocationOwner::Room { room_id } => {
                        require_id("auxiliary.room", room_id)?;
                        let exists: bool = store.connection.query_row("SELECT EXISTS(SELECT 1 FROM product_settings WHERE key=?1)",
                            [format!("{}{room_id}", nola_domain::agent_room::ROOM_KEY_PREFIX)], |row| row.get(0))?;
                        if !exists { return Err(StoreError::Conflict("auxiliary room owner no longer exists".into())); }
                    }
                    AuxiliaryInvocationOwner::Conversation { conversation_id } => {
                        let exists: bool = store.connection.query_row(
                            "SELECT EXISTS(SELECT 1 FROM conversations WHERE id=?1)",
                            [conversation_id],
                            |row| row.get(0),
                        )?;
                        if !exists {
                            return Err(StoreError::Conflict(
                                "auxiliary conversation owner no longer exists".into(),
                            ));
                        }
                    }
                    AuxiliaryInvocationOwner::Account { account_id } => {
                        require_id("auxiliary.account", account_id)?;
                    }
                    AuxiliaryInvocationOwner::SourceAuthority {
                        source_host_id,
                        authority_id,
                    } => {
                        require_id("auxiliary.source_host", source_host_id)?;
                        require_id("auxiliary.authority", authority_id)?;
                    }
                    AuxiliaryInvocationOwner::Task { task_id } => {
                        let exists: bool = store.connection.query_row(
                            "SELECT EXISTS(SELECT 1 FROM durable_tasks WHERE id=?1)",
                            [task_id],
                            |row| row.get(0),
                        )?;
                        if !exists {
                            return Err(StoreError::Conflict(
                                "auxiliary task owner no longer exists".into(),
                            ));
                        }
                    }
                }
            }
            let raw = serde_json::to_string(operation)?;
            match expected_revision {
                None => {
                    store.connection.execute("INSERT INTO auxiliary_operations(id,record_json,revision) VALUES(?1,?2,?3)", params![operation.id, raw, operation.revision])?;
                }
                Some(expected) => {
                    let previous = store.auxiliary_operation(&operation.id)?.ok_or_else(|| {
                        StoreError::Conflict("auxiliary operation missing".into())
                    })?;
                    if previous.owner != operation.owner
                        || previous.scenario != operation.scenario
                        || previous.configuration != operation.configuration
                        || previous.created_at_ms != operation.created_at_ms
                        || expected.checked_add(1) != Some(operation.revision)
                    {
                        return Err(StoreError::Conflict(
                            "auxiliary immutable owner/configuration changed".into(),
                        ));
                    }
                    if store.connection.execute("UPDATE auxiliary_operations SET record_json=?2,revision=?3 WHERE id=?1 AND revision=?4", params![operation.id, raw, operation.revision, expected])? != 1 {
                        return Err(StoreError::Conflict("auxiliary operation revision changed".into()));
                    }
                }
            }
            if let Some(attempt) = attempt {
                if attempt.operation_id != operation.id {
                    return Err(StoreError::Conflict(
                        "auxiliary Attempt owner mismatch".into(),
                    ));
                }
                let raw = serde_json::to_string(attempt)?;
                if raw.len() > 65536 {
                    return Err(StoreError::Invariant(
                        "Attempt metadata exceeds 64KiB".into(),
                    ));
                }
                let state = serde_json::to_value(attempt.state)?;
                let state = state.as_str().expect("Attempt state string");
                match expected_attempt_revision {
                    None => {
                        if attempt.revision != 1 {
                            return Err(StoreError::Invariant(
                                "new Attempt revision must be one".into(),
                            ));
                        }
                        if let Some(predecessor) = &attempt.predecessor_id {
                            let previous = store
                                .auxiliary_attempt(&operation.id, predecessor)?
                                .ok_or_else(|| {
                                    StoreError::Conflict("auxiliary predecessor missing".into())
                                })?;
                            if !previous.state.is_terminal() {
                                return Err(StoreError::Conflict(
                                    "auxiliary predecessor is still active".into(),
                                ));
                            }
                        }
                        store.connection.execute("INSERT INTO run_attempts(id,run_id,operation_id,invocation_id,effect_id,text_part_id,reasoning_part_id,predecessor_id,state,revision,record_json) VALUES(?1,NULL,?2,?2,?2,NULL,NULL,?3,?4,?5,?6)", params![attempt.id, operation.id, attempt.predecessor_id, state, attempt.revision, raw])?;
                    }
                    Some(expected) => {
                        let previous = store
                            .auxiliary_attempt(&operation.id, &attempt.id)?
                            .ok_or_else(|| {
                                StoreError::Conflict("auxiliary Attempt missing".into())
                            })?;
                        if previous.predecessor_id != attempt.predecessor_id
                            || previous.provider != attempt.provider
                            || previous.model != attempt.model
                            || previous.created_at_ms != attempt.created_at_ms
                            || expected.checked_add(1) != Some(attempt.revision)
                        {
                            return Err(StoreError::Conflict(
                                "auxiliary Attempt immutable identity changed".into(),
                            ));
                        }
                        if store.connection.execute("UPDATE run_attempts SET state=?3,revision=?4,record_json=?5 WHERE id=?1 AND operation_id=?2 AND revision=?6", params![attempt.id, operation.id, state, attempt.revision, raw, expected])? != 1 {
                            return Err(StoreError::Conflict("auxiliary Attempt revision changed".into()));
                        }
                    }
                }
            }
            bump_auxiliary_owner_projection(&store.connection, &operation.owner)?;
            Ok(())
        })?;
        transaction.commit()
    }
}

fn bump_auxiliary_owner_projection(
    connection: &rusqlite::Connection,
    owner: &AuxiliaryInvocationOwner,
) -> StoreResult<()> {
    if let AuxiliaryInvocationOwner::Conversation { conversation_id } = owner {
        // Usage changes the aggregate read projection, never execution config.
        // Application's enclosing command transaction emits its bounded feed
        // invalidation from this committed revision, without copying history.
        connection.execute(
            "UPDATE conversations SET revision=revision+1 WHERE id=?1",
            [conversation_id],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod auxiliary_source_page_tests {
    use super::*;
    #[test]
    fn manual_source_page_is_owner_scoped_unknown_only_and_bounded() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = NolaStore::open(&directory.path().join("sources.sqlite")).unwrap();
        store.connection.execute_batch("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','one',1,1,1),('other','two',1,1,1)").unwrap();
        for index in 0..19 {
            let id = format!("source-{index:02}");
            let known = index == 0;
            let raw = serde_json::json!({"source_host_id":"actual-source-host","operation_id":id,"attempt_id":format!("{id}:physical"),
                "receipt":{"dispatch_permitted":false,"operation":{"id":id,"owner":{"kind":"source_authority","source_host_id":"actual-source-host","authority_id":"actual-epoch"},
                    "scenario":"context_summary","configuration":{"provider":"fixture","model":"fixture-model","configuration_revision":1,
                        "configuration_digest":"a".repeat(64),"input_digest":"b".repeat(64),"max_attempts":1,"maximum_duration_ms":1000,
                        "max_output_tokens_per_attempt":100,"total_output_token_budget":100},
                    "state":"succeeded","attempts_started":1,"active_attempt_id":null,"cancellation_requested_at_ms":null,"created_at_ms":1,"revision":3},
                    "attempt":{"id":format!("{id}:physical"),"operation_id":id,"predecessor_id":null,"state":"succeeded","provider":"fixture","model":"fixture-model",
                        "usage":if known {serde_json::json!({"coverage":"known","provider_raw":{"prompt_tokens":1,"completion_tokens":1},"normalized":{"input_tokens":1,"output_tokens":1}})}
                            else {serde_json::json!({"coverage":"unknown","provider_raw":null,"normalized":null})},
                        "failure":null,"output":{"items":[],"explain":"not-in-reference-page"},"created_at_ms":2,"completed_at_ms":3,"revision":2}}});
            let observation: AuxiliarySourceObservation = serde_json::from_value(raw).unwrap();
            let owner = AuxiliaryInvocationOwner::Conversation {
                conversation_id: if index == 18 { "other" } else { "c" }.into(),
            };
            store
                .commit_auxiliary_source_observation(&owner, &observation)
                .unwrap();
        }
        let first = store.conversation_auxiliary_sources("c", None, 16).unwrap();
        assert_eq!(first.references.len(), 16);
        assert_eq!(first.references[0].operation_id, "source-01");
        let serialized = serde_json::to_string(&first).unwrap();
        assert!(!serialized.contains("not-in-reference-page"));
        let last = store
            .conversation_auxiliary_sources("c", first.next_cursor.as_ref(), 16)
            .unwrap();
        assert_eq!(last.references.len(), 1);
        assert_eq!(last.references[0].operation_id, "source-17");
        assert!(last.next_cursor.is_none());
        assert!(store.conversation_auxiliary_sources("c", None, 17).is_err());
        assert_eq!(
            store
                .conversation_auxiliary_sources("other", None, 16)
                .unwrap()
                .references
                .len(),
            1
        );
    }
}
