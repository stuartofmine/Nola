// SPDX-License-Identifier: Apache-2.0
//! Compare fresh execution facts against the immutable Core-owned Run policy.
use nola_agent_engine::agent::{
    context_window::ContextWindowConfig, host::HostCapabilityCatalog,
    permissions::PermissionRuleSet, protocol::AgentCoreConfig,
};
use nola_host_protocol::HostInvocationBinding;
use nola_store_port::run_recovery_inspection::RunRecoveryInspectionRepository;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunRecoveryAuthority {
    pub host_catalog: Option<HostCapabilityCatalog>,
    pub tools: Vec<Value>,
    pub permission_rules: PermissionRuleSet,
    pub context_window: ContextWindowConfig,
    pub host_invocation: HostInvocationBinding,
}

#[derive(Debug, Serialize)]
pub struct RunRecoveryAuthorityReceipt {
    pub conversation_id: String,
    pub run_id: String,
    pub assistant_message_id: String,
    pub invocation_config_digest: String,
}

pub fn validate_run_recovery_authority<R: RunRecoveryInspectionRepository + ?Sized>(
    store: &R,
    conversation_id: String,
    expected_run_id: String,
    assistant_message_id: String,
    expected_invocation_config_digest: String,
    current: RunRecoveryAuthority,
) -> Result<RunRecoveryAuthorityReceipt, String> {
    validate_recovery_authority(store, conversation_id, expected_run_id, assistant_message_id,
        expected_invocation_config_digest, current, false)
}

/// Reconstitute only Core-installed capabilities for an embedded Host. The
/// platform must freshly observe every native tool; this cannot restore a
/// removed physical capability or change the frozen execution model/policy.
pub fn validate_mobile_run_recovery_authority<R: RunRecoveryInspectionRepository + ?Sized>(
    store: &R, conversation_id: String, expected_run_id: String,
    assistant_message_id: String, expected_invocation_config_digest: String,
    current: RunRecoveryAuthority,
) -> Result<RunRecoveryAuthorityReceipt, String> {
    validate_recovery_authority(store, conversation_id, expected_run_id, assistant_message_id,
        expected_invocation_config_digest, current, true)
}

fn validate_recovery_authority<R: RunRecoveryInspectionRepository + ?Sized>(
    store: &R, conversation_id: String, expected_run_id: String,
    assistant_message_id: String, expected_invocation_config_digest: String,
    mut current: RunRecoveryAuthority, mobile: bool,
) -> Result<RunRecoveryAuthorityReceipt, String> {
    // One repository snapshot proves ownership, terminal state, checkpoint and
    // configuration. Re-reading and hashing it in each adapter adds no proof.
    let (exact, frozen) = crate::run_invocation_recovery::load_exact_run_invocation_with_config(
        store, conversation_id.clone(), expected_run_id.clone(), assistant_message_id.clone(),
    )?.ok_or("Recovery Run is unavailable")?;
    if exact.invocation_config_digest != expected_invocation_config_digest || exact.status == "sealed" {
        return Err("Recovery invocation witness changed".into());
    }
    if mobile { restore_core_delegation_observation(&frozen, &mut current)?; }
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "Recovery clock is unavailable")?.as_millis() as u64;
    validate_facts(&frozen, &current, now_ms)?;
    Ok(RunRecoveryAuthorityReceipt {
        conversation_id, run_id: expected_run_id, assistant_message_id,
        invocation_config_digest: expected_invocation_config_digest,
    })
}

fn restore_core_delegation_observation(
    frozen: &AgentCoreConfig, current: &mut RunRecoveryAuthority,
) -> Result<(), String> {
    if frozen.turn_identity.delegation.is_some() {
        // capture_parent installs this Core handler. It does not require an
        // expiring device manifest and is never synthesized from user text.
        let frozen_catalog = frozen.host_catalog.as_ref().ok_or("Frozen Host catalog is missing")?;
        if !frozen_catalog.capabilities.contains_key("spawn_agent") { return Err("Frozen delegation capability is missing".into()); }
        let catalog = current.host_catalog.as_mut().ok_or("Current Host catalog is missing")?;
        for name in ["spawn_agent", "send_agent_message", "read_task", "wait_task"] {
            let Some(capability) = frozen_catalog.capabilities.get(name) else { continue };
            if let Some(observed) = catalog.capabilities.get(name) {
                if observed != capability { return Err("Current delegation capability changed".into()); }
            } else { catalog.capabilities.insert(name.into(), capability.clone()); }
            if !current.tools.iter().any(|t|t.pointer("/function/name").and_then(Value::as_str)==Some(name)) {
                if let Some(schema)=frozen.tools.iter().find(|t|t.pointer("/function/name").and_then(Value::as_str)==Some(name)) {
                    current.tools.push(schema.clone());
                }
            }
        }
    }
    current.tools = select_fresh_tool_observations(&frozen.tools, std::mem::take(&mut current.tools))?;
    Ok(())

}

