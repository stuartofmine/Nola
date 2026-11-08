// SPDX-License-Identifier: Apache-2.0
//! Pure SIWC wire mapping. Core remains the only tool admission/execution owner.
use serde_json::{json,Value};
use std::collections::BTreeSet;

pub const NAMESPACE:&str="nola";

#[derive(Clone,Debug)]
pub struct FrozenSiwcTools {
    functions:Vec<Value>,
    names:BTreeSet<String>,
    calls_allowed:bool,
}

fn name(value:&str)->Result<(),String> {
    if value.is_empty() || value.len()>64 || !value.bytes().all(|c|c.is_ascii_alphanumeric()||matches!(c,b'_'|b'-')) {
        return Err("SIWC invalid function name".into());
    }
    Ok(())
}

impl FrozenSiwcTools {
    pub fn from_chat_tools(tools:&[Value])->Result<Self,String> {
        let mut functions=Vec::new();let mut names=BTreeSet::new();
        for tool in tools {
            if tool["type"]!="function" || tool.as_object().is_none_or(|v|v.keys().any(|k|!matches!(k.as_str(),"type"|"function"))) {
                return Err("SIWC supports only admitted function tools".into());
            }
            let mut function=tool["function"].as_object().cloned().ok_or("SIWC function schema missing")?;
            if function.keys().any(|k|!matches!(k.as_str(),"name"|"description"|"parameters"|"strict")) {
                return Err("SIWC unsupported function schema field".into());
            }
            let id=function.get("name").and_then(Value::as_str).ok_or("SIWC function name missing")?;name(id)?;
            if !names.insert(id.to_owned()) {return Err("SIWC duplicate function name".into());}
            if !function.get("parameters").is_some_and(Value::is_object)
                || function.get("description").is_some_and(|v|!v.is_string())
                || function.get("strict").is_some_and(|v|!v.is_boolean()) {
                return Err("SIWC invalid function schema".into());
            }
            function.entry("strict").or_insert(json!(false));
            function.insert("type".into(),json!("function"));functions.push(Value::Object(function));
        }
        Ok(Self {functions,names,calls_allowed:true})
    }

    pub fn schema(&self)->Vec<Value> {
        if self.functions.is_empty() {vec![]} else {vec![json!({"type":"namespace","name":NAMESPACE,"description":"Nola local tools. Each call is validated and authorized by Nola Core.","tools":self.functions})]}
    }

    /// Reconstruct the exact admitted mapping from the frozen physical request.
    pub fn from_body(body:&Value)->Result<Self,String> {
        let Some(tools)=body.get("tools") else {return Self::from_chat_tools(&[]);};
        let tools=tools.as_array().ok_or("SIWC tools must be an array")?;
        if tools.is_empty() {return Self::from_chat_tools(&[]);}
        if tools.len()!=1 {return Err("SIWC unexpected namespace count".into());}
        let namespace=&tools[0];
        if namespace["type"]!="namespace" || namespace["name"]!=NAMESPACE
            || namespace.as_object().is_none_or(|v|v.keys().any(|k|!matches!(k.as_str(),"type"|"name"|"description"|"tools"))) {
            return Err("SIWC unexpected tool namespace".into());
        }
        let nested=namespace["tools"].as_array().ok_or("SIWC namespace tools missing")?;
        let mut chat=Vec::new();
        for function in nested {
            if function["type"]!="function" {return Err("SIWC unsupported namespace tool".into());}
            let mut function=function.as_object().cloned().ok_or("SIWC function schema missing")?;
            function.remove("type");chat.push(json!({"type":"function","function":function}));
        }
        let mut frozen = Self::from_chat_tools(&chat)?;
        frozen.calls_allowed = body["tool_choice"] != "none";
        Ok(frozen)
    }

    /// Call before emitting added/done deltas, not only after terminal completion.
    pub fn resolve_call(&self,item:&Value)->Result<String,String> {
        if !self.calls_allowed {return Err("SIWC request forbids tool calls".into());}
        let canonical=validate_function_call(item)?;
        if !self.names.contains(canonical) {return Err("SIWC returned an unadmitted function".into());}
        Ok(canonical.into())
    }
}

fn validate_function_call(item:&Value)->Result<&str,String> {
    if item["type"]!="function_call" || item["namespace"]!=NAMESPACE {return Err("SIWC function namespace mismatch".into());}
    let canonical=item["name"].as_str().ok_or("SIWC function name missing")?;name(canonical)?;
    if item["call_id"].as_str().is_none_or(str::is_empty) {return Err("SIWC function call ID missing".into());}
    if item.get("arguments").is_some_and(|v|!v.is_string()) {return Err("SIWC function arguments must remain a string".into());}
    Ok(canonical)
}

/// Only for trusted canonical Core history, not raw provider output.
pub fn encode_history_call(call_id:&str,canonical_name:&str,arguments:&str)->Result<Value,String> {
    let item=json!({"type":"function_call","namespace":NAMESPACE,"name":canonical_name,"call_id":call_id,"arguments":arguments});
    validate_function_call(&item)?;Ok(item)
}

