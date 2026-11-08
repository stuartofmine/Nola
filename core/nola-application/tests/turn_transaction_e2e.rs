#[path = "../../nola-store/tests/support/remote_submission_from.rs"]
mod remote_submission_fixture;
mod support;
#[path = "support/maintenance_run_lifecycle_cases.rs"]
mod maintenance_run_lifecycle_cases;
use remote_submission_fixture::RemoteSubmissionFixture;
use std::collections::BTreeMap;

use nola_application::{
    accept_host_event, begin_assistant_turn, claim_committed_host_requests,
    load_committed_host_requests, load_conversation_projection_v1,
    reduce_and_accept_host_response, reduce_and_commit_active_submission, AcceptHostEvent,
    ApplicationCommand, BeginAssistantTurn, ClaimCommittedHostRequests,
    ReduceAndAcceptHostResponse, ReduceAndCommitActiveSubmission,
};
use nola_agent_engine::agent::host::{
    CapabilityAvailability, EffectDeliveryLease, HostCancellationMode, HostCapabilityCatalog,
    HostCapabilityDescriptor, HostCapabilityExposure, HostCapabilityParallelism,
    HostCapabilityRoute, HostEffectId, HostId, HostOutputPolicy, HostReplaySafety, HostRequest,
    HostRequestEnvelope, HostResponse, HostResponseEnvelope, ModelDeltaChannel, ModelResponseMode,
};
use nola_agent_engine::agent::parts::{AgentPart, PartOwnership, PartUpdate, TextPart};
use nola_agent_engine::agent::protocol::{
    AgentCoreConfig, AgentEvent, AgentInputDelivery, AgentInputOrigin, AgentItem, AgentSubmission,
    ItemId, SessionId, TurnId,
};
use nola_agent_engine::agent::recovery::{CheckpointReason};
use nola_domain::core_event::{CoreEventEnvelope, EventCursor};
use nola_domain::{ActivityState, MessagePartView, MessageState};
use nola_store::{
    BeginTurnInput, NolaStore, OutboxEffectInput,
};
use nola_store_port::assistant_journal::AssistantJournalRepository;
use tempfile::tempdir;

fn request() -> HostRequestEnvelope {
    HostRequestEnvelope {
        attempt_started: true,
        attempt_id: Some("seed-attempt-1".into()),
        delegated_invocation_id: None,
        request_id: "request-1".to_string(),
        effect_id: HostEffectId("effect-1".to_string()),
        host_id: HostId("host-1".to_string()),
        lease_id: None,
        session_id: "conversation-1".to_string(),
        turn_id: Some("run-1".to_string()),
        catalog_revision: 1,
        availability_epoch: 1,
        created_at_ms: 1_000,
        deadline_ms: Some(60_000),
        batch: None,
        resume_after_sequence: None,
        delivery: None,
        payload: HostRequest::ModelStream {
            model_id: "model-1".to_string(),
            response_mode: ModelResponseMode::Conversational,
            text_part_id: "assistant-part-1".to_string(),
            reasoning_part_id: "reasoning-part-1".to_string(),
            messages: Vec::new(),
            tools: Vec::new(),
            settings: serde_json::json!({}),
        },
    }
}

fn initial_checkpoint() -> serde_json::Value {
    checkpoint(1, 0)
}

fn checkpoint(revision: u64, event_sequence: u64) -> serde_json::Value {
    let core = nola_agent_engine::agent::AgentCore::try_new(AgentCoreConfig {
        turn_identity: support::model_identity::identity(),
        session_id: Some(SessionId::from_string("conversation-1".into())),
        ..AgentCoreConfig::default()
    })
    .unwrap();
    let mut checkpoint = core.checkpoint(CheckpointReason::SessionStarted);
    checkpoint.revision = revision;
    checkpoint.state.journal.revision = revision;
    checkpoint.last_event_cursor.sequence = event_sequence;
    checkpoint.state.journal.next_event_sequence = event_sequence.saturating_add(1);
    serde_json::to_value(checkpoint).unwrap()
}

fn begin(store: &mut NolaStore) {
    use nola_domain::assistant_journal::{AssistantJournalIdentity, ASSISTANT_JOURNAL_VERSION};
    use nola_domain::assistant_journal_operations::{
        empty_journal_content, AssistantJournalOperationV1, AssistantOperationBatchV1,
        DeclareJournalPart, JournalContentField, JournalNumber, JournalPartHead,
        JournalPartMetadata, JournalSection, JournalTextMetadata, JournalVisibility,
    };
    use nola_store_port::assistant_journal::{
        AssistantJournalRepository, AssistantSourceCommitStamp, CommitAssistantJournal,
    };

    // This fixture deliberately constructs fixed canonical IDs at the Store
    // boundary. Reproduce Application admission's atomic public placeholder as
    // well; subsequent replacement events must not manufacture missing heads.
    store.begin_write_unit_of_work().unwrap();
    let prepared = nola_kernel::prepare_model_attempt(
        "run-1",
        "effect-1",
        "effect-1",
        "seed-attempt-1",
        "assistant-part-1",
        "reasoning-part-1",
        1000,
    )
    .unwrap();
    let mut attempts = vec![nola_domain::ModelAttemptMutation {
        expected_revision: None,
        record: prepared.clone(),
    }];
    attempts.extend(
        nola_kernel::start_model_attempt(
            &prepared,
            "seed-attempt-1",
            "fixture-provider",
            "model-1",
            "initial",
            1000,
        )
        .unwrap(),
    );
    store
        .begin_turn(&BeginTurnInput {
            user_part_kind: "text".into(),
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            initial_events: Vec::new(),
            initial_assistant_part_mutations: vec![],
            memory_usage_entries: vec![],
            model_attempt_mutations: attempts,
            delegated_invocation_mutations: Vec::new(),
            delegated_permission_mutations: Vec::new(),
            conversation_id: "conversation-1".to_string(),
            conversation_title: "Test".to_string(),
            user_message_id: "user-1".to_string(),
            user_part_id: "user-part-1".to_string(),
            user_text: "Hello".to_string(),
            user_content_json: Some(r#"{"role":"user","content":"Hello"}"#.to_string()),
            additional_user_parts: Vec::new(),
            assistant_message_id: "assistant-1".to_string(),
            assistant_part_id: "assistant-part-1".to_string(),
            assistant_turn_bindings: vec![
                nola_domain::AssistantTurnBinding::Admission {
                    turn_id: "run-1".into(),
                    assistant_message_id: "assistant-1".into(),
                },
                nola_domain::AssistantTurnBinding::Part {
                    turn_id: "run-1".into(),
                    assistant_message_id: "assistant-1".into(),
                    part_id: "assistant-part-1".into(),
                },
            ],
            run_id: "run-1".to_string(),
            context_checkpoint_id: None,
            checkpoint_revision: 1,
            checkpoint_event_cursor: 0,
            checkpoint_json: initial_checkpoint().to_string(),
            invocation_config_json: "{}".to_string(),
            invocation_config_digest: "conversation-config-digest".to_string(),
            outbox_effects: vec![OutboxEffectInput {
                model_attempt_id: Some("seed-attempt-1".into()),
                delegated_invocation_id: None,
                effect_id: request().effect_id.0,
                request_id: request().request_id,
                effect_kind: "model_stream".to_string(),
                write: nola_store_port::run_effect::OutboxEffectWrite::Declare {
                    payload_json: serde_json::to_string(&request()).unwrap(),
                },
            }],
            created_at_ms: 1_000,
        })
        .unwrap();
    let identity = AssistantJournalIdentity {
        source_store_epoch: store.assistant_source_epoch().unwrap(),
        conversation_id: "conversation-1".into(),
        run_id: "run-1".into(),
        assistant_message_id: "assistant-1".into(),
    };
    let stamp = AssistantSourceCommitStamp::Admission {
        run_id: "run-1".into(),
    };
    let order_key = store
        .reserve_assistant_journal_order(
            &stamp,
            &identity,
            JournalSection::Transcript,
            "assistant-part-1",
        )
        .unwrap();
    let expected = store.assistant_journal_head(&identity).unwrap();
    let body = empty_journal_content(
        &identity,
        "assistant-part-1",
        JournalContentField::Markdown,
        JournalNumber(1),
    )
    .unwrap();
    store
        .commit_assistant_journal(&CommitAssistantJournal {
            stamp,
            expected,
            batch: AssistantOperationBatchV1 {
                protocol_version: ASSISTANT_JOURNAL_VERSION,
                identity: identity.clone(),
                operations: vec![AssistantJournalOperationV1::Declare(DeclareJournalPart {
                    part: JournalPartHead {
                        identity,
                        part_id: "assistant-part-1".into(),
                        generation: JournalNumber(1),
                        revision: JournalNumber(1),
                        metadata_revision: JournalNumber(1),
                        order_key,
                        metadata: JournalPartMetadata::Text(JournalTextMetadata {
                            is_final: false,
                            visibility: JournalVisibility::Public,
                            review: None,
                        }),
                        bodies: vec![body],
                    },
                })],
            },
            supplied_chunks: vec![],
        })
        .unwrap();
    store.commit_write_unit_of_work().unwrap();
    let prepared = nola_kernel::prepare_effect_delivery_attempt(
        "delivery-attempt-1",
        "run-1",
        "effect-1",
        "request-1",
        1,
        nola_domain::EffectDeliveryReplaySafety::Recoverable,
        None,
        1_000,
    )
    .unwrap();
    let claimed = nola_kernel::claim_effect_delivery_attempt(
        &prepared.record,
        "generation-1",
        "runtime-1",
        "token-1",
        60_000,
        1_100,
    )
    .unwrap();
    store
        .claim_effect_delivery_attempts(&[nola_domain::EffectDeliveryAttemptMutation {
            expected_revision: None,
            record: claimed.record.clone(),
        }])
        .unwrap();
    let started = nola_kernel::start_effect_delivery_execution(
        &claimed.record,
        "generation-1",
        "runtime-1",
        "token-1",
        1_200,
    )
    .unwrap();
    store
        .commit_effect_delivery_attempt_mutations(&[started])
        .unwrap();
}

// These fixtures use the production Core/Host transaction boundary. No test may
// manufacture a terminal checkpoint, final aggregate or RunResult.
struct CompletionFixture {
    run: String,
    digest: String,
    request: HostRequestEnvelope,
}
fn begin_real_completion(store: &mut NolaStore) -> CompletionFixture {
    let accepted = begin_assistant_turn(store, BeginAssistantTurn {
        expected_configuration_revision: None,
        expected_parent_message_id: None,
        conversation_id: "recovery-conversation".into(), conversation_title: "Atomic answer".into(),
        user_message_id: "completion-user".into(), user_part_id: "completion-user:body".into(),
        user_text: "Hello".into(), user_content_json: None,
        assistant_message_id: "completion:assistant".into(), context_checkpoint_id: None,
        core_config: runtime_config(),
        submission: AgentSubmission::UserInput {
            items: vec![AgentItem::user_message(ItemId::from_string("completion-user".into()), "Hello")],
            turn_context_items: vec![], turn_context_platform: Default::default(), turn_context_components: vec![],
            submission_id: Some("completion".into()), submitted_at_epoch_ms: Some(1000),
            delivery: AgentInputDelivery::CurrentTurn,
        }, created_at_ms: 1000,
    }).unwrap();
    let mut request = load_committed_host_requests(store, &accepted.begin.run_id).unwrap().requests[0].clone();
    support::admit_model_sampling(store, &accepted.begin.run_id,
        &accepted.invocation_config_digest, &mut request, 1100);
    let fixture = CompletionFixture { run: accepted.begin.run_id,
        digest: accepted.invocation_config_digest, request };
    reduce_and_accept_host_response(store, ReduceAndAcceptHostResponse {
        conversation_id: "recovery-conversation".into(), run_id: fixture.run.clone(),
        invocation_config_digest: fixture.digest.clone(), received_at_ms: 1200,
        response: response_for_request(&fixture.request, 1, HostResponse::ModelDelta {
            channel: ModelDeltaChannel::Text,
            part_id: match &fixture.request.payload {
                HostRequest::ModelStream { text_part_id, .. } => text_part_id.clone(),
                _ => panic!("expected model request"),
            }, delta: "World".into(),
        }),
    }).unwrap();
    fixture
}
fn final_response(fixture: &CompletionFixture) -> ReduceAndAcceptHostResponse {
    ReduceAndAcceptHostResponse {
        conversation_id: "recovery-conversation".into(), run_id: fixture.run.clone(),
        invocation_config_digest: fixture.digest.clone(), received_at_ms: 1300,
        response: response_for_request(&fixture.request, 2, HostResponse::ModelCompleted {
            items: vec![], tool_calls: vec![], usage: serde_json::json!({}),
        }),
    }
}
fn assert_answer_seal(store: &NolaStore, fixture: &CompletionFixture) {
    let seal = store.local_assistant_run_terminal_seal("recovery-conversation", &fixture.run)
        .unwrap().unwrap();
    assert!(matches!(seal.result.value, nola_domain::run_result::RunResultValue::Answered { .. }));
    assert_eq!(seal.result.computed_digest().unwrap(), seal.result_digest);
    assert_eq!(seal.result.run_id, fixture.run);
    assert!(store.active_run("recovery-conversation").unwrap().is_none());
    assert!(store.pending_effects(&fixture.run).unwrap().is_empty());
}
fn completion_facts(store: &NolaStore, fixture: &CompletionFixture) -> serde_json::Value {
    let conversation = store.load_conversation("recovery-conversation").unwrap().unwrap();
    let checkpoint = store.load_checkpoint("recovery-conversation").unwrap().unwrap();
    serde_json::json!({
        "messages": conversation.messages,
        "checkpoint_revision": checkpoint.revision,
        "checkpoint_cursor": checkpoint.event_cursor,
        "pending_effects": store.pending_effects(&fixture.run).unwrap(),
        "seal": store.local_assistant_run_terminal_seal("recovery-conversation", &fixture.run).unwrap(),
    })
}

#[test]
fn completion_atomically_updates_message_checkpoint_run_and_outbox() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("completion.db");
    let mut store = NolaStore::open(&database).unwrap();
    let fixture = begin_real_completion(&mut store);
    let before = store.load_checkpoint("recovery-conversation").unwrap().unwrap().revision;
    reduce_and_accept_host_response(&mut store, final_response(&fixture)).unwrap();
    assert_answer_seal(&store, &fixture);
    drop(store);
    let reopened = NolaStore::open(&database).unwrap();
    assert_answer_seal(&reopened, &fixture);
    let conversation = reopened.load_conversation("recovery-conversation").unwrap().unwrap();
    assert_eq!(conversation.messages.len(), 2);
    assert_eq!(conversation.messages[1].status, "completed");
    assert!(conversation.messages[1].parts.iter().any(|part| part.content_text == "World"));
    assert!(reopened.load_checkpoint("recovery-conversation").unwrap().unwrap().revision > before);
}

