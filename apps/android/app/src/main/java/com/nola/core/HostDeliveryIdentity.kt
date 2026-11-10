package com.nola.core

import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.longOrNull

/** Delivery leases fence execution; the independent model attempt may be absent. */
internal fun validatedHostDelivery(request: JsonObject, nowMs: Long? = null): JsonObject {
    request["attempt_id"]?.takeUnless { it == JsonNull }?.let { attempt ->
        require(attempt is JsonPrimitive && attempt.isString && !attempt.content.isBlank()) {
            "Host request has an invalid model Attempt ID"
        }
    }
    val delivery = request["delivery"] as? JsonObject
        ?: error("Host request has no effect-delivery lease")
    validateHostDelivery(delivery, nowMs)
    return delivery
}

internal fun validateHostDelivery(delivery: JsonObject, nowMs: Long? = null) {
    for (key in listOf("attempt_id", "worker_generation", "lease_owner", "lease_token")) {
        val value = delivery[key] as? JsonPrimitive
        require(value?.isString == true && !value.contentOrNull.isNullOrBlank()) {
            "Host-effect delivery has no $key"
        }
    }
    val expiry = delivery["lease_expires_at_ms"] as? JsonPrimitive
    require(expiry != null && !expiry.isString && (expiry.longOrNull ?: 0L) > 0L) {
        "Host-effect delivery has no valid lease_expires_at_ms"
    }
    if (nowMs != null) {
        require(nowMs >= 0L && expiry.longOrNull!! > nowMs) {
            "Host-effect delivery lease has expired"
        }
    }
}
