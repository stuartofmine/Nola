// SPDX-License-Identifier: Apache-2.0
//! Isolated production Core/SQLite pipeline: model UI -> receipt -> queued turn
//! -> model tool -> permission -> durable Host effect. No external file is written.
mod support;
use nola_agent_engine::agent::host::*;
use nola_agent_engine::agent::protocol::*;
use nola_application::{ApplicationCommand, ApplicationCommandReceiptOutcome, BeginAssistantTurn,
    BeginQueuedUserTurn, ClaimCommittedHostRequests, ExecuteReceivedCommand, ReduceAndAcceptHostResponse};
use nola_application::generative_ui::{ExecuteUiTool, SubmitUiInteraction};
use nola_domain::generative_ui::*;
use nola_store::NolaStore;
use serde_json::{json, Value};
use std::collections::BTreeMap;

const CONVERSATION: &str = "ui-permission-conversation";
fn capability(id: &str, route: HostCapabilityRoute) -> HostCapabilityDescriptor {
    HostCapabilityDescriptor {
        id: id.into(), title: id.into(), route, availability: CapabilityAvailability::Available,
        exposure: HostCapabilityExposure::Internal, permission_domain: None,
        input_schema: json!({"type":"object"}), input_normalization: Default::default(),
        output_policy: HostOutputPolicy { preview_char_limit: 4000, persist_full_output: true, allow_artifacts: true },
        supports_streaming: id == "model.stream", supports_argument_streaming: false,
        parallelism: HostCapabilityParallelism::Serial, cancellation: HostCancellationMode::Cooperative,
        replay_safety: HostReplaySafety::Recoverable,
    }
}
fn config() -> AgentCoreConfig {
    let mut capabilities = BTreeMap::new();
    capabilities.insert("model.stream".into(), capability("model.stream", HostCapabilityRoute::Model));
    capabilities.insert("permission.ask".into(), capability("permission.ask", HostCapabilityRoute::Permission));
    let ui = nola_agent_engine::agent::generative_ui::capability_descriptor();
    capabilities.insert(ui.id.clone(), ui);
    let mut write = capability("test.write", HostCapabilityRoute::Tool);
    write.exposure = HostCapabilityExposure::Visible;
    write.permission_domain = Some("filesystem.write".into());
    write.input_normalization.permission_resource = HostPermissionResourceDeclaration::FirstOf { keys: vec!["path".into()] };
    capabilities.insert(write.id.clone(), write);
    AgentCoreConfig {
        turn_identity: support::model_identity::identity(),
        session_id: Some(SessionId::from_string(CONVERSATION.into())), model: Some("test-model".into()),
        tools: vec![nola_agent_engine::agent::generative_ui::tool_schema(),
            json!({"type":"function","function":{"name":"test.write","description":"Write approved fixture","parameters":{"type":"object"}}})],
        host_catalog: Some(HostCapabilityCatalog { host_id: HostId("fixture-host".into()), revision: 1,
            availability_epoch: 1, lease: None, capabilities }),
        ..AgentCoreConfig::default()
    }
}
fn response(request: &HostRequestEnvelope, payload: HostResponse) -> HostResponseEnvelope {
    HostResponseEnvelope { request_id: request.request_id.clone(), attempt_id: request.attempt_id.clone(),
        delegated_invocation_id: request.delegated_invocation_id.clone(), effect_id: request.effect_id.clone(),
        host_id: request.host_id.clone(), lease_id: request.lease_id.clone(), session_id: request.session_id.clone(),
        turn_id: request.turn_id.clone(), catalog_revision: request.catalog_revision, availability_epoch: request.availability_epoch,
        delivery: request.delivery.clone(), sequence: request.resume_after_sequence.unwrap_or(0) + 1, payload }
}
fn send(store: &mut NolaStore, run: &str, digest: &str, request: &mut HostRequestEnvelope, payload: HostResponse, now: i64) {
    let response = response(request, payload);
    let sequence = response.sequence;
    nola_application::reduce_and_accept_host_response(store, ReduceAndAcceptHostResponse {
        conversation_id: CONVERSATION.into(), run_id: run.into(), invocation_config_digest: digest.into(), response, received_at_ms: now,
    }).unwrap();
    request.resume_after_sequence = Some(sequence);
}
fn pending(store: &mut NolaStore, run: &str) -> Vec<HostRequestEnvelope> {
    nola_application::load_committed_host_requests(store, run).unwrap().requests
}
fn start(store: &mut NolaStore, run: &str, digest: &str, effect: &HostRequestEnvelope, now: i64) -> HostRequestEnvelope {
    let mut request = nola_application::claim_committed_host_requests(store, ClaimCommittedHostRequests {
        run_id: run.into(), worker_generation: format!("fixture-{}", effect.effect_id.0), lease_owner: "fixture-worker".into(),
        claimed_at_ms: now, lease_expires_at_ms: now + 60_000, limit: 64,
    }).unwrap().requests.into_iter().find(|request| request.effect_id == effect.effect_id).unwrap();
    send(store, run, digest, &mut request, HostResponse::ExecutionStarted, now + 1);
    request
}
fn model_reply(store: &mut NolaStore, run: &str, digest: &str, tool_calls: Vec<Value>, answer: Option<&str>, now: i64) {
    model_reply_with_reasoning(store, run, digest, tool_calls, answer, None, now);
}
fn model_reply_with_reasoning(store: &mut NolaStore, run: &str, digest: &str, tool_calls: Vec<Value>,
    answer: Option<&str>, reasoning: Option<&str>, now: i64) {
    let mut request = pending(store, run).into_iter().find(|r| matches!(r.payload, HostRequest::ModelStream { .. })).unwrap();
    support::admit_model_sampling(store, run, digest, &mut request, now);
    if let Some(reasoning) = reasoning {
        let HostRequest::ModelStream { reasoning_part_id, .. } = &request.payload else { unreachable!() };
        let part_id = reasoning_part_id.clone();
        send(store, run, digest, &mut request, HostResponse::ModelDelta {
            channel: ModelDeltaChannel::Reasoning, part_id, delta: reasoning.into(),
        }, now + 1);
    }
    let items = answer.into_iter().map(|text| serde_json::to_value(AgentItem::assistant_message(
        ItemId::from_string(format!("answer-{}", request.request_id)), text)).unwrap()).collect();
    send(store, run, digest, &mut request, HostResponse::ModelCompleted { items, tool_calls, usage: json!({}) }, now + 1);
}
fn tool_reply(store: &mut NolaStore, run: &str, digest: &str, request: &mut HostRequestEnvelope, call_id: &str, output: Value, now: i64) {
    send(store, run, digest, request, HostResponse::ToolCompleted { call_id: call_id.into(), output,
        outcome: HostToolOutcome::Succeeded, activation: None, full_output_ref: None, artifacts: vec![] }, now);
}
fn ui_call(data_repetitions: usize) -> Value {
    json!({"call_id":"render-form","name":"render_ui","arguments":{"expected_revision":0,"messages":[
        {"version":"v0.9.1","createSurface":{"surfaceId":"write-form","catalogId":nola_kernel::generative_ui::schema::NOLA_CATALOG_ID,"sendDataModel":true}},
        {"version":"v0.9.1","updateComponents":{"surfaceId":"write-form","components":[
            {"id":"root","component":"Column","children":["value","submit"]},
            {"id":"value","component":"TextField","label":"Report content","value":{"path":"/content"}},
            {"id":"submit","component":"Button","child":"label","action":{"event":{"name":"write_report","context":{"content":{"path":"/content"}}}}},
            {"id":"label","component":"Text","text":"Create report"}
        ]}},
        {"version":"v0.9.1","updateDataModel":{"surfaceId":"write-form","value":{"content":"initial","large_field":"ui-data-".repeat(data_repetitions)}}}
    ]}})
}

fn publish_form_call(store: &mut NolaStore, run: &str, digest: &str, call: Value, now: i64)
    -> nola_application::generative_ui::UiToolPublication {
    execute_form_call(store, run, digest, call, now).unwrap()
}
fn execute_form_call(store: &mut NolaStore, run: &str, digest: &str, call: Value, now: i64)
    -> Result<nola_application::generative_ui::UiToolPublication, String> {
    execute_form_call_with_reasoning(store, run, digest, call, None, now)
}
fn execute_form_call_with_reasoning(store: &mut NolaStore, run: &str, digest: &str, call: Value,
    reasoning: Option<&str>, now: i64) -> Result<nola_application::generative_ui::UiToolPublication, String> {
    let call_id = call["call_id"].as_str().unwrap().to_owned();
    model_reply_with_reasoning(store, run, digest, vec![call], None, reasoning, now);
    let approval = pending(store, run).into_iter().find(|r|
        matches!(&r.payload, HostRequest::RequestPermission { capability_id, .. } if capability_id == "render_ui")).unwrap();
    let mut approval = start(store, run, digest, &approval, now + 10);
    let HostRequest::RequestPermission { permission_request_id, .. } = &approval.payload else { unreachable!() };
    let permission_request_id = permission_request_id.clone();
    send(store, run, digest, &mut approval, HostResponse::PermissionResolved {
        request_id: permission_request_id, resolution: HostPermissionResolution::allow_once(),
    }, now + 20);
    let requests = pending(store, run);
    let effect = requests.iter().find(|r| matches!(&r.payload, HostRequest::ExecuteTool { capability_id, .. }
        if capability_id == "render_ui")).cloned().unwrap_or_else(|| panic!("render_ui effect absent: {requests:#?}"));
    let mut effect = start(store, run, digest, &effect, now + 30);
    let publication = nola_application::generative_ui::execute_ui_tool(store, ExecuteUiTool {
        effect_id: effect.effect_id.0.clone(), request_id: effect.request_id.clone(),
        delivery: effect.delivery.clone().unwrap(), now_ms: now + 32,
    });
    match &publication {
        Ok(result) => tool_reply(store, run, digest, &mut effect, &call_id, serde_json::to_value(result).unwrap(), now + 33),
        Err(error) => send(store, run, digest, &mut effect, HostResponse::ToolCompleted {
            call_id, output:json!({"error":error}), outcome:HostToolOutcome::Failed,
            activation:None,full_output_ref:None,artifacts:vec![],
        }, now + 33),
    }
    publication
}

