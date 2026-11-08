package com.nola.core

import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class AgentCoreContextContractTest {
    private val json = Json { ignoreUnknownKeys = true }

    @Test
    fun snapshotUsesTokenUsageInsteadOfMessageCount() {
        val snapshot = json.decodeFromString<AgentCoreSnapshot>(
            """
            {
              "platform":"android",
              "execution_mode":"device_local",
              "phase":"running",
              "iteration":7,
              "message_count":320,
              "estimated_context_tokens":98304,
              "context_window_tokens":131072,
              "context_usage_percent":75.0,
              "pending_tool_count":0,
              "awaiting_permission":false,
              "awaiting_host_call":false,
              "usage":{"prompt_tokens":96000,"completion_tokens":2304,"total_tokens":98304}
            }
            """.trimIndent()
        )

        assertEquals(320, snapshot.messageCount)
        assertEquals(98_304, snapshot.estimatedContextTokens)
        assertEquals(131_072, snapshot.contextWindowTokens)
        assertEquals(75.0, snapshot.contextUsagePercent ?: 0.0, 0.001)
        assertNotEquals(snapshot.messageCount, snapshot.estimatedContextTokens)
    }

    @Test
    fun contextPlanInputCarriesLongHistoryBudgetFields() {
        val input = AgentCoreContextPlanInput(
            contextWindowTokens = 262_144,
            tokenThreshold = 157_286,
            promptTokenLimit = 253_952,
            reservedOutputTokens = 8_192,
            estimatedTokens = 180_000,
            messageCount = 480,
            keepRecentMessages = 12,
            adaptiveLongHistory = true
        )

        val encoded = Json { encodeDefaults = true }.encodeToString(input)

        assertTrue(encoded.contains("\"estimated_tokens\":180000"))
        assertTrue(encoded.contains("\"message_count\":480"))
        assertTrue(encoded.contains("\"keep_recent_messages\":12"))
        assertTrue(encoded.contains("\"adaptive_long_history\":true"))
    }
}
