#[path = "../../nola-application/tests/support/model_identity.rs"]
mod fixture_model_identity;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use nola_application::{
    begin_assistant_turn, claim_committed_host_requests, load_committed_host_requests,
    reduce_and_accept_host_response, reduce_and_commit_active_submission, BeginAssistantTurn,
    ClaimCommittedHostRequests, ReduceAndAcceptHostResponse, ReduceAndCommitActiveSubmission,
};
use nola_agent_engine::agent::host::{
    AgentBoundarySubmissionSource, AgentHostDriver, CapabilityAvailability, DelegatedAgentEventV1,
    EffectDeliveryLease, HostCancellationMode, HostCapabilityCatalog, HostCapabilityDescriptor,
    HostCapabilityExposure, HostCapabilityParallelism, HostCapabilityRoute, HostId,
    HostOutputPolicy, HostReplaySafety, HostRequest, HostRequestEnvelope, HostResponse,
    HostResponseEnvelope, ModelDeltaChannel,
};
use nola_agent_engine::agent::{
    AgentCoreConfig, AgentInputDelivery, AgentItem, AgentSubmission, ItemId, SessionId,
};
use nola_runtime_driver::{
    CommittedRuntimeAcceptance as ProtocolAcceptance, CommittedRuntimeApplicationPort, CommittedRuntimeDriver,
    CoreHostOutput as ProtocolOutput, RuntimeEffectDeliveryClaim, RuntimeEffectDeliveryRenewal,
    RuntimeEffectDeliveryTiming,
};
type CommittedRuntimeAcceptance = ProtocolAcceptance<nola_agent_engine::agent::host::AgentRuntimeProtocol>;
type CoreHostOutput = ProtocolOutput<nola_agent_engine::agent::host::AgentRuntimeProtocol>;
use nola_store::NolaStore;
use nola_store_port::assistant_journal::AssistantJournalRepository;
use serde_json::json;
use tempfile::tempdir;

fn leased_request(
    effect_id: &str,
    claim: &RuntimeEffectDeliveryClaim,
    payload: HostRequest,
) -> HostRequestEnvelope {
    HostRequestEnvelope {
        attempt_started: false,
        attempt_id: None,
        delegated_invocation_id: Some("delegated-invocation".into()),
        request_id: format!("request-{effect_id}"),
        effect_id: nola_agent_engine::agent::host::HostEffectId(effect_id.into()),
        host_id: HostId("fixture-host".into()),
        lease_id: None,
        session_id: "parked-delivery-session".into(),
        turn_id: Some("parked-delivery-turn".into()),
        catalog_revision: 1,
        availability_epoch: 1,
        created_at_ms: 1,
        deadline_ms: None,
        batch: None,
        resume_after_sequence: None,
        delivery: Some(EffectDeliveryLease {
            attempt_id: format!("attempt-{effect_id}"),
            worker_generation: claim.worker_generation.clone(),
            lease_owner: claim.lease_owner.clone(),
            lease_token: format!("token-{effect_id}"),
            lease_expires_at_ms: claim.lease_expires_at_ms,
        }),
        payload,
    }
}

fn capability(id: &str, route: HostCapabilityRoute) -> HostCapabilityDescriptor {
    HostCapabilityDescriptor {
        id: id.to_string(),
        title: id.to_string(),
        route,
        availability: CapabilityAvailability::Available,
        exposure: HostCapabilityExposure::Internal,
        permission_domain: None,
        input_schema: json!({"type": "object"}),
        input_normalization: Default::default(),
        output_policy: HostOutputPolicy {
            preview_char_limit: 4_000,
            persist_full_output: true,
            allow_artifacts: true,
        },
        supports_streaming: id == "model.stream",
        supports_argument_streaming: false,
        parallelism: HostCapabilityParallelism::Serial,
        cancellation: HostCancellationMode::Cooperative,
        replay_safety: HostReplaySafety::Recoverable,
    }
}

fn config() -> AgentCoreConfig {
    let mut capabilities = BTreeMap::new();
    capabilities.insert(
        "model.stream".to_string(),
        capability("model.stream", HostCapabilityRoute::Model),
    );
    AgentCoreConfig {
        turn_identity: fixture_model_identity::identity(),
        session_id: Some(SessionId::from_string("committed-runtime".to_string())),
        model: Some("fixture-model".to_string()),
        host_catalog: Some(HostCapabilityCatalog {
            host_id: HostId("fixture-host".to_string()),
            revision: 1,
            availability_epoch: 1,
            lease: None,
            capabilities,
        }),
        ..AgentCoreConfig::default()
    }
}

fn response(
    request: &HostRequestEnvelope,
    sequence: u64,
    payload: HostResponse,
) -> HostResponseEnvelope {
    HostResponseEnvelope {
        delegated_invocation_id: request.delegated_invocation_id.clone(),
        attempt_id: request.attempt_id.clone(),
        request_id: request.request_id.clone(),
        effect_id: request.effect_id.clone(),
        host_id: request.host_id.clone(),
        lease_id: request.lease_id.clone(),
        session_id: request.session_id.clone(),
        turn_id: request.turn_id.clone(),
        catalog_revision: request.catalog_revision,
        availability_epoch: request.availability_epoch,
        sequence,
        delivery: request.delivery.clone(),
        payload,
    }
}

#[derive(Default)]
struct StreamingHost;

impl AgentHostDriver for StreamingHost {
    type Protocol = nola_agent_engine::agent::host::AgentRuntimeProtocol;
    type Error = String;

    fn dispatch(
        &mut self,
        request: HostRequestEnvelope,
        emit: &mut dyn FnMut(HostResponseEnvelope) -> Result<(), String>,
    ) -> Result<(), Self::Error> {
        let HostRequest::ModelStream { text_part_id, .. } = &request.payload else {
            return Err("committed Runtime executed a non-model external request".to_string());
        };
        emit(response(&request, 1, HostResponse::ExecutionStarted))?;
        emit(response(
            &request,
            2,
            HostResponse::ModelAttemptStarted {
                provider: "fixture-provider".into(),
                model: "test-model".into(),
                reason: "initial".into(),
            },
        ))?;
        emit(response(
            &request,
            3,
            HostResponse::ModelDelta {
                channel: ModelDeltaChannel::Text,
                part_id: text_part_id.clone(),
                delta: "One ".to_string(),
            },
        ))?;
        emit(response(
            &request,
            4,
            HostResponse::ModelDelta {
                channel: ModelDeltaChannel::Text,
                part_id: text_part_id.clone(),
                delta: "complete answer.".to_string(),
            },
        ))?;
        emit(response(
            &request,
            5,
            HostResponse::ModelCompleted {
                items: Vec::new(),
                tool_calls: Vec::new(),
                usage: json!({"input_tokens": 10, "output_tokens": 3, "total_tokens": 13}),
            },
        ))?;
        Ok(())
    }
}

#[derive(Default)]
struct LateBoundaryState {
    polls: usize,
    submission: Option<AgentSubmission>,
    applied: usize,
    requeued: usize,
}

#[derive(Clone)]
struct LateBoundarySource {
    state: Arc<Mutex<LateBoundaryState>>,
}

impl AgentBoundarySubmissionSource<nola_agent_engine::agent::host::AgentBoundaryInput> for LateBoundarySource {
    fn take_pending(&self) -> Result<Vec<nola_agent_engine::agent::host::AgentBoundaryInput>,String> {
        let mut state = self.state.lock().unwrap();
        state.polls += 1;
        if state.polls == 4 {
            Ok(state.submission.take().into_iter().map(|submission|nola_agent_engine::agent::host::AgentBoundaryInput::Submission {submission}).collect())
        } else {
            Ok(Vec::new())
        }
    }

    fn did_apply(&self, _submission: &nola_agent_engine::agent::host::AgentBoundaryInput) -> Result<(), String> {
        self.state.lock().unwrap().applied += 1;
        Ok(())
    }

    fn requeue(&self, submission: &nola_agent_engine::agent::host::AgentBoundaryInput) -> Result<(),String> {
        let mut state = self.state.lock().unwrap();
        state.requeued += 1;
        let nola_agent_engine::agent::host::AgentBoundaryInput::Submission {submission}=submission else {return Err("unexpected notification".into());};
        state.submission = Some(submission.clone());
        Ok(())
    }
}

struct LateBoundaryStreamingHost {
    source: LateBoundarySource,
    dispatch_count: usize,
}

impl AgentHostDriver for LateBoundaryStreamingHost {
    type Protocol = nola_agent_engine::agent::host::AgentRuntimeProtocol;
    type Error = String;

    fn boundary_submission_source(&self) -> Option<Arc<dyn AgentBoundarySubmissionSource<nola_agent_engine::agent::host::AgentBoundaryInput>>> {
        Some(Arc::new(self.source.clone()))
    }

