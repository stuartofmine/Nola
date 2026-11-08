package com.nola.ui.screen.chat

import com.nola.core.AndroidAgentPermissionMode
import com.nola.domain.model.ConversationGenerationOverride
import com.nola.domain.model.ModelParameters
import com.nola.domain.projection.ConversationProjectionV1
import org.junit.Assert.*
import org.junit.Test

class ConversationPresentationTest {
    @Test fun newConversationDropsAllOldUsageAndConversationOverrides() {
        val original = ChatUiState(conversationId = "A", sessionMemory = "A memory",
            taskChildOwner = ConversationProjectionV1.TaskChildOwner("parent", "parent-run", "task"),
            knowledgePoints = "A knowledge", conversationWiki = "A wiki", responseStyleOverride = "complex",
            selectedDataSourceIds = listOf("A-source"), currentExecutionBackend = "external",
            generationOverride = ConversationGenerationOverride(maxContextLength = 12345, maxOutputTokens = 2345),
            currentResponse = "A output", thinking = "A thinking", activeTurnId = "A-turn",
            currentParameters = ModelParameters(), longTermMemory = "app memory",
            permissionConfigurationOwner = com.nola.data.local.PermissionConfigurationOwner.Remote(
                com.nola.data.remote.api.RemoteSubmissionIdentity("host", "client", "A", "run-A", "user-A", "assistant-A")),
            activeResponseId = "A-response", revealResponseId = "A-reveal", error = "A error")
            .applyCanonicalContextUsage(ConversationProjectionV1.ContextUsage(7500, 32768, 28672, false, 12))
        val empty = original.newConversationPresentation(AndroidAgentPermissionMode.Cautious)
        assertNull(empty.conversationId)
        assertNull(empty.taskChildOwner)
        assertNull(empty.permissionConfigurationOwner)
        assertEquals(0, empty.contextTokens)
        assertEquals(0f, empty.contextUsage)
        assertNull(empty.contextUsageDetail)
        assertEquals(original.currentParameters.maxContextTokens, empty.contextMaxTokens)
        assertNull(empty.responseStyleOverride)
        assertEquals(ConversationGenerationOverride(), empty.generationOverride)
        assertEquals("", empty.sessionMemory)
        assertEquals("", empty.knowledgePoints)
        assertEquals("", empty.conversationWiki)
        assertTrue(empty.selectedDataSourceIds.isEmpty())
        assertEquals("nola", empty.currentExecutionBackend)
        assertNull(empty.activeTurnId)
        assertNull(empty.activeResponseId)
        assertNull(empty.revealResponseId)
        assertNull(empty.error)
        assertEquals("", empty.currentResponse)
        assertEquals("", empty.thinking)
        assertEquals("app memory", empty.longTermMemory)
        assertEquals(original.currentParameters, empty.currentParameters)
    }
}
