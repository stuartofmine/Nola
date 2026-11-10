// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import com.nola.domain.model.ChatRole
import com.nola.domain.model.ConversationMessage
import com.nola.domain.model.ProcessingState
import org.junit.Assert.*
import org.junit.Test

class ExpertReplyProjectionTest {
    @Test fun usesTheActualJournalMessageWhileItsRunIsStillStreaming() {
        val canonical = ConversationMessage("child-run:assistant", "child-context", ChatRole.assistant, "First committed batch")
        val state = ChatUiState(conversationId = "child-context", activeTurnId = "child-run",
            activeResponseId = canonical.id, journalResponse = canonical,
            currentResponse = "an older presentation", thinking = "Compare the evidence", processingState = ProcessingState.Streaming)
        assertSame(canonical, state.exactExpertAssistant("child-context", "child-run"))
        assertNull(state.exactExpertAssistant("another-context", "child-run"))
        assertNull(state.exactExpertAssistant("child-context", "another-run"))
    }

    @Test fun neverBorrowAnAssistantShapedIdFromAnotherOwnerOrAUser() {
        val foreign = ConversationMessage("child-run:assistant", "foreign-context", ChatRole.assistant, "Foreign")
        assertNull(ChatUiState(conversationId = "child-context", messages = listOf(foreign))
            .exactExpertAssistant("child-context", "child-run"))
        val user = foreign.copy(conversationId = "child-context", role = ChatRole.user)
        assertNull(ChatUiState(conversationId = "child-context", messages = listOf(user))
            .exactExpertAssistant("child-context", "child-run"))
        assertNotEquals(ChatViewModel.ExpertParentMessageKey("parent-a", "user-1"),
            ChatViewModel.ExpertParentMessageKey("parent-b", "user-1"))
    }
}
