// SPDX-License-Identifier: Apache-2.0
//! Host-issued availability evidence. This is never an operation grant.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostToolPlanReference {
    pub manifest_id: String,
    pub host_id: String,
    pub client_id: String,
    pub session_id: String,
    pub catalog_revision: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn plan() -> HostToolPlanSnapshot {
        HostToolPlanSnapshot { schema_version: 1, reference: HostToolPlanReference {
            manifest_id: "manifest".into(), host_id:"host".into(), client_id:"client".into(),
            session_id:"session".into(), catalog_revision:"a".repeat(64),
        }, issued_at_ms:1000, expires_at_ms:301000, tools:vec![HostToolPlanTool {
            tool_id:"read_file".into(),route:"agent_host".into(),
            schema:serde_json::json!({"type":"function","function":{"name":"read_file","parameters":{"type":"object"}}}),
        }] }
    }
    #[test]
    fn validates_identity_freshness_and_original_run_observation() {
        let value=plan();
        assert!(value.validate("host","client","session",Some(1000)).is_ok());
        assert!(value.validate("other","client","session",Some(1000)).is_err());
        assert!(value.validate("host","other","session",Some(1000)).is_err());
        assert!(value.validate("host","client","other",Some(1000)).is_err());
        assert!(value.validate("host","client","session",Some(301000)).is_err());
        assert!(value.validate("host","client","session",Some(999)).is_err());
        assert!(value.validate("host","client","session",None).is_ok());
    }
    #[test]
    fn rejects_noncanonical_metadata_and_duplicate_or_unbound_tools() {
        let mutations:Vec<Box<dyn Fn(&mut HostToolPlanSnapshot)>>=vec![
            Box::new(|p|p.schema_version=2), Box::new(|p|p.reference.catalog_revision="A".repeat(64)),
            Box::new(|p|p.reference.manifest_id=" ".into()),Box::new(|p|p.reference.manifest_id="a\nb".into()),
            Box::new(|p|p.reference.manifest_id="x".repeat(513)),Box::new(|p|p.issued_at_ms=0),
            Box::new(|p|p.expires_at_ms+=1),Box::new(|p|{p.issued_at_ms=9_007_199_254_740_990;p.expires_at_ms=p.issued_at_ms+2;}),
            Box::new(|p|p.tools.push(p.tools[0].clone())),Box::new(|p|p.tools[0].route="device".into()),
            Box::new(|p|p.tools[0].tool_id=" ".into()),Box::new(|p|p.tools[0].schema["function"]["name"]="other".into()),
            Box::new(|p|p.tools[0].schema["function"]["parameters"]=serde_json::json!([])),
        ];
        for mutate in mutations { let mut value=plan(); mutate(&mut value); assert!(value.validate("host","client","session",None).is_err()); }
    }
    #[test]
    fn rejects_unknown_protocol_fields() {
        let mut value=serde_json::to_value(plan()).unwrap(); value["policy_digest"]="client".into();
        assert!(serde_json::from_value::<HostToolPlanSnapshot>(value).is_err());
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostToolPlanTool {
    pub tool_id: String,
    pub route: String,
    pub schema: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostToolPlanSnapshot {
    pub schema_version: u32,
    pub reference: HostToolPlanReference,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
    pub tools: Vec<HostToolPlanTool>,
}

impl HostToolPlanSnapshot {
    pub fn validate(&self, host: &str, client: &str, session: &str, fresh_at: Option<u64>) -> Result<(), String> {
        let reference = &self.reference;
        if self.schema_version != 1 || reference.host_id != host || reference.client_id != client
            || reference.session_id != session || [host,client,session,reference.manifest_id.as_str()].iter()
                .any(|value| value.trim().is_empty() || value.len() > 512 || value.chars().any(char::is_control))
            || reference.catalog_revision.len() != 64 || !reference.catalog_revision.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || self.issued_at_ms == 0 || self.expires_at_ms <= self.issued_at_ms
            || self.expires_at_ms > 9_007_199_254_740_991
            || self.expires_at_ms - self.issued_at_ms > 300_000 {
            return Err("invalid_host_tool_plan".into());
        }
        if fresh_at.is_some_and(|now| now < self.issued_at_ms || now >= self.expires_at_ms) {
            return Err("expired_host_tool_plan".into());
        }
        let mut ids = std::collections::BTreeSet::new();
        for tool in &self.tools {
            if tool.route != "agent_host" || tool.tool_id.trim().is_empty() || tool.tool_id.len() > 512
                || tool.tool_id.chars().any(char::is_control) || !ids.insert(&tool.tool_id)
                || tool.schema.get("type").and_then(Value::as_str) != Some("function")
                || tool.schema.pointer("/function/name").and_then(Value::as_str) != Some(tool.tool_id.as_str())
                || !tool.schema.pointer("/function/parameters").is_some_and(Value::is_object) {
                return Err("invalid_host_tool_plan_tool".into());
            }
        }
        Ok(())
    }
}
