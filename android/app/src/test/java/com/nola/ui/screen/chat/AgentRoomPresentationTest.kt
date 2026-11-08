// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import com.nola.data.repository.AgentRoom
import com.nola.data.repository.AgentRoomSnapshot
import com.nola.data.repository.RoomParticipant
import com.nola.domain.model.ChatRole
import com.nola.domain.model.ConversationMessage
import org.junit.Assert.*
import org.junit.Test

class AgentRoomPresentationTest {
    @Test fun timelineKeepsCanonicalPromptBeforeReplyWhenTimestampsTieOrRegress() {
        val presentation = AgentRoomPresentation()
        presentation.open(room())
        presentation.publish(ChatUiState(conversationId = first.contextConversationId, messages = listOf(
            ConversationMessage("z-user", first.contextConversationId, ChatRole.user, "prompt", 20),
            ConversationMessage("a-answer", first.contextConversationId, ChatRole.assistant, "answer", 20),
            ConversationMessage("later-user", first.contextConversationId, ChatRole.user, "next", 10),
        )))
        presentation.publish(context(second.contextConversationId, 15))
        assertEquals(listOf("same-message-id", "z-user", "a-answer", "later-user"), presentation.state.value.rows().map { it.message.id })
    }
    @Test fun inspectingDepartedMemberDoesNotChangeSendingRecipient() {
        val presentation = AgentRoomPresentation()
        presentation.open(room(participants = listOf(first, second.copy(active = false))))
        presentation.select(first.id)
        assertEquals(second.contextConversationId, presentation.inspect(second.id))
        assertEquals(second.id, presentation.state.value.inspectedParticipantId)
        assertEquals(first, presentation.state.value.recipient)
        assertThrows(IllegalStateException::class.java) { presentation.requireRecipient(second.contextConversationId) }
        assertThrows(NoSuchElementException::class.java) { presentation.select(second.id) }
        presentation.open(room(2, listOf(first.copy(active = false), second.copy(active = false))))
        assertNull(presentation.state.value.recipient)
        assertEquals(second.id, presentation.state.value.inspectedParticipantId)
    }
    @Test fun closingOrSupersedingRoomInvalidatesSuspendedLoads() {
        val presentation = AgentRoomPresentation()
        val firstLoad = presentation.beginLoad()
        val secondLoad = presentation.beginLoad()
        assertFalse(presentation.acceptsLoad(firstLoad))
        assertTrue(presentation.acceptsLoad(secondLoad))
        presentation.close()
        assertFalse(presentation.acceptsLoad(secondLoad))
    }
    private val first = RoomParticipant("one", "agent-one", 1, "First", "context-one", true, "Research")
    private val second = RoomParticipant("two", "agent-two", 1, "Second", "context-two", true, "Review")
    private fun room(revision: Long = 1, participants: List<RoomParticipant> = listOf(first, second)) =
        AgentRoomSnapshot(revision, AgentRoom("room", "Discussion", participants))
    private fun context(owner: String, time: Long) = ChatUiState(
        conversationId = owner,
        messages = listOf(ConversationMessage("same-message-id", owner, ChatRole.assistant, owner, time)),
    )

    @Test fun aggregationPreservesOwnersAndIndependentIdentity() {
        val presentation = AgentRoomPresentation()
        presentation.open(room())
        presentation.publish(context(second.contextConversationId, 20))
        presentation.publish(context(first.contextConversationId, 10))
        val rows = presentation.state.value.rows()
        assertEquals(listOf(first, second), rows.map { it.participant })
        assertEquals(2, rows.map { it.key }.toSet().size)
        assertEquals(1, rows[0].context.messages.size)
        assertNull(presentation.state.value.recipient)
    }

    @Test fun switchingRecipientCannotSendToPreviouslyDisplayedContext() {
        val presentation = AgentRoomPresentation()
        presentation.open(room())
        presentation.select(first.id)
        assertEquals(first, presentation.requireRecipient(first.contextConversationId))
        presentation.select(second.id)
        assertThrows(IllegalStateException::class.java) {
            presentation.requireRecipient(first.contextConversationId)
        }
        assertEquals(second, presentation.requireRecipient(second.contextConversationId))
    }

    @Test fun leavingRetainsHistoryButRevokesSelectionAndIgnoresStaleRoom() {
        val presentation = AgentRoomPresentation()
        presentation.open(room())
        presentation.publish(context(first.contextConversationId, 10))
        presentation.select(first.id)
        presentation.open(room(2, listOf(first.copy(active = false), second)))
        assertNull(presentation.state.value.recipient)
        assertEquals(1, presentation.state.value.rows().size)
        presentation.open(room())
        assertEquals(2L, presentation.state.value.room?.revision)
    }

    @Test fun anotherRoomAndLateCallbacksCannotLeakHistory() {
        val presentation = AgentRoomPresentation()
        presentation.open(room())
        presentation.publish(context(first.contextConversationId, 10))
        presentation.open(AgentRoomSnapshot(1, AgentRoom("other", "Other", emptyList())))
        presentation.publish(context(first.contextConversationId, 20))
        assertTrue(presentation.state.value.contexts.isEmpty())
    }

    @Test fun mixedOwnerProjectionIsRejected() {
        val presentation = AgentRoomPresentation()
        presentation.open(room())
        assertThrows(IllegalArgumentException::class.java) {
            presentation.publish(context(second.contextConversationId, 10).copy(conversationId = first.contextConversationId))
        }
    }
}
