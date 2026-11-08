// SPDX-License-Identifier: Apache-2.0
//! Explicit fixture Host descriptor; all starts use the production atomic API.
#![allow(dead_code)]
use nola_agent_engine::agent::{AgentCoreConfig, AgentSubmission, GoalRuntimeCommand};
use nola_application::goal_execution::*;
use nola_domain::goal_execution::*;
use serde_json::json;
pub fn config() -> AgentCoreConfig {
    let route = json!({"kind":"http","protocol":"chat_completions","output_token_field":"max_tokens","provider":"fixture", "endpoint":"https://fixture.invalid/v1",
        "endpoint_query_sha256":null,"model":"original-model","credential_sha256":"a".repeat(64),
        "credential_source":{"kind":"ephemeral"},"endpoint_query_source":{"kind":"none"}});
    let mut config = super::model_identity::config();
    config.host_catalog =
        Some(nola_agent_engine::agent::host::HostCapabilityCatalog::empty("host"));
    config.session_id = Some(nola_agent_engine::agent::protocol::SessionId::from_string(
        "core-c".into(),
    ));
    config.tools = vec![json!({
        "type": "function",
        "function": {"name": "fixture_tool", "parameters": {"type": "object", "properties": {}}},
        "private_context_marker": "must-not-be-disclosed"
    })];
    config.context_window.context_window_tokens = 4096;
    config.context_window.reserved_output_tokens = 64;
    config.context_window.prompt_token_limit = Some(2048);
    config.context_window.auto_compact_tokens = Some(3000);
    config.context_window.compaction_target_tokens = Some(1500);
    config.context_window.auto_compact_percent = 75;
    config.context_window.compaction_target_percent = 40;
    config.turn_identity.remote_tool_plan = Some(serde_json::from_value(json!({
        "schema_version":1,"reference":{"manifest_id":"frozen-plan","host_id":"host","client_id":"client",
            "session_id":"c","catalog_revision":"a".repeat(64)},
        "issued_at_ms":1,"expires_at_ms":300001,"tools":[]
    })).unwrap());
    config.turn_identity.host_invocation = Some(serde_json::from_value(json!({
        "version":nola_host_protocol::HOST_INVOCATION_BINDING_VERSION,
        "primary":route,"fallbacks":[],
        "generation":{"temperature":0,"top_p":0,"max_output_tokens":16384,
            "max_retries":2,"parallel_tool_calls":null,
            "reasoning_selection":null,"sampling_override":false,"generation_options":{},
            "reasoning_tier":"balanced",
            "thinking_enabled":false,"prompt_cache":false,"prompt_cache_key_sha256":null,
            "prompt_cache_key_prefix_sha256":null,"prompt_cache_retention":null},
        "output_admission":{"version":nola_host_protocol::HOST_OUTPUT_ADMISSION_VERSION,
            "routes":[{"route":route,"max_input_tokens":null,"max_output_tokens":null,"max_total_tokens":null}],
            "preference":"default","resolved":{"tokens":16384,"source":"product_default"}},
        "execution_policy_sha256":"b".repeat(64)
    })).unwrap());
    config.turn_identity.host_recovery = Some(nola_host_protocol::HostRunRecoveryReference {
        adapter: "fixture.host".into(),
        handle: "opaque-descriptor".into(),
        sha256: "c".repeat(64),
    });
    config
}

pub fn admit_start(
    store: &mut nola_store::NolaStore,
    command: nola_application::ReduceAndCommitIdleSubmission,
) -> Result<nola_application::AcceptedIdleSubmission, String> {
    let AgentSubmission::GoalRuntime { envelope } = command.submission else {
        return Err("fixture requires Goal Start".into());
    };
    let GoalRuntimeCommand::Start {
        id, now_epoch_ms, ..
    } = &envelope.command
    else {
        return Err("fixture requires Goal Start".into());
    };
    if *now_epoch_ms != command.committed_at_ms {
        return Err("fixture Goal time differs".into());
    }
    let mut core_config = command.core_config;
    let fixture = config();
    if core_config.turn_identity.host_invocation.is_none() {
        core_config.turn_identity.host_invocation = fixture.turn_identity.host_invocation;
    }
    core_config.turn_identity.host_recovery = fixture.turn_identity.host_recovery;
    core_config.initial_items.clear();
    if core_config.host_catalog.is_none() {
        core_config.host_catalog = fixture.host_catalog;
    }
    let source_host_id = core_config.host_catalog.as_ref().unwrap().host_id.0.clone();
    start_goal_execution(
        store,
        StartGoalExecution {
            identity: GoalExecutionBindingIdentity {
                conversation_id: command.conversation_id,
                goal_id: id.clone(),
                operation_id: envelope.operation_id.clone(),
                source_host_id,
            },
            template: GoalExecutionTemplate {
                model_selection: GoalModelSelection::DirectRoute,
                core_config,
            },
            envelope,
        },
    )
}

pub fn with_execution_binding(mut config: AgentCoreConfig) -> AgentCoreConfig {
    let fixture = self::config();
    config.turn_identity.host_invocation = fixture.turn_identity.host_invocation;
    config.turn_identity.host_recovery = fixture.turn_identity.host_recovery;
    config
}