#[test]
fn ui_event_replay_preserves_one_input_and_permission_decision_gates_host_execution() {
    for (approved, data_repetitions) in [(false, 3000), (true, 3000), (true, 1)] {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("ui-event.sqlite");
        let mut store = NolaStore::open(&database).unwrap();
        let initial = nola_application::begin_assistant_turn(&mut store, BeginAssistantTurn {
            expected_configuration_revision: None, expected_parent_message_id: None,
            conversation_id: CONVERSATION.into(), conversation_title: "UI event".into(),
            user_message_id: "initial-user".into(), user_part_id: "initial-user:body".into(), user_text: "Create a report form".into(),
            user_content_json: None, assistant_message_id: "initial:assistant".into(), context_checkpoint_id: None,
            core_config: config(), submission: AgentSubmission::UserInput {
                items: vec![AgentItem::user_message(ItemId::from_string("initial-user".into()), "Create a report form")],
                turn_context_items: vec![], turn_context_platform: Default::default(), turn_context_components: vec![],
                submission_id: Some("initial".into()), submitted_at_epoch_ms: Some(1000), delivery: AgentInputDelivery::CurrentTurn,
            }, created_at_ms: 1000,
        }).unwrap();
        let run = &initial.begin.run_id;
        let digest = &initial.invocation_config_digest;
        let model_request = pending(&mut store, run).into_iter().find(|request|
            matches!(request.payload, HostRequest::ModelStream { .. })).unwrap();
        let encoded_request = serde_json::to_value(&model_request).unwrap();
        let tools = encoded_request["payload"]["tools"].as_array().unwrap();
        assert!(tools.iter().any(|tool| tool["function"]["name"] == "render_ui"),
            "The shared publisher must reach the actual model step without tool_search or a fixture exposure override");
        assert!(!tools.iter().any(|tool| tool["function"]["name"] == "tool_search"));
        let first = publish_form_call(&mut store, run, digest, ui_call(data_repetitions), 1100);
        assert!(first.changed);
        let before = nola_application::generative_ui::read_ui_snapshot(&store, &first.identity).unwrap().unwrap();
        let inspect = rusqlite::Connection::open(&database).unwrap();
        let versions_before: i64 = inspect.query_row("SELECT COUNT(*) FROM ui_surface_revisions", [], |row| row.get(0)).unwrap();
        for (index, now) in [(1, 1140), (2, 1180)] {
            let mut same = ui_call(data_repetitions);
            same["call_id"] = json!(format!("unchanged-form-{index}"));
            same["arguments"]["expected_revision"] = json!(first.revision);
            same["arguments"]["messages"].as_array_mut().unwrap().remove(0);
            let unchanged = publish_form_call(&mut store, run, digest, same, now);
            assert!(!unchanged.changed);
            assert_eq!(unchanged.identity, first.identity);
            assert_eq!(unchanged.revision, first.revision);
            assert!(unchanged.publication_guidance.starts_with("No change:"));
            assert_eq!(nola_application::generative_ui::read_ui_snapshot(&store, &first.identity).unwrap().unwrap(), before);
            assert_eq!(inspect.query_row::<i64, _, _>("SELECT COUNT(*) FROM ui_surface_revisions", [], |row| row.get(0)).unwrap(), versions_before);
        }
        assert!(first.publication_guidance.contains("Publication succeeded"));
        assert!(first.publication_guidance.contains("separate real ui_action"));
        assert!(pending(&mut store, run).iter().any(|request| matches!(request.payload, HostRequest::ModelStream { .. })),
            "UI publication must leave the model in control of its final response, not automatically complete the Run");
        let deletion = json!({"call_id":"delete-form","name":"render_ui","arguments":{
            "expected_revision":first.revision,"messages":[{"version":"v0.9.1","deleteSurface":{"surfaceId":"write-form"}}]
        }});
        let deleted = publish_form_call(&mut store, run, digest, deletion, 1260);
        assert_eq!(deleted.phase, UiSurfacePhase::Deleted);
        let mut recreation = ui_call(data_repetitions);
        recreation["call_id"] = json!("recreate-form");
        let publication = publish_form_call(&mut store, run, digest, recreation, 1340);
        assert!(publication.identity.generation > first.identity.generation);
        assert_ne!(publication.identity, first.identity);
        assert!(nola_application::generative_ui::read_ui_revision_snapshot(&store, &first.identity, first.revision).unwrap().is_some());
        model_reply(&mut store, run, digest, vec![], Some("Fill in the form."), 1500);
        assert!(pending(&mut store, run).is_empty());
        let audit = rusqlite::Connection::open(&database).unwrap();
        let sealed_before: String = audit.query_row(
            "SELECT seal_json FROM assistant_run_terminal_seals WHERE run_id=?1", [run], |row| row.get(0)).unwrap();
        let (surface_owner, surface_part, frozen_sequence, frozen_head): (String, String, i64, String) = audit.query_row(
            "SELECT owner,part_id,sequence,head_json FROM assistant_journal_part_versions WHERE part_id LIKE 'ui-surface:%' ORDER BY sequence DESC LIMIT 1",
            [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))).unwrap();
        let snapshot = nola_application::generative_ui::read_ui_snapshot(&store, &publication.identity).unwrap().unwrap();
        let field = snapshot.nodes.values().find(|node| node.component == "TextField").unwrap();
        let button = snapshot.nodes.values().find(|node| node.component == "Button").unwrap();
        let command = ApplicationCommand::SubmitUiInteraction { input: SubmitUiInteraction {
            interaction: UiInteraction { identity: snapshot.identity.clone(), expected_surface_revision: snapshot.revision,
                edits: vec![UiEdit::Set { instance_id: field.instance_id.clone(), binding: "value".into(), value: json!("approved content") }],
                action: Some(button.actions["action"].clone()) },
            submission_id: "card-submit".into(), submitted_at_ms: 2000,
        } };
        drop(store);
        let encoded = serde_json::to_string(&command).unwrap();
        let envelope = || ExecuteReceivedCommand { client_id: "ui-client".into(), request_id: "ui-action-once".into(),
            command_type: "submit_ui_interaction".into(), command_json: encoded.clone(), expected_revision: None, received_at: "2000".into() };
        // A remote renderer may possess this source snapshot and all its
        // interaction handles. That possession does not authorize its local
        // Application to execute the action or create an independent Surface.
        let mirror_database = directory.path().join("mirror.sqlite");
        let mut mirror = support::open_runtime(&mirror_database).unwrap();
        assert!(mirror.execute(ApplicationCommand::ReadUiSurfaceChanges {
            identity: snapshot.identity.clone(), base_revision: Some(snapshot.revision),
        }).is_err());
        let mirror_attempt = mirror.execute_received_command(envelope(), serde_json::from_str(&encoded).unwrap()).unwrap();
        assert!(matches!(mirror_attempt.outcome, ApplicationCommandReceiptOutcome::Rejected { .. }));
        let mirror_db = rusqlite::Connection::open(&mirror_database).unwrap();
        for table in ["ui_surfaces", "ui_surface_revisions", "agent_commands"] {
            assert_eq!(mirror_db.query_row::<i64,_,_>(&format!("SELECT count(*) FROM {table}"), [], |row| row.get(0)).unwrap(), 0);
        }
        drop(mirror);
        let mut runtime = support::open_runtime(&database).unwrap();
        let committed = runtime.execute_received_command(envelope(), serde_json::from_str(&encoded).unwrap()).unwrap();
        let ApplicationCommandReceiptOutcome::Committed { ref result, .. } = committed.outcome else { panic!("UI receipt not committed") };
        assert_eq!(result["queued_submission_id"], "card-submit");
        // Simulate commit acknowledged by the source but its transport response
        // lost: restart the source before the mirror retries the exact receipt.
        drop(runtime);
        let mut runtime = support::open_runtime(&database).unwrap();
        let replay = runtime.execute_received_command(envelope(), serde_json::from_str(&encoded).unwrap()).unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.outcome, committed.outcome);
        let after_reconnect = runtime.execute(ApplicationCommand::ReadUiSurfaceChanges {
            identity:snapshot.identity.clone(),base_revision:Some(snapshot.revision),
        }).unwrap();
        let after_reconnect: UiSurfaceUpdate = serde_json::from_value(after_reconnect).unwrap();
        match after_reconnect {
            UiSurfaceUpdate::Changes { changes } => {
                assert_eq!(changes.identity, snapshot.identity);
                assert_eq!(changes.base_revision, snapshot.revision);
                assert_eq!(changes.revision, snapshot.revision + 1);
            }
            UiSurfaceUpdate::Snapshot { snapshot: current } => {
                assert_eq!(current.identity, snapshot.identity);
                assert_eq!(current.revision, snapshot.revision + 1);
            }
            other => panic!("source reconnect failed to advance the stale mirror: {other:?}"),
        }
        let db = rusqlite::Connection::open(&database).unwrap();
        let sealed_after: String = db.query_row(
            "SELECT seal_json FROM assistant_run_terminal_seals WHERE run_id=?1", [run], |row| row.get(0)).unwrap();
        assert_eq!(sealed_after, sealed_before, "post-terminal UI edits must not recapture the sealed Run frontier");
        let preserved_head: String = db.query_row(
            "SELECT head_json FROM assistant_journal_part_versions WHERE owner=?1 AND part_id=?2 AND sequence=?3",
            rusqlite::params![surface_owner, surface_part, frozen_sequence], |row| row.get(0)).unwrap();
        assert_eq!(preserved_head, frozen_head, "the sealed historical Surface reference remains immutable");
        let current_head: String = db.query_row(
            "SELECT head_json FROM assistant_journal_parts WHERE owner=?1 AND part_id=?2",
            rusqlite::params![surface_owner, surface_part], |row| row.get(0)).unwrap();
        assert_ne!(current_head, frozen_head, "live Surface references advance independently of the sealed history");
        assert_eq!(db.query_row::<i64, _, _>("SELECT count(*) FROM agent_commands WHERE command_id='card-submit'", [], |row| row.get(0)).unwrap(), 1);
        let lease = runtime.execute(ApplicationCommand::ClaimNextAgentCommand {
            session_id: CONVERSATION.into(), lease_owner: "ui-worker".into(), now_ms: 2100, lease_until_ms: 60_000,
        }).unwrap();
        let payload: Value = serde_json::from_str(lease["payload_json"].as_str().unwrap()).unwrap();
        assert!(payload.to_string().contains("approved content"));
        assert!(payload.to_string().contains("ui:card-submit:user"));
        drop(runtime);
        let revision = db.query_row("SELECT configuration_revision FROM conversations WHERE id=?1", [CONVERSATION], |row| row.get(0)).unwrap();
        let parent = db.query_row("SELECT id FROM messages WHERE conversation_id=?1 ORDER BY ordinal DESC LIMIT 1", [CONVERSATION], |row| row.get(0)).unwrap();
        let mut store = NolaStore::open(&database).unwrap();
        let next = nola_application::begin_queued_user_turn(&mut store, BeginQueuedUserTurn {
            conversation_id: CONVERSATION.into(), command_id: "card-submit".into(), lease_owner: "ui-worker".into(),
            delivery_attempt: lease["delivery_attempt"].as_i64().unwrap(), expected_configuration_revision: revision,
            expected_parent_message_id: Some(parent), core_config: config(), turn_context_items: vec![],
            turn_context_platform: Default::default(), turn_context_components: vec![], now_ms: 2200,
        }).unwrap();
        assert_ne!(next.begin.run_id, initial.begin.run_id);
        let (kind, summary, canonical): (String, String, String) = db.query_row(
            "SELECT kind,content_text,content_json FROM message_parts WHERE message_id='ui:card-submit:user'",
            [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).unwrap();
        assert_eq!(kind, "ui_action");
        assert_eq!(summary, "Submitted form · write_report");
        assert!(!summary.contains("approved content"));
        let nola_domain::MessagePartView::UiAction { envelope: action_envelope, summary: stored_summary, .. } =
            serde_json::from_str(&canonical).unwrap() else { panic!("expected canonical UiAction") };
        assert_eq!(stored_summary, summary);
        assert_eq!(action_envelope["message"]["action"]["context"]["content"], "approved content");
        assert!(action_envelope.to_string().contains(&"ui-data-".repeat(data_repetitions)));
        // Exercise the real StoredMessage -> bounded historical fields ->
        // public Part path used by clients, rather than decoding SQL alone.
        let projected = nola_application::load_conversation_projection_v1(&store, CONVERSATION).unwrap();
        let user_projection = projected.messages.iter().find(|message| message.message_id.0 == "ui:card-submit:user").unwrap();
        if data_repetitions == 1 {
            let nola_domain::MessagePartView::UiAction { envelope: projected_envelope, summary: projected_summary, .. } =
                &user_projection.parts[0] else { panic!("small persisted action must project as typed: {:?}", user_projection.parts[0]) };
            assert_eq!(projected_envelope, &action_envelope);
            assert_eq!(projected_summary, &summary);
        } else {
            let nola_domain::MessagePartView::Historical { projection, .. } = &user_projection.parts[0]
                else { panic!("large action should retain bounded historical metadata") };
            assert_eq!(projection.status, nola_domain::HistoricalProjectionStatus::Deferred);
            assert_eq!(projection.fields["summary"], nola_domain::SemanticValue::Complete {
                value: nola_domain::StructuredValue::String(summary.clone()) });
        }

        let run = &next.begin.run_id;
        let digest = &next.invocation_config_digest;
        let model = pending(&mut store, run).into_iter().find(|request|
            matches!(request.payload, HostRequest::ModelStream { .. })).unwrap();
        let HostRequest::ModelStream { messages, .. } = &model.payload else { unreachable!() };
        let model_input = serde_json::to_string(messages).unwrap();
        assert!(model_input.contains("write_report"), "the accepted UI event must reach the actual model request");
        assert!(model_input.contains("approved content"), "the committed form edit must reach the actual model request");
        assert!(model_input.contains(&"ui-data-".repeat(data_repetitions)), "sendDataModel must survive queue admission without Document truncation");
        assert!(model_input.contains("a2uiClientDataModel"));
        // A committed provider request is recovered exactly, rather than
        // rebuilding its full-data metadata from a newer Surface revision.
        drop(store);
        let mut store = NolaStore::open(&database).unwrap();
        let recovered_model = pending(&mut store, run).into_iter().find(|request|
            matches!(request.payload, HostRequest::ModelStream { .. })).unwrap();
        let HostRequest::ModelStream { messages: recovered_messages, .. } = &recovered_model.payload else { unreachable!() };
        assert_eq!(serde_json::to_string(recovered_messages).unwrap(), model_input);
        let current_surface = nola_application::generative_ui::read_ui_snapshot(&store, &publication.identity).unwrap().unwrap();
        let stale = execute_form_call(&mut store, run, digest, json!({
            "call_id":"stale-existing-form","name":"render_ui","arguments":{
                "expected_revision":current_surface.revision - 1,"messages":[{
                    "version":"v0.9.1","updateDataModel":{"surfaceId":"write-form","path":"/status","value":"received"}
                }]
            }
        }), 2210).unwrap_err();
        let conflict: Value = serde_json::from_str(&stale).unwrap();
        assert_eq!(conflict["error"], "ui_revision_conflict");
        assert_eq!(conflict["current_revision"], current_surface.revision);
        assert_eq!(nola_application::generative_ui::read_ui_snapshot(&store, &publication.identity).unwrap().unwrap().revision, current_surface.revision);
        let follow_up = publish_form_call(&mut store, run, digest, json!({
            "call_id":"update-existing-form","name":"render_ui","arguments":{
                "expected_revision":current_surface.revision,"messages":[{
                    "version":"v0.9.1","updateDataModel":{"surfaceId":"write-form","path":"/status","value":"received"}
                }]
            }
        }), 2250);
        assert_eq!(follow_up.identity, publication.identity, "a later Run updates the original card rather than creating a new owner");
        assert_eq!(follow_up.revision, current_surface.revision + 1);
        assert_ne!(follow_up.identity.owner.originating_run_id, *run);
        model_reply(&mut store, run, digest, vec![json!({"call_id":"write-after-ui","name":"test.write",
            "arguments":{"path":"/workspace/report.txt","content":"approved content"}})], None, 2300);
        let requests = pending(&mut store, run);
        assert!(!requests.iter().any(|r| matches!(r.payload, HostRequest::ExecuteTool { .. })));
        let ask = requests.into_iter().find(|r| matches!(r.payload, HostRequest::RequestPermission { .. })).unwrap();
        let mut ask = start(&mut store, run, digest, &ask, 2400);
        let HostRequest::RequestPermission { permission_request_id, .. } = &ask.payload else { unreachable!() };
        let permission_request_id = permission_request_id.clone();
        send(&mut store, run, digest, &mut ask, HostResponse::PermissionResolved {
            request_id: permission_request_id, resolution: if approved { HostPermissionResolution::allow_once() } else { HostPermissionResolution::deny() },
        }, 2500);
        let tools: Vec<_> = pending(&mut store, run).into_iter().filter(|r| matches!(r.payload, HostRequest::ExecuteTool { .. })).collect();
        assert_eq!(tools.len(), usize::from(approved));
        if approved {
            let HostRequest::ExecuteTool { call_id, input, .. } = &tools[0].payload else { unreachable!() };
            assert_eq!(call_id, "write-after-ui");
            assert_eq!(input, &json!({"path":"/workspace/report.txt","content":"approved content"}));
            let mut tool = start(&mut store, run, digest, &tools[0], 2600);
            tool_reply(&mut store, run, digest, &mut tool, "write-after-ui", json!({"written":true}), 2700);
            assert!(!pending(&mut store, run).iter().any(|r| matches!(r.payload, HostRequest::ExecuteTool { .. })));
        }
        // The UI receipt remains idempotent even after queue acknowledgement
        // and permission processing; replay must not create another task.
        drop(store);
        let mut runtime = support::open_runtime(&database).unwrap();
        let replay = runtime.execute_received_command(envelope(), serde_json::from_str(&encoded).unwrap()).unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.outcome, committed.outcome);
        assert_eq!(db.query_row::<i64, _, _>("SELECT count(*) FROM messages WHERE id='ui:card-submit:user'", [], |row| row.get(0)).unwrap(), 1);
        assert!(runtime.execute(ApplicationCommand::PeekAgentCommand { session_id: CONVERSATION.into() }).unwrap().is_null());
    }
}

