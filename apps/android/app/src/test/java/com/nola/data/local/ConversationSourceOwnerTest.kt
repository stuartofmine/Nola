package com.nola.data.local

import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

class ConversationSourceOwnerTest {
    @Test fun remoteIdentityRequiresBothHostAndEpoch() {
        val value = Json.parseToJsonElement("""{"kind":"remote","source_host_id":"host","source_store_epoch":"epoch"}""").jsonObject
        assertEquals(ConversationSourceOwner.Remote("host","epoch"),decodeConversationSourceOwner(value))
        for (invalid in listOf(JsonObject(value-"source_store_epoch"), JsonObject(value+("source_store_epoch" to JsonPrimitive(""))),
            buildJsonObject { put("kind","unknown") })) {
            assertTrue(runCatching { decodeConversationSourceOwner(invalid) }.isFailure)
        }
        assertEquals(ConversationSourceOwner.Unavailable, decodeConversationSourceOwner(buildJsonObject { put("kind","unavailable") }))
        assertEquals(ConversationSourceOwner.Local, decodeConversationSourceOwner(buildJsonObject { put("kind","local") }))
    }
}
