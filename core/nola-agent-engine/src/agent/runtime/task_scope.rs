// SPDX-License-Identifier: Apache-2.0
//! Optional model-authored task decomposition; never grants execution authority.
use serde_json::{json, Value};

pub(super) const TOOL: &str = "task_scope_update";

pub(super) fn tool_schema() -> Value {
    let reference = json!({"type":"object","additionalProperties":false,"properties":{
        "kind":{"type":"string","enum":["existing_id","local_key"]},
        "id":{"type":"string","description":"Current rN ref."},
        "key":{"type":"string","description":"New requirement key."}},"required":["kind"]});
    let sources = json!({"type":"array","items":{"type":"string"},"description":"Current sN refs."});
    let review = json!({"type":["object","null"],"additionalProperties":false,"required":["source_evidence"],"properties":{
        "source_evidence":{"type":"array","minItems":1,"items":{"type":"string"},"description":"Current eN review-finding evidence."},
        "counterexample_evidence":{"type":["string","null"],"description":"Current eN failing-check receipt; omit before reproducer."}}});
    let claim = json!({"type":"object","additionalProperties":false,"required":["resource_uri","operation","source_refs"],"properties":{
        "resource_uri":{"type":"string","format":"uri","description":"Exact file:///absolute/path URI from Host receipt."},
        "operation":{"type":"string","description":"Effect operation (write, create, delete, commit; not tool name)."},
        "version":{"type":["string","null"],"description":"Observed version, or omit if unknown."},
        "predicate":{"type":["string","null"],"description":"Exact check predicate or regression_process_exit:sha256:...; omit if action-only."},
        "review":review,"source_refs":sources}});
    let exclusion = json!({"type":"object","additionalProperties":false,"required":["resource_uri","reason","source_refs","provenance"],"properties":{
        "resource_uri":{"type":"string"},"reason":{"type":"string"},"source_refs":sources,
        "provenance":{"type":"string","enum":["user_input_interpretation","assistant_suggestion"],"description":"user_input_interpretation requires user backing; assistant_suggestion remains unresolved."}}});
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
    let disposition = json!({"description":"retain (no extra fields), replace, or withdraw each old rN.","oneOf":[retain,replace,withdraw]});
    let parameters = json!({"type":"object","additionalProperties":false,"required":["requirements","dispositions"],"properties":{
        "requirements":{"type":"array","items":requirement},"dispositions":{"type":"array","items":disposition}}});
    json!({"type":"function","function":{"name":TOOL,
        "description":"Optionally decompose the task into explicit deliverable resource claims before work. Call alone; does not end the turn or grant permission. Preserve every old rN via retain/replace/withdraw without dropping prior required resources on the same user frontier. For adopted remediation findings, record each finding with review evidence and exact check predicate, reproduce failure before repair, then rerun unchanged.",
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