#[test]
fn ui_publication_lost_ack_cold_recovery_reads_durable_effect_result_without_republishing() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("ui-lost-ack.sqlite");
    let mut store = NolaStore::open(&database).unwrap();
        let initial = nola_application::begin_assistant_turn(&mut store, BeginAssistantTurn {
            expected_configuration_revision: None, expected_parent_message_id: None,
            conversation_id: CONVERSATION.into(), conversation_title: "UI event".into(),
            user_message_id: "initial-user".into(), user_part_id: "initial-user:body".into(), user_text: "Create a report form".into(),
            user_content_json: None, assistant_message_id: "initial:assistant".into(), context_checkpoint_id: None,
            core_config: config(), submission: AgentSubmission::UserInput {
                items: vec![AgentItem::user_message(ItemId::from_string("initial-user".into()), "Create a report form")],
                turn_context_items: vec![], turn_context_platform: Default::default(), turn_context_components: vec![],
                submission_id: Some("initial".into()), submitted_at_epoch_ms: Some(1000), delivery: AgentInputDelivery::CurrentTurn,
            }, created_at_ms: 1000,
        }).unwrap();
        let run = &initial.begin.run_id;
        let digest = &initial.invocation_config_digest;

    let call = ui_call(1);
    let now = 1100;
    let call_id = call["call_id"].as_str().unwrap().to_owned();
    model_reply(&mut store, run, digest, vec![call], None, now);
    let approval = pending(&mut store, run).into_iter().find(|r|
        matches!(&r.payload, HostRequest::RequestPermission { capability_id, .. } if capability_id == "render_ui")).unwrap();
    let mut approval = start(&mut store, run, digest, &approval, now + 10);
    let HostRequest::RequestPermission { permission_request_id, .. } = &approval.payload else { unreachable!() };
    let permission_request_id = permission_request_id.clone();
    send(&mut store, run, digest, &mut approval, HostResponse::PermissionResolved {
        request_id: permission_request_id, resolution: HostPermissionResolution::allow_once(),
    }, now + 20);
    let requests = pending(&mut store, run);
    let effect = requests.iter().find(|r| matches!(&r.payload, HostRequest::ExecuteTool { capability_id, .. }
        if capability_id == "render_ui")).cloned().unwrap_or_else(|| panic!("render_ui effect absent: {requests:#?}"));
    let effect = start(&mut store, run, digest, &effect, now + 30);

    let command = ApplicationCommand::ExecuteUiTool { input: ExecuteUiTool {
        effect_id: effect.effect_id.0.clone(), request_id: effect.request_id.clone(),
        delivery: effect.delivery.clone().unwrap(), now_ms: now + 32,
    }};
    let encoded = serde_json::to_string(&command).unwrap();
    drop(store);
    let mut runtime = support::open_runtime(&database).unwrap();
    let receipt = runtime.execute_received_command(ExecuteReceivedCommand {
        client_id:"publisher".into(), request_id:"publication-command".into(),
        command_type:"execute_ui_tool".into(), command_json:encoded, expected_revision:None,
        received_at:"1132".into(),
    },command).unwrap();
    let ApplicationCommandReceiptOutcome::Committed {result: original, ..} = receipt.outcome else {panic!("publication must commit")};
    // Both the Application ACK and Host ToolCompleted are lost. Cold recovery
    // must use the existing effect result; no replacement model call is sent.
    drop(runtime);
    let mut runtime = support::open_runtime(&database).unwrap();
    let saved = runtime.command_receipt("publisher","publication-command").unwrap().unwrap();
    assert!(matches!(saved.outcome,ApplicationCommandReceiptOutcome::Committed {ref result,..} if result==&original));
    runtime.execute(ApplicationCommand::RecoverOrphanedRuns {
        conversation_id:Some(CONVERSATION.into()),reason:"lost host ACK".into(),
        interrupted_at:"70000".into(),interrupted_at_ms:70000,
    }).unwrap();
    drop(runtime);
    let mut store = NolaStore::open(&database).unwrap();
    let mut reclaimed = nola_application::claim_committed_host_requests(&mut store, ClaimCommittedHostRequests {
        run_id:run.clone(),worker_generation:"recovered-worker".into(),lease_owner:"recovered-host".into(),
        claimed_at_ms:70001,lease_expires_at_ms:130001,limit:64,
    }).unwrap().requests.into_iter().find(|request|request.effect_id==effect.effect_id)
        .expect("idempotent publication must be recoverable under its original effect");
    send(&mut store,run,digest,&mut reclaimed,HostResponse::ExecutionStarted,70002);
    let restored = nola_application::generative_ui::execute_ui_tool(&mut store,ExecuteUiTool {
        effect_id:reclaimed.effect_id.0.clone(),request_id:reclaimed.request_id.clone(),
        delivery:reclaimed.delivery.clone().unwrap(),now_ms:70003,
    }).unwrap();
    assert_eq!(serde_json::to_value(&restored).unwrap(),original);
    let db=rusqlite::Connection::open(&database).unwrap();
    for table in ["ui_tool_publications","ui_surfaces","ui_surface_revisions"] {
        assert_eq!(db.query_row::<i64,_,_>(&format!("SELECT count(*) FROM {table}"),[],|row|row.get(0)).unwrap(),1,
            "recovery must not create a second publication/revision");
    }
    tool_reply(&mut store,run,digest,&mut reclaimed,&call_id,original,70004);
    assert!(pending(&mut store,run).iter().any(|request|matches!(request.payload,HostRequest::ModelStream{..})),
        "the original tool result must let the same model Run continue");
}