    fn dispatch(
        &mut self,
        request: HostRequestEnvelope,
        emit: &mut dyn FnMut(HostResponseEnvelope) -> Result<(), String>,
    ) -> Result<(), Self::Error> {
        let HostRequest::ModelStream { text_part_id, .. } = &request.payload else {
            return Err("committed Runtime executed a non-model external request".to_string());
        };
        self.dispatch_count += 1;
        let answer = format!("answer {}", self.dispatch_count);
        emit(response(&request, 1, HostResponse::ExecutionStarted))?;
        emit(response(
            &request,
            2,
            HostResponse::ModelAttemptStarted {
                provider: "fixture-provider".into(),
                model: "test-model".into(),
                reason: "initial".into(),
            },
        ))?;
        emit(response(
            &request,
            3,
            HostResponse::ModelDelta {
                channel: ModelDeltaChannel::Text,
                part_id: text_part_id.clone(),
                delta: answer,
            },
        ))?;
        emit(response(
            &request,
            4,
            HostResponse::ModelDelta {
                channel: ModelDeltaChannel::Text,
                part_id: text_part_id.clone(),
                delta: ".".to_string(),
            },
        ))?;
        emit(response(
            &request,
            5,
            HostResponse::ModelCompleted {
                items: Vec::new(),
                tool_calls: Vec::new(),
                usage: json!({"input_tokens": 10, "output_tokens": 3, "total_tokens": 13}),
            },
        ))?;
        Ok(())
    }
}

struct ApplicationPort<'a> {
    store: &'a mut NolaStore,
    invocation_config_digest: String,
    run_id: String,
    now_ms: i64,
}

impl CommittedRuntimeApplicationPort for ApplicationPort<'_> {
    type Protocol = nola_agent_engine::agent::host::AgentRuntimeProtocol;
    fn load_committed_requests(&mut self) -> Result<Vec<HostRequestEnvelope>, String> {
        Ok(load_committed_host_requests(self.store, &self.run_id)?.requests)
    }

    fn claim_committed_requests(
        &mut self,
        claim: RuntimeEffectDeliveryClaim,
    ) -> Result<Vec<HostRequestEnvelope>, String> {
        self.now_ms = self.now_ms.max(claim.claimed_at_ms);
        Ok(claim_committed_host_requests(
            self.store,
            ClaimCommittedHostRequests {
                run_id: self.run_id.clone(),
                worker_generation: claim.worker_generation,
                lease_owner: claim.lease_owner,
                claimed_at_ms: claim.claimed_at_ms,
                lease_expires_at_ms: claim.lease_expires_at_ms,
                limit: claim.limit,
            },
        )?
        .requests)
    }

    fn renew_effect_delivery(
        &mut self,
        renewal: RuntimeEffectDeliveryRenewal,
    ) -> Result<nola_agent_engine::agent::host::EffectDeliveryLease, String> {
        self.now_ms = self.now_ms.max(renewal.renewed_at_ms);
        let current = self
            .store
            .effect_delivery_attempt(&renewal.delivery.attempt_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "renewed delivery attempt is missing".to_string())?;
        let mutation = nola_kernel::heartbeat_effect_delivery_attempt(
            &current,
            &renewal.delivery.worker_generation,
            &renewal.delivery.lease_owner,
            &renewal.delivery.lease_token,
            renewal.lease_expires_at_ms,
            renewal.renewed_at_ms,
        )?;
        let mut committed = self
            .store
            .commit_effect_delivery_attempt_mutations(&[mutation])
            .map_err(|error| error.to_string())?;
        let record = committed
            .pop()
            .ok_or_else(|| "renewal commit returned no attempt".to_string())?;
        Ok(nola_agent_engine::agent::host::EffectDeliveryLease {
            attempt_id: record.id,
            worker_generation: record.worker_generation.unwrap(),
            lease_owner: record.lease_owner.unwrap(),
            lease_token: record.lease_token.unwrap(),
            lease_expires_at_ms: record.lease_expires_at_ms.unwrap(),
        })
    }


    fn accept_raw_host_response(
        &mut self,
        response: HostResponseEnvelope,
    ) -> Result<CommittedRuntimeAcceptance, String> {
        self.now_ms += 1;
        let accepted = reduce_and_accept_host_response(
            self.store,
            ReduceAndAcceptHostResponse {
                conversation_id: "committed-runtime".to_string(),
                run_id: self.run_id.clone(),
                invocation_config_digest: self.invocation_config_digest.clone(),
                response,
                received_at_ms: self.now_ms,
            },
        )?;
        Ok(CommittedRuntimeAcceptance {
            events: accepted.events,
            traces: accepted.traces,
            control: accepted.control,
        })
    }

    fn accept_boundary_input(
        &mut self,
        input: nola_agent_engine::agent::host::AgentBoundaryInput,
    ) -> Result<CommittedRuntimeAcceptance, String> {
        let nola_agent_engine::agent::host::AgentBoundaryInput::Submission { submission } = input else {
            return Err("fixture cannot import Task notifications".into());
        };
        self.now_ms += 1;
        let accepted = reduce_and_commit_active_submission(
            self.store,
            ReduceAndCommitActiveSubmission {
                conversation_id: "committed-runtime".to_string(),
                run_id: self.run_id.clone(),
                transition_id: format!("boundary-{}", self.now_ms),
                invocation_config_digest: self.invocation_config_digest.clone(),
                submission,
                committed_at_ms: self.now_ms,
            },
        )?;
        Ok(CommittedRuntimeAcceptance {
            events: accepted.events,
            traces: accepted.traces,
            control: Some(accepted.control),
        })
    }
}

#[test]
fn runtime_executes_only_committed_work_and_application_persists_each_stream_step() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("committed-runtime.db");
    let config = config();
    let submission = AgentSubmission::UserInput {
        items: vec![AgentItem::user_message(
            ItemId::from_string("user-1".to_string()),
            "Give a complete answer",
        )],
        turn_context_items: Vec::new(),
        turn_context_platform: Default::default(),
        turn_context_components: Vec::new(),
        submission_id: Some("submission-1".to_string()),
        submitted_at_epoch_ms: Some(1_000),
        delivery: AgentInputDelivery::CurrentTurn,
    };
    let mut store = NolaStore::open(&database).unwrap();
    let begun = begin_assistant_turn(
        &mut store,
        BeginAssistantTurn {
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            conversation_id: "committed-runtime".to_string(),
            conversation_title: "Committed runtime".to_string(),
            user_message_id: "user-1".to_string(),
            user_part_id: "user-1:body".to_string(),
            user_text: "Give a complete answer".to_string(),
            user_content_json: None,
            assistant_message_id: "submission-1:assistant".to_string(),
            context_checkpoint_id: None,
            core_config: config.clone(),
            submission,
            created_at_ms: 1_000,
        },
    )
    .unwrap();
    let invocation_config_digest = begun.invocation_config_digest.clone();
    let run_id = begun.begin.run_id;

    let mut driver = test_driver(StreamingHost);
    let mut port = ApplicationPort {
        store: &mut store,
        invocation_config_digest,
        run_id: run_id.clone(),
        now_ms: 2_000,
    };
    let mut outputs = Vec::new();
    driver
        .execute(&mut port, &mut |output| outputs.push(output))
        .unwrap();
    let control = outputs
        .iter()
        .rev()
        .find_map(|output| match output {
            CoreHostOutput::Control(control) => Some(control),
            _ => None,
        })
        .expect(
            "the committed terminal control is published, not duplicated as a pump return value",
        );
    assert!(control.turn_status.is_terminal());
    assert!(!outputs.is_empty());
    drop(port);
    drop(store);

    let store = NolaStore::open(&database).unwrap();
    let conversation = store
        .load_conversation("committed-runtime")
        .unwrap()
        .unwrap();
    assert_eq!(conversation.messages.len(), 2);
    assert_eq!(conversation.messages[1].parts.len(), 1);
    assert_eq!(
        conversation.messages[1].parts[0].content_text,
        "One complete answer."
    );
    assert_eq!(conversation.messages[1].parts[0].state, "completed");
    assert!(store.active_run("committed-runtime").unwrap().is_none());
    assert!(store.pending_effects(&run_id).unwrap().is_empty());
}

