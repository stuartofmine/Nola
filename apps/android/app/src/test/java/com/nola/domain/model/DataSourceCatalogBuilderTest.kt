package com.nola.domain.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class DataSourceCatalogBuilderTest {
    @Test
    fun chatContextCatalogMapsDocumentsAndMcpServers() {
        val sources = DataSourceCatalogBuilder.forChatContext(
            uploadedDocuments = listOf(
                UploadedContent(
                    name = "Project Notes.md",
                    content = "local project notes",
                    kind = "markdown"
                )
            ),
            hostUploadedDocuments = listOf(
                UploadedContent(
                    name = "Host Runbook",
                    content = "host document",
                    kind = "document"
                )
            ),
            mcpServers = listOf(
                McpServerConfig(
                    name = "filesystem",
                    url = "http://127.0.0.1:9000/mcp"
                )
            ),
            hostMcpServers = emptyList(),
            includeHealthSummary = false
        )

        assertEquals(3, sources.size)

        val localDocument = sources[0]
        assertEquals(UnifiedDataSourceKind.UploadedDocument, localDocument.kind)
        assertTrue(localDocument.capabilities.contains(UnifiedDataSourceCapability.Readable))
        assertTrue(localDocument.capabilities.contains(UnifiedDataSourceCapability.Searchable))
        assertTrue(localDocument.capabilities.contains(UnifiedDataSourceCapability.Contextual))
        assertTrue(localDocument.privacy.scopes.contains(UnifiedDataSourcePrivacyScope.LocalStorage))
        assertTrue(localDocument.privacy.scopes.contains(UnifiedDataSourcePrivacyScope.AllowCloudContext))
        assertFalse(localDocument.privacy.rawSamplesLeaveDevice)
        assertEquals("local", localDocument.location)
        assertEquals("0", localDocument.metadata["originIndex"])

        val mcp = sources[2]
        assertEquals(UnifiedDataSourceKind.Mcp, mcp.kind)
        assertTrue(mcp.capabilities.contains(UnifiedDataSourceCapability.Executable))
        assertTrue(mcp.privacy.scopes.contains(UnifiedDataSourcePrivacyScope.AllowAgentTools))
        assertTrue(mcp.privacy.scopes.contains(UnifiedDataSourcePrivacyScope.RequiresConfirmation))
        assertEquals("0", mcp.metadata["originIndex"])
    }

    @Test
    fun disabledSourcesRemainInCatalogWithDisabledState() {
        val sources = DataSourceCatalogBuilder.forChatContext(
            uploadedDocuments = listOf(
                UploadedContent(
                    name = "Disabled",
                    content = "not active",
                    enabled = false
                )
            ),
            hostUploadedDocuments = emptyList(),
            mcpServers = emptyList(),
            hostMcpServers = emptyList(),
            includeHealthSummary = false
        )

        assertEquals(1, sources.size)
        assertEquals(UnifiedDataSourceConnectionState.Disabled, sources.first().connectionState)
    }

    @Test
    fun healthSummaryIsLocalSummaryOnlyByDefault() {
        val source = DataSourceCatalogBuilder.healthSummary(connected = true, lastSyncedAt = 123L)

        assertEquals("health:android", source.id)
        assertEquals(
            DataSourceAccessPolicy.ConversationOptIn.name,
            source.metadata["accessPolicy"]
        )
        assertEquals(UnifiedDataSourceKind.Health, source.kind)
        assertEquals(UnifiedDataSourceConnectionState.Ready, source.connectionState)
        assertEquals(123L, source.lastSyncedAt)
        assertTrue(source.privacy.scopes.contains(UnifiedDataSourcePrivacyScope.LocalStorage))
        assertTrue(source.privacy.scopes.contains(UnifiedDataSourcePrivacyScope.SummaryOnly))
        assertFalse(source.privacy.rawSamplesLeaveDevice)
        assertFalse(isHealthDataSourceSelected(emptyList()))
        assertTrue(isHealthDataSourceSelected(listOf(ANDROID_HEALTH_DATA_SOURCE_ID)))
    }

    @Test
    fun chatContextCatalogIncludesHealthSummarySourceByDefault() {
        val sources = DataSourceCatalogBuilder.forChatContext(
            uploadedDocuments = emptyList(),
            hostUploadedDocuments = emptyList(),
            mcpServers = emptyList(),
            hostMcpServers = emptyList()
        )

        assertEquals(1, sources.size)
        assertEquals(UnifiedDataSourceKind.Health, sources.first().kind)
        assertEquals(UnifiedDataSourceConnectionState.NeedsPermission, sources.first().connectionState)
    }
}