#[test]
fn long_chinese_surface_keeps_full_text_with_journal_sized_summary_and_cold_receipt() {
    use nola_store_port::assistant_journal::AssistantJournalRepository;
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("long-ui.sqlite");
    let mut store = NolaStore::open(&database).unwrap();
    let initial = nola_application::begin_assistant_turn(&mut store, BeginAssistantTurn {
        expected_configuration_revision: None, expected_parent_message_id: None,
        conversation_id: CONVERSATION.into(), conversation_title: "Long form".into(),
        user_message_id: "long-user".into(), user_part_id: "long-user:body".into(), user_text: "Create a long form".into(),
        user_content_json: None, assistant_message_id: "long:assistant".into(), context_checkpoint_id: None,
        core_config: config(), submission: AgentSubmission::UserInput {
            items: vec![AgentItem::user_message(ItemId::from_string("long-user".into()), "Create a long form")],
            turn_context_items: vec![], turn_context_platform: Default::default(), turn_context_components: vec![],
            submission_id: Some("long".into()), submitted_at_epoch_ms: Some(1000), delivery: AgentInputDelivery::CurrentTurn,
        }, created_at_ms: 1000,
    }).unwrap();
    let text = "中文长任务正文需要完整保留。".repeat(500);
    assert!(text.len() > 16 * 1024);
    let call = json!({"call_id":"long-ui","name":"render_ui","arguments":{"expected_revision":0,"messages":[
            {"version":"v0.9.1","createSurface":{"surfaceId":"long-form","catalogId":nola_kernel::generative_ui::schema::NOLA_CATALOG_ID}},
            {"version":"v0.9.1","updateComponents":{"surfaceId":"long-form","components":[{"id":"root","component":"Text","text":text}]}}
        ]}});
    let run = &initial.begin.run_id;
    let digest = &initial.invocation_config_digest;
    model_reply(&mut store, run, digest, vec![call], None, 1100);
    let approval = pending(&mut store, run).into_iter().find(|r| matches!(&r.payload,
        HostRequest::RequestPermission { capability_id, .. } if capability_id == "render_ui")).unwrap();
    let mut approval = start(&mut store, run, digest, &approval, 1110);
    let HostRequest::RequestPermission { permission_request_id, .. } = &approval.payload else { unreachable!() };
    let permission_request_id = permission_request_id.clone();
    send(&mut store, run, digest, &mut approval, HostResponse::PermissionResolved {
        request_id: permission_request_id, resolution: HostPermissionResolution::allow_once(),
    }, 1120);
    let effect = pending(&mut store, run).into_iter().find(|r| matches!(&r.payload,
        HostRequest::ExecuteTool { capability_id, .. } if capability_id == "render_ui")).unwrap();
    let effect = start(&mut store, run, digest, &effect, 1130);
    let publication = nola_application::generative_ui::execute_ui_tool(&mut store, ExecuteUiTool {
        effect_id: effect.effect_id.0.clone(), request_id: effect.request_id.clone(),
        delivery: effect.delivery.clone().unwrap(), now_ms: 1132,
    }).unwrap();
    // Crash after the atomic publication but before its Host tool response.
    drop(store);
    let mut store = NolaStore::open(&database).unwrap();
    let snapshot = nola_application::generative_ui::read_ui_snapshot(&store, &publication.identity).unwrap().unwrap();
    assert_eq!(snapshot.nodes.values().next().unwrap().props["text"], text);
    assert!(snapshot.summary.len() <= nola_domain::assistant_journal_operations::MAX_METADATA_STRING_BYTES);
    assert!(snapshot.summary.ends_with('…'));
    assert!(text.starts_with(snapshot.summary.trim_end_matches('…')));
    let audit = rusqlite::Connection::open(&database).unwrap();
    let (part_id, raw): (String, String) = audit.query_row("SELECT id,content_json FROM message_parts WHERE kind='ui_surface'", [], |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
    let nola_domain::MessagePartView::UiSurface { surface, summary, .. } = serde_json::from_str(&raw).unwrap() else { panic!("missing Surface Part") };
    assert_eq!(summary, snapshot.summary);
    let identity = nola_domain::assistant_journal::AssistantJournalIdentity {
        source_store_epoch: publication.identity.owner.authority_id.clone(), conversation_id: CONVERSATION.into(),
        run_id: initial.begin.run_id, assistant_message_id: "long:assistant".into(),
    };
    let head = store.assistant_journal_part(&identity, &part_id).unwrap().unwrap();
    let nola_domain::assistant_journal_operations::JournalPartMetadata::UiSurface(metadata) = head.metadata else { panic!("missing Surface journal metadata") };
    assert_eq!(metadata.surface, surface);
    assert_eq!(metadata.summary, snapshot.summary);
    let receipt: String = audit.query_row("SELECT result_json FROM ui_tool_publications", [], |row| row.get(0)).unwrap();
    let restored: nola_application::generative_ui::UiToolPublication = serde_json::from_str(&receipt).unwrap();
    assert_eq!(restored.identity, publication.identity);
    assert_eq!(restored.revision, publication.revision);
    let replay = nola_application::generative_ui::execute_ui_tool(&mut store, ExecuteUiTool {
        effect_id: effect.effect_id.0, request_id: effect.request_id,
        delivery: effect.delivery.unwrap(), now_ms: 1133,
    }).unwrap();
    assert_eq!(replay.identity, publication.identity);
    assert_eq!(replay.revision, publication.revision);
}

#[test]
fn recovered_ui_effect_can_be_cancelled_without_fabricating_not_started() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("ui-lost-ack.sqlite");
    let mut store = NolaStore::open(&database).unwrap();
        let initial = nola_application::begin_assistant_turn(&mut store, BeginAssistantTurn {
            expected_configuration_revision: None, expected_parent_message_id: None,
            conversation_id: CONVERSATION.into(), conversation_title: "UI event".into(),
            user_message_id: "initial-user".into(), user_part_id: "initial-user:body".into(), user_text: "Create a report form".into(),
            user_content_json: None, assistant_message_id: "initial:assistant".into(), context_checkpoint_id: None,
            core_config: config(), submission: AgentSubmission::UserInput {
                items: vec![AgentItem::user_message(ItemId::from_string("initial-user".into()), "Create a report form")],
                turn_context_items: vec![], turn_context_platform: Default::default(), turn_context_components: vec![],
                submission_id: Some("initial".into()), submitted_at_epoch_ms: Some(1000), delivery: AgentInputDelivery::CurrentTurn,
            }, created_at_ms: 1000,
        }).unwrap();
        let run = &initial.begin.run_id;
        let digest = &initial.invocation_config_digest;

    let call = ui_call(1);
    let now = 1100;
    model_reply(&mut store, run, digest, vec![call], None, now);
    let approval = pending(&mut store, run).into_iter().find(|r|
        matches!(&r.payload, HostRequest::RequestPermission { capability_id, .. } if capability_id == "render_ui")).unwrap();
    let mut approval = start(&mut store, run, digest, &approval, now + 10);
    let HostRequest::RequestPermission { permission_request_id, .. } = &approval.payload else { unreachable!() };
    let permission_request_id = permission_request_id.clone();
    send(&mut store, run, digest, &mut approval, HostResponse::PermissionResolved {
        request_id: permission_request_id, resolution: HostPermissionResolution::allow_once(),
    }, now + 20);
    let requests = pending(&mut store, run);
    let effect = requests.iter().find(|r| matches!(&r.payload, HostRequest::ExecuteTool { capability_id, .. }
        if capability_id == "render_ui")).cloned().unwrap_or_else(|| panic!("render_ui effect absent: {requests:#?}"));
    let effect = start(&mut store, run, digest, &effect, now + 30);

    drop(store);
    let mut runtime = support::open_runtime(&database).unwrap();
    runtime.execute(ApplicationCommand::RecoverOrphanedRuns {
        conversation_id: Some(CONVERSATION.into()), reason: "UI worker lost".into(),
        interrupted_at: "70000".into(), interrupted_at_ms: 70000,
    }).unwrap();
    runtime.execute(ApplicationCommand::ReduceAndCommitActiveSubmission {
        input: nola_application::ReduceAndCommitActiveSubmission {
            conversation_id: CONVERSATION.into(), run_id: run.clone(),
            transition_id: "rebind-ui".into(), invocation_config_digest: digest.clone(),
            submission: AgentSubmission::ResumeHostEffect {effect_id: effect.effect_id.0.clone()},
            committed_at_ms: 70001,
        },
    }).unwrap();
    drop(runtime);
    let mut store = NolaStore::open(&database).unwrap();
    let rebound = pending(&mut store,run).into_iter().find(|r|r.effect_id==effect.effect_id).unwrap();
    assert_ne!(rebound.request_id,effect.request_id);
    drop(store);
    let mut runtime = support::open_runtime(&database).unwrap();
    runtime.execute(ApplicationCommand::RequestRunCancellation {
        input: nola_application::RequestRunCancellation {
            conversation_id: CONVERSATION.into(), expected_run_id:run.clone(),
            assistant_message_id:"initial:assistant".into(), cancellation_id:"cancel-recovered-ui".into(),
            reason:Some("user stop".into()),requested_at_ms:70002,
        },
    }).unwrap();
    drop(runtime);
    let mut store = NolaStore::open(&database).unwrap();
    let requests = pending(&mut store,run);
    assert!(requests.iter().any(|r|matches!(&r.payload,HostRequest::Cancel {target_effect_id,target_not_started:None} if target_effect_id==&effect.effect_id)),
        "cancel must be admitted without turning historical execution into a no-start proof");
    use nola_store_port::assistant_journal::AssistantJournalRepository;
    assert!(store.local_assistant_run_terminal_seal(CONVERSATION,run).unwrap().is_none());
    assert!(store.effect_delivery_attempt(&effect.delivery.as_ref().unwrap().attempt_id).unwrap().unwrap().execution_started_at_ms.is_some());
}

