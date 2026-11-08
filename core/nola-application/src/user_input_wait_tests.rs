// SPDX-License-Identifier: Apache-2.0
//! Real Core reductions and isolated SQLite, without a live provider or user data.
#[path = "user_input_wait_process_tests.rs"]
mod process_recovery;
use super::*;
use nola_agent_engine::agent::host::*;
use nola_agent_engine::agent::{AgentInputDelivery, AgentItem, ItemId, SessionId};
use nola_domain::user_input_wait::*;
use nola_store_port::user_input_wait::{StoredUserInputWait, UserInputWaitRepository};

pub(super) fn config() -> AgentCoreConfig {
    let model = HostCapabilityDescriptor {
        id: "model.stream".into(),
        title: "fixture".into(),
        route: HostCapabilityRoute::Model,
        availability: CapabilityAvailability::Available,
        exposure: HostCapabilityExposure::Internal,
        permission_domain: None,
        input_schema: serde_json::json!({"type":"object"}),
        input_normalization: Default::default(),
        output_policy: HostOutputPolicy {
            preview_char_limit: 4000,
            persist_full_output: true,
            allow_artifacts: true,
        },
        supports_streaming: true,
        supports_argument_streaming: false,
        parallelism: HostCapabilityParallelism::Serial,
        cancellation: HostCancellationMode::Cooperative,
        replay_safety: HostReplaySafety::Recoverable,
    };
    let mut question = model.clone();
    question.id = "request_user_input".into();
    question.route = HostCapabilityRoute::Tool;
    question.exposure = HostCapabilityExposure::Visible;
    question.input_normalization.permission_contract.risk = nola_domain::permission_resource_plan::PermissionOperationRisk::OrdinaryRead;
    let mut read = question.clone();
    read.id = "read_file".into();
    AgentCoreConfig {
        turn_identity: crate::model_fixture::identity(),
        session_id: Some(SessionId::from_string("conversation".into())),
        model: Some("fixture".into()),
        tools: vec![
            serde_json::json!({"type":"function","function":{"name":"read_file","parameters":{"type":"object"}}}),
            serde_json::json!({"type":"function","function":{"name":"request_user_input","parameters":{"type":"object"}}}),
        ],
        host_catalog: Some(HostCapabilityCatalog {
            host_id: HostId("host".into()),
            revision: 1,
            availability_epoch: 1,
            lease: None,
            capabilities: [
                (model.id.clone(), model),
                (question.id.clone(), question),
                (read.id.clone(), read),
            ]
            .into_iter()
            .collect(),
        }),
        ..Default::default()
    }
}

fn complete_model(
    store: &mut nola_store::NolaStore,
    run: &str,
    digest: &str,
    asking: bool,
    now: i64,
) {
    complete_model_with_read(store, run, digest, asking, now, false)
}

fn complete_model_with_read(
    store: &mut nola_store::NolaStore,
    run: &str,
    digest: &str,
    asking: bool,
    now: i64,
    independent_read: bool,
) {
    let requests = claim_committed_host_requests(
        store,
        ClaimCommittedHostRequests {
            run_id: run.into(),
            worker_generation: "fixture".into(),
            lease_owner: "fixture".into(),
            claimed_at_ms: now,
            lease_expires_at_ms: now + 9000,
            limit: 8,
        },
    )
    .unwrap()
    .requests;
    assert_eq!(
        requests.len(),
        1,
        "exactly one model continuation, never a question host effect"
    );
    let request = &requests[0];
    let mut tool_calls = if asking {
        vec![
            serde_json::json!({"call_id":"question-call","name":"request_user_input",
        "arguments":{"questions":[{"id":"scope","prompt":"Which scope?","options":[{"id":"small","label":"Small","description":null}],"allow_freeform":true,"required":true,"max_text_chars":128}]},
        "permission_scope":null,"permission_rationale":null}),
        ]
    } else {
        vec![]
    };
    if independent_read {
        tool_calls.push(serde_json::json!({"call_id":"independent-read","name":"read_file","arguments":{"path":"fixture.txt"}}));
    }
    let items = if asking {
        vec![]
    } else {
        vec![serde_json::to_value(AgentItem::assistant_message(
            ItemId::from_string("answer".into()),
            "Done",
        ))
        .unwrap()]
    };
    for (sequence, payload) in [
        (1, HostResponse::ExecutionStarted),
        (
            2,
            HostResponse::ModelAttemptStarted {
                provider: "fixture".into(),
                model: "fixture".into(),
                reason: "initial".into(),
            },
        ),
        (
            3,
            HostResponse::ModelCompleted {
                items,
                tool_calls,
                usage: serde_json::json!({"input_tokens":10,"output_tokens":2}),
            },
        ),
    ] {
        reduce_and_accept_host_response(
            store,
            ReduceAndAcceptHostResponse {
                conversation_id: "conversation".into(),
                run_id: run.into(),
                invocation_config_digest: digest.into(),
                received_at_ms: now + sequence as i64,
                response: HostResponseEnvelope {
                    request_id: request.request_id.clone(),
                    attempt_id: request.attempt_id.clone(),
                    effect_id: request.effect_id.clone(),
                    delegated_invocation_id: request.delegated_invocation_id.clone(),
                    host_id: request.host_id.clone(),
                    lease_id: request.lease_id.clone(),
                    session_id: request.session_id.clone(),
                    turn_id: request.turn_id.clone(),
                    catalog_revision: request.catalog_revision,
                    availability_epoch: request.availability_epoch,
                    delivery: request.delivery.clone(),
                    sequence,
                    payload,
                },
            },
        )
        .unwrap();
    }
}

