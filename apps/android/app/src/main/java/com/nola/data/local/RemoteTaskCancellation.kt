package com.nola.data.local

import kotlinx.coroutines.CancellationException
import kotlinx.serialization.json.*

/** Monotonic exact-owner cancellation. Saved intent is not an executor stop ACK. */
internal suspend fun requestRemoteTaskCancellation(intent: JsonObject,
    transport: suspend (receiptOnly: Boolean) -> JsonObject): JsonObject {
    val owners = setOf("hostId", "conversationId", "parentRunId", "taskId")
    require(intent.keys == owners)
    owners.forEach { require((intent[it] as? JsonPrimitive)?.let { p -> p.isString && p.content.isNotBlank() } == true) }
    fun validate(value: JsonObject): JsonObject {
        require(value.keys == owners + setOf("cancelRequested", "terminal", "state", "applicationPending"))
        owners.forEach { require(value[it] == intent[it]) { "Task cancellation receipt owner mismatch" } }
        for (field in listOf("cancelRequested", "terminal", "applicationPending")) {
            require((value[field] as? JsonPrimitive)?.takeUnless { it.isString }?.booleanOrNull != null)
        }
        require(value["applicationPending"] == JsonPrimitive(false)) { "Task cancellation has no committed receipt" }
        val state = (value["state"] as? JsonPrimitive)?.takeIf { it.isString }?.content
        val states = if (value["terminal"] == JsonPrimitive(true)) setOf("sealed", "failed", "cancelled")
            else setOf("queued", "running", "retry_waiting", "waiting_permission", "suspended", "awaiting_reconciliation")
        require(state in states) { "Task cancellation state disagrees with terminal receipt" }
        return value
    }
    fun settled(value: JsonObject) = value["cancelRequested"] == JsonPrimitive(true) || value["terminal"] == JsonPrimitive(true)
    val observed = validate(transport(true))
    if (observed["terminal"] == JsonPrimitive(true)) return observed
    return try {
        validate(transport(false)).also { require(settled(it)) { "Task cancellation is not committed" } }
    } catch (cancelled: CancellationException) { throw cancelled }
    catch (failure: Exception) {
        val recovered = validate(transport(true))
        if (recovered["terminal"] != JsonPrimitive(true)) throw failure
        recovered
    }
}
