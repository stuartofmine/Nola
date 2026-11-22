// SPDX-License-Identifier: Apache-2.0
//! Thin native Host transport ABI; exact Core delivery admission precedes I/O.
use super::{encode,utf8_input,NolaCoreApplication};
use serde_json::{json,Value};
use std::os::raw::c_char;
#[no_mangle]
pub extern "C" fn nola_core_mobile_mcp_host(application:*mut NolaCoreApplication,ptr:*const u8,len:usize)->*mut c_char {
    let result=(||->Result<Value,String>{
        if len>nola_protocol::APPLICATION_SERVICE_MAX_FRAME_BYTES {return Err("Invalid MCP Host frame".into());}
        let raw=utf8_input(ptr,len).ok_or("Invalid MCP Host input")?;
        let mut input = parse_request(&raw)?;
        if input["type"]=="cancel" {return nola_mcp_host::mobile::command(input);}
        if application.is_null(){return Err("MCP Application is unavailable".into());}
        let handle=unsafe{&*application};
        {
            let mut runtime=handle.application.lock()?;
            let servers=runtime.execute(serde_json::from_value(json!({"type":"mcp","command":{"type":"list"}})).map_err(|e|e.to_string())?)?;
            let allowed=input.get("allowed_server_ids").and_then(Value::as_array);
            input["servers"]=Value::Array(servers.as_array().ok_or("Invalid MCP store projection")?.iter().filter(|s|allowed.is_none_or(|ids|ids.contains(&s["id"]))).cloned().collect());
            if input["type"]=="execute" {
                let inspected=runtime.execute(serde_json::from_value(json!({"type":"inspect_mobile_mcp_request","request":input["request"],"now_ms":chrono::Utc::now().timestamp_millis()})).map_err(|e|e.to_string())?)?;
                input["admitted_capability"]=inspected["capability"].clone();
            }
        }
        let mut prepare=input.clone();prepare["type"]=json!("prepare");
        let catalog=nola_mcp_host::mobile::command(prepare)?;
        if input["type"]=="prepare" {Ok(catalog)} else {nola_mcp_host::mobile::command(input)}
    })();
    match result {Ok(value)=>encode(json!({"ok":true,"result":value})),Err(message)=>encode(json!({"ok":false,"error":{"code":"mcp_host_rejected","message":message}}))}
}

/// Retain before leaving the native store lock; release with Application free.
#[no_mangle]
pub extern "C" fn nola_core_mobile_mcp_retain(application:*mut NolaCoreApplication)->*mut NolaCoreApplication {
    if application.is_null(){return std::ptr::null_mut();}
    let application=unsafe{&*application}.application.clone();
    Box::into_raw(Box::new(NolaCoreApplication{application}))
}

fn parse_request(raw: &str) -> Result<Value, String> {
    let object: serde_json::Map<String, Value> = serde_json::from_str(raw).map_err(|e| format!("Invalid MCP Host request object: {e}"))?;
    if let Some(ids) = object.get("allowed_server_ids") {
        if !ids.as_array().is_some_and(|ids| ids.iter().all(Value::is_string)) {
            return Err("allowed_server_ids must be an array of server IDs".into());
        }
    }
    Ok(Value::Object(object))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_the_mobile_mcp_request_and_preserves_explicit_empty_server_scope() {
        for raw in ["null", "[]", "42", r#"{"allowed_server_ids":"one"}"#, r#"{"allowed_server_ids":[1]}"#] {
            assert!(parse_request(raw).is_err());
        }
        assert_eq!(parse_request(r#"{"allowed_server_ids":[]}"#).unwrap()["allowed_server_ids"], json!([]));
        assert!(parse_request(r#"{"type":"catalog"}"#).unwrap().get("allowed_server_ids").is_none());
    }
}
