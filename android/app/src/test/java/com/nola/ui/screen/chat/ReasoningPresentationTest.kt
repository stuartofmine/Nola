package com.nola.ui.screen.chat

import com.nola.domain.projection.ConversationProjectionV1
import com.nola.domain.projection.TextReviewState
import com.nola.domain.projection.ToolExecutionSource
import org.junit.Assert.*
import org.junit.Test
import kotlinx.serialization.json.JsonNull

class ReasoningPresentationTest {
    @Test fun publicCommentaryRemainsVisibleBeforeAndAfterTools() {
        val parts = listOf(
            ConversationProjectionV1.Part.Text("comment", 1, "checking", false),
            ConversationProjectionV1.Part.Reasoning("reason", 1, ConversationProjectionV1.ActivityState.Completed, "verified"),
            ConversationProjectionV1.Part.Text("answer", 1, "done", true),
        )
        assertEquals("checkingdone", executionAnswer(parts, "stale flattened text"))
        assertEquals("fallback", executionAnswer(emptyList(), "fallback"))
    }
    @Test fun canonicalReviewCannotRevealStaleFlattenedText() {
        TextReviewState.entries.filter { it != TextReviewState.Approved }.forEach { review ->
            val hidden = ConversationProjectionV1.Part.Text("review", 1, "withheld", false, review)
            assertEquals("", executionAnswer(listOf(hidden), "withheld stale fallback"))
        }
    }

    @Test fun toolsAndReasoningAreNeverFlattenedIntoTheAnswer() {
        val tool = ConversationProjectionV1.Part.Tool(
            partId = "tool", revision = 1L, toolCallId = "call", capabilityId = "render_ui",
            executionSource = ToolExecutionSource.Model,
            state = ConversationProjectionV1.ActivityState.Running, displayKey = null,
            input = kotlinx.serialization.json.JsonPrimitive("raw A2UI JSON"), output = null, affectedResources = emptyList(),
        )
        val parts = listOf(
            ConversationProjectionV1.Part.Text("public", 1, "Preparing the form", false),
            tool,
            ConversationProjectionV1.Part.Reasoning("reason", 1, ConversationProjectionV1.ActivityState.Running, "public summary"),
        )
        assertEquals("Preparing the form", executionAnswer(parts, "raw A2UI JSON"))
    }

    @Test fun laterToolOrSettledReasoningDoesNotReviveAnOldPlaceholder() {
        val old = ConversationProjectionV1.Part.Reasoning("old", 1L, ConversationProjectionV1.ActivityState.Running, null)
        val settled = old.copy(partId = "settled", state = ConversationProjectionV1.ActivityState.Completed)
        assertNull(visibleReasoningPlaceholderId(listOf(old, settled), isLive = true))
        val tool = ConversationProjectionV1.Part.Tool(
            partId = "tool", revision = 1L, toolCallId = "call", capabilityId = "bash",
            executionSource = ToolExecutionSource.Model,
            state = ConversationProjectionV1.ActivityState.Running, displayKey = null,
            input = JsonNull, output = null, affectedResources = emptyList(),
        )
        assertNull(visibleReasoningPlaceholderId(listOf(old, tool), isLive = true))
        assertEquals("old", visibleReasoningPlaceholderId(listOf(tool, old), isLive = true))
    }

    @Test fun restoredReasoningNeverAnimatesStaleRunningState() {
        ConversationProjectionV1.ActivityState.entries.forEach {
            assertTrue(reasoningIsFinished(it, isLive = false))
        }
        assertFalse(reasoningIsFinished(ConversationProjectionV1.ActivityState.Running, isLive = true))
        assertTrue(reasoningIsFinished(ConversationProjectionV1.ActivityState.Completed, isLive = true))
    }

    @Test fun onlyLatestLiveReasoningCanShowAnEmptyPlaceholder() {
        val parts = listOf(
            ConversationProjectionV1.Part.Reasoning("old", 1L, ConversationProjectionV1.ActivityState.Running, null),
            ConversationProjectionV1.Part.Reasoning("current", 1L, ConversationProjectionV1.ActivityState.Running, ""),
        )
        assertEquals("current", visibleReasoningPlaceholderId(parts, isLive = true))
        assertNull(visibleReasoningPlaceholderId(parts, isLive = false))
        assertEquals(2, parts.size)
    }
}