/// Preserve provider history byte values and fields; historical catalog presence
/// does not authorize a new call in the current frozen tool map.
pub fn replay_history(item:&Value)->Result<Value,String> {
    match item["type"].as_str() {
        Some("function_call")=>{validate_function_call(item)?;},
        Some("function_call_output")=>{
            if item["call_id"].as_str().is_none_or(str::is_empty) {return Err("SIWC function output call ID missing".into());}
            if let Some(namespace)=item.get("namespace") {if namespace!=NAMESPACE {return Err("SIWC output namespace mismatch".into());}}
            if let Some(value)=item.get("name") {name(value.as_str().ok_or("SIWC output name invalid")?)?;}
        },
        Some("custom_tool_call"|"custom_tool_call_output"|"tool_search_call"|"tool_search_output")=>return Err("SIWC unsupported tool history".into()),
        _=>{},
    }
    Ok(item.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tools()->Vec<Value> {vec![json!({"type":"function","function":{"name":"read_file","description":"读取文件 — file content","parameters":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"],"additionalProperties":false}}})]}
    #[test]
    fn model_siwc_namespace_schema_preserves_core_contract_and_freezes_mapping() {
        let original=tools();let frozen=FrozenSiwcTools::from_chat_tools(&original).unwrap();let schema=frozen.schema();
        assert_eq!(schema[0]["tools"][0]["parameters"],original[0]["function"]["parameters"]);
        assert_eq!(schema[0]["tools"][0]["strict"],false);
        assert_eq!(schema[0]["tools"][0]["description"],original[0]["function"]["description"]);
        let restored=FrozenSiwcTools::from_body(&json!({"tools":schema})).unwrap();
        let call=encode_history_call("c1","read_file","{}").unwrap();assert_eq!(restored.resolve_call(&call).unwrap(),"read_file");
        assert_eq!(restored.schema(),frozen.schema());
    }
    #[test]
    fn model_siwc_namespace_none_preserves_schema_without_authorizing_calls() {
        let original=FrozenSiwcTools::from_chat_tools(&tools()).unwrap();
        let restored=FrozenSiwcTools::from_body(&json!({"tools":original.schema(),"tool_choice":"none"})).unwrap();
        assert_eq!(restored.schema(),original.schema());
        assert!(restored.resolve_call(&encode_history_call("c1","read_file","{}").unwrap()).is_err());
    }
    #[test]
    fn model_siwc_namespace_unknown_names_and_namespace_are_never_guessed() {
        let frozen=FrozenSiwcTools::from_chat_tools(&tools()).unwrap();
        for (namespace,id) in [(json!("other"),"read_file"),(Value::Null,"read_file"),(json!(NAMESPACE),"other.read_file"),(json!(NAMESPACE),"unknown")] {
            let call=json!({"type":"function_call","namespace":namespace,"name":id,"call_id":"c1","arguments":""});assert!(frozen.resolve_call(&call).is_err());
        }
        let mut added=encode_history_call("c1","read_file","").unwrap();assert!(frozen.resolve_call(&added).is_ok());
        added["namespace"]=json!("changed");assert!(frozen.resolve_call(&added).is_err());
    }
    #[test]
    fn model_siwc_namespace_history_keeps_namespace_and_retracted_tools_without_authorizing() {
        let history=json!({"type":"function_call","namespace":"nola","name":"old_tool","arguments":"{\"text\":\"搜索 current\"}","call_id":"c1","id":"provider-id","status":"completed"});
        assert_eq!(replay_history(&history).unwrap(),history);
        assert!(FrozenSiwcTools::from_chat_tools(&tools()).unwrap().resolve_call(&history).is_err());
        let output=json!({"type":"function_call_output","namespace":"nola","name":"old_tool","call_id":"c1","output":"done"});assert_eq!(replay_history(&output).unwrap(),output);
        let mut missing=history;missing.as_object_mut().unwrap().remove("namespace");assert!(replay_history(&missing).is_err());
    }
    #[test]
    fn model_siwc_namespace_rejects_duplicate_hosted_deferred_and_custom_tools() {
        let mut duplicate=tools();duplicate.extend(tools());assert!(FrozenSiwcTools::from_chat_tools(&duplicate).is_err());
        for kind in ["tool_search","custom","mcp","programmatic_tool_calling"] {assert!(FrozenSiwcTools::from_chat_tools(&[json!({"type":kind})]).is_err());}
        let mut deferred=tools();deferred[0]["function"]["defer_loading"]=json!(true);assert!(FrozenSiwcTools::from_chat_tools(&deferred).is_err());
        assert!(FrozenSiwcTools::from_body(&json!({"tools":[{"type":"namespace","name":"foreign","tools":[]}]})).is_err());
    }
}