pub(crate) fn validate_facts(
    frozen: &AgentCoreConfig,
    current: &RunRecoveryAuthority,
    now_ms: u64,
) -> Result<(), String> {
    current.host_invocation.validate().map_err(str::to_string)?;
    let binding = frozen
        .turn_identity
        .host_invocation
        .as_ref()
        .ok_or("Missing frozen Host invocation")?;
    // The client policy digest includes expiring observation receipts. It is
    // retained unchanged, but cannot substitute for comparison of Core facts.
    let mut current_binding = current.host_invocation.clone();
    current_binding.execution_policy_sha256 = binding.execution_policy_sha256.clone();
    if binding != &current_binding {
        return Err("Recovery model authority changed".into());
    }
    if frozen.host_catalog != current.host_catalog {
        return Err("Recovery Host catalog changed".into());
    }
    if frozen.permission_rules != current.permission_rules {
        return Err("Recovery permission authority changed".into());
    }
    if serde_json::to_value(&frozen.context_window).map_err(|_| "Invalid frozen context policy")?
        != serde_json::to_value(&current.context_window)
            .map_err(|_| "Invalid current context policy")?
    {
        return Err("Recovery context policy changed".into());
    }
    for tool in &current.tools {
        if let Some(metadata) = tool.get("x_nola").and_then(Value::as_object) {
            let fields = [
                "policy_digest",
                "manifest_digest",
                "manifest_valid_until_ms",
            ];
            if fields.iter().any(|key| metadata.contains_key(*key)) {
                let valid_digest = |key| {
                    metadata
                        .get(key)
                        .and_then(Value::as_str)
                        .is_some_and(|value| {
                            value.len() == 64
                                && value
                                    .bytes()
                                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                        })
                };
                if !valid_digest("policy_digest")
                    || !valid_digest("manifest_digest")
                    || !metadata
                        .get("manifest_valid_until_ms")
                        .and_then(Value::as_u64)
                        .is_some_and(|expires| expires > now_ms)
                {
                    return Err("Recovery tool observation is unavailable or expired".into());
                }
            }
        }
    }
    if stable_tools(&frozen.tools) != stable_tools(&current.tools) {
        return Err("Recovery tool schemas changed".into());
    }
    Ok(())
}

/// A Host may observe a wider installed catalog, but a child only receives
/// the tools in its frozen policy, in that policy's order. Missing or duplicate
/// names are rejected rather than falling back to old observation receipts.
pub(crate) fn select_fresh_tool_observations(
    frozen: &[Value], observed: Vec<Value>,
) -> Result<Vec<Value>, String> {
    let mut by_name = std::collections::BTreeMap::new();
    for tool in observed {
        let name = tool.pointer("/function/name").and_then(Value::as_str)
            .filter(|name| !name.is_empty()).ok_or("Current tool observation has no identity")?.to_owned();
        if by_name.insert(name, tool).is_some() {
            return Err("Duplicate current tool observation".into());
        }
    }
    let mut selected = Vec::with_capacity(frozen.len());
    for tool in frozen {
        let name = tool.pointer("/function/name").and_then(Value::as_str)
            .ok_or("Frozen tool has no identity")?;
        selected.push(by_name.remove(name).ok_or("Frozen child tool is currently unavailable")?);
    }
    if stable_tools(frozen) != stable_tools(&selected) {
        return Err("Current child tool schema or authority changed".into());
    }
    Ok(selected)
}