fn fixture(path: &std::path::Path) -> (String, StoredUserInputWait) {
    fixture_with_task(path, false)
}

fn fixture_with_task(path: &std::path::Path, task_child: bool) -> (String, StoredUserInputWait) {
    fixture_with_read(path, task_child, false)
}

pub(super) fn fixture_with_read(
    path: &std::path::Path,
    task_child: bool,
    independent_read: bool,
) -> (String, StoredUserInputWait) {
    let mut store = nola_store::NolaStore::open(path).unwrap();
    let lease = if task_child {
        Some(prepare_task(&mut store))
    } else {
        None
    };
    let input = BeginAssistantTurn {
        expected_configuration_revision: None,
        expected_parent_message_id: None,
        conversation_id: "conversation".into(),
        conversation_title: "Questions".into(),
        user_message_id: "origin".into(),
        user_part_id: "body".into(),
        user_text: "Ask for the scope".into(),
        user_content_json: None,
        assistant_message_id: "assistant".into(),
        context_checkpoint_id: None,
        core_config: config(),
        created_at_ms: 1000,
        submission: AgentSubmission::UserInput {
            items: vec![AgentItem::user_message(
                ItemId::from_string("origin".into()),
                "Ask for the scope",
            )],
            turn_context_items: vec![],
            turn_context_platform: Default::default(),
            turn_context_components: vec![],
            submission_id: Some("origin".into()),
            submitted_at_epoch_ms: Some(1000),
            delivery: AgentInputDelivery::CurrentTurn,
        },
    };
    let accepted = crate::assistant_turn::admit_with_core(&mut store, input, lease)
        .unwrap()
        .0;
    complete_model_with_read(
        &mut store,
        &accepted.begin.run_id,
        &accepted.invocation_config_digest,
        true,
        1001,
        independent_read,
    );
    let waits = store
        .open_user_input_waits("conversation", Some(&accepted.begin.run_id))
        .unwrap();
    assert_eq!(waits.len(), 1);
    assert_eq!(
        load_committed_host_requests(&store, &accepted.begin.run_id)
            .unwrap()
            .requests
            .len(),
        usize::from(independent_read)
    );
    assert!(
        store.active_run("conversation").unwrap().is_some(),
        "waiting is not terminal"
    );
    (accepted.invocation_config_digest, waits[0].clone())
}

fn prepare_task(store: &mut nola_store::NolaStore) -> crate::RuntimeTaskLeaseWitness {
    prepare_task_from_source(store, "epoch")
}

