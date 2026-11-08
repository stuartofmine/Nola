package com.nola.domain.projection

import com.nola.domain.model.ChatRole
import com.nola.domain.model.ConversationMessage
import com.nola.domain.model.decodeAgentThinkingFromMeta
import com.nola.domain.model.decodeHostToolEventsFromMeta
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertFailsWith
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive

class CanonicalConversationProjectorTests {
    private fun journalMessage(id: String, text: String = id) = ConversationMessage(
        id = id, conversationId = "conversation", role = ChatRole.assistant, text = text, createdAt = 1,
    )

    @Test
    fun journalMergeKeepsPagedHistoryAndNewerUnrelatedRows() {
        val older = journalMessage("older")
        val unrelated = journalMessage("unrelated", "newer committed content")
        val active = journalMessage("active")
        val result = CanonicalConversationProjector.mergeJournalMessage(
            listOf(older, unrelated, active), listOf(journalMessage("unrelated", "stale"), active),
            active.copy(text = "streamed"),
        )
        assertEquals(listOf("older", "unrelated", "active"), result.map { it.id })
        assertEquals(unrelated, result[1])
        assertEquals("streamed", result.last().text)
    }

    @Test
    fun journalMergeInsertsMissingAdmissionsWithoutReorderingLoadedHistory() {
        val window = listOf(journalMessage("first"), journalMessage("second"), journalMessage("third"))
        val result = CanonicalConversationProjector.mergeJournalMessage(
            listOf(journalMessage("older"), window.last()), window, window[1].copy(text = "delta"),
        )
        assertEquals(listOf("older", "first", "second", "third"), result.map { it.id })
        assertEquals("delta", result[2].text)
    }

    @Test
    fun journalMergeRejectsForeignConversationAndMissingAdmission() {
        val active = journalMessage("active")
        assertFailsWith<IllegalArgumentException> {
            CanonicalConversationProjector.mergeJournalMessage(
                listOf(active.copy(conversationId = "foreign")), listOf(active), active,
            )
        }
        assertFailsWith<IllegalArgumentException> {
            CanonicalConversationProjector.mergeJournalMessage(emptyList(), emptyList(), active)
        }
    }

    @Test
    fun canonicalPartsOwnRestoredAssistantTextThinkingToolsAndContext() {
        val source = ToolExecutionSource.CodeCell(CodeCellIdentity(
            "host", "conversation-1", "turn-1", "parent-call", "cell", "generation",
        ))
        val projection = ConversationProjectionV1(
            contractVersion = 1,
            conversationId = "conversation-1",
            revision = 7,
            title = "Projection",
            messages = listOf(
                ConversationProjectionV1.Message(
                    messageId = "turn-1:assistant",
                    role = ConversationProjectionV1.Role.Assistant,
                    state = ConversationProjectionV1.MessageState.Completed,
                    revision = 7,
                    createdAtMs = 1234,
                    parts = listOf(
                        ConversationProjectionV1.Part.Reasoning(
                            "reasoning-1", 1,
                            ConversationProjectionV1.ActivityState.Completed,
                            "Checked the file",
                        ),
                        ConversationProjectionV1.Part.Tool(
                            "tool-1", 2, "call-1", "file.read",
                            source,
                            ConversationProjectionV1.ActivityState.Completed,
                            null,
                            JsonObject(mapOf("path" to JsonPrimitive("report.md"))),
                            JsonObject(mapOf("text" to JsonPrimitive("marker"))),
                            emptyList(),
                        ),
                        ConversationProjectionV1.Part.Text(
                            "text-1", 3, "## Verified\n\nmarker", true,
                        ),
                    ),
                ),
            ),
            activeRun = null,
            contextUsage = ConversationProjectionV1.ContextUsage(
                usedTokens = 512,
                windowTokens = 65_536,
                promptTokenLimit = 61_440,
                estimated = false,
                measuredAtRevision = 7,
            ),
            surface = ConversationProjectionV1.Surface(
                revision = 7,
                committedMessageIds = listOf("turn-1:assistant"),
                archivedActivityPartIds = listOf("reasoning-1", "tool-1"),
            ),
        )

        val message = CanonicalConversationProjector.project(projection).single()

        assertEquals("turn-1:assistant", message.id)
        assertEquals(ChatRole.assistant, message.role)
        assertEquals("## Verified\n\nmarker", message.text)
        assertEquals("Checked the file", decodeAgentThinkingFromMeta(message.meta))
        assertEquals("file.read", decodeHostToolEventsFromMeta(message.meta).single().toolName)
        assertEquals(source, message.canonicalParts.filterIsInstance<ConversationProjectionV1.Part.Tool>()
            .single().executionSource)
        assertEquals("512", message.meta?.get("nola.context.usedTokens"))
        assertEquals("65536", message.meta?.get("nola.context.limitTokens"))
        assertEquals("false", message.meta?.get(CONVERSATION_SURFACE_LIVE_META_KEY))
        assertFalse(message.meta.isNullOrEmpty())
    }

