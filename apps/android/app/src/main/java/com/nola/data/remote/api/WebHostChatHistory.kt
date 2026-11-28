// SPDX-License-Identifier: Apache-2.0
package com.nola.data.remote.api

import com.nola.data.remote.sse.SseEvent
import com.nola.domain.model.ChatRole
import com.nola.domain.model.ConversationMessage
import com.nola.domain.model.HostAgentToolEvent
import com.nola.domain.model.encodeAgentTraceProjectionForMeta
import com.nola.domain.model.encodeHostToolEventsForMeta
import kotlinx.serialization.json.*

data class WebHostChatHistory(
    val messages: List<ConversationMessage>,
    val active: Boolean,
    val pending: List<SseEvent>,
    val surfaces: List<JsonObject>,
)

internal fun decodeWebHostChatHistory(
    localId: String, remoteId: String, hostId: String, progress: JsonObject, envelope: JsonObject,
): WebHostChatHistory {
    require(progress["source_host_id"] == JsonPrimitive(hostId) && progress["conversationId"] == JsonPrimitive(remoteId))
    require(envelope["source_host_id"] == JsonPrimitive(hostId))
    val history = envelope.getValue("result").jsonObject
    require(history["conversationId"] == JsonPrimitive(remoteId))
    val rows = history.getValue("messages").jsonArray.map { it.jsonObject }
    val messages = rows.map { row ->
        val tools = (row["toolCalls"] as? JsonArray).orEmpty().mapNotNull { item ->
            val tool = item as? JsonObject ?: return@mapNotNull null
            val id = (tool["callId"] ?: tool["call_id"] ?: tool["id"])?.jsonPrimitive?.contentOrNull ?: return@mapNotNull null
            HostAgentToolEvent(id = id, toolName = (tool["name"] ?: tool["tool"])?.jsonPrimitive?.contentOrNull.orEmpty(),
                status = when (tool["ok"]?.jsonPrimitive?.booleanOrNull) {
                    true -> "done"
                    false -> "error"
                    null -> tool["status"]?.jsonPrimitive?.contentOrNull ?: "done"
                },
                resultSummary = tool["summary"]?.jsonPrimitive?.contentOrNull)
        }
        val meta = buildMap {
            put("nola.v2Seq", row.getValue("seq").jsonPrimitive.long.toString())
            putAll(encodeAgentTraceProjectionForMeta(row["reasoning"]?.jsonPrimitive?.contentOrNull.orEmpty(), null))
            putAll(encodeHostToolEventsForMeta(tools).orEmpty())
        }
        ConversationMessage(
            id = row.getValue("id").jsonPrimitive.content, conversationId = localId,
            role = ChatRole.valueOf(row.getValue("role").jsonPrimitive.content),
            text = row.getValue("content").jsonPrimitive.content, createdAt = row.getValue("createdAtMs").jsonPrimitive.long, meta = meta,
        )
    }
    val pending = buildList {
        (progress["pendingPermission"] as? JsonObject)?.let {
            add(SseEvent.V2PermissionRequested(it.getValue("requestId").jsonPrimitive.content,
                it.getValue("tool").jsonPrimitive.content, it["reason"]?.jsonPrimitive?.contentOrNull.orEmpty(), it["arguments"]))
        }
        (progress["pendingQuestion"] as? JsonObject)?.let {
            add(SseEvent.V2UserQuestion(it.getValue("requestId").jsonPrimitive.content,
                it.getValue("question").jsonPrimitive.content, (it["options"] as? JsonArray).orEmpty().map { option -> option.jsonPrimitive.content }))
        }
    }
    return WebHostChatHistory(messages, progress.getValue("active").jsonPrimitive.boolean, pending,
        rows.flatMap { (it["uiSurfaces"] as? JsonArray).orEmpty().mapNotNull { item -> item as? JsonObject } })
}