#[test]
fn input_after_the_last_delta_is_committed_before_terminal_turn_promotion() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("late-boundary.db");
    let runtime_config = config();
    let initial_submission = AgentSubmission::UserInput {
        items: vec![AgentItem::user_message(
            ItemId::from_string("user-initial".to_string()),
            "initial request",
        )],
        turn_context_items: Vec::new(),
        turn_context_platform: Default::default(),
        turn_context_components: Vec::new(),
        submission_id: Some("submission-initial".to_string()),
        submitted_at_epoch_ms: Some(1_000),
        delivery: AgentInputDelivery::CurrentTurn,
    };
    let queued_submission = AgentSubmission::UserInput {
        items: vec![AgentItem::user_message(
            ItemId::from_string("user-follow-up".to_string()),
            "queued follow up",
        )],
        turn_context_items: Vec::new(),
        turn_context_platform: Default::default(),
        turn_context_components: Vec::new(),
        submission_id: Some("submission-follow-up".to_string()),
        submitted_at_epoch_ms: Some(1_500),
        delivery: AgentInputDelivery::NextTurn,
    };
    let mut store = NolaStore::open(&database).unwrap();
    let begun = begin_assistant_turn(
        &mut store,
        BeginAssistantTurn {
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            conversation_id: "committed-runtime".to_string(),
            conversation_title: "Late boundary".to_string(),
            user_message_id: "user-initial".to_string(),
            user_part_id: "user-initial:body".to_string(),
            user_text: "initial request".to_string(),
            user_content_json: None,
            assistant_message_id: "submission-initial:assistant".to_string(),
            context_checkpoint_id: None,
            core_config: runtime_config,
            submission: initial_submission,
            created_at_ms: 1_000,
        },
    )
    .unwrap();
    let state = Arc::new(Mutex::new(LateBoundaryState {
        submission: Some(queued_submission),
        ..LateBoundaryState::default()
    }));
    let host = LateBoundaryStreamingHost {
        source: LateBoundarySource {
            state: Arc::clone(&state),
        },
        dispatch_count: 0,
    };
    let mut driver = test_driver(host);
    let initial_run_id = begun.begin.run_id.clone();
    let mut port = ApplicationPort {
        store: &mut store,
        invocation_config_digest: begun.invocation_config_digest,
        run_id: begun.begin.run_id,
        now_ms: 2_000,
    };

    let mut latest = None;
    driver
        .execute(&mut port, &mut |output| {
            if let CoreHostOutput::Control(control) = output {
                latest = Some(control);
            }
        })
        .unwrap();
    let control = latest.expect("boundary acceptance publishes its committed control");
    assert!(control.turn_status.is_terminal());
    assert_eq!(state.lock().unwrap().applied, 1);
    assert_eq!(state.lock().unwrap().requeued, 0);
    drop(port);

    assert!(store.local_assistant_run_terminal_seal("committed-runtime", &initial_run_id)
        .unwrap().is_some());
    assert!(load_committed_host_requests(&store, &initial_run_id).unwrap().requests.is_empty());
    let initial_conversation = store.load_conversation("committed-runtime").unwrap().unwrap();
    assert_eq!(initial_conversation.messages.len(), 2,
        "NextTurn must not execute inside the terminal predecessor Run");
    drop(store);
    let mut store = NolaStore::open(&database).unwrap();
    let successor = nola_application::promote_queued_assistant_turn(
        &mut store, nola_application::PromoteQueuedAssistantTurn {
            conversation_id: "committed-runtime".into(),
            created_at_ms: nola_runtime_driver::monotonic_epoch_millis(),
        },
    ).unwrap().expect("durable queued input survives database reopen");
    let successor_run_id = successor.begin.run_id.clone();
    assert_ne!(successor_run_id, initial_run_id);
    let mut port = ApplicationPort {
        store: &mut store,
        invocation_config_digest: successor.invocation_config_digest,
        run_id: successor_run_id.clone(),
        now_ms: nola_runtime_driver::monotonic_epoch_millis(),
    };
    driver.execute(&mut port, &mut |_| {}).unwrap();
    drop(port);
    assert!(store.local_assistant_run_terminal_seal("committed-runtime", &successor_run_id)
        .unwrap().is_some());

    let conversation = store
        .load_conversation("committed-runtime")
        .unwrap()
        .unwrap();
    let visible = conversation
        .messages
        .iter()
        .map(|message| {
            (
                message.role.as_str(),
                message
                    .parts
                    .iter()
                    .map(|part| part.content_text.as_str())
                    .collect::<String>(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        visible,
        vec![
            ("user", "initial request".to_string()),
            ("assistant", "answer 1.".to_string()),
            ("user", "queued follow up".to_string()),
            ("assistant", "answer 2.".to_string()),
        ]
    );
}

#[derive(Default)]
struct InterruptedPermissionHost {
    requests: Vec<HostRequestEnvelope>,
}
impl AgentHostDriver for InterruptedPermissionHost {
    type Protocol = nola_agent_engine::agent::host::AgentRuntimeProtocol;
    type Error = String;
    fn dispatch(
        &mut self,
        request: HostRequestEnvelope,
        emit: &mut dyn FnMut(HostResponseEnvelope) -> Result<(), String>,
    ) -> Result<(), String> {
        assert!(matches!(
            request.payload,
            HostRequest::RequestPermission { .. }
        ));
        assert!(
            self.requests.len() < 2,
            "one pump must not retry an interrupted ask"
        );
        self.requests.push(request.clone());
        let first_sequence = request.resume_after_sequence.unwrap_or(0) + 1;
        emit(response(
            &request,
            first_sequence,
            HostResponse::ExecutionStarted,
        ))?;
        emit(response(
            &request,
            first_sequence + 1,
            HostResponse::PermissionTransportInterrupted {
                reason: nola_agent_engine::agent::host::PermissionTransportInterruptionReason::Disconnected,
            },
        ))?;
        Ok(())
    }
}

#[derive(Default)]
struct YieldingModelHost {
    requests: Vec<HostRequestEnvelope>,
}

impl AgentHostDriver for YieldingModelHost {
    type Protocol = nola_agent_engine::agent::host::AgentRuntimeProtocol;
    type Error = String;

    fn dispatch(
        &mut self,
        request: HostRequestEnvelope,
        emit: &mut dyn FnMut(HostResponseEnvelope) -> Result<(), String>,
    ) -> Result<(), String> {
        let HostRequest::ModelStream { text_part_id, .. } = &request.payload else {
            return Err("expected a model stream".into());
        };
        self.requests.push(request.clone());
        emit(response(&request, 1, HostResponse::ExecutionStarted))?;
        emit(response(
            &request,
            2,
            HostResponse::ModelAttemptStarted {
                provider: "fixture-provider".into(),
                model: "test-model".into(),
                reason: "initial".into(),
            },
        ))?;
        emit(response(
            &request,
            3,
            HostResponse::ModelDelta {
                channel: ModelDeltaChannel::Text,
                part_id: text_part_id.clone(),
                delta: "partial".into(),
            },
        ))?;
        Ok(())
    }
}

#[test]
fn yielded_model_stream_is_not_reclaimed_by_another_runtime() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("yielded-stream.db");
    let mut store = NolaStore::open(&database).unwrap();
    let submission = AgentSubmission::UserInput {
        items: vec![AgentItem::user_message(
            ItemId::from_string("stream-user".to_string()),
            "stream",
        )],
        turn_context_items: Vec::new(),
        turn_context_platform: Default::default(),
        turn_context_components: Vec::new(),
        submission_id: Some("stream-submission".into()),
        submitted_at_epoch_ms: Some(1_000),
        delivery: AgentInputDelivery::CurrentTurn,
    };
    let begun = begin_assistant_turn(
        &mut store,
        BeginAssistantTurn {
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            conversation_id: "committed-runtime".into(),
            conversation_title: "Yielded stream".into(),
            user_message_id: "stream-user".into(),
            user_part_id: "stream-user:body".into(),
            user_text: "stream".into(),
            user_content_json: None,
            assistant_message_id: "stream-submission:assistant".into(),
            context_checkpoint_id: None,
            core_config: config(),
            submission,
            created_at_ms: 1_000,
        },
    )
    .unwrap();
    let run_id = begun.begin.run_id;
    let digest = begun.invocation_config_digest;

    let mut first_driver = CommittedRuntimeDriver::with_delivery_identity(
        YieldingModelHost::default(),
        "stream-generation-1",
        "stream-runtime-1",
    );
    {
        let mut first_port = ApplicationPort {
            store: &mut store,
            invocation_config_digest: digest.clone(),
            run_id: run_id.clone(),
            now_ms: 2_000,
        };
        first_driver.execute(&mut first_port, &mut |_| {}).unwrap();
    }
    assert_eq!(first_driver.driver().requests.len(), 1);

    let mut second_driver = CommittedRuntimeDriver::with_delivery_identity(
        YieldingModelHost::default(),
        "stream-generation-2",
        "stream-runtime-2",
    );
    {
        let mut second_port = ApplicationPort {
            store: &mut store,
            invocation_config_digest: digest,
            run_id: run_id.clone(),
            now_ms: 3_000,
        };
        second_driver
            .execute(&mut second_port, &mut |_| {})
            .unwrap();
    }
    assert!(second_driver.driver().requests.is_empty());

    let pending = load_committed_host_requests(&store, &run_id)
        .unwrap()
        .requests;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].resume_after_sequence, Some(3));
}

