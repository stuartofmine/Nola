package com.nola.data.local

import kotlinx.serialization.json.*

/** Serialization ownership only; this does not authorize or route a command. */
internal sealed interface ApplicationCommandScope {
    data object Global : ApplicationCommandScope
    data class Conversation(val id: String) : ApplicationCommandScope
    data class Owners(val ids: Set<String>) : ApplicationCommandScope
    // Snapshot pins protect immutable journal reads; they cannot execute a Run.
    data class JournalSnapshot(val conversationId: String) : ApplicationCommandScope

    fun conflicts(other: ApplicationCommandScope): Boolean = when {
        this == Global || other == Global -> true
        this is Owners && other is Owners -> ids.any { it in other.ids }
        this is Owners && other is Conversation -> other.id in ids
        this is Conversation && other is Owners -> id in other.ids
        else -> this == other
    }

    companion object {
        // Exact command schemas. Unknown or incomplete schemas remain globally ordered.
        private val conversationField = setOf(
            "delete_conversation", "rename_conversation", "apply_active_permission_policy", "apply_pending_task_notice",
            "recover_expired_run_deliveries", "recover_observed_orphaned_run", "step_goal",
        )
        private val inputConversation = setOf(
            "begin_assistant_turn", "begin_queued_user_turn", "promote_queued_assistant_turn", "begin_automation_assistant_turn", "reduce_and_accept_host_response",
            "reduce_and_commit_active_submission", "patch_conversation_configuration",
            "prepare_remote_submission", "resolve_unknown_effect_recovery", "request_run_cancellation",
            "prepare_run_effect_recovery",
            "reduce_and_commit_idle_submission", "respond_to_conversation_wait", "patch_conversation_metadata", "revert_conversation",
        )
        private val sessionField = setOf(
            "claim_next_agent_command", "acknowledge_agent_command", "release_agent_command_lease",
            "release_session_run_lease",
        )
        private val runInput = setOf("claim_committed_host_requests", "heartbeat_effect_delivery")

        fun command(command: JsonObject, resolveRun: (String) -> String? = { null }): ApplicationCommandScope {
            val type = command.text("type") ?: return Global
            val input = command["input"] as? JsonObject
            val id = when (type) {
                "submit_ui_interaction" -> command.ownerAt("input", "interaction", "identity", "owner")
                "read_ui_surface", "read_ui_surface_revision", "read_ui_surface_changes" -> command.ownerAt("identity", "owner")
                "list_ui_surfaces" -> command.text("conversation_id")
                in conversationField -> command.text("conversation_id")
                in inputConversation -> input?.text("conversation_id")
                in sessionField -> command.text("session_id")
                in runInput -> input?.text("run_id")?.let(resolveRun)
                "enqueue_agent_command", "acquire_session_run_lease" -> input?.text("session_id")
                "claim_cancellation_host_requests" -> (input?.get("identity") as? JsonObject)?.text("conversation_id")
                "create_conversation" -> input?.text("id")
                "open_assistant_journal_snapshot" ->
                    ((command["at"] as? JsonObject)?.get("identity") as? JsonObject)?.text("conversation_id")
                "release_assistant_journal_snapshot" ->
                    (((command["snapshot"] as? JsonObject)?.get("at") as? JsonObject)?.get("identity") as? JsonObject)?.text("conversation_id")
                "bind_remote_submission_request", "abandon_prepared_remote_submission", "cancel_source_submission",
                "register_source_submission", "reject_source_submission" ->
                    (command["identity"] as? JsonObject)?.text("conversation_id")
                "accept_remote_submission_receipt", "adopt_observed_remote_run" ->
                    ((command["receipt"] as? JsonObject)?.get("identity") as? JsonObject)?.text("conversation_id")
                "accept_remote_run_context_usage" -> command.ownerAt("record", "identity")
                "receive_run_message_admissions" -> command.ownerAt("page", "through", "identity")
                "receive_assistant_journal_record" -> command.ownerAt("record", "header", "identity")
                "stage_assistant_journal_content" -> command.ownerAt("input", "record", "header", "identity")
                "begin_receive_assistant_run_terminal", "commit_received_assistant_run_terminal" ->
                    command.ownerAt("seal", "identity")
                "stage_assistant_run_terminal_manifest" -> command.ownerAt("page", "seal", "identity")
                else -> return Global
            }
            requireNotNull(id) { "Command $type has no validated canonical conversation owner" }
            return if (type in setOf("open_assistant_journal_snapshot", "release_assistant_journal_snapshot"))
                JournalSnapshot(id) else Conversation(id)
        }

        fun envelope(envelope: JsonObject, resolveRun: (String) -> String? = { null }): ApplicationCommandScope {
            val wire = envelope["command"] as? JsonObject ?: return Global
            val args = wire["arguments"] as? JsonObject ?: return Global
            return command(JsonObject(args + ("type" to (wire["type"] ?: JsonNull))), resolveRun)
        }

        private fun JsonObject.ownerAt(vararg path: String): String? {
            var node: JsonObject = this
            for (field in path) node = node[field] as? JsonObject ?: return null
            return node.text("conversation_id")
        }

        private fun JsonObject.text(key: String): String? = (get(key) as? JsonPrimitive)
            ?.takeIf { it.isString }?.content?.takeIf {
                it.isNotBlank() && it.length <= 512 && it.trim() == it && it.none(Char::isISOControl)
            }
    }
}
