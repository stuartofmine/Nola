package com.nola.data.local

import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive

/** Creation is not an upsert: never adopt another owner's existing aggregate. */
internal fun requireCreatedConversation(receipt: JsonElement) {
    check((receipt as? JsonObject)?.get("created") == JsonPrimitive(true)) {
        "Conversation was not created; its existing configuration was not changed."
    }
}
