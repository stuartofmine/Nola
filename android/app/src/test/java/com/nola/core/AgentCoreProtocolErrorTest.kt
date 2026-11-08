package com.nola.core

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Test

class AgentCoreProtocolErrorTest {
    private val json = Json { ignoreUnknownKeys = true }

    @Test
    fun preservesStableCoreRejectionMetadata() {
        val envelope = objectValue(
            """
            {
              "ok": false,
              "error": "rejected Host response",
              "error_code": "stale_lease",
              "error_category": "stale_authority"
            }
            """.trimIndent(),
        )

        val error = assertThrows(AgentCoreProtocolException::class.java) {
            envelope.agentCoreTransitionOrThrow()
        }
        assertEquals("stale_lease", error.code)
        assertEquals("stale_authority", error.category)
        assertEquals("rejected Host response", error.message)
    }

    @Test
    fun acceptsTransitionWithoutRewritingPayload() {
        val envelope = objectValue(
            """
            {"ok": true, "transition": {"event_envelopes": [], "host_requests": []}}
            """.trimIndent(),
        )

        val transition = envelope.agentCoreTransitionOrThrow()
        assertEquals(0, (transition["host_requests"] as JsonArray).size)
    }

    private fun objectValue(value: String): JsonObject =
        json.parseToJsonElement(value) as JsonObject
}