#[test]
fn draft_preview_reuses_final_validation_without_writing_or_enqueuing() {
    use nola_domain::generative_ui::{UiDraftPreviewInput, UiError};
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("ui-event.sqlite");
        let mut store = NolaStore::open(&database).unwrap();
        let initial = nola_application::begin_assistant_turn(&mut store, BeginAssistantTurn {
            expected_configuration_revision: None, expected_parent_message_id: None,
            conversation_id: CONVERSATION.into(), conversation_title: "UI event".into(),
            user_message_id: "initial-user".into(), user_part_id: "initial-user:body".into(), user_text: "Create a report form".into(),
            user_content_json: None, assistant_message_id: "initial:assistant".into(), context_checkpoint_id: None,
            core_config: config(), submission: AgentSubmission::UserInput {
                items: vec![AgentItem::user_message(ItemId::from_string("initial-user".into()), "Create a report form")],
                turn_context_items: vec![], turn_context_platform: Default::default(), turn_context_components: vec![],
                submission_id: Some("initial".into()), submitted_at_epoch_ms: Some(1000), delivery: AgentInputDelivery::CurrentTurn,
            }, created_at_ms: 1000,
        }).unwrap();
        let run = &initial.begin.run_id;
        let digest = &initial.invocation_config_digest;

    let mut call = ui_call(1);
    call["arguments"]["messages"][1]["updateComponents"]["components"][2]["checks"] = json!([
        {"condition":{"call":"required","args":{"value":{"path":"/content"}}},"message":"Content is required"}
    ]);
    call["arguments"]["messages"][2]["updateDataModel"]["value"]["content"] = json!("");
    let publication = publish_form_call(&mut store, run, digest, call, 1100);
    let snapshot = nola_application::generative_ui::read_ui_snapshot(&store, &publication.identity).unwrap().unwrap();
    let field = snapshot.nodes.values().find(|node| node.component == "TextField").unwrap();
    let button = snapshot.nodes.values().find(|node| node.component == "Button").unwrap();
    assert_eq!(button.validation_errors, vec!["Content is required"]);
    let preview_input = UiDraftPreviewInput { identity:snapshot.identity.clone(), expected_surface_revision:snapshot.revision,
        edits:vec![UiEdit::Set { instance_id:field.instance_id.clone(), binding:"value".into(), value:json!("有效草稿") }], draft_generation:2 };
    assert!(ApplicationCommand::PreviewUiSurface { input:preview_input.clone() }.is_read_only_query());
    let valid = nola_application::generative_ui::preview_ui_surface(&store, preview_input.clone()).unwrap();
    assert_eq!(valid.base_revision, snapshot.revision);
    assert_eq!(valid.snapshot.revision, snapshot.revision);
    assert!(valid.snapshot.nodes[&button.instance_id].validation_errors.is_empty());
    assert_eq!(nola_application::generative_ui::read_ui_snapshot(&store, &publication.identity).unwrap().unwrap(), snapshot);
    let mut invalid_input = preview_input.clone();
    invalid_input.draft_generation = 3;
    invalid_input.edits = vec![UiEdit::Set { instance_id:field.instance_id.clone(), binding:"value".into(), value:json!("") }];
    let invalid = nola_application::generative_ui::preview_ui_surface(&store, invalid_input.clone()).unwrap();
    assert_eq!(invalid.snapshot.nodes[&button.instance_id].validation_errors, vec!["Content is required"]);
    let state = nola_store_port::generative_ui::GenerativeUiRepository::read_ui_surface(&store, &snapshot.identity).unwrap().unwrap();
    let interaction = |edits| UiInteraction { identity:snapshot.identity.clone(), expected_surface_revision:snapshot.revision,
        edits, action:Some(button.actions["action"].clone()) };
    assert!(matches!(nola_kernel::generative_ui::interaction::prepare_interaction(&state, &interaction(invalid_input.edits.clone())), Err(UiError::ValidationFailed { .. })));
    assert!(nola_kernel::generative_ui::interaction::prepare_interaction(&state, &interaction(preview_input.edits.clone())).unwrap().event.is_some());
    for revision in [0, snapshot.revision + 1, u64::MAX] {
        let mut stale = preview_input.clone(); stale.expected_surface_revision = revision;
        assert!(nola_application::generative_ui::preview_ui_surface(&store, stale).is_err(), "reject revision {revision}");
    }
    for generation in [0, 9_007_199_254_740_992, u64::MAX] {
        let mut invalid_generation = preview_input.clone(); invalid_generation.draft_generation = generation;
        assert!(nola_application::generative_ui::preview_ui_surface(&store, invalid_generation).is_err(), "reject generation {generation}");
    }
    let mut action_in_preview = serde_json::to_value(&preview_input).unwrap();
    action_in_preview["action"] = serde_json::to_value(&button.actions["action"]).unwrap();
    assert!(serde_json::from_value::<UiDraftPreviewInput>(action_in_preview).is_err(), "a preview cannot carry an action");
    for phase in [UiSurfacePhase::ReadOnly, UiSurfacePhase::Deleted, UiSurfacePhase::Pending] {
        let mut closed = state.clone(); closed.phase = phase;
        assert!(matches!(nola_kernel::generative_ui::interaction::prepare_interaction(&closed,
            &UiInteraction { identity:snapshot.identity.clone(), expected_surface_revision:snapshot.revision,
                edits:preview_input.edits.clone(), action:None }), Err(UiError::SurfaceClosed)));
    }
    let mut foreign = preview_input.clone(); foreign.identity.owner.authority_id = "foreign".into();
    assert!(nola_application::generative_ui::preview_ui_surface(&store, foreign).is_err());
    assert!(nola_store_port::agent_queue::AgentQueueRepository::peek_agent_command(&mut store, CONVERSATION).unwrap().is_none());
    if let Ok(path) = std::env::var("NOLA_A2UI_PREVIEW_FIXTURE_OUTPUT") {
        std::fs::write(path, serde_json::to_vec_pretty(&json!({"initial":snapshot,"valid_input":preview_input,
            "valid_preview":valid,"invalid_input":invalid_input,"invalid_preview":invalid})).unwrap()).unwrap();
    }
}


