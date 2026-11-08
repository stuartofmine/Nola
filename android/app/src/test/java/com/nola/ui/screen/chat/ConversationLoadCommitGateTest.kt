package com.nola.ui.screen.chat

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ConversationLoadCommitGateTest {
    @Test
    fun latestIdleLoadCanCommit() {
        assertTrue(
            ConversationLoadCommitGate.shouldCommit(
                requestedGeneration = 4,
                currentGeneration = 4,
            )
        )
    }

    @Test
    fun supersededLoadCannotOverwriteNewerConversation() {
        assertFalse(
            ConversationLoadCommitGate.shouldCommit(
                requestedGeneration = 4,
                currentGeneration = 5,
            )
        )
    }

    @Test
    fun latestSelectionCanCommitWhileAnotherConversationRuns() {
        assertTrue(
            ConversationLoadCommitGate.shouldCommit(
                requestedGeneration = 4,
                currentGeneration = 4,
            )
        )
    }
}