#[test]
fn resume_plan_waits_for_a_fresh_committed_request_before_any_runtime_claims() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("resume-waits-for-rebind.db");
    let mut store = NolaStore::open(&database).unwrap();
    let begun = begin_assistant_turn(
        &mut store,
        BeginAssistantTurn {
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            conversation_id: "committed-runtime".into(),
            conversation_title: "Resume waits for rebind".into(),
            user_message_id: "resume-user".into(),
            user_part_id: "resume-user:body".into(),
            user_text: "resume explicitly".into(),
            user_content_json: None,
            assistant_message_id: "resume-assistant".into(),
            context_checkpoint_id: None,
            core_config: config(),
            submission: AgentSubmission::UserInput {
                items: vec![AgentItem::user_message(
                    ItemId::from_string("resume-input".into()),
                    "resume explicitly",
                )],
                turn_context_items: Vec::new(),
                turn_context_platform: Default::default(),
                turn_context_components: Vec::new(),
                submission_id: Some("resume-submission".into()),
                submitted_at_epoch_ms: Some(1_000),
                delivery: AgentInputDelivery::CurrentTurn,
            },
            created_at_ms: 1_000,
        },
    )
    .unwrap();
    let run_id = begun.begin.run_id;
    let digest = begun.invocation_config_digest;
    let request = claim_committed_host_requests(
        &mut store,
        ClaimCommittedHostRequests {
            run_id: run_id.clone(),
            worker_generation: "old-generation".into(),
            lease_owner: "old-runtime".into(),
            claimed_at_ms: 1_100,
            lease_expires_at_ms: 10_000,
            limit: 1,
        },
    )
    .unwrap()
    .requests
    .remove(0);
    reduce_and_accept_host_response(
        &mut store,
        ReduceAndAcceptHostResponse {
            conversation_id: "committed-runtime".into(),
            run_id: run_id.clone(),
            invocation_config_digest: digest.clone(),
            response: response(&request, 1, HostResponse::ExecutionStarted),
            received_at_ms: 1_200,
        },
    )
    .unwrap();
    let lease = request.delivery.as_ref().unwrap();
    let started = store
        .effect_delivery_attempt(&lease.attempt_id)
        .unwrap()
        .unwrap();
    let interrupted = nola_kernel::interrupt_effect_delivery_attempt(
        &started,
        &lease.worker_generation,
        &lease.lease_owner,
        &lease.lease_token,
        nola_domain::EffectDeliveryRecoveryPlan::Resume,
        "worker detached after execution started",
        1_300,
    )
    .unwrap();
    store
        .commit_effect_delivery_attempt_mutations(&[interrupted.clone()])
        .unwrap();

    let mut first_driver = CommittedRuntimeDriver::with_delivery_identity(
        YieldingModelHost::default(),
        "new-generation-1",
        "new-runtime-1",
    );
    {
        let mut port = ApplicationPort {
            store: &mut store,
            invocation_config_digest: digest.clone(),
            run_id: run_id.clone(),
            now_ms: 2_000,
        };
        first_driver.execute(&mut port, &mut |_| {}).unwrap();
    }
    assert!(first_driver.driver().requests.is_empty());
    assert_eq!(
        store.effect_delivery_attempt(&lease.attempt_id).unwrap(),
        Some(interrupted.record.clone())
    );
    assert!(store
        .active_effect_delivery_attempt(&request.effect_id.0)
        .unwrap()
        .is_none());

    let mut second_driver = CommittedRuntimeDriver::with_delivery_identity(
        YieldingModelHost::default(),
        "new-generation-2",
        "new-runtime-2",
    );
    {
        let mut port = ApplicationPort {
            store: &mut store,
            invocation_config_digest: digest,
            run_id,
            now_ms: 3_000,
        };
        second_driver.execute(&mut port, &mut |_| {}).unwrap();
    }
    assert!(second_driver.driver().requests.is_empty());
    assert!(store
        .active_effect_delivery_attempt(&request.effect_id.0)
        .unwrap()
        .is_none());
}

#[test]
fn interrupted_execution_started_permission_is_not_reclaimed_by_a_second_driver() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("committed-runtime.db");
    let mut config = config();
    config.tools = vec![
        json!({"type":"function","function":{"name":"test.write","parameters":{"type":"object"}}}),
    ];
    let mut tool = capability("test.write", HostCapabilityRoute::Tool);
    tool.exposure = HostCapabilityExposure::Visible;
    tool.permission_domain = Some("filesystem.write".into());
    tool.input_normalization.permission_resource =
        nola_agent_engine::agent::host::HostPermissionResourceDeclaration::FirstOf {
            keys: vec!["path".into()],
        };
    config
        .host_catalog
        .as_mut()
        .unwrap()
        .capabilities
        .insert(tool.id.clone(), tool);
    let submission = AgentSubmission::UserInput {
        items: vec![AgentItem::user_message(
            ItemId::from_string("user-1".to_string()),
            "Give a complete answer",
        )],
        turn_context_items: Vec::new(),
        turn_context_platform: Default::default(),
        turn_context_components: Vec::new(),
        submission_id: Some("submission-1".to_string()),
        submitted_at_epoch_ms: Some(1_000),
        delivery: AgentInputDelivery::CurrentTurn,
    };
    let mut store = NolaStore::open(&database).unwrap();
    let begun = begin_assistant_turn(
        &mut store,
        BeginAssistantTurn {
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            conversation_id: "committed-runtime".to_string(),
            conversation_title: "Committed runtime".to_string(),
            user_message_id: "user-1".to_string(),
            user_part_id: "user-1:body".to_string(),
            user_text: "Give a complete answer".to_string(),
            user_content_json: None,
            assistant_message_id: "submission-1:assistant".to_string(),
            context_checkpoint_id: None,
            core_config: config.clone(),
            submission,
            created_at_ms: 1_000,
        },
    )
    .unwrap();
    let invocation_config_digest = begun.invocation_config_digest.clone();
    let run_id = begun.begin.run_id;

    let model = claim_committed_host_requests(
        &mut store,
        ClaimCommittedHostRequests {
            run_id: run_id.clone(),
            worker_generation: "model-generation".into(),
            lease_owner: "model-runtime".into(),
            claimed_at_ms: 1_800,
            lease_expires_at_ms: 60_000,
            limit: 1,
        },
    )
    .unwrap()
    .requests
    .remove(0);
    reduce_and_accept_host_response(
        &mut store,
        ReduceAndAcceptHostResponse {
            conversation_id: "committed-runtime".into(),
            run_id: run_id.clone(),
            invocation_config_digest: invocation_config_digest.clone(),
            response: response(&model, 1, HostResponse::ExecutionStarted),
            received_at_ms: 1_850,
        },
    )
    .unwrap();
    reduce_and_accept_host_response(
        &mut store,
        ReduceAndAcceptHostResponse {
            conversation_id: "committed-runtime".into(),
            run_id: run_id.clone(),
            invocation_config_digest: invocation_config_digest.clone(),
            response: response(
                &model,
                2,
                HostResponse::ModelAttemptStarted {
                    provider: "fixture-provider".into(),
                    model: "test-model".into(),
                    reason: "initial".into(),
                },
            ),
            received_at_ms: 1900,
        },
    )
    .unwrap();
    reduce_and_accept_host_response(&mut store, ReduceAndAcceptHostResponse {
        conversation_id: "committed-runtime".into(), run_id: run_id.clone(), invocation_config_digest: invocation_config_digest.clone(),
        response: response(&model, 3, HostResponse::ModelCompleted { items: vec![], tool_calls: vec![json!({"call_id":"call-1","name":"test.write","arguments":{"path":"/workspace/original"}})], usage: json!({}) }), received_at_ms: 2000,
    }).unwrap();
    let mut first_driver = CommittedRuntimeDriver::with_delivery_identity(
        InterruptedPermissionHost::default(),
        "generation-1",
        "runtime-1",
    );
    {
        let mut port = ApplicationPort {
            store: &mut store,
            invocation_config_digest: invocation_config_digest.clone(),
            run_id: run_id.clone(),
            now_ms: 3000,
        };
        first_driver.execute(&mut port, &mut |_| {}).unwrap();
    }
    assert_eq!(first_driver.driver().requests.len(), 1);
    let first_delivery = first_driver.driver().requests[0]
        .delivery
        .as_ref()
        .expect("claimed request carries Application fencing");
    assert_eq!(first_delivery.worker_generation, "generation-1");
    assert_eq!(first_delivery.lease_owner, "runtime-1");
    drop(store);
    let mut store = NolaStore::open(&database).unwrap();
    let mut second_driver = CommittedRuntimeDriver::with_delivery_identity(
        InterruptedPermissionHost::default(),
        "generation-2",
        "runtime-2",
    );
    {
        let mut port = ApplicationPort {
            store: &mut store,
            invocation_config_digest,
            run_id: run_id.clone(),
            now_ms: 4000,
        };
        second_driver.execute(&mut port, &mut |_| {}).unwrap();
    }
    assert!(
        second_driver.driver().requests.is_empty(),
        "another Runtime must not reclaim a delivery whose execution started"
    );
    let pending = load_committed_host_requests(&store, &run_id)
        .unwrap()
        .requests;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].resume_after_sequence, Some(2));
}

