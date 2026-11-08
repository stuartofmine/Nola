// SPDX-License-Identifier: Apache-2.0
//! Real provider/tool boundaries, cold Application recovery and bounded writes.
//! SQL below observes WAL/counters only; all execution facts use public commands.
mod support;

use nola_application::{
    ApplicationCommand, BeginAssistantTurn, ClaimCommittedHostRequests, ReduceAndAcceptHostResponse,
};
use nola_agent_engine::agent::host::*;
use nola_agent_engine::agent::protocol::*;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn capability(id: &str, route: HostCapabilityRoute, padding: usize) -> HostCapabilityDescriptor {
    HostCapabilityDescriptor {
        id: id.into(),
        title: id.into(),
        route,
        availability: CapabilityAvailability::Available,
        exposure: if id == "model.stream" {
            HostCapabilityExposure::Internal
        } else {
            HostCapabilityExposure::Visible
        },
        permission_domain: None,
        input_schema: json!({"type":"object","description":"x".repeat(padding)}),
        input_normalization: HostInputNormalization {
            permission_contract:nola_domain::permission_resource_plan::PermissionOperationContract {
                risk:if id == "test.read" {nola_domain::permission_resource_plan::PermissionOperationRisk::OrdinaryRead}
                    else {nola_domain::permission_resource_plan::PermissionOperationRisk::Unknown},..Default::default()
            },..Default::default()
        },
        output_policy: HostOutputPolicy {
            preview_char_limit: 4000,
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

struct Fixture {
    runtime: support::ApplicationRuntime,
    path: PathBuf,
    run: String,
    digest: String,
    clock: i64,
    conversation: String,
    assistant: String,
    reopen_after_boundary: bool,
    claimed_effects: Vec<String>,
}

impl Fixture {
    fn declare_read_scope(&mut self, round: usize) {
        let mut model = self.claim();
        self.send(&mut model, HostResponse::ExecutionStarted, "scope model started");
        self.send(&mut model, HostResponse::ModelAttemptStarted {
            provider:"fixture-provider".into(), model:"fixture-model".into(), reason:"initial".into(),
        }, "scope model attempt");
        self.send(&mut model, HostResponse::ModelCompleted {
            items:vec![], tool_calls:vec![json!({"call_id":"declare-read", "name":"task_scope_update", "arguments":{
                "requirements":[{"key":"read","description":"Read the fixture resource","source_refs":["s1"],"dependencies":[],
                    "resource_scope":{"kind":"declared","required":[{"resource_uri":format!("file:///fixture/{round}"),
                        "operation":"read","source_refs":["s1"]}],"exclusions":[]}}],
                "dispositions":[{"kind":"replace","requirement_id":"r1","successors":[{"kind":"local_key","key":"read"}],
                    "reason":"Record exact fixture input","user_input_refs":["s1"]}]
            }})], usage:json!({}),
        }, "declare read scope");
    }

    fn new(path: &Path, padding: usize) -> Self {
        Self::named(path, padding, "conversation")
    }
    fn named(path: &Path, padding: usize, conversation: &str) -> Self {
        Self::configured(path, padding, conversation, None)
    }
    fn configured(
        path: &Path,
        padding: usize,
        conversation: &str,
        binding: Option<nola_host_protocol::HostInvocationBinding>,
    ) -> Self {
        Self::configured_with_recovery_limit(path, padding, conversation, binding, 3)
    }

    fn configured_with_recovery_limit(
        path: &Path,
        padding: usize,
        conversation: &str,
        binding: Option<nola_host_protocol::HostInvocationBinding>,
        recovery_limit: u32,
    ) -> Self {
        let user = format!("{conversation}:user");
        let assistant = format!("{conversation}:assistant");
        let mut runtime = support::open_runtime(path).unwrap();
        let mut config = AgentCoreConfig {
            max_strategy_recovery_attempts: recovery_limit,
            turn_identity: support::model_identity::identity(),
            session_id: Some(SessionId::from_string(format!("{conversation}:core"))),
            model: Some("fixture-model".into()),
            tools: vec![
                json!({"type":"function","function":{"name":"test.read","description":"read fixture","parameters":{"type":"object"}}}),
            ],
            host_catalog: Some(HostCapabilityCatalog {
                host_id: HostId("fixture-host".into()),
                revision: 1,
                availability_epoch: 1,
                lease: None,
                capabilities: [
                    (
                        "model.stream".into(),
                        capability("model.stream", HostCapabilityRoute::Model, padding),
                    ),
                    (
                        "test.read".into(),
                        capability("test.read", HostCapabilityRoute::Tool, 0),
                    ),
                ]
                .into_iter()
                .collect(),
            }),
            ..Default::default()
        };
        config.turn_identity.host_invocation = binding;
        let accepted: nola_application::AcceptedAssistantTurn = serde_json::from_value(
            runtime
                .execute(ApplicationCommand::BeginAssistantTurn {
                    input: BeginAssistantTurn {
                        expected_configuration_revision: None,
                        expected_parent_message_id: None,
                        conversation_id: conversation.into(),
                        conversation_title: "Replay boundaries".into(),
                        user_message_id: user.clone(),
                        user_part_id: format!("{user}:body"),
                        user_text: "Read fixtures".into(),
                        user_content_json: None,
                        assistant_message_id: assistant.clone(),
                        context_checkpoint_id: None,
                        core_config: config,
                        created_at_ms: 1000,
                        submission: AgentSubmission::UserInput {
                            items: vec![AgentItem::user_message(
                                ItemId::from_string(user),
                                "Read fixtures",
                            )],
                            turn_context_items: vec![],
                            turn_context_platform: Default::default(),
                            turn_context_components: vec![],
                            submission_id: Some(format!("{conversation}:submission")),
                            submitted_at_epoch_ms: Some(1000),
                            delivery: AgentInputDelivery::CurrentTurn,
                        },
                    },
                })
                .unwrap(),
        )
        .unwrap();
        Self {
            runtime,
            path: path.into(),
            run: accepted.begin.run_id,
            digest: accepted.invocation_config_digest,
            clock: 1100,
            conversation: conversation.into(),
            assistant,
            reopen_after_boundary: true,
            claimed_effects: Vec::new(),
        }
    }

    fn query_state(&mut self) -> (Value, Value, Value) {
        let checkpoint = self
            .runtime
            .execute(ApplicationCommand::LoadCheckpoint {
                conversation_id: self.conversation.clone(),
            })
            .unwrap();
        let active = self
            .runtime
            .execute(ApplicationCommand::ActiveRun {
                conversation_id: self.conversation.clone(),
            })
            .unwrap();
        let requests = self
            .runtime
            .execute(ApplicationCommand::LoadCommittedHostRequests {
                run_id: self.run.clone(),
            })
            .unwrap();
        if !active.is_null() {
            assert_eq!(active["run_id"], self.run);
            assert_eq!(active["assistant_message_id"], self.assistant);
        }
        (checkpoint, active, requests)
    }

    fn assert_cold_identity_and_cursor(&mut self, boundary: &str) {
        let before = self.query_state();
        self.runtime = support::open_runtime(&self.path).unwrap();
        let after = self.query_state();
        assert_eq!(
            before, after,
            "cold replay changed {boundary}: checkpoint/cursor/Run/effect identity"
        );
    }

    fn claim(&mut self) -> HostRequestEnvelope {
        self.clock += 10;
        let claimed: nola_application::CommittedHostRequests = serde_json::from_value(
            self.runtime
                .execute(ApplicationCommand::ClaimCommittedHostRequests {
                    input: ClaimCommittedHostRequests {
                        run_id: self.run.clone(),
                        worker_generation: "fixture-generation".into(),
                        lease_owner: "fixture-worker".into(),
                        claimed_at_ms: self.clock,
                        lease_expires_at_ms: 100_000,
                        limit: 1,
                    },
                })
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            claimed.requests.len(),
            1,
            "exactly one serial effect must be dispatchable"
        );
        let request = claimed.requests.into_iter().next().unwrap();
        self.claimed_effects.push(request.effect_id.0.clone());
        request
    }

    fn send(&mut self, request: &mut HostRequestEnvelope, payload: HostResponse, boundary: &str) {
        self.clock += 1;
        let control_boundary = !matches!(&payload, HostResponse::ModelDelta { .. });
        let sequence = request.resume_after_sequence.unwrap_or(0) + 1;
        let input = ReduceAndAcceptHostResponse {
            conversation_id: self.conversation.clone(),
            run_id: self.run.clone(),
            invocation_config_digest: self.digest.clone(),
            received_at_ms: self.clock,
            response: HostResponseEnvelope {
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
                sequence,
                payload,
            },
        };
        let command = ApplicationCommand::ReduceAndAcceptHostResponse { input };
        let envelope = nola_application::ExecuteReceivedCommand {
            client_id: "boundary-fixture".into(),
            request_id: format!("{}:{sequence}", request.request_id),
            command_type: "reduce_and_accept_host_response".into(),
            command_json: serde_json::to_string(&command).unwrap(),
            expected_revision: None,
            received_at: self.clock.to_string(),
        };
        let accepted = self
            .runtime
            .execute_received_command(
                envelope.clone(),
                serde_json::from_str(&envelope.command_json).unwrap(),
            )
            .unwrap();
        assert!(!accepted.replayed);
        assert!(
            matches!(
                &accepted.outcome,
                nola_application::ApplicationCommandReceiptOutcome::Committed { .. }
            ),
            "provider boundary was rejected: {:?}",
            accepted.outcome
        );
        if control_boundary {
            // Observe the actual durable receipt, not a stripped transport view.
            // New control responses must never retain the cumulative transcript.
            let connection = rusqlite::Connection::open(&self.path).unwrap();
            let stored: String = connection
                .query_row(
                    "SELECT result_json FROM command_receipts WHERE client_id=?1 AND request_id=?2",
                    rusqlite::params![envelope.client_id, envelope.request_id],
                    |row| row.get(0),
                )
                .unwrap();
            let result: Value = serde_json::from_str(&stored).unwrap();
            assert!(
                result.get("snapshot").is_none(),
                "{boundary} retained the old full snapshot"
            );
            assert!(
                stored.len() < 64 * 1024,
                "{boundary} receipt grew with cumulative text/tool output: {} bytes",
                stored.len()
            );
        }
        if self.reopen_after_boundary {
            self.assert_cold_identity_and_cursor(boundary);
        } else {
            // Exercise successive declarations under one hot owner. Reads
            // still prove durable recovery without replacing that owner.
            self.query_state();
        }
        let before_duplicate = self.query_state();
        let replayed = self
            .runtime
            .execute_received_command(envelope, command)
            .unwrap();
        assert!(replayed.replayed);
        assert_eq!(accepted.outcome, replayed.outcome);
        assert_eq!(
            before_duplicate,
            self.query_state(),
            "duplicate {boundary} emitted another effect or advanced the cursor"
        );
        request.resume_after_sequence = Some(sequence);
    }

    fn model_tool_cycle(&mut self, round: usize) {
        self.model_tool_cycle_with_content(round, 0, 0);
    }

    fn model_tool_cycle_with_content(
        &mut self,
        round: usize,
        text_repeats: usize,
        tool_repeats: usize,
    ) {
        let mut model = self.claim();
        let part_id = match &model.payload {
            HostRequest::ModelStream { text_part_id, .. } => text_part_id.clone(),
            other => panic!("expected model request, got {other:?}"),
        };
        self.send(
            &mut model,
            HostResponse::ExecutionStarted,
            "model execution start",
        );
        self.send(
            &mut model,
            HostResponse::ModelAttemptStarted {
                provider: "fixture-provider".into(),
                model: "fixture-model".into(),
                reason: "initial".into(),
            },
            "model attempt start",
        );
        self.send(
            &mut model,
            HostResponse::ModelDelta {
                channel: ModelDeltaChannel::Text,
                part_id,
                delta: format!("Round {round} 中🙂{}", "累计正文🙂".repeat(text_repeats)),
            },
            "model delta",
        );
        let call_id = format!("call-{round}");
        self.send(
            &mut model,
            HostResponse::ModelCompleted {
                items: vec![],
                // Exercise distinct work at each boundary, not the independent
                // repeated-identical-tool-batch safety guard.
                tool_calls: vec![json!({"call_id":call_id,"name":"test.read","arguments":{"round":round}})],
                usage: json!({}),
            },
            "model completion with tool calls",
        );
        let mut tool = self.claim();
        match &tool.payload {
            HostRequest::ExecuteTool {
                call_id: actual, ..
            } => assert_eq!(actual, &call_id),
            other => panic!("expected exact tool request, got {other:?}"),
        }
        self.send(
            &mut tool,
            HostResponse::ExecutionStarted,
            "tool execution start",
        );
        self.send(
            &mut tool,
            HostResponse::ToolCompleted {
                call_id,
                output: json!({"contract":"nola.tool_result.v1",
                    "data":{"content":format!("result {round} {}", "工具结果🙂".repeat(tool_repeats))},
                    "presentation":{"text":"Fixture read completed","summary":"Read completed"},
                    "execution":{"state":"completed","local_stop":null,"effects":[{
                        "resource_uri":format!("file:///fixture/{round}"),"operation":"read",
                        "before_version":format!("fixture-version-{round}"),
                        "after_version":format!("fixture-version-{round}")}],"checks":[]},
                    "full_output_ref":null,"memory_usage":null}),
                outcome: HostToolOutcome::Succeeded,
                activation: None,
                full_output_ref: None,
                artifacts: vec![],
            },
            "tool completion and next model",
        );
    }
}

#[test]
fn ordinary_and_core_control_sources_survive_cold_lineage_without_checkpoint_lookup() {
    use nola_agent_engine::agent::{recovery::CoreCheckpoint,AgentThreadState};
    use nola_store_port::tool_source::ToolSourceRepository;
    let directory=tempfile::tempdir().unwrap();
    let path=directory.path().join("tool-original-lineage.db");
    let mut fixture=Fixture::new(&path,0);
    fixture.declare_read_scope(0);
    fixture.model_tool_cycle(0);
    let mut model=fixture.claim();
    fixture.send(&mut model,HostResponse::ExecutionStarted,"source control execution start");
    fixture.send(&mut model,HostResponse::ModelAttemptStarted {
        provider:"fixture-provider".into(),model:"fixture-model".into(),reason:"initial".into(),
    },"source control attempt start");
    fixture.send(&mut model,HostResponse::ModelCompleted {
        items:vec![],tool_calls:vec![json!({"call_id":"source-control","name":"requirements_update",
            "arguments":{"outcome":"completed","answer":"The fixture result is reported.",
                "assessments":[{"requirement":"r1","verdict":"satisfied","rationale":"The actual read result is available.","evidence":["e1"]}]}})],usage:json!({}),
    },"source control completion");
    assert!(fixture.query_state().1.is_null());
    let checkpoint=support::current_checkpoint(&path,&fixture.conversation);
    let checkpoint:CoreCheckpoint<AgentThreadState>=serde_json::from_str(&checkpoint.checkpoint_json).unwrap();
    let expected:Vec<_>=checkpoint.state.conversation.transcript.iter().filter(|item|
        matches!(item,AgentItem::ToolCall {..}|AgentItem::ToolResult {..})).cloned().collect();
    assert_eq!(expected.len(),6,"one ordinary pair and scope/result Core control pairs");
    let source_ids:Vec<_>=expected.iter().map(|item|item.id().as_str().to_owned()).collect();
    drop(fixture);
    // The lineage lookup is a formal source read, not execution recovery. Make
    // the latest checkpoint unavailable in this owned fixture to prove that it
    // cannot be an accidental body source. Keep all immutable Part bindings.
    let db=rusqlite::Connection::open(&path).unwrap();
    db.execute("UPDATE core_checkpoint_bodies SET checkpoint_json='{}'",[]).unwrap();
    let next:i64=db.query_row("SELECT COALESCE(MAX(ordinal),0)+1 FROM messages WHERE conversation_id='conversation'",[],|r|r.get(0)).unwrap();
    db.execute("INSERT INTO messages VALUES('lineage-message','conversation',NULL,'assistant','completed',?1,2000,2000)",[next]).unwrap();
    db.execute("INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,content_json,created_at_ms,updated_at_ms)
        VALUES('lineage-compaction','lineage-message','compaction',1,'completed','summary',?1,2000,2000)",
        [json!({"retained_item_ids":source_ids,"source_fingerprint":"a".repeat(64)}).to_string()]).unwrap();
    db.execute("UPDATE conversations SET revision=revision+1 WHERE id='conversation'",[]).unwrap();
    drop(db);
    let store=nola_store::NolaStore::open(&path).unwrap();
    let revision=store.load_conversation("conversation").unwrap().unwrap().revision;
    for item in &expected {
        let binding=store.tool_source_by_id("conversation",item.id().as_str(),revision).unwrap().unwrap();
        assert_eq!(binding.owner.owner.conversation_id,"conversation");
        let mut offset=0;
        let mut original=String::new();
        loop {
            let page=store.read_tool_source_range(&binding,revision,offset,32).unwrap();
            assert_eq!(page.offset_bytes,offset);
            assert_eq!(page.next_offset_bytes,offset+page.text.len() as u64);
            offset=page.next_offset_bytes;
            original.push_str(&page.text);
            if page.end_of_content { break; }
        }
        assert_eq!(serde_json::from_str::<AgentItem>(&original).unwrap(),*item);
        assert!(store.tool_source_by_id("conversation",item.id().as_str(),revision-1).is_err());
    }
    let lineage=nola_application::load_compaction_lineage(&store,"conversation",Some("lineage-compaction")).unwrap();
    assert_eq!(lineage["missing_source_item_ids"],json!([]));
    assert_eq!(lineage["source_item_ids"],json!(source_ids));
    assert_eq!(lineage["sources"].as_array().unwrap().len(),6);
    assert!(lineage["sources"].as_array().unwrap().iter().all(|source|source["identity_kind"]=="tool_item"));
    assert!(lineage["sources"].as_array().unwrap().iter().any(|source|
        source["original_item_preview"]["type"]=="tool_result" && source["original_item_preview"]["name"]=="requirements_update"));
}

#[test]
fn compact_result_with_observed_evidence_seals_across_cold_and_duplicate_delivery() {
    for outcome in ["completed", "incomplete"] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("compact-result.db");
        let mut fixture = Fixture::new(&path, 0);
        fixture.declare_read_scope(0);
        fixture.model_tool_cycle(0);
        let mut model = fixture.claim();
        let frozen = match &model.payload {
            HostRequest::ModelStream { settings, .. } => settings["requirement_binding"].clone(),
            _ => panic!("expected model"),
        };
        assert!(frozen["evidence"]["e1"].is_string());
        fixture.send(&mut model, HostResponse::ExecutionStarted, "compact model started");
        fixture.send(&mut model, HostResponse::ModelAttemptStarted {
            provider: "fixture-provider".into(), model: "fixture-model".into(), reason: "initial".into(),
        }, "compact model attempt");
        fixture.send(&mut model, HostResponse::ModelCompleted {
            items: vec![],
            tool_calls: vec![json!({"call_id":"result-control","name":"requirements_update",
                "arguments":{"outcome":outcome,"answer":"The observed result is reported.",
                    "assessments":[{"requirement":"r1",
                        "verdict":if outcome == "completed" { "satisfied" } else { "insufficient_evidence" },
                        "rationale":"Judged from the observed tool result.","evidence":["e1"]}]}})],
            usage: json!({}),
        }, "compact result with evidence");
        let (_, active, requests) = fixture.query_state();
        assert!(active.is_null(), "result did not settle: active={active}; requests={requests}");
        assert!(requests["requests"].as_array().unwrap().is_empty());
        let seal: Option<nola_domain::assistant_run_terminal::AssistantRunTerminalSeal> = serde_json::from_value(
            fixture.runtime.execute(ApplicationCommand::LocalAssistantRunTerminalSeal {
                conversation_id: fixture.conversation.clone(), run_id: fixture.run.clone(),
            }).unwrap()).unwrap();
        let seal = seal.expect("compact result must persist a terminal seal");
        assert_eq!(serde_json::to_value(&seal.result.value).unwrap()["kind"],
            if outcome == "completed" { "action_completed" } else { "incomplete" });
        let db = rusqlite::Connection::open(&path).unwrap();
        let raw: String = db.query_row(
            "SELECT payload_json FROM run_events WHERE run_id=?1 AND event_kind='requirement_assessed'",
            [&fixture.run], |row| row.get(0)).unwrap();
        let assessment: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(assessment["assessment"]["request_binding"], frozen);
        assert_eq!(assessment["assessment"]["evidence_refs"], json!([frozen["evidence"]["e1"]]));
    }
}

#[test]
fn compact_task_result_assessments_commit_and_survive_cold_reopen() {
    compact_task_result_fixture(false);
}

#[test]
fn compact_task_result_rejects_tampered_frozen_request_atomically() {
    compact_task_result_fixture(true);
}

fn compact_task_result_fixture(tamper: bool) {
    use nola_store_port::assistant_journal::AssistantJournalRepository;
    use nola_store_port::run_decision::{RunDecisionQuery, RunDecisionRepository};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("compact-task-result.db");
    let mut fixture = Fixture::new(&path, 0);
    // Generate actual committed execution evidence, rather than inventing eN
    // references or manufacturing a checkpoint containing successful work.
    fixture.declare_read_scope(1);
    fixture.model_tool_cycle(1);
    let mut model = fixture.claim();
    fixture.send(&mut model, HostResponse::ExecutionStarted, "result execution start");
    fixture.send(&mut model, HostResponse::ModelAttemptStarted {
        provider: "fixture-provider".into(), model: "fixture-model".into(), reason: "initial".into(),
    }, "result attempt start");
    let binding = match &model.payload {
        HostRequest::ModelStream { settings, .. } => settings["requirement_binding"].clone(),
        other => panic!("expected result model request, got {other:?}"),
    };
    let evidence: Vec<_> = binding["evidence"].as_object().unwrap().keys().cloned().collect();
    assert!(!evidence.is_empty());
    let assessments: Vec<_> = binding["requirements"].as_object().unwrap().keys().map(|reference|
        json!({"requirement":reference,"verdict":"satisfied",
            "rationale":"The requested fixture was read successfully.","evidence":evidence})
    ).collect();
    assert!(!assessments.is_empty());
    let payload = HostResponse::ModelCompleted {
        items: vec![], tool_calls: vec![json!({"call_id":"final-result","name":"requirements_update",
            "arguments":{"outcome":"completed","answer":"Read the fixture successfully.","assessments":assessments}})],
        usage: json!({}),
    };
    if tamper {
        // Corruption is confined to this isolated fixture. The persisted digest
        // must reject even an unrelated mutation before publishing completion.
        let db = rusqlite::Connection::open(&path).unwrap();
        let raw: String = db.query_row("SELECT payload_json FROM effect_outbox_payloads WHERE effect_id=?1",
            [&model.effect_id.0], |row| row.get(0)).unwrap();
        let before: i64 = db.query_row("SELECT COUNT(*) FROM run_requirement_sources WHERE kind='requirement_assessed'",
            [], |row| row.get(0)).unwrap();
        db.execute("UPDATE effect_outbox_payloads SET payload_json=json_set(payload_json,'$.payload.settings.requirement_binding.scope_id','forged') WHERE effect_id=?1",
            [&model.effect_id.0]).unwrap();
        let result = fixture.runtime.execute(ApplicationCommand::ReduceAndAcceptHostResponse {
            input: ReduceAndAcceptHostResponse {
                conversation_id: fixture.conversation.clone(), run_id: fixture.run.clone(),
                invocation_config_digest: fixture.digest.clone(), received_at_ms: fixture.clock + 1,
                response: HostResponseEnvelope {
                    attempt_id: model.attempt_id.clone(), delegated_invocation_id: model.delegated_invocation_id.clone(),
                    request_id: model.request_id.clone(), effect_id: model.effect_id.clone(), host_id: model.host_id.clone(),
                    lease_id: model.lease_id.clone(), session_id: model.session_id.clone(), turn_id: model.turn_id.clone(),
                    catalog_revision: model.catalog_revision, availability_epoch: model.availability_epoch,
                    delivery: model.delivery.clone(), sequence: model.resume_after_sequence.unwrap_or(0) + 1,
                    payload: payload.clone(),
                },
            },
        });
        assert!(result.is_err(), "corrupted frozen request was trusted");
        let after: i64 = db.query_row("SELECT COUNT(*) FROM run_requirement_sources WHERE kind='requirement_assessed'",
            [], |row| row.get(0)).unwrap();
        assert_eq!(before, after, "rejected completion left partial assessments");
        db.execute("UPDATE effect_outbox_payloads SET payload_json=?1 WHERE effect_id=?2",
            rusqlite::params![raw, model.effect_id.0]).unwrap();
        fixture.assert_cold_identity_and_cursor("after rejected result");
    }
    fixture.send(&mut model, payload, "compact final result");
    let store = nola_store::NolaStore::open(&path).unwrap();
    let seal = store.local_assistant_run_terminal_seal(&fixture.conversation, &fixture.run)
        .unwrap().expect("compact result must seal the Run");
    assert!(matches!(&seal.result.value, nola_domain::run_result::RunResultValue::ActionCompleted { .. }));
    assert!(store.active_run(&fixture.conversation).unwrap().is_none());
    drop(store);
    // Normal open performs settled-outbox maintenance. The terminal decision
    // must still be independently auditable, not merely readable as seal JSON.
    let reopened = nola_store::NolaStore::open(&path).unwrap();
    assert_eq!(reopened.local_assistant_run_terminal_seal(&fixture.conversation, &fixture.run).unwrap(), Some(seal.clone()));
    assert!(reopened.load_run_decision(&RunDecisionQuery {
        conversation_id: fixture.conversation.clone(), run_id: fixture.run.clone(),
        turn_id: model.turn_id.clone().unwrap(),
        core_revision: seal.result.decided_at_revision.0,
        core_event_cursor: seal.result.decision_event_cursor.0,
    }).unwrap().is_some());
    let db = rusqlite::Connection::open(&path).unwrap();
    let retained: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM effect_outbox e JOIN effect_outbox_payloads p USING(effect_id) WHERE e.effect_id=?1 AND e.status='acknowledged')",
        [&model.effect_id.0], |row| row.get(0)).unwrap();
    assert!(retained, "maintenance deleted the frozen result request");
    let count: usize = db.query_row("SELECT COUNT(*) FROM run_requirement_sources WHERE run_id=?1 AND kind='requirement_assessed'",
        [&fixture.run], |row| row.get(0)).unwrap();
    assert_eq!(count, assessments.len());
    if tamper {
        // A persisted semantic binding cannot hide later corruption of the
        // retained request. Cold provenance audits must check its digest too.
        let raw: String = db.query_row("SELECT payload_json FROM effect_outbox_payloads WHERE effect_id=?1",
            [&model.effect_id.0], |row| row.get(0)).unwrap();
        db.execute("UPDATE effect_outbox_payloads SET payload_json=json_set(payload_json,'$.payload.settings.requirement_binding.scope_id','forged-after-seal') WHERE effect_id=?1",
            [&model.effect_id.0]).unwrap();
        let query = RunDecisionQuery {
            conversation_id: fixture.conversation.clone(), run_id: fixture.run.clone(),
            turn_id: model.turn_id.clone().unwrap(),
            core_revision: seal.result.decided_at_revision.0,
            core_event_cursor: seal.result.decision_event_cursor.0,
        };
        assert!(reopened.load_run_decision(&query).is_err(), "cold audit trusted a corrupted frozen request");
        db.execute("UPDATE effect_outbox_payloads SET payload_json=?1 WHERE effect_id=?2",
            rusqlite::params![raw, model.effect_id.0]).unwrap();
        assert!(reopened.load_run_decision(&query).unwrap().is_some());
    }
}

