package com.nola.ui.screen.settings

import com.nola.domain.model.ChatMemoryProfile
import com.nola.domain.model.McpServerConfig
import com.nola.domain.model.UploadedContent
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test

class SettingsExportPolicyTest {
    @Test
    fun userTextSurvivesWithoutConnectionCredentialsOrAccountDetails() {
        val privateMarker = "test-credential-never-export"
        val profile = ChatMemoryProfile(
            longTermMemory = listOf("Choose lightweight models"),
            mcpServers = listOf(McpServerConfig(name = "private", url = "https://example.org/$privateMarker")),
            uploadedDocuments = listOf(UploadedContent(name = "Plan", content = "Reviewed", accessOwnerId = privateMarker)),
        )
        val exported = exportableProfile(profile)
        assertEquals(profile.longTermMemory, exported.longTermMemory)
        assertEquals("Reviewed", exported.uploadedDocuments.single().content)
        assertFalse(Json.encodeToString(exported).contains(privateMarker))
    }
}