    @Test
    fun surfaceOrdersCommittedHistoryBeforeOneLivePlaceholder() {
        fun message(id: String, state: ConversationProjectionV1.MessageState) =
            ConversationProjectionV1.Message(
                messageId = id,
                role = ConversationProjectionV1.Role.Assistant,
                state = state,
                revision = 1,
                createdAtMs = 1,
                parts = emptyList(),
            )
        val projection = ConversationProjectionV1(
            contractVersion = 1,
            conversationId = "conversation-1",
            revision = 2,
            title = "Projection",
            messages = listOf(
                message("live", ConversationProjectionV1.MessageState.Running),
                message("history", ConversationProjectionV1.MessageState.Completed),
            ),
            activeRun = null,
            contextUsage = ConversationProjectionV1.ContextUsage(0, 65_536, 61_440, true, 2),
            surface = ConversationProjectionV1.Surface(
                revision = 2,
                committedMessageIds = listOf("history"),
                liveMessageId = "live",
                showAssistantPlaceholder = true,
            ),
        )

        val projected = CanonicalConversationProjector.project(projection)

        assertEquals(listOf("history", "live"), projected.map { it.id })
        assertEquals("true", projected.last().meta?.get(CONVERSATION_SURFACE_LIVE_META_KEY))
        assertEquals("true", projected.last().meta?.get(CONVERSATION_SURFACE_PLACEHOLDER_META_KEY))
    }

    @Test
    fun systemProtocolMessagesStayOutOfTheChatTimeline() {
        fun message(id: String, role: ConversationProjectionV1.Role) =
            ConversationProjectionV1.Message(
                messageId = id,
                role = role,
                state = ConversationProjectionV1.MessageState.Completed,
                revision = 1,
                createdAtMs = 1,
                parts = listOf(ConversationProjectionV1.Part.Text("$id:text", 1, id, true)),
            )
        val projection = ConversationProjectionV1(
            contractVersion = 1,
            conversationId = "conversation-1",
            revision = 3,
            title = "Projection",
            messages = listOf(
                message("system-policy", ConversationProjectionV1.Role.System),
                message("user-visible", ConversationProjectionV1.Role.User),
                message("assistant-visible", ConversationProjectionV1.Role.Assistant),
            ),
            activeRun = null,
            contextUsage = ConversationProjectionV1.ContextUsage(0, 65_536, 61_440, true, 3),
            surface = ConversationProjectionV1.Surface(
                revision = 3,
                committedMessageIds = listOf("system-policy", "user-visible", "assistant-visible"),
            ),
        )

        assertEquals(
            listOf("user-visible", "assistant-visible"),
            CanonicalConversationProjector.project(projection).map { it.id },
        )
    }
    @Test
    fun uiActionRetainsEnvelopeButDisplaysOnlyItsSummary() {
        val part = ConversationProjectionV1Decoder.decodeDisplayPart(kotlinx.serialization.json.Json.parseToJsonElement("""
            {"kind":"ui_action","part_id":"action","revision":1,
             "surface":{"identity":{"owner":{"authority_id":"host","conversation_id":"conversation-1",
             "participant_id":"agent","context_id":"context","originating_run_id":"origin",
             "message_id":"original-assistant","part_id":"publisher"},"surface_id":"form","generation":1},"revision":2},
             "envelope":{"version":"v0.9.1","action":{"name":"submit","context":{"name":"Android验收"}}},
             "summary":"Submitted Acceptance form"}
        """.trimIndent()), "new-user") as ConversationProjectionV1.Part.UiAction
        assertEquals("original-assistant", part.surface.identity.owner.message_id)
        assertEquals("v0.9.1", (part.envelope.getValue("version") as JsonPrimitive).content)
        val projection = ConversationProjectionV1(1, "conversation-1", 1, "UI action",
            listOf(ConversationProjectionV1.Message("new-user", ConversationProjectionV1.Role.User,
                ConversationProjectionV1.MessageState.Completed, 1, 1, listOf(part))), null,
            ConversationProjectionV1.ContextUsage(0, 65536, 61440, true, 1),
            ConversationProjectionV1.Surface(1, listOf("new-user")))
        val message = CanonicalConversationProjector.project(projection).single()
        assertEquals("Submitted Acceptance form", message.text)
        assertFalse(message.text.contains("v0.9.1"))
    }

}
