#[path = "support/model_identity.rs"]
mod model_identity;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

use nola_core::agent::host::{
    CapabilityAvailability, HostCancellationMode, HostCapabilityCatalog, HostCapabilityDescriptor,
    HostCapabilityExposure, HostCapabilityParallelism, HostCapabilityRoute, HostOutputPolicy,
    HostReplaySafety, HostRequestEnvelope, HostResponseEnvelope,
};
use nola_core::agent::{
    AgentCore, AgentCoreConfig, AgentInputDelivery, AgentItem, AgentSubmission, ItemId,
    PermissionRequestId, PermissionUserDecision, SessionId,
};
use serde_json::{json, Value};

const FIXTURE_JSON: &str = include_str!("fixtures/canonical_part_lifecycle_v1.json");

fn capability(
    id: &str,
    route: HostCapabilityRoute,
    permission_domain: Option<&str>,
    input_schema: Value,
) -> HostCapabilityDescriptor {
    HostCapabilityDescriptor {
        id: id.to_string(),
        title: id.to_string(),
        route,
        availability: CapabilityAvailability::Available,
        exposure: HostCapabilityExposure::Visible,
        permission_domain: permission_domain.map(str::to_string),
        input_schema,
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

fn host_catalog() -> HostCapabilityCatalog {
    let mut catalog = HostCapabilityCatalog::empty("canonical-fixture-host");
    catalog.revision = 1;
    catalog.availability_epoch = 1;
    for descriptor in [
        capability(
            "model.stream",
            HostCapabilityRoute::Model,
            None,
            json!({"type": "object"}),
        ),
        capability(
            "permission.ask",
            HostCapabilityRoute::Permission,
            None,
            json!({"type": "object"}),
        ),
        capability(
            "host.cancel",
            HostCapabilityRoute::Tool,
            None,
            json!({"type": "object"}),
        ),
        capability(
            "web_search",
            HostCapabilityRoute::Tool,
            Some("network.read"),
            json!({
                "type": "object",
                "properties": { "query": { "type": "string" } },
                "required": ["query"],
                "additionalProperties": false
            }),
        ),
    ] {
        catalog
            .capabilities
            .insert(descriptor.id.clone(), descriptor);
    }
    catalog
}

fn payload_kind(value: &Value) -> Option<&str> {
    value
        .get("payload")
        .and_then(|payload| payload.get("type").or_else(|| payload.get("kind")))
        .and_then(Value::as_str)
}

fn request_of_kind(
    transition: &nola_core::agent::AgentTransition,
    kind: &str,
) -> HostRequestEnvelope {
    transition
        .host_requests
        .iter()
        .find(|request| {
            serde_json::to_value(request)
                .ok()
                .and_then(|value| payload_kind(&value).map(ToOwned::to_owned))
                .as_deref()
                == Some(kind)
        })
        .map(|request| {
            serde_json::from_value(serde_json::to_value(request).expect("serialize request"))
                .expect("clone request through its stable wire contract")
        })
        .unwrap_or_else(|| {
            let available = transition
                .host_requests
                .iter()
                .filter_map(|request| serde_json::to_value(request).ok())
                .filter_map(|value| payload_kind(&value).map(ToOwned::to_owned))
                .collect::<Vec<_>>();
            let snapshot = serde_json::to_value(&transition.snapshot)
                .unwrap_or_else(|_| json!({"error": "snapshot serialization failed"}));
            panic!(
                "transition must contain {kind} Host request; available requests: {available:?}; snapshot: {snapshot}"
            )
        })
}

fn request_where(
    transition: &nola_core::agent::AgentTransition,
    predicate: impl Fn(&str) -> bool,
) -> HostRequestEnvelope {
    transition
        .host_requests
        .iter()
        .find(|request| {
            serde_json::to_value(request)
                .ok()
                .and_then(|value| payload_kind(&value).map(ToOwned::to_owned))
                .is_some_and(|kind| predicate(&kind))
        })
        .map(|request| {
            serde_json::from_value(serde_json::to_value(request).expect("serialize request"))
                .expect("clone request through its stable wire contract")
        })
        .expect("transition must contain matching Host request")
}

fn response_for(
    request: &HostRequestEnvelope,
    sequence: u64,
    payload: Value,
) -> HostResponseEnvelope {
    let mut value = serde_json::to_value(request).expect("serialize Host request envelope");
    let object = value.as_object_mut().expect("Host envelope is an object");
    object.insert("sequence".to_string(), json!(sequence));
    object.insert("payload".to_string(), payload);
    serde_json::from_value(value).expect("request identity must form a correlated Host response")
}

fn record(
    core: &mut AgentCore,
    events: &mut Vec<Value>,
    submission: AgentSubmission,
) -> nola_core::agent::AgentTransition {
    // Exercise the production observation-time boundary with a reproducible
    // clock, including the real ToolPart.started_at_ms event field.
    let observed_at_ms = 1_700_000_000_000 + events.len() as u64;
    let transition = core
        .submit_at(submission, observed_at_ms)
        .expect("canonical Core transition");
    events.extend(
        transition
            .event_envelopes
            .iter()
            .map(|event| serde_json::to_value(event).expect("serialize Core event envelope")),
    );
    transition
}

fn host_response(
    core: &mut AgentCore,
    events: &mut Vec<Value>,
    request: &HostRequestEnvelope,
    sequence: u64,
    payload: Value,
) -> nola_core::agent::AgentTransition {
    record(
        core,
        events,
        AgentSubmission::HostResponse {
            response: response_for(request, sequence, payload),
        },
    )
}

fn part_events(events: &[Value]) -> impl Iterator<Item = &Value> {
    events.iter().filter(|envelope| {
        envelope
            .get("event")
            .and_then(|event| event.get("type"))
            .and_then(Value::as_str)
            == Some("part_updated")
    })
}

fn part_id(event_envelope: &Value) -> &str {
    event_envelope["event"]["part"]["id"]
        .as_str()
        .expect("PartUpdated.part.id")
}

fn generate_fixture() -> Value {
    let mut core = AgentCore::try_new(AgentCoreConfig {
        turn_identity: model_identity::host_identity(),
        session_id: Some(SessionId::from_string(
            "session-part-lifecycle-v1".to_string(),
        )),
        model: Some("fixture-model".to_string()),
        tools: vec![json!({
            "type": "function",
            "function": {
                "name": "web_search",
                "description": "Return cited Web evidence.",
                "parameters": {
                    "type": "object",
                    "properties": { "query": { "type": "string" } },
                    "required": ["query"],
                    "additionalProperties": false
                }
            }
        })],
        host_catalog: Some(host_catalog()),
        ..AgentCoreConfig::default()
    })
    .expect("create canonical Agent Core");

    let mut events = Vec::new();

    let submitted = record(
        &mut core,
        &mut events,
        AgentSubmission::UserInput {
            items: vec![AgentItem::user_message(
                ItemId::from_string("user-fixture-complete".to_string()),
                "Find one source and explain the Core contract.",
            )],
            turn_context_items: Vec::new(),
            turn_context_platform: Default::default(),
            turn_context_components: Vec::new(),
            submission_id: Some("submission-fixture-complete".to_string()),
            submitted_at_epoch_ms: None,
            delivery: AgentInputDelivery::CurrentTurn,
        },
    );
    let first_model = request_of_kind(&submitted, "model_stream");
    let first_model_value = serde_json::to_value(&first_model).expect("first model request");
    let first_reasoning_part_id = first_model_value["payload"]["reasoning_part_id"]
        .as_str()
        .expect("reasoning part id");
    let first_text_part_id = first_model_value["payload"]["text_part_id"]
        .as_str()
        .expect("first text part id");

    host_response(
        &mut core,
        &mut events,
        &first_model,
        1,
        json!({
            "type": "model_delta",
            "channel": "reasoning",
            "part_id": first_reasoning_part_id,
            "delta": "Inspect the available evidence before answering."
        }),
    );
    host_response(
        &mut core,
        &mut events,
        &first_model,
        2,
        json!({
            "type": "model_delta",
            "channel": "text",
            "part_id": first_text_part_id,
            "delta": "A provisional answer that must be replaced before the tool round."
        }),
    );
    let permission_pending = host_response(
        &mut core,
        &mut events,
        &first_model,
        3,
        json!({
            "type": "model_completed",
            "items": [
                {
                    "type": "reasoning",
                    "id": "reasoning-fixture-complete",
                    "summary": "Inspect evidence",
                    "content": "Inspect the available evidence before answering."
                }
            ],
            "tool_calls": [{
                "call_id": "call-fixture-search",
                "name": "web_search",
                "arguments": { "query": "Nola Agent Core contract" },
                "permission_scope": "network.read",
                "permission_rationale": "Retrieve one public source for the answer."
            }],
            "usage": {
                "input_tokens": 120,
                "output_tokens": 24,
                "reasoning_tokens": 12,
                "total_tokens": 156
            }
        }),
    );
    let permission_request = request_of_kind(&permission_pending, "request_permission");
    let permission_value = serde_json::to_value(&permission_request).expect("permission request");
    let request_id = permission_value["payload"]["permission_request_id"]
        .as_str()
        .expect("permission request id")
        .to_string();

    let permission_resolved = record(
        &mut core,
        &mut events,
        AgentSubmission::PermissionResponse {
            request_id: PermissionRequestId::from_string(request_id),
            decision: PermissionUserDecision::AllowOnce,
            grant: None,
        },
    );
    let tool_request = request_of_kind(&permission_resolved, "execute_tool");
    host_response(
        &mut core,
        &mut events,
        &tool_request,
        1,
        json!({
            "type": "tool_progress",
            "title": "Searching",
            "detail": "Nola Agent Core contract",
            "metadata": { "phase": "retrieval" }
        }),
    );
    let tool_completed = host_response(
        &mut core,
        &mut events,
        &tool_request,
        2,
        json!({
            "type": "tool_completed",
            "call_id": "call-fixture-search",
            "output": {
                "results": [{
                    "title": "Agent Core contract",
                    "url": "https://example.com/nola/core-contract/",
                    "snippet": "Core owns stable Part identity and terminal lifecycle."
                }]
            },
            "outcome": "succeeded",
            "full_output_ref": null,
            "artifacts": []
        }),
    );
    let second_model = request_of_kind(&tool_completed, "model_stream");
    let second_model_value = serde_json::to_value(&second_model).expect("second model request");
    let second_text_part_id = second_model_value["payload"]["text_part_id"]
        .as_str()
        .expect("second text part id");
    host_response(
        &mut core,
        &mut events,
        &second_model,
        1,
        json!({
            "type": "model_delta",
            "channel": "text",
            "part_id": second_text_part_id,
            "delta": "## Verified Result\n\n"
        }),
    );
    host_response(
        &mut core,
        &mut events,
        &second_model,
        2,
        json!({
            "type": "model_delta",
            "channel": "text",
            "part_id": second_text_part_id,
            "delta": "The evidence confirms that Core owns stable Part identity, lifecycle, and citations."
        }),
    );
    host_response(
        &mut core,
        &mut events,
        &second_model,
        3,
        json!({
            "type": "model_completed",
            "items": [{
                "type": "message",
                "id": "assistant-fixture-complete",
                "role": "assistant",
                "content": "## Verified Result\n\nThe evidence confirms that Core owns stable Part identity, lifecycle, and citations."
            }],
            "tool_calls": [],
            "usage": {
                "input_tokens": 210,
                "output_tokens": 18,
                "reasoning_tokens": 0,
                "total_tokens": 228
            }
        }),
    );
    let cancel_submitted = record(
        &mut core,
        &mut events,
        AgentSubmission::UserInput {
            items: vec![AgentItem::user_message(
                ItemId::from_string("user-fixture-cancel".to_string()),
                "Start another answer, then cancel it.",
            )],
            turn_context_items: Vec::new(),
            turn_context_platform: Default::default(),
            turn_context_components: Vec::new(),
            submission_id: Some("submission-fixture-cancel".to_string()),
            submitted_at_epoch_ms: None,
            delivery: AgentInputDelivery::CurrentTurn,
        },
    );
    let cancelled_model = request_of_kind(&cancel_submitted, "model_stream");
    let cancelled_model_value =
        serde_json::to_value(&cancelled_model).expect("cancelled model request");
    let cancelled_text_part_id = cancelled_model_value["payload"]["text_part_id"]
        .as_str()
        .expect("cancelled text part id");
    host_response(
        &mut core,
        &mut events,
        &cancelled_model,
        1,
        json!({
            "type": "model_delta",
            "channel": "text",
            "part_id": cancelled_text_part_id,
            "delta": "This partial answer must remain attached to the cancelled Turn."
        }),
    );
    let cancelling = record(
        &mut core,
        &mut events,
        AgentSubmission::Interrupt { goal_control: None, cancellation_id: "test-cancellation".into(),
            reason: Some("canonical fixture cancellation".to_string()),
        },
    );
    let cancel_request = request_where(&cancelling, |kind| kind.contains("cancel"));
    let cancel_value = serde_json::to_value(&cancel_request).expect("cancel request");
    let target_effect_id = cancel_value["payload"]["target_effect_id"]
        .as_str()
        .expect("cancel target effect id")
        .to_string();
    host_response(
        &mut core,
        &mut events,
        &cancel_request,
        1,
        json!({
            "type": "cancellation_confirmed",
            "target_effect_id": target_effect_id
        }),
    );

    let before_late_snapshot =
        serde_json::to_value(core.snapshot()).expect("snapshot before late response");
    let event_count_before_late = events.len();
    let late_response = response_for(
        &cancelled_model,
        2,
        json!({
            "type": "model_delta",
            "channel": "text",
            "part_id": cancelled_text_part_id,
            "delta": "LATE CONTENT MUST NOT APPEAR"
        }),
    );
    let late_response_wire = serde_json::to_value(&late_response).expect("late response wire");
    let late_result = core.submit(AgentSubmission::HostResponse {
        response: late_response,
    });
    let late_outcome = match late_result {
        Ok(transition) => {
            assert!(
                transition.event_envelopes.is_empty(),
                "late response emitted Core events"
            );
            json!({ "accepted_as_noop": true, "emitted_event_count": 0 })
        }
        Err(_) => json!({ "accepted_as_noop": false, "emitted_event_count": 0 }),
    };
    assert_eq!(events.len(), event_count_before_late);
    assert_eq!(
        serde_json::to_value(core.snapshot()).expect("snapshot after late response"),
        before_late_snapshot,
        "late response must not mutate public Core state"
    );

    let mut order = Vec::new();
    let mut revisions = BTreeMap::new();
    for event in part_events(&events) {
        let id = part_id(event).to_string();
        if !order.contains(&id) {
            order.push(id.clone());
        }
        revisions.insert(
            id,
            event["event"]["projection_revision"]
                .as_u64()
                .expect("projection revision"),
        );
    }

    json!({
        "schema_version": 1,
        "fixture_id": "canonical-part-lifecycle-v1",
        "generated_by": "nola_core::agent::AgentCore",
        "script": [
            "submit user input and create assistant shell",
            "stream reasoning",
            "complete model round with permission-gated web_search",
            "allow once and run tool with progress",
            "complete tool and project source/citation",
            "replace the tool-round preamble and stream the canonical answer",
            "complete Turn",
            "start second Turn and stream partial text",
            "interrupt and acknowledge cancellation",
            "submit late model delta and prove no mutation"
        ],
        "event_envelopes": events,
        "part_order": order,
        "latest_projection_revision_by_part": revisions,
        "late_response_probe": {
            "response": late_response_wire,
            "outcome": late_outcome,
            "snapshot_unchanged": true
        }
    })
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/canonical_part_lifecycle_v1.json")
}

fn event_type(envelope: &Value) -> Option<&str> {
    envelope["event"]["type"].as_str()
}

fn part_type(envelope: &Value) -> Option<&str> {
    envelope["event"]["part"]
        .get("type")
        .or_else(|| envelope["event"]["part"].get("kind"))
        .and_then(Value::as_str)
}

fn update_type(envelope: &Value) -> Option<&str> {
    envelope["event"]["update"]
        .get("type")
        .or_else(|| envelope["event"]["update"].get("kind"))
        .and_then(Value::as_str)
}

fn verify_fixture(fixture: &Value) {
    assert_eq!(fixture["generated_by"], "nola_core::agent::AgentCore");
    let events = fixture["event_envelopes"]
        .as_array()
        .expect("event_envelopes array");
    assert!(!events.is_empty());

    let mut last_sequence = 0;
    let mut last_revision = 0;
    let mut terminal_by_turn = BTreeMap::<String, usize>::new();
    for envelope in events {
        let sequence = envelope["cursor"]["sequence"]
            .as_u64()
            .expect("event cursor sequence");
        assert_eq!(sequence, last_sequence + 1, "Core envelope sequence gap");
        last_sequence = sequence;
        let revision = envelope["revision"].as_u64().expect("journal revision");
        assert!(
            revision >= last_revision,
            "journal revision moved backwards"
        );
        last_revision = revision;

        if matches!(
            event_type(envelope),
            Some("turn_completed" | "turn_cancelled" | "turn_failed")
        ) {
            let turn = envelope["event"]["turn_id"]
                .as_str()
                .expect("terminal turn id")
                .to_string();
            *terminal_by_turn.entry(turn).or_default() += 1;
        }
    }
    assert_eq!(terminal_by_turn.len(), 2);
    assert!(terminal_by_turn.values().all(|count| *count == 1));

    let parts = part_events(events).collect::<Vec<_>>();
    assert!(!parts.is_empty());
    let mut last_projection_revision = 0;
    let mut projection_revision_advanced = false;
    for envelope in &parts {
        let projection_revision = envelope["event"]["projection_revision"]
            .as_u64()
            .expect("projection revision");
        assert!(
            projection_revision >= last_projection_revision,
            "Part projection revision moved backwards"
        );
        projection_revision_advanced |= projection_revision > last_projection_revision;
        last_projection_revision = projection_revision;
        let turn_id = envelope["event"]["turn_id"].as_str().expect("turn id");
        assert_eq!(
            envelope["event"]["part"]["turn_id"].as_str(),
            Some(turn_id),
            "Part ownership turn mismatch"
        );
        assert_eq!(
            envelope["event"]["part"]["message_id"].as_str(),
            Some(format!("{turn_id}:assistant").as_str()),
            "Part ownership message mismatch"
        );
    }
    assert!(
        projection_revision_advanced,
        "Part projection never advanced"
    );

    let has_part = |kind: &str| parts.iter().any(|event| part_type(event) == Some(kind));
    assert!(has_part("reasoning"));
    assert!(has_part("tool"));
    assert!(has_part("source"));
    assert!(has_part("citation"));
    assert!(has_part("text"));

    let mut reasoning_ids_by_content = BTreeMap::<(String, String), BTreeSet<String>>::new();
    for event in parts
        .iter()
        .filter(|event| part_type(event) == Some("reasoning"))
    {
        let detail = event["event"]["part"]["detail"]
            .as_str()
            .unwrap_or_default();
        if detail.is_empty() {
            continue;
        }
        let turn_id = event["event"]["turn_id"]
            .as_str()
            .expect("reasoning turn id");
        reasoning_ids_by_content
            .entry((turn_id.to_string(), detail.to_string()))
            .or_default()
            .insert(part_id(event).to_string());
    }
    assert!(
        reasoning_ids_by_content
            .values()
            .all(|part_ids| part_ids.len() == 1),
        "streamed and completed copies of the same reasoning must reconcile into one canonical Part"
    );

    let tool_states = parts
        .iter()
        .filter(|event| part_type(event) == Some("tool"))
        .filter_map(|event| event["event"]["part"]["state"].as_str())
        .collect::<Vec<_>>();
    assert!(tool_states.contains(&"waiting_for_permission"));
    assert!(tool_states.contains(&"running"));
    assert!(tool_states.contains(&"completed"));

    assert!(parts
        .iter()
        .any(|event| update_type(event) == Some("text_delta")));
    assert!(parts
        .iter()
        .any(|event| update_type(event) == Some("text_replaced")));
    assert_eq!(fixture["late_response_probe"]["snapshot_unchanged"], true);
    assert_eq!(
        fixture["late_response_probe"]["outcome"]["emitted_event_count"],
        0
    );
}

#[test]
fn canonical_part_lifecycle_fixture_is_generated_by_core_without_drift() {
    let generated = generate_fixture();
    if std::env::var_os("NOLA_UPDATE_CANONICAL_PART_FIXTURE").is_some() {
        fs::write(
            fixture_path(),
            format!(
                "{}\n",
                serde_json::to_string_pretty(&generated).expect("pretty fixture JSON")
            ),
        )
        .expect("write canonical fixture");
        return;
    }

    let committed: Value = serde_json::from_str(FIXTURE_JSON).expect("decode committed fixture");
    let generated: Value = serde_json::from_str(
        &serde_json::to_string(&generated).expect("encode generated canonical fixture"),
    )
    .expect("normalize generated canonical fixture");
    assert_eq!(
        committed, generated,
        "canonical fixture drifted; review the Core contract, then regenerate with \
         NOLA_UPDATE_CANONICAL_PART_FIXTURE=1 cargo test --test \
         canonical_part_lifecycle_fixture canonical_part_lifecycle_fixture_is_generated_by_core_without_drift"
    );
    verify_fixture(&committed);
}

#[test]
fn committed_canonical_part_lifecycle_fixture_satisfies_identity_and_terminal_contract() {
    let committed: Value = serde_json::from_str(FIXTURE_JSON).expect("decode committed fixture");
    verify_fixture(&committed);
}