#[test]
fn recovery_budget_stop_seals_on_tool_completion_with_cold_and_duplicate_delivery() {
    for (limit, last_tool_failed) in [(0, false), (3, false), (3, true)] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("recovery-stop.db");
        let mut fixture = Fixture::configured_with_recovery_limit(&path, 0, "recovery-stop", None, limit);
        // Fixture::send cold-reopens and replays each exact received command,
        // asserting stable cursor, effects and receipt at every boundary.
        for round in 0..(3 + limit) {
            let mut model = fixture.claim();
            assert!(matches!(model.payload, HostRequest::ModelStream { .. }));
            fixture.send(&mut model, HostResponse::ExecutionStarted, "recovery model started");
            fixture.send(&mut model, HostResponse::ModelAttemptStarted {
                provider: "fixture-provider".into(), model: "fixture-model".into(), reason: "initial".into(),
            }, "recovery model attempt");
            let call_id = format!("recovery-call-{round}");
            fixture.send(&mut model, HostResponse::ModelCompleted {
                items: vec![], tool_calls: vec![json!({"call_id":call_id,"name":"test.read",
                    "arguments":{"candidate":if round < 3 { 0 } else { round }}})], usage: json!({}),
            }, "recovery model completed");
            if round == 2 { continue; } // identical third call is never executed
            let mut tool = fixture.claim();
            assert!(matches!(tool.payload, HostRequest::ExecuteTool { .. }));
            fixture.send(&mut tool, HostResponse::ExecutionStarted, "recovery tool started");
            fixture.send(&mut tool, HostResponse::ToolCompleted {
                call_id, output: json!({"content":"known execution result"}),
                outcome: if last_tool_failed && round + 1 == 3 + limit {
                    HostToolOutcome::Failed
                } else { HostToolOutcome::Succeeded },
                activation: None, full_output_ref: None, artifacts: vec![],
            }, "recovery tool completed");
        }
        let (_, active, requests) = fixture.query_state();
        assert!(active.is_null());
        assert!(requests["requests"].as_array().unwrap().is_empty());
        let seal: Option<nola_domain::assistant_run_terminal::AssistantRunTerminalSeal> = serde_json::from_value(
            fixture.runtime.execute(ApplicationCommand::LocalAssistantRunTerminalSeal {
                conversation_id: fixture.conversation.clone(), run_id: fixture.run.clone(),
            }).unwrap()).unwrap();
        let seal = seal.expect("runtime policy stop must persist a terminal seal");
        assert!(matches!(&seal.result.value,
            nola_domain::run_result::RunResultValue::Incomplete { reason, .. } if reason.code == "execution.no_progress"));
        let db = rusqlite::Connection::open(&path).unwrap();
        let facts: Vec<String> = db.prepare("SELECT payload_json FROM run_events WHERE run_id=?1 AND event_kind='runtime_decision_fact_observed'")
            .unwrap().query_map([&fixture.run], |row| row.get(0)).unwrap().collect::<Result<_,_>>().unwrap();
        let stops: Vec<Value> = facts.iter().map(|raw| serde_json::from_str::<Value>(raw).unwrap())
            .filter(|event| event["fact"]["kind"]["kind"] == "recovery_stopped").collect();
        assert_eq!(stops.len(), 1);
        assert_eq!(stops[0]["fact"]["kind"]["cause"]["limit"], limit);
        let original_response: String = db.query_row(
            "SELECT reduction_json FROM host_event_inbox WHERE run_id=?1 ORDER BY rowid DESC LIMIT 1",
            [&fixture.run], |row| row.get(0)).unwrap();
        assert!(original_response.contains(if limit == 0 { "model_completed" } else { "tool_completed" }));
    }
}

