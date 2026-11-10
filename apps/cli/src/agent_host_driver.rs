// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Agent Host driver facade with responsibility-focused implementation units.

#![allow(dead_code)]

include!("agent_host_driver_parts/orchestration.rs");
include!("agent_host_driver_parts/code_mode.rs");
include!("agent_host_driver_parts/lifecycle_execution.rs");
include!("agent_host_driver_parts/mcp_connection.rs");
include!("agent_host_driver_parts/tool_projection.rs");
include!("agent_host_driver_parts/permission_projection.rs");
include!("agent_host_driver_parts/compaction_summary.rs");
include!("agent_host_driver_parts/model_conversion.rs");

#[cfg(test)]
#[path = "agent_host_driver_parts/model_protocol_contract_tests.rs"]
mod model_protocol_contract_tests;

#[cfg(test)]
#[path = "agent_host_driver_parts/local_execution_stop_tests.rs"]
mod local_execution_stop_tests;
