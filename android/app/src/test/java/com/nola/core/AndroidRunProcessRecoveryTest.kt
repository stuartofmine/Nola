package com.nola.core

import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

class AndroidRunProcessRecoveryTest {
    private val descriptor = AndroidCanonicalTurnDescriptor("conversation", "Title", "run", "user", "Hi", null,
        assistantMessageId = "assistant")
    private val frozen = Json.parseToJsonElement("""{"conversation_id":"conversation","run_id":"run","assistant_message_id":"assistant","invocation_config_digest":"digest","host_invocation":{"host_id":"original"},"core_session_id":"core","context_budget":{"capacity":1000},"remote_tool_plan":null,"status":"running","checkpoint_revision":3}""").jsonObject
    private val owner = Json.parseToJsonElement("""{"pid":1234,"boot_id":"boot","start_id":"123","nonce":"worker"}""").jsonObject
    private val deliveries = Json.parseToJsonElement("""[{"attempt_id":"attempt","worker_generation":"original-generation","lease_owner":"original-owner","lease_token":"original-token","lease_expires_at_ms":241000}]""").jsonArray
    private fun plan(session: Boolean = false) = buildJsonObject {
        for (key in listOf("conversation_id", "run_id", "assistant_message_id", "invocation_config_digest", "host_invocation")) put(key, frozen.getValue(key))
        put("has_session_lease", session); put("process_owners", JsonArray(listOf(owner)))
        put("deliveries", deliveries); put("has_unobservable_owner", false)
    }

    @Test fun hostObservationForwardsExactCorePlanAndOriginalLeaseBeforeExpiry() {
        val commands = mutableListOf<JsonObject>()
        val original = plan()
        val observed = buildJsonObject { put("state", "gone"); put("pid", 1234); put("boot_id", "boot") }
        val recovered = reconcileAndroidObservedRun(descriptor, frozen, { command ->
            commands += command
            when (command.getValue("type").jsonPrimitive.content) {
                "inspect_run_process_owners" -> original
                "validate_run_process_observations" -> {
                    assertEquals(original, command["plan"])
                    assertEquals(JsonArray(listOf(buildJsonObject { put("owner", owner); put("observation", observed) })), command["observations"])
                    JsonPrimitive(true)
                }
                "recover_observed_orphaned_run" -> {
                    assertEquals(deliveries, command["observed_deliveries"])
                    assertEquals(JsonNull, command["observed_session_lease"])
                    assertEquals(frozen["host_invocation"], command["host_invocation"])
                    assertEquals(JsonPrimitive(2000L), command["interrupted_at_ms"])
                    JsonPrimitive(1)
                }
                else -> error("Unexpected mutation")
            }
        }, { pid -> assertEquals(1234, pid); observed }, { 2000L })
        assertTrue(recovered)
        assertEquals(3, commands.size)
    }

    @Test fun noDeathProofOrIndependentSessionOwnerDoesNotMutateOrChangeDeadline() {
        for (session in listOf(false, true)) {
            val commands = mutableListOf<String>()
            assertFalse(reconcileAndroidObservedRun(descriptor, frozen, { command ->
                val type = command.getValue("type").jsonPrimitive.content
                commands += type
                when (type) {
                    "inspect_run_process_owners" -> plan(session)
                    "validate_run_process_observations" -> JsonPrimitive(false)
                    else -> error("No death proof must not reconcile")
                }
            }, { buildJsonObject { put("state", "unknown"); put("pid", it) } }, { error("No write clock") }))
            assertEquals(if (session) 1 else 2, commands.size)
        }
    }

    @Test fun authorityChangeIsRejectedAndOnlyExactSealedOwnerReadsTerminalSeal() {
        val sealed = JsonObject(frozen + mapOf("status" to JsonPrimitive("sealed"), "checkpoint_revision" to JsonPrimitive(99)))
        assertTrue(sameAndroidFrozenRunAuthority(frozen, sealed))
        for (key in listOf("run_id", "assistant_message_id", "core_session_id", "invocation_config_digest", "host_invocation", "context_budget", "remote_tool_plan")) {
            val changed = JsonObject(sealed + (key to JsonPrimitive("foreign")))
            assertThrows(IllegalArgumentException::class.java) {
                loadAndroidRecoveryTerminalIfSealed(descriptor, frozen, changed) { error("Must not read another owner") }
            }
        }
        assertNull(loadAndroidRecoveryTerminalIfSealed(descriptor, frozen, frozen) { error("Active Run cannot complete") })
        var reads = 0
        assertThrows(IllegalStateException::class.java) {
            loadAndroidRecoveryTerminalIfSealed(descriptor, frozen, sealed) { command ->
                reads++
                assertEquals(JsonPrimitive("local_assistant_run_terminal_seal"), command["type"])
                assertEquals(JsonPrimitive("run"), command["run_id"])
                JsonNull
            }
        }
        assertEquals(1, reads)
    }
}