fn prepare_task_from_source(store: &mut nola_store::NolaStore, source_epoch: &str) -> crate::RuntimeTaskLeaseWitness {
    let task = serde_json::from_value(serde_json::json!({"id":"task","kind":"agent","status":"queued","title":"Task",
        "project_root":"/fixture","owner_session_id":"parent","child_session_id":"conversation","parent_tool_call_id":"call",
        "unread":false,"cancel_requested":false,"retry_count":0,"created_at_ms":1,"updated_at_ms":1,"metadata_json":"{}"})).unwrap();
    store.create_runtime_task_with_event(&task).unwrap();
    // This test starts at the trusted receiver-import boundary. It does not
    // claim to test parent delegation transport/authentication. No SQL bypass,
    // fabricated checkpoint, or manually inserted Run/attempt/binding is used.
    let mut admission: nola_domain::agent_task_admission::AgentTaskAdmission = serde_json::from_value(serde_json::json!({
        "protocol_version":nola_domain::agent_task_admission::AGENT_TASK_ADMISSION_VERSION,"task_id":"task","parent_conversation_id":"parent","parent_core_session_id":"parent-core",
        "parent_run_id":"parent-run","parent_turn_id":"parent-turn","child_conversation_id":"conversation",
        "parent_tool_call_id":"call","parent_effect_id":"effect","input_watermark":null,
        "source_store_epoch":source_epoch,"source_transition_id":"transition","source_host_event_id":"host-event",
        "source_checkpoint_revision":1,"core_revision":1,"core_event_cursor":1,"admission_digest":""
    })).unwrap();
    admission.admission_digest = admission.canonical_digest().unwrap();
    store.begin_write_unit_of_work().unwrap();
    store.admit_runtime_agent_task(&admission).unwrap();
    store.commit_write_unit_of_work().unwrap();
    let attempt = store
        .claim_runtime_task_fair("task", "worker", "generation", "lease", 900, 9000, 1, 1)
        .unwrap()
        .unwrap();
    (&attempt).into()
}

#[test]
fn startup_recovery_preserves_committed_question_without_dispatchable_effect() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("startup-wait.sqlite");
    let (_, expected) = fixture(&path);
    let mut store = nola_store::NolaStore::open(&path).unwrap();
    assert!(load_committed_host_requests(&store, &expected.run_id).unwrap().requests.is_empty());
    crate::run_recovery::recover(&mut store, Some("conversation"), "restart", "2000", 2000)
        .expect("a persisted user question is legitimate recoverable work");
    let actual = store.open_user_input_waits("conversation", Some(&expected.run_id)).unwrap();
    assert_eq!(actual.len(), 1);
    assert_eq!(actual[0].wait, expected.wait);
    assert!(load_committed_host_requests(&store, &expected.run_id).unwrap().requests.is_empty());
    drop(store);
    let store = nola_store::NolaStore::open(&path).unwrap();
    assert_eq!(store.open_user_input_waits("conversation", Some(&expected.run_id)).unwrap()[0].wait, expected.wait);
}

#[test]
fn startup_recovery_rejects_mismatched_question_facts_without_changing_store() {
    use nola_store_port::store_maintenance::{RunRecoveryPlan, RunRecoveryRecord, StoreMaintenanceRepository};
    struct CorruptRead<'a> { store: &'a mut nola_store::NolaStore, case: usize }
    impl StoreMaintenanceRepository for CorruptRead<'_> {
        type Error = String;
        fn with_run_recovery_plan<F>(&mut self, conversation: Option<&str>, reason: &str,
            at: &str, at_ms: i64, mut planner: F) -> Result<usize, String>
        where F: FnMut(&RunRecoveryRecord) -> Result<RunRecoveryPlan, String> {
            let case = self.case;
            self.store.with_run_recovery_plan(conversation, reason, at, at_ms, |record| {
                let mut corrupted = record.clone();
                assert_eq!(corrupted.open_user_input_waits.len(), 1);
                match case {
                    0 => corrupted.open_user_input_waits.clear(),
                    1 => corrupted.open_user_input_waits[0].conversation_id = "foreign".into(),
                    2 => corrupted.open_user_input_waits[0].run_id = "foreign".into(),
                    3 => corrupted.open_user_input_waits[0].wait.core_session_id = "foreign".into(),
                    4 => corrupted.open_user_input_waits[0].wait.turn_id = "foreign".into(),
                    5 => {
                        let wait = &mut corrupted.open_user_input_waits[0].wait;
                        wait.question_set.questions[0].prompt = "substituted question".into();
                        wait.response_schema_digest = wait.question_set.digest().unwrap();
                    }
                    _ => corrupted.open_user_input_waits.push(corrupted.open_user_input_waits[0].clone()),
                }
                planner(&corrupted)
            }).map_err(|error| error.to_string())
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mismatched-wait.sqlite");
    let (_, expected) = fixture(&path);
    let mut store = nola_store::NolaStore::open(&path).unwrap();
    for case in 0..7 {
        let error = crate::run_recovery::recover(&mut CorruptRead { store: &mut store, case },
            Some("conversation"), "restart", "2000", 2000).err().expect("corrupt question must reject recovery");
        assert!(error.contains("user wait"), "case {case}: {error}");
        let actual = store.open_user_input_waits("conversation", Some(&expected.run_id)).unwrap();
        assert_eq!(actual.len(), 1);
        assert_eq!(actual[0].wait, expected.wait);
        assert!(load_committed_host_requests(&store, &expected.run_id).unwrap().requests.is_empty());
    }
}

