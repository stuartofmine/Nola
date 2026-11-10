package com.nola.ui.screen.chat

import com.nola.domain.model.HostAgentPermissionRequest
import com.nola.domain.model.HostAgentToolEvent
import com.nola.domain.projection.ConversationProjectionV1
import com.nola.domain.projection.cardFacts
import kotlinx.serialization.json.JsonNull
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test

class ChatToolActivityPresentationTest {
    private fun tool(artifacts: String = "[]"): ConversationProjectionV1.Part.Tool =
        com.nola.domain.projection.ConversationProjectionV1Decoder.decodeDisplayPart(
            kotlinx.serialization.json.Json.parseToJsonElement("""{
                "kind":"tool","execution_source":{"kind":"model"},"part_id":"tool","revision":1,"tool_call_id":"call","capability_id":"file.read",
                "title":"Read document","summary":"","state":"completed","input":null,"output":null,"origin":{"kind":"builtin"},
                "output_preview":"technical","affected_resources":[],"artifacts":$artifacts,
                "full_output":{"message_id":"assistant","part_id":"tool","tool_call_id":"call"}
            }""").let { it as kotlinx.serialization.json.JsonObject }, "assistant"
        ) as ConversationProjectionV1.Part.Tool

    @Test
    fun activeJournalMessageUsesTypedToolCardsWithoutAuditSummaryFallback() {
        val message = com.nola.domain.model.ConversationMessage(id = "assistant", conversationId = "conversation",
            role = com.nola.domain.model.ChatRole.assistant, text = "", createdAt = 1, canonicalParts = listOf(tool()))
        val state = ChatUiState(conversationId = "conversation", activeResponseId = "assistant",
            activeResponseStartedAt = 1, journalResponse = message)
        val active = state.activeAssistantMessage() ?: error("active assistant missing")
        val facts = (active.canonicalParts.single() as ConversationProjectionV1.Part.Tool).cardFacts()
        assertEquals("Read document", facts.title); assertEquals("", facts.summary)
        assertEquals(JsonNull, facts.input); assertEquals(JsonNull, facts.result)
        assertEquals("technical", facts.preview); assertEquals("assistant", active.id)
        assertEquals("assistant", facts.fullOutput?.messageId)
        assertEquals(message, state.timelineMessages().single())
        assertFalse(facts.toString().contains("/private/output"))
        val foreign = state.copy(conversationId = "foreign").activeAssistantMessage()
        assertEquals("A journal row cannot attach to another conversation", null, foreign)
    }

    @Test
    fun typedToolArtifactsPreserveOpenableLinksAndOperationSemantics() {
        val facts = tool("""[
            {"uri":"https://example.com/source","name":"Source","operation":"open"},
            {"uri":"https://host.example/artifacts/report.docx","name":"report.docx","operation":"create"}
        ]""").cardFacts()
        assertEquals("https://example.com/source", facts.artifacts[0].uri)
        assertEquals("Source", facts.artifacts[0].name)
        assertEquals("open", facts.artifacts[0].operation)
        assertEquals("https://host.example/artifacts/report.docx", facts.artifacts[1].uri)
        assertEquals("report.docx", facts.artifacts[1].name)
        assertEquals("create", facts.artifacts[1].operation)
        assertEquals("technical", facts.preview)
    }

    @Test
    fun permissionRequiredHasPriorityWithoutDuplicatingTheTool() {
        val request = HostAgentPermissionRequest(
            allowConversation = true,
            id = "tool-1",
            toolName = "write_file",
            args = mapOf("path" to "/tmp/report.md"),
            match = "filesystem.write",
            reason = "Write the requested report",
            message = "Allow writing the report?",
        )
        val running = HostAgentToolEvent(
            id = request.id,
            toolName = request.toolName,
            status = "running",
        )

        val events = transitionHostToolPermission(
            events = listOf(running),
            request = request,
            status = "permission_required",
        )

        assertEquals(1, events.size)
        assertEquals("permission_required", events.single().status)
        assertEquals(
            HostToolActivityPresentation(HostToolActivityState.PermissionRequired, 1),
            hostToolActivityPresentation(events),
        )
    }

    @Test
    fun permissionDecisionKeepsTheStableEventIdentity() {
        val request = HostAgentPermissionRequest(
            allowConversation = true,
            id = "tool-1",
            toolName = "shell",
            args = mapOf("command" to "mkdir output"),
            match = "shell.execute",
            reason = "Create an output directory",
            message = "Allow the command?",
        )
        val pending = transitionHostToolPermission(
            events = emptyList(),
            request = request,
            status = "permission_required",
        )
        val resumed = transitionHostToolPermission(
            events = pending,
            request = request,
            status = "running",
        )

        assertEquals(1, resumed.size)
        assertEquals(request.id, resumed.single().id)
        assertEquals("running", resumed.single().status)
        assertFalse(resumed.single().error)
    }
}
