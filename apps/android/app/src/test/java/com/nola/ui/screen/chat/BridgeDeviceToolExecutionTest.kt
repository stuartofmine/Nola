package com.nola.ui.screen.chat

import com.nola.core.AndroidAgentHostCallRequest
import com.nola.core.AndroidAgentHostCallResult
import com.nola.core.AndroidAgentPermissionDecision
import com.nola.core.AndroidAgentPermissionUserDecision
import com.nola.data.remote.sse.SseEvent
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class BridgeDeviceToolExecutionTest {
    private val sampleRequest = SseEvent.DeviceToolRequest(
        id = "call-1",
        nonce = "nonce-1",
        hostId = "host-1",
        clientId = "client-1",
        sessionId = "conv-1",
        toolName = "device_clipboard_read",
        capability = "clipboard",
        arguments = JsonObject(mapOf("format" to JsonPrimitive("text"))),
        args = mapOf("format" to "text"),
        expiresAt = 9_999_999_999L,
        protocolVersion = 1,
    )

    @Test
    fun approvedDecisionExecutesHostCallWithArgumentsAndScopedUriGrant() = runTest {
        var captured: AndroidAgentHostCallRequest? = null
        val decision = AndroidAgentPermissionDecision(
            requestId = "call-1",
            decision = AndroidAgentPermissionUserDecision.AllowOnce,
            scopedUri = "content://com.example/doc/1",
        )

        val result = executeBridgeDeviceCall(sampleRequest, decision) { call ->
            captured = call
            AndroidAgentHostCallResult(
                callId = call.callId,
                output = JsonObject(mapOf("text" to JsonPrimitive("hello"))),
                isError = false,
            )
        }

        assertFalse(result.isError)
        assertEquals("call-1", result.callId)
        assertEquals("hello", result.output.jsonObject["text"]?.jsonPrimitive?.content)
        assertEquals("device_clipboard_read", captured?.toolName)
        assertEquals("clipboard", captured?.capability)
        assertEquals("content://com.example/doc/1", captured?.grant?.get("uri")?.jsonPrimitive?.content)
    }

    @Test
    fun deniedDecisionSkipsExecutionAndReturnsPermissionError() = runTest {
        var invoked = false
        val decision = AndroidAgentPermissionDecision(
            requestId = "call-1",
            decision = AndroidAgentPermissionUserDecision.Deny,
        )

        val result = executeBridgeDeviceCall(sampleRequest, decision) {
            invoked = true
            error("Must not execute when denied")
        }

        assertFalse(invoked)
        assertTrue(result.isError)
        assertEquals("Permission denied by user or system", result.output.jsonObject["error"]?.jsonPrimitive?.content)
    }

    @Test
    fun executorFailureReturnsStructuredErrorResult() = runTest {
        val decision = AndroidAgentPermissionDecision(
            requestId = "call-1",
            decision = AndroidAgentPermissionUserDecision.AllowOnce,
        )

        val result = executeBridgeDeviceCall(sampleRequest, decision) {
            throw IllegalStateException("Clipboard unavailable")
        }

        assertTrue(result.isError)
        assertEquals("Clipboard unavailable", result.output.jsonObject["error"]?.jsonPrimitive?.content)
    }
}