#[test]
fn all_cold_readers_materialize_the_same_checkpoint_and_exact_digest() {
    use sha2::{Digest, Sha256};
    let route = json!({"kind":"http","protocol":"chat_completions",
        "output_token_field":"max_tokens","provider":"fixture",
        "endpoint":"https://fixture.invalid/v1","endpoint_query_sha256":null,
        "model":"fixture-model","credential_sha256":"a".repeat(64),
        "credential_source":{"kind":"ephemeral"},"endpoint_query_source":{"kind":"none"}});
    let binding = serde_json::from_value(json!({
        "version":nola_host_protocol::HOST_INVOCATION_BINDING_VERSION,
        "primary":route,"fallbacks":[],
        "generation":{"temperature":0,"top_p":0,"max_output_tokens":16384,
            "max_retries":2,"parallel_tool_calls":null,"reasoning_selection":null,
            "reasoning_tier":"balanced",
            "sampling_override":false,"generation_options":{},"thinking_enabled":false,
            "prompt_cache":false,"prompt_cache_key_sha256":null,
            "prompt_cache_key_prefix_sha256":null,"prompt_cache_retention":null},
        "output_admission":{"version":nola_host_protocol::HOST_OUTPUT_ADMISSION_VERSION,
            "routes":[{"route":route,"max_input_tokens":null,"max_output_tokens":null,"max_total_tokens":null}],
            "preference":"default","resolved":{"tokens":16384,"source":"product_default"}},
        "execution_policy_sha256":"b".repeat(64)
    })).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("exact-materialization.db");
    let mut fixture = Fixture::configured(&path, 0, "exact-materialization", Some(binding));
    fixture.reopen_after_boundary = false;
    for round in 0..2 {
        fixture.model_tool_cycle(round);
        for cold in [false, true] {
            if cold {
                fixture.runtime = support::open_runtime(&path).unwrap();
            }
            let stored: nola_store_port::run_effect::StoredCheckpoint = serde_json::from_value(
                fixture
                    .runtime
                    .execute(ApplicationCommand::LoadCheckpoint {
                        conversation_id: fixture.conversation.clone(),
                    })
                    .unwrap(),
            )
            .unwrap();
            let checkpoint: nola_agent_engine::agent::recovery::CoreCheckpoint<
                nola_agent_engine::agent::AgentThreadState,
            > = serde_json::from_str(&stored.checkpoint_json).unwrap();
            assert_ne!(
                checkpoint.reason,
                nola_agent_engine::agent::recovery::CheckpointReason::Manual
            );
            let exact: nola_application::ExactRunInvocation = serde_json::from_value(
                fixture
                    .runtime
                    .execute(ApplicationCommand::LoadExactRunInvocation {
                        conversation_id: fixture.conversation.clone(),
                        expected_run_id: fixture.run.clone(),
                        assistant_message_id: fixture.assistant.clone(),
                    })
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(
                exact.checkpoint_sha256,
                Some(format!(
                    "{:x}",
                    Sha256::digest(stored.checkpoint_json.as_bytes())
                ))
            );
            assert_eq!(exact.checkpoint_revision, Some(stored.revision));
            assert_eq!(exact.checkpoint_event_cursor, Some(stored.event_cursor));
            // A presentation query warms the snapshot-bound recovery memo.
            // The exact invocation proof must be identical on that read path.
            fixture.runtime.execute(ApplicationCommand::LoadConversationProjectionWindowV1 {
                conversation_id: fixture.conversation.clone(), before_ordinal: None,
                limit: 64, content_byte_limit: 64 * 1024,
            }).unwrap();
            let warm = fixture.runtime.execute(ApplicationCommand::LoadExactRunInvocation {
                conversation_id: fixture.conversation.clone(), expected_run_id: fixture.run.clone(),
                assistant_message_id: fixture.assistant.clone(),
            }).unwrap();
            assert_eq!(warm, serde_json::to_value(&exact).unwrap());
            assert_eq!(
                exact.core_session_id.as_deref(),
                Some(checkpoint.session_id.as_str())
            );
            let projection: nola_application::SessionProjection = serde_json::from_value(
                fixture
                    .runtime
                    .execute(ApplicationCommand::LoadSessionProjection {
                        conversation_id: fixture.conversation.clone(),
                    })
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(projection.store_revision, stored.revision);
            assert_eq!(projection.core_revision, checkpoint.revision);
            assert_eq!(
                projection.event_cursor,
                checkpoint.last_event_cursor.sequence
            );
            assert_eq!(
                projection.checkpoint_created_at_ms,
                checkpoint.created_at_ms
            );
        }
    }
}

#[test]
fn warm_new_effects_record_prepared_attempt_and_delivery_before_sampling() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("warm-bindings.db");
    let mut fixture = Fixture::new(&path, 0);
    fixture.reopen_after_boundary = false;
    for round in 0..3 {
        fixture.model_tool_cycle(round);
    }
    fixture.assert_cold_identity_and_cursor("continuous warm model/tool cycles");
    let db = rusqlite::Connection::open(&path).unwrap();
    let mut statement = db.prepare(
        "SELECT reduction_json FROM host_event_inbox WHERE run_id=?1 AND reduction_json IS NOT NULL",
    ).unwrap();
    let records = statement
        .query_map([&fixture.run], |row| row.get::<_, String>(0))
        .unwrap();
    let mut starts = 0;
    for record in records {
        let fact: nola_store_port::host_replay::HostReplayReduction =
            serde_json::from_str(&record.unwrap()).unwrap();
        let response: HostResponseEnvelope =
            serde_json::from_str(fact.normalized_response().get()).unwrap();
        if !matches!(response.payload, HostResponse::Accepted) || response.attempt_id.is_none() {
            continue;
        }
        let nola_store_port::host_replay::HostReplayReduction::Replayable {
            binding: Some(binding),
            ..
        } = fact
        else {
            panic!("prepared model execution must commit its actual binding before sampling");
        };
        assert_eq!(binding.attempt_id, response.attempt_id);
        assert_eq!(binding.delivery, response.delivery);
        assert_eq!(binding.effect_id, response.effect_id);
        assert_eq!(binding.request_id, response.request_id);
        assert!(
            !binding.attempt_started,
            "execution start is not sampling admission"
        );
        starts += 1;
    }
    assert_eq!(starts, 3);
}

#[test]
fn durable_boundary_receipts_do_not_copy_accumulated_text_or_tool_results() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("large-public-bodies.db");
    let mut fixture = Fixture::new(&path, 0);
    // Over one MiB of actual public text/tool data, not padding hidden in an
    // internal capability schema. Every boundary checks its persisted receipt
    // and independently compares hot/cold state and duplicate-command replay.
    for round in 0..3 {
        fixture.model_tool_cycle_with_content(round, 16_384, 16_384);
    }
    let connection = rusqlite::Connection::open(&path).unwrap();
    let (count, largest): (i64, i64) = connection
        .query_row(
            "SELECT count(*),max(length(CAST(result_json AS BLOB))) FROM command_receipts
         WHERE client_id='boundary-fixture'
           AND json_extract(command_json,'$.input.response.payload.type') <> 'model_delta'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert!(
        count >= 15,
        "all non-delta receipt boundaries must be observed"
    );
    assert!(largest < 64 * 1024);
}

fn boundary_wal(padding: usize) -> u64 {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("boundaries.db");
    let wal = directory.path().join("boundaries.db-wal");
    let mut fixture = Fixture::new(&path, padding);
    let reader = rusqlite::Connection::open(&path).unwrap();
    let busy: i64 = reader
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))
        .unwrap();
    assert_eq!(busy, 0);
    reader.execute_batch("BEGIN").unwrap();
    reader
        .query_row("SELECT count(*) FROM runs", [], |row| row.get::<_, i64>(0))
        .unwrap();
    let start = std::fs::metadata(&wal).map(|file| file.len()).unwrap_or(0);
    for round in 0..6 {
        fixture.model_tool_cycle(round);
    }
    let growth = std::fs::metadata(&wal).unwrap().len() - start;
    reader.execute_batch("ROLLBACK").unwrap();
    assert!(growth > 0);
    let state = fixture.query_state();
    let pending: nola_application::CommittedHostRequests = serde_json::from_value(state.2).unwrap();
    assert_eq!(pending.requests.len(), 1);
    assert!(matches!(
        pending.requests[0].payload,
        HostRequest::ModelStream { .. }
    ));
    let effects: (i64, i64) = reader
        .query_row(
            "SELECT count(*),count(DISTINCT effect_id) FROM effect_outbox WHERE run_id=?1",
            [&fixture.run],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        effects,
        (1, 1),
        "cold reopening prunes acknowledged outbox rows, retaining only the next model"
    );
    // Delivery history, not the prunable dispatch queue, proves completed work.
    // Compare identities observed through actual public claims so duplicate
    // delivery or an unrelated historical effect cannot satisfy a mere count.
    let mut statement = reader
        .prepare("SELECT effect_id,state FROM effect_delivery_attempts WHERE run_id=?1")
        .unwrap();
    let deliveries: Vec<(String, String)> = statement
        .query_map([&fixture.run], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(deliveries.len(), 12);
    assert!(deliveries.iter().all(|(_, state)| state == "settled"));
    let delivered: std::collections::BTreeSet<_> =
        deliveries.into_iter().map(|(effect, _)| effect).collect();
    assert_eq!(fixture.claimed_effects.len(), 12);
    let claimed: std::collections::BTreeSet<_> = fixture.claimed_effects.into_iter().collect();
    assert_eq!(claimed.len(), 12, "each model/tool effect is claimed once");
    assert_eq!(delivered, claimed);
    assert!(
        !delivered.contains(&pending.requests[0].effect_id.0),
        "the thirteenth effect is a new model request, not a replay of settled work"
    );
    growth
}