#[test]
fn wrong_host_effect_rolls_back_every_completion_projection() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("wrong-effect.db");
    let mut store = NolaStore::open(&database).unwrap();
    let fixture = begin_real_completion(&mut store);
    let before = completion_facts(&store, &fixture);
    let mut response = final_response(&fixture);
    response.response.effect_id = HostEffectId("wrong-effect".into());
    assert!(reduce_and_accept_host_response(&mut store, response).is_err());
    assert_eq!(completion_facts(&store, &fixture), before);
    reduce_and_accept_host_response(&mut store, final_response(&fixture)).unwrap();
    assert_answer_seal(&store, &fixture);
}

#[test]
fn terminal_seal_write_failure_rolls_back_canonical_completion_and_publication() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("seal-failure.db");
    let mut store = NolaStore::open(&database).unwrap();
    let fixture = begin_real_completion(&mut store);
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_terminal_seal BEFORE INSERT ON assistant_run_terminal_seals BEGIN SELECT RAISE(ABORT,'fixture seal failure'); END;").unwrap();
    let before = completion_facts(&store, &fixture);
    let error = reduce_and_accept_host_response(&mut store, final_response(&fixture)).unwrap_err();
    assert!(error.contains("fixture seal failure"), "{error}");
    assert_eq!(completion_facts(&store, &fixture), before);
    assert_eq!(connection.query_row("SELECT count(*) FROM assistant_run_terminal_manifest", [],
        |row| row.get::<_, i64>(0)).unwrap(), 0);
    connection.execute_batch("DROP TRIGGER reject_terminal_seal").unwrap();
    reduce_and_accept_host_response(&mut store, final_response(&fixture)).unwrap();
    assert_answer_seal(&store, &fixture);
    assert!(connection.execute("UPDATE assistant_run_terminal_seals SET seal_json='{}'", []).is_err());
    assert!(connection.execute("UPDATE assistant_run_terminal_manifest SET entry_json='{}'", []).is_err());
}

#[test]
fn terminal_host_replay_is_exact_and_cannot_replace_committed_answer() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("replay.db");
    let mut store = NolaStore::open(&database).unwrap();
    let fixture = begin_real_completion(&mut store);
    reduce_and_accept_host_response(&mut store, final_response(&fixture)).unwrap();
    drop(store);
    let mut store = NolaStore::open(&database).unwrap();
    let before = completion_facts(&store, &fixture);
    let replay = reduce_and_accept_host_response(&mut store, final_response(&fixture)).unwrap();
    assert!(!replay.commit.applied);
    assert_eq!(completion_facts(&store, &fixture), before);
    let mut forged = final_response(&fixture);
    forged.response.payload = HostResponse::ModelCompleted {
        items: vec![serde_json::to_value(AgentItem::assistant_message(
            ItemId::from_string("forged".into()), "Overwritten final text")).unwrap()],
        tool_calls: vec![], usage: serde_json::json!({}),
    };
    assert!(reduce_and_accept_host_response(&mut store, forged).is_err());
    assert_eq!(completion_facts(&store, &fixture), before);
}

#[test]
fn completed_run_rejects_late_host_work_without_reopening_its_assistant() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("sealed-owner.db");
    let mut store = NolaStore::open(&database).unwrap();
    let fixture = begin_real_completion(&mut store);
    reduce_and_accept_host_response(&mut store, final_response(&fixture)).unwrap();
    let before = completion_facts(&store, &fixture);
    let mut late = final_response(&fixture);
    late.response.sequence += 1;
    late.response.payload = HostResponse::ModelDelta { channel: ModelDeltaChannel::Text,
        part_id: match &fixture.request.payload {
            HostRequest::ModelStream { text_part_id, .. } => text_part_id.clone(),
            _ => panic!("expected model request"),
        }, delta: "Late mutation".into() };
    assert!(reduce_and_accept_host_response(&mut store, late).is_err());
    assert_eq!(completion_facts(&store, &fixture), before);
}
// Successor user-message ownership is tested below by the actual NextTurn
// promotion fixture. The removed internal-run fixture used to manufacture
// terminal checkpoints and reuse a sealed assistant, which is no longer legal.

fn response(sequence: u64, payload: HostResponse) -> HostResponseEnvelope {
    HostResponseEnvelope {
        attempt_id: request().attempt_id,
        delegated_invocation_id: None,
        request_id: "request-1".to_string(),
        effect_id: HostEffectId("effect-1".to_string()),
        host_id: HostId("host-1".to_string()),
        lease_id: None,
        session_id: "conversation-1".to_string(),
        turn_id: Some("run-1".to_string()),
        catalog_revision: 1,
        availability_epoch: 1,
        delivery: Some(EffectDeliveryLease {
            attempt_id: "delivery-attempt-1".into(),
            worker_generation: "generation-1".into(),
            lease_owner: "runtime-1".into(),
            lease_token: "token-1".into(),
            lease_expires_at_ms: 60_000,
        }),
        sequence,
        payload,
    }
}

