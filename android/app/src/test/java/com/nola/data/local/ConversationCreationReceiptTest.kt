package com.nola.data.local

import kotlinx.serialization.json.Json
import org.junit.Assert.assertThrows
import org.junit.Test

class ConversationCreationReceiptTest {
    @Test fun onlyConfirmedCreationSucceeds() {
        requireCreatedConversation(Json.parseToJsonElement("""{"created":true}"""))
        listOf("""{"created":false}""", "{}", "null", """{"created":"true"}""").forEach {
            assertThrows(IllegalStateException::class.java) {
                requireCreatedConversation(Json.parseToJsonElement(it))
            }
        }
    }
}