#[test]
fn every_nonterminal_model_and_tool_boundary_cold_replays_without_payload_sized_writes() {
    let small = boundary_wal(4096);
    let large = boundary_wal(2 * 1024 * 1024);
    assert!(large <= small + 8 * 1024 * 1024,
        "frozen configuration multiplied across model/tool boundaries: small={small}, large={large}");
    assert!(
        large < 32 * 1024 * 1024,
        "36 nonterminal response commits exceeded WAL budget: {large}"
    );
}

#[test]
fn cold_cancellation_preserves_the_actual_inflight_effect_target() {
    cold_cancellation_fixture(false);
}

#[test]
fn cold_cancellation_uses_original_prompt_after_renderer_changes() {
    cold_cancellation_fixture(true);
}

fn cold_cancellation_fixture(changed_renderer: bool) {
    let directory = tempfile::tempdir().unwrap();
    let mut fixture = Fixture::new(&directory.path().join("cancel.db"), 4096);
    let mut model = fixture.claim();
    fixture.send(
        &mut model,
        HostResponse::ExecutionStarted,
        "model before cancel",
    );
    fixture.send(
        &mut model,
        HostResponse::ModelAttemptStarted {
            provider: "fixture-provider".into(),
            model: "fixture-model".into(),
            reason: "initial".into(),
        },
        "sampling before cancel",
    );
    if changed_renderer {
        use sha2::{Digest, Sha256};
        // Isolated fixture represents an input committed by an older renderer.
        // Change its prompt and its matching integrity record together; ordinary
        // corruption without an updated digest is covered by the negative tests.
        let db = rusqlite::Connection::open(&fixture.path).unwrap();
        let raw: String = db.query_row("SELECT payload_json FROM effect_outbox_payloads WHERE effect_id=?1",
            [&model.effect_id.0], |row| row.get(0)).unwrap();
        let mut frozen: Value = serde_json::from_str(&raw).unwrap();
        frozen["payload"]["messages"][0]["content"] = json!("Original frozen system prompt, not today's renderer");
        let frozen = serde_json::to_string(&frozen).unwrap();
        db.execute("UPDATE effect_outbox_payloads SET payload_json=?1 WHERE effect_id=?2",
            rusqlite::params![frozen, model.effect_id.0]).unwrap();
        db.execute("UPDATE effect_outbox SET payload_digest=?1 WHERE effect_id=?2",
            rusqlite::params![format!("{:x}", Sha256::digest(frozen.as_bytes())), model.effect_id.0]).unwrap();
        fixture.runtime = support::open_runtime(&fixture.path).unwrap();
        fixture.assert_cold_identity_and_cursor("original prompt after renderer change");
    }
    fixture.clock += 1;
    fixture
        .runtime
        .execute(ApplicationCommand::RequestRunCancellation {
            input: nola_application::RequestRunCancellation {
                conversation_id: "conversation".into(),
                expected_run_id: fixture.run.clone(),
                assistant_message_id: fixture.assistant.clone(),
                cancellation_id: "exact-cancel".into(),
                reason: None,
                requested_at_ms: fixture.clock,
            },
        })
        .unwrap();
    // Cancellation evidence queries must not replay the entire stream while a
    // newly claimed stop lease expires. Interrupt commits a normal baseline.
    let observer = nola_store::NolaStore::open(&fixture.path).unwrap();
    let recovery = nola_store_port::run_effect::RunEffectRepository::load_recovery_checkpoint(
        &observer, "conversation").unwrap().unwrap();
    assert!(recovery.replay_inputs.is_empty(), "Interrupt must materialize its durable frontier");
    assert_eq!(recovery.replay_base.as_ref().unwrap().checkpoint_revision,
        recovery.checkpoint.revision);
    fixture.assert_cold_identity_and_cursor("cancel requested before acknowledgement");
    let mut cancel = fixture.claim();
    let target = match &cancel.payload {
        HostRequest::Cancel { target_effect_id, .. } => target_effect_id.clone(),
        other => panic!("expected actual cancellation effect, got {other:?}"),
    };
    assert_eq!(target, model.effect_id);
    fixture.send(
        &mut cancel,
        HostResponse::ExecutionStarted,
        "cancel execution start",
    );
    fixture.send(
        &mut cancel,
        HostResponse::CancellationConfirmed {
            target_effect_id: target,
        },
        "cancel acknowledgement",
    );
    assert!(fixture.query_state().1.is_null());
    let seal: Option<nola_domain::assistant_run_terminal::AssistantRunTerminalSeal> =
        serde_json::from_value(
            fixture
                .runtime
                .execute(ApplicationCommand::LocalAssistantRunTerminalSeal {
                    conversation_id: "conversation".into(),
                    run_id: fixture.run.clone(),
                })
                .unwrap(),
        )
        .unwrap();
    let seal = seal.expect("a real cancellation confirmation must seal the original Run");
    assert_eq!(seal.identity.run_id, fixture.run);
    assert_eq!(seal.identity.assistant_message_id, fixture.assistant);
    assert!(matches!(
        &seal.result.value,
        nola_domain::run_result::RunResultValue::Cancelled { cancellation_id, .. }
            if cancellation_id == "exact-cancel"
    ));
}