#[test]
fn production_surface_order_survives_live_terminal_budget_deferral_and_real_rebase() {
    use nola_domain::MessagePartView;
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("surface-order.sqlite");
    let mut store = NolaStore::open(&database).unwrap();
    let initial = nola_application::begin_assistant_turn(&mut store, BeginAssistantTurn {
        expected_configuration_revision: None, expected_parent_message_id: None,
        conversation_id: CONVERSATION.into(), conversation_title: "Ordered UI".into(),
        user_message_id: "order:user".into(), user_part_id: "order:user:body".into(),
        user_text: "Render cards, then continue thinking and explaining".into(), user_content_json: None,
        assistant_message_id: "order:assistant".into(), context_checkpoint_id: None,
        core_config: config(), submission: AgentSubmission::UserInput {
            items: vec![AgentItem::user_message(ItemId::from_string("order:user".into()), "Render cards")],
            turn_context_items: vec![], turn_context_platform: Default::default(), turn_context_components: vec![],
            submission_id: Some("order".into()), submitted_at_epoch_ms: Some(1000), delivery: AgentInputDelivery::CurrentTurn,
        }, created_at_ms: 1000,
    }).unwrap();
    let run = &initial.begin.run_id;
    let digest = &initial.invocation_config_digest;
    let mut publications = Vec::new();
    // Actual model -> permission -> tool delivery -> kernel publication for
    // every card. Long readable summaries exhaust the public selection budget.
    for index in 0..8 {
        let id = format!("ordered-form-{index}");
        let call = json!({"call_id":format!("ordered-call-{index}"),"name":"render_ui","arguments":{
            "expected_revision":0,"messages":[
                {"version":"v0.9.1","createSurface":{"surfaceId":id,"catalogId":nola_kernel::generative_ui::schema::NOLA_CATALOG_ID}},
                {"version":"v0.9.1","updateComponents":{"surfaceId":id,"components":[
                    {"id":"root","component":"Text","text":"可公开的真实卡片正文。".repeat(200)}]}}
            ]}});
        publications.push(execute_form_call_with_reasoning(&mut store, run, digest, call,
            (index == 0).then_some("Think before publishing"), 1100 + index * 100).unwrap());
    }
    let mut request = pending(&mut store, run).into_iter().find(|value| matches!(value.payload, HostRequest::ModelStream { .. })).unwrap();
    support::admit_model_sampling(&mut store, run, digest, &mut request, 3000);
    let HostRequest::ModelStream { reasoning_part_id, text_part_id, .. } = &request.payload else { unreachable!() };
    let (reasoning, text) = (reasoning_part_id.clone(), text_part_id.clone());
    send(&mut store, run, digest, &mut request, HostResponse::ModelDelta {
        channel: ModelDeltaChannel::Reasoning, part_id: reasoning, delta: "Think after publishing".into(),
    }, 3001);
    send(&mut store, run, digest, &mut request, HostResponse::ModelCommittedDelta {
        part_id: text, delta: "The cards are ready.".into(),
    }, 3002);
    let audit = rusqlite::Connection::open(&database).unwrap();
    let mut statement = audit.prepare("SELECT id,content_json FROM message_parts WHERE message_id='order:assistant' AND kind='ui_surface' ORDER BY ordinal").unwrap();
    let stored: Vec<(String,UiSurfaceRevisionRef)> = statement.query_map([], |row| {
        let id: String = row.get(0)?;
        let raw: String = row.get(1)?;
        let MessagePartView::UiSurface { surface, .. } = serde_json::from_str(&raw).unwrap() else { panic!("not a Surface") };
        Ok((id,surface))
    }).unwrap().collect::<Result<_,_>>().unwrap();
    assert_eq!(stored.len(),8);
    let original_ids: Vec<(String,String)> = audit.prepare("SELECT id,kind FROM message_parts WHERE message_id='order:assistant' AND kind!='compaction' ORDER BY ordinal").unwrap()
        .query_map([],|row|Ok((row.get(0)?,row.get(1)?))).unwrap().collect::<Result<_,_>>().unwrap();
    let check = |store: &NolaStore, rebased: bool, original_ids: &[(String,String)]| {
        let projection = nola_application::load_conversation_projection_v1(store, CONVERSATION).unwrap();
        let assistant = projection.messages.iter().find(|message| message.message_id.0 == "order:assistant").unwrap();
        let ids: Vec<String> = assistant.parts.iter().map(|part| serde_json::to_value(part).unwrap()["part_id"].as_str().unwrap().to_owned()).collect();
        assert!(assistant.parts.iter().any(|part| matches!(part, MessagePartView::Historical { projection, .. }
            if projection.canonical_kind=="ui_surface")), "A legitimate public budget deferral must remain visible as history");
        for (id,kind) in original_ids {
            let at=ids.iter().position(|candidate|candidate==id).unwrap_or_else(||panic!("Original {kind} Part {id} disappeared after projection/rebase"));
            assert_eq!(ids.iter().filter(|candidate|*candidate==id).count(),1);
            if rebased && kind=="text" {
                match &assistant.parts[at] {
                    MessagePartView::Text {markdown,is_final,..} => {
                        assert!(*is_final,"Retired plain Text cannot become an active step");
                        let original:String=audit.query_row("SELECT content_text FROM message_parts WHERE id=?1",[id],|row|row.get(0)).unwrap();
                        assert!(original.is_empty() && markdown.is_empty(),"Only original empty plain placeholders take this path in this fixture");
                    }
                    MessagePartView::Historical {projection,..} => assert_eq!(projection.canonical_kind,"text"),
                    other => panic!("Retired Text became a different semantic kind: {other:?}"),
                }
                assert_ne!(projection.surface.current_activity_part_id.as_ref().map(|part|part.0.as_str()),Some(id.as_str()));
            }
            if rebased && kind=="reasoning" {
                let MessagePartView::Historical { projection, .. }=&assistant.parts[at] else { panic!("Retired checkpoint {kind} Part {id} must remain readonly history; actual={:?}",assistant.parts[at]) };
                assert_eq!(projection.owner.part_id(),id);
                assert_eq!(&projection.canonical_kind,kind);
                // The existing source reader either admits exact public bytes
                // or retains explicit deferred evidence. No unchecked fallback.
                assert!(projection.source.is_some() || matches!(projection.status,nola_domain::HistoricalProjectionStatus::NotLoaded { .. }));
            }
        }
        let selected:Vec<_>=ids.iter().filter(|id|original_ids.iter().any(|(original,_)|original==*id)).collect();
        assert_eq!(selected,original_ids.iter().map(|(id,_)|id).collect::<Vec<_>>());
        for (id,surface) in &stored {
            let at = ids.iter().position(|candidate| candidate==id).unwrap();
            assert!(at>0);
            assert_eq!(ids[at-1],surface.identity.owner.part_id,"Surface follows its exact origin, never the later prose");
            if rebased {
                assert!(matches!(&assistant.parts[at-1], MessagePartView::Historical { projection, .. } if projection.canonical_kind=="tool"),
                    "Missing checkpoint anchors are readonly historical facts, never reconstructed live Tools");
            }
        }
        if !rebased {
            let last_surface = ids.iter().position(|id| id==&stored.last().unwrap().0).unwrap();
            let after = assistant.parts.iter().position(|part| matches!(part, MessagePartView::Reasoning { summary:Some(summary), .. } if summary.contains("Think after publishing"))).unwrap();
            assert!(last_surface<after);
            let prose = assistant.parts.iter().position(|part| matches!(part, MessagePartView::Text { markdown, .. } if markdown.contains("The cards are ready."))).unwrap();
            assert!(after<prose);
        }
    };
    check(&store,false,&original_ids);
    send(&mut store, run, digest, &mut request, HostResponse::ModelCompleted { items:vec![
        serde_json::to_value(AgentItem::assistant_message(ItemId::from_string("order:answer".into()), "The cards are ready.")).unwrap()
    ],tool_calls:vec![],usage:json!({}) }, 3003);
    let original_ids: Vec<(String,String)> = audit.prepare("SELECT id,kind FROM message_parts WHERE message_id='order:assistant' AND kind!='compaction' ORDER BY ordinal").unwrap()
        .query_map([],|row|Ok((row.get(0)?,row.get(1)?))).unwrap().collect::<Result<_,_>>().unwrap();
    check(&store,false,&original_ids);
    // Reject a cross-message owner through the actual publication Store port;
    // the original data and every canonical order relation remain unchanged.
    use nola_store_port::generative_ui::GenerativeUiRepository;
    let mut wrong = store.read_ui_surface(&publications[0].identity).unwrap().unwrap();
    let previous_revision=wrong.revision;
    wrong.identity.owner.message_id = "order:user".into();
    wrong.revision+=1;
    store.begin_write_unit_of_work().unwrap();
    assert!(store.write_ui_surface(previous_revision,&wrong).is_err());
    store.rollback_write_unit_of_work().unwrap();
    assert!(store.read_ui_surface_revision(&wrong.identity,wrong.revision).unwrap().is_none());
    check(&store,false,&original_ids);
    nola_application::reduce_and_commit_idle_submission(&mut store, nola_application::ReduceAndCommitIdleSubmission {
        conversation_id:CONVERSATION.into(),core_config:config(),submission:AgentSubmission::RebaseConversation {
            items:vec![],item_ownership:Default::default(),
        },committed_at_ms:4000,
    }).unwrap();
    let checkpoint:String=audit.query_row("SELECT checkpoint_json FROM core_checkpoint_bodies WHERE conversation_id=?1",[CONVERSATION],|row|row.get(0)).unwrap();
    assert!(serde_json::from_str::<Value>(&checkpoint).unwrap()["state"]["conversation"]["projection"]["parts"].as_object().unwrap().is_empty());
    let before:i64=audit.query_row("SELECT count(*) FROM run_events",[],|row|row.get(0)).unwrap();
    check(&store,true,&original_ids);
    assert_eq!(audit.query_row::<i64,_,_>("SELECT count(*) FROM run_events",[],|row|row.get(0)).unwrap(),before,
        "Readonly history recovery must not replay tools or emit business events");
    drop(statement);drop(store);
    let reopened=NolaStore::open(&database).unwrap();
    check(&reopened,true,&original_ids);
    drop(audit);
}

