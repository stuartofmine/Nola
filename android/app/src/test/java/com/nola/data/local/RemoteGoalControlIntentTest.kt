package com.nola.data.local

import com.nola.data.repository.*
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.*
import kotlin.test.*

class RemoteGoalControlIntentTest {
    private fun intent(id: String = "operation") = buildJsonObject {
        put("expected_host_id", "host"); put("conversation_id", "conversation")
        put("control", buildJsonObject { put("operationId", id); put("action", "pause") })
    }
    private fun progress(applied: Boolean = false, rejection: String? = null) = ConversationProgress(
        "conversation", "Objective", "running", emptyList(), emptyList(),
        GoalControlObservation("goal", "opaque", 4uL,
            if (applied) mapOf("operation" to GoalOperationObservation("a".repeat(64), 4uL, rejection)) else emptyMap(), true, false, true))
    private fun pending(host: String = "host") = buildJsonObject {
        put("source_host_id", host); put("conversationId", "conversation"); put("operationId", "operation")
        put("authoritative", false); put("controlPending", true)
    }
    @Test fun productionControlBuilderMatchesHostNullableContract() {
        for (action in listOf("start", "pause", "resume", "cancel", "schedule")) {
            val input = remoteGoalControlInput("operation", action, null, null)
            assertEquals(JsonPrimitive(""), input["objective"])
            assertEquals(JsonArray(emptyList()), input["constraints"])
            assertEquals(JsonNull, input["expectedGoalId"])
            assertEquals(JsonNull, input["expectedControlToken"])
            assertEquals(JsonNull, input["wakeAtEpochMs"])
        }
    }
    @Test fun httpErrorAndWrongConversationOrOperationLeaveIntentPending() = runTest {
        var saved: String? = intent().toString()
        assertFailsWith<IllegalStateException> {
            submitRecoverableGoalControl(null, "host", "conversation", {saved}, {saved=it}, {progress()}) { error("HTTP 409") }
        }
        for (field in listOf("conversationId", "operationId")) {
            assertFailsWith<IllegalArgumentException> {
                submitRecoverableGoalControl(null, "host", "conversation", {saved}, {saved=it}, {progress()}) {
                    JsonObject(pending().toMutableMap().apply { put(field, JsonPrimitive("foreign")) })
                }
            }
            assertEquals(intent().toString(), saved)
        }
    }
    @Test fun lostResponsePreservesExactIntentAndLedgerRecoversWithoutResending() = runTest {
        var saved: String? = null
        assertFailsWith<IllegalStateException> {
            submitRecoverableGoalControl(intent(), "host", "conversation", {saved}, {saved=it}, {progress()}) {
                assertEquals(intent().toString(), saved); error("lost response")
            }
        }
        val result = submitRecoverableGoalControl(null, "host", "conversation", {saved}, {saved=it}, {progress(true)}) {
            error("Must not send an already applied command")
        }
        assertEquals("running", result.status); assertNull(saved)
    }
    @Test fun pendingResendsOriginalOperationAndDoesNotSettleOn202() = runTest {
        var saved: String? = intent().toString()
        submitRecoverableGoalControl(null, "host", "conversation", {saved}, {saved=it}, {progress()}) {
            assertEquals(intent(), it); pending()
        }
        assertEquals(intent().toString(), saved)
    }
    @Test fun differentPayloadAndForeignResponseCannotReplaceIntent() = runTest {
        var saved: String? = intent().toString()
        assertFailsWith<IllegalArgumentException> {
            submitRecoverableGoalControl(intent("new"), "host", "conversation", {saved}, {saved=it}, {progress()}) { error("unexpected") }
        }
        assertFailsWith<IllegalArgumentException> {
            submitRecoverableGoalControl(null, "host", "conversation", {saved}, {saved=it}, {progress()}) { pending("foreign") }
        }
        assertEquals(intent().toString(), saved)
    }
    @Test fun onlyLedgerRejectionSettlesAndPreservesReason() = runTest {
        var saved: String? = intent().toString()
        val failure = assertFailsWith<IllegalStateException> {
            submitRecoverableGoalControl(null, "host", "conversation", {saved}, {saved=it}, {progress(true,"Goal changed")}) { error("unexpected") }
        }
        assertEquals("Goal changed", failure.message); assertNull(saved)
    }
}
