package com.nola.data.local

import kotlinx.serialization.json.*

/** Durable client intent only. Acceptance/delivery remains a Core receipt. */
internal suspend fun submitRecoverableTaskGuidance(
    proposed: JsonObject,
    read: () -> String?,
    write: (String?) -> Unit,
    transport: suspend (JsonObject, Boolean) -> JsonObject,
): String {
    val saved = read()
    val intent = saved?.let { Json.parseToJsonElement(it).jsonObject } ?: proposed
    for (key in listOf("hostId", "conversationId", "parentRunId", "taskId"))
        require(intent[key] == proposed[key]) { "Stored guidance belongs to another owner" }
    require(saved == null || intent["message"] == proposed["message"]) { "An earlier instruction is unconfirmed; retry its original text first" }
    if (saved == null) write(intent.toString())
    fun accept(response: JsonObject): String? {
        for (key in listOf("hostId", "conversationId", "taskId", "messageId"))
            require(response[key] == intent[key]) { "Guidance receipt owner changed" }
        val value = response.getValue("guidance")
        if (value == JsonNull) return null
        val record = value.jsonObject
        for ((wire, local) in listOf("task_id" to "taskId", "message_id" to "messageId",
            "owner_session_id" to "conversationId", "content" to "message", "submitted_at_ms" to "submittedAtMs"))
            require(record[wire] == intent[local]) { "Guidance receipt input changed" }
        return record.getValue("status").jsonPrimitive.content.also {
            require(it in setOf("submitted", "accepted", "delivered", "rejected"))
            write(null)
        }
    }
    if (saved != null) {
        accept(transport(intent, true))?.let { return it }
    }
    return requireNotNull(accept(transport(intent, false))) { "Submitted guidance has no receipt" }
}