fn begin_silent_model_run(store: &mut NolaStore, suffix: &str) -> (String, String) {
    let submission_id = format!("silent-submission-{suffix}");
    let begun = begin_assistant_turn(
        store,
        BeginAssistantTurn {
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            conversation_id: "committed-runtime".into(),
            conversation_title: "Silent delivery".into(),
            user_message_id: format!("silent-user-{suffix}"),
            user_part_id: format!("silent-user-{suffix}:body"),
            user_text: "wait".into(),
            user_content_json: None,
            assistant_message_id: format!("{submission_id}:assistant"),
            context_checkpoint_id: None,
            core_config: config(),
            submission: AgentSubmission::UserInput {
                items: vec![AgentItem::user_message(
                    ItemId::from_string(format!("silent-input-{suffix}")),
                    "wait",
                )],
                turn_context_items: Vec::new(),
                turn_context_platform: Default::default(),
                turn_context_components: Vec::new(),
                submission_id: Some(submission_id),
                submitted_at_epoch_ms: Some(1_000),
                delivery: AgentInputDelivery::CurrentTurn,
            },
            created_at_ms: 1_000,
        },
    )
    .unwrap();
    (begun.begin.run_id, begun.invocation_config_digest)
}

/// Reading a clock never consumes time. Only the fixture's accepted execution
/// and renewal boundaries advance its logical epoch; scheduling cadence remains real.
struct LogicalDeliveryClock {
    epoch: std::sync::atomic::AtomicI64,
    step_ms: i64,
}
impl LogicalDeliveryClock {
    fn now(&self) -> i64 { self.epoch.load(std::sync::atomic::Ordering::SeqCst) }
    fn advance(&self) { self.epoch.fetch_add(self.step_ms, std::sync::atomic::Ordering::SeqCst); }
}
#[test]
fn observing_delivery_time_never_consumes_the_fixture_lease() {
    let clock = LogicalDeliveryClock { epoch: std::sync::atomic::AtomicI64::new(2_000), step_ms:25 };
    for _ in 0..10_000 { assert_eq!(clock.now(),2_000); }
    clock.advance();
    for _ in 0..10_000 { assert_eq!(clock.now(),2_025); }
}

struct ObservingPort<'a> {
    clock: Arc<LogicalDeliveryClock>,
    inner: ApplicationPort<'a>,
    renewals: Arc<std::sync::atomic::AtomicUsize>,
    accepted_responses: Arc<std::sync::atomic::AtomicUsize>,
    terminal_expiry: Arc<Mutex<Option<i64>>>,
    reject_renewal: bool,
    renewal_delay: std::time::Duration,
    terminal_ready: Option<Arc<std::sync::atomic::AtomicBool>>,
    last_acked_expiry: Option<i64>,
}

