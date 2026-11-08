package com.nola.ui.screen.chat

import com.nola.domain.model.ProcessingState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Test

class ChatErrorDismissalTest {
    @Test
    fun dismissalPreservesExecutionAndResponseEvidence() {
        val state = ChatUiState(conversationId = "conversation", error = "failed",
            processingState = ProcessingState.Error, activeResponseId = "response",
            activeTurnId = "turn", activeResponseStartedAt = 123L,
            currentResponse = "partial response", thinking = "partial reasoning")
        val dismissed = state.dismissingDisplayedError(state)
        assertNull(dismissed.error)
        assertEquals(state.copy(error = null), dismissed)
        assertEquals(ProcessingState.Error, dismissed.processingState)
    }

    @Test
    fun aStaleCallbackCannotDismissAnotherConversationOrRepeatedError() {
        val old = ChatUiState(conversationId = "first", error = "same error")
        val second = old.copy(conversationId = "second")
        assertSame(second, second.dismissingDisplayedError(old))
        val repeated = old.copy()
        assertSame(repeated, repeated.dismissingDisplayedError(old))
        val dismissed = old.dismissingDisplayedError(old)
        val nextError = dismissed.copy(error = "same error")
        assertEquals("same error", nextError.error)
        assertSame(nextError, nextError.dismissingDisplayedError(old))
    }
}
