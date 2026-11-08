// SPDX-License-Identifier: Apache-2.0
//! Task scheduling follows committed question heads; it never owns question data.
use crate::{params, Connection, OptionalExtension, StoreError, StoreResult};

/// Called after all wait facts in a transition have been indexed, inside that
/// same write transaction. Only the bound Task is affected; effects are untouched.
pub(crate) fn synchronize(c: &Connection, run_id: &str, now: i64) -> StoreResult<()> {
    if c.is_autocommit() {
        return Err(StoreError::Invariant("Task wait projection requires the transition transaction".into()));
    }
    let task: Option<(String, String, bool)> = c.query_row(
        "SELECT t.id,t.status,t.cancel_requested FROM runtime_task_run_bindings b
         JOIN durable_tasks t ON t.id=b.task_id AND t.kind='agent' AND t.child_session_id=b.child_session_id
           AND t.owner_session_id=b.owner_session_id
         WHERE b.run_id=?1", [run_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    ).optional()?;
    let Some((task_id, status, cancelled)) = task else { return Ok(()); };
    let open: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM user_input_wait_heads WHERE run_id=?1 AND state='open')", [run_id], |r| r.get(0))?;
    // An open question does not stop independent effects in the same batch.
    // Only Core's committed wait-only lifecycle relinquishes the executor.
    let wait_only: bool = c.query_row("SELECT status='waiting_input' FROM runs WHERE id=?1", [run_id], |r| r.get(0))?;
    if open && wait_only && status == "running" && !cancelled {
        if !crate::agent_task_admission_store::permits_execution(c, &task_id)? {
            return Err(StoreError::Conflict("Task question has no admitted authority".into()));
        }
        let attempt_id: String = c.query_row("SELECT id FROM runtime_task_attempts WHERE task_id=?1 AND state='leased'", [&task_id], |r| r.get(0))?;
        let retired = c.execute(
            "UPDATE runtime_task_attempts SET state='superseded',revision=revision+1
             WHERE task_id=?1 AND state='leased' AND lease_expires_at_ms>?2
               AND EXISTS(SELECT 1 FROM durable_tasks t WHERE t.id=?1 AND t.worker_id=runtime_task_attempts.worker_id
                   AND t.ownership_token=runtime_task_attempts.lease_token
                   AND t.lease_until_ms=runtime_task_attempts.lease_expires_at_ms)", params![task_id,now])?;
        if retired != 1 { return Err(StoreError::Conflict("Task question requires its live exact attempt".into())); }
        c.execute("UPDATE durable_tasks SET status='waiting_input',worker_id=NULL,ownership_token=NULL,
            lease_until_ms=NULL,heartbeat_at_ms=NULL,updated_at_ms=?2,unread=1,progress_summary='Waiting for user input'
            WHERE id=?1", params![task_id,now])?;
        c.execute("INSERT INTO durable_task_events(task_id,kind,status,summary,detail_json,created_at_ms)
            VALUES(?1,'user_input_wait_opened','waiting_input','Waiting for user input',?2,?3)",
            params![task_id,serde_json::json!({"run_id":run_id,"retired_attempt_id":attempt_id}).to_string(),now])?;
    } else if !open && status == "waiting_input" && !cancelled {
        // A response grants no execution authority. The regular fair scheduler
        // must issue a fresh attempt; its worker resumes the immutable bound Run.
        let active: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM runs WHERE id=?1 AND status <> 'sealed')
            AND (SELECT state FROM user_input_wait_heads WHERE run_id=?1 ORDER BY core_cursor DESC LIMIT 1)='resolved'", [run_id], |r| r.get(0))?;
        if active {
            c.execute("UPDATE durable_tasks SET status='queued',retry_at_ms=NULL,updated_at_ms=?2,
                progress_summary='User input received' WHERE id=?1", params![task_id,now])?;
            c.execute("INSERT INTO durable_task_events(task_id,kind,status,summary,detail_json,created_at_ms)
                VALUES(?1,'user_input_wait_resolved','queued','User input received',?2,?3)",
                params![task_id,serde_json::json!({"run_id":run_id}).to_string(),now])?;
        }
    }
    Ok(())
}

impl crate::NolaStore {
    /// Read-only negative-authority evidence: this exact attempt must stop
    /// renewing, without mistaking committed suspension for user cancellation.
    pub fn runtime_task_attempt_retired_for_user_input(&self, task_id: &str, attempt_id: &str) -> StoreResult<bool> {
        self.connection.query_row("SELECT EXISTS(SELECT 1 FROM durable_task_events e
            JOIN runtime_task_attempts a ON a.id=?2 AND a.task_id=e.task_id AND a.state='superseded'
            JOIN runtime_task_run_bindings b ON b.task_id=e.task_id
            WHERE e.task_id=?1 AND e.kind='user_input_wait_opened'
              AND json_extract(e.detail_json,'$.retired_attempt_id')=a.id
              AND json_extract(e.detail_json,'$.run_id')=b.run_id)",params![task_id,attempt_id],|r|r.get(0)).map_err(StoreError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NolaStore;
    use serde_json::json;

    fn fixture() -> NolaStore {
        fixture_at(std::path::Path::new(":memory:"))
    }

    fn fixture_at(path: &std::path::Path) -> NolaStore {
        let mut store = NolaStore::open(path).unwrap();
        let set: nola_domain::user_input_wait::QuestionSet = serde_json::from_value(json!({"questions":[{
            "id":"q","prompt":"Choose","options":[{"id":"a","label":"A"}],
            "allow_freeform":false,"required":true,"max_text_chars":32}]})).unwrap();
        let wait = nola_domain::user_input_wait::UserInputWait {
            wait_id:"wait".into(),core_session_id:"core".into(),turn_id:"run".into(),
            originating_call_id:None,revision:1,created_at_revision:10,
            response_schema_digest:set.digest().unwrap(),question_set:set,
            state:nola_domain::user_input_wait::UserInputWaitState::Open,
        };
        store.begin_turn(&crate::input_facts::tests::input(vec![nola_store_port::run_creation::InitialCoreRunEvent {
            core_session_id:"core".into(),core_turn_id:Some("run".into()),core_cursor:1,core_revision:10,
            event_kind:"user_input_wait_changed".into(),payload_json:json!({"type":"user_input_wait_changed","wait":wait}).to_string(),
        }])).unwrap();
        let task = serde_json::from_value(json!({"id":"task","kind":"agent","status":"queued","title":"Task",
            "project_root":"/fixture","owner_session_id":"parent","child_session_id":"conversation","parent_tool_call_id":"call",
            "unread":false,"cancel_requested":false,"retry_count":0,"created_at_ms":1,"updated_at_ms":1,"metadata_json":"{}"})).unwrap();
        store.create_runtime_task_with_event(&task).unwrap();
        // A fully formed immutable admission fixture, not a bypass of the
        // production fair scheduler or live-attempt checks under test.
        let mut admission: nola_domain::agent_task_admission::AgentTaskAdmission = serde_json::from_value(json!({
            "protocol_version":nola_domain::agent_task_admission::AGENT_TASK_ADMISSION_VERSION,"task_id":"task","parent_conversation_id":"parent","parent_core_session_id":"parent-core",
            "parent_run_id":"parent-run","parent_turn_id":"parent-turn","child_conversation_id":"conversation",
            "parent_tool_call_id":"call","parent_effect_id":"effect","input_watermark":null,
            "source_store_epoch":"epoch","source_transition_id":"transition","source_host_event_id":"host-event",
            "source_checkpoint_revision":1,"core_revision":1,"core_event_cursor":1,"admission_digest":""
        })).unwrap();
        admission.admission_digest = admission.canonical_digest().unwrap();
        store.connection.execute("UPDATE runtime_agent_task_authorities SET phase='admitted',authority_revision=2,admission_json=?1 WHERE task_id='task'", [serde_json::to_string(&admission).unwrap()]).unwrap();
        store.claim_runtime_task_fair("task","worker","generation","lease",10,1000,1,1).unwrap().unwrap();
        store.connection.execute("INSERT INTO runtime_task_run_bindings VALUES('task','parent','conversation','run','attempt',NULL,10)", []).unwrap();
        // This leaf fixture isolates scheduling after Core's wait-only commit.
        store.connection.execute("UPDATE runs SET status='waiting_input' WHERE id='run'", []).unwrap();
        store
    }

    #[test]
    fn open_question_preserves_executor_until_independent_tools_finish() {
        let mut store = fixture();
        store.connection.execute("UPDATE runs SET status='running' WHERE id='run'", []).unwrap();
        store.begin_write_unit_of_work().unwrap();
        synchronize(&store.connection, "run", 20).unwrap();
        store.commit_write_unit_of_work().unwrap();
        let active = store.runtime_task("task").unwrap().unwrap();
        assert_eq!(active.status, "running");
        assert_eq!(active.ownership_token.as_deref(), Some("lease"));
        let leased: i64 = store.connection.query_row(
            "SELECT count(*) FROM runtime_task_attempts WHERE task_id='task' AND state='leased'", [], |r| r.get(0)).unwrap();
        assert_eq!(leased, 1);
        let retired: i64 = store.connection.query_row(
            "SELECT count(*) FROM durable_task_events WHERE task_id='task' AND kind='user_input_wait_opened'", [], |r| r.get(0)).unwrap();
        assert_eq!(retired, 0);
        // No second question event is necessary: the ordinary last-tool
        // transition publishes the Core lifecycle and re-evaluates suspension.
        store.begin_write_unit_of_work().unwrap();
        store.connection.execute("UPDATE runs SET status='waiting_input' WHERE id='run'", []).unwrap();
        synchronize(&store.connection, "run", 30).unwrap();
        store.commit_write_unit_of_work().unwrap();
        assert_eq!(store.runtime_task("task").unwrap().unwrap().status, "waiting_input");
        assert!(store.runtime_task("task").unwrap().unwrap().ownership_token.is_none());
    }

    #[test]
    fn question_delivery_is_exact_version_consumer_ack_not_answer_or_permission() {
        use nola_store_port::runtime_task::{RuntimeTaskNotificationAcknowledgement as Ack, RuntimeTaskNotificationDelivery, user_input_notification_occurrence_id};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("question-ack.sqlite");
        let mut store = fixture_at(&path);
        store.begin_write_unit_of_work().unwrap();
        synchronize(&store.connection, "run", 20).unwrap();
        store.commit_write_unit_of_work().unwrap();
        let schema: String = store.connection.query_row("SELECT response_schema_digest FROM user_input_wait_heads WHERE wait_id='wait'", [], |r|r.get(0)).unwrap();
        let delivery = RuntimeTaskNotificationDelivery {
            owner_conversation_id:Some("parent".into()),task_id:"task".into(),
            occurrence_id:user_input_notification_occurrence_id("run","wait",1,&schema),
            consumer_id:"cli".into(),command_id:"question-display".into(),
        };
        assert!(!store.runtime_task_notification_delivered(&delivery).unwrap());
        for (run,id,revision,digest) in [("other","wait",1,schema.as_str()),("run","other",1,schema.as_str()),
            ("run","wait",2,schema.as_str()),("run","wait",1,"wrong-schema")] {
            let mut wrong = delivery.clone();
            wrong.occurrence_id=user_input_notification_occurrence_id(run,id,revision,digest);
            assert!(store.acknowledge_runtime_task_delivery(&wrong).is_err());
        }
        let mut wrong = delivery.clone();
        wrong.owner_conversation_id=Some("other".into());
        assert!(store.acknowledge_runtime_task_delivery(&wrong).is_err());
        wrong.owner_conversation_id=None;
        assert!(store.acknowledge_runtime_task_delivery(&wrong).is_err());
        assert!(store.append_runtime_task_event("task","notification_delivered","delivered",None,
            &serde_json::to_string(&delivery).unwrap(),21).is_err());
        let task = store.runtime_task("task").unwrap().unwrap();
        assert_eq!(store.acknowledge_runtime_task_delivery(&delivery).unwrap(), Ack::Delivered);
        assert!(store.runtime_task_notification_delivered(&delivery).unwrap());
        assert_eq!(store.acknowledge_runtime_task_delivery(&delivery).unwrap(), Ack::AlreadyDelivered);
        let after = store.runtime_task("task").unwrap().unwrap();
        assert_eq!(after.status, task.status);
        assert_eq!(after.updated_at_ms, task.updated_at_ms);
        assert_eq!(after.unread, task.unread);
        let mut changed = delivery.clone();
        changed.command_id="new-command".into();
        assert!(store.acknowledge_runtime_task_delivery(&changed).is_err());
        let mut other = delivery.clone();
        other.consumer_id="mac".into();
        assert_eq!(store.acknowledge_runtime_task_delivery(&other).unwrap(), Ack::Delivered);
        drop(store);
        let mut store = NolaStore::open(&path).unwrap();
        assert!(store.runtime_task_notification_delivered(&delivery).unwrap());
        assert_eq!(store.acknowledge_runtime_task_delivery(&delivery).unwrap(), Ack::AlreadyDelivered);
        // Isolate the ACK gate from the independently tested answer reducer.
        // A new consumer cannot ACK a resolved or withdrawn occurrence, but
        // that proven display race must not terminate the client observation.
        for state in ["resolved", "withdrawn"] {
            store.connection.execute("UPDATE user_input_wait_heads SET state=?1 WHERE wait_id='wait'", [state]).unwrap();
            let mut late = delivery.clone();
            late.consumer_id=format!("late-{state}");
            assert_eq!(store.acknowledge_runtime_task_delivery(&late).unwrap(), Ack::Superseded);
            assert!(!store.runtime_task_notification_delivered(&late).unwrap());
            assert_eq!(store.acknowledge_runtime_task_delivery(&delivery).unwrap(), Ack::AlreadyDelivered);
        }
        // Even negative presentation outcomes need a formal indexed source,
        // not a same-kind event body with a plausible question identifier.
        let (source_id, original): (String, String) = store.connection.query_row(
            "SELECT h.source_event_id,e.payload_json FROM user_input_wait_heads h
             JOIN run_events e ON e.event_id=h.source_event_id WHERE h.wait_id='wait'",
            [], |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
        for fault in ["event_type", "core_session", "core_turn", "future_cursor", "zero_cursor"] {
            let mut event:serde_json::Value=serde_json::from_str(&original).unwrap();
            match fault {
                "event_type" => event["type"]=json!("unrelated"),
                "core_session" => event["core_envelope"]["session_id"]=json!("other-core"),
                "core_turn" => event["core_envelope"]["turn_id"]=json!("other-turn"),
                "future_cursor" => event["core_envelope"]["cursor"]["sequence"]=json!(999),
                "zero_cursor" => event["core_envelope"]["cursor"]["sequence"]=json!(0),
                _ => unreachable!(),
            }
            store.connection.execute("UPDATE run_events SET payload_json=?1 WHERE event_id=?2",
                params![event.to_string(),source_id]).unwrap();
            let mut malformed=delivery.clone();
            malformed.consumer_id=format!("malformed-{fault}");
            assert!(store.acknowledge_runtime_task_delivery(&malformed).is_err(),"{fault}");
            assert!(!store.runtime_task_notification_delivered(&malformed).unwrap());
        }
        store.connection.execute("UPDATE run_events SET payload_json=?1 WHERE event_id=?2",
            params![original,source_id]).unwrap();
        store.connection.execute("DELETE FROM user_input_wait_heads WHERE wait_id='wait'", []).unwrap();
        let mut unindexed=delivery.clone(); unindexed.consumer_id="unindexed".into();
        assert!(store.acknowledge_runtime_task_delivery(&unindexed).is_err());
        let count:i64=store.connection.query_row("SELECT count(*) FROM durable_task_events WHERE kind='notification_delivered'",[],|r|r.get(0)).unwrap();
        assert_eq!(count,2);
    }

    #[test]
    fn committed_task_question_retires_attempt_without_settling_effects() {
        let mut store=fixture();
        let original:String=store.connection.query_row("SELECT id FROM runtime_task_attempts WHERE task_id='task' AND state='leased'",[],|r|r.get(0)).unwrap();
        assert!(!store.runtime_task_attempt_retired_for_user_input("task",&original).unwrap());
        assert!(store.append_runtime_task_event("task","user_input_wait_opened","waiting_input",None,"{}",10).is_err());
        let before: i64=store.connection.query_row("SELECT count(*) FROM effect_outbox WHERE run_id='run' AND status<>'acknowledged'",[],|r|r.get(0)).unwrap();
        store.begin_write_unit_of_work().unwrap();
        synchronize(&store.connection,"run",20).unwrap();
        store.commit_write_unit_of_work().unwrap();
        let task=store.runtime_task("task").unwrap().unwrap();
        assert_eq!(task.status,"waiting_input");
        assert!(task.ownership_token.is_none());
        assert!(task.lease_until_ms.is_none());
        let state:String=store.connection.query_row("SELECT state FROM runtime_task_attempts WHERE task_id='task'",[],|r|r.get(0)).unwrap();
        assert_eq!(state,"superseded");
        assert!(store.runtime_task_attempt_retired_for_user_input("task",&original).unwrap());
        assert!(!store.runtime_task_attempt_retired_for_user_input("wrong-task",&original).unwrap());
        let after:i64=store.connection.query_row("SELECT count(*) FROM effect_outbox WHERE run_id='run' AND status<>'acknowledged'",[],|r|r.get(0)).unwrap();
        assert_eq!(before,after);
        assert!(store.claim_runtime_task_fair("task","worker2","generation2","lease2",30,2000,1,1).unwrap().is_none());
        // Durable answer projection is the sole readiness prerequisite; this
        // unit fixture mutates the head only to isolate scheduling semantics.
        store.begin_write_unit_of_work().unwrap();
        store.connection.execute("UPDATE user_input_wait_heads SET state='resolved' WHERE wait_id='wait'",[]).unwrap();
        synchronize(&store.connection,"run",40).unwrap();
        store.commit_write_unit_of_work().unwrap();
        assert_eq!(store.runtime_task("task").unwrap().unwrap().status,"queued");
        let claimed=store.claim_runtime_task_fair("task","worker2","generation2","lease2",50,2000,1,1).unwrap().unwrap();
        assert_eq!(claimed.lease_token,"lease2");
        assert!(store.runtime_task_attempt_retired_for_user_input("task",&original).unwrap());
        assert!(!store.runtime_task_attempt_retired_for_user_input("task",&claimed.id).unwrap());
    }

    #[test]
    fn task_question_suspension_rolls_back_with_transition_and_rejects_expired_attempt() {
        let mut store=fixture();
        store.begin_write_unit_of_work().unwrap();
        synchronize(&store.connection,"run",20).unwrap();
        store.rollback_write_unit_of_work().unwrap();
        assert_eq!(store.runtime_task("task").unwrap().unwrap().status,"running");
        store.begin_write_unit_of_work().unwrap();
        assert!(synchronize(&store.connection,"run",1001).is_err());
        store.rollback_write_unit_of_work().unwrap();
        assert_eq!(store.runtime_task("task").unwrap().unwrap().status,"running");
        assert!(synchronize(&store.connection,"run",20).is_err());
    }

    #[test]
    fn waiting_task_cancel_intent_cannot_fabricate_completion_or_answer_resume() {
        let mut store=fixture();
        store.begin_write_unit_of_work().unwrap();
        synchronize(&store.connection,"run",20).unwrap();
        store.commit_write_unit_of_work().unwrap();
        assert!(store.request_runtime_task_cancel("task",30).unwrap());
        let task=store.runtime_task("task").unwrap().unwrap();
        assert!(task.cancel_requested);
        assert_eq!(task.status,"waiting_input");
        assert!(task.ended_at_ms.is_none());
        assert!(store.runtime_task_terminal_receipt("task").unwrap().is_none());
        store.begin_write_unit_of_work().unwrap();
        store.connection.execute("UPDATE user_input_wait_heads SET state='withdrawn' WHERE wait_id='wait'",[]).unwrap();
        synchronize(&store.connection,"run",40).unwrap();
        store.commit_write_unit_of_work().unwrap();
        assert!(store.claim_runtime_task_fair("task","worker2","generation2","lease2",50,2000,1,1).unwrap().is_none());
        assert_eq!(store.runtime_task("task").unwrap().unwrap().status,"waiting_input");
    }
}
