package com.nola.data.local

import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

class ApplicationCommandScopeTest {
    @Test fun queuedTurnAdmissionsRemainOrderedWithinTheirConversation() {
        for (type in listOf("begin_queued_user_turn", "promote_queued_assistant_turn")) {
            val command = buildJsonObject {
                put("type", type)
                put("input", buildJsonObject { put("conversation_id", "queued-owner") })
            }
            val scope = ApplicationCommandScope.command(command)
            assertEquals(ApplicationCommandScope.Conversation("queued-owner"), scope)
            assertTrue(scope.conflicts(ApplicationCommandScope.Conversation("queued-owner")))
            assertFalse(scope.conflicts(ApplicationCommandScope.Conversation("other-owner")))
        }
    }

    @Test fun remoteConsumerWritesUseCanonicalOwnerAndRejectMisplacedIdentity() {
        val paths = mapOf(
            "accept_remote_run_context_usage" to listOf("record", "identity"),
            "receive_run_message_admissions" to listOf("page", "through", "identity"),
            "receive_assistant_journal_record" to listOf("record", "header", "identity"),
            "stage_assistant_journal_content" to listOf("input", "record", "header", "identity"),
            "begin_receive_assistant_run_terminal" to listOf("seal", "identity"),
            "commit_received_assistant_run_terminal" to listOf("seal", "identity"),
            "stage_assistant_run_terminal_manifest" to listOf("page", "seal", "identity"),
        )
        for ((type, path) in paths) {
            var body = buildJsonObject { put("conversation_id", "b") }
            for (field in path.reversed()) body = buildJsonObject { put(field, body) }
            val command = JsonObject(body + ("type" to JsonPrimitive(type)))
            val scope = ApplicationCommandScope.command(command)
            assertEquals(ApplicationCommandScope.Conversation("b"), scope)
            assertFalse(scope.conflicts(ApplicationCommandScope.Conversation("a")))
            assertTrue(scope.conflicts(ApplicationCommandScope.Conversation("b")))
            try {
                ApplicationCommandScope.command(buildJsonObject {
                    put("type", type); put("conversation_id", "b")
                })
                fail("Misplaced remote identity accepted: $type")
            } catch (_: IllegalArgumentException) {}
        }
    }
    @Test fun journalSnapshotWritesKeepExactContextAndBodyReadsAreObservations() {
        val at = buildJsonObject { put("identity", buildJsonObject { put("conversation_id", "b") }) }
        val open = buildJsonObject { put("type", "open_assistant_journal_snapshot"); put("at", at) }
        val close = buildJsonObject {
            put("type", "release_assistant_journal_snapshot")
            put("snapshot", buildJsonObject { put("at", at) })
        }
        for (command in listOf(open, close)) {
            assertEquals(ApplicationCommandScope.JournalSnapshot("b"), ApplicationCommandScope.command(command))
            assertFalse(ApplicationCommandScope.command(command).conflicts(ApplicationCommandScope.Conversation("b")))
            assertTrue(ApplicationCommandScope.command(command).conflicts(ApplicationCommandScope.JournalSnapshot("b")))
            assertFalse(ApplicationCommandEffect.isObservation(command))
        }
        for (type in listOf("read_assistant_journal_parts", "read_assistant_journal_part_changes", "read_assistant_journal_range")) {
            assertTrue(ApplicationCommandEffect.isObservation(buildJsonObject { put("type", type) }))
        }
    }
    @Test fun ownershipUsesExactSchemaNotAnArbitraryNestedConversationField() {
        val command = buildJsonObject {
            put("type", "begin_assistant_turn")
            put("conversation_id", "wrong-top-level")
            put("input", buildJsonObject { put("conversation_id", "a") })
        }
        assertEquals(ApplicationCommandScope.Conversation("a"), ApplicationCommandScope.command(command))
        assertEquals(ApplicationCommandScope.Global, ApplicationCommandScope.command(JsonObject(command +
            ("type" to JsonPrimitive("unknown_command")))))
    }

    @Test fun knownCommandWithMissingOrMalformedOwnerIsRejected() {
        for (owner in listOf(JsonNull, JsonPrimitive(12), JsonPrimitive(" a "), JsonPrimitive("a\nb"))) {
            val command = buildJsonObject {
                put("type", "begin_assistant_turn")
                put("input", buildJsonObject { put("conversation_id", owner) })
            }
            try { ApplicationCommandScope.command(command); fail("Invalid owner accepted") }
            catch (_: IllegalArgumentException) {}
        }
    }

    @Test fun runOwnerLookupFailureCannotBecomeAnIndependentLane() {
        val command = buildJsonObject {
            put("type", "heartbeat_effect_delivery")
            put("input", buildJsonObject { put("run_id", "run-a") })
        }
        try { ApplicationCommandScope.command(command) { error("identity unavailable") }; fail("Must retain identity failure") }
        catch (failure: IllegalStateException) { assertEquals("identity unavailable", failure.message) }
    }
}
