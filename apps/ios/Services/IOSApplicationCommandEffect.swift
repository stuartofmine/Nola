// SPDX-License-Identifier: Apache-2.0
// Conservative current-protocol observation subset of ApplicationCommand::is_read_only_query.
// Unknown commands retain mutation ordering. Names/prompt text never imply authority.
import Foundation

extension IOSApplicationRuntime {
    nonisolated static func isReadOnlyCommand(type: String, arguments: [String: Any]) -> Bool {
        switch type {
        case "agent_room":
            let kind = (arguments["command"] as? [String: Any])?["type"] as? String
            return ["directory", "read", "list", "for_conversation", "submission", "submissions", "execution_intent"].contains(kind ?? "")
        case "auxiliary_invocation":
            let kind = (arguments["command"] as? [String: Any])?["type"] as? String
            return kind == "inspect" || kind == "inspect_source"
        case "read_ui_authority", "read_ui_capability", "read_ui_surface", "read_ui_surface_revision", "read_ui_surface_changes", "list_ui_surfaces", "command_receipt", "load_run_identity", "load_conversation", "list_conversations", "load_conversation_settings",
             "load_committed_host_requests", "load_runtime_effect_work_schedule", "load_host_execution_state",
             "load_session_projection", "load_goal_control_observation", "validate_mobile_run_recovery_authority",
             "query_claimable_memory_jobs", "build_memory_worker_plan", "read_remote_submission_intent", "read_admitted_remote_run_identity", "read_remote_submission_request",
             "peek_agent_command", "load_task_guidance_receipt",
             "capture_mobile_delegation_configuration", "load_mobile_task_child_configuration", "load_mobile_task_wake_plan",
             "inspect_mobile_permission_request", "inspect_mobile_mcp_request",
             "load_task_child_conversation_window", "load_conversation_projection_v1", "load_conversation_projection_page_v1",
             "load_conversation_source_owner", "load_conversation_projection_window_v1", "load_conversation_projection_window_update_v1",
             "load_conversation_tail_snapshot_v1", "load_conversation_context_usage_v1",
             "load_conversation_permission_projection_v1", "load_conversation_task_projection_v1",
             "load_prepared_edited_resend", "load_today_facts", "list_recoverable_runs", "list_open_user_input_waits", "list_owned_task_user_input_waits",
             "inspect_run_recovery", "validate_run_recovery_authority", "encode_host_worker_identity", "inspect_cancellation_process_owners", "validate_cancellation_process_observations", "inspect_run_process_owners", "validate_run_process_observations", "read_permission_configuration", "inspect_owned_task_recovery", "inspect_unknown_effect_recovery", "load_exact_run_invocation", "load_exact_run_trace_replay",
             "canonical_run_terminal", "active_run", "local_assistant_run_terminal_seal",
             "local_assistant_journal_head", "local_run_assistant_journal_heads", "local_run_context_usage",
             "read_assistant_journal", "read_assistant_journal_range", "read_assistant_journal_parts",
             "read_assistant_journal_part_changes", "read_assistant_journal_manifest", "read_assistant_journal_chunk",
             "read_assistant_journal_record_chunk", "next_assistant_journal_content",
             "local_run_message_admission_head", "read_run_message_admissions",
             "read_local_assistant_run_terminal_manifest", "run_message_admission_reception_state",
             "assistant_journal_reception_state", "get_product_setting", "get_sync_cursor",
             "resolve_model_reasoning", "resolve_remote_model_profile", "query_automation_occurrences", "resolve_chat_model", "admit_host_output",
             "step_compaction_summary", "evaluate_compaction_maintenance",
             "load_auxiliary_source_slot", "load_auxiliary_operation", "list_conversation_auxiliary_sources":
            return true
        case "context":
            let kind = (arguments["command"] as? [String: Any])?["type"] as? String
            return ["count_checkpoints", "prepare_compaction", "prepare_maintenance", "load_checkpoint"].contains(kind ?? "")
        case "conversation_plan":
            let kind = (arguments["command"] as? [String: Any])?["type"] as? String
            return ["get", "history", "diff"].contains(kind ?? "")
        case "model":
            guard let command = arguments["command"] as? [String: Any], let kind = command["type"] as? String else { return false }
            return ["get_profile", "list_profiles", "get_runtime", "get_connection", "list_connections",
                    "get_account", "list_accounts"].contains(kind)
        default: return false
        }
    }
}
