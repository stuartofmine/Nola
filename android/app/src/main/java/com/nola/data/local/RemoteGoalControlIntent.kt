package com.nola.data.local

import com.nola.data.repository.ConversationProgress
import kotlinx.serialization.json.*

/** The optional identity/time fields are nullable; Host String/Vec inputs never are. */
internal fun remoteGoalControlInput(operationId: String, action: String, goalId: String?, token: String?,
    objective: String = "", constraints: List<String> = emptyList(), wakeAtEpochMs: Long? = null): JsonObject = buildJsonObject {
    require(operationId.isNotBlank() && action in setOf("start", "pause", "resume", "cancel", "schedule"))
    put("operationId", operationId); put("action", action)
    put("expectedGoalId", goalId?.let(::JsonPrimitive) ?: JsonNull)
    put("expectedControlToken", token?.let(::JsonPrimitive) ?: JsonNull)
    put("objective", objective); put("constraints", JsonArray(constraints.map(::JsonPrimitive)))
    put("wakeAtEpochMs", wakeAtEpochMs?.let(::JsonPrimitive) ?: JsonNull)
}

/** Persists transport intent, never derives or owns Goal state. */
internal suspend fun submitRecoverableGoalControl(
    proposed: JsonObject?, hostId: String, conversationId: String,
    read: () -> String?, write: (String?) -> Unit,
    observe: suspend () -> ConversationProgress,
    transport: suspend (JsonObject) -> JsonObject,
): ConversationProgress {
    val saved = read()
    val intent = saved?.let { Json.parseToJsonElement(it).jsonObject } ?: requireNotNull(proposed)
    require(intent.keys == setOf("expected_host_id", "conversation_id", "control"))
    require(hostId.isNotBlank() && conversationId.isNotBlank())
    require(intent["expected_host_id"] == JsonPrimitive(hostId) &&
        intent["conversation_id"] == JsonPrimitive(conversationId)) { "Goal intent owner changed" }
    require(saved == null || proposed == null || intent == proposed) { "An earlier Goal operation is unconfirmed" }
    val operationId = intent.getValue("control").jsonObject.getValue("operationId").jsonPrimitive.content
    require(operationId.isNotBlank())
    if (saved == null) write(intent.toString())
    fun settled(progress: ConversationProgress): Boolean {
        require(progress.conversationId == conversationId)
        val receipt = progress.goalControl?.appliedOperations?.get(operationId) ?: return false
        write(null)
        receipt.rejection?.let { error(it) }
        return true
    }
    if (saved != null) {
        val current = observe()
        if (settled(current)) return current
    }
    val response = transport(intent)
    require(response["source_host_id"] == JsonPrimitive(hostId) &&
        response["conversationId"] == JsonPrimitive(conversationId) &&
        response["operationId"] == JsonPrimitive(operationId)) { "Goal response owner changed" }
    val authoritative = response["authoritative"]?.jsonPrimitive?.booleanOrNull
        ?: error("Goal response lacks authority")
    require(authoritative || response["controlPending"]?.jsonPrimitive?.booleanOrNull == true) {
        "Goal response is neither authoritative nor pending"
    }
    // Even an HTTP success/202 does not settle intent: only the Core operation ledger does.
    val progress = observe()
    settled(progress)
    return progress
}
