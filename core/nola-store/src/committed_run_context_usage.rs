// SPDX-License-Identifier: Apache-2.0
//! Bounded control facts: source reads existing committed events; replicas keep
//! only the latest exact-Run record, with no execution or transcript authority.
use crate::{params, NolaStore, OptionalExtension, StoreError, StoreResult};
use nola_domain::assistant_journal::{AssistantDigest, AssistantJournalIdentity};
use nola_domain::assistant_journal_operations::JournalNumber;
use nola_domain::committed_run_context_usage::*;
use nola_domain::run_message_admission::RunMessageAdmissionRecord;
use nola_store_port::assistant_journal::AssistantJournalRepository;
use nola_store_port::run_message_admission::RunMessageAdmissionRepository;

fn invalid(error: impl ToString) -> StoreError {
    StoreError::Invariant(error.to_string())
}

fn merge_cache_usage(
    combined: &mut Option<nola_domain::ConversationCacheUsage>,
    next: nola_domain::ConversationCacheUsage,
) -> StoreResult<()> {
    *combined = Some(match *combined {
        None => next,
        Some(previous) => previous
            .checked_merge(next)
            .ok_or_else(|| invalid("cache request count overflow"))?,
    });
    Ok(())
}

#[cfg(test)]
mod conversation_cache_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn auxiliary_cache_usage_is_scoped_updates_in_place_and_survives_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("usage.sqlite");
        let mut store = NolaStore::open(&path).unwrap();
        store.connection.execute_batch("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','one',1,1,1),('other','two',1,1,1)").unwrap();
        let configuration = nola_domain::AuxiliaryInvocationConfiguration {
            provider: "fixture".into(),
            model: Some("fixture-model".into()),
            configuration_revision: 1,
            configuration_digest: "a".repeat(64),
            input_digest: "b".repeat(64),
            max_attempts: 1,
            maximum_duration_ms: 1000,
            max_output_tokens_per_attempt: Some(100),
            total_output_token_budget: Some(100),
        };
        let mut operation = nola_domain::AuxiliaryOperation {
            id: "summary".into(),
            owner: nola_domain::AuxiliaryInvocationOwner::Conversation {
                conversation_id: "c".into(),
            },
            scenario: nola_domain::AuxiliaryInvocationScenario::ContextSummary,
            configuration,
            state: nola_domain::AuxiliaryOperationState::Running,
            attempts_started: 1,
            active_attempt_id: Some("attempt".into()),
            cancellation_requested_at_ms: None,
            created_at_ms: 1,
            revision: 1,
        };
        let mut attempt: nola_domain::AuxiliaryModelAttempt = serde_json::from_value(json!({
            "id":"attempt","operation_id":"summary","predecessor_id":null,"state":"running","provider":"fixture",
            "model":"fixture-model","usage":{"coverage":"unknown","provider_raw":null,"normalized":null},
            "failure":null,"output":null,"created_at_ms":2,"completed_at_ms":null,"revision":1
        })).unwrap();
        nola_store_port::auxiliary_invocation::AuxiliaryInvocationRepository::commit_auxiliary_invocation(
            &mut store, &operation, None, Some(&attempt), None).unwrap();
        let unknown = store.conversation_cache_usage("c").unwrap().unwrap();
        assert_eq!(unknown.unreported_requests, 1);
        assert_eq!(unknown.totals.hit_rate_bps, None);
        assert!(store.conversation_cache_usage("other").unwrap().is_none());
        operation.revision = 2;
        operation.state = nola_domain::AuxiliaryOperationState::Succeeded;
        operation.active_attempt_id = None;
        attempt.revision = 2;
        attempt.state = nola_domain::ModelAttemptState::Succeeded;
        attempt.completed_at_ms = Some(3);
        attempt.usage = json!({"coverage":"known","provider_raw":{"input_tokens":1000},
            "normalized":{"input_tokens":1000,"output_tokens":10,"cached_input_tokens":750,"cache_write_input_tokens":null}});
        nola_store_port::auxiliary_invocation::AuxiliaryInvocationRepository::commit_auxiliary_invocation(
            &mut store, &operation, Some(1), Some(&attempt), Some(1)).unwrap();
        let total = store.conversation_cache_usage("c").unwrap().unwrap();
        assert_eq!((total.reported_requests, total.unreported_requests), (1, 0));
        assert_eq!(total.totals.hit_rate_bps, Some(7500));
        assert_eq!(store.conversation_cache_usage("c").unwrap(), Some(total));
        drop(store);
        let store = NolaStore::open(&path).unwrap();
        assert_eq!(store.conversation_cache_usage("c").unwrap(), Some(total));
        store
            .connection
            .execute("DELETE FROM conversations WHERE id='c'", [])
            .unwrap();
        assert!(store.conversation_cache_usage("c").unwrap().is_none());
    }

    #[test]
    fn source_receipt_cache_totals_are_observations_not_duplicate_physical_rows() {
        use nola_store_port::auxiliary_invocation::AuxiliaryInvocationRepository;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("source.sqlite");
        let mut store = NolaStore::open(&path).unwrap();
        store.connection.execute_batch("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','one',1,1,1),('other','two',1,1,1)").unwrap();
        let local_owner = nola_domain::AuxiliaryInvocationOwner::Conversation {
            conversation_id: "c".into(),
        };
        let mut observation: nola_domain::AuxiliarySourceObservation = serde_json::from_value(json!({
            "source_host_id":"source-host","operation_id":"remote-op","attempt_id":"remote-op:physical",
            "receipt":{"dispatch_permitted":false,"operation":{
                "id":"remote-op","owner":{"kind":"source_authority","source_host_id":"source-host","authority_id":"authority"},
                "scenario":"context_summary","configuration":{"provider":"fixture","model":"fixture","configuration_revision":1,
                    "configuration_digest":"a".repeat(64),"input_digest":"b".repeat(64),"max_attempts":1,
                    "maximum_duration_ms":1000,"max_output_tokens_per_attempt":100,"total_output_token_budget":100},
                "state":"succeeded","attempts_started":1,"active_attempt_id":null,"cancellation_requested_at_ms":null,"created_at_ms":1,"revision":2},
                "attempt":{"id":"remote-op:physical","operation_id":"remote-op","predecessor_id":null,"state":"succeeded",
                    "provider":"fixture","model":"fixture","usage":{"coverage":"unknown","provider_raw":null,"normalized":null},
                    "failure":null,"output":null,"created_at_ms":2,"completed_at_ms":3,"revision":2}}
        })).unwrap();
        store
            .commit_auxiliary_source_observation(&local_owner, &observation)
            .unwrap();
        assert_eq!(
            store
                .conversation_cache_usage("c")
                .unwrap()
                .unwrap()
                .unreported_requests,
            1
        );
        assert!(store.conversation_cache_usage("other").unwrap().is_none());
        observation.receipt.operation.revision = 3;
        let attempt = observation.receipt.attempt.as_mut().unwrap();
        attempt.revision = 3;
        attempt.usage = json!({"coverage":"known","provider_raw":{"prompt_tokens":200},
            "normalized":{"input_tokens":200,"output_tokens":10,"cached_input_tokens":100,"cache_write_input_tokens":null}});
        store
            .commit_auxiliary_source_observation(&local_owner, &observation)
            .unwrap();
        store
            .commit_auxiliary_source_observation(&local_owner, &observation)
            .unwrap();
        let total = store.conversation_cache_usage("c").unwrap().unwrap();
        assert_eq!((total.reported_requests, total.unreported_requests), (1, 0));
        assert_eq!(total.totals.hit_rate_bps, Some(5000));
        assert_eq!(
            store
                .connection
                .query_row("SELECT count(*) FROM run_attempts", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        drop(store);
        assert_eq!(
            NolaStore::open(&path)
                .unwrap()
                .conversation_cache_usage("c")
                .unwrap(),
            Some(total)
        );
    }

    #[test]
    fn durable_cache_totals_are_weighted_deduplicated_and_conversation_scoped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("usage.sqlite");
        let store = NolaStore::open(&path).unwrap();
        store.connection.execute_batch("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','one',0,1,1),('other','two',0,1,1);
            INSERT INTO runs(id,conversation_id,kind,status,next_event_sequence,created_at_ms,updated_at_ms)
            VALUES('r1','c','conversation','running',20,1,1),('r2','c','conversation','running',20,2,2),
            ('other-run','other','conversation','running',20,1,1);").unwrap();
        let event = |run: &str, seq: i64, kind: &str, value: serde_json::Value| {
            store
                .connection
                .execute(
                    "INSERT INTO run_events VALUES(?1,?2,?3,?4,?5,1)",
                    params![format!("{run}:{seq}"), run, seq, kind, value.to_string()],
                )
                .unwrap();
        };
        let usage = |input, read| json!({"cache_usage":nola_domain::PromptCacheUsage::measured(Some(input),Some(read),Some(0),false)});
        let attempt =
            |id, input, read| json!({"id":id,"state":"succeeded","usage":usage(input,read)});
        event("r1", 1, "model_attempt_changed", attempt("a", 100, 100));
        event("r1", 2, "model_attempt_changed", attempt("a", 100, 100)); // duplicate state, not another request
        event("r2", 1, "model_attempt_changed", attempt("b", 900, 0));
        event(
            "other-run",
            1,
            "model_attempt_changed",
            attempt("a", 9999, 9999),
        );
        let total = store.conversation_cache_usage("c").unwrap().unwrap();
        assert_eq!(total.totals.input_tokens, Some(1000));
        assert_eq!(total.totals.read_tokens, Some(100));
        assert_eq!(total.totals.hit_rate_bps, Some(1000)); // not (100%+0%)/2
        event(
            "r2",
            2,
            "compaction_completed",
            json!({"compaction_item_id":"compact"}),
        );
        let compact = json!({"context_usage":{"cache_usage":nola_domain::PromptCacheUsage::measured(Some(1000),Some(900),Some(0),true)}});
        event("r2", 3, "context_updated", compact.clone());
        event("r2", 4, "context_updated", compact);
        // Compaction has its own physical ModelAttempt; context notifications
        // must not count its token usage a second time.
        event(
            "r2",
            5,
            "model_attempt_changed",
            json!({"id":"compact-attempt","state":"succeeded",
            "usage":{"cache_usage":nola_domain::PromptCacheUsage::measured(Some(1000),Some(900),Some(0),true)}}),
        );
        let total = store.conversation_cache_usage("c").unwrap().unwrap();
        assert_eq!(total.totals.input_tokens, Some(2000));
        assert_eq!(total.totals.hit_rate_bps, Some(5000));
        assert!(!total.totals.is_compaction);
        drop(store);
        let store = NolaStore::open(&path).unwrap();
        assert_eq!(store.conversation_cache_usage("c").unwrap(), Some(total));
        assert!(store.conversation_cache_usage("empty").unwrap().is_none());
        store.connection.execute("INSERT INTO run_events VALUES('unknown','r2',6,'model_attempt_changed',?1,1)",
            [json!({"id":"unknown","state":"succeeded","usage":{"input_tokens":50,"cached_input_tokens_reported":false}}).to_string()]).unwrap();
        let unknown = store.conversation_cache_usage("c").unwrap().unwrap();
        assert_eq!(unknown.totals.input_tokens, Some(2000));
        assert_eq!(unknown.totals.read_tokens, Some(1000));
        assert_eq!(unknown.totals.hit_rate_bps, Some(5000));
        assert_eq!(unknown.reported_requests, 3);
        assert_eq!(unknown.unreported_requests, 1);
        for (seq, id, state, usage) in [
            (7, "retry", "superseded", serde_json::Value::Null),
            (
                8,
                "malformed",
                "succeeded",
                json!({"cache_usage":{"input_tokens":-1,"read_tokens":"bad"}}),
            ),
        ] {
            store
                .connection
                .execute(
                    "INSERT INTO run_events VALUES(?1,'r2',?2,'model_attempt_changed',?3,1)",
                    params![
                        id,
                        seq,
                        json!({"id":id,"state":state,"started_at_ms":1,"usage":usage}).to_string()
                    ],
                )
                .unwrap();
        }
        store
            .connection
            .execute(
                "INSERT INTO run_events VALUES('not-sent','r2',9,'model_attempt_changed',?1,1)",
                [
                    json!({"id":"not-sent","state":"failed","started_at_ms":null,"usage":null})
                        .to_string(),
                ],
            )
            .unwrap();
        let partial = store.conversation_cache_usage("c").unwrap().unwrap();
        assert_eq!(partial.totals, unknown.totals);
        assert_eq!(partial.reported_requests, 3);
        assert_eq!(partial.unreported_requests, 3);
    }
}
impl NolaStore {
    /// Rebuildable read model, never a client-side counter. Immutable committed
    /// request events are the persistent ledger; pagination, compaction and
    /// duplicate deliveries cannot reset or increment these totals twice.
    pub fn conversation_cache_usage(
        &self,
        conversation: &str,
    ) -> StoreResult<Option<nola_domain::ConversationCacheUsage>> {
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        let exists: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM conversations WHERE id=?1)",
            [conversation],
            |row| row.get(0),
        )?;
        if !exists {
            return Ok(None);
        }
        let mut statement = self.connection.prepare(
            "SELECT r.id,s.source_host_id,u.record_json FROM runs r
             LEFT JOIN remote_run_sources s ON s.run_id=r.id
             LEFT JOIN remote_run_context_usage u ON u.run_id=r.id AND u.source_host_id=s.source_host_id
             WHERE r.conversation_id=?1"
        )?;
        let rows = statement.query_map([conversation], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?;
        let mut combined: Option<nola_domain::ConversationCacheUsage> = None;
        for row in rows {
            let (run, host, json) = row?;
            // Remote Runs never contribute the local projection's copied events.
            let usage = if let Some(host) = host {
                let Some(json) = json else { continue };
                let record: CommittedRunContextUsage = serde_json::from_str(&json)?;
                record.validate().map_err(invalid)?;
                if record.identity.run_id != run || record.identity.conversation_id != conversation
                {
                    return Err(invalid("cache usage Run owner differs"));
                }
                let received =
                    self.run_message_admission_reception_state(&host, &record.admission)?;
                if received.sequence.0 < record.admission.sequence.0 {
                    return Err(invalid("cache usage admission missing"));
                }
                record.run_cache_usage
            } else {
                self.local_run_cache_usage(&run)?
            };
            let Some(usage) = usage else { continue };
            merge_cache_usage(&mut combined, usage)?;
        }
        // Operation and Attempt identities are canonical rows, not streamed
        // notifications. Late usage updates the same row and cannot add a
        // second request. No account-wide or currently selected UI scope leaks
        // into this owner's totals.
        let mut statement = self.connection.prepare(
            "SELECT o.record_json,a.record_json FROM auxiliary_operations o
             JOIN run_attempts a ON a.operation_id=o.id AND a.run_id IS NULL
             WHERE json_extract(o.record_json,'$.owner.kind')='conversation'
               AND json_extract(o.record_json,'$.owner.conversation_id')=?1
               AND a.state<>'prepared'",
        )?;
        let rows = statement.query_map([conversation], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (operation, attempt) = row?;
            let operation: nola_domain::AuxiliaryOperation = serde_json::from_str(&operation)?;
            let attempt: nola_domain::AuxiliaryModelAttempt = serde_json::from_str(&attempt)?;
            if operation.owner
                != (nola_domain::AuxiliaryInvocationOwner::Conversation {
                    conversation_id: conversation.into(),
                })
                || attempt.operation_id != operation.id
            {
                return Err(invalid("auxiliary cache usage owner differs"));
            }
            merge_cache_usage(
                &mut combined,
                nola_domain::ConversationCacheUsage::from_request(
                    nola_domain::PromptCacheUsage::from_auxiliary_attempt_usage(&attempt.usage),
                ),
            )?;
        }
        let mut statement = self.connection.prepare(
            "SELECT source_host_id,operation_id,record_json FROM auxiliary_source_observations
             WHERE json_extract(local_owner_json,'$.kind')='conversation'
               AND json_extract(local_owner_json,'$.conversation_id')=?1",
        )?;
        let rows = statement.query_map([conversation], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        for row in rows {
            let (host, operation_id, raw) = row?;
            let observation: nola_domain::AuxiliarySourceObservation = serde_json::from_str(&raw)?;
            let receipt = &observation.receipt;
            if observation.source_host_id != host
                || observation.operation_id != operation_id
                || receipt.operation.id != operation_id
                || receipt.operation.configuration.max_attempts != 1
                || !matches!(&receipt.operation.owner,
                    nola_domain::AuxiliaryInvocationOwner::SourceAuthority { source_host_id, .. } if source_host_id == &host)
            {
                return Err(invalid("source auxiliary cache usage identity differs"));
            }
            let Some(attempt) = &receipt.attempt else {
                continue;
            };
            if attempt.operation_id != operation_id || attempt.id != observation.attempt_id {
                return Err(invalid("source auxiliary cache Attempt owner differs"));
            }
            // A source observation is a reference, not another physical row.
            // Conflicting ownership must not manufacture a second charge.
            let duplicated: bool = self.connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM run_attempts a JOIN auxiliary_operations o ON o.id=a.operation_id
                 WHERE a.id=?1 AND o.id=?2 AND json_extract(o.record_json,'$.owner.kind')='conversation'
                   AND json_extract(o.record_json,'$.owner.conversation_id')=?3)",
                params![attempt.id, operation_id, conversation], |row| row.get(0))?;
            if duplicated {
                return Err(invalid(
                    "source auxiliary Attempt already has a local physical owner",
                ));
            }
            if attempt.state != nola_domain::ModelAttemptState::Prepared {
                merge_cache_usage(
                    &mut combined,
                    nola_domain::ConversationCacheUsage::from_request(
                        nola_domain::PromptCacheUsage::from_auxiliary_attempt_usage(&attempt.usage),
                    ),
                )?;
            }
        }
        Ok(combined)
    }

    fn local_run_cache_usage(
        &self,
        run: &str,
    ) -> StoreResult<Option<nola_domain::ConversationCacheUsage>> {
        let mut statement = self.connection.prepare(
            "WITH attempts AS (
                SELECT e.payload_json, ROW_NUMBER() OVER (
                    PARTITION BY e.run_id,json_extract(e.payload_json,'$.id') ORDER BY e.sequence DESC
                ) AS version
                FROM run_events e
                WHERE e.run_id=?1 AND e.event_kind='model_attempt_changed'
            )
            SELECT json_extract(payload_json,'$.usage') FROM attempts
            WHERE version=1 AND json_extract(payload_json,'$.state') IN ('running','succeeded','failed','cancelled','superseded','interrupted')
            AND (json_extract(payload_json,'$.started_at_ms') IS NOT NULL OR json_type(payload_json,'$.usage')='object')"
        )?;
        let rows = statement.query_map([run], |row| row.get::<_, Option<String>>(0))?;
        let mut total = None;
        for row in rows {
            let value: serde_json::Value = row?
                .map(|json| serde_json::from_str(&json))
                .transpose()?
                .unwrap_or_default();
            merge_cache_usage(
                &mut total,
                nola_domain::ConversationCacheUsage::from_request(
                    nola_domain::PromptCacheUsage::from_model_attempt_usage(&value),
                ),
            )?;
        }
        Ok(total)
    }

    pub fn local_run_context_usage(
        &self,
        identity: &AssistantJournalIdentity,
    ) -> StoreResult<Option<CommittedRunContextUsage>> {
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        // Imported history can carry Run metadata without a source execution
        // or admission. Only a real committed usage fact requires that proof.
        let event: Option<(i64,String)> = self.connection.query_row(
            "SELECT sequence,payload_json FROM run_events WHERE run_id=?1 AND event_kind='context_updated' ORDER BY sequence DESC LIMIT 1",
            [&identity.run_id], |r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let Some((sequence, payload)) = event else {
            return Ok(None);
        };
        let root = crate::run_message_admission::local_head(
            &self.connection,
            &identity.conversation_id,
            &identity.run_id,
        )?;
        if root.identity != *identity {
            return Err(invalid("context usage source root differs"));
        }
        if root.sequence.0 == 0 {
            return Err(invalid("context usage source admission missing"));
        }
        let owner_valid: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM messages WHERE id=?1 AND conversation_id=?2 AND run_id=?3 AND role='assistant')",
            params![identity.assistant_message_id, identity.conversation_id, identity.run_id], |r| r.get(0))?;
        if !owner_valid {
            return Err(invalid("context usage source message owner differs"));
        }
        #[derive(serde::Deserialize)]
        struct Usage {
            #[serde(default)]
            cache_usage: Option<nola_domain::PromptCacheUsage>,
            used_tokens: u64,
            context_window_tokens: u64,
            prompt_token_limit: u64,
            usage_is_estimated: bool,
        }
        #[derive(serde::Deserialize)]
        struct Event {
            context_usage: Usage,
            measurement: Option<nola_domain::ContextUsageMeasurement>,
            prompt_cache_measurement: Option<nola_domain::ContextUsageMeasurement>,
            prompt_cache_diagnostic: Option<nola_domain::PromptCacheDiagnostic>,
        }
        let event: Event = serde_json::from_str(&payload)?;
        let owner = serde_json::to_string(identity)?;
        // Root declarations are immutable. A later steering admission cannot
        // change the digest of an already-observed context event.
        let declaration: String = self.connection.query_row(
            "SELECT record_json FROM run_message_admission_records WHERE owner=?1 AND json_extract(record_json,'$.message_id')=?2 ORDER BY sequence DESC LIMIT 1",
            params![owner,identity.assistant_message_id], |r|r.get(0))?;
        let declaration: RunMessageAdmissionRecord = serde_json::from_str(&declaration)?;
        declaration.validate().map_err(invalid)?;
        if declaration
            .offset
            .0
            .checked_add(declaration.text.len() as u64)
            != Some(declaration.total_utf8_bytes.0)
        {
            return Err(invalid("context usage root admission is incomplete"));
        }
        let mut record = CommittedRunContextUsage {
            protocol_version: 1,
            identity: identity.clone(),
            admission: declaration.cursor,
            event_sequence: JournalNumber(u64::try_from(sequence).map_err(invalid)?),
            attempt_sequence: JournalNumber(self.connection.query_row(
                "SELECT COALESCE(MAX(sequence),0) FROM run_events WHERE run_id=?1 AND event_kind='model_attempt_changed'",
                [&identity.run_id], |r| r.get::<_,u64>(0))?),
            run_cache_usage: self.local_run_cache_usage(&identity.run_id)?,
            usage: CommittedContextUsage {
                measurement: event.measurement,
                prompt_cache_measurement: event.prompt_cache_measurement,
                prompt_cache_diagnostic: event.prompt_cache_diagnostic,
                cache_usage: event.context_usage.cache_usage,
                used_tokens: JournalNumber(event.context_usage.used_tokens),
                context_window_tokens: JournalNumber(event.context_usage.context_window_tokens),
                prompt_token_limit: JournalNumber(event.context_usage.prompt_token_limit),
                estimated: event.context_usage.usage_is_estimated,
            },
            record_digest: AssistantDigest::parse_hex(&"0".repeat(64)).map_err(invalid)?,
        };
        record.record_digest = record.computed_digest().map_err(invalid)?;
        record.validate().map_err(invalid)?;
        Ok(Some(record))
    }

    pub fn accept_remote_run_context_usage(
        &mut self,
        host: &str,
        record: &CommittedRunContextUsage,
    ) -> StoreResult<CommittedRunContextUsage> {
        if self.connection.is_autocommit() {
            return Err(invalid("context usage reception requires a unit of work"));
        }
        record.validate().map_err(invalid)?;
        let after = self.run_message_admission_reception_state(host, &record.admission)?;
        if after.sequence.0 < record.admission.sequence.0 {
            return Err(invalid("context usage requires received root admission"));
        }
        let existing: Option<(String, String)> = self
            .connection
            .query_row(
                "SELECT source_host_id,record_json FROM remote_run_context_usage WHERE run_id=?1",
                [&record.identity.run_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((old_host, json)) = existing {
            let old: CommittedRunContextUsage = serde_json::from_str(&json)?;
            old.validate().map_err(invalid)?;
            if old_host != host || old.identity != record.identity {
                return Err(invalid("context usage source changed"));
            }
            let context_order = record.event_sequence.cmp(&old.event_sequence);
            let attempt_order = record.attempt_sequence.cmp(&old.attempt_sequence);
            use std::cmp::Ordering::{Equal, Greater, Less};
            if context_order == Equal && attempt_order == Equal && old != *record {
                return Err(invalid("context usage sequence fork"));
            }
            if matches!(
                (context_order, attempt_order),
                (Less, Greater) | (Greater, Less)
            ) {
                return Err(invalid("context usage crossed frontiers"));
            }
            if context_order == Equal && old.usage != record.usage {
                return Err(invalid(
                    "context usage unchanged context frontier changed usage",
                ));
            }
            if attempt_order == Equal && old.run_cache_usage != record.run_cache_usage {
                return Err(invalid(
                    "context usage unchanged attempt frontier changed totals",
                ));
            }
            if context_order != Greater && attempt_order != Greater {
                return Ok(old);
            }
        }
        self.connection.execute("INSERT INTO remote_run_context_usage(run_id,source_host_id,event_sequence,record_json) VALUES(?1,?2,?3,?4)
            ON CONFLICT(run_id) DO UPDATE SET event_sequence=excluded.event_sequence,record_json=excluded.record_json",
            params![record.identity.run_id,host,i64::try_from(record.event_sequence.0).map_err(invalid)?,serde_json::to_string(record)?])?;
        Ok(record.clone())
    }

    pub fn conversation_committed_context_usage(
        &self,
        conversation: &str,
    ) -> StoreResult<Option<CommittedRunContextUsage>> {
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        let latest:Option<(String,Option<String>,Option<String>)> = self.connection.query_row(
            "SELECT r.id,r.assistant_message_id,s.source_host_id FROM runs r LEFT JOIN remote_run_sources s ON s.run_id=r.id
             WHERE r.conversation_id=?1 AND r.kind='conversation' ORDER BY r.created_at_ms DESC,r.rowid DESC LIMIT 1",
            [conversation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        let Some((run, assistant, host)) = latest else {
            return Ok(None);
        };
        let Some(assistant) = assistant else {
            let has_fact:bool = self.connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM run_events WHERE run_id=?1 AND event_kind='context_updated') OR EXISTS(SELECT 1 FROM remote_run_context_usage WHERE run_id=?1)",
                [&run],|r|r.get(0))?;
            return if has_fact {
                Err(invalid("context usage Run has no root owner"))
            } else {
                Ok(None)
            };
        };
        if let Some(host) = host {
            let json:Option<String> = self.connection.query_row("SELECT record_json FROM remote_run_context_usage WHERE run_id=?1 AND source_host_id=?2",
                params![run,host],|r|r.get(0)).optional()?;
            let Some(json) = json else { return Ok(None) };
            let record: CommittedRunContextUsage = serde_json::from_str(&json)?;
            record.validate().map_err(invalid)?;
            if record.identity.conversation_id != conversation
                || record.identity.run_id != run
                || record.identity.assistant_message_id != assistant
            {
                return Err(invalid("context usage read owner changed"));
            }
            let after = self.run_message_admission_reception_state(&host, &record.admission)?;
            if after.sequence.0 < record.admission.sequence.0 {
                return Err(invalid("context usage read admission missing"));
            }
            Ok(Some(record))
        } else {
            self.local_run_context_usage(&AssistantJournalIdentity {
                source_store_epoch: self.assistant_source_epoch()?,
                conversation_id: conversation.into(),
                run_id: run,
                assistant_message_id: assistant,
            })
        }
    }
}