#[test]
fn task_open_question_does_not_retire_attempt_until_independent_tool_settles() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mixed-wait.sqlite");
    let (digest, wait) = fixture_with_read(&path, true, true);
    let mut store = nola_store::NolaStore::open(&path).unwrap();
    let running = store.runtime_task("task").unwrap().unwrap();
    assert_eq!(running.status, "running");
    assert_eq!(running.ownership_token.as_deref(), Some("lease"));
    let request = claim_committed_host_requests(
        &mut store,
        ClaimCommittedHostRequests {
            run_id: wait.run_id.clone(),
            worker_generation: "tool-worker".into(),
            lease_owner: "tool-worker".into(),
            claimed_at_ms: 1100,
            lease_expires_at_ms: 8000,
            limit: 8,
        },
    )
    .unwrap()
    .requests
    .remove(0);
    for (sequence, payload) in [
        (1, HostResponse::ExecutionStarted),
        (
            2,
            HostResponse::ToolCompleted {
                call_id: "independent-read".into(),
                output: serde_json::json!({"content":"read completed"}),
                outcome: HostToolOutcome::Succeeded,
                activation: None,
                full_output_ref: None,
                artifacts: vec![],
            },
        ),
    ] {
        reduce_and_accept_host_response(
            &mut store,
            ReduceAndAcceptHostResponse {
                conversation_id: "conversation".into(),
                run_id: wait.run_id.clone(),
                invocation_config_digest: digest.clone(),
                received_at_ms: 1100 + sequence as i64,
                response: HostResponseEnvelope {
                    request_id: request.request_id.clone(),
                    attempt_id: request.attempt_id.clone(),
                    effect_id: request.effect_id.clone(),
                    delegated_invocation_id: request.delegated_invocation_id.clone(),
                    host_id: request.host_id.clone(),
                    lease_id: request.lease_id.clone(),
                    session_id: request.session_id.clone(),
                    turn_id: request.turn_id.clone(),
                    catalog_revision: request.catalog_revision,
                    availability_epoch: request.availability_epoch,
                    delivery: request.delivery.clone(),
                    sequence,
                    payload,
                },
            },
        )
        .unwrap();
    }
    let waiting = store.runtime_task("task").unwrap().unwrap();
    assert_eq!(waiting.status, "waiting_input");
    assert!(waiting.ownership_token.is_none());
    assert_eq!(
        store
            .open_user_input_waits("conversation", None)
            .unwrap()
            .len(),
        1
    );
    assert!(load_committed_host_requests(&store, &wait.run_id)
        .unwrap()
        .requests
        .is_empty());
}

#[test]
fn waiting_input_history_projection_survives_cold_open_without_resolving_wait() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("waiting-history.sqlite");
    let (_, wait) = fixture(&path);
    let mut runtime = ApplicationRuntime::open(&path).unwrap();
    let projection: serde_json::Value = runtime.execute(ApplicationCommand::LoadConversationProjectionV1 {
        conversation_id: "conversation".into(),
    }).expect("a current waiting_input message must be readable by every client");
    let assistant = projection["messages"].as_array().unwrap().iter()
        .find(|message| message["role"] == "assistant").unwrap();
    assert_eq!(assistant["state"], "running");
    let open = runtime.store.open_user_input_waits("conversation", None).unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(serde_json::to_value(&open[0]).unwrap(), serde_json::to_value(&wait).unwrap(),
        "reading history must not answer or rewrite the wait");
}

