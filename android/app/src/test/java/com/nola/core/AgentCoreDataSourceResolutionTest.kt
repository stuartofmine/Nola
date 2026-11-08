package com.nola.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AgentCoreDataSourceResolutionTest {
    @Test
    fun contractUsesCoreCamelCaseInputAndSnakeCaseReason() {
        val encoded = AgentCoreDataSourceResolver.encodeInput(
            AgentCoreDataSourceResolutionInput(
                sourceId = "calendar",
                policy = "askWhenRelevant",
                conversationOverride = null,
                now = "2026-08-05T00:00:00Z",
            )
        ).decodeToString()
        assertTrue(encoded.contains("\"schemaVersion\":1"))
        assertTrue(encoded.contains("\"sourceId\":\"calendar\""))
        assertTrue(encoded.contains("\"policy\":\"askWhenRelevant\""))

        val decoded = AgentCoreDataSourceResolver.decodeOutput(
            """{"schemaVersion":1,"enabled":false,"usable":false,"shouldRequestConsent":true,"shouldRefresh":false,"reason":"consent_required"}""".encodeToByteArray()
        )
        assertEquals(1, decoded.schemaVersion)
        assertFalse(decoded.enabled)
        assertFalse(decoded.usable)
        assertTrue(decoded.shouldRequestConsent)
        assertEquals("consent_required", decoded.reason)
    }
}