impl CommittedRuntimeApplicationPort for ObservingPort<'_> {
    type Protocol = nola_agent_engine::agent::host::AgentRuntimeProtocol;
    fn load_committed_requests(&mut self) -> Result<Vec<HostRequestEnvelope>, String> {
        self.inner.load_committed_requests()
    }

    fn claim_committed_requests(
        &mut self,
        claim: RuntimeEffectDeliveryClaim,
    ) -> Result<Vec<HostRequestEnvelope>, String> {
        self.inner.claim_committed_requests(claim)
    }

    fn renew_effect_delivery(
        &mut self,
        renewal: RuntimeEffectDeliveryRenewal,
    ) -> Result<nola_agent_engine::agent::host::EffectDeliveryLease, String> {
        assert!(self.accepted_responses.load(std::sync::atomic::Ordering::SeqCst) > 0,
            "renewal must never precede accepted ExecutionStarted");
        self.renewals
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if let Some(ready) = &self.terminal_ready {
            if self.accepted_responses.load(std::sync::atomic::Ordering::SeqCst) >= 3 {
                ready.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
        if !self.renewal_delay.is_zero() {
            std::thread::sleep(self.renewal_delay);
        }
        if self.reject_renewal {
            return Err("fixture rejected effect-delivery renewal".into());
        }
        let renewed = self.inner.renew_effect_delivery(renewal)?;
        self.last_acked_expiry = Some(renewed.lease_expires_at_ms);
        self.clock.advance();
        Ok(renewed)
    }


    fn accept_raw_host_response(
        &mut self,
        response: HostResponseEnvelope,
    ) -> Result<CommittedRuntimeAcceptance, String> {
        if response.payload.is_terminal() {
            if let Some(expiry) = self.last_acked_expiry {
                assert_eq!(response.delivery.as_ref().unwrap().lease_expires_at_ms, expiry,
                    "terminal acceptance uses the latest serialized renewal acknowledgement");
            }
            *self.terminal_expiry.lock().unwrap() = response
                .delivery
                .as_ref()
                .map(|delivery| delivery.lease_expires_at_ms);
        }
        let started = matches!(&response.payload, HostResponse::ExecutionStarted);
        let accepted = self.inner.accept_raw_host_response(response)?;
        if started { self.clock.advance(); }
        self.accepted_responses
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(accepted)
    }

    fn accept_boundary_input(
        &mut self,
        input: nola_agent_engine::agent::host::AgentBoundaryInput,
    ) -> Result<CommittedRuntimeAcceptance, String> {
        self.inner.accept_boundary_input(input)
    }
}

struct SilentModelHost {
    before_start: std::time::Duration,
    after_start: std::time::Duration,
    renewals: Arc<std::sync::atomic::AtomicUsize>,
    renewals_seen_before_start: Arc<std::sync::atomic::AtomicUsize>,
    initial_expiry: Arc<Mutex<Option<i64>>>,
    continue_after_rejection: bool,
    abort_requested: Arc<std::sync::atomic::AtomicBool>,
    chatty: bool,
    terminal_ready: Option<Arc<std::sync::atomic::AtomicBool>>,
}

impl AgentHostDriver for SilentModelHost {
    type Protocol = nola_agent_engine::agent::host::AgentRuntimeProtocol;
    type Error = String;

    fn dispatch(
        &mut self,
        request: HostRequestEnvelope,
        emit: &mut dyn FnMut(HostResponseEnvelope) -> Result<(), String>,
    ) -> Result<(), String> {
        *self.initial_expiry.lock().unwrap() = request
            .delivery
            .as_ref()
            .map(|delivery| delivery.lease_expires_at_ms);
        std::thread::sleep(self.before_start);
        let _ = self.renewals_seen_before_start.compare_exchange(
            usize::MAX,
            self.renewals.load(std::sync::atomic::Ordering::SeqCst),
            std::sync::atomic::Ordering::SeqCst,
            std::sync::atomic::Ordering::SeqCst,
        );
        emit(response(&request, 1, HostResponse::ExecutionStarted))?;
        let wait_started = std::time::Instant::now();
        while wait_started.elapsed() < self.after_start
            || (!self.after_start.is_zero() && !self.continue_after_rejection
                && self.renewals.load(std::sync::atomic::Ordering::SeqCst) < 4)
        {
            assert!(wait_started.elapsed() < std::time::Duration::from_secs(5),
                "silent heartbeat barrier timed out");
            if self
                .abort_requested
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let attempt = emit(response(
            &request,
            2,
            HostResponse::ModelAttemptStarted {
                provider: "fixture-provider".into(),
                model: "fixture-model".into(),
                reason: "initial".into(),
            },
        ));
        if attempt.is_err() && !self.continue_after_rejection {
            return attempt;
        }
        let text_part_id = match &request.payload {
            HostRequest::ModelStream { text_part_id, .. } => text_part_id.clone(),
            _ => return Err("silent fixture expected model request".into()),
        };
        if self.chatty {
            for sequence in 3..25 {
                std::thread::sleep(std::time::Duration::from_millis(5));
                emit(response(
                    &request,
                    sequence,
                    HostResponse::ModelDelta {
                        channel: ModelDeltaChannel::Text,
                        part_id: text_part_id.clone(),
                        delta: "x".into(),
                    },
                ))?;
            }
            return emit(response(
                &request,
                25,
                HostResponse::ModelCompleted {
                    items: Vec::new(),
                    tool_calls: Vec::new(),
                    usage: json!({}),
                },
            ));
        }
        let delta = emit(response(
            &request,
            3,
            HostResponse::ModelDelta {
                channel: ModelDeltaChannel::Text,
                part_id: text_part_id,
                delta: "done".into(),
            },
        ));
        if let Some(ready) = &self.terminal_ready {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while !ready.load(std::sync::atomic::Ordering::SeqCst) {
                assert!(std::time::Instant::now() < deadline, "renewal barrier timed out");
                std::thread::yield_now();
            }
        }
        let terminal = emit(response(
            &request,
            4,
            HostResponse::ModelCompleted {
                items: Vec::new(),
                tool_calls: Vec::new(),
                usage: json!({}),
            },
        ));
        attempt.and(delta).and(terminal)
    }
}

fn short_timing(lease_ms: u64, cadence_ms: u64) -> RuntimeEffectDeliveryTiming {
    RuntimeEffectDeliveryTiming {
        lease_duration: std::time::Duration::from_millis(lease_ms),
        heartbeat_cadence: std::time::Duration::from_millis(cadence_ms),
    }
}

fn run_silent_delivery(
    database_name: &str,
    host: SilentModelHost,
    timing: RuntimeEffectDeliveryTiming,
    reject_renewal: bool,
    renewal_delay: std::time::Duration,
) -> (
    Result<(), nola_runtime_driver::RuntimeDriverError<String>>,
    Arc<std::sync::atomic::AtomicUsize>,
    Arc<std::sync::atomic::AtomicUsize>,
    Arc<Mutex<Option<i64>>>,
    Option<nola_agent_engine::agent::protocol::AgentControlSnapshot>,
) {
    let directory = tempdir().unwrap();
    let database = directory.path().join(database_name);
    let mut store = NolaStore::open(&database).unwrap();
    let (run_id, invocation_config_digest) = begin_silent_model_run(&mut store, database_name);
    let renewals = Arc::clone(&host.renewals);
    let abort_requested = Arc::clone(&host.abort_requested);
    let terminal_ready = host.terminal_ready.clone();
    let accepted_responses = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let terminal_expiry = Arc::new(Mutex::new(None));
    let clock = Arc::new(LogicalDeliveryClock {
        epoch: std::sync::atomic::AtomicI64::new(2_000),
        step_ms: timing.heartbeat_cadence.as_millis() as i64,
    });
    let mut driver = CommittedRuntimeDriver::with_delivery_identity_and_timing(
        host,
        format!("generation-{database_name}"),
        format!("owner-{database_name}"),
        timing,
    )
    .with_delivery_clock(Arc::new({
        let clock = Arc::clone(&clock);
        move || clock.now()
    }))
    .with_delivery_abort(Arc::new(move |_| {
        abort_requested.store(true, std::sync::atomic::Ordering::SeqCst);
    }));
    let mut port = ObservingPort {
        clock,
        inner: ApplicationPort {
            store: &mut store,
            invocation_config_digest,
            run_id,
            now_ms: 2_000,
        },
        renewals: Arc::clone(&renewals),
        accepted_responses: Arc::clone(&accepted_responses),
        terminal_expiry: Arc::clone(&terminal_expiry),
        reject_renewal,
        renewal_delay,
        terminal_ready,
        last_acked_expiry: None,
    };
    let mut last_control = None;
    let result = driver.execute(&mut port, &mut |output| {
        if let CoreHostOutput::Control(control) = output {
            last_control = Some(control);
        }
    });
    (
        result,
        renewals,
        accepted_responses,
        terminal_expiry,
        last_control,
    )
}

#[test]
fn silent_dispatch_renews_past_the_original_lease_and_uses_the_acked_expiry() {
    let renewals = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let before_start = Arc::new(std::sync::atomic::AtomicUsize::new(usize::MAX));
    let initial_expiry = Arc::new(Mutex::new(None));
    let host = SilentModelHost {
        before_start: std::time::Duration::ZERO,
        after_start: std::time::Duration::from_millis(170),
        renewals: Arc::clone(&renewals),
        renewals_seen_before_start: Arc::clone(&before_start),
        initial_expiry: Arc::clone(&initial_expiry),
        continue_after_rejection: false,
        abort_requested: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        chatty: false,
        terminal_ready: None,
    };
    let (result, renewals, _, terminal_expiry, control) = run_silent_delivery(
        "silent-renew.db",
        host,
        short_timing(90, 25),
        false,
        std::time::Duration::ZERO,
    );
    result.unwrap();
    assert!(control
        .expect("actual committed terminal control")
        .turn_status
        .is_terminal());
    assert!(renewals.load(std::sync::atomic::Ordering::SeqCst) >= 4);
    assert_eq!(before_start.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert!(terminal_expiry.lock().unwrap().unwrap() >= initial_expiry.lock().unwrap().unwrap() + 100,
        "four logical heartbeats carry the dispatch past its original 90ms lease");
}

#[test]
fn continuous_model_deltas_do_not_starve_delivery_heartbeats() {
    let renewals = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let initial_expiry = Arc::new(Mutex::new(None));
    let host = SilentModelHost {
        before_start: std::time::Duration::ZERO,
        after_start: std::time::Duration::ZERO,
        renewals: Arc::clone(&renewals),
        renewals_seen_before_start: Arc::new(std::sync::atomic::AtomicUsize::new(usize::MAX)),
        initial_expiry: Arc::clone(&initial_expiry),
        continue_after_rejection: false,
        abort_requested: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        chatty: true,
        terminal_ready: None,
    };
    let (result, renewals, _, terminal_expiry, control) = run_silent_delivery(
        "continuous-deltas.db",
        host,
        short_timing(90, 25),
        false,
        std::time::Duration::ZERO,
    );
    result.unwrap();
    assert!(control
        .expect("actual committed terminal control")
        .turn_status
        .is_terminal());
    assert!(renewals.load(std::sync::atomic::Ordering::SeqCst) >= 2);
    assert!(terminal_expiry.lock().unwrap().unwrap() > initial_expiry.lock().unwrap().unwrap());
}

#[test]
fn renewal_rejection_closes_admission_and_execute_fails() {
    let renewals = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let abort_requested = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let host = SilentModelHost {
        before_start: std::time::Duration::ZERO,
        after_start: std::time::Duration::from_millis(80),
        renewals: Arc::clone(&renewals),
        renewals_seen_before_start: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        initial_expiry: Arc::new(Mutex::new(None)),
        continue_after_rejection: true,
        abort_requested: Arc::clone(&abort_requested),
        chatty: false,
        terminal_ready: None,
    };
    let (result, renewals, accepted, _, control) = run_silent_delivery(
        "renew-rejected.db",
        host,
        short_timing(150, 25),
        true,
        std::time::Duration::ZERO,
    );
    assert!(
        matches!(result, Err(nola_runtime_driver::RuntimeDriverError::Core(error)) if error == "fixture rejected effect-delivery renewal")
    );
    assert_eq!(renewals.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(accepted.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(abort_requested.load(std::sync::atomic::Ordering::SeqCst));
    assert!(
        control
            .as_ref()
            .is_none_or(|control| !control.turn_status.is_terminal()),
        "rejected renewal must not manufacture a terminal control result"
    );
}

#[test]
fn terminal_queued_during_renewal_uses_the_serialized_renewal_ack() {
    let renewals = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let initial_expiry = Arc::new(Mutex::new(None));
    let host = SilentModelHost {
        before_start: std::time::Duration::ZERO,
        after_start: std::time::Duration::ZERO,
        renewals: Arc::clone(&renewals),
        renewals_seen_before_start: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        initial_expiry: Arc::clone(&initial_expiry),
        continue_after_rejection: false,
        abort_requested: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        chatty: false,
        terminal_ready: Some(Arc::new(std::sync::atomic::AtomicBool::new(false))),
    };
    let (result, renewals, _, terminal_expiry, control) = run_silent_delivery(
        "terminal-renew-race.db",
        host,
        short_timing(150, 30),
        false,
        std::time::Duration::from_millis(40),
    );
    result.unwrap();
    assert!(control
        .expect("actual committed terminal control")
        .turn_status
        .is_terminal());
    assert!(renewals.load(std::sync::atomic::Ordering::SeqCst) >= 1);
    assert!(terminal_expiry.lock().unwrap().unwrap() > initial_expiry.lock().unwrap().unwrap());
}

#[test]
fn runtime_does_not_renew_before_execution_started_is_accepted() {
    let renewals = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let before_start = Arc::new(std::sync::atomic::AtomicUsize::new(usize::MAX));
    let host = SilentModelHost {
        before_start: std::time::Duration::from_millis(90),
        after_start: std::time::Duration::ZERO,
        renewals: Arc::clone(&renewals),
        renewals_seen_before_start: Arc::clone(&before_start),
        initial_expiry: Arc::new(Mutex::new(None)),
        continue_after_rejection: false,
        abort_requested: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        chatty: false,
        terminal_ready: None,
    };
    let (result, _, _, _, control) = run_silent_delivery(
        "no-renew-before-start.db",
        host,
        short_timing(250, 20),
        false,
        std::time::Duration::ZERO,
    );
    result.unwrap();
    assert!(control
        .expect("actual committed terminal control")
        .turn_status
        .is_terminal());
    assert_eq!(before_start.load(std::sync::atomic::Ordering::SeqCst), 0);
    // Later accepted frames may legitimately cross a cadence under slow I/O;
    // ObservingPort asserts their ordering instead of a wall-time total count.
}

struct TwoEffectPort {
    requests: Vec<HostRequestEnvelope>,
    renewed_b: Arc<std::sync::atomic::AtomicUsize>,
    terminal_b: Arc<std::sync::atomic::AtomicBool>,
}

impl CommittedRuntimeApplicationPort for TwoEffectPort {
    type Protocol = nola_agent_engine::agent::host::AgentRuntimeProtocol;
    fn load_committed_requests(&mut self) -> Result<Vec<HostRequestEnvelope>, String> {
        Ok(Vec::new())
    }

    fn claim_committed_requests(
        &mut self,
        claim: RuntimeEffectDeliveryClaim,
    ) -> Result<Vec<HostRequestEnvelope>, String> {
        Ok(std::mem::take(&mut self.requests)
            .into_iter()
            .map(|mut request| {
                request.delivery = Some(nola_agent_engine::agent::host::EffectDeliveryLease {
                    attempt_id: format!("attempt-{}", request.effect_id.0),
                    worker_generation: claim.worker_generation.clone(),
                    lease_owner: claim.lease_owner.clone(),
                    lease_token: format!("token-{}", request.effect_id.0),
                    lease_expires_at_ms: claim.lease_expires_at_ms,
                });
                request
            })
            .collect())
    }

    fn renew_effect_delivery(
        &mut self,
        renewal: RuntimeEffectDeliveryRenewal,
    ) -> Result<nola_agent_engine::agent::host::EffectDeliveryLease, String> {
        if renewal.delivery.attempt_id == "attempt-effect-a" {
            return Err("effect-a renewal rejected".into());
        }
        self.renewed_b
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut renewed = renewal.delivery;
        renewed.lease_expires_at_ms = renewal.lease_expires_at_ms;
        Ok(renewed)
    }


    fn accept_raw_host_response(
        &mut self,
        response: HostResponseEnvelope,
    ) -> Result<CommittedRuntimeAcceptance, String> {
        if response.effect_id.0 == "effect-b" && response.payload.is_terminal() {
            self.terminal_b
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
        Ok(CommittedRuntimeAcceptance {
            events: Vec::new(),
            traces: Vec::new(),
            control: None,
        })
    }

    fn accept_boundary_input(
        &mut self,
        _: nola_agent_engine::agent::host::AgentBoundaryInput,
    ) -> Result<CommittedRuntimeAcceptance, String> {
        Err("two-effect fixture has no boundary submissions".into())
    }
}

struct InterleavedTwoEffectHost;

impl AgentHostDriver for InterleavedTwoEffectHost {
    type Protocol = nola_agent_engine::agent::host::AgentRuntimeProtocol;
    type Error = String;

    fn dispatch(
        &mut self,
        _: HostRequestEnvelope,
        _: &mut dyn FnMut(HostResponseEnvelope) -> Result<(), String>,
    ) -> Result<(), Self::Error> {
        unreachable!("two-effect fixture uses its interleaved batch")
    }

    fn dispatch_batch(
        &mut self,
        requests: Vec<HostRequestEnvelope>,
        emit: &mut dyn FnMut(HostResponseEnvelope) -> Result<(), String>,
    ) -> Result<(), Self::Error> {
        let a = requests
            .iter()
            .find(|request| request.effect_id.0 == "effect-a")
            .unwrap();
        let b = requests
            .iter()
            .find(|request| request.effect_id.0 == "effect-b")
            .unwrap();
        emit(response(a, 1, HostResponse::ExecutionStarted))?;
        emit(response(b, 1, HostResponse::ExecutionStarted))?;
        std::thread::sleep(std::time::Duration::from_millis(90));
        emit(response(
            b,
            2,
            HostResponse::Completed {
                output: json!({"effect":"b"}),
                usage: None,
                full_output_ref: None,
                artifacts: Vec::new(),
            },
        ))?;
        Ok(())
    }
}

fn two_effect_request(effect_id: &str) -> HostRequestEnvelope {
    serde_json::from_value(json!({
        "request_id": format!("request-{effect_id}"),
        "effect_id": effect_id,
        "host_id": "fixture-host",
        "session_id": "two-effect-session",
        "catalog_revision": 1,
        "availability_epoch": 1,
        "created_at_ms": 1,
        "payload": {"type":"fetch_context", "source_id":"fixture", "query":{}}
    }))
    .unwrap()
}

#[test]
fn one_renewal_rejection_does_not_stop_another_effect_from_renewing_and_settling() {
    let renewed_b = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let terminal_b = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let aborted = Arc::new(Mutex::new(Vec::<String>::new()));
    let observed_aborts = Arc::clone(&aborted);
    let mut driver = CommittedRuntimeDriver::with_delivery_identity_and_timing(
        InterleavedTwoEffectHost,
        "two-effect-generation",
        "two-effect-owner",
        short_timing(120, 20),
    )
    .with_delivery_abort(Arc::new(move |effect_id| {
        observed_aborts.lock().unwrap().push(effect_id.to_string());
    }));
    let mut port = TwoEffectPort {
        requests: vec![
            two_effect_request("effect-a"),
            two_effect_request("effect-b"),
        ],
        renewed_b: Arc::clone(&renewed_b),
        terminal_b: Arc::clone(&terminal_b),
    };

    let result = driver.execute(&mut port, &mut |_| {});
    assert!(matches!(
        result,
        Err(nola_runtime_driver::RuntimeDriverError::Core(ref error))
            if error == "effect-a renewal rejected"
    ));
    assert_eq!(aborted.lock().unwrap().as_slice(), ["effect-a"]);
    assert!(renewed_b.load(std::sync::atomic::Ordering::SeqCst) >= 2);
    assert!(terminal_b.load(std::sync::atomic::Ordering::SeqCst));
}

#[derive(Default)]
struct ParkedDeliveryHost {
    parent: Option<HostRequestEnvelope>,
}

impl AgentHostDriver for ParkedDeliveryHost {
    type Protocol = nola_agent_engine::agent::host::AgentRuntimeProtocol;
    type Error = String;

    fn dispatch(
        &mut self,
        request: HostRequestEnvelope,
        emit: &mut dyn FnMut(HostResponseEnvelope) -> Result<(), String>,
    ) -> Result<(), Self::Error> {
        emit(response(&request, 1, HostResponse::ExecutionStarted))?;
        match request.effect_id.0.as_str() {
            "delegated-parent" => {
                self.parent = Some(request.clone());
                emit(response(
                    &request,
                    2,
                    HostResponse::DelegatedAgentEvent {
                        event: serde_json::from_value(json!({
                            "type":"permission_requested",
                            "occurrence": {
                                "occurrence_id":"permission-occurrence",
                                "worker_generation":"backend-generation"
                            },
                            "request_id":"backend-request",
                            "call_id":"backend-call",
                            "capability_id":"filesystem.write",
                            "resource":"/fixture/result",
                            "reason":"fixture",
                            "input":{"path":"/fixture/result"}
                        }))
                        .unwrap(),
                    },
                ))?;
            }
            "permission-wait" => {
                std::thread::sleep(std::time::Duration::from_millis(90));
                emit(response(
                    &request,
                    2,
                    HostResponse::Completed {
                        output: json!({"approved":true}),
                        usage: None,
                        full_output_ref: None,
                        artifacts: Vec::new(),
                    },
                ))?;
            }
            "permission-control" => {
                let parent = self.parent.as_ref().expect("parked parent request");
                emit(response(
                    parent,
                    3,
                    HostResponse::DelegatedAgentEvent {
                        event: DelegatedAgentEventV1::TextDelta {
                            part_id: "delegated-text".into(),
                            delta: "resumed".into(),
                        },
                    },
                ))?;
                emit(response(
                    parent,
                    4,
                    HostResponse::DelegatedAgentEvent {
                        event: DelegatedAgentEventV1::Completed {
                            items: Vec::new(),
                            usage: json!({}),
                        },
                    },
                ))?;
                emit(response(
                    &request,
                    2,
                    HostResponse::Completed {
                        output: json!({"written":true}),
                        usage: None,
                        full_output_ref: None,
                        artifacts: Vec::new(),
                    },
                ))?;
            }
            effect => return Err(format!("unexpected effect {effect}")),
        }
        Ok(())
    }
}

struct ParkedDeliveryPort {
    next_effect: usize,
    expiries: std::collections::HashMap<String, i64>,
    resumed_parent_expiry: Option<i64>,
}

impl CommittedRuntimeApplicationPort for ParkedDeliveryPort {
    type Protocol = nola_agent_engine::agent::host::AgentRuntimeProtocol;
    fn load_committed_requests(&mut self) -> Result<Vec<HostRequestEnvelope>, String> {
        Ok(Vec::new())
    }

    fn claim_committed_requests(
        &mut self,
        claim: RuntimeEffectDeliveryClaim,
    ) -> Result<Vec<HostRequestEnvelope>, String> {
        let (effect_id, payload) = match self.next_effect {
            0 => (
                "delegated-parent",
                HostRequest::FetchContext {
                    source_id: "fixture".into(),
                    query: json!({}),
                    freshness_deadline_ms: None,
                },
            ),
            1 => (
                "permission-wait",
                HostRequest::RequestPermission {
                    allow_conversation: true,
                    permission_instructions: Vec::new(),
                    permission_request_id: "permission-occurrence".into(),
                    capability_id: "filesystem.write".into(),
                    resource: "/fixture/result".into(),
                    reason: "fixture".into(),
                    input: json!({"path":"/fixture/result"}),
                    evaluation: None,
                },
            ),
            2 => (
                "permission-control",
                HostRequest::FetchContext {
                    source_id: "fixture-control".into(),
                    query: json!({}),
                    freshness_deadline_ms: None,
                },
            ),
            _ => return Ok(Vec::new()),
        };
        self.next_effect += 1;
        let request = leased_request(effect_id, &claim, payload);
        self.expiries.insert(
            request.delivery.as_ref().unwrap().attempt_id.clone(),
            claim.lease_expires_at_ms,
        );
        Ok(vec![request])
    }

    fn renew_effect_delivery(
        &mut self,
        renewal: RuntimeEffectDeliveryRenewal,
    ) -> Result<EffectDeliveryLease, String> {
        self.expiries.insert(
            renewal.delivery.attempt_id.clone(),
            renewal.lease_expires_at_ms,
        );
        if renewal.delivery.attempt_id == "attempt-permission-wait" {
            self.expiries.insert(
                "attempt-delegated-parent".into(),
                renewal.lease_expires_at_ms,
            );
        }
        Ok(EffectDeliveryLease {
            lease_expires_at_ms: renewal.lease_expires_at_ms,
            ..renewal.delivery
        })
    }


    fn accept_raw_host_response(
        &mut self,
        response: HostResponseEnvelope,
    ) -> Result<CommittedRuntimeAcceptance, String> {
        if let Some(delivery) = &response.delivery {
            let expected = self
                .expiries
                .get(&delivery.attempt_id)
                .copied()
                .ok_or("response used an unknown delivery")?;
            if delivery.lease_expires_at_ms != expected {
                return Err(format!(
                    "response used stale expiry {} instead of {expected}",
                    delivery.lease_expires_at_ms
                ));
            }
            if response.effect_id.0 == "delegated-parent" && response.sequence >= 3 {
                self.resumed_parent_expiry = Some(delivery.lease_expires_at_ms);
            }
        }
        Ok(CommittedRuntimeAcceptance {
            events: Vec::new(),
            traces: Vec::new(),
            control: None,
        })
    }

    fn accept_boundary_input(
        &mut self,
        _submission: nola_agent_engine::agent::host::AgentBoundaryInput,
    ) -> Result<CommittedRuntimeAcceptance, String> {
        Err("boundary submissions are unused by this fixture".into())
    }
}

#[test]
fn delegated_permission_wait_keeps_parent_fence_across_pumps_and_heartbeats() {
    let mut runtime = CommittedRuntimeDriver::with_delivery_identity_and_timing(
        ParkedDeliveryHost::default(),
        "parked-generation",
        "parked-owner",
        short_timing(60, 20),
    );
    let mut port = ParkedDeliveryPort {
        next_effect: 0,
        expiries: Default::default(),
        resumed_parent_expiry: None,
    };
    runtime.execute(&mut port, &mut |_| {}).unwrap();
    let resumed_expiry = port
        .resumed_parent_expiry
        .expect("parent event was admitted after permission resolution");
    assert_eq!(
        Some(&resumed_expiry),
        port.expiries.get("attempt-delegated-parent")
    );
}

/// Exercise the Runtime's nonterminal Code Mode protocol across separate
/// dispatch pumps. The fake port checks the latest lease on every receipt;
/// this deliberately does not claim to validate Core cell admission.
struct ParkedCodeHost {
    parent: Option<HostRequestEnvelope>,
    observation: bool,
    slow_sibling: bool,
}

impl AgentHostDriver for ParkedCodeHost {
    type Protocol = nola_agent_engine::agent::host::AgentRuntimeProtocol;
    type Error = String;

    fn dispatch(&mut self, request: HostRequestEnvelope,
        emit: &mut dyn FnMut(HostResponseEnvelope) -> Result<(), String>,
    ) -> Result<(), String> {
        emit(response(&request, 1, HostResponse::ExecutionStarted))?;
        if request.effect_id.0 == "delegated-parent" {
            let cell = nola_domain::code_mode::CodeCellIdentity {
                host_id: request.host_id.0.clone(), conversation_id: request.session_id.clone(),
                turn_id: request.turn_id.clone().unwrap(), parent_call_id: "exec".into(),
                cell_id: "cell".into(), generation: "generation".into(),
            };
            let payload = if self.observation {
                HostResponse::CodeModeObservationSuspended { cell, call_id: "wait".into() }
            } else {
                HostResponse::CodeModeYielded { cell, output_cursor: 0 }
            };
            emit(response(&request, 2, payload))?;
            self.parent = Some(request);
        } else {
            if request.effect_id.0 == "permission-wait" {
                if self.slow_sibling {
                    std::thread::sleep(std::time::Duration::from_millis(90));
                }
            } else {
                emit(response(self.parent.as_ref().unwrap(), 3, HostResponse::Completed {
                    output: json!({}), usage: None, full_output_ref: None, artifacts: Vec::new(),
                }))?;
            }
            emit(response(&request, 2, HostResponse::Completed {
                output: json!({}), usage: None, full_output_ref: None, artifacts: Vec::new(),
            }))?;
        }
        Ok(())
    }
}

struct ParkedCodePort {
    inner: ParkedDeliveryPort,
    parent_renewals: usize,
}

impl CommittedRuntimeApplicationPort for ParkedCodePort {
    type Protocol = nola_agent_engine::agent::host::AgentRuntimeProtocol;
    fn load_committed_requests(&mut self) -> Result<Vec<HostRequestEnvelope>, String> {
        self.inner.load_committed_requests()
    }
    fn claim_committed_requests(&mut self, claim: RuntimeEffectDeliveryClaim)
        -> Result<Vec<HostRequestEnvelope>, String> {
        self.inner.claim_committed_requests(claim)
    }
    fn renew_effect_delivery(&mut self, renewal: RuntimeEffectDeliveryRenewal)
        -> Result<EffectDeliveryLease, String> {
        if renewal.delivery.attempt_id == "attempt-delegated-parent" {
            self.parent_renewals += 1;
        }
        // Unlike delegated permissions, Code Mode must renew its own lease:
        // no implicit parent renewal by a child permission effect is allowed.
        self.inner.expiries.insert(renewal.delivery.attempt_id.clone(), renewal.lease_expires_at_ms);
        Ok(EffectDeliveryLease { lease_expires_at_ms: renewal.lease_expires_at_ms, ..renewal.delivery })
    }
    fn accept_raw_host_response(&mut self, response: HostResponseEnvelope)
        -> Result<CommittedRuntimeAcceptance, String> {
        self.inner.accept_raw_host_response(response)
    }
    fn accept_boundary_input(&mut self, input: nola_agent_engine::agent::host::AgentBoundaryInput)
        -> Result<CommittedRuntimeAcceptance, String> {
        self.inner.accept_boundary_input(input)
    }
}

#[test]
fn code_cell_and_observer_keep_independent_fences_across_parked_dispatch_pumps() {
    for observation in [false, true] {
        let mut runtime = CommittedRuntimeDriver::with_delivery_identity_and_timing(
            ParkedCodeHost { parent: None, observation, slow_sibling: true }, "code-worker", "code-owner", short_timing(60, 20));
        let mut port = ParkedCodePort {
            inner: ParkedDeliveryPort { next_effect: 0, expiries: Default::default(), resumed_parent_expiry: None },
            parent_renewals: 0,
        };
        runtime.execute(&mut port, &mut |_| {}).unwrap();
        assert!(port.parent_renewals >= 3, "parked Code Mode lease must renew during a slow sibling");
        assert_eq!(port.inner.resumed_parent_expiry,
            port.inner.expiries.get("attempt-delegated-parent").copied());
    }
}

#[test]
fn code_cell_fast_pumps_in_one_clock_tick_do_not_request_nonadvancing_renewals() {
    for observation in [false, true] {
        let mut runtime = CommittedRuntimeDriver::with_delivery_identity_and_timing(
            ParkedCodeHost { parent: None, observation, slow_sibling: false },
            "code-worker", "code-owner", short_timing(60_000, 20_000))
            .with_delivery_clock(Arc::new(|| 1000));
        let mut port = ParkedCodePort {
            inner: ParkedDeliveryPort { next_effect: 0, expiries: Default::default(), resumed_parent_expiry: None },
            parent_renewals: 0,
        };
        runtime.execute(&mut port, &mut |_| {}).unwrap();
        assert_eq!(port.parent_renewals, 0, "all pumps are in the original lease's clock tick");
        assert_eq!(port.inner.resumed_parent_expiry, Some(61_000));
    }
}

fn test_driver<D: nola_host_protocol::HostDriver>(driver: D) -> nola_runtime_driver::CommittedRuntimeDriver<D> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    nola_runtime_driver::CommittedRuntimeDriver::new(driver, nola_host_protocol::HostWorkerProcessIdentity {
        pid: 1, boot_id: "fixture-boot".into(), start_id: "1".into(),
        nonce: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed).to_string(),
    }).unwrap()
}
