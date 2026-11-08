// SPDX-License-Identifier: Apache-2.0
//! Decision provenance from immutable committed event ordering, never latest state.
use super::*;
use nola_store_port::run_decision::{RunDecisionQuery, RunDecisionRepository, StoredRunDecision};
use nola_domain::core_event::CoreEventSource;


#[cfg(test)]
mod tests {
    use super::*;

    fn query() -> RunDecisionQuery {
        RunDecisionQuery { conversation_id:"conversation".into(),run_id:"run".into(),
            turn_id:"run".into(),core_revision:11,core_event_cursor:5 }
    }

    fn fixture(path:&std::path::Path)->NolaStore {
        let mut store=NolaStore::open(path).unwrap();
        let mut input=crate::input_facts::tests::input(
            crate::run_requirements::tests::events().into_iter().take(3).collect());
        input.checkpoint_event_cursor=3;
        let mut checkpoint:serde_json::Value=serde_json::from_str(&input.checkpoint_json).unwrap();
        checkpoint["last_event_cursor"]["sequence"]=3.into();
        input.checkpoint_json=checkpoint.to_string();
        store.begin_turn(&input).unwrap();
        store
    }

    fn event()->serde_json::Value {
        serde_json::from_str(&crate::run_requirements::tests::events()[4].payload_json).unwrap()
    }

    fn commit(store:&mut NolaStore,revision:i64,event:Option<serde_json::Value>)->StoreResult<()> {
        let mut events=Vec::new();
        if let Some(result)=event {
            let error:serde_json::Value=serde_json::from_str(
                &crate::run_requirements::tests::events()[3].payload_json).unwrap();
            for (cursor,mut payload) in [(4,error),(5,result)] {
                payload["core_envelope"]=serde_json::json!({"session_id":"core","turn_id":"run",
                    "revision":11,"cursor":{"sequence":cursor}});
                events.push(nola_store_port::run_effect::RunEventInput {
                    event_id:format!("core:run:{cursor}"),sequence:cursor,
                    event_kind:payload["type"].as_str().unwrap().into(),
                    payload_json:payload.to_string(),created_at_ms:revision,
                });
            }
        }
        store.commit_transition(&nola_store_port::run_effect::CommitTransitionInput {
            transition_id:format!("transition-{revision}"),run_id:"run".into(),
            expected_checkpoint_revision:revision-1,checkpoint_revision:revision,checkpoint_event_cursor:5,
            checkpoint:nola_store_port::run_effect::CheckpointWrite::Baseline {
                checkpoint_json:serde_json::json!({"session_id":"core","revision":revision+9,
                    "last_event_cursor":{"sequence":5},"state":{"session_id":"core"}}).to_string() },
            host_event:None,submission_reduction_json:None,events,ensured_messages:vec![],
            part_message_owners:Default::default(),assistant_turn_bindings:vec![],part_mutations:vec![],
            outbox_effects:vec![],model_attempt_mutations:vec![],delegated_invocation_mutations:vec![],
            delegated_permission_mutations:vec![],effect_delivery_attempt_mutations:vec![],
            acknowledged_effect_ids:vec![],memory_usage_entries:vec![],run_status:None,
            assistant_status:None,assistant_status_message_id:None,committed_at_ms:revision,
        })?;
        Ok(())
    }

    #[test]
    fn run_decision_reopens_exact_origin_after_later_checkpoint_and_empty_commits() {
        let directory=tempfile::tempdir().unwrap();
        let path=directory.path().join("decision.sqlite");
        let mut store=fixture(&path);
        assert!(store.load_run_decision(&query()).unwrap().is_none());
        commit(&mut store,2,Some(event())).unwrap();
        let expected=store.load_run_decision(&query()).unwrap().unwrap();
        assert_eq!(expected.source_transition_id,"transition-2");
        assert_eq!(expected.checkpoint_revision,2);
        assert_eq!(serde_json::from_str::<serde_json::Value>(&expected.event_json).unwrap()
            ["decision"]["value"]["kind"],"failed");
        commit(&mut store,3,None).unwrap();
        drop(store);
        let store=NolaStore::open(&path).unwrap();
        assert_eq!(store.load_run_decision(&query()).unwrap(),Some(expected));
        let mut wrong=query();wrong.core_event_cursor=999;
        assert!(store.load_run_decision(&wrong).unwrap().is_none());
        for field in ["run","conversation","turn","revision"] {
            let mut wrong=query();
            match field {
                "run"=>wrong.run_id="foreign".into(),
                "conversation"=>wrong.conversation_id="foreign".into(),
                "turn"=>wrong.turn_id="foreign".into(),
                _=>wrong.core_revision+=1,
            }
            assert!(store.load_run_decision(&wrong).is_err(),"{field}");
        }
    }