#[test]
fn ordinary_history_without_surface_survives_rebase_budget_and_keeps_candidates_private() {
    use nola_agent_engine::agent::parts::{AgentPart,PartOwnership,TextPart,TextReviewState};
    use nola_store_port::transcript_write::{AppendMessagesInput,MessageInput};
    let directory=tempfile::tempdir().unwrap();let database=directory.path().join("ordinary-rebase.sqlite");
    let mut store=NolaStore::open(&database).unwrap();
    store.create_conversation(&nola_store::CreateConversationInput {id:CONVERSATION.into(),title:"Ordinary history".into(),
        revision:1,settings_json:None,created_at_ms:1,updated_at_ms:1}).unwrap();
    // Current protected archival records through the real transcript port.
    // These fixtures are not model inference or a fabricated review decision.
    let prefix="archive:assistant";
    let archived_answer="Public archived answer.\n".to_owned()+&"Approved public archival detail.\n".repeat(2000)+"ARCHIVED-PUBLIC-TAIL";
    let parts=[("approved",archived_answer.as_str(),TextReviewState::Approved),
        ("pending","PRIVATE-CANDIDATE-PENDING",TextReviewState::Pending),
        ("rejected","PRIVATE-CANDIDATE-REJECTED",TextReviewState::Rejected),
        ("interrupted","PRIVATE-CANDIDATE-INTERRUPTED",TextReviewState::Interrupted)]
        .into_iter().enumerate().map(|(ordinal,(id,text,review))| {
            let part=AgentPart::Text(TextPart {id:format!("archive:{id}"),ownership:PartOwnership {
                message_id:prefix.into(),turn_id:"archive-turn".into()},text:text.into(),is_complete:true,review:Some(review)});
            nola_domain::MessagePartInput {id:part.id().into(),kind:"text".into(),ordinal:ordinal as i64+1,
                state:"completed".into(),content_text:text.into(),content_json:Some(serde_json::to_string(&part).unwrap()),
                content_digest:None,created_at_ms:1,updated_at_ms:1}
        }).collect();
    store.append_messages(&AppendMessagesInput {conversation_id:CONVERSATION.into(),expected_message_count:0,
        messages:vec![MessageInput {id:prefix.into(),run_id:None,role:"assistant".into(),status:"completed".into(),ordinal:1,
            created_at_ms:1,completed_at_ms:Some(1),parts}],updated_at_ms:1}).unwrap();
    let existing=store.load_conversation(CONVERSATION).unwrap().unwrap();
    let initial=nola_application::begin_assistant_turn(&mut store,BeginAssistantTurn {
        expected_configuration_revision:Some(existing.configuration_revision),expected_parent_message_id:existing.messages.last().map(|message|message.id.clone()),conversation_id:CONVERSATION.into(),
        conversation_title:"Ordinary history".into(),user_message_id:"ordinary:user".into(),user_part_id:"ordinary:user:body".into(),
        user_text:"Continue with a public answer".into(),user_content_json:None,assistant_message_id:"ordinary:assistant".into(),
        context_checkpoint_id:None,core_config:config(),submission:AgentSubmission::UserInput {
            items:vec![AgentItem::user_message(ItemId::from_string("ordinary:user".into()),"Continue with a public answer")],
            turn_context_items:vec![],turn_context_platform:Default::default(),turn_context_components:vec![],
            submission_id:Some("ordinary".into()),submitted_at_epoch_ms:Some(1000),delivery:AgentInputDelivery::CurrentTurn,
        },created_at_ms:1000,
    }).unwrap();
    let run=&initial.begin.run_id;let digest=&initial.invocation_config_digest;
    let mut request=pending(&mut store,run).into_iter().find(|request|matches!(request.payload,HostRequest::ModelStream {..})).unwrap();
    support::admit_model_sampling(&mut store,run,digest,&mut request,1100);
    let HostRequest::ModelStream {reasoning_part_id,text_part_id,..}=&request.payload else {unreachable!()};
    let (reasoning,text)=(reasoning_part_id.clone(),text_part_id.clone());
    let answer="A public answer with a bounded historical preview. ".repeat(1200)+"PUBLIC-TAIL";
    send(&mut store,run,digest,&mut request,HostResponse::ModelDelta {channel:ModelDeltaChannel::Reasoning,
        part_id:reasoning,delta:"Current public reasoning".into()},1101);
    send(&mut store,run,digest,&mut request,HostResponse::ModelCommittedDelta {part_id:text,delta:answer.clone()},1102);
    let live=nola_application::load_conversation_projection_v1(&store,CONVERSATION).unwrap();
    assert!(!serde_json::to_string(&live).unwrap().contains("PRIVATE-CANDIDATE"));
    assert!(live.active_run.is_some());
    send(&mut store,run,digest,&mut request,HostResponse::ModelCompleted {items:vec![
        serde_json::to_value(AgentItem::assistant_message(ItemId::from_string("ordinary:answer".into()),answer.clone())).unwrap()
    ],tool_calls:vec![],usage:json!({})},1103);
    // The intentionally long repetitive answer exercises public body budget,
    // then the real final-response contract requests a synthesis successor.
    // Complete that actual model request through normal sampling/response ports.
    assert!(pending(&mut store,run).iter().any(|request|matches!(request.payload,HostRequest::ModelStream {..})));
    model_reply(&mut store,run,digest,vec![],Some("The public answer is complete."),1110);
    assert!(nola_application::load_conversation_projection_v1(&store,CONVERSATION).unwrap().active_run.is_none(), "The real synthesis successor must seal before idle rebase");
    let audit=rusqlite::Connection::open(&database).unwrap();
    let ids:Vec<String>=audit.prepare("SELECT p.id FROM message_parts p JOIN messages m ON m.id=p.message_id WHERE m.conversation_id=?1 AND m.role='assistant' AND p.kind!='compaction' ORDER BY m.ordinal,p.ordinal").unwrap()
        .query_map([CONVERSATION],|row|row.get(0)).unwrap().collect::<Result<_,_>>().unwrap();
    nola_application::reduce_and_commit_idle_submission(&mut store,nola_application::ReduceAndCommitIdleSubmission {
        conversation_id:CONVERSATION.into(),core_config:config(),submission:AgentSubmission::RebaseConversation {
            items:vec![],item_ownership:Default::default()},committed_at_ms:1200,
    }).unwrap();
    drop(store);
    let store=NolaStore::open(&database).unwrap();
    let projection=nola_application::load_conversation_projection_v1(&store,CONVERSATION).unwrap();
    let restored:Vec<String>=projection.messages.iter().filter(|message|message.role==nola_domain::MessageRole::Assistant)
        .flat_map(|message|message.parts.iter().map(|part|serde_json::to_value(part).unwrap()["part_id"].as_str().unwrap().to_owned())).collect();
    assert_eq!(restored,ids,"Ordinary history must not disappear merely because no Surface is present");
    let serialized=serde_json::to_string(&projection).unwrap();
    assert!(!serialized.contains("PRIVATE-CANDIDATE"),"Archived candidate bodies remain withheld after real rebase and cold load");
    assert!(serialized.contains("Public archived answer"));
    assert!(!serialized.contains("PUBLIC-TAIL"),"Historical budget must not be lifted to disclose the whole large body inline");
    let archived=projection.messages.iter().find(|message|message.message_id.0==prefix).unwrap()
        .parts.iter().find(|part|matches!(part,nola_domain::MessagePartView::Historical {part_id,..} if part_id.0=="archive:approved")).unwrap();
    let nola_domain::MessagePartView::Historical {projection:history,..}=archived else {unreachable!()};
    assert_eq!(history.canonical_kind,"text");
    assert!(history.source.is_some());
    // An unbound archived AgentPart is not promoted into a live typed Part.
    // Its independently approved canonical-text preview has its own real
    // bounded disclosure contract, even when JSON semantics are unsupported.
    assert!(matches!(history.status,nola_domain::HistoricalProjectionStatus::Unsupported {..}));
    let preview=history.canonical_text.as_ref().unwrap();
    assert_eq!(preview.total_utf8_bytes,archived_answer.len() as u64);
    assert!(preview.text.len()<archived_answer.len());
    assert!(preview.text.starts_with("Public archived answer."));
    assert!(!preview.text.contains("ARCHIVED-PUBLIC-TAIL"));
    assert!(projection.active_run.is_none());
}