#[test]
fn task_child_answer_requeues_exact_task_without_dispatch_and_returns_scheduler_policy() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("task-wait.sqlite");
    let (_, wait) = fixture_with_task(&path, true);
    let mut runtime = ApplicationRuntime::open(&path).unwrap();
    let before = runtime.store.runtime_task("task").unwrap().unwrap();
    assert_eq!(before.status, "waiting_input");
    assert!(before.ownership_token.is_none());
    let binding = runtime
        .store
        .runtime_task_run_binding("task")
        .unwrap()
        .unwrap();
    assert_eq!(binding.run_id, wait.run_id);
    let input = answer(&wait);
    let applied = runtime
        .execute(ApplicationCommand::RespondToConversationWait {
            input: input.clone(),
        })
        .unwrap();
    assert_eq!(applied["type"], "applied");
    assert_eq!(applied["resume_policy"], "task_scheduler");
    let queued = runtime.store.runtime_task("task").unwrap().unwrap();
    assert_eq!(queued.status, "queued");
    assert!(queued.ownership_token.is_none());
    let replay = runtime
        .execute(ApplicationCommand::RespondToConversationWait { input })
        .unwrap();
    assert_eq!(replay["type"], "replay");
    assert_eq!(replay["resume_policy"], "task_scheduler");
    assert_eq!(
        load_committed_host_requests(&runtime.store, &wait.run_id)
            .unwrap()
            .requests
            .len(),
        1
    );
    // The answer writes one ready model effect, not an execution attempt.
    let db =
        rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let attempts: (i64, i64) = db
        .query_row(
            "SELECT count(*),sum(state='leased') FROM runtime_task_attempts WHERE task_id='task'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(attempts, (1, 0));
    let effect: (String, Option<i64>) = db.query_row(
        "SELECT status,acknowledged_at_ms FROM effect_outbox WHERE run_id=?1 AND status<>'acknowledged'",
        [&wait.run_id], |r| Ok((r.get(0)?, r.get(1)?)),
    ).unwrap();
    assert_eq!(effect, ("pending".into(), None));
    let responses: i64 = db
        .query_row(
            "SELECT count(*) FROM user_input_wait_responses WHERE wait_id=?1",
            [&wait.wait.wait_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(responses, 1);
    let fresh = runtime
        .store
        .claim_runtime_task_fair("task", "worker2", "generation2", "lease2", 2100, 9000, 1, 1)
        .unwrap()
        .unwrap();
    assert_ne!(fresh.id, binding.admission_attempt_id);
    assert_eq!(
        runtime
            .store
            .runtime_task_run_binding("task")
            .unwrap()
            .unwrap(),
        binding
    );
}

fn owned_questions(
    runtime: &mut ApplicationRuntime<nola_store::NolaStore>,
    owner: &str,
    task: Option<&str>,
) -> Value {
    let command = ApplicationCommand::ListOwnedTaskUserInputWaits {
        owner_conversation_id: owner.into(),
        task_id: task.map(str::to_owned),
    };
    assert!(command.is_read_only_query());
    runtime.execute(command).unwrap()
}

#[test]
fn parent_wait_query_preserves_child_identity_and_excludes_wrong_owner_and_resolved() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("owned-wait.sqlite");
    let (_, wait) = fixture_with_task(&path, true);
    let mut runtime = ApplicationRuntime::open(&path).unwrap();
    let all = owned_questions(&mut runtime, "parent", None);
    assert_eq!(all.as_array().unwrap().len(), 1);
    assert_eq!(all[0]["task_id"], "task");
    assert_eq!(all[0]["conversation_id"], wait.conversation_id);
    assert_eq!(all[0]["run_id"], wait.run_id);
    assert_eq!(all[0]["wait"], serde_json::to_value(&wait.wait).unwrap());
    assert_eq!(owned_questions(&mut runtime, "parent", Some("task")), all);
    assert_eq!(
        owned_questions(&mut runtime, "other", Some("task")),
        serde_json::json!([])
    );
    assert_eq!(
        owned_questions(&mut runtime, "conversation", None),
        serde_json::json!([])
    );
    assert_eq!(
        owned_questions(&mut runtime, "parent", Some("missing")),
        serde_json::json!([])
    );
    assert!(runtime
        .execute(ApplicationCommand::ListOwnedTaskUserInputWaits {
            owner_conversation_id: " ".into(),
            task_id: None
        })
        .is_err());
    runtime
        .execute(ApplicationCommand::RespondToConversationWait {
            input: answer(&wait),
        })
        .unwrap();
    assert_eq!(
        owned_questions(&mut runtime, "parent", None),
        serde_json::json!([])
    );
    drop(runtime);
    let mut runtime = ApplicationRuntime::open(&path).unwrap();
    assert_eq!(
        owned_questions(&mut runtime, "parent", None),
        serde_json::json!([])
    );
}

#[test]
fn parent_wait_query_requires_binding_and_excludes_cancellation_intent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("unbound-wait.sqlite");
    let (_, wait) = fixture(&path);
    let mut runtime = ApplicationRuntime::open(&path).unwrap();
    // Real prepared/admitted Task and claim alone cannot attach an unrelated
    // existing Run. Only BeginTaskAssistantTurn's immutable binding authorizes it.
    let _ = prepare_task(&mut runtime.store);
    assert!(runtime
        .store
        .load_user_input_wait("conversation", &wait.wait.wait_id)
        .unwrap()
        .is_some());
    assert_eq!(
        owned_questions(&mut runtime, "parent", None),
        serde_json::json!([])
    );
    drop(runtime);
    let path = dir.path().join("cancelled-wait.sqlite");
    let (digest, wait) = fixture_with_task(&path, true);
    let mut runtime = ApplicationRuntime::open(&path).unwrap();
    assert_eq!(
        owned_questions(&mut runtime, "parent", None)
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(runtime
        .store
        .request_runtime_task_cancel("task", 2000)
        .unwrap());
    assert_eq!(
        owned_questions(&mut runtime, "parent", None),
        serde_json::json!([])
    );
    // Filtering is a read projection: it does not counterfeit a withdrawal or
    // remove the child's durable question before Core cancellation settles.
    assert!(runtime
        .store
        .load_user_input_wait("conversation", &wait.wait.wait_id)
        .unwrap()
        .unwrap()
        .wait
        .is_open());
    reduce_and_commit_active_submission(
        &mut runtime.store,
        ReduceAndCommitActiveSubmission {
            conversation_id: "conversation".into(),
            run_id: wait.run_id.clone(),
            transition_id: "settle-cancel".into(),
            invocation_config_digest: digest,
            submission: AgentSubmission::Interrupt {
                goal_control: None, cancellation_id: "cancel-waiting-task".into(),
                reason: Some("user cancelled task".into()),
            },
            committed_at_ms: 2001,
        },
    )
    .unwrap();
    assert!(runtime
        .store
        .open_user_input_waits("conversation", None)
        .unwrap()
        .is_empty());
    assert!(runtime.store.active_run("conversation").unwrap().is_none());
    assert_eq!(
        owned_questions(&mut runtime, "parent", None),
        serde_json::json!([])
    );
}

fn answer(wait: &StoredUserInputWait) -> RespondToConversationWait {
    RespondToConversationWait {
        conversation_id: wait.conversation_id.clone(),
        run_id: wait.run_id.clone(),
        committed_at_ms: 2000,
        response: RespondToUserInputWait {
            command_id: "answer-command".into(),
            wait_id: wait.wait.wait_id.clone(),
            expected_revision: wait.wait.revision,
            response_schema_digest: wait.wait.response_schema_digest.clone(),
            response: UserInputWaitResponse::Answers {
                answers: vec![QuestionAnswer {
                    question_id: "scope".into(),
                    value: QuestionAnswerValue::Option {
                        option_id: "small".into(),
                    },
                }],
            },
        },
    }
}

#[test]
fn task_cancel_intent_and_answer_order_is_shared_by_runtime_and_stateless_dispatch() {
    for stateful in [false, true] {
        for cancel_first in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("cancel-answer.sqlite");
            let (_, wait) = fixture_with_task(&path, true);
            let mut runtime = ApplicationRuntime::open(&path).unwrap();
            let input = answer(&wait);
            let submit = |runtime: &mut ApplicationRuntime<nola_store::NolaStore>,
                          input: RespondToConversationWait| {
                let command = ApplicationCommand::RespondToConversationWait { input };
                if stateful {
                    runtime.execute(command).unwrap()
                } else {
                    crate::execute_application_command(&mut runtime.store, command).unwrap()
                }
            };
            let facts = || {
                let db = rusqlite::Connection::open_with_flags(
                    &path,
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
                )
                .unwrap();
                db.query_row("SELECT (SELECT count(*) FROM run_events),(SELECT count(*) FROM user_input_wait_responses),
                    (SELECT count(*) FROM effect_outbox),(SELECT revision FROM user_input_wait_heads LIMIT 1)",[],
                    |r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?))).unwrap()
            };
            if cancel_first {
                assert!(runtime
                    .store
                    .request_runtime_task_cancel("task", 1900)
                    .unwrap());
                let before = facts();
                let rejected = submit(&mut runtime, input);
                assert_eq!(rejected["type"], "conflict");
                assert_eq!(rejected["reason"], "cancellation_requested");
                assert_eq!(rejected["wait"]["wait"]["state"]["type"], "open");
                assert_eq!(
                    facts(),
                    before,
                    "rejection cannot write answers, Core events or effects"
                );
                assert_eq!(
                    runtime.store.runtime_task("task").unwrap().unwrap().status,
                    "waiting_input"
                );
            } else {
                assert_eq!(submit(&mut runtime, input.clone())["type"], "applied");
                assert!(runtime
                    .store
                    .request_runtime_task_cancel("task", 2100)
                    .unwrap());
                let before = facts();
                let replay = submit(&mut runtime, input.clone());
                assert_eq!(replay["type"], "replay");
                let mut changed = input;
                changed.response.command_id = "different-command-after-cancel".into();
                assert_eq!(submit(&mut runtime, changed)["reason"], "wait_not_open");
                assert_eq!(
                    facts(),
                    before,
                    "Replay and rejection must not duplicate continuation"
                );
            }
            assert!(runtime
                .store
                .claim_runtime_task_fair(
                    "task",
                    "new-worker",
                    "new-generation",
                    "new-lease",
                    2200,
                    9000,
                    1,
                    1
                )
                .unwrap()
                .is_none());
        }
    }
}

