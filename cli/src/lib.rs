// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Shared executable composition. Binary identity is injected by typed entry points.

#[cfg(any(feature = "terminal-ui", test))]
mod agent;
#[cfg(test)]
mod test_model_identity;
mod agent_command_port;
mod agent_host_driver;
mod agent_registry;
mod application_port;
mod application_service_entry;
#[cfg(target_os = "macos")]
mod background_scheduler;
mod builtin_skills;
mod code_intelligence_provider;
mod codebase_memory;
mod config;
mod host_invocation_request;
mod host_tool_permission_evidence;
mod model_request_overrides;
mod native_host_request;
mod native_host_entry;
mod native_host_launch;
mod native_host_commands;
mod session_projection_output;
mod session_process_observation;
mod native_tool_policy;
mod config_paths;
mod configuration_setup;
#[cfg(any(feature = "terminal-ui", test))]
mod cli_config_setup;
mod context_budget;
mod core_agent_runtime;
mod goal_runtime;
mod goal_control_port;
mod data_sources;
mod desktop_host_control;
mod native_host_output;
mod external_executor;
mod external_execution_settings;
mod host_execution_settings;
mod execution_context;
mod execution_signals;
mod runtime_input;
mod runtime_output;
mod runtime_control;
mod runtime_interaction;
mod native_runtime_output;
mod boundary_input_intent;
mod host_output_admission;
mod model_invocation_settings;
mod model_route;
mod model_profile_resolution;
mod delegated_invocation;
mod conversation_invocation;
mod host_memory_admission;
mod assistant_memory_preferences;
mod assistant_retrieval_preferences;
mod conversation_knowledge_context;
mod historical_part;
mod host_prompts;
mod host_credentials;
mod model_connection_host;
mod model_catalog_discovery;
pub mod model_capability_probe;
mod model_import_native_bridge;
mod model_siwc_native_bridge;
mod model_gateway_host;
mod model_gateway_usage;
mod model_client_adapter;
mod model_source_account;
mod model_source_login;
mod model_kimi_login;
mod model_opencode_login;
#[cfg(unix)]
pub mod model_siwc;
#[cfg(unix)]
mod model_siwc_cli;
mod i18n;
#[cfg(any(feature = "terminal-ui", test))]
mod init;
mod instructions;
#[cfg(any(feature = "terminal-ui", test))]
mod interaction;
mod user_input_wait;
#[cfg(any(feature = "terminal-ui", test))]
mod composer;
mod composer_media;
mod llm;
mod room_routing;
mod local_models;
mod mcp_catalog_port;
mod plugin_manager;
pub mod plugin_runtime;
mod mcp_host;
mod mcp_connection_authority;
mod mcp_stdio;
mod memory_admin;
mod memory_worker;
mod messaging_channel_worker;
mod model_output;
#[cfg(any(feature = "terminal-ui", test))]
mod onboarding;
mod permission_grants;
mod permission_request_port;
mod permissions;
mod platform_paths;
mod process_observation;
mod replica_sync;
mod resource_access;
mod runtime_config;
mod runtime_effect_supervisor;
mod runtime_task_port;
mod session;
mod source_journal;
#[cfg(any(feature = "terminal-ui", test))]
mod terminal_journal_presentation;
mod skill_catalog_port;
mod skill_runtime;
mod store;
mod software_update;
mod task_runtime;
#[cfg(any(feature = "terminal-ui", test))]
mod terminal_ui;
#[cfg(any(feature = "terminal-ui", test))]
mod cli_view_state;
#[cfg(any(feature = "terminal-ui", test))]
mod terminal_frames;
#[cfg(any(feature = "terminal-ui", test))]
mod terminal_document;
#[cfg(any(feature = "terminal-ui", test))]
mod terminal_input;
mod tool_policy;
mod process_sandbox;
mod tailcat_transport;
mod tailcat_web_client;
mod tools;
mod web;
mod work;
mod sop_context;
mod work_step_port;
mod unknown_effect_recovery;

#[cfg(any(feature = "terminal-ui", test))]
include!("main_parts/bootstrap.rs");
#[cfg(any(feature = "terminal-ui", test))]
include!("main_parts/interactive.rs");
#[cfg(any(feature = "terminal-ui", test))]
include!("main_parts/room_interactive.rs");
#[cfg(any(feature = "terminal-ui", test))]
include!("main_parts/commands.rs");
#[cfg(any(feature = "terminal-ui", test))]
include!("main_parts/tests.rs");

pub mod entry;
