// SPDX-License-Identifier: Apache-2.0
#![allow(dead_code, unused_imports)]

use crate::remote_submission_from as remote_submission_fixture;
use crate::support;
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


#[path = "../../support/maintenance_run_lifecycle_cases.rs"]
mod maintenance_run_lifecycle_cases;

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
pub(super) struct CompletionFixture {
    pub(super) run: String,
    pub(super) digest: String,
    pub(super) request: HostRequestEnvelope,
}
pub(super) fn begin_real_completion(store: &mut NolaStore) -> CompletionFixture {
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
pub(super) fn final_response(fixture: &CompletionFixture) -> ReduceAndAcceptHostResponse {
    ReduceAndAcceptHostResponse {
        conversation_id: "recovery-conversation".into(), run_id: fixture.run.clone(),
        invocation_config_digest: fixture.digest.clone(), received_at_ms: 1300,
        response: response_for_request(&fixture.request, 2, HostResponse::ModelCompleted {
            items: vec![], tool_calls: vec![], usage: serde_json::json!({}),
        }),
    }
}
pub(super) fn assert_answer_seal(store: &NolaStore, fixture: &CompletionFixture) {
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
    let before_revision = store.load_checkpoint("recovery-conversation").unwrap().unwrap().revision;
    let pre_commit_facts = completion_facts(&store, &fixture);

    // 1. Wrong host effect ID rolls back every completion projection.
    let mut wrong_effect = final_response(&fixture);
    wrong_effect.response.effect_id = HostEffectId("wrong-effect".into());
    assert!(reduce_and_accept_host_response(&mut store, wrong_effect).is_err());
    assert_eq!(completion_facts(&store, &fixture), pre_commit_facts);

    // 2. Terminal seal write failure rolls back canonical completion and publication.
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER reject_terminal_seal BEFORE INSERT ON assistant_run_terminal_seals              BEGIN SELECT RAISE(ABORT,'fixture seal failure'); END;",
        )
        .unwrap();
    let seal_err = reduce_and_accept_host_response(&mut store, final_response(&fixture)).unwrap_err();
    assert!(seal_err.contains("fixture seal failure"), "{seal_err}");
    assert_eq!(completion_facts(&store, &fixture), pre_commit_facts);
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM assistant_run_terminal_manifest", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        0
    );
    connection.execute_batch("DROP TRIGGER reject_terminal_seal").unwrap();

    // 3. Real completion commits message, checkpoint, run, outbox, and immutable terminal seal.
    reduce_and_accept_host_response(&mut store, final_response(&fixture)).unwrap();
    assert_answer_seal(&store, &fixture);
    assert!(connection
        .execute("UPDATE assistant_run_terminal_seals SET seal_json='{}'", [])
        .is_err());
    assert!(connection
        .execute("UPDATE assistant_run_terminal_manifest SET entry_json='{}'", [])
        .is_err());
    drop(connection);
    drop(store);

    // 4. Reopen verifies durable answer seal and completed message state.
    let mut store = NolaStore::open(&database).unwrap();
    assert_answer_seal(&store, &fixture);
    let conversation = store.load_conversation("recovery-conversation").unwrap().unwrap();
    assert_eq!(conversation.messages.len(), 2);
    assert_eq!(conversation.messages[1].status, "completed");
    assert!(conversation.messages[1]
        .parts
        .iter()
        .any(|part| part.content_text == "World"));
    assert!(store.load_checkpoint("recovery-conversation").unwrap().unwrap().revision > before_revision);

    // 5. Terminal host replay is exact/idempotent, forged completion is rejected, and late deltas cannot reopen.
    let sealed_facts = completion_facts(&store, &fixture);
    let replay = reduce_and_accept_host_response(&mut store, final_response(&fixture)).unwrap();
    assert!(!replay.commit.applied);
    assert_eq!(completion_facts(&store, &fixture), sealed_facts);

    let mut forged = final_response(&fixture);
    forged.response.payload = HostResponse::ModelCompleted {
        items: vec![serde_json::to_value(AgentItem::assistant_message(
            ItemId::from_string("forged".into()),
            "Overwritten final text",
        ))
        .unwrap()],
        tool_calls: vec![],
        usage: serde_json::json!({}),
    };
    assert!(reduce_and_accept_host_response(&mut store, forged).is_err());
    assert_eq!(completion_facts(&store, &fixture), sealed_facts);

    let mut late = final_response(&fixture);
    late.response.sequence += 1;
    late.response.payload = HostResponse::ModelDelta {
        channel: ModelDeltaChannel::Text,
        part_id: match &fixture.request.payload {
            HostRequest::ModelStream { text_part_id, .. } => text_part_id.clone(),
            _ => panic!("expected model request"),
        },
        delta: "Late mutation".into(),
    };
    assert!(reduce_and_accept_host_response(&mut store, late).is_err());
    assert_eq!(completion_facts(&store, &fixture), sealed_facts);
}

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