// Only top-level, Host-produced observation receipt fields may renew. Tool
// parameters, routes, permission contracts and all other metadata stay exact.
fn stable_tools(tools: &[Value]) -> Vec<Value> {
    tools
        .iter()
        .map(|tool| {
            let mut tool = tool.clone();
            if let Some(metadata) = tool.get_mut("x_nola").and_then(Value::as_object_mut) {
                for key in [
                    "policy_digest",
                    "manifest_digest",
                    "manifest_valid_until_ms",
                ] {
                    metadata.remove(key);
                }
            }
            tool
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn fixture() -> (AgentCoreConfig, RunRecoveryAuthority) {
        let mut frozen = AgentCoreConfig::default();
        frozen.turn_identity.host_invocation = Some(
            crate::run_recovery_port_tests::recovery_host_binding("model"),
        );
        frozen.host_catalog = Some(HostCapabilityCatalog::empty("host"));
        frozen.tools = vec![
            json!({"type":"function","function":{"name":"read","parameters":{"type":"object"}},
            "x_nola":{"authorization":"ask","policy_digest":"a".repeat(64),"manifest_digest":"b".repeat(64),"manifest_valid_until_ms":100}}),
        ];
        let current = RunRecoveryAuthority {
            host_catalog: frozen.host_catalog.clone(),
            tools: frozen.tools.clone(),
            permission_rules: frozen.permission_rules.clone(),
            context_window: frozen.context_window.clone(),
            host_invocation: frozen.turn_identity.host_invocation.clone().unwrap(),
        };
        (frozen, current)
    }
    struct SingleSnapshot {
        value: nola_store_port::run_recovery_inspection::StoredExactRunInvocation,
        reads: std::cell::Cell<usize>,
    }
    impl RunRecoveryInspectionRepository for SingleSnapshot {
        type Error = String;
        fn load_exact_run_invocation(&self, conversation: &str, run: &str, assistant: &str)
            -> Result<Option<nola_store_port::run_recovery_inspection::StoredExactRunInvocation>, String> {
            assert_eq!((conversation, run, assistant), ("conversation", "run", "assistant"));
            self.reads.set(self.reads.get() + 1);
            assert_eq!(self.reads.get(), 1, "authority must use one pinned invocation snapshot");
            Ok(Some(self.value.clone()))
        }
        fn list_recoverable_runs(&self, _: Option<&str>, _: usize)
            -> Result<Vec<nola_store_port::run_recovery_inspection::RecoverableRunRef>, String> { unreachable!() }
        fn inspect_unknown_effect_recovery(&self, _: &str, _: &str, _: &str)
            -> Result<Option<nola_store_port::run_recovery_inspection::StoredUnknownEffectRecoveryInspection>, String> { unreachable!() }
        fn inspect_exact_run(&self, _: &str)
            -> Result<Option<nola_store_port::run_recovery_inspection::StoredRunRecoveryInspection>, String> { unreachable!() }
    }
    #[test]
    fn authority_uses_one_verified_snapshot_and_still_rejects_tampered_bytes() {
        use sha2::{Digest, Sha256};
        for mobile in [false, true] {
            for tampered in [false, true] {
                let (mut frozen, mut current) = fixture();
                frozen.tools[0]["x_nola"]["manifest_valid_until_ms"] = json!(u64::MAX);
                current.tools = frozen.tools.clone();
                let json = serde_json::to_string(&frozen).unwrap();
                let digest = format!("{:x}", Sha256::digest(json.as_bytes()));
                let snapshot = SingleSnapshot {
                    value: nola_store_port::run_recovery_inspection::StoredExactRunInvocation {
                        conversation_id: "conversation".into(), run_id: "run".into(), assistant_message_id: "assistant".into(),
                        status: "running".into(), completed_at_ms: None, terminal: None, core_checkpoint: None,
                        invocation_config_json: if tampered { format!("{json} ") } else { json },
                        invocation_config_digest: digest.clone(),
                    }, reads: std::cell::Cell::new(0),
                };
                let result = validate_recovery_authority(&snapshot, "conversation".into(), "run".into(),
                    "assistant".into(), digest, current, mobile);
                assert_eq!(result.is_err(), tampered);
                assert_eq!(snapshot.reads.get(), 1);
            }
        }
    }

    #[test]
    fn mobile_parent_restores_only_its_frozen_core_capability() {
        let mut base = crate::user_input_wait_tests::config();
        base.turn_identity.host_invocation = Some(crate::run_recovery_port_tests::recovery_host_binding("fixture"));
        base.turn_identity.model_capability.native_tool_calls = true;
        let mut current = RunRecoveryAuthority {
            host_catalog: base.host_catalog.clone(), tools: base.tools.clone(),
            permission_rules: base.permission_rules.clone(), context_window: base.context_window.clone(),
            host_invocation: base.turn_identity.host_invocation.clone().unwrap(),
        };
        let frozen = crate::mobile_delegation_configuration::capture_parent(base,
            crate::mobile_delegation_configuration::MobileDelegationSettings {
                expert_leaf: false,
                project_root: "/fixture".into(), selected_profile: None, agents: Default::default(), independent_model_profiles: false,
            }).unwrap();
        assert!(frozen.turn_identity.delegation.is_some());
        assert!(validate_facts(&frozen, &current, 0).is_err());
        restore_core_delegation_observation(&frozen, &mut current).unwrap();
        validate_facts(&frozen, &current, 0).unwrap();
        assert_eq!(current.tools, frozen.tools);
        let mut missing_native = current.clone();
        missing_native.tools.retain(|t| t.pointer("/function/name").and_then(Value::as_str) != Some("read_file"));
        assert!(restore_core_delegation_observation(&frozen, &mut missing_native).is_err());
        for name in ["spawn_agent", "send_agent_message", "read_task", "wait_task"] {
            let mut changed = current.clone();
            changed.host_catalog.as_mut().unwrap().capabilities.get_mut(name).unwrap().title = "changed".into();
            assert!(restore_core_delegation_observation(&frozen, &mut changed).is_err());
        }
    }

    #[test]
    fn child_observation_renews_only_receipts_and_cannot_expand_its_frozen_tools() {
        let (frozen, mut current) = fixture();
        let before = serde_json::to_value(&frozen).unwrap();
        let mut fresh = frozen.tools[0].clone();
        fresh["x_nola"]["manifest_valid_until_ms"] = json!(2000);
        fresh["x_nola"]["manifest_digest"] = json!("d".repeat(64));
        fresh["x_nola"]["policy_digest"] = json!("e".repeat(64));
        let extra = json!({"type":"function","function":{"name":"unadmitted","parameters":{"type":"object"}}});
        current.tools = select_fresh_tool_observations(&frozen.tools, vec![extra, fresh.clone()]).unwrap();
        assert_eq!(current.tools.len(), frozen.tools.len());
        validate_facts(&frozen, &current, 1500).unwrap();
        assert!(validate_facts(&frozen, &current, 2000).is_err());
        assert_eq!(serde_json::to_value(&frozen).unwrap(), before);
        assert!(select_fresh_tool_observations(&frozen.tools, vec![]).is_err());
        assert!(select_fresh_tool_observations(&frozen.tools, vec![fresh.clone(), fresh.clone()]).is_err());
        fresh["x_nola"]["authorization"] = json!("allow");
        assert!(select_fresh_tool_observations(&frozen.tools, vec![fresh]).is_err());
    }

    #[test]
    fn refreshed_observation_receipts_do_not_change_frozen_execution_authority() {
        let (frozen, mut current) = fixture();
        current.tools[0]["x_nola"]["policy_digest"] = json!("c".repeat(64));
        current.tools[0]["x_nola"]["manifest_digest"] = json!("c".repeat(64));
        current.tools[0]["x_nola"]["manifest_valid_until_ms"] = json!(1000);
        current.host_invocation.execution_policy_sha256 = "c".repeat(64);
        assert!(validate_facts(&frozen, &current, 0).is_ok());
        assert!(validate_facts(&frozen, &current, 1000).is_err());
        assert_eq!(
            frozen
                .turn_identity
                .host_invocation
                .unwrap()
                .execution_policy_sha256,
            "b".repeat(64)
        );
    }
    #[test]
    fn rejects_real_permission_schema_host_and_context_changes() {
        let (frozen, current) = fixture();
        let mut changed = current.clone();
        changed.tools[0]["x_nola"]["authorization"] = json!("allow");
        assert!(validate_facts(&frozen, &changed, 0).is_err());
        let mut changed = current.clone();
        changed.tools[0]["function"]["parameters"]["policy_digest"] = json!("schema change");
        assert!(validate_facts(&frozen, &changed, 0).is_err());
        let mut changed = current.clone();
        changed.host_catalog.as_mut().unwrap().host_id = nola_host_protocol::HostId("other".into());
        assert!(validate_facts(&frozen, &changed, 0).is_err());
        let mut changed = current.clone();
        changed.context_window.reserved_output_tokens += 1;
        assert!(validate_facts(&frozen, &changed, 0).is_err());
        let mut changed = current;
        changed.tools.clear();
        assert!(validate_facts(&frozen, &changed, 0).is_err());
    }
    #[test]
    fn exact_model_authority_is_not_replaced_by_a_policy_digest() {
        let (frozen, mut current) = fixture();
        current.host_invocation.generation.thinking_enabled = true;
        assert!(validate_facts(&frozen, &current, 0).is_err());
        let (frozen, mut current) = fixture();
        if let nola_host_protocol::HostModelRouteBinding::Http(route) =
            &mut current.host_invocation.primary
        {
            route.credential_sha256 = "c".repeat(64);
        }
        current.host_invocation.output_admission.routes[0].route =
            current.host_invocation.primary.clone();
        assert!(current.host_invocation.validate().is_ok());
        assert!(validate_facts(&frozen, &current, 0).is_err());
    }
    #[test]
    fn recovery_validation_is_a_strict_read_only_application_contract() {
        let (_, current) = fixture();
        let mut wire = json!({"type":"validate_run_recovery_authority", "conversation_id":"c",
            "expected_run_id":"r", "assistant_message_id":"a", "expected_invocation_config_digest":"b".repeat(64),
            "current": current});
        let command: crate::ApplicationCommand = serde_json::from_value(wire.clone()).unwrap();
        assert!(command.is_read_only_query());
        wire["current"]["unexpected_authorization"] = json!(true);
        assert!(serde_json::from_value::<crate::ApplicationCommand>(wire).is_err());
    }
}
