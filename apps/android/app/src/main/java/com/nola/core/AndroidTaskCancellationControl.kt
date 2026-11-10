package com.nola.core

import kotlinx.serialization.json.*

/** Transport adapter only: Core owns exact control/fence and durable intent. */
internal fun executeAndroidTaskCancellation(
    request: JsonObject,
    delivery: JsonElement,
    nowMs: Long,
    execute: (JsonObject) -> JsonElement,
): JsonObject {
    val payload = request.getValue("payload").jsonObject
    require(payload["type"] == JsonPrimitive("cancel_task"))
    val taskId = payload.getValue("task_id").jsonPrimitive.content
    val result = execute(buildJsonObject {
        put("type", "execute_task_cancellation_for_effect")
        put("request", JsonObject(request + ("delivery" to delivery)))
        put("now_ms", nowMs)
    }).jsonObject
    return when (result["kind"]?.jsonPrimitive?.content) {
        "applied" -> {
            require(result.keys == setOf("kind", "cancel_requested") && (result["cancel_requested"] as? JsonPrimitive)?.takeUnless { it.isString }?.booleanOrNull != null) {
                "Core did not confirm durable task cancellation intent"
            }
            buildJsonObject {
                put("type", "completed")
                put("output", buildJsonObject { put("task_id", taskId); put("cancel_requested", result.getValue("cancel_requested")) })
                put("usage", JsonNull); put("full_output_ref", JsonNull); put("artifacts", JsonArray(emptyList()))
            }
        }
        "transfer" -> buildJsonObject {
            put("type", "failed")
            put("error", buildJsonObject {
                put("code", "task_cancellation_transfer_unsupported")
                put("message", "This mobile Host cannot transfer task cancellation to a different execution Store.")
                put("retryable", false)
            })
        }
        else -> error("Invalid Core task cancellation response")
    }
}