#[test]
fn corrupt_current_replay_requires_recovery_without_blocking_another_real_session() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("corrupt-recovery.db");
    let mut ambiguous = Fixture::named(&path, 4096, "ambiguous");
    let mut model = ambiguous.claim();
    ambiguous.send(
        &mut model,
        HostResponse::ExecutionStarted,
        "current fixture execution",
    );
    ambiguous.send(
        &mut model,
        HostResponse::ModelAttemptStarted {
            provider: "fixture-provider".into(),
            model: "fixture-model".into(),
            reason: "initial".into(),
        },
        "current fixture sampling",
    );
    let healthy = Fixture::named(&path, 4096, "healthy");
    let healthy_run = healthy.run.clone();
    let healthy_digest = healthy.digest.clone();
    drop(healthy);
    let run = ambiguous.run.clone();
    drop(ambiguous);
    let db = rusqlite::Connection::open(&path).unwrap();
    // Corrupt only this Run's current replay records. An obsolete database
    // schema is rejected at open and is not a per-session recovery fixture.
    let damaged = db.execute(
        "UPDATE host_event_inbox SET reduction_json=?2 WHERE run_id=?1",
        rusqlite::params![run, r#"{"corrupt":true}"#],
    ).unwrap();
    assert!(damaged > 0);
    drop(db);
    let mut runtime =
        support::open_runtime(&path).expect("one ambiguous Run must not prevent opening the store");
    let error = runtime
        .execute(ApplicationCommand::LoadCheckpoint {
            conversation_id: "ambiguous".into(),
        })
        .unwrap_err();
    assert!(
        error.contains("recovery") || error.contains("replay"),
        "unexpected rejection: {error}"
    );
    let healthy_checkpoint = runtime
        .execute(ApplicationCommand::LoadCheckpoint {
            conversation_id: "healthy".into(),
        })
        .unwrap();
    assert!(!healthy_checkpoint.is_null());
    let mut healthy = Fixture {
        runtime,
        path,
        run: healthy_run,
        digest: healthy_digest,
        clock: 1100,
        conversation: "healthy".into(),
        assistant: "healthy:assistant".into(),
        reopen_after_boundary: true,
        claimed_effects: Vec::new(),
    };
    healthy.model_tool_cycle(0);
    assert_eq!(healthy.query_state().1["run_id"], healthy.run);
    // This proves explicit session read/execution isolation, not all-session
    // RecoverOrphanedRuns(None), which has a different transaction boundary.
}

