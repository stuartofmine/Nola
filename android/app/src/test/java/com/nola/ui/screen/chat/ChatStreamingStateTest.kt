package com.nola.ui.screen.chat

import com.nola.domain.model.HostAgentToolEvent
import com.nola.domain.model.ProcessingState
import com.nola.domain.model.ChatRole
import com.nola.domain.model.ConversationMessage
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test

class ChatStreamingStateTest {
    @Test
    fun compactionSourceTargetIsConsumedExactlyOnceForItsRequest() {
        val pending = ChatUiState(
            compactionSourceTargetId = "canonical-source",
            compactionSourceRequest = 7,
        )

        val consumed = pending.consumeCompactionSourceTarget(7)
        assertEquals(null, consumed.compactionSourceTargetId)
        assertSame(consumed, consumed.consumeCompactionSourceTarget(7))
        assertSame(pending, pending.consumeCompactionSourceTarget(6))
    }

    @Test
    fun requestCreatesStableAssistantPlaceholderBeforeAnyDelta() {
        val started = ChatUiState().beginAssistantTurn("assistant-turn-1", 1234L)

        assertEquals(ProcessingState.BuildingContext, started.processingState)
        assertTrue(started.processingState.isGenerating)
        assertTrue(started.processingState.blocksInput)
        assertEquals("assistant-turn-1", started.timelineMessages().single().id)
        assertEquals("", started.timelineMessages().single().text)
    }

    @Test
    fun deltasThinkingAndToolsKeepTheSameTimelineIdentity() {
        val started = ChatUiState().beginAssistantTurn("assistant-turn-1", 1234L)
        val streaming = started.copy(
            processingState = ProcessingState.Streaming,
            currentResponse = "**Partial",
            thinking = "Checking sources",
            toolEvents = listOf(
                HostAgentToolEvent(id = "tool-1", toolName = "web_search", status = "running")
            ),
        )

        val message = streaming.timelineMessages().single()
        assertEquals("assistant-turn-1", message.id)
        assertEquals(1234L, message.createdAt)
        assertEquals("**Partial", message.text)
        assertEquals("tool-1", streaming.toolEvents.single().id)
    }

    @Test
    fun completionReusesPlaceholderIdentityWithoutDuplicateItem() {
        val streaming = ChatUiState()
            .beginAssistantTurn("assistant-turn-1", 1234L)
            .copy(processingState = ProcessingState.Streaming, currentResponse = "**Complete**")
        val finalMessage = ConversationMessage(
            id = "assistant-turn-1",
            conversationId = "conversation-1",
            role = ChatRole.assistant,
            text = "**Complete**",
            createdAt = 1234L,
            meta = mapOf("nola.answerBasis" to "tools"),
        )
        val completed = streaming.completeAssistantTurn(finalMessage)

        assertEquals("assistant-turn-1", finalMessage.id)
        assertEquals(1234L, finalMessage.createdAt)
        assertEquals(1, completed.timelineMessages().size)
        assertEquals("assistant-turn-1", completed.timelineMessages().single().id)
        assertNull(completed.activeResponseId)
        assertEquals(ProcessingState.Idle, completed.processingState)
    }

    @Test
    fun errorsDoNotPermanentlyBlockTheComposer() {
        assertFalse(ProcessingState.Error.blocksInput)
        assertFalse(ProcessingState.Idle.blocksInput)
        assertTrue(ProcessingState.BuildingContext.blocksInput)
        assertTrue(ProcessingState.ParsingThinking.blocksInput)
    }
}