#[test]
fn question_survives_reopen_and_answer_replays_after_terminal_without_new_effects() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wait.sqlite");
    let (digest, wait) = fixture(&path);
    let mut runtime = ApplicationRuntime::open(&path).unwrap();
    let listed = runtime
        .execute(ApplicationCommand::ListOpenUserInputWaits {
            conversation_id: "conversation".into(),
            run_id: Some(wait.run_id.clone()),
        })
        .unwrap();
    let listed: Vec<StoredUserInputWait> = serde_json::from_value(listed).unwrap();
    assert_eq!(listed[0].wait, wait.wait);
    let input = answer(&wait);
    let applied = runtime
        .execute(ApplicationCommand::RespondToConversationWait {
            input: input.clone(),
        })
        .unwrap();
    assert_eq!(applied["type"], "applied");
    assert_eq!(applied["resume_policy"], "conversation_run");
    let replay = runtime
        .execute(ApplicationCommand::RespondToConversationWait {
            input: input.clone(),
        })
        .unwrap();
    assert_eq!(replay["type"], "replay");
    assert_eq!(replay["resume_policy"], "conversation_run");
    drop(runtime);
    let mut store = nola_store::NolaStore::open(&path).unwrap();
    complete_model(&mut store, &wait.run_id, &digest, false, 3000);
    assert!(store.active_run("conversation").unwrap().is_none());
    drop(store);
    let mut runtime = ApplicationRuntime::open(&path).unwrap();
    assert_eq!(
        runtime
            .execute(ApplicationCommand::RespondToConversationWait {
                input: input.clone()
            })
            .unwrap()["type"],
        "replay"
    );
    let mut changed = input;
    changed.response.response = UserInputWaitResponse::Decline {
        reason: Some("changed".into()),
    };
    assert_eq!(
        runtime
            .execute(ApplicationCommand::RespondToConversationWait { input: changed })
            .unwrap()["type"],
        "conflict"
    );
    assert!(load_committed_host_requests(&runtime.store, &wait.run_id)
        .unwrap()
        .requests
        .is_empty());
}

