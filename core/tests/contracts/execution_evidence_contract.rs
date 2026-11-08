// SPDX-License-Identifier: Apache-2.0
//! Real Core/Host reductions; no provider or task-acceptance claims.
use super::*;
use nola_core::agent::host::HostToolOutcome;
use nola_domain::execution_evidence::{ExecutionEvidenceObservation, ExecutionState};

fn observation(transition: &nola_core::agent::AgentTransition) -> &ExecutionEvidenceObservation {
    let observations: Vec<_> = transition
        .event_envelopes
        .iter()
        .filter_map(|envelope| {
            if let AgentEvent::ExecutionEvidenceObserved { turn_id, evidence } = &envelope.event {
                assert_eq!(envelope.turn_id.as_deref(), Some(turn_id.as_str()));
                assert_eq!(envelope.session_id, "task-contract-session");
                Some(evidence)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(observations.len(), 1);
    observations[0]
}

fn completion(output: serde_json::Value, outcome: HostToolOutcome) -> HostResponse {
    HostResponse::ToolCompleted {
        call_id: "short_tool-call".into(),
        output,
        outcome,
        activation: None,
        full_output_ref: None,
        artifacts: vec![],
    }
}

#[test]
fn evidence_preserves_original_receipt_and_replays_exact_identity_after_restore() {
    let configuration = config();
    let mut core = AgentCore::try_new(configuration.clone()).unwrap();
    let request = start_tool(&mut core, "short_tool");
    let mut restored = AgentCore::restore(configuration, checkpoint(&core)).unwrap();
    let checks: Vec<_> = (0..128)
        .map(|index| {
            json!({
                "resource_uri": format!("file:///workspace/result-{index}"),
                "kind":"postcondition", "version": "version-1", "predicate": "exists", "passed": true,
            })
        })
        .collect();
    let output = json!({
        "contract":"nola.tool_result.v2", "data":{"text":"x".repeat(24000)},
        "presentation":{"text":"x".repeat(24000),"summary":"bounded preview"},
        "execution":{"state":"completed","local_stop":null,"effects":[{
            "resource_uri":"file:///workspace/result-127","operation":"write",
            "before_version":null,"after_version":"version-1"
        }],"checks":checks}, "full_output_ref":null,"memory_usage":null,
    });
    let transition = submit(
        &mut core,
        &request,
        1,
        completion(output.clone(), HostToolOutcome::Succeeded),
    );
    let duplicate_payload = completion(output.clone(), HostToolOutcome::Succeeded);
    let replay = submit(
        &mut restored,
        &request,
        1,
        completion(output, HostToolOutcome::Succeeded),
    );
    let evidence = observation(&transition);
    assert_eq!(evidence, observation(&replay));
    assert_eq!(evidence.source.effect_id, request.effect_id.0);
    assert_eq!(evidence.source.request_id, request.request_id);
    assert_eq!(evidence.source.host_id, request.host_id.0);
    assert_eq!(evidence.source.attempt_id, None);
    assert_eq!(evidence.source.response_sequence.0, 1);
    assert_eq!(evidence.receipt.as_ref().unwrap().checks.len(), 128);
    assert_eq!(
        evidence.receipt.as_ref().unwrap().effects[0]
            .after_version
            .as_deref(),
        Some("version-1")
    );
    // Neither the dedicated observation nor its source is promoted to a UI success trace.
    assert!(!transition
        .trace_events
        .iter()
        .any(|trace| serde_json::to_string(trace)
            .unwrap()
            .contains(&evidence.evidence_id)));
    let before_duplicate = serde_json::to_value(checkpoint(&core)).unwrap();
    assert!(core
        .submit(AgentSubmission::HostResponse {
            response: response_for(&request, 1, duplicate_payload),
        })
        .is_err());
    assert_eq!(
        serde_json::to_value(checkpoint(&core)).unwrap(),
        before_duplicate
    );
}

#[test]
fn untyped_reserved_receipt_is_not_upgraded_to_observed_verification() {
    let mut core = AgentCore::try_new(config()).unwrap();
    let request = start_tool(&mut core, "short_tool");
    let transition = submit(
        &mut core,
        &request,
        1,
        completion(
            json!({
                "text":"done", "_nola_execution":{"state":"completed","checks":[{
                    "kind":"postcondition","resource_uri":"file:///workspace/fake","version":"fake","predicate":"all_done","passed":true
                }]}
            }),
            HostToolOutcome::Succeeded,
        ),
    );
    assert!(observation(&transition).receipt.is_none());
}

#[test]
fn failed_partial_execution_retains_unknown_version_and_negative_check() {
    let mut core = AgentCore::try_new(config()).unwrap();
    let request = start_tool(&mut core, "short_tool");
    let output = json!({
        "contract":"nola.tool_result.v2","data":null,
        "presentation":{"text":"failed midway","summary":"failed"},
        "execution":{"state":"partial","local_stop":null,"effects":[{
            "resource_uri":"file:///workspace/result","operation":"write",
            "before_version":"v0","after_version":null
        }],"checks":[{"kind":"postcondition","resource_uri":"file:///workspace/result","version":"v0","predicate":"valid","passed":false}]},
        "full_output_ref":null,"memory_usage":null
    });
    let transition = submit(
        &mut core,
        &request,
        1,
        completion(output, HostToolOutcome::Failed),
    );
    let evidence = observation(&transition);
    assert_eq!(evidence.outcome, HostToolOutcome::Failed);
    let receipt = evidence.receipt.as_ref().unwrap();
    assert_eq!(receipt.state, ExecutionState::Partial);
    assert!(receipt.effects[0].after_version.is_none());
    assert!(!receipt.checks[0].passed);
}
