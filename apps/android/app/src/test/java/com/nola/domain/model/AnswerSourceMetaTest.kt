package com.nola.domain.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class AnswerSourceMetaTest {
    @Test
    fun answerSourcesRoundTripForConversationHistory() {
        val source = AnswerSource(
            "citation-1",
            "source-1",
            "Evidence",
            "https://example.com/report",
            "text/html",
            2,
            10,
            14,
            "Results",
            memoryScope = "global",
            memoryNamespace = "workspace",
            memorySource = "conversation://session-1/message-9",
            memoryObservedAt = "2026-08-06T00:00:00Z",
            memoryUsageCount = 4,
            memorySupersedesCount = 1,
            memoryRequiresVerification = true,
        )
        val meta = encodeAnswerSourcesForMeta(listOf(source))
        assertEquals(listOf(source), decodeAnswerSourcesFromMeta(meta))
        assertTrue(decodeAnswerSourcesFromMeta(null).isEmpty())
    }
}
