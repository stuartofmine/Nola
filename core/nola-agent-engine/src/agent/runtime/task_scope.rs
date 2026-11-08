// SPDX-License-Identifier: Apache-2.0
//! Optional model-authored task decomposition; never grants execution authority.
use serde_json::{json, Value};

pub(super) const TOOL: &str = "task_scope_update";

pub(super) fn tool_schema() -> Value {
    let reference = json!({"type":"object","additionalProperties":false,"properties":{
        "kind":{"type":"string","enum":["existing_id","local_key"]},
        "id":{"type":"string","description":"Current rN reference for existing_id."},
        "key":{"type":"string","description":"New requirement key for local_key."}},"required":["kind"]});
    let sources = json!({"type":"array","items":{"type":"string"},"description":"Current sN source references, not assistant assumptions."});
    let claim = json!({"type":"object","additionalProperties":false,"required":["resource_uri","operation","source_refs"],"properties":{
        "resource_uri":{"type":"string"},"operation":{"type":"string","description":"Exact operation identifier from the relevant execution receipt, not a natural-language description."},"version":{"type":["string","null"],"description":"Exact observed version, or omit when not yet known; never invent a version."},"predicate":{"type":["string","null"],"description":"Exact receipt postcondition identifier, not prose. Omit when only action coverage is claimed."},"source_refs":sources}});
    let exclusion = json!({"type":"object","additionalProperties":false,"required":["resource_uri","reason","source_refs","provenance"],"properties":{
        "resource_uri":{"type":"string"},"reason":{"type":"string"},"source_refs":sources,
        "provenance":{"type":"string","enum":["user_input_interpretation","assistant_suggestion"],"description":"User input interpretation must be supported by the cited user request. Assistant suggestions remain unresolved scope and cannot support assessed completed results. Recording a suggestion does not make it a user exclusion."}}});
    let scope = json!({"type":"object","additionalProperties":false,"required":["kind"],"properties":{
        "kind":{"type":"string","enum":["undeclared","declared"]},
        "required":{"type":"array","items":claim},"exclusions":{"type":"array","items":exclusion}}});
    let requirement = json!({"type":"object","additionalProperties":false,"required":["key","description","source_refs","dependencies","resource_scope"],"properties":{
        "key":{"type":"string"},"description":{"type":"string"},"source_refs":sources,
        "dependencies":{"type":"array","items":reference},"resource_scope":scope}});
    let retain = json!({"type":"object","additionalProperties":false,"required":["kind","requirement_id"],"properties":{
            "kind":{"type":"string","enum":["retain"]},"requirement_id":{"type":"string"}}});
    let replace = json!({"type":"object","additionalProperties":false,"required":["kind","requirement_id","successors","reason","user_input_refs"],"properties":{
            "kind":{"type":"string","enum":["replace"]},"requirement_id":{"type":"string"},
            "successors":{"type":"array","items":reference},"reason":{"type":"string"},"user_input_refs":sources}});
    let withdraw = json!({"type":"object","additionalProperties":false,"required":["kind","requirement_id","reason","user_input_refs"],"properties":{
            "kind":{"type":"string","enum":["withdraw"]},"requirement_id":{"type":"string"},
            "reason":{"type":"string"},"user_input_refs":sources}});
    let disposition = json!({"description":"Use replace to refine an existing requirement with new requirements. retain keeps it unchanged and accepts no reason, successors or user_input_refs.","oneOf":[retain,replace,withdraw]});
    let parameters = json!({"type":"object","additionalProperties":false,"required":["requirements","dispositions"],"properties":{
        "requirements":{"type":"array","items":requirement},"dispositions":{"type":"array","items":disposition}}});
    json!({"type":"function","function":{"name":TOOL,
        "description":"Optionally refine the current task into explicit deliverables and resource coverage before work. Does not end the turn or require a completion assessment. Call alone. Preserve every old requirement with retain or explicit replacement/withdrawal. Required resources are model interpretations, not authorization. Record assistant suggestions separately from user-input interpretations; neither proves user acceptance. Same user frontier cannot discard previously required resources. Receipts cover only their concrete resources, actions and versions, not all user intent.",
        "parameters":parameters}})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn advertised_dispositions_reject_the_real_invalid_retain_shape() {
        let schema = tool_schema();
        let schema = &schema["function"]["parameters"];
        let mut request = json!({"requirements":[],"dispositions":[{"kind":"retain","requirement_id":"r1"}]});
        assert!(crate::agent::host::validate_tool_input_schema(schema,&request).is_ok());
        request["dispositions"][0]["reason"] = json!("refine resources");
        request["dispositions"][0]["successors"] = json!([{"kind":"local_key","key":"new"}]);
        request["dispositions"][0]["user_input_refs"] = json!(["s1"]);
        assert!(crate::agent::host::validate_tool_input_schema(schema,&request).is_err());
        request["dispositions"][0]["kind"] = json!("replace");
        assert!(crate::agent::host::validate_tool_input_schema(schema,&request).is_ok());
    }
}
