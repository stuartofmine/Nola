package com.nola.data.local

import kotlinx.serialization.json.*

/** Persisted Core provenance; independent of selected Host and active Run. */
sealed interface ConversationSourceOwner {
    data object Local : ConversationSourceOwner
    data object Unavailable : ConversationSourceOwner
    data class Remote(val hostId: String, val storeEpoch: String) : ConversationSourceOwner
}

fun decodeConversationSourceOwner(value: JsonObject): ConversationSourceOwner = when (value["kind"]?.jsonPrimitive?.content) {
    "local" -> { require(value.keys == setOf("kind")); ConversationSourceOwner.Local }
    "unavailable" -> { require(value.keys == setOf("kind")); ConversationSourceOwner.Unavailable }
    "remote" -> {
        require(value.keys == setOf("kind", "source_host_id", "source_store_epoch"))
        val host = value.getValue("source_host_id").jsonPrimitive.content
        val epoch = value.getValue("source_store_epoch").jsonPrimitive.content
        require(host.isNotBlank() && epoch.isNotBlank())
        ConversationSourceOwner.Remote(host, epoch)
    }
    else -> error("Unrecognized conversation source owner")
}

fun AndroidApplicationRuntime.conversationSourceOwner(conversationId: String): ConversationSourceOwner =
    decodeConversationSourceOwner(executeQuery(buildJsonObject {
        put("type", "load_conversation_source_owner"); put("conversation_id", conversationId)
    }).jsonObject)

fun AndroidApplicationRuntime.remoteUiEpoch(conversationId: String, expectedHostId: String): String? =
    when (val owner = conversationSourceOwner(conversationId)) {
        ConversationSourceOwner.Local -> null // No imported source admission yet.
        ConversationSourceOwner.Unavailable -> error("Conversation source is unconfirmed")
        is ConversationSourceOwner.Remote -> { require(owner.hostId == expectedHostId); owner.storeEpoch }
    }