#[test]
fn cold_cancellation_reuses_recovered_core_across_delivery_fact_queries() {
    use nola_application::cancellation_delivery::CancellationRunIdentity;
    let directory = tempfile::tempdir().unwrap();
    let mut fixture = Fixture::new(&directory.path().join("cancel-long-tail.db"), 32_768);
    fixture.reopen_after_boundary = false;
    let mut model = fixture.claim();
    let HostRequest::ModelStream { text_part_id, .. } = &model.payload else { panic!("model expected") };
    let part_id = text_part_id.clone();
    fixture.send(&mut model, HostResponse::ExecutionStarted, "started");
    fixture.send(&mut model, HostResponse::ModelAttemptStarted {
        provider: "fixture".into(), model: "fixture".into(), reason: "initial".into(),
    }, "attempt");
    for index in 0..256 {
        fixture.clock += 1;
        let sequence = model.resume_after_sequence.unwrap_or(0) + 1;
        fixture.runtime.execute(ApplicationCommand::ReduceAndAcceptHostResponse {
            input: ReduceAndAcceptHostResponse {
                conversation_id: fixture.conversation.clone(), run_id: fixture.run.clone(),
                invocation_config_digest: fixture.digest.clone(), received_at_ms: fixture.clock,
                response: HostResponseEnvelope {
                    request_id: model.request_id.clone(), effect_id: model.effect_id.clone(),
                    attempt_id: model.attempt_id.clone(), delegated_invocation_id: model.delegated_invocation_id.clone(),
                    host_id: model.host_id.clone(), lease_id: model.lease_id.clone(), session_id: model.session_id.clone(),
                    turn_id: model.turn_id.clone(), catalog_revision: model.catalog_revision,
                    availability_epoch: model.availability_epoch, delivery: model.delivery.clone(), sequence,
                    payload: HostResponse::ModelDelta { channel: ModelDeltaChannel::Text,
                        part_id: part_id.clone(), delta: format!("{index}: durable stream text. ") },
                },
            },
        }).unwrap();
        model.resume_after_sequence = Some(sequence);
    }
    fixture.send(&mut model, HostResponse::Accepted, "durable control boundary");
    {
        use nola_store_port::run_effect::RunEffectRepository;
        let store = nola_store::NolaStore::open(&fixture.path).unwrap();
        let recovery = store.load_recovery_checkpoint(&fixture.conversation).unwrap().unwrap();
        assert!(recovery.replay_inputs.len() <= nola_store_port::host_replay::HOST_REPLAY_CHECKPOINT_RECORDS, "normal streaming writer must bound cold replay work");
    }
    fixture.runtime = support::open_runtime(&fixture.path).unwrap();
    let started = std::time::Instant::now();
    fixture.runtime.execute(ApplicationCommand::RequestRunCancellation {
        input: nola_application::RequestRunCancellation {
            conversation_id: fixture.conversation.clone(), expected_run_id: fixture.run.clone(),
            assistant_message_id: fixture.assistant.clone(), cancellation_id: "long-tail-stop".into(),
            reason: None, requested_at_ms: fixture.clock + 1,
        },
    }).unwrap();
    let recovered_in = started.elapsed();
    let identity = CancellationRunIdentity { conversation_id: fixture.conversation.clone(),
        expected_run_id: fixture.run.clone(), assistant_message_id: fixture.assistant.clone() };
    let expected = fixture.runtime.execute(ApplicationCommand::LoadRunCancellationDeliveryFacts {
        input: identity.clone(),
    }).unwrap();
    let reads = std::time::Instant::now();
    for _ in 0..8 {
        assert_eq!(fixture.runtime.execute(ApplicationCommand::LoadRunCancellationDeliveryFacts {
            input: identity.clone(),
        }).unwrap(), expected);
    }
    eprintln!("long-tail cold cancel {:?}; eight exact delivery reads {:?}", recovered_in, reads.elapsed());
    let mut cold = support::open_runtime(&fixture.path).unwrap();
    assert_eq!(cold.execute(ApplicationCommand::LoadRunCancellationDeliveryFacts { input: identity }).unwrap(), expected);
    assert_eq!(expected["identity"]["expected_run_id"], fixture.run);
}