#[test]
fn wrong_owner_schema_and_option_leave_open_question_and_outbox_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wait.sqlite");
    let (_, wait) = fixture(&path);
    let mut runtime = ApplicationRuntime::open(&path).unwrap();
    let mut wrong = answer(&wait);
    wrong.conversation_id = "other".into();
    assert!(runtime
        .execute(ApplicationCommand::RespondToConversationWait { input: wrong })
        .is_err());
    let mut wrong = answer(&wait);
    wrong.response.response_schema_digest = "0".repeat(64);
    assert_eq!(
        runtime
            .execute(ApplicationCommand::RespondToConversationWait { input: wrong })
            .unwrap()["type"],
        "conflict"
    );
    let mut wrong = answer(&wait);
    wrong.response.response = UserInputWaitResponse::Answers {
        answers: vec![QuestionAnswer {
            question_id: "scope".into(),
            value: QuestionAnswerValue::Option {
                option_id: "forged".into(),
            },
        }],
    };
    assert!(runtime
        .execute(ApplicationCommand::RespondToConversationWait { input: wrong })
        .is_err());
    assert_eq!(
        runtime
            .store
            .load_user_input_wait("conversation", &wait.wait.wait_id)
            .unwrap()
            .unwrap()
            .wait,
        wait.wait
    );
    assert!(load_committed_host_requests(&runtime.store, &wait.run_id)
        .unwrap()
        .requests
        .is_empty());
}

