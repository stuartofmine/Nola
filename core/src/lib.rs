// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements lib logic in the Rust core module.
// - Interfaces:
//   - Internal implementation details only.
// - APIs:
//   - none
//
//! Nola Core - Platform-neutral Agent policy, context, data-source, and FFI facade.

pub mod prompt_cache;
pub mod utils;
// Agent policy and orchestration live under `agent`; platform clients use the
// versioned Agent Core ABI exported below.
pub mod agent;
pub mod agent_model;
pub mod agent_trace;
pub mod code_intelligence;
pub mod context_engine;
pub mod data_source;
pub mod device_bridge;
pub mod external_context;
pub mod host_auth;
pub mod lifecycle_hooks;
pub mod memory;
pub mod messaging_channel;
pub mod mobile_crypto;
pub mod tool_catalog;
pub mod tool_policy;

// Re-export FFI functions from modules
pub use agent::prompt_composer::{
    nola_core_compose_agent_prompt_json, nola_core_prompt_composer_abi_version,
};
pub use agent::resource_access::{
    nola_core_resource_access_abi_version, nola_core_resource_access_evaluate_json,
};
pub use agent_model::{
    nola_core_parse_agent_model_output, nola_core_stitch_agent_model_continuation,
};
pub use agent_trace::{
    reduce_agent_trace, AgentRunProjection, AgentTraceContext, AgentTraceEvent, AgentTraceKind,
    AgentTraceReducer, AGENT_TRACE_SCHEMA_VERSION,
};
pub use context_engine::nola_core_context_plan;
pub use context_engine::{nola_core_context_engine_abi_version, nola_core_context_prepare};
pub use data_source::{
    nola_core_data_source_abi_version, nola_core_data_source_resolve,
    nola_core_data_source_seal_contract,
};
pub use lifecycle_hooks::{
    nola_core_lifecycle_hook_abi_version, nola_core_lifecycle_hook_build_registry,
    nola_core_lifecycle_hook_plan, nola_core_lifecycle_hook_project_execution,
};
pub use memory::{nola_core_memory_select_json, nola_core_memory_selection_abi_version};
pub use prompt_cache::{nola_core_prompt_cache_abi_version, nola_core_prompt_cache_diagnose};
pub use tool_policy::{
    nola_core_tool_policy_abi_version, nola_core_tool_policy_mcp_allowed,
    nola_core_tool_policy_resolve,
};
pub use utils::nola_core_free_c_string;

pub use nola_kernel::compaction_maintenance;
pub use nola_kernel::compaction_summary;
pub use nola_kernel::strict_json;

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;
    use std::ptr;

    #[test]
    fn test_free_c_string_null_and_valid_pointer() {
        nola_core_free_c_string(ptr::null_mut());
        let allocated = CString::new("{\"ok\":true}").unwrap().into_raw();
        assert!(!allocated.is_null());
        nola_core_free_c_string(allocated);
    }
}
