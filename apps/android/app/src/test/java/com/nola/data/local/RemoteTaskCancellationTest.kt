package com.nola.data.local

import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

class RemoteTaskCancellationTest {
    private val intent = buildJsonObject { put("hostId", "host"); put("conversationId", "parent"); put("parentRunId", "original"); put("taskId", "child") }
    private fun receipt(requested: Boolean, terminal: Boolean = false) = JsonObject(intent + buildJsonObject {
        put("cancelRequested", requested); put("terminal", terminal); put("state", if (terminal) "sealed" else "running"); put("applicationPending", false)
    })
    @Test fun lostResponseReadsSameOwnerWithoutInventingTerminalState() = runBlocking {
        val calls = mutableListOf<Boolean>()
        try {
            requestRemoteTaskCancellation(intent) { read ->
                calls += read
                when (calls.size) { 1 -> receipt(true); 2 -> error("stop evidence missing"); else -> receipt(true) }
            }
            fail("An intent cannot confirm the failed stop")
        } catch (failure: IllegalStateException) {
            assertEquals("stop evidence missing", failure.message)
        }
        assertEquals(listOf(true, false, true), calls)
    }
    @Test fun alreadyAnsweredChildDoesNotSendCancelOrBecomeCancelled() = runBlocking {
        var reads = 0
        val result = requestRemoteTaskCancellation(intent) { read -> assertTrue(read); reads++; receipt(false, true) }
        assertEquals(1, reads)
        assertEquals(JsonPrimitive("sealed"), result["state"])
        assertEquals(JsonPrimitive(false), result["cancelRequested"])
    }
    @Test fun foreignOwnerAndPendingCannotConfirmCancellation() = runBlocking {
        for (bad in listOf(JsonObject(receipt(true) + ("parentRunId" to JsonPrimitive("other"))),
            JsonObject(receipt(true) + ("applicationPending" to JsonPrimitive(true))))) {
            var rejected = false
            try { requestRemoteTaskCancellation(intent) { bad } } catch (_: IllegalArgumentException) { rejected = true }
            assertTrue(rejected)
        }
    }
}
