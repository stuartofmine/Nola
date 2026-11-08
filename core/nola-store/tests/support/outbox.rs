// SPDX-License-Identifier: Apache-2.0
//! Real protocol envelopes for low-level Store-only ledger fixtures.
#![allow(dead_code)]
use nola_store_port::run_effect::OutboxEffectWrite;
pub fn declare(effect: &str, request: &str, payload: serde_json::Value) -> OutboxEffectWrite {
    let raw = serde_json::json!({"attempt_started":false,"request_id":request,"effect_id":effect,
        "host_id":"fixture-host","lease_id":null,"session_id":"fixture-session","turn_id":null,
        "catalog_revision":1,"availability_epoch":1,"created_at_ms":1,"deadline_ms":null,
        "payload":payload});
    // Fixture construction, not a production compatibility parser.
    let envelope: nola_domain::host_effect::HostRequestEnvelope<
        nola_domain::host_effect::HostRequest,
    > = serde_json::from_value(raw).unwrap();
    OutboxEffectWrite::Declare {
        payload_json: serde_json::to_string(&envelope).unwrap(),
    }
}
pub fn tool(effect: &str, request: &str, input: serde_json::Value) -> OutboxEffectWrite {
    declare(
        effect,
        request,
        serde_json::json!({"type":"execute_tool","permission_profile":"cautious",
            "capability_id":"fixture.tool","call_id":effect,"input":input}),
    )
}
pub fn model(effect: &str, request: &str, text: &str, reasoning: &str) -> OutboxEffectWrite {
    declare(
        effect,
        request,
        serde_json::json!({"type":"model_stream","model_id":"test-model",
        "text_part_id":text,"reasoning_part_id":reasoning,"response_mode":"conversational",
        "messages":[],"tools":[],"settings":{}}),
    )
}
pub fn delegated(effect: &str, request: &str, attempt: &str) -> OutboxEffectWrite {
    declare(
        effect,
        request,
        serde_json::json!({"type":"delegated_agent","request":{
        "contract_version":1,"backend_id":"codex","text_part_id":format!("{attempt}:text"),
        "reasoning_part_id":format!("{attempt}:reasoning"),"context":{"mode":"bootstrap",
        "target_anchor_item_id":"user","identity_digest":"fixture-context","messages":[{"role":"user","content":"fixture"}]}}}),
    )
}
