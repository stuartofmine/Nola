// SPDX-License-Identifier: Apache-2.0

use nola_core::agent::{
    CallId, PendingPermissionRequest, PermissionEvaluation, PermissionEvaluationReason,
    PermissionPolicyDecision, PermissionProjection, PermissionRequestId, TurnId,
};
use serde_json::json;

fn request(id: &str, turn: &str) -> PendingPermissionRequest {
    PendingPermissionRequest {
        allow_conversation: true,
        permission_instructions: Vec::new(),
        request_id: PermissionRequestId::from_string(id.to_string()),
        turn_id: TurnId::from_string(turn.to_string()),
        call_id: CallId::from_string(format!("call-{id}")),
        capability_id: "write_file".to_string(),
        resource: json!({"path":"/tmp/report.md"}).to_string(),
        arguments: json!({"path":"/tmp/report.md","content":"report"}),
        invocation_context: None,
        permission_domain: "workspace.write".to_string(),
        rationale: "Write the requested report".to_string(),
        evaluation: PermissionEvaluation {
            decision: PermissionPolicyDecision::Ask,
            reason: PermissionEvaluationReason::NoMatchingRule,
            rule_index: None,
            rule: None,
        },
        continuation: Default::default(),
    }
}

#[test]
fn permission_projection_round_trips_and_removes_requests_by_turn() {
    let mut projection = PermissionProjection::default();
    projection.upsert(request("permission-1", "turn-1"));
    projection.upsert(request("permission-2", "turn-1"));
    projection.upsert(request("permission-3", "turn-2"));
    assert_eq!(projection.revision, 3);

    let encoded = serde_json::to_vec(&projection).expect("serialize permission projection");
    let mut restored: PermissionProjection =
        serde_json::from_slice(&encoded).expect("restore permission projection");
    assert_eq!(restored, projection);

    assert_eq!(
        restored.remove_turn(&TurnId::from_string("turn-1".to_string())),
        2
    );
    assert_eq!(restored.pending.len(), 1);
    assert_eq!(restored.pending[0].request_id.as_str(), "permission-3");
    assert_eq!(restored.revision, 4);
}

#[test]
fn permission_projection_upsert_is_idempotent() {
    let mut projection = PermissionProjection::default();
    let pending = request("permission-1", "turn-1");
    projection.upsert(pending.clone());
    projection.upsert(pending);

    assert_eq!(projection.pending.len(), 1);
    assert_eq!(projection.revision, 1);
}