#[test]
fn cancellation_withdraws_question_and_rejects_late_answer_without_effects() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wait.sqlite");
    let (digest, wait) = fixture(&path);
    let mut store = nola_store::NolaStore::open(&path).unwrap();
    reduce_and_commit_active_submission(
        &mut store,
        ReduceAndCommitActiveSubmission {
            conversation_id: "conversation".into(),
            run_id: wait.run_id.clone(),
            transition_id: "cancel".into(),
            invocation_config_digest: digest,
            submission: AgentSubmission::Interrupt {
                goal_control: None, cancellation_id: "cancel-waiting-user".into(),
                reason: Some("user cancelled".into()),
            },
            committed_at_ms: 2000,
        },
    )
    .unwrap();
    assert!(store
        .open_user_input_waits("conversation", None)
        .unwrap()
        .is_empty());
    drop(store);
    let mut runtime = ApplicationRuntime::open(&path).unwrap();
    assert_eq!(
        runtime
            .execute(ApplicationCommand::RespondToConversationWait {
                input: answer(&wait)
            })
            .unwrap()["type"],
        "conflict"
    );
    assert!(load_committed_host_requests(&runtime.store, &wait.run_id)
        .unwrap()
        .requests
        .is_empty());
}

#[test]
fn competing_application_answers_commit_one_resolution_and_one_continuation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wait.sqlite");
    let (_, wait) = fixture(&path);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles = (0..2)
        .map(|index| {
            let path = path.clone();
            let barrier = barrier.clone();
            let mut input = answer(&wait);
            input.response.command_id = format!("competing-answer-{index}");
            std::thread::spawn(move || {
                let mut runtime = ApplicationRuntime::open(path).unwrap();
                barrier.wait();
                runtime
                    .execute(ApplicationCommand::RespondToConversationWait { input })
                    .unwrap()
            })
        })
        .collect::<Vec<_>>();
    let results = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        results
            .iter()
            .filter(|result| result["type"] == "applied")
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|result| result["type"] == "conflict")
            .count(),
        1
    );
    let store = nola_store::NolaStore::open(&path).unwrap();
    assert!(store
        .open_user_input_waits("conversation", None)
        .unwrap()
        .is_empty());
    assert_eq!(
        load_committed_host_requests(&store, &wait.run_id)
            .unwrap()
            .requests
            .len(),
        1
    );
}

#[test]
fn task_binding_publication_requires_owner_only_in_its_source_store() {
    use nola_store_port::assistant_journal::AssistantJournalRepository;
    let mut receiver = nola_store::NolaStore::open(":memory:").unwrap();
    prepare_task(&mut receiver);
    assert!(crate::assistant_journal_producer::publish_task_run_binding(
        &mut receiver, "task", "child-run",
    ).unwrap().is_empty());

    let mut source = nola_store::NolaStore::open(":memory:").unwrap();
    let epoch = source.assistant_source_epoch().unwrap();
    prepare_task_from_source(&mut source, &epoch);
    let error = crate::assistant_journal_producer::publish_task_run_binding(
        &mut source, "task", "child-run",
    ).unwrap_err();
    assert!(error.contains("no persisted owner"), "{error}");
}
