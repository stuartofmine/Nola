// SPDX-License-Identifier: Apache-2.0
// Process-level contract coverage for crash recovery of pending Host effects.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use nola_core::agent::host::{
    CapabilityAvailability, HostCancellationMode, HostCapabilityCatalog, HostCapabilityDescriptor,
    HostCapabilityExposure, HostCapabilityParallelism, HostCapabilityRoute, HostId, HostLease,
    HostOutputPolicy, HostReplaySafety, HostRequest, HostRequestEnvelope, HostResponse,
    HostResponseEnvelope, ModelDeltaChannel,
};
use nola_core::agent::recovery::{CheckpointReason, CoreCheckpoint};
use nola_core::agent::{
    AgentCore, AgentCoreConfig, AgentEvent, AgentInputDelivery, AgentItem, AgentSubmission,
    AgentThreadState, ContextPlatform, ItemId, SessionId,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const CHILD_MODE: &str = "NOLA_PENDING_EFFECT_CHILD_MODE";
const CHILD_SCENARIO: &str = "NOLA_PENDING_EFFECT_SCENARIO";
const CHILD_FIXTURE: &str = "NOLA_PENDING_EFFECT_FIXTURE";
const PROCESS_TIMEOUT: Duration = Duration::from_secs(12);

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Scenario {
    PermissionWait,
    RunningTool,
    ModelStream,
}

#[derive(Debug, Serialize, Deserialize)]
struct RecoveryFixture {
    scenario: Scenario,
    checkpoint: Value,
    pending_request: HostRequestEnvelope,
}

fn capability(
    id: &str,
    route: HostCapabilityRoute,
    permission_domain: Option<&str>,
) -> HostCapabilityDescriptor {
    HostCapabilityDescriptor {
        id: id.to_string(),
        title: id.to_string(),
        route,
        availability: CapabilityAvailability::Available,
        exposure: HostCapabilityExposure::Visible,
        permission_domain: permission_domain.map(str::to_string),
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
    for (id, route, permission) in [
        ("model.stream", HostCapabilityRoute::Model, None),
        ("context.compact", HostCapabilityRoute::Compaction, None),
        ("permission.ask", HostCapabilityRoute::Permission, None),
        ("host.cancel", HostCapabilityRoute::Tool, None),
        (
            "process_tool",
            HostCapabilityRoute::Tool,
            Some("workspace.write"),
        ),
    ] {
        capabilities.insert(id.to_string(), capability(id, route, permission));
    }
    AgentCoreConfig {
        turn_identity: nola_core::agent::AgentTurnIdentity {
            model_capability: nola_core::agent::ModelCapabilityProfile {
                profile_id: "pending-effect-fixture".into(),
                revision: 1,
                native_tool_calls: true,
                stream_text: true,
                ..Default::default()
            },
            ..Default::default()
        },
        session_id: Some(SessionId::from_string(
            "process-recovery-session".to_string(),
        )),
        model: Some("test-model".to_string()),
        tools: vec![json!({
            "type": "function",
            "function": {
                "name": "process_tool",
                "description": "exercise a recoverable Host effect",
                "parameters": {"type": "object"}
            }
        })],
        host_catalog: Some(HostCapabilityCatalog {
            host_id: HostId("process-host".to_string()),
            revision: 11,
            availability_epoch: 5,
            lease: Some(HostLease {
                lease_id: "process-lease".to_string(),
                issued_at_ms: 1,
                last_heartbeat_ms: 1,
                expires_at_ms: u64::MAX,
            }),
            capabilities,
        }),
        ..AgentCoreConfig::default()
    }
}

fn response_for(
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
        delivery: request.delivery.clone(),
        sequence,
        payload,
    }
}

fn find_request(
    requests: &[HostRequestEnvelope],
    predicate: impl Fn(&HostRequest) -> bool,
) -> HostRequestEnvelope {
    requests
        .iter()
        .find(|request| predicate(&request.payload))
        .cloned()
        .expect("expected Host request")
}

fn checkpoint_value(core: &AgentCore) -> Value {
    serde_json::to_value(core.checkpoint(CheckpointReason::Manual)).expect("encode checkpoint")
}

fn model_completion(tool_calls: Vec<Value>) -> HostResponse {
    HostResponse::ModelCompleted {
        items: Vec::new(),
        tool_calls,
        usage: json!({"input_tokens": 12, "output_tokens": 4, "total_tokens": 16}),
    }
}

fn final_model_completion() -> HostResponse {
    HostResponse::ModelCompleted {
        items: vec![serde_json::to_value(AgentItem::assistant_message(
            ItemId::from_string("process-assistant".to_string()),
            "The operation completed.",
        ))
        .expect("encode assistant item")],
        tool_calls: Vec::new(),
        usage: json!({"input_tokens": 12, "output_tokens": 4, "total_tokens": 16}),
    }
}

fn complete_model(
    core: &mut AgentCore,
    request: &HostRequestEnvelope,
) -> nola_core::agent::AgentTransition {
    let text_part_id = match &request.payload {
        HostRequest::ModelStream { text_part_id, .. } => text_part_id.clone(),
        _ => panic!("expected model stream request"),
    };
    submit(
        core,
        request,
        1,
        HostResponse::ModelDelta {
            channel: ModelDeltaChannel::Text,
            part_id: text_part_id,
            delta: "The operation completed.".to_string(),
        },
    );
    submit(core, request, 2, model_completion(Vec::new()))
}

fn submit(
    core: &mut AgentCore,
    request: &HostRequestEnvelope,
    sequence: u64,
    response: HostResponse,
) -> nola_core::agent::AgentTransition {
    core.submit(AgentSubmission::HostResponse {
        response: response_for(request, sequence, response),
    })
    .expect("correlated Host response must be accepted")
}

fn create_fixture(scenario: Scenario, path: &Path) {
    let mut core = AgentCore::try_new(config()).expect("create Core");
    let started = core
        .submit(AgentSubmission::UserInput {
            items: vec![AgentItem::user_message(
                ItemId::from_string("process-user".to_string()),
                "write the requested file",
            )],
            turn_context_items: Vec::new(),
            turn_context_platform: ContextPlatform::Unknown,
            turn_context_components: Vec::new(),
            submission_id: Some("process-submission".to_string()),
            submitted_at_epoch_ms: None,
            delivery: AgentInputDelivery::CurrentTurn,
        })
        .expect("start Turn");
    let model = find_request(&started.host_requests, |request| {
        matches!(request, HostRequest::ModelStream { .. })
    });
    let started_checkpoint = checkpoint_value(&core);

    let (checkpoint, pending_request) = match scenario {
        Scenario::ModelStream => (started_checkpoint, model),
        Scenario::PermissionWait | Scenario::RunningTool => {
            let waiting = submit(
                &mut core,
                &model,
                1,
                model_completion(vec![json!({
                    "call_id": "process-write-call",
                    "name": "process_tool",
                    "arguments": {"value": "hello"}
                })]),
            );
            let permission = find_request(&waiting.host_requests, |request| {
                matches!(request, HostRequest::RequestPermission { .. })
            });
            if scenario == Scenario::PermissionWait {
                (checkpoint_value(&core), permission)
            } else {
                let permission_request_id = match &permission.payload {
                    HostRequest::RequestPermission {
                        permission_request_id,
                        ..
                    } => permission_request_id.clone(),
                    _ => unreachable!(),
                };
                let running = submit(
                    &mut core,
                    &permission,
                    1,
                    HostResponse::PermissionResolved {
                        request_id: permission_request_id,
                        resolution: nola_core::agent::host::HostPermissionResolution::allow_once(),
                    },
                );
                let tool = find_request(&running.host_requests, |request| {
                    matches!(request, HostRequest::ExecuteTool { .. })
                });
                (checkpoint_value(&core), tool)
            }
        }
    };

    let fixture = RecoveryFixture {
        scenario,
        checkpoint,
        pending_request,
    };
    fs::write(
        path,
        serde_json::to_vec_pretty(&fixture).expect("encode fixture"),
    )
    .expect("write fixture");
}

fn terminal_response(request: &HostRequestEnvelope, scenario: Scenario) -> HostResponse {
    match scenario {
        Scenario::PermissionWait => {
            let request_id = match &request.payload {
                HostRequest::RequestPermission {
                    permission_request_id,
                    ..
                } => permission_request_id.clone(),
                _ => panic!("expected permission request"),
            };
            HostResponse::PermissionResolved {
                request_id,
                resolution: nola_core::agent::host::HostPermissionResolution::allow_once(),
            }
        }
        Scenario::RunningTool => HostResponse::ToolCompleted {
            call_id: "process-write-call".to_string(),
            output: json!({"ok": true}),
            outcome: nola_core::agent::host::HostToolOutcome::Succeeded,
            activation: None,
            full_output_ref: None,
            artifacts: Vec::new(),
        },
        Scenario::ModelStream => final_model_completion(),
    }
}

fn finish_turn(
    core: &mut AgentCore,
    scenario: Scenario,
    transition: nola_core::agent::AgentTransition,
) -> nola_core::agent::AgentTransition {
    let after_tool = if scenario == Scenario::PermissionWait {
        let tool = find_request(&transition.host_requests, |request| {
            matches!(request, HostRequest::ExecuteTool { .. })
        });
        submit(
            core,
            &tool,
            1,
            HostResponse::ToolCompleted {
                call_id: "process-write-call".to_string(),
                output: json!({"ok": true}),
                outcome: nola_core::agent::host::HostToolOutcome::Succeeded,
                activation: None,
                full_output_ref: None,
                artifacts: Vec::new(),
            },
        )
    } else {
        transition
    };
    if scenario == Scenario::ModelStream {
        return after_tool;
    }
    let model = find_request(&after_tool.host_requests, |request| {
        matches!(request, HostRequest::ModelStream { .. })
    });
    let mut completed = complete_model(core, &model);
    // A tool-backed response crosses Core's explicit final-synthesis boundary:
    // the first post-tool prose remains provisional and the following
    // tool-free model round owns the terminal answer. Continue the recovered
    // process through that boundary instead of mistaking the draft Part for a
    // completed Turn.
    for _ in 0..3 {
        if completed.event_envelopes.iter().any(|event| {
            matches!(
                event.event,
                AgentEvent::TurnCompleted { .. }
                    | AgentEvent::TurnFailed { .. }
                    | AgentEvent::TurnCancelled { .. }
            )
        }) {
            return completed;
        }
        let Some(model) = completed
            .host_requests
            .iter()
            .find(|request| matches!(request.payload, HostRequest::ModelStream { .. }))
            .cloned()
        else {
            return completed;
        };
        completed = complete_model(core, &model);
    }
    completed
}

fn restore_and_verify(path: &Path) {
    let fixture: RecoveryFixture =
        serde_json::from_slice(&fs::read(path).expect("read fixture")).expect("decode fixture");
    let checkpoint: CoreCheckpoint<AgentThreadState> =
        serde_json::from_value(fixture.checkpoint).expect("decode checkpoint");
    let original_turn = fixture
        .pending_request
        .turn_id
        .clone()
        .expect("pending effect belongs to a Turn");
    let original_effect = fixture.pending_request.effect_id.clone();
    let mut core = AgentCore::restore(config(), checkpoint).expect("restore Core");
    let pending = core.pending_host_requests().expect("load pending requests");
    assert!(
        pending.is_empty(),
        "recoverable effects must wait for an explicit recovery decision"
    );
    assert_eq!(core.snapshot().pending_recovery_effect_count, 1);
    let recovery = core
        .submit(AgentSubmission::ResumeHostEffect {
            effect_id: original_effect.0.clone(),
        })
        .expect("resume recoverable Host effect");
    let resumed = recovery
        .host_requests
        .into_iter()
        .find(|request| request.effect_id == original_effect)
        .expect("effect must be recovered exactly once");
    assert_eq!(resumed.effect_id, original_effect);
    assert_eq!(resumed.turn_id.as_deref(), Some(original_turn.as_str()));
    assert_eq!(resumed.resume_after_sequence, None);

    let completed = if fixture.scenario == Scenario::ModelStream {
        let text_part_id = match &resumed.payload {
            HostRequest::ModelStream { text_part_id, .. } => text_part_id.clone(),
            _ => panic!("expected recovered model stream request"),
        };
        let delta = HostResponse::ModelDelta {
            channel: ModelDeltaChannel::Text,
            part_id: text_part_id,
            delta: "The operation completed.".to_string(),
        };
        let mut wrong_identity = response_for(&resumed, 1, delta.clone());
        wrong_identity.request_id.push_str("-stale");
        assert!(
            core.submit(AgentSubmission::HostResponse {
                response: wrong_identity
            })
            .is_err(),
            "a stale response binding must not consume the effect"
        );

        submit(&mut core, &resumed, 1, delta);
        let payload = model_completion(Vec::new());
        let terminal = submit(&mut core, &resumed, 2, payload.clone());
        assert!(
            core.submit(AgentSubmission::HostResponse {
                response: response_for(&resumed, 2, payload.clone()),
            })
            .is_err(),
            "duplicate terminal response must be rejected"
        );
        assert!(
            core.submit(AgentSubmission::HostResponse {
                response: response_for(&resumed, 3, payload),
            })
            .is_err(),
            "late terminal response must be rejected"
        );
        finish_turn(&mut core, fixture.scenario, terminal)
    } else {
        let sequence = 1;
        let payload = terminal_response(&resumed, fixture.scenario);

        let mut wrong_identity = response_for(&resumed, sequence, payload.clone());
        wrong_identity.request_id.push_str("-stale");
        assert!(
            core.submit(AgentSubmission::HostResponse {
                response: wrong_identity
            })
            .is_err(),
            "a stale response binding must not consume the effect"
        );

        let terminal = core
            .submit(AgentSubmission::HostResponse {
                response: response_for(&resumed, sequence, payload.clone()),
            })
            .expect("current correlated terminal response");
        assert!(
            core.submit(AgentSubmission::HostResponse {
                response: response_for(&resumed, sequence, payload.clone()),
            })
            .is_err(),
            "duplicate terminal response must be rejected"
        );
        assert!(
            core.submit(AgentSubmission::HostResponse {
                response: response_for(&resumed, sequence + 1, payload),
            })
            .is_err(),
            "late terminal response must be rejected"
        );

        finish_turn(&mut core, fixture.scenario, terminal)
    };
    let terminal_events = completed
        .event_envelopes
        .iter()
        .filter(|event| {
            matches!(
                event.event,
                AgentEvent::TurnCompleted { .. }
                    | AgentEvent::TurnFailed { .. }
                    | AgentEvent::TurnCancelled { .. }
            )
        })
        .count();
    assert_eq!(
        terminal_events, 1,
        "Turn must have one terminal event; events={:?}",
        completed.event_envelopes
    );
    let final_checkpoint: CoreCheckpoint<AgentThreadState> =
        serde_json::from_value(checkpoint_value(&core)).expect("decode final checkpoint");
    assert_eq!(final_checkpoint.state.host.pending_effects.len(), 0);
    assert_eq!(
        final_checkpoint
            .state
            .execution
            .active_turn
            .as_ref()
            .expect("terminal Turn retained in checkpoint")
            .turn_id
            .as_str(),
        original_turn
    );
}

fn wait_with_timeout(child: &mut Child, timeout: Duration) -> ExitStatus {
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("poll child") {
            return status;
        }
        if started.elapsed() >= timeout {
            child.kill().expect("kill timed-out child");
            panic!("child process exceeded {timeout:?}");
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn run_child(mode: &str, scenario: Scenario, fixture: &Path) {
    let mut child = Command::new(std::env::current_exe().expect("current test executable"))
        .args(["--exact", "pending_effect_recovery_child", "--nocapture"])
        .env(CHILD_MODE, mode)
        .env(
            CHILD_SCENARIO,
            serde_json::to_string(&scenario).expect("encode scenario"),
        )
        .env(CHILD_FIXTURE, fixture)
        .spawn()
        .expect("spawn recovery child");
    let status = wait_with_timeout(&mut child, PROCESS_TIMEOUT);
    assert!(status.success(), "{mode} child failed with {status}");
}

#[test]
fn pending_effect_recovery_child() {
    let Ok(mode) = std::env::var(CHILD_MODE) else {
        return;
    };
    let scenario: Scenario =
        serde_json::from_str(&std::env::var(CHILD_SCENARIO).expect("child scenario"))
            .expect("decode child scenario");
    let fixture = PathBuf::from(std::env::var_os(CHILD_FIXTURE).expect("child fixture"));
    match mode.as_str() {
        "create" => create_fixture(scenario, &fixture),
        "restore" => restore_and_verify(&fixture),
        other => panic!("unknown child mode {other}"),
    }
}

#[test]
fn pending_effects_are_exactly_once_across_process_recovery() {
    if std::env::var_os(CHILD_MODE).is_some() {
        return;
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "nola-pending-effect-recovery-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(&root).expect("create fixture root");

    for scenario in [
        Scenario::PermissionWait,
        Scenario::RunningTool,
        Scenario::ModelStream,
    ] {
        let fixture = root.join(format!("{scenario:?}.json"));
        run_child("create", scenario, &fixture);
        run_child("restore", scenario, &fixture);
    }

    fs::remove_dir_all(root).expect("remove fixture root");
}
