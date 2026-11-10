package com.nola.data.local

import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

class ApplicationCommandEffectTest {
    private fun command(type: String, nested: String? = null) = buildJsonObject {
        put("type", type)
        nested?.let { put("command", buildJsonObject { put("type", it) }) }
    }

    @Test fun exactRunRecoveryObservationsDoNotEnterTheMutationJournal() {
        for (type in listOf("list_activity", "load_activity_detail", "preview_ui_surface", "read_ui_surface", "read_ui_surface_revision", "read_ui_surface_changes",
            "inspect_owned_task_recovery", "inspect_unknown_effect_recovery", "inspect_run_recovery", "active_run",
            "inspect_run_process_owners", "validate_run_process_observations",
            "load_exact_run_invocation", "load_committed_host_requests", "load_runtime_effect_work_schedule",
            "load_host_execution_state", "load_conversation_task_projection_v1",
            "load_conversation_permission_projection_v1", "local_assistant_run_terminal_seal",
            "load_conversation", "load_conversation_projection_v1", "resolve_remote_model_profile", "query_automation_occurrences", "get_product_setting")) {
            assertTrue(type, ApplicationCommandEffect.isObservation(command(type)))
        }
    }

    @Test fun privateExpertHostOperationsAreNeverRendererQueries() {
        for (type in listOf("load_expert_message_manifest", "load_prepared_expert_resend", "list_readable_expert_templates",
            "load_edited_resend_child_instructions", "native_expert_host_operation", "preview_capture")) {
            assertFalse(type, ApplicationCommandEffect.isObservation(command(type)))
        }
    }

    @Test fun namesAndNestedReadNamesCannotAuthorizeMutations() {
        for (type in listOf("get_future_mutation", "load_unregistered_state", "resolve_owned_task_recovery", "resolve_unknown_effect_recovery",
            "recover_expired_run_deliveries", "recover_observed_orphaned_run", "claim_committed_host_requests", "heartbeat_effect_delivery")) {
            assertFalse(type, ApplicationCommandEffect.isObservation(command(type)))
        }
        assertTrue(ApplicationCommandEffect.isObservation(command("model", "get_profile")))
        assertFalse(ApplicationCommandEffect.isObservation(command("model", "reset_parameters")))
        assertFalse(ApplicationCommandEffect.isObservation(command("unknown", "get_profile")))
        assertFalse(ApplicationCommandEffect.isObservation(command("model")))
    }
}
