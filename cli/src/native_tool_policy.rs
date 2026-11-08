// SPDX-License-Identifier: Apache-2.0
//! Native execution policy receipt validation, independent of CLI parsing.
use base64::Engine as _;
use serde_json::Value;

pub(crate) fn decode_desktop_host_tool_allowlist(
    encoded: &str,
    expected_policy_digest: &str,
    expected_manifest_digest: &str,
    expected_session_id: &str,
    now_ms: u64,
) -> Result<Vec<String>, String> {
    let data = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .map_err(|_| "Agent Host tool policy receipt is not valid Base64.".to_string())?;
    let receipt: Value = serde_json::from_slice(&data)
        .map_err(|_| "Agent Host tool policy receipt is not valid JSON.".to_string())?;
    let string_at = |pointer: &str| {
        receipt
            .pointer(pointer)
            .and_then(Value::as_str)
            .unwrap_or_default()
    };
    if string_at("/policy_digest") != expected_policy_digest
        || string_at("/manifest_digest") != expected_manifest_digest
    {
        return Err("Agent Host tool policy digest does not match the receipt.".to_string());
    }
    if string_at("/expected_identity/client_id") != "mac"
        || string_at("/expected_identity/session_id") != expected_session_id
    {
        return Err("Agent Host tool policy identity does not match this run.".to_string());
    }
    let expires_at = receipt
        .pointer("/manifest_valid_until_ms")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    if expires_at <= now_ms {
        return Err("Agent Host tool policy receipt has expired.".to_string());
    }
    let mut tools = receipt
        .pointer("/enabled_tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|tool| tool.get("enabled").and_then(Value::as_bool) == Some(true))
        .filter(|tool| tool.get("route").and_then(Value::as_str) == Some("agent_host"))
        .filter_map(|tool| tool.get("tool_id").and_then(Value::as_str))
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>();
    tools.sort();
    tools.dedup();
    if tools.is_empty() {
        return Err("Agent Host tool policy enables no tools.".to_string());
    }
    let input: nola_core::tool_policy::ToolPolicyInput = serde_json::from_value(receipt["policy_input"].clone())
        .map_err(|_| "Agent Host tool policy input is missing or invalid")?;
    let resolved = nola_core::tool_policy::resolve_tool_policy(&input);
    let mut recomputed = resolved.enabled_tools.iter()
        .filter(|tool| tool.enabled && tool.route == Some(nola_core::tool_policy::ToolRoute::AgentHost))
        .map(|tool| tool.tool_id.to_ascii_lowercase()).collect::<Vec<_>>();
    recomputed.sort(); recomputed.dedup();
    if resolved.policy_digest != expected_policy_digest || resolved.manifest_digest != expected_manifest_digest
        || recomputed != tools {
        return Err("Agent Host tool policy input does not match its receipt".into());
    }
    Ok(tools)
}

pub(crate) fn desktop_mcp_policy(session: &str) -> Result<Option<nola_core::tool_policy::ToolPolicyInput>, String> {
    if std::env::var_os("NOLA_DESKTOP_HOST").is_none() { return Ok(None); }
    let encoded = std::env::var("NOLA_CORE_TOOL_POLICY_RECEIPT").map_err(|_| "Missing desktop policy")?;
    let receipt: Value = serde_json::from_slice(&base64::engine::general_purpose::STANDARD.decode(encoded.trim())
        .map_err(|_| "Invalid desktop policy")?).map_err(|_| "Invalid desktop policy")?;
    // Child sessions need their own policy; a parent receipt cannot widen them.
    if receipt.pointer("/expected_identity/session_id").and_then(Value::as_str) != Some(session) { return Ok(None); }
    let policy: nola_core::tool_policy::ToolPolicyInput = serde_json::from_value(receipt["policy_input"].clone())
        .map_err(|_| "Invalid desktop policy input")?;
    // Native entry checks expiry at admission. Later rounds in that same
    // owned process retain its policy; each concrete approval is revalidated.
    decode_desktop_host_tool_allowlist(&encoded,
        &std::env::var("NOLA_CORE_TOOL_POLICY_DIGEST").map_err(|_| "Missing desktop policy digest")?,
        &std::env::var("NOLA_CORE_TOOL_MANIFEST_DIGEST").map_err(|_| "Missing desktop manifest digest")?,session,policy.now_ms)?;
    Ok(Some(policy))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn receipt_cannot_add_tools_or_change_original_policy() {
        let input: nola_core::tool_policy::ToolPolicyInput = serde_json::from_value(serde_json::json!({
            "schema_version":2,"platform":"macos","now_ms":1000,
            "expected_identity":{"host_id":"local","client_id":"mac","session_id":"session"},
            "runtime":{"manifest_id":"test","manifest_version":1,"issued_at_ms":1,"expires_at_ms":60000,
                "host_id":"local","client_id":"mac","session_id":"session","routes":["agent_host"],
                "capabilities":["files_read","network"],"authorizations":{"files_read":"granted","network":"granted"},
                "tools":[{"tool_id":"read_file","handler_id":"read_file","route":"agent_host","available":true,
                    "schema":{"type":"function","function":{"name":"read_file","parameters":{"type":"object"}}}}]},
            "user_policy":{"max_risk":"high"}
        })).unwrap();
        let output = nola_core::tool_policy::resolve_tool_policy(&input);
        let mut receipt = serde_json::to_value(&output).unwrap();
        receipt["expected_identity"] = serde_json::to_value(&input.expected_identity).unwrap();
        receipt["policy_input"] = serde_json::to_value(&input).unwrap();
        let check = |value: &Value| decode_desktop_host_tool_allowlist(
            &base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(value).unwrap()),
            &output.policy_digest,&output.manifest_digest,"session",1001);
        assert_eq!(check(&receipt).unwrap(), vec!["read_file"]);
        let mut changed = receipt.clone();
        changed["enabled_tools"][0]["tool_id"] = serde_json::json!("mcp_connect");
        assert!(check(&changed).is_err());
        receipt["policy_input"]["user_policy"]["deny_capabilities"] = serde_json::json!(["network"]);
        assert!(check(&receipt).is_err());
    }
}