#[test]
fn unknown_permission_stop_preserves_uncertainty_and_rejects_independent_input() {
    for independent_input in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("unknown-permission-stop.db");
        let mut store = NolaStore::open(&database).unwrap();
        let mut configuration = support::goal_execution::with_execution_binding(config());
        for capability in ["permission.ask", "render_ui"] {
            configuration.host_catalog.as_mut().unwrap().capabilities.get_mut(capability)
                .unwrap().replay_safety = HostReplaySafety::RequiresRecoveryDecision;
        }
        let begun = nola_application::begin_assistant_turn(&mut store, BeginAssistantTurn {
            expected_configuration_revision: None, expected_parent_message_id: None,
            conversation_id: CONVERSATION.into(), conversation_title: "Unknown permission".into(),
            user_message_id: "unknown-user".into(), user_part_id: "unknown-user:body".into(),
            user_text: "Publish the native fixture".into(), user_content_json: None,
            assistant_message_id: "unknown:assistant".into(), context_checkpoint_id: None,
            core_config: configuration, submission: AgentSubmission::UserInput {
                items: vec![AgentItem::user_message(ItemId::from_string("unknown-user".into()), "Publish the native fixture")],
                turn_context_items: vec![], turn_context_platform: Default::default(), turn_context_components: vec![],
                submission_id: Some("unknown-permission-run".into()), submitted_at_epoch_ms: Some(1000),
                delivery: AgentInputDelivery::CurrentTurn,
            }, created_at_ms: 1000,
        }).unwrap();
        let run = &begun.begin.run_id;
        let digest = &begun.invocation_config_digest;
        model_reply(&mut store, run, digest, vec![ui_call(1)], None, 1100);
        let ask = pending(&mut store, run).into_iter().find(|request|
            matches!(request.payload, HostRequest::RequestPermission { .. })).unwrap();
        let mut ask = start(&mut store, run, digest, &ask, 1200);
        assert_eq!(store.effect_delivery_attempt(&ask.delivery.as_ref().unwrap().attempt_id).unwrap().unwrap().replay_safety,
            nola_domain::EffectDeliveryReplaySafety::RequiresRecoveryDecision);
        if independent_input {
            nola_application::reduce_and_commit_active_submission(&mut store,
                nola_application::ReduceAndCommitActiveSubmission {
                    conversation_id: CONVERSATION.into(), run_id: run.clone(),
                    invocation_config_digest: digest.clone(), transition_id: "independent-input".into(),
                    submission: AgentSubmission::UserInput {
                        items: vec![AgentItem::user_message(ItemId::from_string("independent-user".into()), "Separate pending work")],
                        turn_context_items: vec![], turn_context_platform: Default::default(), turn_context_components: vec![],
                        submission_id: Some("independent-submission".into()), submitted_at_epoch_ms: Some(1300),
                        delivery: AgentInputDelivery::CurrentTurn,
                    }, committed_at_ms: 1300,
                }).unwrap();
        }
        send(&mut store, run, digest, &mut ask, HostResponse::PermissionTransportInterrupted {
            reason: nola_host_protocol::PermissionTransportInterruptionReason::Disconnected,
        }, 1400);
        drop(store);
        let mut runtime = support::open_runtime(&database).unwrap();
        let inspection: nola_protocol::UnknownEffectRecoveryInspection = serde_json::from_value(
            runtime.execute(ApplicationCommand::InspectUnknownEffectRecovery {
                conversation_id: CONVERSATION.into(), expected_run_id: run.clone(),
                assistant_message_id: "unknown:assistant".into(),
            }).unwrap()).unwrap();
        assert_eq!(inspection.targets.len(), 1);
        assert_eq!(inspection.targets[0].effect_id, ask.effect_id.0);
        let command = ApplicationCommand::ResolveUnknownEffectRecovery {
            input: nola_application::ResolveUnknownEffectRecovery {
                conversation_id: CONVERSATION.into(), run_id: run.clone(), invocation_config_digest: digest.clone(),
                resolution_id: "stop-preserving-unknown-permission".into(), targets: inspection.targets,
                choice: nola_application::UnknownEffectRecoveryChoice::KeepUnknownBlocked {
                    reason: "Close this fixture without retrying or claiming its permission outcome".into(),
                }, resolved_at_ms: 1500,
            },
        };
        let command_json = serde_json::to_string(&command).unwrap();
        let first = runtime.execute(command);
        if independent_input {
            assert!(first.is_err(), "Other admitted user input must not be withdrawn by this decision");
            let state = runtime.execute(ApplicationCommand::LoadHostExecutionState { conversation_id: CONVERSATION.into() }).unwrap();
            assert_eq!(state["permissions"]["pending"].as_array().unwrap().len(), 1,
                "A rejected stop must leave its permission question intact");
            continue;
        }
        let first = first.expect("The target's own permission question is not independent work");
        assert_eq!(first["outcome"], "unknown_preserved_blocked");
        let state = runtime.execute(ApplicationCommand::LoadHostExecutionState { conversation_id: CONVERSATION.into() }).unwrap();
        assert!(state["permissions"]["pending"].as_array().unwrap().is_empty());
        assert_eq!(state["permissions"]["receipts"].as_array().unwrap().last().unwrap()["outcome"]["type"], "interrupted");
        assert_eq!(runtime.execute(serde_json::from_str(&command_json).unwrap()).unwrap(), first);
        drop(runtime);
        let mut runtime = support::open_runtime(&database).unwrap();
        assert_eq!(runtime.execute(serde_json::from_str(&command_json).unwrap()).unwrap(), first);
        drop(runtime);
        let mut store = NolaStore::open(&database).unwrap();
        let HostRequest::RequestPermission { permission_request_id, .. } = &ask.payload else { unreachable!() };
        assert!(nola_application::reduce_and_accept_host_response(&mut store, ReduceAndAcceptHostResponse {
            conversation_id: CONVERSATION.into(), run_id: run.clone(), invocation_config_digest: digest.clone(),
            response: response(&ask, HostResponse::PermissionResolved {
                request_id: permission_request_id.clone(), resolution: HostPermissionResolution::allow_once(),
            }), received_at_ms: 1501,
        }).is_err(), "An old worker's late approval must not reopen the stopped owner, even before lease expiry");
        let attempt = store.effect_delivery_attempt(&ask.delivery.as_ref().unwrap().attempt_id).unwrap().unwrap();
        assert_eq!(attempt.state, nola_domain::EffectDeliveryAttemptState::OutcomeUnknown);
        let db = rusqlite::Connection::open(&database).unwrap();
        let result: String = db.query_row("SELECT result_json FROM run_results WHERE run_id=?1", [run], |row| row.get(0)).unwrap();
        let result: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(result["value"]["kind"], "blocked");
        assert!(!result["value"]["blocking_fact_refs"].as_array().unwrap().is_empty());
        let attempts: i64 = db.query_row("SELECT COUNT(*) FROM effect_delivery_attempts WHERE effect_id=?1", [&ask.effect_id.0], |row| row.get(0)).unwrap();
        assert_eq!(attempts, 1, "Stopping must not create a retry delivery");
    }
}
