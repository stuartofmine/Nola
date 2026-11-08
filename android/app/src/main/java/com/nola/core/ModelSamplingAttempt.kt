package com.nola.core

import java.util.UUID
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
