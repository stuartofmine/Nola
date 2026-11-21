package com.nola.data.repository

import com.nola.data.local.AndroidJournalPresentation
import com.nola.domain.projection.ConversationProjectionV1Decoder
import com.nola.domain.projection.ConversationProjectionWindowV1
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.*

/** A read-only window; it never creates a local writable conversation. */
internal suspend fun readTaskChildDetail(
    parentConversation: String, parentRun: String, taskId: String, childConversation: String,
    before: Long?, remote: Boolean, expertCallId: String? = null,
    expertQuery: (suspend (JsonObject) -> JsonObject)? = null, query: suspend (JsonObject) -> JsonElement,
): ConversationProjectionWindowV1 = withContext(Dispatchers.IO) {
    val request = buildJsonObject {
        put(if (remote) "operation" else "type", if (remote) "task_child_window" else "load_task_child_conversation_window")
        put("parent_conversation_id", parentConversation); put("parent_run_id", parentRun)
        put("task_id", taskId); put("child_conversation_id", childConversation)
        put("before_ordinal", before?.let(::JsonPrimitive) ?: JsonNull)
        put("limit", 32); put("content_byte_limit", 256 * 1024)
    }
    val raw = if (expertCallId != null) {
        val read = checkNotNull(expertQuery)
        val reply = read(buildJsonObject {
            put("type", "child_history"); put("conversation_id", parentConversation)
            put("child_conversation_id", childConversation); put("before_ordinal", request.getValue("before_ordinal"))
            put("limit", 32); put("content_byte_limit", 256 * 1024)
        })
        require(reply["version"] == JsonPrimitive(1) && reply["conversation_id"] == JsonPrimitive(parentConversation) &&
            reply["child_conversation_id"] == JsonPrimitive(childConversation))
        var exchanges = reply.getValue("expert_exchanges").jsonArray
        var offset: Int? = 0
        while (exchanges.none { it.jsonObject["exchange_id"] == JsonPrimitive(taskId) } && offset != null) {
            val start = offset
            val page = read(buildJsonObject {
                put("type", "peer_exchanges"); put("conversation_id", parentConversation)
                put("offset", start); put("limit", 40)
            })
            require(page["version"] == JsonPrimitive(1) && page["conversation_id"] == JsonPrimitive(parentConversation))
            exchanges = page.getValue("exchanges").jsonArray
            offset = page["next_offset"]?.jsonPrimitive?.intOrNull
            require(offset == null || (exchanges.isNotEmpty() && offset == start + exchanges.size))
        }
        val exchange = exchanges.single { it.jsonObject["exchange_id"] == JsonPrimitive(taskId) }.jsonObject
        require(exchange["parent_conversation_id"] == JsonPrimitive(parentConversation) &&
            exchange["source_run_id"] == JsonPrimitive(parentRun) && exchange["source_call_id"] == JsonPrimitive(expertCallId) &&
            exchange["room_id"] == reply["room_id"] && exchange.getValue("target").jsonObject["context_conversation_id"] == JsonPrimitive(childConversation)) {
            "Expert task detail ownership mismatch"
        }
        reply.getValue("window").jsonObject
    } else {
        val window = query(request).jsonObject
        val ownership = window.getValue("projection").jsonObject.getValue("execution_ownership").jsonObject
        require(ownership == buildJsonObject {
            put("kind","task_child"); put("parent_conversation_id",parentConversation)
            put("parent_run_id",parentRun); put("task_id",taskId); put("child_conversation_id",childConversation)
        }) { "Task detail execution owner mismatch" }
        window
    }
    val window = ConversationProjectionV1Decoder.decodeWindow(raw)
    require(window.projection.conversationId == childConversation)
    val presentation = AndroidJournalPresentation { command ->
        runBlocking {
            if (!remote) query(command) else {
                val operation = when (command.getValue("type").jsonPrimitive.content) {
                    "open_assistant_journal_snapshot" -> "open_snapshot"
                    "release_assistant_journal_snapshot" -> "release_snapshot"
                    "read_assistant_journal_parts" -> "parts"
                    "read_assistant_journal_range" -> "range"
                    else -> error("Unsupported task detail journal read")
                }
                query(JsonObject(command.filterKeys { it != "type" } + ("operation" to JsonPrimitive(operation))))
            }
        }
    }
    window.copy(projection = presentation.overlay(window.projection, window.assistantJournalHeads))
}
