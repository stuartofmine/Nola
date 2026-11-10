package com.nola.ui.screen.chat

import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Test

class CanonicalUserMessageAdmissionRetryTest {
    @Test fun retryReusesConversationMessageTextAndTimestamp() {
        var ids = 0
        var clock = 100L
        val state = CanonicalUserMessageAdmissionRetryState(
            newMessageId = { "message-${++ids}" },
            nowMs = { clock.also { clock += 50 } },
        )

        val first = state.beginOrResume("conversation:c1", "c1", "Ask the experts", setOf("expert-a"))
        val retry = state.beginOrResume("conversation:c1", "c1", "Ask the experts", setOf("expert-a"))

        assertEquals(setOf("expert-a"), first.selectedAgentIds)
        assertEquals("c1", first.conversationId)
        assertEquals("message-1", first.messageId)
        assertEquals("Ask the experts", first.text)
        assertEquals(100L, first.createdAtMs)
        assertEquals(first, retry)
        assertEquals(1, ids)
    }

    @Test fun retryAfterNewConversationOwnerIsCreatedUsesSameAdmission() {
        var ids = 0
        var clock = 100L
        val state = CanonicalUserMessageAdmissionRetryState(
            newMessageId = { "message-${++ids}" },
            nowMs = { clock.also { clock += 50 } },
        )
        val first = state.beginOrResume("draft:pending", "conversation-1", "Ask the experts", setOf("expert-a"))

        assertEquals(first, state.find("conversation:conversation-1", "conversation-1"))
        assertEquals(first, state.beginOrResume("conversation:conversation-1", "conversation-1", "Ask the experts", setOf("expert-b")))
        assertEquals(1, ids)
    }

    @Test fun changedDraftOrConversationCannotReplaceUncertainAdmission() {
        val state = CanonicalUserMessageAdmissionRetryState({ "message-1" }, { 100L })
        state.beginOrResume("conversation:c1", "c1", "Original", setOf("expert-a"))

        assertThrows(IllegalArgumentException::class.java) {
            state.beginOrResume("conversation:c1", "c1", "Edited", setOf("expert-a"))
        }
        assertThrows(IllegalArgumentException::class.java) {
            state.beginOrResume("conversation:c1", "c2", "Original", setOf("expert-a"))
        }
    }

    @Test fun onlyConfirmedMatchingAttemptClearsPendingIdentity() {
        var ids = 0
        val state = CanonicalUserMessageAdmissionRetryState({ "message-${++ids}" }, { 100L })
        val first = state.beginOrResume("conversation:c1", "c1", "Original", setOf("expert-a"))
        state.confirm("conversation:c1", first.copy(createdAtMs = 101))
        assertEquals(first, state.beginOrResume("conversation:c1", "c1", "Original", setOf("expert-a")))

        state.confirm("conversation:c1", first)
        assertEquals("message-2", state.beginOrResume("conversation:c1", "c1", "Original", setOf("expert-a")).messageId)
    }

    @Test fun uncertainAdmissionRetryKeepsOriginalExpertSetWhenUiSelectionChanges() {
        val state = CanonicalUserMessageAdmissionRetryState({ "message-1" }, { 100L })
        val first = state.beginOrResume("conversation:c1", "c1", "Original", setOf("expert-a", "expert-b"))

        val retry = state.beginOrResume("conversation:c1", "c1", "Original", setOf("expert-c"))

        assertEquals(first.messageId, retry.messageId)
        assertEquals(setOf("expert-a", "expert-b"), retry.selectedAgentIds)
    }

}
