package com.nola.ui.screen.settings

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class SettingsExportPolicyTest {
    @Test
    fun mcpUrlExportRemovesStructuredCredentials() {
        val result = sanitizeMcpUrlForExport(
            "https://user:password@mcp.example/v1?transport=sse&api_key=secret&token=second#private"
        )

        assertEquals(4, result.redactionCount)
        assertFalse(result.url.contains("user"))
        assertFalse(result.url.contains("password"))
        assertFalse(result.url.contains("secret"))
        assertFalse(result.url.contains("second"))
        assertFalse(result.url.contains("private"))
        assertTrue(result.url.contains("transport=sse"))
        assertTrue(result.url.contains("api_key=REDACTED"))
        assertTrue(result.url.contains("token=REDACTED"))
    }

    @Test
    fun mcpUrlExportPreservesOrdinaryEndpoint() {
        val raw = "https://mcp.example/v1?transport=sse"
        val result = sanitizeMcpUrlForExport(raw)

        assertEquals(0, result.redactionCount)
        assertEquals(raw, result.url)
    }
}