fn runtime_capability(id: &str, route: HostCapabilityRoute) -> HostCapabilityDescriptor {
    HostCapabilityDescriptor {
        id: id.to_string(),
        title: id.to_string(),
        route,
        availability: CapabilityAvailability::Available,
        exposure: HostCapabilityExposure::Internal,
        permission_domain: None,
        input_schema: serde_json::json!({"type": "object"}),
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

fn runtime_config() -> AgentCoreConfig {
    let mut capabilities = BTreeMap::new();
    capabilities.insert(
        "model.stream".to_string(),
        runtime_capability("model.stream", HostCapabilityRoute::Model),
    );
    AgentCoreConfig {
        turn_identity: support::model_identity::identity(),
        session_id: Some(SessionId::from_string("recovery-conversation".to_string())),
        model: Some("test-model".to_string()),
        host_catalog: Some(HostCapabilityCatalog {
            host_id: HostId("recovery-host".to_string()),
            revision: 1,
            availability_epoch: 1,
            lease: None,
            capabilities,
        }),
        ..AgentCoreConfig::default()
    }
}

#[test]
fn scheduled_action_begins_a_canonical_assistant_turn() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("scheduled-action.db");
    let mut store = NolaStore::open(&database).unwrap();

    let begun = begin_assistant_turn(
        &mut store,
        BeginAssistantTurn {
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            conversation_id: "recovery-conversation".to_string(),
            conversation_title: "Scheduled action".to_string(),
            user_message_id: "scheduled-user".to_string(),
            user_part_id: "scheduled-user:body".to_string(),
            user_text: "run the scheduled workflow".to_string(),
            user_content_json: None,
            assistant_message_id: "scheduled-submission:assistant".to_string(),
            context_checkpoint_id: None,
            core_config: runtime_config(),
            submission: AgentSubmission::AutomatedInput {
                origin: AgentInputOrigin::ScheduledAction,
                items: vec![AgentItem::user_message(
                    ItemId::from_string("scheduled-user".to_string()),
                    "run the scheduled workflow",
                )],
                turn_context_items: Vec::new(),
                turn_context_platform: Default::default(),
                turn_context_components: Vec::new(),
                submission_id: Some("scheduled-submission".to_string()),
                submitted_at_epoch_ms: Some(1_000),
            },
            created_at_ms: 1_000,
        },
    )
    .unwrap();

    assert_eq!(begun.begin.conversation_id, "recovery-conversation");
    assert_eq!(begun.begin.user_message_id, "scheduled-user");
}

#[test]
fn promoted_next_turn_messages_are_atomic_canonical_and_survive_cold_reopen() {
    promoted_next_turn_fixture(false);
}

#[test]
fn successor_failure_before_first_body_has_a_durable_assistant_owner() {
    promoted_next_turn_fixture(true);
}

fn promoted_next_turn_fixture(fail_before_body: bool) {
    let directory = tempdir().unwrap();
    let database = directory.path().join("promoted-next-turn.db");
    let config = runtime_config();
    let mut store = NolaStore::open(&database).unwrap();
    store.register_source_submission(&nola_domain::remote_submission::RemoteSubmissionIdentity {
        source_host_id:"source".into(),client_id:"fixture-client".into(),conversation_id:"recovery-conversation".into(),submission_id:"initial-submission".into(),user_message_id:"initial-user".into(),assistant_message_id:"initial-submission:assistant".into(),
    },&serde_json::json!({"fixture_submission":"initial-submission","user_text":"initial request"})).unwrap();
    let begun = begin_assistant_turn(
        &mut store,
        BeginAssistantTurn {
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            conversation_id: "recovery-conversation".to_string(),
            conversation_title: "Promoted next turn".to_string(),
            user_message_id: "initial-user".to_string(),
            user_part_id: "initial-user:body".to_string(),
            user_text: "initial request".to_string(),
            user_content_json: None,
            assistant_message_id: "initial-submission:assistant".to_string(),
            context_checkpoint_id: None,
            core_config: config,
            submission: AgentSubmission::UserInput {
                items: vec![AgentItem::user_message(
                    ItemId::from_string("initial-user".to_string()),
                    "initial request",
                )],
                turn_context_items: Vec::new(),
                turn_context_platform: Default::default(),
                turn_context_components: Vec::new(),
                submission_id: Some("initial-submission".to_string()),
                submitted_at_epoch_ms: Some(1_000),
                delivery: AgentInputDelivery::CurrentTurn,
            },
            created_at_ms: 1_000,
        },
    )
    .unwrap();
    let run_id = begun.begin.run_id.clone();
    let invocation_config_digest = begun.invocation_config_digest.clone();
    let mut first_model = load_committed_host_requests(&mut store, &run_id)
        .unwrap()
        .requests
        .into_iter()
        .find(|request| matches!(request.payload, HostRequest::ModelStream { .. }))
        .unwrap();
    support::admit_model_sampling(
        &mut store,
        &run_id,
        &invocation_config_digest,
        &mut first_model,
        1100,
    );

    let baseline_before_submission = store.load_checkpoint("recovery-conversation").unwrap().unwrap();
    store.register_source_submission(&nola_domain::remote_submission::RemoteSubmissionIdentity {
        source_host_id: "source".into(), client_id: "fixture-client".into(),
        conversation_id: "recovery-conversation".into(), submission_id: "steering-submission".into(),
        user_message_id: "steering-user".into(), assistant_message_id: "steering-submission:assistant".into(),
    }, &serde_json::json!({"fixture_submission":"steering-submission","user_text":"steering request"})).unwrap();
    let queued = reduce_and_commit_active_submission(
        &mut store,
        ReduceAndCommitActiveSubmission {
            conversation_id: "recovery-conversation".to_string(),
            run_id: run_id.clone(),
            transition_id: "queue-steering".to_string(),
            invocation_config_digest: invocation_config_digest.clone(),
            submission: AgentSubmission::UserInput {
                items: vec![AgentItem::user_message(
                    ItemId::from_string("steering-user".to_string()),
                    "steering request",
                )],
                turn_context_items: Vec::new(),
                turn_context_platform: Default::default(),
                turn_context_components: Vec::new(),
                submission_id: Some("steering-submission".to_string()),
                submitted_at_epoch_ms: Some(1_500),
                delivery: AgentInputDelivery::NextTurn,
            },
            committed_at_ms: 1_500,
        },
    )
    .unwrap();
    assert!(queued.commit.applied);
    assert_eq!(store.load_checkpoint("recovery-conversation").unwrap().unwrap().checkpoint_json,
        baseline_before_submission.checkpoint_json,
        "a queued submission must append reducer evidence, not rewrite the full baseline");
    let recovery = nola_store_port::run_effect::RunEffectRepository::load_recovery_checkpoint(
        &store, "recovery-conversation").unwrap().unwrap();
    assert_eq!(recovery.replay_inputs.last().unwrap().kind,
        nola_store_port::run_effect::ReplayInputKind::Submission);
    let expected: serde_json::Value = serde_json::from_str(
        &support::current_checkpoint(&database, "recovery-conversation").checkpoint_json).unwrap();
    assert_eq!(expected["revision"], queued.control.revision);
    assert_eq!(expected["last_event_cursor"]["sequence"], queued.control.last_event_cursor.sequence);
    let reply = serde_json::to_value(&queued).unwrap();
    assert!(reply.get("checkpoint").is_none());
    assert!(reply.get("snapshot").is_none());
    drop(store);
    let store = NolaStore::open(&database).unwrap();
    let cold: serde_json::Value = serde_json::from_str(
        &support::current_checkpoint(&database, "recovery-conversation").checkpoint_json).unwrap();
    assert_eq!(cold, expected,
        "reopening must reconstruct queued input, identity, clocks and checkpoint reason exactly");
    let inspect = rusqlite::Connection::open(&database).unwrap();
    let original: String = inspect.query_row(
        "SELECT submission_json FROM host_event_replay_order WHERE submission_id='queue-steering'",
        [], |row| row.get(0)).unwrap();
    let replace_submission = |replacement: &str| {
        let tx = inspect.unchecked_transaction().unwrap();
        tx.execute("UPDATE host_event_replay_heads SET reduction_bytes=reduction_bytes+length(CAST(?1 AS BLOB))-
            (SELECT length(CAST(submission_json AS BLOB)) FROM host_event_replay_order WHERE submission_id='queue-steering')
            WHERE run_id=?2", rusqlite::params![replacement,run_id]).unwrap();
        tx.execute("UPDATE host_event_replay_order SET submission_json=?1 WHERE submission_id='queue-steering'",
            [replacement]).unwrap();
        tx.commit().unwrap();
    };
    for corruption in ["session", "clock", "host_response"] {
        let mut facts: nola_store_port::run_effect::SubmissionReplayReduction =
            serde_json::from_str(&original).unwrap();
        match corruption {
            "session" => facts.session_id = "foreign-session".into(),
            "clock" => facts.decision_time_ms += 1,
            _ => facts.submission_json = serde_json::to_string(&AgentSubmission::HostResponse {
                response: response_for_request(&first_model, 1, HostResponse::Accepted),
            }).unwrap(),
        }
        replace_submission(&serde_json::to_string(&facts).unwrap());
        let mut runtime = support::open_runtime(&database).unwrap();
        let result = runtime.execute(ApplicationCommand::LoadCheckpoint {
            conversation_id: "recovery-conversation".into(),
        });
        let error = result.expect_err("cold recovery must reject corrupted reducer input");
        assert!(error.contains(if corruption == "host_response" { "Host response cannot bypass" }
            else { "submission recovery identity or decision clock mismatch" }), "{corruption}: {error}");
        drop(runtime);
    }
    replace_submission(&original);
    drop(store);
    let mut store = NolaStore::open(&database).unwrap();
    assert_eq!(
        store
            .load_conversation("recovery-conversation")
            .unwrap()
            .unwrap()
            .messages
            .len(),
        2,
        "queued input is not canonical until Core applies it at a safe boundary"
    );

    let promoted = reduce_and_accept_host_response(
        &mut store,
        ReduceAndAcceptHostResponse {
            conversation_id: "recovery-conversation".to_string(),
            run_id: run_id.clone(),
            invocation_config_digest: invocation_config_digest.clone(),
            response: response_for_request(
                &first_model,
                1,
                HostResponse::ModelCompleted {
                    items: vec![serde_json::to_value(AgentItem::assistant_message(
                        ItemId::from_string("initial-answer".to_string()),
                        "initial answer",
                    ))
                    .unwrap()],
                    tool_calls: Vec::new(),
                    usage: serde_json::json!({}),
                },
            ),
            received_at_ms: 2_000,
        },
    )
    .unwrap();
    assert!(promoted.commit.applied);
    assert!(load_committed_host_requests(&store, &run_id).unwrap().requests.is_empty());
    let successor = nola_application::promote_queued_assistant_turn(
        &mut store, nola_application::PromoteQueuedAssistantTurn {
            conversation_id: "recovery-conversation".into(), created_at_ms: 2050,
        },
    ).unwrap().expect("explicit successor admission");
    assert_ne!(successor.begin.run_id, run_id);
    let run_id = successor.begin.run_id;
    let invocation_config_digest = successor.invocation_config_digest;
    let mut successor_model = load_committed_host_requests(&mut store, &run_id)
        .unwrap()
        .requests
        .into_iter()
        .find(|request| matches!(request.payload, HostRequest::ModelStream { .. }))
        .expect("independent successor admission commits its Model effect");
    let successor_part_id = match &successor_model.payload {
        HostRequest::ModelStream { text_part_id, .. } => text_part_id.clone(),
        _ => unreachable!(),
    };
    let after_promotion = store
        .load_conversation("recovery-conversation")
        .unwrap()
        .unwrap();
    assert_eq!(after_promotion.messages.len(), 4);
    // Preparing the successor Attempt atomically reserves its empty assistant
    // placeholder before any provider output; it does not duplicate user input.
    assert_eq!(after_promotion.messages[3].role, "assistant");
    assert!(after_promotion.messages[3]
        .parts
        .iter()
        .all(|part| part.content_text.is_empty()));
    assert!(!after_promotion.messages[3].parts.is_empty());
    let successor_owner = store.assistant_owner_for_turn(
        "recovery-conversation", &run_id, successor_model.turn_id.as_deref().unwrap(),
    ).unwrap().expect("successor Turn owns its assistant before the first model body or failure");
    assert_eq!(successor_owner.assistant_message_id, after_promotion.messages[3].id);
    let reopened_owner = NolaStore::open(&database).unwrap().assistant_owner_for_turn(
        "recovery-conversation", &run_id, successor_model.turn_id.as_deref().unwrap(),
    ).unwrap().expect("Turn birth binding survives reopening before any body");
    assert_eq!(reopened_owner, successor_owner);
    assert_eq!(
        after_promotion.messages[3].run_id.as_deref(),
        Some(run_id.as_str())
    );
    let prepared = store
        .active_model_attempt_for_effect(&successor_model.effect_id.0)
        .unwrap()
        .unwrap();
    assert_eq!(prepared.state, nola_domain::ModelAttemptState::Prepared);
    assert_eq!(prepared.text_part_id, successor_part_id);

    assert_eq!(after_promotion.messages[2].role, "user");
    assert_eq!(
        after_promotion.messages[2].parts[0].content_text,
        "steering request"
    );
    assert!(store.active_run("recovery-conversation").unwrap().is_some());
    support::admit_model_sampling(
        &mut store,
        &run_id,
        &invocation_config_digest,
        &mut successor_model,
        2100,
    );

    if fail_before_body {
        drop(store);
        let mut store = NolaStore::open(&database).unwrap();
        let failed = reduce_and_accept_host_response(&mut store, ReduceAndAcceptHostResponse {
            conversation_id: "recovery-conversation".into(), run_id: run_id.clone(),
            invocation_config_digest,
            response: response_for_request(&successor_model, 1, HostResponse::Failed {
                error: nola_agent_engine::agent::host::HostError {
                    code: "fixture_failure".into(), message: "failed before first body".into(),
                    retryable: false, http_status: None,
                },
            }), received_at_ms: 2500,
        }).unwrap();
        assert!(failed.commit.applied);
        assert!(!failed.commit.assistant_publications.is_empty(), "failure must publish, not only persist");
        assert_eq!(store.assistant_owner_for_turn("recovery-conversation", &run_id,
            successor_model.turn_id.as_deref().unwrap()).unwrap().unwrap(), successor_owner);
        assert_eq!(store.active_model_attempt_for_effect(&successor_model.effect_id.0)
            .unwrap().unwrap().state, nola_domain::ModelAttemptState::Failed);
        return;
    }

    reduce_and_accept_host_response(
        &mut store,
        ReduceAndAcceptHostResponse {
            conversation_id: "recovery-conversation".to_string(),
            run_id: run_id.clone(),
            invocation_config_digest: invocation_config_digest.clone(),
            response: response_for_request(
                &successor_model,
                1,
                HostResponse::ModelDelta {
                    channel: ModelDeltaChannel::Text,
                    part_id: successor_part_id,
                    delta: "steering answer".to_string(),
                },
            ),
            received_at_ms: 2_500,
        },
    )
    .unwrap();
    assert_live_steering_admissions_recover(&store, "recovery-conversation", &run_id);
    reduce_and_accept_host_response(
        &mut store,
        ReduceAndAcceptHostResponse {
            conversation_id: "recovery-conversation".to_string(),
            run_id: run_id.clone(),
            invocation_config_digest,
            response: response_for_request(
                &successor_model,
                2,
                HostResponse::ModelCompleted {
                    items: Vec::new(),
                    tool_calls: Vec::new(),
                    usage: serde_json::json!({}),
                },
            ),
            received_at_ms: 3_000,
        },
    )
    .unwrap();
    drop(store);

    let store = NolaStore::open(&database).unwrap();
    let conversation = store
        .load_conversation("recovery-conversation")
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
                message.status.as_str(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        visible,
        vec![
            ("user", "initial request".to_string(), "completed"),
            ("assistant", "initial answer".to_string(), "completed"),
            ("user", "steering request".to_string(), "completed"),
            ("assistant", "steering answer".to_string(), "completed"),
        ]
    );
    assert!(store.active_run("recovery-conversation").unwrap().is_none());
    assert!(store.pending_effects(&run_id).unwrap().is_empty());
}

fn assert_live_steering_admissions_recover(
    source: &NolaStore,
    conversation_id: &str,
    run_id: &str,
) {
    use nola_store_port::assistant_journal::AssistantJournalRepository;
    use nola_store_port::remote_session::AdmittedReplicaTurnInput;
    use nola_store_port::run_message_admission::RunMessageAdmissionRepository;
    let source_conversation = source.load_conversation(conversation_id).unwrap().unwrap();
    let through = source
        .local_run_message_admission_head(conversation_id, run_id)
        .unwrap();
    assert!(
        source
            .local_assistant_run_terminal_seal(conversation_id, run_id)
            .unwrap()
            .is_none(),
        "steering ownership is available before Run terminal"
    );
    let assistant_index = source_conversation.messages.iter().position(
        |message| message.id == through.identity.assistant_message_id
    ).expect("admitted Run has an exact assistant owner");
    let user = &source_conversation.messages[assistant_index - 1];
    let receipt = source
        .observed_source_submission_receipt(
            "source",
            "fixture-client",
            conversation_id,
            run_id,
            &through.identity.assistant_message_id,
        )
        .unwrap()
        .unwrap();
    assert_eq!(receipt.phase, nola_domain::remote_submission::RemoteSubmissionPhase::Admitted,
        "successor admission must atomically settle its original source submission");
    assert_eq!(receipt.admission.as_ref().unwrap().identity, through.identity);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("steering-recipient.db");
    let mut target = NolaStore::open(&path).unwrap();
    target
        .admit_remote_fixture_from(
            &AdmittedReplicaTurnInput {
                user_part_kind: "text".into(),
                source_host_id: "source".into(),
                conversation_id: conversation_id.into(),
                conversation_title: "remote".into(),
                user_message_id: user.id.clone(),
                user_part_id: user.parts[0].id.clone(),
                user_text: user.parts[0].content_text.clone(),
                user_content_json: user.parts[0].content_json.clone(),
                assistant_message_id: through.identity.assistant_message_id.clone(),
                assistant_part_id: "remote-placeholder".into(),
                run_id: run_id.into(),
                context_checkpoint_id: None,
                created_at_ms: 1,
            },
            receipt,
        )
        .unwrap();
    assert!(target
        .run_message_admission_reception_state("wrong-host", &through)
        .is_err());
    let mut after = target
        .run_message_admission_reception_state("source", &through)
        .unwrap();
    while after.sequence.0 < through.sequence.0 {
        let page = source
            .read_run_message_admissions(&after, &through, 1, 262144)
            .unwrap();
        target.begin_write_unit_of_work().unwrap();
        let mut forged = page.clone();
        forged.records[0].text.push('x');
        assert!(target
            .receive_run_message_admissions("source", &forged)
            .is_err());
        assert_eq!(
            target
                .run_message_admission_reception_state("source", &through)
                .unwrap(),
            after
        );
        after = target
            .receive_run_message_admissions("source", &page)
            .unwrap();
        assert_eq!(
            target
                .receive_run_message_admissions("source", &page)
                .unwrap(),
            after,
            "exact replay is idempotent"
        );
        target.commit_write_unit_of_work().unwrap();
        drop(target);
        target = NolaStore::open(&path).unwrap();
        assert_eq!(
            target
                .run_message_admission_reception_state("source", &through)
                .unwrap(),
            after
        );
    }
    let mirrored = target.load_conversation(conversation_id).unwrap().unwrap();
    assert_eq!(
        mirrored
            .messages
            .iter()
            .map(|m| (&m.id, &m.role))
            .collect::<Vec<_>>(),
        source_conversation
            .messages
            .iter()
            .skip(assistant_index - 1)
            .take(2)
            .map(|m| (&m.id, &m.role))
            .collect::<Vec<_>>()
    );
    let user_bodies = mirrored
        .messages
        .iter()
        .filter(|m| m.role == "user")
        .map(|m| {
            m.parts
                .iter()
                .map(|p| p.content_text.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    assert_eq!(user_bodies, vec!["steering request"]);
    let successor = source_conversation.messages.last().unwrap();
    assert_eq!(successor.id, through.identity.assistant_message_id);
    let identity = nola_domain::assistant_journal::AssistantJournalIdentity {
        assistant_message_id: successor.id.clone(),
        ..through.identity.clone()
    };
    let cursor = source.local_assistant_journal_head(&identity).unwrap();
    assert!(
        target
            .assistant_journal_reception_state("source", &cursor)
            .is_ok(),
        "live successor is admitted before receiving its journal"
    );
    assert!(
        target.active_run(conversation_id).unwrap().is_some(),
        "admission never finishes the Run"
    );
}

#[test]
fn cold_runtime_rebinds_the_active_request_before_caching_a_boundary_submission() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("cold-boundary-hot-response.db");
    let mut config = runtime_config();
    config.session_id = Some(SessionId::from_string("cold-boundary-conversation".into()));
    let mut store = NolaStore::open(&database).unwrap();
    let begun = begin_assistant_turn(
        &mut store,
        BeginAssistantTurn {
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            conversation_id: "cold-boundary-conversation".to_string(),
            conversation_title: "Cold boundary".to_string(),
            user_message_id: "cold-boundary-user".to_string(),
            user_part_id: "cold-boundary-user:body".to_string(),
            user_text: "first request".to_string(),
            user_content_json: None,
            assistant_message_id: "cold-boundary-start:assistant".to_string(),
            context_checkpoint_id: None,
            core_config: config,
            submission: AgentSubmission::UserInput {
                items: vec![AgentItem::user_message(
                    ItemId::from_string("cold-boundary-user".to_string()),
                    "first request",
                )],
                turn_context_items: Vec::new(),
                turn_context_platform: Default::default(),
                turn_context_components: Vec::new(),
                submission_id: Some("cold-boundary-start".to_string()),
                submitted_at_epoch_ms: Some(1_000),
                delivery: AgentInputDelivery::CurrentTurn,
            },
            created_at_ms: 1_000,
        },
    )
    .unwrap();
    let run_id = begun.begin.run_id;
    let invocation_config_digest = begun.invocation_config_digest;
    let mut first_model = load_committed_host_requests(&mut store, &run_id)
        .unwrap()
        .requests
        .into_iter()
        .find(|request| matches!(request.payload, HostRequest::ModelStream { .. }))
        .expect("initial model request");
    support::admit_model_sampling(
        &mut store,
        &run_id,
        &invocation_config_digest,
        &mut first_model,
        1100,
    );
    drop(store);

    // Opening a fresh runtime reproduces process recovery: its hot Core cache
    // is empty while the canonical checkpoint contains redacted requests.
    let mut runtime = support::open_runtime(&database).unwrap();
    let queued: nola_application::AcceptedActiveSubmission = serde_json::from_value(
        runtime
            .execute(ApplicationCommand::ReduceAndCommitActiveSubmission {
                input: ReduceAndCommitActiveSubmission {
                    conversation_id: "cold-boundary-conversation".to_string(),
                    run_id: run_id.clone(),
                    transition_id: "cold-boundary-follow-up".to_string(),
                    invocation_config_digest: invocation_config_digest.clone(),
                    submission: AgentSubmission::UserInput {
                        items: vec![AgentItem::user_message(
                            ItemId::from_string("cold-boundary-follow-up-user".to_string()),
                            "second request",
                        )],
                        turn_context_items: Vec::new(),
                        turn_context_platform: Default::default(),
                        turn_context_components: Vec::new(),
                        submission_id: Some("cold-boundary-follow-up".to_string()),
                        submitted_at_epoch_ms: Some(1_500),
                        delivery: AgentInputDelivery::NextTurn,
                    },
                    committed_at_ms: 1_500,
                },
            })
            .unwrap(),
    )
    .unwrap();
    assert!(queued.commit.applied);

    // This response is accepted by the same runtime immediately after the
    // boundary transaction. It previously failed with
    // `pending Host effect has no request` because the cached Core retained
    // the persistence-redacted request.
    let promoted: nola_application::AcceptedHostResponse = serde_json::from_value(
        runtime
            .execute(ApplicationCommand::ReduceAndAcceptHostResponse {
                input: ReduceAndAcceptHostResponse {
                    conversation_id: "cold-boundary-conversation".to_string(),
                    run_id: run_id.clone(),
                    invocation_config_digest,
                    response: response_for_request(
                        &first_model,
                        1,
                        HostResponse::ModelCompleted {
                            items: vec![serde_json::to_value(AgentItem::assistant_message(
                                ItemId::from_string("cold-boundary-first-answer".to_string()),
                                "first answer",
                            ))
                            .unwrap()],
                            tool_calls: Vec::new(),
                            usage: serde_json::json!({}),
                        },
                    ),
                    received_at_ms: 2_000,
                },
            })
            .unwrap(),
    )
    .unwrap();
    assert!(promoted.commit.applied);
    drop(runtime);
    let mut store = NolaStore::open(&database).unwrap();
    assert!(load_committed_host_requests(&store, &run_id).unwrap().requests.is_empty());
    let successor = nola_application::promote_queued_assistant_turn(
        &mut store, nola_application::PromoteQueuedAssistantTurn {
            conversation_id: "cold-boundary-conversation".into(), created_at_ms: 2050,
        },
    ).unwrap().expect("explicit successor admission after cold boundary reduction");
    assert_ne!(successor.begin.run_id, run_id);
    let run_id = successor.begin.run_id;
    assert!(load_committed_host_requests(&store, &run_id)
        .unwrap()
        .requests
        .iter()
        .any(|request| {
            request.turn_id.as_deref() != first_model.turn_id.as_deref()
                && matches!(request.payload, HostRequest::ModelStream { .. })
        }));
}

fn response_for_request(
    request: &HostRequestEnvelope,
    sequence: u64,
    payload: HostResponse,
) -> HostResponseEnvelope {
    HostResponseEnvelope {
        attempt_id: request.attempt_id.clone(),
        delegated_invocation_id: request.delegated_invocation_id.clone(),
        request_id: request.request_id.clone(),
        effect_id: request.effect_id.clone(),
        host_id: request.host_id.clone(),
        lease_id: request.lease_id.clone(),
        session_id: request.session_id.clone(),
        turn_id: request.turn_id.clone(),
        catalog_revision: request.catalog_revision,
        availability_epoch: request.availability_epoch,
        delivery: request.delivery.clone(),
        sequence: sequence + request.resume_after_sequence.unwrap_or(0),
        payload,
    }
}

fn claim_and_start_request(
    store: &mut NolaStore,
    run_id: &str,
    digest: &str,
    request: &HostRequestEnvelope,
    now_ms: i64,
) -> HostRequestEnvelope {
    let claimed = claim_committed_host_requests(
        store,
        ClaimCommittedHostRequests {
            run_id: run_id.into(),
            worker_generation: format!("generation-{now_ms}-{}", request.request_id),
            lease_owner: format!("runtime-{now_ms}-{}", request.request_id),
            claimed_at_ms: now_ms,
            lease_expires_at_ms: now_ms + 60_000,
            limit: 1,
        },
    )
    .unwrap();
    let mut claimed = claimed.requests.into_iter().next().unwrap();
    assert_eq!(claimed.effect_id, request.effect_id);
    let start_sequence = claimed
        .resume_after_sequence
        .map_or(1, |sequence| sequence + 1);
    reduce_and_accept_host_response(
        store,
        ReduceAndAcceptHostResponse {
            conversation_id: request.session_id.clone(),
            run_id: run_id.into(),
            invocation_config_digest: digest.into(),
            response: response_for_request(
                &claimed,
                start_sequence,
                HostResponse::ExecutionStarted,
            ),
            received_at_ms: now_ms + 1,
        },
    )
    .unwrap();
    claimed.resume_after_sequence = Some(start_sequence);
    claimed
}

#[test]
fn raw_host_delta_is_reduced_from_sqlite_after_runtime_loss() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("runtime-recovery.db");
    let config = runtime_config();
    let user_id = "recovery-user";
    let submission = AgentSubmission::UserInput {
        items: vec![AgentItem::user_message(
            ItemId::from_string(user_id.to_string()),
            "Stream after recovery",
        )],
        turn_context_items: Vec::new(),
        turn_context_platform: Default::default(),
        turn_context_components: Vec::new(),
        submission_id: Some("recovery-submission".to_string()),
        submitted_at_epoch_ms: Some(1_000),
        delivery: AgentInputDelivery::CurrentTurn,
    };

    let mut store = NolaStore::open(&database).unwrap();
    let begun = begin_assistant_turn(
        &mut store,
        BeginAssistantTurn {
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            conversation_id: "recovery-conversation".to_string(),
            conversation_title: "Recovery".to_string(),
            user_message_id: user_id.to_string(),
            user_part_id: format!("{user_id}:body"),
            user_text: "Stream after recovery".to_string(),
            user_content_json: None,
            assistant_message_id: "recovery-submission:assistant".to_string(),
            context_checkpoint_id: None,
            core_config: config.clone(),
            submission,
            created_at_ms: 1_000,
        },
    )
    .unwrap();
    let invocation_config_digest = begun.invocation_config_digest.clone();
    let run_id = begun.begin.run_id;
    drop(store);

    let mut store = NolaStore::open(&database).unwrap();
    let mut model_request = load_committed_host_requests(&mut store, &run_id)
        .unwrap()
        .requests
        .into_iter()
        .find(|request| matches!(request.payload, HostRequest::ModelStream { .. }))
        .unwrap();
    support::admit_model_sampling(
        &mut store,
        &run_id,
        &invocation_config_digest,
        &mut model_request,
        1100,
    );
    let baseline_json = store
        .load_checkpoint("recovery-conversation")
        .unwrap()
        .unwrap()
        .checkpoint_json;
    let admitted_replay_inputs =
        nola_store_port::run_effect::RunEffectRepository::load_recovery_checkpoint(
            &store,
            "recovery-conversation",
        )
        .unwrap()
        .unwrap()
        .replay_inputs;
    let guard = rusqlite::Connection::open(&database).unwrap();
    guard.execute_batch("CREATE TRIGGER forbid_stream_baseline_write BEFORE UPDATE OF checkpoint_json ON core_checkpoint_bodies
        BEGIN SELECT RAISE(ABORT,'stream flush must not write a full checkpoint'); END;
        CREATE TRIGGER forbid_stream_request_rewrite BEFORE UPDATE OF payload_json ON effect_outbox_payloads
        BEGIN SELECT RAISE(ABORT,'stream flush must not rewrite immutable Host request'); END;").unwrap();
    let accepted = reduce_and_accept_host_response(
        &mut store,
        ReduceAndAcceptHostResponse {
            conversation_id: "recovery-conversation".to_string(),
            run_id: run_id.clone(),
            invocation_config_digest: invocation_config_digest.clone(),
            response: HostResponseEnvelope {
                attempt_id: model_request.attempt_id.clone(),
                delegated_invocation_id: None,
                request_id: model_request.request_id.clone(),
                effect_id: model_request.effect_id.clone(),
                host_id: model_request.host_id.clone(),
                lease_id: model_request.lease_id.clone(),
                session_id: model_request.session_id.clone(),
                turn_id: model_request.turn_id.clone(),
                catalog_revision: model_request.catalog_revision,
                availability_epoch: model_request.availability_epoch,
                delivery: model_request.delivery.clone(),
                sequence: 3,
                payload: HostResponse::ModelDelta {
                    channel: ModelDeltaChannel::Text,
                    part_id: match &model_request.payload {
                        HostRequest::ModelStream { text_part_id, .. } => text_part_id.clone(),
                        _ => unreachable!(),
                    },
                    delta: "Recovered stream".to_string(),
                },
            },
            received_at_ms: 2_000,
        },
    )
    .unwrap();
    assert!(accepted.commit.applied);
    assert!(accepted
        .events
        .iter()
        .all(|event| !matches!(event.event, AgentEvent::PartUpdated { .. })));
    assert!(!accepted.commit.assistant_publications.is_empty());
    assert_eq!(
        store
            .load_checkpoint("recovery-conversation")
            .unwrap()
            .unwrap()
            .checkpoint_json,
        baseline_json,
        "a durable text delta retains the full baseline bytes"
    );
    let recovery = nola_store_port::run_effect::RunEffectRepository::load_recovery_checkpoint(
        &store,
        "recovery-conversation",
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        recovery.replay_inputs.len(),
        admitted_replay_inputs.len() + 1
    );
    assert_eq!(
        &recovery.replay_inputs[..admitted_replay_inputs.len()],
        admitted_replay_inputs.as_slice(),
        "a text delta retains the exact earlier sampling admission facts"
    );
    guard
        .execute_batch("DROP TRIGGER forbid_stream_baseline_write; DROP TRIGGER forbid_stream_request_rewrite;")
        .unwrap();
    drop(store);

    let mut store = NolaStore::open(&database).unwrap();
    let conversation = store
        .load_conversation("recovery-conversation")
        .unwrap()
        .unwrap();
    assert_eq!(
        conversation.messages[1].parts[0].content_text,
        "Recovered stream"
    );
    assert_eq!(
        conversation.messages[1].status, "running",
        "Application must persist the Core execution state across cold reopen"
    );
    assert_eq!(conversation.messages[1].parts[0].state, "open");
    assert_eq!(store.pending_effects(&run_id).unwrap().len(), 1);
    assert!(
        store
            .load_local_presentation_inputs("recovery-conversation", "[]", None)
            .is_err(),
        "raw presentation readers must not mistake the baseline for current state"
    );
    let visible =
        nola_application::load_conversation_projection_v1(&store, "recovery-conversation").unwrap();
    assert!(serde_json::to_string(&visible)
        .unwrap()
        .contains("Recovered stream"));
    let mut reader = support::open_runtime(&database).unwrap();
    reader
        .execute(ApplicationCommand::LoadConversationContextUsageV1 {
            conversation_id: "recovery-conversation".into(),
        })
        .expect("context usage must use the recovered cursor");
    reader
        .execute(ApplicationCommand::LoadConversationPermissionProjectionV1 {
            conversation_id: "recovery-conversation".into(),
            run_id: run_id.clone(),
            scope: nola_application::PermissionProjectionReadScope::CurrentRun,
        })
        .expect("permission reads must not reject a valid incremental checkpoint cursor");
    drop(reader);
    let cold_resumed_request = load_committed_host_requests(&mut store, &run_id)
        .unwrap()
        .requests
        .into_iter()
        .find(|candidate| candidate.effect_id == model_request.effect_id)
        .expect("cold recovery must retain the committed model effect");
    assert_eq!(
        cold_resumed_request.resume_after_sequence,
        Some(3),
        "checkpoint and outbox cursor must advance in the same durable transaction"
    );
    assert_eq!(
        store
            .load_checkpoint("recovery-conversation")
            .unwrap()
            .unwrap()
            .event_cursor,
        accepted
            .control
            .expect("durable Host response must include control")
            .last_event_cursor
            .sequence as i64
    );

    let completed = reduce_and_accept_host_response(
        &mut store,
        ReduceAndAcceptHostResponse {
            conversation_id: "recovery-conversation".to_string(),
            run_id: run_id.clone(),
            invocation_config_digest,
            response: HostResponseEnvelope {
                attempt_id: model_request.attempt_id.clone(),
                delegated_invocation_id: None,
                request_id: model_request.request_id.clone(),
                effect_id: model_request.effect_id.clone(),
                host_id: model_request.host_id.clone(),
                lease_id: model_request.lease_id.clone(),
                session_id: model_request.session_id.clone(),
                turn_id: model_request.turn_id.clone(),
                catalog_revision: model_request.catalog_revision,
                availability_epoch: model_request.availability_epoch,
                delivery: model_request.delivery.clone(),
                sequence: 4,
                payload: HostResponse::ModelCompleted {
                    items: Vec::new(),
                    tool_calls: Vec::new(),
                    usage: serde_json::json!({}),
                },
            },
            received_at_ms: 3_000,
        },
    )
    .unwrap();
    assert!(completed.commit.applied);
    let conversation = store
        .load_conversation("recovery-conversation")
        .unwrap()
        .unwrap();
    assert_eq!(
        conversation.messages[1].parts[0].content_text,
        "Recovered stream"
    );
    assert_eq!(conversation.messages[1].parts[0].state, "completed");
}

#[test]
fn receipted_delta_survives_restart_and_replays_without_duplicate_text() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("receipted-stream.db");
    let mut store = NolaStore::open(&database).unwrap();
    let begun = begin_assistant_turn(
        &mut store,
        BeginAssistantTurn {
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            conversation_id: "receipt-conversation".into(),
            conversation_title: "Receipt".into(),
            user_message_id: "receipt-user".into(),
            user_part_id: "receipt-user:body".into(),
            user_text: "Stream".into(),
            user_content_json: None,
            assistant_message_id: "receipt-submission:assistant".into(),
            context_checkpoint_id: None,
            core_config: AgentCoreConfig {
                turn_identity: support::model_identity::identity(),
                session_id: Some(SessionId::from_string("receipt-conversation".into())),
                ..runtime_config()
            },
            submission: AgentSubmission::UserInput {
                items: vec![AgentItem::user_message(
                    ItemId::from_string("receipt-user".into()),
                    "Stream",
                )],
                turn_context_items: Vec::new(),
                turn_context_platform: Default::default(),
                turn_context_components: Vec::new(),
                submission_id: Some("receipt-submission".into()),
                submitted_at_epoch_ms: Some(1_000),
                delivery: AgentInputDelivery::CurrentTurn,
            },
            created_at_ms: 1_000,
        },
    )
    .unwrap();
    let mut request = load_committed_host_requests(&mut store, &begun.begin.run_id)
        .unwrap()
        .requests
        .into_iter()
        .find(|request| matches!(request.payload, HostRequest::ModelStream { .. }))
        .unwrap();
    support::admit_model_sampling(
        &mut store,
        &begun.begin.run_id,
        &begun.invocation_config_digest,
        &mut request,
        1100,
    );
    let text_part_id = match &request.payload {
        HostRequest::ModelStream { text_part_id, .. } => text_part_id.clone(),
        _ => unreachable!(),
    };
    drop(store);
    let command = ApplicationCommand::ReduceAndAcceptHostResponse {
        input: ReduceAndAcceptHostResponse {
            conversation_id: "receipt-conversation".into(),
            run_id: begun.begin.run_id,
            invocation_config_digest: begun.invocation_config_digest,
            response: response_for_request(
                &request,
                1,
                HostResponse::ModelDelta {
                    channel: ModelDeltaChannel::Text,
                    part_id: text_part_id,
                    delta: "Durable small delta".into(),
                },
            ),
            received_at_ms: 2_000,
        },
    };
    let command_json = serde_json::to_string(&command).unwrap();
    let receipt_input = || nola_application::ExecuteReceivedCommand {
        client_id: "receipt-client".into(),
        request_id: "delta-1".into(),
        command_type: "reduce_and_accept_host_response".into(),
        command_json: command_json.clone(),
        expected_revision: None,
        received_at: "2000".into(),
    };
    let mut runtime = support::open_runtime(&database).unwrap();
    // The Port transition may release its savepoint, but the receipt still owns
    // the outer UoW: a failure at the final boundary must restore every fact.
    let inspect = rusqlite::Connection::open(&database).unwrap();
    let facts = || -> Vec<Vec<Vec<String>>> {
        [
            "conversations",
            "messages",
            "message_parts",
            "runs",
            "core_checkpoints",
            "effect_outbox",
            "host_event_inbox",
            "run_events",
            "committed_transitions",
            "sync_outbox",
            "projection_cursors",
            "projection_events",
            "command_receipts",
        ]
        .iter()
        .map(|table| {
            let mut statement = inspect
                .prepare(&format!("SELECT * FROM {table} ORDER BY 1"))
                .unwrap();
            let count = statement.column_count();
            statement
                .query_map([], |row| {
                    (0..count)
                        .map(|index| row.get_ref(index).map(|value| format!("{value:?}")))
                        .collect::<Result<Vec<_>, _>>()
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        })
        .collect()
    };
    let before_failure = facts();
    inspect.execute_batch("CREATE TRIGGER fail_host_receipt BEFORE UPDATE ON command_receipts WHEN NEW.status='committed' BEGIN SELECT RAISE(ABORT,'host receipt fault'); END;").unwrap();
    let failed = runtime
        .execute_received_command(
            receipt_input(),
            serde_json::from_str(&command_json).unwrap(),
        )
        .unwrap_err();
    assert!(failed.to_string().contains("host receipt fault"));
    assert_eq!(
        facts(),
        before_failure,
        "transition, effect settlement, journal and receipt rolled back together"
    );
    inspect
        .execute_batch("DROP TRIGGER fail_host_receipt")
        .unwrap();
    let first = runtime
        .execute_received_command(
            receipt_input(),
            serde_json::from_str(&command_json).unwrap(),
        )
        .unwrap();
    assert!(!first.replayed);
    if let nola_application::ApplicationCommandReceiptOutcome::Committed { result, .. } =
        &first.outcome
    {
        assert!(
            result.get("snapshot").is_none(),
            "durable stream receipts carry deltas, never cumulative conversation snapshots"
        );
        assert!(
            serde_json::to_vec(result).unwrap().len() < 16_000,
            "small stream acknowledgement must stay bounded"
        );
    }
    assert!(matches!(
        &first.outcome,
        nola_application::ApplicationCommandReceiptOutcome::Committed { .. }
    ));
    drop(runtime);
    let mut runtime = support::open_runtime(&database).unwrap();
    let replay = runtime
        .execute_received_command(
            receipt_input(),
            serde_json::from_str(&command_json).unwrap(),
        )
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(first.outcome, replay.outcome);
    drop(runtime);
    let store = NolaStore::open(&database).unwrap();
    let projection =
        nola_application::load_conversation_projection_v1(&store, "receipt-conversation").unwrap();
    let text: String = projection
        .messages
        .iter()
        .flat_map(|message| &message.parts)
        .filter_map(|part| match part {
            nola_domain::MessagePartView::Text { markdown, .. } => Some(markdown.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text.matches("Durable small delta").count(), 1);
}

#[test]
fn application_runtime_broadcasts_small_deltas_before_durable_batch_commit() {
    for (interrupt, fail_boundary) in [(false, false), (true, false), (true, true)] {
        check_runtime_batch_history_boundary(interrupt, fail_boundary);
    }
}

fn physical_rows(
    connection: &rusqlite::Connection,
) -> BTreeMap<String, Vec<Vec<rusqlite::types::Value>>> {
    let tables = connection.prepare(
        "SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name"
    ).unwrap().query_map([], |row| row.get::<_, String>(0)).unwrap()
        .collect::<Result<Vec<_>, _>>().unwrap();
    tables
        .into_iter()
        .map(|table| {
            let quoted = table.replace('"', "\"\"");
            let mut statement = connection
                .prepare(&format!("SELECT * FROM \"{quoted}\""))
                .unwrap();
            let columns = statement.column_count();
            let mut rows = statement
                .query_map([], |row| {
                    (0..columns)
                        .map(|index| row.get::<_, rusqlite::types::Value>(index))
                        .collect()
                })
                .unwrap()
                .collect::<Result<Vec<Vec<_>>, _>>()
                .unwrap();
            rows.sort_by_cached_key(|row| format!("{row:?}"));
            (table, rows)
        })
        .collect()
}

fn check_runtime_batch_history_boundary(interrupt: bool, fail_boundary: bool) {
    let directory = tempdir().unwrap();
    let database = directory.path().join("runtime-stream-batch.db");
    let mut config = runtime_config();
    config.session_id = Some(SessionId::from_string("batch-conversation".into()));
    let submission = AgentSubmission::UserInput {
        items: vec![AgentItem::user_message(
            ItemId::from_string("batch-user".to_string()),
            "Stream in a bounded transaction",
        )],
        turn_context_items: Vec::new(),
        turn_context_platform: Default::default(),
        turn_context_components: Vec::new(),
        submission_id: Some("batch-submission".to_string()),
        submitted_at_epoch_ms: Some(1_000),
        delivery: AgentInputDelivery::CurrentTurn,
    };
    let mut store = NolaStore::open(&database).unwrap();
    let begun = begin_assistant_turn(
        &mut store,
        BeginAssistantTurn {
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            conversation_id: "batch-conversation".to_string(),
            conversation_title: "Batch".to_string(),
            user_message_id: "batch-user".to_string(),
            user_part_id: "batch-user:body".to_string(),
            user_text: "Stream in a bounded transaction".to_string(),
            user_content_json: None,
            assistant_message_id: "batch-submission:assistant".to_string(),
            context_checkpoint_id: None,
            core_config: config.clone(),
            submission,
            created_at_ms: 1_000,
        },
    )
    .unwrap();
    let run_id = begun.begin.run_id;
    let invocation_config_digest = begun.invocation_config_digest;
    let mut request = load_committed_host_requests(&mut store, &run_id)
        .unwrap()
        .requests
        .into_iter()
        .find(|request| matches!(request.payload, HostRequest::ModelStream { .. }))
        .unwrap();
    support::admit_model_sampling(
        &mut store,
        &run_id,
        &invocation_config_digest,
        &mut request,
        1100,
    );
    let text_part_id = match &request.payload {
        HostRequest::ModelStream { text_part_id, .. } => text_part_id.clone(),
        _ => unreachable!(),
    };
    let recovery = nola_store_port::run_effect::RunEffectRepository::load_recovery_checkpoint(
        &store,
        "batch-conversation",
    )
    .unwrap()
    .unwrap();
    let base = recovery
        .replay_base
        .expect("admitted execution must bind its checkpoint replay prefix");
    assert_eq!(base.run_id, run_id);
    let baseline = recovery.checkpoint;
    let current = support::current_checkpoint(&database, "batch-conversation");
    let admission_checkpoint: nola_agent_engine::agent::recovery::CoreCheckpoint<
        nola_agent_engine::agent::AgentThreadState,
    > = serde_json::from_str(&current.checkpoint_json).unwrap();
    let admitted_request = admission_checkpoint
        .state
        .host
        .pending_effects
        .get(&request.effect_id.0)
        .unwrap()
        .validated_request()
        .unwrap();
    assert_eq!(admitted_request.attempt_id, request.attempt_id);
    assert!(admitted_request.attempt_started,
        "cold materialization must contain the typed sampling admission, even when the physical baseline predates it");
    let initial_revision = baseline.revision;
    assert_eq!(current.revision, initial_revision);
    assert!(
        base.checkpoint_revision <= initial_revision,
        "the replay prefix belongs to an earlier or equal physical baseline CAS"
    );
    let baseline_revision = base.checkpoint_revision;
    let baseline_replay_ordinal = base.through_ordinal;
    let rollback_rows =
        fail_boundary.then(|| physical_rows(&rusqlite::Connection::open(&database).unwrap()));
    drop(store);

    let mut runtime = support::open_runtime(&database).unwrap();
    let transient: nola_application::AcceptedHostResponse = serde_json::from_value(
        runtime
            .execute(ApplicationCommand::ReduceAndAcceptHostResponse {
                input: ReduceAndAcceptHostResponse {
                    conversation_id: "batch-conversation".to_string(),
                    run_id: run_id.clone(),
                    invocation_config_digest: invocation_config_digest.clone(),
                    response: response_for_request(
                        &request,
                        1,
                        HostResponse::ModelDelta {
                            channel: ModelDeltaChannel::Text,
                            part_id: text_part_id,
                            delta: "Hello ".to_string(),
                        },
                    ),
                    received_at_ms: 2_000,
                },
            })
            .unwrap(),
    )
    .unwrap();
    assert!(!transient.commit.applied);
    assert!(transient
        .events
        .iter()
        .all(|event| !matches!(&event.event, AgentEvent::PartUpdated { .. })));
    assert!(transient.commit.assistant_publications.is_empty());
    assert!(transient.control.is_none());
    assert!(
        serde_json::to_vec(&transient).unwrap().len() < 64 * 1_024,
        "a transient presentation response must stay independent of checkpoint size"
    );
    let observer = NolaStore::open(&database).unwrap();
    assert_eq!(
        observer
            .load_checkpoint("batch-conversation")
            .unwrap()
            .unwrap()
            .revision,
        initial_revision,
        "a small presentation delta must not force a durable transaction"
    );
    drop(observer);

    let second_transient: nola_application::AcceptedHostResponse = serde_json::from_value(
        runtime
            .execute(ApplicationCommand::ReduceAndAcceptHostResponse {
                input: ReduceAndAcceptHostResponse {
                    conversation_id: "batch-conversation".to_string(),
                    run_id: run_id.clone(),
                    invocation_config_digest: invocation_config_digest.clone(),
                    response: response_for_request(
                        &request,
                        2,
                        HostResponse::ModelDelta {
                            channel: ModelDeltaChannel::Text,
                            part_id: match &request.payload {
                                HostRequest::ModelStream { text_part_id, .. } => {
                                    text_part_id.clone()
                                }
                                _ => unreachable!(),
                            },
                            delta: "wor".to_string(),
                        },
                    ),
                    received_at_ms: 2_500,
                },
            })
            .unwrap(),
    )
    .unwrap();
    assert!(!second_transient.commit.applied);
    assert!(second_transient.control.is_none());
    let reconnected: nola_application::CommittedHostRequests = serde_json::from_value(
        runtime
            .execute(ApplicationCommand::LoadCommittedHostRequests {
                run_id: run_id.clone(),
            })
            .unwrap(),
    )
    .unwrap();
    let resumed_request = reconnected
        .requests
        .iter()
        .find(|candidate| candidate.effect_id == request.effect_id)
        .expect("reconnecting Host must receive the committed model effect");
    assert_eq!(
        resumed_request.resume_after_sequence,
        Some(4),
        "a reconnect to the surviving Application Runtime must use the hot Core cursor"
    );

    let boundary = if interrupt {
        ApplicationCommand::ReduceAndCommitActiveSubmission {
            input: ReduceAndCommitActiveSubmission {
                conversation_id: "batch-conversation".into(),
                run_id: run_id.clone(),
                transition_id: "batch-interrupt".into(),
                invocation_config_digest: invocation_config_digest.clone(),
                submission: AgentSubmission::Interrupt {
                    goal_control: None, cancellation_id: "test-interruption".into(),
                    reason: Some("test interruption".into()),
                },
                committed_at_ms: 3_000,
            },
        }
    } else {
        ApplicationCommand::ReduceAndAcceptHostResponse {
            input: ReduceAndAcceptHostResponse {
                conversation_id: "batch-conversation".to_string(),
                run_id: run_id.clone(),
                invocation_config_digest,
                response: response_for_request(
                    &request,
                    3,
                    HostResponse::ModelCompleted {
                        items: vec![serde_json::to_value(AgentItem::assistant_message(
                            ItemId::from_string("provider-batch-answer".to_string()),
                            "Hello world",
                        ))
                        .unwrap()],
                        tool_calls: Vec::new(),
                        usage: serde_json::json!({}),
                    },
                ),
                received_at_ms: 3_000,
            },
        }
    };
    if fail_boundary {
        let inject = rusqlite::Connection::open(&database).unwrap();
        inject.execute_batch("CREATE TRIGGER reject_batch_checkpoint BEFORE UPDATE ON core_checkpoints
            BEGIN SELECT RAISE(ABORT, 'injected boundary failure'); END;").unwrap();
    }
    let result = runtime.execute(boundary);
    if fail_boundary {
        assert!(result.is_err());
        drop(runtime);
        let inspect = rusqlite::Connection::open(&database).unwrap();
        let after = physical_rows(&inspect);
        let mut before = rollback_rows.unwrap();
        // The failed business UoW must not advance the archive target. Its
        // independent publisher may nevertheless ACK or record a retry during
        // this interval; physical database quiescence is not a rollback promise.
        let old_publication = before.get("work_archive_publication").unwrap();
        let publication = after.get("work_archive_publication").unwrap();
        assert_eq!(old_publication.len(), 1);
        assert_eq!(publication.len(), 1);
        let old = &old_publication[0];
        let new = &publication[0];
        assert_eq!(new.len(), 8);
        assert_eq!(new[0], old[0], "archive singleton identity must not change");
        assert_eq!(new[1], old[1], "rolled-back source writes cannot advance archive target");
        let integer = |value: &rusqlite::types::Value| match value {
            rusqlite::types::Value::Integer(value) => *value,
            other => panic!("archive publication integer required: {other:?}"),
        };
        assert!(integer(&new[2]) >= integer(&old[2]));
        assert!(integer(&new[2]) <= integer(&new[1]));
        assert!(integer(&new[3]) >= 0);
        assert!(integer(&new[5]) >= 0);
        for value in [&new[6], &new[7]] {
            assert!(matches!(value, rusqlite::types::Value::Null)
                || matches!(value, rusqlite::types::Value::Integer(time) if *time >= 0));
        }
        if integer(&new[3]) > 0 {
            assert_eq!(new[4], rusqlite::types::Value::Text("archive_publication_failed".into()));
            assert!(integer(&new[5]) >= integer(&new[6]));
        } else {
            assert_eq!(new[4], rusqlite::types::Value::Null);
            assert_eq!(integer(&new[5]), 0);
        }
        // Only independently validated publisher metadata is permitted to
        // progress; every other physical row remains compared byte-for-byte.
        before.insert("work_archive_publication".into(), publication.clone());
        let changed: Vec<_> = before.keys().chain(after.keys()).collect::<std::collections::BTreeSet<_>>()
            .into_iter().filter(|table| before.get(*table) != after.get(*table)).collect();
        assert!(changed.is_empty(),
            "failed batch must roll back every physical table; changed tables: {changed:?}; archive publication before={:?}, after={:?}",
            before.get("work_archive_publication"), after.get("work_archive_publication"));
        let count: i64 = inspect.query_row(
            "SELECT count(*) FROM host_event_inbox WHERE run_id=?1 AND json_extract(payload_json,'$.payload.type')='model_delta'",
            [&run_id], |row| row.get(0)).unwrap();
        assert_eq!(
            count, 0,
            "history must roll back with the failed boundary checkpoint"
        );
        let store = NolaStore::open(&database).unwrap();
        assert_eq!(
            store
                .load_checkpoint("batch-conversation")
                .unwrap()
                .unwrap()
                .revision,
            initial_revision
        );
        return;
    }
    let durable = result.unwrap();
    assert_eq!(durable["commit"]["applied"], true);
    assert_eq!(
        durable["commit"]["checkpoint_revision"],
        initial_revision + 1
    );
    drop(runtime);

    let store = NolaStore::open(&database).unwrap();
    let conversation = store
        .load_conversation("batch-conversation")
        .unwrap()
        .unwrap();
    if !interrupt {
        assert_eq!(
            conversation.messages[1].parts[0].content_text,
            "Hello world"
        );
        assert_eq!(conversation.messages[1].parts[0].state, "completed");
    }
    let inspect = rusqlite::Connection::open(&database).unwrap();
    let mut query = inspect
        .prepare("SELECT payload_json FROM host_event_inbox WHERE run_id=?1 ORDER BY sequence")
        .unwrap();
    let history = query
        .query_map([&run_id], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let deltas = history
        .iter()
        .map(|raw| serde_json::from_str::<serde_json::Value>(raw).unwrap())
        .filter_map(|response| {
            response
                .pointer("/payload/delta")
                .and_then(|value| value.as_str())
                .map(str::to_owned)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        deltas,
        vec!["Hello ", "wor"],
        "either boundary must persist every accepted transient response, not just the last input"
    );
    let (wire, reduced): (String, String) = inspect
        .query_row(
            "SELECT payload_json,reduction_json FROM host_event_inbox WHERE run_id=?1
         AND json_extract(payload_json,'$.payload.type')='execution_started' LIMIT 1",
            [&run_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let wire: serde_json::Value = serde_json::from_str(&wire).unwrap();
    let facts: nola_store_port::host_replay::HostReplayReduction =
        serde_json::from_str(&reduced).unwrap();
    let reduced: serde_json::Value =
        serde_json::from_str(facts.normalized_response().get()).unwrap();
    assert_eq!(wire["payload"]["type"], "execution_started");
    assert_eq!(reduced["payload"]["type"], "accepted");
    assert_eq!(wire["request_id"], reduced["request_id"]);
    assert_eq!(wire["sequence"], reduced["sequence"]);
    // Independent cold Core reconstruction from the pre-batch checkpoint.
    // Use a Run-wide prefix, not per-effect sequence: different effects may
    // interleave or be rebound while retaining the same logical effect ID.
    let baseline_checkpoint: nola_agent_engine::agent::recovery::CoreCheckpoint<
        nola_agent_engine::agent::AgentThreadState,
    > = serde_json::from_str(&baseline.checkpoint_json).unwrap();
    let mut replay_reason = baseline_checkpoint.reason;
    let mut replay =
        nola_agent_engine::agent::AgentCore::restore(config.clone(), baseline_checkpoint).unwrap();
    // Replay begins with the frozen baseline request. The live delivery below
    // already advertises a newer response cursor and must not be injected into
    // history before those responses have themselves been replayed.
    let mut query = inspect
        .prepare(
            "SELECT COALESCE(o.submission_json,h.reduction_json),COALESCE(o.decision_time_ms,h.received_at_ms),
                o.ordinal,o.submission_id IS NOT NULL FROM host_event_replay_order o
         LEFT JOIN host_event_inbox h ON h.event_id=o.event_id AND h.run_id=o.run_id
         WHERE o.run_id=?1 AND o.ordinal>?2 ORDER BY o.ordinal",
        )
        .unwrap();
    let replay_inputs = query
        .query_map(rusqlite::params![run_id, baseline_replay_ordinal], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, bool>(3)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let mut consumed_ordinal = baseline_replay_ordinal;
    for (raw, observed_at_ms, ordinal, is_submission) in replay_inputs {
        assert_eq!(
            ordinal,
            consumed_ordinal + 1,
            "recovery may not skip a missing input"
        );
        if is_submission {
            let facts: nola_store_port::run_effect::SubmissionReplayReduction = serde_json::from_str(&raw).unwrap();
            assert_eq!(facts.decision_time_ms, observed_at_ms);
            for binding in &facts.bindings {
                let mut control = replay.committed_host_effect_control(&binding.effect_id.0).unwrap().unwrap();
                control.apply_binding(binding).unwrap();
                replay.bind_committed_host_control(&control).unwrap();
            }
            let transition = replay.reduce_at(serde_json::from_str(&facts.submission_json).unwrap(),
                u64::try_from(observed_at_ms).unwrap()).unwrap();
            replay_reason = transition.durability.as_ref()
                .map_or(CheckpointReason::Manual, |intent| intent.checkpoint_reason);
            consumed_ordinal = ordinal;
            continue;
        }
        let facts: nola_store_port::host_replay::HostReplayReduction =
            serde_json::from_str(&raw).unwrap();
        facts.validate_clock(observed_at_ms).unwrap();
        let nola_store_port::host_replay::HostReplayReduction::Replayable {
            normalized_response,
            binding,
            decision_time_ms,
            ..
        } = facts
        else {
            panic!("fixture requires complete committed reducer facts")
        };
        let response: HostResponseEnvelope =
            serde_json::from_str(normalized_response.get()).unwrap();
        if let Some(binding) = binding {
            assert_eq!(
                binding.payload_binding,
                nola_store_port::host_replay::CoreHostPayloadBinding::Unchanged,
                "this fixture has only model envelopes"
            );
            let frozen = replay
                .committed_host_request_resume_snapshot(&response.effect_id.0)
                .unwrap()
                .unwrap();
            let mut envelope = serde_json::to_value(binding).unwrap();
            envelope.as_object_mut().unwrap().remove("payload_binding");
            envelope["payload"] = serde_json::to_value(frozen.payload).unwrap();
            let request: HostRequestEnvelope = serde_json::from_value(envelope).unwrap();
            replay.bind_committed_host_effect(&request).unwrap();
        }
        let transition = replay
            .reduce_at(
                AgentSubmission::HostResponse { response },
                u64::try_from(decision_time_ms).unwrap(),
            )
            .unwrap();
        replay_reason = transition
            .durability
            .as_ref()
            .map_or(CheckpointReason::Manual, |intent| intent.checkpoint_reason);
        consumed_ordinal = ordinal;
    }
    let durable_head: i64 = inspect
        .query_row(
            "SELECT ordinal FROM host_event_replay_heads WHERE run_id=?1",
            [&run_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        consumed_ordinal, durable_head,
        "recovery must reach the committed input frontier"
    );
    let stored: nola_agent_engine::agent::recovery::CoreCheckpoint<nola_agent_engine::agent::AgentThreadState> =
        serde_json::from_str(
            &support::current_checkpoint(&database, "batch-conversation").checkpoint_json,
        )
        .unwrap();
    assert_eq!(
        serde_json::to_value(replay.checkpoint(replay_reason)).unwrap(),
        serde_json::to_value(&stored).unwrap(),
        "batch input history must reproduce the exact checkpoint, not just visible text"
    );
    if !interrupt {
        // Exercise the production cold reader, not only an independent Core
        // replay. Preserve the canonical frontier while replacing its full
        // body with the earlier baseline and its explicit replay prefix.
        inspect.execute_batch("BEGIN IMMEDIATE;").unwrap();
        inspect.execute("UPDATE core_checkpoint_bodies SET checkpoint_json=?1 WHERE conversation_id='batch-conversation'",
            [&baseline.checkpoint_json]).unwrap();
        inspect.execute("INSERT INTO core_checkpoint_replay_bases(conversation_id,run_id,checkpoint_revision,through_ordinal,through_reduction_bytes)
            VALUES('batch-conversation',?1,?2,?3,(SELECT COALESCE(sum(length(CAST(h.reduction_json AS BLOB))),0)
                FROM host_event_replay_order o JOIN host_event_inbox h USING(event_id,run_id)
                WHERE o.run_id=?1 AND o.ordinal<=?3))",
            rusqlite::params![run_id, baseline_revision, baseline_replay_ordinal]).unwrap();
        inspect.execute_batch("COMMIT;").unwrap();
        let mut reopened = support::open_runtime(&database).unwrap();
        let result = reopened
            .execute(ApplicationCommand::LoadCheckpoint {
                conversation_id: "batch-conversation".into(),
            })
            .unwrap();
        let current: nola_application::ConversationCheckpoint =
            serde_json::from_value(result).unwrap();
        let recovered: nola_agent_engine::agent::recovery::CoreCheckpoint<
            nola_agent_engine::agent::AgentThreadState,
        > = serde_json::from_str(&current.checkpoint_json).unwrap();
        assert_eq!(
            serde_json::to_value(&recovered).unwrap(),
            serde_json::to_value(&stored).unwrap(),
            "cold replay must preserve every checkpoint field, including reason and clocks"
        );
        let projection: nola_application::SessionProjection = serde_json::from_value(
            reopened
                .execute(ApplicationCommand::LoadSessionProjection {
                    conversation_id: "batch-conversation".into(),
                })
                .unwrap(),
        )
        .unwrap();
        assert_eq!(projection.core_revision, stored.revision);
        assert_eq!(projection.event_cursor, stored.last_event_cursor.sequence);
        assert_eq!(
            serde_json::to_value(&projection.conversation_projection).unwrap(),
            serde_json::to_value(&stored.state.conversation.projection).unwrap(),
            "session display and cold execution must reconstruct the same committed frontier"
        );
        let after: i64 = inspect
            .query_row(
                "SELECT ordinal FROM host_event_replay_heads WHERE run_id=?1",
                [&run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            after, durable_head,
            "reading recovered state must not append new inputs"
        );
        let frontier = support::admission_frontier::read(&NolaStore::open(&database).unwrap(), "batch-conversation");
        let next = reopened.execute(ApplicationCommand::BeginAssistantTurn {
            input: BeginAssistantTurn {
                expected_configuration_revision: frontier.0,
                expected_parent_message_id: frontier.1,
                conversation_id: "batch-conversation".into(), conversation_title: "Batch".into(),
                user_message_id: "next-user".into(), user_part_id: "next-user:body".into(),
                user_text: "Continue from the recovered answer".into(), user_content_json: None,
                assistant_message_id: "next-submission:assistant".into(), context_checkpoint_id: None,
                core_config: config,
                submission: AgentSubmission::UserInput {
                    items: vec![AgentItem::user_message(ItemId::from_string("next-user".into()),
                        "Continue from the recovered answer")],
                    turn_context_items: vec![], turn_context_platform: Default::default(),
                    turn_context_components: vec![], submission_id: Some("next-submission".into()),
                    submitted_at_epoch_ms: Some(4_000), delivery: AgentInputDelivery::CurrentTurn,
                },
                created_at_ms: 4_000,
            },
        }).expect("a new Turn must start from the completed replay frontier, not the awaiting-model baseline");
        assert_ne!(next["begin"]["run_id"].as_str().unwrap(), run_id);
    }
}

#[test]
fn active_interrupt_is_committed_once_and_survives_cold_reopen() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("active-interrupt.db");
    let config = runtime_config();
    let mut store = NolaStore::open(&database).unwrap();
    let begun = begin_assistant_turn(
        &mut store,
        BeginAssistantTurn {
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            conversation_id: "recovery-conversation".to_string(),
            conversation_title: "Interrupt".to_string(),
            user_message_id: "interrupt-user".to_string(),
            user_part_id: "interrupt-user:body".to_string(),
            user_text: "Stop this turn".to_string(),
            user_content_json: None,
            assistant_message_id: "interrupt-start:assistant".to_string(),
            context_checkpoint_id: None,
            core_config: config.clone(),
            submission: AgentSubmission::UserInput {
                items: vec![AgentItem::user_message(
                    ItemId::from_string("interrupt-user".to_string()),
                    "Stop this turn",
                )],
                turn_context_items: Vec::new(),
                turn_context_platform: Default::default(),
                turn_context_components: Vec::new(),
                submission_id: Some("interrupt-start".to_string()),
                submitted_at_epoch_ms: Some(1_000),
                delivery: AgentInputDelivery::CurrentTurn,
            },
            created_at_ms: 1_000,
        },
    )
    .unwrap();
    let invocation_config_digest = begun.invocation_config_digest.clone();
    let run_id = begun.begin.run_id;
    let command = ReduceAndCommitActiveSubmission {
        conversation_id: "recovery-conversation".to_string(),
        run_id: run_id.clone(),
        transition_id: "interrupt-transition".to_string(),
        invocation_config_digest: invocation_config_digest.clone(),
        submission: AgentSubmission::Interrupt {
            goal_control: None, cancellation_id: "user-cancelled".into(),
            reason: Some("user_cancelled".to_string()),
        },
        committed_at_ms: 2_000,
    };
    let accepted = reduce_and_commit_active_submission(&mut store, command.clone()).unwrap();
    assert!(accepted.commit.applied);
    assert_eq!(
        accepted.control.turn_status,
        nola_agent_engine::agent::AgentTurnStatus::Cancelling
    );
    let cancellation = load_committed_host_requests(&mut store, &run_id).unwrap();
    let cancel_request = cancellation
        .requests
        .iter()
        .find(|request| matches!(request.payload, HostRequest::Cancel { .. }))
        .expect("interrupt must durably dispatch Host cancellation before becoming terminal")
        .clone();
    let cancel_request = claim_and_start_request(
        &mut store,
        &run_id,
        &invocation_config_digest,
        &cancel_request,
        2_050,
    );
    assert!(
        store.active_run("recovery-conversation").unwrap().is_some(),
        "canonical Run remains active until Host cancellation is acknowledged"
    );

    let target_effect_id = match &cancel_request.payload {
        HostRequest::Cancel { target_effect_id, .. } => target_effect_id.clone(),
        _ => unreachable!(),
    };
    let settled = reduce_and_accept_host_response(
        &mut store,
        ReduceAndAcceptHostResponse {
            conversation_id: "recovery-conversation".to_string(),
            run_id: run_id.clone(),
            invocation_config_digest,
            response: HostResponseEnvelope {
                attempt_id: cancel_request.attempt_id.clone(),
                delegated_invocation_id: None,
                request_id: cancel_request.request_id.clone(),
                effect_id: cancel_request.effect_id.clone(),
                host_id: cancel_request.host_id.clone(),
                lease_id: cancel_request.lease_id.clone(),
                session_id: cancel_request.session_id.clone(),
                turn_id: cancel_request.turn_id.clone(),
                catalog_revision: cancel_request.catalog_revision,
                availability_epoch: cancel_request.availability_epoch,
                delivery: cancel_request.delivery.clone(),
                sequence: 2,
                payload: HostResponse::CancellationConfirmed { target_effect_id },
            },
            received_at_ms: 2_100,
        },
    )
    .unwrap();
    assert!(settled
        .control
        .expect("terminal Host response must include control")
        .turn_status
        .is_terminal());
    drop(store);

    let mut store = NolaStore::open(&database).unwrap();
    assert!(store.active_run("recovery-conversation").unwrap().is_none());
    let conversation = store
        .load_conversation("recovery-conversation")
        .unwrap()
        .unwrap();
    let assistant = conversation
        .messages
        .iter()
        .find(|message| message.id == "interrupt-start:assistant")
        .unwrap();
    assert_eq!(assistant.status, "cancelled");
    let projection = load_conversation_projection_v1(&store, "recovery-conversation")
        .expect("cold-reopened zero-output cancellation must remain projectable");
    let projected_assistant = projection.messages.last().expect("assistant projection");
    assert_eq!(projected_assistant.state, MessageState::Cancelled);
    assert!(matches!(
        projected_assistant.parts.as_slice(),
        [MessagePartView::ModelAttempt {
            state: ActivityState::Cancelled,
            provider: None,
            model: None,
            failure,
            fallback_attempt_id: None,
            ..
        }] if failure.code == "model_effect_retired" && !failure.retryable
    ));
    assert!(projection.active_run.is_none());
    assert!(load_committed_host_requests(&mut store, &run_id)
        .unwrap()
        .requests
        .is_empty());
}

fn text_event(cursor: u64, text: &str, is_complete: bool) -> CoreEventEnvelope<AgentEvent> {
    let part = AgentPart::Text(TextPart {
        review: None,
        id: "assistant-part-1".to_string(),
        ownership: PartOwnership {
            message_id: "assistant-1".to_string(),
            turn_id: "run-1".to_string(),
        },
        text: text.to_string(),
        is_complete,
    });
    CoreEventEnvelope {
        session_id: "conversation-1".to_string(),
        turn_id: Some("run-1".to_string()),
        cursor: EventCursor { sequence: cursor },
        revision: cursor + 1,
        event: AgentEvent::PartUpdated {
            turn_id: TurnId::from_string("run-1".to_string()),
            projection_revision: cursor,
            update: PartUpdate::TextReplaced {
                part_id: "assistant-part-1".to_string(),
                text: text.to_string(),
                is_complete,
            },
            part,
        },
    }
}

#[test]
fn host_usage_ledger_replay_survives_deletion_and_conflicts_roll_back() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("usage.db");
    let mut store = NolaStore::open(&database).unwrap();
    begin(&mut store);
    store
        .upsert_memory_record(&nola_store_port::memory::StoredMemoryRecord {
            id: "memory".into(),
            owner_json: serde_json::to_string(&nola_domain::memory_owner::MemoryOwner::local_profile()).unwrap(),
            namespace_json: r#"{"kind":"personal"}"#.into(),
            scope: "global".into(),
            kind: "fact".into(),
            content: "fixture".into(),
            summary: None,
            tags_json: "[]".into(),
            confidence: 1.0,
            importance: 0.5,
            sensitivity: None,
            retention_json: "{}".into(),
            expires_at: None,
            supersedes_json: "[]".into(),
            provenance_json: "[]".into(),
            created_at_ms: 1,
            updated_at_ms: 1,
            last_used_at_ms: None,
            deleted_at_ms: None,
        })
        .unwrap();
    let command = |sequence, usage_id: &str, digest: &str| {
        let mut state = checkpoint(sequence + 1, sequence);
        state["state"]["memory_runtime"]["usage_ledger"] = serde_json::json!([{
            "usage_id": usage_id, "memory_id": "memory", "session_id": "conversation-1",
            "turn_id": "run-1", "used_at_epoch_ms": 1, "context_payload_digest": digest,
            "citation_ids": ["citation"]
        }]);
        AcceptHostEvent {
            conversation_id: "conversation-1".into(),
            run_id: "run-1".into(),
            response: response(
                sequence,
                HostResponse::ModelDelta {
                    channel: ModelDeltaChannel::Text,
                    part_id: "assistant-part-1".into(),
                    delta: "x".into(),
                },
            ),
            events: vec![text_event(sequence, "x", false)],
            successor_requests: Vec::new(),
            checkpoint: serde_json::from_value(state).unwrap(),
            received_at_ms: 2_000 + sequence as i64,
        }
    };
    assert!(
        accept_host_event(&mut store, command(1, "usage", "digest"))
            .unwrap()
            .applied
    );
    store.delete_memory_record("memory", 3_000).unwrap();
    let db = rusqlite::Connection::open(&database).unwrap();
    db.execute_batch(
        "CREATE TRIGGER forbid_usage_replay BEFORE INSERT ON memory_usage_ledger
        BEGIN SELECT RAISE(ABORT,'historical usage was resubmitted'); END;",
    )
    .unwrap();
    assert!(
        accept_host_event(&mut store, command(2, "usage", "digest"))
            .unwrap()
            .applied
    );
    let before = store.load_checkpoint("conversation-1").unwrap().unwrap();
    let conflict = accept_host_event(&mut store, command(3, "usage", "changed")).unwrap_err();
    assert!(
        conflict.contains("memory usage identity conflict"),
        "{conflict}"
    );
    let missing = accept_host_event(&mut store, command(3, "new-usage", "digest")).unwrap_err();
    assert!(missing.contains("missing active memory"), "{missing}");
    let after = store.load_checkpoint("conversation-1").unwrap().unwrap();
    assert_eq!(after.revision, before.revision);
    assert_eq!(after.checkpoint_json, before.checkpoint_json);
    assert!(
        accept_host_event(&mut store, command(3, "usage", "digest"))
            .unwrap()
            .applied
    );
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM memory_usage_ledger", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn host_transition_identity_and_cursor_reject_without_consuming_response() {
    let directory = tempdir().unwrap();
    let mut store = NolaStore::open(directory.path().join("nola.db")).unwrap();
    begin(&mut store);
    let before = store.load_checkpoint("conversation-1").unwrap().unwrap();
    let make_command = |cursor| AcceptHostEvent {
        conversation_id: "conversation-1".into(),
        run_id: "run-1".into(),
        response: response(
            1,
            HostResponse::ModelDelta {
                channel: ModelDeltaChannel::Text,
                part_id: "assistant-part-1".into(),
                delta: "Hello".into(),
            },
        ),
        events: vec![text_event(1, "Hello", false)],
        successor_requests: Vec::new(),
        checkpoint: serde_json::from_value(checkpoint(2, cursor)).unwrap(),
        received_at_ms: 2_000,
    };
    let error = accept_host_event(&mut store, make_command(u64::MAX)).unwrap_err();
    assert!(error.contains("cursor"), "{error}");
    for case in 0..14 {
        let mut command = make_command(1);
        match case {
            0 => command.response.session_id = "foreign-session".into(),
            1 => command.response.host_id = HostId("foreign-host".into()),
            2 => command.checkpoint.session_id = "foreign-session".into(),
            3 => {
                command.checkpoint.state.session_id =
                    SessionId::from_string("foreign-session".into())
            }
            4 => command.events[0].session_id = "foreign-session".into(),
            5 => {
                let mut successor = request();
                successor.session_id = "foreign-session".into();
                command.successor_requests.push(successor);
            }
            6 => command.response.lease_id = Some("foreign-host-lease".into()),
            7 => command.response.turn_id = Some("foreign-turn".into()),
            8 => command.response.catalog_revision += 1,
            9 => command.response.availability_epoch += 1,
            10 => command.checkpoint.revision += 1,
            11 => command.checkpoint.last_event_cursor.sequence += 1,
            12 => command.checkpoint.capability_catalog_revision += 1,
            13 => command.checkpoint.schema_version += 1,
            _ => unreachable!(),
        }
        let error = accept_host_event(&mut store, command).unwrap_err();
        assert!(
            error.contains("identity differs")
                || error.contains("another Core session")
                || error.contains("inconsistent")
                || error.contains("unsupported Agent checkpoint"),
            "case {case}: {error}"
        );
    }
    let after = store.load_checkpoint("conversation-1").unwrap().unwrap();
    assert_eq!(after.revision, before.revision);
    assert_eq!(after.checkpoint_json, before.checkpoint_json);
    assert_eq!(store.pending_effects("run-1").unwrap().len(), 1);
    // Same sequence is still admissible after the rejected transaction. The
    // in-process Host's optional Turn echo remains legal; foreign echoes above
    // are not equivalent to omission.
    let mut valid = make_command(1);
    valid.response.turn_id = None;
    assert!(accept_host_event(&mut store, valid).unwrap().applied);
}

#[test]
fn host_stream_events_commit_parts_checkpoint_and_terminal_ack_across_reopen() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("stream-terminal.db");
    let mut store = NolaStore::open(&database).unwrap();
    let fixture = begin_real_completion(&mut store);
    let streaming = store.load_conversation("recovery-conversation").unwrap().unwrap();
    assert!(streaming.messages[1].parts.iter().any(|part| part.content_text == "World" && part.state == "open"));
    drop(store);
    let mut store = NolaStore::open(&database).unwrap();
    // The terminal checkpoint/result must be produced by Core, not by a
    // handcrafted TurnCompleted envelope that bypasses result provenance.
    reduce_and_accept_host_response(&mut store, final_response(&fixture)).unwrap();
    drop(store);
    let store = NolaStore::open(&database).unwrap();
    assert_answer_seal(&store, &fixture);
    let seal = store.local_assistant_run_terminal_seal("recovery-conversation", &fixture.run).unwrap().unwrap();
    let page = store.read_local_assistant_run_terminal_manifest(
        &seal, nola_domain::assistant_journal_operations::JournalNumber(0), 64, 262144,
    ).unwrap();
    assert!(page.complete);
    seal.validate_manifest(&page.entries).unwrap();
    let mut forged = seal.clone();
    forged.identity.source_store_epoch = "another-source".into();
    forged.seal_digest = forged.computed_digest().unwrap();
    assert!(store.read_local_assistant_run_terminal_manifest(
        &forged, nola_domain::assistant_journal_operations::JournalNumber(0), 64, 262144,
    ).is_err());
    assert!(store.local_assistant_run_terminal_seal("another-conversation", &fixture.run).is_err());
    let completed = store.load_conversation("recovery-conversation").unwrap().unwrap();
    assert_eq!(completed.messages[1].status, "completed");
    assert!(completed.messages[1].parts.iter().any(|part| part.content_text == "World" && part.state == "completed"));
}

#[test]
fn unknown_terminal_effect_rolls_back_host_event_part_and_checkpoint() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("nola.db");
    let mut store = NolaStore::open(&database).unwrap();
    begin(&mut store);
    let mut invalid_response = response(
        1,
        HostResponse::ModelCompleted {
            items: Vec::new(),
            tool_calls: Vec::new(),
            usage: serde_json::json!({}),
        },
    );
    invalid_response.effect_id = HostEffectId("unknown-effect".to_string());
    assert!(accept_host_event(
        &mut store,
        AcceptHostEvent {
            conversation_id: "conversation-1".to_string(),
            run_id: "run-1".to_string(),
            response: invalid_response,
            events: vec![text_event(1, "must rollback", true)],
            successor_requests: Vec::new(),
            checkpoint: serde_json::from_value(checkpoint(2, 1)).unwrap(),
            received_at_ms: 2_000,
        },
    )
    .is_err());
    drop(store);

    let store = NolaStore::open(&database).unwrap();
    let conversation = store.load_conversation("conversation-1").unwrap().unwrap();
    assert!(conversation.messages[1].parts[0].content_text.is_empty());
    assert_eq!(conversation.messages[1].status, "queued");
    assert_eq!(
        store
            .load_checkpoint("conversation-1")
            .unwrap()
            .unwrap()
            .revision,
        1
    );
    assert_eq!(store.pending_effects("run-1").unwrap().len(), 1);
}

#[test]
fn creation_receipt_failure_rolls_back_all_facts_then_replays_after_restart() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("creation-receipt.db");
    let input = BeginAssistantTurn {
        expected_configuration_revision: None,
        expected_parent_message_id: None,
        conversation_id: "receipt-conversation".into(),
        conversation_title: "Receipt".into(),
        user_message_id: "receipt-user".into(),
        user_part_id: "receipt-user:body".into(),
        user_text: "Stream".into(),
        user_content_json: None,
        assistant_message_id: "receipt-submission:assistant".into(),
        context_checkpoint_id: None,
        core_config: AgentCoreConfig {
            turn_identity: support::model_identity::identity(),
            session_id: Some(SessionId::from_string("receipt-conversation".into())),
            ..runtime_config()
        },
        submission: AgentSubmission::UserInput {
            items: vec![AgentItem::user_message(
                ItemId::from_string("receipt-user".into()),
                "Stream",
            )],
            turn_context_items: Vec::new(),
            turn_context_platform: Default::default(),
            turn_context_components: Vec::new(),
            submission_id: Some("receipt-submission".into()),
            submitted_at_epoch_ms: Some(1_000),
            delivery: AgentInputDelivery::CurrentTurn,
        },
        created_at_ms: 1_000,
    };

    let command = ApplicationCommand::BeginAssistantTurn { input };
    let command_json = serde_json::to_string(&command).unwrap();
    let receipt_input = || nola_application::ExecuteReceivedCommand {
        client_id: "creation-client".into(),
        request_id: "creation-1".into(),
        command_type: "begin_assistant_turn".into(),
        command_json: command_json.clone(),
        expected_revision: None,
        received_at: "1000".into(),
    };
    let mut runtime = support::open_runtime(&database).unwrap();
    let inspect = rusqlite::Connection::open(&database).unwrap();
    let facts = || {
        [
            "conversations",
            "messages",
            "message_parts",
            "runs",
            "core_checkpoints",
            "effect_outbox",
            "host_event_inbox",
            "run_events",
            "committed_transitions",
            "sync_outbox",
            "projection_cursors",
            "projection_events",
            "command_receipts",
        ]
        .into_iter()
        .map(|table| {
            let mut stmt = inspect
                .prepare(&format!("SELECT * FROM {table} ORDER BY 1"))
                .unwrap();
            let count = stmt.column_count();
            stmt.query_map([], |row| {
                Ok((0..count)
                    .map(|i| format!("{:?}", row.get_ref(i).unwrap()))
                    .collect::<Vec<_>>())
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
        })
        .collect::<Vec<_>>()
    };
    let baseline = facts();
    inspect.execute_batch("CREATE TRIGGER fail_creation_receipt BEFORE UPDATE ON command_receipts WHEN NEW.status='committed' BEGIN SELECT RAISE(ABORT,'creation receipt fault'); END;").unwrap();
    let error = runtime
        .execute_received_command(
            receipt_input(),
            serde_json::from_str(&command_json).unwrap(),
        )
        .unwrap_err();
    assert!(
        error.to_string().contains("creation receipt fault"),
        "{error}"
    );
    assert_eq!(
        facts(),
        baseline,
        "creation savepoint must not commit receipt UoW"
    );
    inspect
        .execute_batch("DROP TRIGGER fail_creation_receipt;")
        .unwrap();
    let committed = runtime
        .execute_received_command(
            receipt_input(),
            serde_json::from_str(&command_json).unwrap(),
        )
        .unwrap();
    let committed_facts = facts();
    drop(runtime);
    let mut reopened = support::open_runtime(&database).unwrap();
    let replay = reopened
        .execute_received_command(
            receipt_input(),
            serde_json::from_str(&command_json).unwrap(),
        )
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.outcome, committed.outcome);
    assert_eq!(facts(), committed_facts);
    let store = NolaStore::open(&database).unwrap();
    let run = store.active_run("receipt-conversation").unwrap().unwrap();
    assert!(!load_committed_host_requests(&store, &run.run_id)
        .unwrap()
        .requests
        .is_empty());
}