#[test]
fn checkpoint_command_reuses_snapshot_cache_and_invalidates_on_new_replay_head() {
    use nola_store_port::run_effect::RunEffectRepository;
    let directory = tempfile::tempdir().unwrap();
    let mut fixture = Fixture::new(&directory.path().join("read-cache.db"), 32768);
    fixture.reopen_after_boundary = false;
    let mut model = fixture.claim();
    let HostRequest::ModelStream { text_part_id, .. } = &model.payload else { panic!("model expected") };
    let part = text_part_id.clone();
    fixture.send(&mut model, HostResponse::ExecutionStarted, "start");
    fixture.send(&mut model, HostResponse::ModelAttemptStarted {
        provider: "fixture".into(), model: "fixture".into(), reason: "initial".into(),
    }, "attempt");
    fixture.send(&mut model, HostResponse::ModelDelta {
        channel: ModelDeltaChannel::Text, part_id: part.clone(), delta: "FIRST_CACHE_MARKER".into(),
    }, "first");
    let mut reader = nola_store::NolaStore::open(&fixture.path).unwrap();
    let read = |reader: &mut nola_store::NolaStore| nola_application::execute_application_command(reader,
        ApplicationCommand::LoadCheckpoint { conversation_id: fixture.conversation.clone() }).unwrap();
    nola_application::execute_application_command(&mut reader,
        ApplicationCommand::LoadSessionGoalProjection { conversation_id: fixture.conversation.clone() }).unwrap();
    reader.with_consistent_read(|store| {
        assert!(store.cached_read_checkpoint(&fixture.conversation).unwrap().is_some(),
            "Goal projection must populate the same checkpoint cache");
        Ok(())
    }).unwrap();
    let first = read(&mut reader);
    reader.with_consistent_read(|store| {
        let cached = store.cached_read_checkpoint(&fixture.conversation).unwrap().expect("production LoadCheckpoint must populate shared cache");
        assert!(cached.checkpoint_json.contains("FIRST_CACHE_MARKER"));
        Ok(())
    }).unwrap();
    assert_eq!(read(&mut reader), first);
    let baseline_path = fixture.path.clone();
    let baseline_conversation = fixture.conversation.clone();
    let baseline_revision = || rusqlite::Connection::open(&baseline_path).unwrap().query_row(
        "SELECT checkpoint_revision FROM core_checkpoint_replay_bases WHERE conversation_id=?1",
        [&baseline_conversation], |r| r.get::<_, i64>(0)).unwrap();
    let revision = baseline_revision();
    fixture.send(&mut model, HostResponse::ModelDelta {
        channel: ModelDeltaChannel::Text, part_id: part, delta: "SECOND_CACHE_MARKER".into(),
    }, "second");
    assert_eq!(baseline_revision(), revision,
        "incremental replay head changes without changing baseline revision");
    reader.with_consistent_read(|store| {
        assert!(store.cached_read_checkpoint(&fixture.conversation).unwrap().is_none());
        Ok(())
    }).unwrap();
    let second = nola_application::execute_application_command(&mut reader,
        ApplicationCommand::LoadCheckpoint { conversation_id: fixture.conversation.clone() }).unwrap();
    assert!(second["checkpoint_json"].as_str().unwrap().contains("SECOND_CACHE_MARKER"));
    assert_ne!(first, second);
}

