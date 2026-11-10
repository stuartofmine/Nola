package com.nola.domain.model

import kotlin.test.Test
import kotlin.test.assertEquals

class ConversationTitlePresentationPolicyTest {
    @Test
    fun runtimeIdentifiersUseStableNewConversationLabel() {
        val id = "01DAF2BB-1906-4993-9B5A-9AC74E05B5D8"
        listOf(
            id,
            "00000000-0000-0000-0000-000000000000",
            "Session 01DAF2BB",
            "Conversation 01DAF2BB",
        ).forEach { title ->
            assertEquals(
                "New chat",
                ConversationTitlePresentationPolicy.displayTitle(title, id, "New chat"),
            )
        }
    }

    @Test
    fun userTitlesWithSimilarWordsRemainVisible() {
        val id = "01DAF2BB-1906-4993-9B5A-9AC74E05B5D8"
        assertEquals(
            "Session planning",
            ConversationTitlePresentationPolicy.displayTitle("Session planning", id, "New chat"),
        )
        assertEquals(
            "Conversation design review",
            ConversationTitlePresentationPolicy.displayTitle("Conversation design review", id, "New chat"),
        )
    }
}
