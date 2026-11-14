package com.nola.core

import java.util.UUID
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonPrimitive

internal data class ModelSamplingAttempt(val id: String, val resampling: Boolean)

/** Consume Core's attempt state; never infer it from process lifetime or UI state. */
internal fun modelSamplingAttempt(request: JsonObject, samplingCount: Int): ModelSamplingAttempt {
    require(samplingCount >= 0)
    val started = requireNotNull(request["attempt_started"]?.jsonPrimitive?.booleanOrNull) {
        "Model Host request has no Core attempt state"
    }
    val original = requireNotNull(request["attempt_id"]?.jsonPrimitive?.contentOrNull?.takeIf(String::isNotBlank)) {
        "Model Host request has no Attempt ID"
    }
    val resampling = started || samplingCount > 0
    return ModelSamplingAttempt(if (resampling) UUID.randomUUID().toString() else original, resampling)
}

/** Provider call IDs are scoped by their protocol, not globally unique Store Part IDs. */
internal fun modelToolCallDeltaPayload(event: AgentModelEvent.ToolCallDelta, admittedAttemptId: String): JsonObject {
    require(admittedAttemptId.isNotBlank() && event.partId.isNotBlank())
    return buildJsonObject {
        put("type", "model_tool_call_delta")
        put("part_id", "$admittedAttemptId:${event.partId}")
        put("call_id", event.callId)
        put("capability_id", event.capabilityId)
        put("arguments_delta", event.argumentsDelta)
    }
}
