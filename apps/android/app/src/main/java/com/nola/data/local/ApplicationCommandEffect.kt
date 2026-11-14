package com.nola.data.local

import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive

/** Conservative observation subset from ApplicationCommand::is_read_only_query.
 * Unknown commands retain the durable write path. This does not grant authority.
 */
internal object ApplicationCommandEffect {
    private val observations = setOf(
        "list_activity", "load_activity_detail", "read_expert_exchange_capability",
        "preview_ui_surface", "read_ui_authority", "read_ui_capability", "read_ui_surface", "read_ui_surface_revision", "read_ui_surface_changes", "list_ui_surfaces", "peek_agent_command",
        "inspect_owned_task_recovery", "inspect_unknown_effect_recovery", "inspect_run_recovery", "read_permission_configuration", "load_exact_run_invocation", "validate_run_recovery_authority", "validate_mobile_run_recovery_authority", "encode_host_worker_identity",
        "inspect_cancellation_process_owners", "validate_cancellation_process_observations",
        "inspect_run_process_owners", "validate_run_process_observations",
        "load_committed_host_requests", "load_host_execution_state", "active_run", "list_recoverable_runs",
        "load_run_identity", "load_run_delivery_ownership", "load_run_cancellation_delivery_facts", "load_runtime_effect_work_schedule",
        "local_assistant_run_terminal_seal", "load_conversation_task_projection_v1", "read_historical_part",
        "canonical_run_terminal", "read_assistant_journal", "local_assistant_journal_head", "local_run_assistant_journal_heads",
        "local_run_message_admission_head", "local_run_context_usage", "read_run_message_admissions", "run_message_admission_reception_state",
        "read_local_assistant_run_terminal_manifest", "read_assistant_journal_range", "read_assistant_journal_parts",
        "read_assistant_journal_part_changes", "read_assistant_journal_manifest", "read_assistant_journal_chunk",
        "read_assistant_journal_record_chunk", "next_assistant_journal_content", "assistant_journal_reception_state",
        "trace_events_after", "trace_runs_for_conversation", "latest_trace_id", "resolve_model_reasoning", "resolve_remote_model_profile", "query_automation_occurrences", "resolve_chat_model", "admit_host_output",
        "command_receipt", "get_product_setting", "pending_sync_changes", "get_sync_cursor", "load_checkpoint",
        "load_prepared_in_place_resend", "load_prepared_edited_resend", "load_today_facts", "load_conversation", "list_conversations", "list_root_conversations", "load_conversation_settings", "load_session_projection", "load_goal_control_observation",
        "load_task_child_conversation_window", "load_conversation_projection_v1", "load_conversation_projection_page_v1",
        "capture_mobile_delegation_configuration", "load_mobile_task_child_configuration",
        "load_mobile_task_wake_plan", "inspect_mobile_permission_request", "inspect_mobile_mcp_request", "load_conversation_source_owner", "load_conversation_projection_window_v1", "load_conversation_projection_window_update_v1",
        "load_conversation_tail_snapshot", "load_conversation_tail_snapshot_v1", "load_conversation_context_usage_v1",
        "read_remote_submission_intent", "read_admitted_remote_run_identity", "read_remote_submission_request", "list_open_user_input_waits",
        "load_user_input_wait", "load_conversation_permission_projection_v1"
    )
    private val nestedObservations = mapOf(
        "conversation_plan" to setOf("get"),
        "runtime_task" to setOf("get"),
        "auxiliary_invocation" to setOf("inspect", "inspect_source"),
        "agent_room" to setOf("child_history", "child_conversations", "for_conversation", "role_templates", "directory", "read", "list", "submission", "submissions", "execution_intent"),
        "model" to setOf("get_profile", "list_profiles", "get_runtime", "list_connections", "get_connection", "list_accounts", "get_account", "list_gateway_attempts"),
        "skill" to setOf("list", "list_editors", "get_editor", "load_catalog"),
        "mcp" to setOf("list"),
        "memory" to setOf("list", "get", "get_many", "usage_stats", "recall"),
        "data_source" to setOf("get", "list", "resolve_for_conversation", "get_conversation_override", "list_usage"),
        "context" to setOf("count_checkpoints", "load_checkpoint", "prepare_compaction")
    )

    fun isObservation(command: JsonObject): Boolean {
        val type = (command["type"] as? JsonPrimitive)?.takeIf { it.isString }?.content ?: return false
        if (type in observations) return true
        val nested = command["command"] as? JsonObject ?: return false
        val nestedType = (nested["type"] as? JsonPrimitive)?.takeIf { it.isString }?.content ?: return false
        return nestedType in nestedObservations[type].orEmpty()
    }
}
