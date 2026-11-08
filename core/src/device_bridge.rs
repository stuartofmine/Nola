// SPDX-License-Identifier: Apache-2.0
//! Shared contract for a paired host requesting explicitly shared device tools.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

use crate::tool_catalog::{
    builtin_tool_metadata, canonical_device_tool_capability, CANONICAL_DEVICE_TOOL_IDS,
};

pub const DEVICE_BRIDGE_PROTOCOL_VERSION: u32 = 1;
pub const MAX_ADVERTISED_DEVICE_TOOLS: usize = 16;
pub const MAX_DEVICE_TOOL_SCHEMA_BYTES: usize = 64 * 1024;
pub const MAX_DEVICE_TOOL_RESULT_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeviceToolAdvertisement {
    pub schema: Value,
    pub capability: String,
    pub allow_result_egress: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeviceBridgeAdvertisement {
    pub protocol_version: u32,
    pub host_id: String,
    pub client_id: String,
    pub session_id: String,
    pub tools: Vec<DeviceToolAdvertisement>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeviceToolRequest {
    pub protocol_version: u32,
    pub nonce: String,
    pub request_id: String,
    pub host_id: String,
    pub client_id: String,
    pub session_id: String,
    pub tool_name: String,
    pub capability: String,
    pub arguments: Value,
    pub expires_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeviceToolResult {
    pub protocol_version: u32,
    pub nonce: String,
    pub request_id: String,
    pub host_id: String,
    pub client_id: String,
    pub session_id: String,
    pub tool_name: String,
    pub output: Value,
    pub error: bool,
}

pub fn validate_advertisement(
    advertisement: &DeviceBridgeAdvertisement,
    expected_host_id: &str,
    expected_client_id: &str,
    expected_session_id: &str,
) -> Result<(), String> {
    if advertisement.protocol_version != DEVICE_BRIDGE_PROTOCOL_VERSION {
        return Err("device bridge protocol version does not match".to_string());
    }
    validate_context(
        &advertisement.host_id,
        &advertisement.client_id,
        &advertisement.session_id,
        expected_host_id,
        expected_client_id,
        expected_session_id,
    )?;
    if advertisement.tools.len() > MAX_ADVERTISED_DEVICE_TOOLS {
        return Err("too many advertised device tools".to_string());
    }
    let mut names = HashSet::new();
    let mut encoded_bytes = 0usize;
    for tool in &advertisement.tools {
        if !tool.allow_result_egress {
            return Err(
                "a host-visible device tool must explicitly allow result egress".to_string(),
            );
        }
        let name = device_tool_name(&tool.schema)?;
        if !names.insert(name.to_string()) {
            return Err("duplicate advertised device tool".to_string());
        }
        let expected_capability = canonical_device_tool_capability(name)
            .ok_or_else(|| "advertised device tool is not canonical".to_string())?;
        if tool.capability.trim().to_ascii_lowercase() != expected_capability {
            return Err(
                "advertised device tool capability does not match Core catalog".to_string(),
            );
        }
        encoded_bytes = encoded_bytes.saturating_add(
            serde_json::to_vec(&tool.schema)
                .map(|value| value.len())
                .unwrap_or(usize::MAX),
        );
        if encoded_bytes > MAX_DEVICE_TOOL_SCHEMA_BYTES {
            return Err("advertised device tool schemas are too large".to_string());
        }
    }
    Ok(())
}

pub fn validate_device_tool_result(
    request: &DeviceToolRequest,
    result: &DeviceToolResult,
    now: u64,
) -> Result<(), String> {
    if request.protocol_version != DEVICE_BRIDGE_PROTOCOL_VERSION
        || result.protocol_version != DEVICE_BRIDGE_PROTOCOL_VERSION
    {
        return Err("device bridge protocol version does not match".to_string());
    }
    if request.expires_at < now {
        return Err("device tool request expired".to_string());
    }
    let expected_capability =
        canonical_device_tool_capability(&request.tool_name).ok_or_else(|| {
            "device tool request does not use a canonical Core catalog ID".to_string()
        })?;
    if request.capability.trim().to_ascii_lowercase() != expected_capability {
        return Err("device tool request capability does not match Core catalog".to_string());
    }
    if request.nonce != result.nonce
        || request.request_id != result.request_id
        || request.host_id != result.host_id
        || request.client_id != result.client_id
        || request.session_id != result.session_id
        || request.tool_name != result.tool_name
    {
        return Err("device tool result context does not match".to_string());
    }
    let output_bytes = serde_json::to_vec(&result.output)
        .map(|value| value.len())
        .unwrap_or(usize::MAX);
    if output_bytes > MAX_DEVICE_TOOL_RESULT_BYTES {
        return Err("device tool result is too large".to_string());
    }
    Ok(())
}

pub fn device_tool_name(schema: &Value) -> Result<&str, String> {
    let name = schema
        .pointer("/function/name")
        .and_then(Value::as_str)
        .ok_or_else(|| "device tool schema is missing function.name".to_string())?;
    if !CANONICAL_DEVICE_TOOL_IDS.contains(&name) || builtin_tool_metadata(name).is_none() {
        return Err("device tool name is not a canonical Core catalog ID".to_string());
    }
    if schema.pointer("/x_nola/target").and_then(Value::as_str) != Some("device") {
        return Err("device tool target metadata is invalid".to_string());
    }
    Ok(name)
}

fn validate_context(
    host_id: &str,
    client_id: &str,
    session_id: &str,
    expected_host_id: &str,
    expected_client_id: &str,
    expected_session_id: &str,
) -> Result<(), String> {
    if host_id != expected_host_id {
        return Err("device bridge host identity does not match".to_string());
    }
    if client_id != expected_client_id {
        return Err("device bridge client identity does not match".to_string());
    }
    if session_id != expected_session_id {
        return Err("device bridge session does not match".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema(name: &str) -> Value {
        json!({
            "type": "function",
            "function": {"name": name, "description": "test", "parameters": {"type": "object"}},
            "x_nola": {"target": "device"}
        })
    }

    #[test]
    fn advertisement_requires_explicit_egress_and_device_target() {
        let mut advertisement = DeviceBridgeAdvertisement {
            protocol_version: 1,
            host_id: "host".into(),
            client_id: "client".into(),
            session_id: "session".into(),
            tools: vec![DeviceToolAdvertisement {
                schema: schema("device_clipboard_read"),
                capability: "clipboard".into(),
                allow_result_egress: true,
            }],
        };
        assert!(validate_advertisement(&advertisement, "host", "client", "session").is_ok());
        advertisement.tools[0].allow_result_egress = false;
        assert!(validate_advertisement(&advertisement, "host", "client", "session").is_err());
        advertisement.tools[0].allow_result_egress = true;
        advertisement.tools[0].schema["x_nola"]["target"] = json!("host");
        assert!(validate_advertisement(&advertisement, "host", "client", "session").is_err());
    }

    #[test]
    fn bridge_and_core_catalog_share_one_canonical_device_id_set() {
        for tool_id in CANONICAL_DEVICE_TOOL_IDS {
            let value = schema(tool_id);
            assert_eq!(device_tool_name(&value).unwrap(), *tool_id);
            let metadata = builtin_tool_metadata(tool_id).unwrap();
            assert_eq!(
                metadata.capability,
                canonical_device_tool_capability(tool_id).unwrap()
            );
        }
        for obsolete_id in [
            "device_read_clipboard",
            "device_write_clipboard",
            "device_get_calendar_events",
            "device.calendar.search",
        ] {
            assert!(
                device_tool_name(&schema(obsolete_id)).is_err(),
                "obsolete ID {obsolete_id}"
            );
            assert!(
                builtin_tool_metadata(obsolete_id).is_none(),
                "obsolete ID {obsolete_id}"
            );
        }
    }

    #[test]
    fn advertisement_capability_must_match_core_catalog() {
        let advertisement = DeviceBridgeAdvertisement {
            protocol_version: 1,
            host_id: "host".into(),
            client_id: "client".into(),
            session_id: "session".into(),
            tools: vec![DeviceToolAdvertisement {
                schema: schema("device_clipboard_read"),
                capability: "files_read".into(),
                allow_result_egress: true,
            }],
        };
        assert!(validate_advertisement(&advertisement, "host", "client", "session").is_err());
    }

    #[test]
    fn result_is_bound_to_request_and_size_limited() {
        let request = DeviceToolRequest {
            protocol_version: 1,
            nonce: "nonce".into(),
            request_id: "call".into(),
            host_id: "host".into(),
            client_id: "client".into(),
            session_id: "session".into(),
            tool_name: "device_get_location".into(),
            capability: "location".into(),
            arguments: json!({}),
            expires_at: 200,
        };
        let result = DeviceToolResult {
            protocol_version: 1,
            nonce: "nonce".into(),
            request_id: "call".into(),
            host_id: "host".into(),
            client_id: "client".into(),
            session_id: "session".into(),
            tool_name: "device_get_location".into(),
            output: json!({"latitude": 1.0}),
            error: false,
        };
        assert!(validate_device_tool_result(&request, &result, 100).is_ok());
        let mut mismatched_request = request.clone();
        mismatched_request.tool_name = "device_read_clipboard".into();
        let mut mismatched_result = result.clone();
        mismatched_result.tool_name = "device_read_clipboard".into();
        assert!(validate_device_tool_result(&mismatched_request, &mismatched_result, 100).is_err());
        let mut replay = result;
        replay.request_id = "changed".into();
        assert!(validate_device_tool_result(&request, &replay, 100).is_err());
    }
}
