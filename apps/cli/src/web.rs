// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Web Host facade grouped into routes, native bridge, and streaming state.

include!("web_parts/routes.rs");
include!("web_parts/native_bridge.rs");
include!("web_parts/host_tool_plan.rs");
include!("web_parts/auxiliary_invocation.rs");
include!("web_parts/assistant_journal.rs");
include!("web_parts/conversation_progress.rs");
include!("web_parts/paired_execution_selection.rs");
include!("web_parts/streaming_state.rs");
include!("web_parts/public_error.rs");

include!("web_parts/permission_resume.rs");
include!("web_parts/run_observation.rs");
include!("web_parts/delegated_permissions.rs");
include!("web_parts/journal_notifications.rs");
include!("web_parts/user_input_wait.rs");
include!("web_parts/unknown_effect_recovery.rs");
include!("web_parts/submission_receipt.rs");
include!("web_parts/model_profiles.rs");
include!("web_parts/conversation_configuration.rs");
include!("web_parts/edited_resend.rs");
include!("web_parts/paired_permission_configuration.rs");
include!("web_parts/submission_frontier.rs");
include!("web_parts/paired_context_compaction.rs");
include!("web_parts/conversation_resource.rs");
include!("web_parts/generative_ui.rs");
include!("web_parts/conversation_browser.rs");
include!("web_parts/assistant_defaults.rs");
include!("web_parts/agent_rooms.rs");
include!("web_parts/expert_tasks.rs");
include!("web_parts/model_connections.rs");

include!("web_parts/goal_worker.rs");

include!("web_parts/tailcat_sessions.rs");

include!("web_parts/software_update.rs");
