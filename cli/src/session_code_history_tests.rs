// SPDX-License-Identifier: Apache-2.0
//! Real SessionStore/Application/Store history dispatch through Runtime.
use super::*;
use nola_core::agent::host::*;
use nola_domain::code_mode::*;

fn response(request: &HostRequestEnvelope, sequence: u64, payload: HostResponse) -> HostResponseEnvelope {
    HostResponseEnvelope {
        request_id: request.request_id.clone(), effect_id: request.effect_id.clone(),
        host_id: request.host_id.clone(), session_id: request.session_id.clone(),
        turn_id: request.turn_id.clone(), attempt_id: request.attempt_id.clone(),
        delegated_invocation_id: request.delegated_invocation_id.clone(), lease_id: request.lease_id.clone(),
        catalog_revision: request.catalog_revision, availability_epoch: request.availability_epoch,
        sequence, delivery: request.delivery.clone(), payload,
    }
}

struct HistoryModelHost { sampled: usize }
impl AgentHostDriver for HistoryModelHost {
    type Protocol = AgentRuntimeProtocol;
    type Error = String;
    fn dispatch(&mut self, request: HostRequestEnvelope,
        emit: &mut dyn FnMut(HostResponseEnvelope) -> Result<(), String>,
    ) -> Result<(), String> {
        let HostRequest::ModelStream { messages, text_part_id, .. } = &request.payload else {
            panic!("history must never reach physical Host or replay JS: {:?}", request.payload);
        };
        emit(response(&request, 1, HostResponse::ExecutionStarted))?;
        emit(response(&request, 2, HostResponse::ModelAttemptStarted {
            provider: "fixture".into(), model: "fixture".into(), reason: "initial".into(),
        }))?;
        let calls = if self.sampled == 0 {
            vec![serde_json::json!({"call_id":"history-wait","name":"code_mode_wait",
                "arguments":{"cell_id":"old-cell","after_output_sequence":1,"wait_ms":0,"terminate":false}})]
        } else {
            assert_eq!(self.sampled, 1, "history resumes exactly one model request");
            assert!(messages.iter().any(|message| message["type"] == "tool_result"
                && message["call_id"] == "history-wait" && message["is_error"] == false
                && message["output"]["output"][0]["text"] == "persisted result"), "{messages:?}");
            emit(response(&request, 3, HostResponse::ModelDelta {
                channel: ModelDeltaChannel::Text, part_id: text_part_id.clone(),
                delta: "The persisted result is available.".into(),
            }))?;
            Vec::new()
        };
        self.sampled += 1;
        emit(response(&request, if self.sampled == 1 { 3 } else { 4 }, HostResponse::ModelCompleted {
            items: Vec::new(), tool_calls: calls, usage: serde_json::json!({}),
        }))
    }
}

