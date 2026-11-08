package com.nola.data.remote.dto

import kotlinx.serialization.decodeFromString
import kotlinx.serialization.json.Json
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class AgentSettingsResponseTest {
    private val json = Json { ignoreUnknownKeys = true }

    @Test
    fun decodesComputerUseHostsFromWebHostSettings() {
        val decoded = json.decodeFromString<AgentSettingsResponse>(
            """
            {
              "uploaded_skills": [],
              "native_browser": {
                "token_configured": true,
                "connected": true,
                "hosts": [
                  {
                    "host_id": "chrome-main",
                    "capabilities": ["status", "inspect"],
                    "last_seen_age_sec": 0.1
                  }
                ],
                "tools": ["chrome_native_status", "chrome_native_inspect"],
                "pending_commands": 2,
                "in_flight_commands": 1
              },
              "computer_use": {
                "feature_enabled": true,
                "token_configured": true,
                "connected": true,
                "hosts": [
                  {
                    "host_id": "macbook-main",
                    "capabilities": ["driver:computer_use"],
                    "last_seen_age_sec": 0.2
                  }
                ],
                "tools": ["computer_status", "computer_screenshot_window"]
              }
            }
            """.trimIndent()
        )

        val computerUse = decoded.computerUse!!
        assertTrue(computerUse.featureEnabled)
        assertTrue(computerUse.tokenConfigured)
        assertTrue(computerUse.connected)
        assertEquals("macbook-main", computerUse.hosts!!.first().hostId)
        assertEquals("driver:computer_use", computerUse.hosts!!.first().capabilities!!.first())
        assertEquals("computer_screenshot_window", computerUse.tools!!.last())
        val nativeBrowser = decoded.nativeBrowser!!
        assertTrue(nativeBrowser.tokenConfigured)
        assertTrue(nativeBrowser.connected)
        assertEquals("chrome-main", nativeBrowser.hosts!!.first().hostId)
        assertEquals("inspect", nativeBrowser.hosts!!.first().capabilities!!.last())
        assertEquals("chrome_native_inspect", nativeBrowser.tools!!.last())
        assertEquals(2, nativeBrowser.pendingCommands)
        assertEquals(1, nativeBrowser.inFlightCommands)
    }

    @Test
    fun decodesCanonicalWebHostMcpEndpoint() {
        val decoded = json.decodeFromString<AgentSettingsResponse>(
            """
            {
              "mcp_servers": [
                {
                  "name": "Docs MCP",
                  "endpoint": "https://mcp.example.com/docs",
                  "description": "Documentation tools",
                  "enabled": false
                }
              ]
            }
            """.trimIndent()
        )

        val server = decoded.mcpServers!!.first()
        assertEquals("Docs MCP", server.name)
        assertEquals("https://mcp.example.com/docs", server.resolvedUrl)
        assertEquals("Documentation tools", server.description)
        assertEquals(false, server.enabled)
    }

    @Test
    fun decodesCanonicalUploadedDocumentContent() {
        val decoded = json.decodeFromString<AgentSettingsResponse>(
            """
            {
              "uploaded_documents": [
                {
                  "name": "Design Notes",
                  "content": "Host document content",
                  "kind": "markdown",
                  "enabled": true
                }
              ]
            }
            """.trimIndent()
        )

        val document = decoded.uploadedDocuments!!.first()
        assertEquals("Design Notes", document.name)
        assertEquals("Host document content", document.resolvedContent)
        assertEquals("markdown", document.kind)
        assertTrue(document.enabled)
    }

    @Test
    fun decodesAgentMemoryFromCanonicalArrays() {
        val decoded = json.decodeFromString<AgentSettingsResponse>(
            """
            {
              "long_term_memory": ["alpha", "beta"],
              "user_preferences": ["concise"]
            }
            """.trimIndent()
        )
        assertEquals(listOf("alpha", "beta"), decoded.longTermMemory)
        assertEquals(listOf("concise"), decoded.userPreferences)
    }
}