    #[test]
    fn legacy_turn_terminal_is_not_a_run_result_and_bad_result_cannot_commit() {
        let mut store=fixture(std::path::Path::new(":memory:"));
        commit(&mut store,2,Some(serde_json::json!({
            "type":"turn_completed","turn_id":"run","session_id":"core","outcome":"completed"
        }))).unwrap();
        assert!(store.load_run_decision(&query()).unwrap().is_none());
        for damage in ["turn","scope","frontier","source","cancel"] {
            let mut store=fixture(std::path::Path::new(":memory:"));
            let mut payload=event();
            match damage {
                "turn"=>payload["turn_id"]="foreign".into(),
                "scope"=>payload["decision"]["requirement_revision"]="2".into(),
                "frontier"=>payload["decision"]["evidence_frontier"]="999".into(),
                "source"=>payload["decision"]["source"]=serde_json::json!({"kind":"model",
                    "effect_id":"old-effect","request_id":"old-request","attempt_id":null,"call_id":null}),
                _=>payload["decision"]["value"]=serde_json::json!({"kind":"cancelled",
                    "cancellation_id":"fabricated","observed_effect_refs":[],"unknown_effect_refs":[]}),
            }
            assert!(commit(&mut store,2,Some(payload)).is_err(),"{damage}");
            assert!(store.load_run_decision(&query()).unwrap().is_none(),"{damage}");
        }
    }
}

impl RunDecisionRepository for NolaStore {
    type Error = StoreError;

    fn load_run_decision(&self, query: &RunDecisionQuery) -> StoreResult<Option<StoredRunDecision>> {
        for (field, value) in [("conversation_id", &query.conversation_id),
            ("run_id", &query.run_id), ("turn_id", &query.turn_id)] {
            require_id(field, value)?;
        }
        let _snapshot = read_snapshot::pin(&self.connection)?;
        let owned: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM runs r WHERE r.id=?1 AND r.conversation_id=?2
             AND r.kind='conversation' AND NOT EXISTS(SELECT 1 FROM remote_run_sources s WHERE s.run_id=r.id))",
            params![query.run_id, query.conversation_id], |row| row.get(0))?;
        if !owned {
            return Err(StoreError::Conflict("decision source is not the exact local Run".into()));
        }
        let event_id = format!("core:{}:{}", query.run_id, query.core_event_cursor);
        let row: Option<(i64, String, String)> = self.connection.query_row(
            "SELECT sequence,event_kind,payload_json FROM run_events WHERE event_id=?1 AND run_id=?2",
            params![event_id, query.run_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).optional()?;
        let Some((sequence, kind, event_json)) = row else { return Ok(None); };
        if kind != "run_result_decided" {
            return Ok(None);
        }
        let payload: serde_json::Value = serde_json::from_str(&event_json)?;
        let identity: CoreEventSource = serde_json::from_value(payload.get("core_envelope")
            .cloned().ok_or_else(|| StoreError::Invariant("decision has no committed Core envelope".into()))?)?;
        if identity.cursor.sequence != query.core_event_cursor || identity.revision != query.core_revision
            || identity.revision == 0 || identity.session_id.trim().is_empty()
            || identity.turn_id.as_deref() != Some(query.turn_id.as_str())
            || payload.get("type").and_then(|v| v.as_str()) != Some(kind.as_str())
            || payload.get("turn_id").and_then(|v| v.as_str()) != Some(query.turn_id.as_str()) {
            return Err(StoreError::Invariant("decision identity/frontier differs".into()));
        }
        let bound: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM assistant_turn_message_bindings WHERE conversation_id=?1 AND run_id=?2 AND turn_id=?3)",
            params![query.conversation_id, query.run_id, query.turn_id], |row| row.get(0))?;
        if !bound { return Err(StoreError::Invariant("decision Turn is not owned by Run".into())); }
        let result=crate::run_requirements::validate_result_event(&self.connection,&event_id)?;
        if result.run_id!=query.run_id || result.conversation_id!=query.conversation_id
            || result.decided_at_revision.0!=query.core_revision || result.decision_event_cursor.0!=query.core_event_cursor {
            return Err(StoreError::Invariant("Run result source binding differs".into()));
        }
        // A transition stores its exclusive end sequence. The first committed
        // boundary containing the event is its origin, including empty later batches.
        let transition: Option<(String, u64)> = self.connection.query_row(
            "SELECT transition_id,checkpoint_revision FROM committed_transitions
             WHERE run_id=?1 AND next_event_sequence>?2 ORDER BY checkpoint_revision LIMIT 1",
            params![query.run_id, sequence], |row| Ok((row.get(0)?, row.get(1)?))).optional()?;
        let Some((source_transition_id, checkpoint_revision)) = transition else {
            return Err(StoreError::Invariant("decision has no committed transition".into()));
        };
        Ok(Some(StoredRunDecision { source_transition_id, checkpoint_revision,
            core_session_id: identity.session_id, event_json }))
    }
}
