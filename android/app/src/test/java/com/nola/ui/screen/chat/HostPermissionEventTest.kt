package com.nola.ui.screen.chat

import com.nola.data.remote.sse.SseEvent
import com.nola.data.remote.sse.decodeSsePayload
import com.nola.data.remote.mobile_crypto.MobileCrypto
import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

class HostPermissionEventTest {
    private fun event(): SseEvent.PermissionRequest = decodeSsePayload("""{"type":"permission_request", "allow_conversation": true,"id":"p2","tool_name":"write_file","args":{"path":"original"},"protocol_version": 4,"nonce":"nonce2","host_id":"host","client_id":"client","session_id":"session","expires_at":200,"permission_instructions":[{"permission_request_id":"p1","instruction":"Keep format"}]}""", null) as SseEvent.PermissionRequest

    @Test fun currentV4SseEntersProductionUiTransitionAndSendsTypedCustom() {
        val state = acceptHostPermissionEvent(ChatUiState(conversationId = "session"), event(), "session", "host", "client", 100)!!
        val request = state.permissionRequest!!
        assertEquals("p2", request.id)
        assertEquals(4, request.protocolVersion)
        assertEquals("p1", request.permissionInstructions.single().permissionRequestId)
        val wire = MobileCrypto.decryptJsonObject(MobileCrypto.encryptJson(hostPermissionInstructionPayload(request, "Keep encoding"), "fixture-token"), "fixture-token")
        assertEquals(4, wire["protocol_version"]!!.jsonPrimitive.int)
        assertEquals("p2", wire["request_id"]!!.jsonPrimitive.content)
        assertEquals("nonce2", wire["nonce"]!!.jsonPrimitive.content)
        val resolution = wire["resolution"]!!.jsonObject
        assertEquals("intervention", resolution["type"]!!.jsonPrimitive.content)
        assertEquals("provide_instruction", resolution["intervention"]!!.jsonObject["type"]!!.jsonPrimitive.content)
        assertFalse(resolution.containsKey("decision"))
        assertEquals("original", request.args["path"])
    }

    @Test fun wrongIdentityExpiredOrOldProtocolCannotClearSuccessorUi() {
        val successor = acceptHostPermissionEvent(ChatUiState(conversationId = "session"), event(), "session", "host", "client", 100)!!
        val invalid = listOf(event().copy(id = "p1", permissionInstructions = emptyList()), event().copy(hostId = "other"), event().copy(clientId = "other"), event().copy(sessionId = "other"), event().copy(expiresAt = 99), event().copy(protocolVersion = 1), event().copy(protocolVersion = 2), event().copy(protocolVersion = 3), event().copy(protocolVersion = 5), event().copy(nonce = ""))
        invalid.forEach { old ->
            // Explicit rejection is required; identical-request deduplication
            // must not let a legal version pass a supposed negative case.
            assertNull(acceptHostPermissionEvent(successor, old, "session", "host", "client", 100))
            assertEquals("p2", successor.permissionRequest!!.id)
        }
        listOf(3, 5).forEach { version ->
            assertNull(acceptHostPermissionEvent(
                successor, event().copy(id = "different-v$version", protocolVersion = version),
                "session", "host", "client", 100,
            ))
        }
        assertNull(acceptHostPermissionEvent(successor.copy(conversationId = "switched"), event(), "session", "host", "client", 100))
    }

    @Test fun replayedIdenticalRequestPreservesTheExistingUiState() {
        val current = acceptHostPermissionEvent(
            ChatUiState(conversationId = "session"), event(), "session", "host", "client", 100,
        )!!

        val replayed = acceptHostPermissionEvent(
            current, event(), "session", "host", "client", 100,
        )

        assertSame(current, replayed)
    }
}
