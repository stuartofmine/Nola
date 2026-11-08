// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements lib logic in the Rust core module.
// - Interfaces:
//   - Internal implementation details only.
// - APIs:
//   - none
//
//! Nola Core - Rust library for event processing, decision making, and notification management
//!
//! This crate provides FFI functions for Swift to call into Rust for:
//! - URL parsing and raw event processing
//! - Event scheduling
//! - Decision engine (action recommendations)
//! - Rules engine (filtering, throttling)
//! - Model management (download, caching)
//! - Persistence (storage management)

// Module declarations
pub mod decision;
pub mod parser;
pub mod preprocessor;
pub mod prompt_cache;
pub mod rules;
pub mod scheduler;
pub mod transformer;
pub mod types;
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

// Re-export commonly used types
pub use types::*;

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
pub use decision::nola_core_decide_actions;
pub use lifecycle_hooks::{
    nola_core_lifecycle_hook_abi_version, nola_core_lifecycle_hook_build_registry,
    nola_core_lifecycle_hook_plan, nola_core_lifecycle_hook_project_execution,
};
pub use memory::{nola_core_memory_select_json, nola_core_memory_selection_abi_version};
pub use parser::{
    nola_core_generate_dedupe_key, nola_core_parse_url_json, nola_core_process_raw_events,
};
pub use preprocessor::nola_core_preprocess_raw_events;
pub use prompt_cache::{nola_core_prompt_cache_abi_version, nola_core_prompt_cache_diagnose};
pub use rules::{
    nola_core_rules_clear_history, nola_core_rules_filter, nola_core_rules_init,
    nola_core_rules_is_quiet_hours, nola_core_rules_mark_sent,
};
pub use scheduler::nola_core_scheduler_schedule;
pub use tool_policy::{nola_core_tool_policy_abi_version, nola_core_tool_policy_resolve};
pub use transformer::nola_core_transform_model_output;
pub use utils::{nola_core_free_c_string, nola_core_transform_model_output_passthrough};

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn test_generate_dedupe_key() {
        let c_source = CString::new("url").unwrap();
        let c_title = CString::new("Pay bill").unwrap();
        let c_date = CString::new("2025-12-05").unwrap();
        let c_id = CString::new("1").unwrap();
        let out_ptr = nola_core_generate_dedupe_key(
            c_source.as_ptr(),
            c_title.as_ptr(),
            c_date.as_ptr(),
            c_id.as_ptr(),
        );
        assert!(!out_ptr.is_null());
        let s = unsafe {
            std::ffi::CStr::from_ptr(out_ptr)
                .to_string_lossy()
                .into_owned()
        };
        assert!(s.starts_with("url:"));
        nola_core_free_c_string(out_ptr);
    }

    #[test]
    fn test_parse_url_json() {
        let json = r#"{"events": [{"id":"1","type":"x","title":"Pay bill","date":"2025-12-10","description":"desc","priority":0.8}] }"#;
        let ptr = json.as_ptr();
        let out_ptr = nola_core_parse_url_json(ptr, json.len());
        assert!(!out_ptr.is_null());
        let s = unsafe {
            std::ffi::CStr::from_ptr(out_ptr)
                .to_string_lossy()
                .into_owned()
        };
        assert!(s.contains("Pay bill"));
        nola_core_free_c_string(out_ptr);
    }
}

pub use nola_kernel::compaction_maintenance;

pub use nola_kernel::compaction_summary;
pub use nola_kernel::strict_json;