#[test]
fn cold_history_wait_dispatches_through_session_application_without_physical_host_execution() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("history.sqlite");
    let mut session = super::foreground_cancellation_tests::source_with_config(&path, |config| {
        let mut wait = config.host_catalog.as_ref().unwrap().capabilities["model.stream"].clone();
        wait.id = "code_mode_wait".into();
        wait.route = HostCapabilityRoute::Tool;
        wait.exposure = HostCapabilityExposure::Visible;
        wait.input_schema = CodeCellWaitRequest::input_schema();
        wait.input_normalization.permission_contract.risk =
            nola_domain::permission_resource_plan::PermissionOperationRisk::OrdinaryRead;
        config.tools = vec![serde_json::json!({"type":"function","function":{
            "name":"code_mode_wait","description":"Read history","parameters":wait.input_schema}})];
        config.host_catalog.as_mut().unwrap().capabilities.insert(wait.id.clone(), wait);
    });
    // Old canonical rows have no checkpoint or executable source. Reopening
    // the Application below proves this is durable history, not a hot cache.
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("BEGIN;
        INSERT INTO runs(id,conversation_id,kind,status,next_event_sequence,created_at_ms,updated_at_ms)
            VALUES('old-run','parent','conversation','completed',1,1,1);
        INSERT INTO messages(id,conversation_id,run_id,role,status,ordinal,created_at_ms)
            VALUES('old-message','parent','old-run','assistant','completed',0,1);
        INSERT INTO message_parts(id,message_id,kind,ordinal,state,created_at_ms,updated_at_ms)
            VALUES('old-part','old-message','tool',1,'completed',1,1);
        COMMIT;").unwrap();
    let epoch = db.query_row("SELECT value FROM store_metadata WHERE key='assistant_source_epoch'", [],
        |row| row.get(0)).unwrap();
    let record = nola_store_port::code_cell::CodeCellRecord {
        store_epoch: epoch, conversation_id: "parent".into(), run_id: "old-run".into(),
        parent_part_id: "old-part".into(), core_revision: 1,
        cell: CodeCellIdentity { host_id: "old-host".into(), conversation_id: "old-session".into(),
            turn_id: "old-turn".into(), parent_call_id: "old-call".into(), cell_id: "old-cell".into(),
            generation: "old-generation".into() },
        observation: CodeCellView { cell_id: "old-cell".into(), parent_call_id: "old-call".into(),
            phase: CodeCellPhase::Completed, terminal: true, settled: true, next_output_sequence: 2,
            has_more_output: false, output: vec![CodeOutputChunk { sequence: 1, text: "first".into() },
                CodeOutputChunk { sequence: 2, text: "persisted result".into() }],
            unsettled_calls: 0, completion: CodeCellCompletionDetails::default() },
    };
    record.validate().unwrap();
    db.execute("INSERT INTO code_cell_observations(conversation_id,cell_id,run_id,parent_part_id,core_revision,record_json)
        VALUES('parent','old-cell','old-run','old-part',1,?1)", [serde_json::to_string(&record).unwrap()]).unwrap();
    drop(db);
    session.application_port = CliApplicationPort::InProcess(support::open_runtime(&path).unwrap());
    let mut runtime = nola_runtime_driver::CommittedRuntimeDriver::with_delivery_identity(
        HistoryModelHost { sampled: 0 }, "history-worker", "history-owner");
    runtime.execute(&mut session, &mut |_| {}).unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    let saved: String = db.query_row("SELECT record_json FROM code_cell_observations WHERE cell_id='old-cell'", [],
        |row| row.get(0)).unwrap();
    assert_eq!(serde_json::from_str::<serde_json::Value>(&saved).unwrap(), serde_json::to_value(record).unwrap());
}

#[path = "../../core/nola-application/tests/support/mod.rs"]
mod support;

struct LostCodeHost { starts: Arc<std::sync::atomic::AtomicUsize> }
impl AgentHostDriver for LostCodeHost {
    type Protocol = AgentRuntimeProtocol;
    type Error = String;
    fn dispatch(&mut self, request: HostRequestEnvelope,
        emit: &mut dyn FnMut(HostResponseEnvelope) -> Result<(), String>,
    ) -> Result<(), String> {
        emit(response(&request, 1, HostResponse::ExecutionStarted))?;
        match &request.payload {
            HostRequest::RequestPermission { permission_request_id, capability_id, .. } => {
                assert_eq!(capability_id, "code_mode_exec");
                emit(response(&request, 2, HostResponse::PermissionResolved {
                    request_id: permission_request_id.clone(), resolution: HostPermissionResolution::allow_once(),
                }))
            }
            HostRequest::ModelStream { .. } => {
                emit(response(&request, 2, HostResponse::ModelAttemptStarted {
                    provider: "fixture".into(), model: "fixture".into(), reason: "initial".into(),
                }))?;
                emit(response(&request, 3, HostResponse::ModelCompleted {
                    items: Vec::new(), usage: serde_json::json!({}),
                    tool_calls: vec![serde_json::json!({"call_id":"lost-exec","name":"code_mode_exec",
                        "arguments":{"source":"text('before host loss');","initial_wait_ms":0}})],
                }))
            }
            HostRequest::StartCodeCell { identity, .. } => {
                assert_eq!(self.starts.fetch_add(1, std::sync::atomic::Ordering::SeqCst), 0,
                    "unknown source must never be replayed");
                emit(response(&request, 2, HostResponse::CodeModeOutput {
                    frame: CodeOutputFrame { cell: identity.clone(),
                        chunk: CodeOutputChunk { sequence: 1, text: "before host loss".into() } },
                }))?;
                Err("fixture host disappeared after acknowledged output".into())
            }
            _ => panic!("unexpected request after Host loss: {:?}", request.payload),
        }
    }
}

#[test]
fn lost_code_host_cold_recovery_preserves_output_and_blocks_source_replay() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("lost-host.sqlite");
    let mut session = super::foreground_cancellation_tests::source_with_config(&path, |config| {
        for mut capability in nola_core::agent::code_mode::capability_descriptors(&Default::default()).unwrap() {
            capability.exposure = HostCapabilityExposure::Visible;
            config.tools.push(serde_json::json!({"type":"function","function":{
                "name":capability.id,"description":capability.title,"parameters":capability.input_schema}}));
            config.host_catalog.as_mut().unwrap().capabilities.insert(capability.id.clone(), capability);
        }
    });
    let starts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut runtime = nola_runtime_driver::CommittedRuntimeDriver::with_delivery_identity(
        LostCodeHost { starts: Arc::clone(&starts) }, "lost-worker", "lost-owner");
    let error = runtime.execute(&mut session, &mut |_| {}).unwrap_err();
    assert!(matches!(error, nola_runtime_driver::RuntimeDriverError::Host(ref message)
        if message == "fixture host disappeared after acknowledged output"));
    drop(runtime);
    session.application_port = CliApplicationPort::InProcess(support::open_runtime(&path).unwrap());
    let recovered_at = monotonic_epoch_millis() + 600_000;
    let _: Value = session.application_port.execute(ApplicationCommand::RecoverOrphanedRuns {
        conversation_id: Some("parent".into()), reason: "fixture worker disappeared".into(),
        interrupted_at: recovered_at.to_string(), interrupted_at_ms: recovered_at,
    }).unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    let saved: String = db.query_row("SELECT record_json FROM code_cell_observations WHERE conversation_id='parent'", [],
        |row| row.get(0)).unwrap();
    let record: nola_store_port::code_cell::CodeCellRecord = serde_json::from_str(&saved).unwrap();
    assert_eq!(record.observation.output, vec![CodeOutputChunk { sequence: 1, text: "before host loss".into() }]);
    assert!(!record.observation.settled, "a lost worker cannot establish settlement");
    let facts: Option<nola_protocol::UnknownEffectRecoveryInspection> = session.application_port.execute(
        ApplicationCommand::InspectUnknownEffectRecovery {
            conversation_id: "parent".into(), expected_run_id: "cancel-run".into(),
            assistant_message_id: "assistant".into(),
        }).unwrap();
    assert_eq!(facts.expect("original Run must expose reconciliation facts").targets.len(), 1);
    let requests = session.claim_committed_requests(RuntimeEffectDeliveryClaim {
        worker_generation: "replacement-worker".into(), lease_owner: "replacement-owner".into(),
        claimed_at_ms: recovered_at + 1, lease_expires_at_ms: recovered_at + 60_000, limit: 64,
    }).unwrap();
    assert!(requests.is_empty(), "recovery must not dispatch source or unknown side effects: {requests:?}");
    assert_eq!(starts.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(session.load_assistant_run_terminal_seal("cancel-run").unwrap().is_none(),
        "Host loss must not manufacture completion");
}