#[test]
fn permission_configuration_hot_and_cold_reads_share_exact_committed_policy() {
    let directory = tempfile::tempdir().unwrap();
    let mut fixture = Fixture::new(&directory.path().join("permission-hot.db"), 2 * 1024 * 1024);
    fixture.reopen_after_boundary = false;
    let mut model = fixture.claim();
    let HostRequest::ModelStream { text_part_id, .. } = &model.payload else { panic!("model expected") };
    let part = text_part_id.clone();
    fixture.send(&mut model, HostResponse::ExecutionStarted, "start");
    fixture.send(&mut model, HostResponse::ModelAttemptStarted {
        provider: "fixture".into(), model: "fixture".into(), reason: "initial".into(),
    }, "attempt");
    fixture.send(&mut model, HostResponse::ModelDelta {
        channel: ModelDeltaChannel::Text, part_id: part, delta: "committed text".into(),
    }, "delta");
    let command = || serde_json::from_value::<ApplicationCommand>(json!({
        "type":"read_permission_configuration", "conversation_id":fixture.conversation,
    })).unwrap();
    let start = std::time::Instant::now();
    let hot = fixture.runtime.execute(command()).unwrap();
    for _ in 0..8 { assert_eq!(fixture.runtime.execute(command()).unwrap(), hot); }
    let hot_elapsed = start.elapsed();
    let mut cold = support::open_runtime(&fixture.path).unwrap();
    let start = std::time::Instant::now();
    assert_eq!(cold.execute(command()).unwrap(), hot);
    eprintln!("nine hot permission reads {:?}; first cold permission read {:?}", hot_elapsed, start.elapsed());
    assert_eq!(hot["active_run_id"], fixture.run);
    assert_eq!(hot["owner"]["kind"], "local");
}
