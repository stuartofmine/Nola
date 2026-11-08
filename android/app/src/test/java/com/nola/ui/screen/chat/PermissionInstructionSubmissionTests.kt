package com.nola.ui.screen.chat

import com.nola.core.*
import com.nola.data.remote.mobile_crypto.MobileCrypto
import com.nola.domain.model.HostAgentPermissionRequest
import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

class PermissionInstructionSubmissionTests {
    @Test fun localInstructionSerializesTypedInterventionWithoutAuthorizationOrGrant() {
        val decision = AndroidAgentPermissionDecision("p1", AndroidAgentPermissionUserDecision.ProvideInstruction, instruction = " Keep 🙂 formatting ")
        assertFalse(decision.isAllowed)
        assertEquals(decision, decision.withConversationScope(true))
        val wire = Json.parseToJsonElement(corePermissionResolution(decision).toString()).jsonObject
        assertEquals("intervention", wire["type"]!!.jsonPrimitive.content)
        assertEquals("provide_instruction", wire["intervention"]!!.jsonObject["type"]!!.jsonPrimitive.content)
        assertEquals("Keep 🙂 formatting", wire["intervention"]!!.jsonObject["instruction"]!!.jsonPrimitive.content)
        assertFalse(wire.containsKey("decision"))
        assertFalse(wire.containsKey("grant"))
    }

    @Test fun remoteEncryptedTransportPreservesIdentityAndTypedInstruction() {
        val request = HostAgentPermissionRequest(true, "p1", "write_file", args = mapOf("path" to "original"), nonce = "nonce", hostId = "host", clientId = "client", sessionId = "session", protocolVersion = 4)
        val payload = hostPermissionInstructionPayload(request, "Preserve encoding")
        val wire = MobileCrypto.decryptJsonObject(MobileCrypto.encryptJson(payload, "fixture-token"), "fixture-token")
        assertEquals("p1", wire["request_id"]!!.jsonPrimitive.content)
        assertEquals("nonce", wire["nonce"]!!.jsonPrimitive.content)
        assertEquals("session", wire["session_id"]!!.jsonPrimitive.content)
        val resolution = wire["resolution"]!!.jsonObject
        assertEquals("intervention", resolution["type"]!!.jsonPrimitive.content)
        assertEquals("provide_instruction", resolution["intervention"]!!.jsonObject["type"]!!.jsonPrimitive.content)
        assertFalse(resolution.containsKey("decision"))
        assertFalse(wire.containsKey("messages"))
        assertEquals(mapOf("path" to "original"), request.args)
    }

    @Test fun limitsCountUtf8AndRequestIdentitiesBeforeSending() {
        assertEquals("🙂".repeat(1024), normalizedPermissionInstruction("p1", emptyList(), "🙂".repeat(1024)))
        assertThrows(IllegalArgumentException::class.java) { normalizedPermissionInstruction("p1", emptyList(), "🙂".repeat(1025)) }
        assertThrows(IllegalArgumentException::class.java) { normalizedPermissionInstruction("p1", emptyList(), "  ") }
        val full = (0..15).map { AgentPermissionInstruction("p$it", "note") }
        assertThrows(IllegalArgumentException::class.java) { normalizedPermissionInstruction("next", full, "note") }
        val bytes = (0..3).map { AgentPermissionInstruction("id", "a".repeat(4094)) }
        assertThrows(IllegalArgumentException::class.java) { normalizedPermissionInstruction("next", bytes, "x") }
        val request = HostAgentPermissionRequest(true, "next", "tool", permissionInstructions = full)
        assertThrows(IllegalArgumentException::class.java) { hostPermissionInstructionPayload(request, "note") }
    }

    @Test fun predecessorCompletionCannotDismissSuccessorOrSwitchedConversationPermission() {
        assertTrue(permissionResponseStillCurrent("p1", "p1"))
        assertFalse(permissionResponseStillCurrent("p1", "p2"))
        assertFalse(permissionResponseStillCurrent("p1", null))
        assertFalse(permissionResponseStillCurrent("p1", "other-conversation-p1"))
    }
}
