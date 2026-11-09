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
    let review = json!({"type":["object","null"],"additionalProperties":false,"required":["source_evidence"],"properties":{
        "source_evidence":{"type":"array","minItems":1,"items":{"type":"string"},"description":"Current eN observations containing the adopted review finding. These are evidence, not user instructions."},
        "counterexample_evidence":{"type":["string","null"],"description":"Current eN receipt for the original failing check; omit while constructing the reproducer. Completion requires the same check to pass after repair on the current artifact version."}}});
    let claim = json!({"type":"object","additionalProperties":false,"required":["resource_uri","operation","source_refs"],"properties":{
        "resource_uri":{"type":"string","format":"uri","description":"Exact absolute URI used by the Host receipt, e.g. file:///absolute/path. Filesystem paths such as /absolute/path are invalid; do not register a path alias."},"operation":{"type":"string","description":"Exact effect operation from the actual mutation receipt, e.g. write, create, delete or commit; NOT a tool name such as bash or write_file. For a planned write_file repair use write."},"version":{"type":["string","null"],"description":"Exact observed version, or omit when not yet known; never invent a version."},"predicate":{"type":["string","null"],"description":"Exact receipt check identifier, not prose. Adopted review findings may omit it while constructing the reproducer; completion requires a postcondition or a regression_process_exit:sha256: identifier from an actual regression test receipt. Omit when only action coverage is claimed."},"review":review,"source_refs":sources}});
    let exclusion = json!({"type":"object","additionalProperties":false,"required":["resource_uri","reason","source_refs","provenance"],"properties":{
        "resource_uri":{"type":"string"},"reason":{"type":"string"},"source_refs":sources,
        "provenance":{"type":"string","enum":["user_input_interpretation","assistant_suggestion"],"description":"User input interpretation must be supported by the cited user request. Assistant suggestions remain unresolved scope and cannot support assessed completed results. Recording a suggestion does not make it a user exclusion."}}});
    let scope = json!({"oneOf":[
        {"type":"object","additionalProperties":false,"required":["kind"],"properties":{"kind":{"type":"string","enum":["undeclared"]}}},
        {"type":"object","additionalProperties":false,"required":["kind","required","exclusions"],"properties":{
            "kind":{"type":"string","enum":["declared"]},"required":{"type":"array","items":claim},"exclusions":{"type":"array","items":exclusion}}}
    ]});
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
        "description":"Optionally refine the current task into explicit deliverables and resource coverage before work. Does not end the turn or require a completion assessment. Call alone. Preserve every old requirement with retain or explicit replacement/withdrawal. Required resources are model interpretations, not authorization. Record assistant suggestions separately from user-input interpretations; neither proves user acceptance. Same user frontier cannot discard previously required resources. Receipts cover only their concrete resources, actions and versions, not all user intent. When remediation is in scope, record every adopted review finding as a separate requirement/claim with review evidence and an exact check predicate; retain unresolved findings. Reproduce failure before repair, then rerun the unchanged check on the final resource. Ordinary exit-zero, child success claims and unrelated scale tests cannot close a review finding. A read-only review need not adopt remediation obligations.",
        "parameters":parameters}})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn advertised_declared_scope_requires_the_same_fields_as_runtime() {
        let schema = tool_schema(); let schema = &schema["function"]["parameters"];
        let mut request = json!({"requirements":[{"key":"review","description":"Adopt finding", "source_refs":["s1"],"dependencies":[],
            "resource_scope":{"kind":"declared","required":[{"resource_uri":"file:///final","operation":"write","source_refs":["s1"],
                "review":{"source_evidence":["e1"]}}]}}],"dispositions":[{"kind":"retain","requirement_id":"r1"}]});
        assert!(crate::agent::host::validate_tool_input_schema(schema,&request).is_err());
        request["requirements"][0]["resource_scope"]["exclusions"] = json!([]);
        assert!(crate::agent::host::validate_tool_input_schema(schema,&request).is_ok());
    }

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
