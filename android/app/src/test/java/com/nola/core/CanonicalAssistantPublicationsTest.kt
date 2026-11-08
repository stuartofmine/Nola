package com.nola.core

import kotlinx.serialization.json.*
import kotlin.test.*

class CanonicalAssistantPublicationsTest {
    private val cursor = buildJsonObject {
        put("identity", buildJsonObject { put("source_store_epoch", "epoch"); put("conversation_id", "conversation")
            put("run_id", "run"); put("assistant_message_id", "assistant") })
        put("sequence", "18446744073709551615"); put("record_digest", "a".repeat(64))
    }
    @Test fun hostResponseKeepsActualCommitCursorsEvenWhenBodyEventsAreAbsent() {
        val response = buildJsonObject { put("commit", buildJsonObject { put("assistant_publications", JsonArray(listOf(cursor))) })
            put("events", JsonArray(emptyList())); put("traces", JsonArray(emptyList())) }
        assertEquals(listOf(cursor), canonicalAssistantPublications(response))
        assertEquals("18446744073709551615", canonicalAssistantPublications(response).single().getValue("sequence").jsonPrimitive.content)
    }
    @Test fun beginAndIdlePublicationCarriersStayDistinctFromTransientControlEvents() {
        assertEquals(listOf(cursor), canonicalAssistantPublications(buildJsonObject { put("assistant_publications", JsonArray(listOf(cursor))) }))
        assertTrue(canonicalAssistantPublications(buildJsonObject { put("events", JsonArray(emptyList())) }).isEmpty())
    }
}
