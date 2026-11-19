// SPDX-License-Identifier: Apache-2.0

package com.nola.agent.context

import com.nola.core.AgentCoreContextComponent
import com.nola.core.AgentCoreContextComponentSource
import com.nola.domain.model.ChatRole
import com.nola.domain.model.Conversation
import com.nola.domain.model.ConversationMessage
import com.nola.domain.model.UploadedContent
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ChatContextPreparerContractTest {
    private val json = Json { ignoreUnknownKeys = true }

    @Test
    fun extractsAttachmentComponentWithTitleAndStripsMarkupFromMessage() {
        val message = ConversationMessage(
            id = "m-1",
            conversationId = "conv-1",
            role = ChatRole.user,
            text = """Please review <attachment name="report.pdf">PDF text body</attachment> carefully.""",
        )

        val payload = extractConversationPayload(listOf(message))

        assertEquals(1, payload.attachments.size)
        val attachment = payload.attachments.single()
        assertEquals("attachment", attachment.kind)
        assertEquals("report.pdf", attachment.source?.title)
        assertEquals("PDF text body", attachment.content.jsonPrimitive.content)
        val renderedMessage = payload.messages.single()["content"]!!.jsonPrimitive.content
        assertFalse(renderedMessage.contains("<attachment"))
        assertTrue(renderedMessage.contains("Please review"))
        assertTrue(renderedMessage.contains("carefully."))
    }

    @Test
    fun conversationKnowledgeAndWikiUseCanonicalDocumentKind() {
        val conversation = Conversation(
            id = "conv-1",
            knowledgePoints = "Release deadline is Friday.",
            wiki = "Project wiki notes",
        )

        val components = assembleChatContextComponents(
            systemPrompt = null,
            builtComponents = emptyList(),
            conversation = conversation,
            knowledgeTitle = "Conversation Knowledge",
            wikiTitle = "Conversation Wiki",
            personalContext = null,
            healthSummary = null,
            healthTitle = "Health Connect",
            uploadedSkills = emptyList(),
            uploadedDocuments = emptyList(),
            attachments = emptyList(),
        )

        assertEquals(2, components.size)
        assertTrue(components.all { it.kind == "document" })
        assertEquals("conversation-knowledge-conv-1", components[0].id)
        assertEquals("Conversation Knowledge", components[0].source?.title)
        assertEquals("conversation-wiki-conv-1", components[1].id)
        assertEquals("Conversation Wiki", components[1].source?.title)
    }

    @Test
    fun uploadedSkillsAndDocumentsDeduplicateAndPopulateMetadata() {
        val builtComponents = listOf(
            AgentCoreContextComponent(
                kind = "skill",
                id = "skill:existing-id",
                content = JsonPrimitive("{}"),
                source = AgentCoreContextComponentSource(platform = "android", title = "Existing By Title"),
            ),
            AgentCoreContextComponent(
                kind = "document",
                id = "doc:1",
                content = JsonPrimitive("Already included"),
                source = AgentCoreContextComponentSource(platform = "android", title = "Shared Handbook"),
            ),
        )
        val uploadedSkills = listOf(
            UploadedContent(
                name = "Brand New Title Same ID",
                content = "Should be deduped by skill id",
                description = "Skip",
                resourceId = "existing-id",
            ),
            UploadedContent(
                name = "Existing By Title",
                content = "Should be deduped by source title",
                description = "Skip",
                resourceId = "different-id",
            ),
            UploadedContent(
                name = "deploy-helper",
                content = "# Heading\nVerify release checksums before upload.\nSecond line.",
                description = "",
                resourceId = "deploy-helper-id",
            ),
            UploadedContent(
                name = "title-fallback-skill",
                content = "# Only headings",
                description = "",
                resourceId = null,
            ),
        )
        val uploadedDocuments = listOf(
            UploadedContent(name = "Shared Handbook", content = "Duplicate document"),
            UploadedContent(name = "Runbook", content = "Unique document body"),
        )

        val components = assembleChatContextComponents(
            systemPrompt = "System prompt",
            builtComponents = builtComponents,
            conversation = Conversation(id = "conv-1"),
            knowledgeTitle = "Knowledge",
            wikiTitle = "Wiki",
            personalContext = null,
            healthSummary = null,
            healthTitle = "Health",
            uploadedSkills = uploadedSkills,
            uploadedDocuments = uploadedDocuments,
            attachments = emptyList(),
        )

        val addedSkills = components.filter { it.kind == "skill" && it.id != "skill:existing-id" }
        assertEquals(2, addedSkills.size)

        val deployMeta = json.parseToJsonElement(addedSkills[0].content.jsonPrimitive.content).jsonObject
        assertEquals("Verify release checksums before upload.", deployMeta["summary"]?.jsonPrimitive?.content)
        assertTrue(deployMeta["usage_contract"]?.jsonPrimitive?.content?.contains("skill_invoke") == true)

        val fallbackMeta = json.parseToJsonElement(addedSkills[1].content.jsonPrimitive.content).jsonObject
        assertEquals("title-fallback-skill", fallbackMeta["summary"]?.jsonPrimitive?.content)
        assertFalse(fallbackMeta["usage_contract"]?.jsonPrimitive?.content.isNullOrBlank())

        val addedDocuments = components.filter { it.kind == "document" && it.id != "doc:1" }
        assertEquals(1, addedDocuments.size)
        assertEquals("Runbook", addedDocuments.single().source?.title)

        val nonSystem = nonSystemTurnContextComponents(components)
        assertTrue(nonSystem.none { it.kind == "system" })
        assertTrue(nonSystem.any { it.kind == "skill" })
        assertTrue(nonSystem.any { it.kind == "document" })
    }
}
