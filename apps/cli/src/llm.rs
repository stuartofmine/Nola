// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: LLM facade separating streaming transport from provider protocols.

#[path = "llm_parts/contracts.rs"]
mod contracts;
#[path = "llm_parts/auxiliary.rs"]
mod auxiliary;
pub(crate) use auxiliary::{invoke_auxiliary_once, AuxiliaryPhysicalOutcome};
#[path = "llm_parts/diagnostics.rs"]
mod diagnostics;
#[path = "llm_parts/provider_protocols.rs"]
mod provider_protocols;
#[path = "llm_parts/responses.rs"]
mod responses;
#[path = "llm_parts/request_snapshot.rs"]
mod request_snapshot;
#[path = "llm_parts/maintenance.rs"]
mod maintenance;
pub(crate) use maintenance::configure_cold_compaction_reasoning;
pub(crate) use contracts::PreparedModelRequest;
pub(crate) use request_snapshot::{request_configuration_identity, valid_compaction_summary};
#[path = "llm_parts/stream_state.rs"]
mod stream_state;
#[path = "llm_parts/streaming.rs"]
mod streaming;
#[path = "llm_parts/transport.rs"]
mod transport;
pub(crate) use transport::normalized_model_endpoint;

#[cfg(test)]
#[path = "llm_parts/output_budget_contract_tests.rs"]
mod output_budget_contract_tests;

// Only the facade is visible to Host callers. Provider implementations are
// private modules; sharing a helper requires an explicit parent-scoped export.
pub use responses::{model_route_identity, responses_route_identity};

#[cfg(test)]
pub(crate) fn siwc_request_body_for_test(config: &LlmConfig) -> Result<serde_json::Value, String> {
    responses::siwc_responses_body(config, &[serde_json::json!({"role":"user","content":"fixture only"})], &[], "auto")
}

// The CLI and agent-host binaries consume different subsets of this API.
pub(crate) use contracts::{
    resolved_model_protocol, resolved_output_token_field, supports_provider_fallbacks,
};
#[allow(unused_imports)]
pub use contracts::{
    LlmConfig, LlmEvent, LlmProviderTarget, LlmUsage, ProviderFailure, StreamedChoice, ToolCall,
};
#[allow(unused_imports)]
pub use diagnostics::{
    configure_execution_debug_log, configure_local_debug_log, log_local_debug_payload,
    ExecutionDebugLogGuard,
};
#[allow(unused_imports)]
pub(crate) use streaming::{
    apply_main_turn_prompt_cache_identity, configured_llm_max_retries, default_llm_max_retries,
    model_failure_projection, normalized_tool_arguments_value, output_token_parameter_name,
    prompt_cache_provider_id, stream_chat_checked_with_presentation_ticks,
    structured_provider_error_is_context_limit, ModelFailureProjection,
    ModelPresentationNotification,
};
#[allow(unused_imports)]
pub use streaming::{
    provider_continuation_namespace, stream_chat, stream_chat_checked,
    OPENAI_COMPATIBLE_CONTINUATION_NAMESPACE,
};
