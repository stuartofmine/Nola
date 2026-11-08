package com.nola.core

import java.util.ArrayDeque
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.longOrNull

/** Process-local mirror of Core Host-effect identity and replay sequencing. */
internal class DeviceHostExactlyOnceCoordinator(
    private val sessionId: String,
    private val hostId: String,
) {
    private val pending = ArrayDeque<JsonObject>()
    private val requestByEffect = mutableMapOf<String, JsonObject>()
    private val effectByRequest = mutableMapOf<String, String>()
    private val completedEffects = mutableSetOf<String>()
    private val responseSequences = mutableMapOf<String, Long>()
    private val responseAttempts = mutableMapOf<String, String>()
    private val responseDeliveries = mutableMapOf<String, JsonObject>()
    private var terminal = false

    val pendingCount: Int get() = pending.size
    val isEmpty: Boolean get() = pending.isEmpty() && requestByEffect.isEmpty()

    fun restore(recovery: JsonObject): List<JsonObject> {
        check(recovery.boolean("ok") == true) {
            recovery.string("error") ?: "Core pending Host request recovery failed"
        }
        return (recovery["host_requests"] as? JsonArray).orEmpty().mapNotNull { element ->
            val request = element as? JsonObject
                ?: error("Core recovery returned a non-object Host request")
            request.takeIf(::accept)
        }
    }

    fun accept(request: JsonObject): Boolean {
        validate(request)
        if (terminal) return false
        val effectId = request.requiredString("effect_id")
        val requestId = request.requiredString("request_id")
        if (effectId in completedEffects) return false
        effectByRequest[requestId]?.let { existingEffect ->
            require(existingEffect == effectId) {
                "Host request ID $requestId is already bound to effect $existingEffect"
            }
        }
        requestByEffect[effectId]?.let { existing ->
            require(existing == request) {
                "Host effect $effectId was rebound without a recovery boundary"
            }
            return false
        }
        effectByRequest[requestId] = effectId
        requestByEffect[effectId] = request
        responseDeliveries[effectId] = request["delivery"] as JsonObject
        pending.addLast(request)
        return true
    }

    fun poll(): JsonObject? = pending.pollFirst()

    fun nextSequence(request: JsonObject): Long {
        validate(request)
        val effectId = request.requiredString("effect_id")
        require(requestByEffect[effectId] == request) {
            "Host response does not match the active request for effect $effectId"
        }
        require(effectId !in completedEffects && !terminal) {
            "Host effect $effectId is already terminal"
        }
        val resumeAfter = request.long("resume_after_sequence") ?: 0L
        val next = maxOf(responseSequences[effectId] ?: 0L, resumeAfter) + 1L
        responseSequences[effectId] = next
        return next
    }

    fun bindResponseAttempt(request: JsonObject, attemptId: String) {
        validate(request)
        require(attemptId.isNotBlank()) { "Host response Attempt ID is empty" }
        val effectId = request.requiredString("effect_id")
        require(requestByEffect[effectId] == request && effectId !in completedEffects && !terminal) {
            "Host response does not match the active request for effect $effectId"
        }
        responseAttempts[effectId] = attemptId
    }

    fun responseAttemptId(request: JsonObject): String? {
        validate(request)
        return responseAttempts[request.requiredString("effect_id")]
    }

    fun bindResponseDelivery(request: JsonObject, delivery: JsonObject) {
        validate(request)
        val effectId = request.requiredString("effect_id")
        require(requestByEffect[effectId] == request && effectId !in completedEffects && !terminal) {
            "Host response does not match the active request for effect $effectId"
        }
        validateHostDelivery(delivery)
        val current = responseDeliveries[effectId] ?: error("Host effect has no active delivery lease")
        for (key in listOf("attempt_id", "worker_generation", "lease_owner", "lease_token")) {
            require(delivery[key] == current[key]) { "Renewed Host delivery changed $key" }
        }
        require(delivery.long("lease_expires_at_ms")!! >= current.long("lease_expires_at_ms")!!) {
            "Renewed Host delivery moved its expiry backwards"
        }
        responseDeliveries[effectId] = delivery
    }

    fun responseDelivery(request: JsonObject): JsonObject {
        validate(request)
        return responseDeliveries[request.requiredString("effect_id")]
            ?: error("Host effect has no active delivery lease")
    }

    fun completeEffect(effectId: String) {
        require(requestByEffect.containsKey(effectId)) { "Unknown Host effect $effectId" }
        completedEffects += effectId
        responseAttempts.remove(effectId)
        responseDeliveries.remove(effectId)
        requestByEffect.remove(effectId)?.string("request_id")?.let(effectByRequest::remove)
        pending.removeAll { it.string("effect_id") == effectId }
    }

    fun finishTurn() {
        terminal = true
        pending.clear()
        requestByEffect.clear()
        effectByRequest.clear()
        completedEffects.clear()
        responseSequences.clear()
        responseAttempts.clear()
        responseDeliveries.clear()
    }

    private fun validate(request: JsonObject) {
        val requestSessionId = request.requiredString("session_id")
        require(requestSessionId == sessionId) {
            "Host request session $requestSessionId does not match $sessionId"
        }
        val requestHostId = request.requiredString("host_id")
        require(requestHostId == hostId) {
            "Host request Host $requestHostId does not match $hostId"
        }
        request.requiredString("request_id")
        request.requiredString("effect_id")
        validatedHostDelivery(request)
        require(request["payload"] is JsonObject) { "Host request has no payload" }
    }

    private fun JsonObject.requiredString(key: String): String = string(key)
        ?.takeIf(String::isNotBlank)
        ?: error("Host request has no $key")

    private fun JsonObject.string(key: String): String? =
        (this[key] as? JsonPrimitive)?.contentOrNull

    private fun JsonObject.long(key: String): Long? =
        (this[key] as? JsonPrimitive)?.longOrNull

    private fun JsonObject.boolean(key: String): Boolean? =
        (this[key] as? JsonPrimitive)?.contentOrNull?.toBooleanStrictOrNull()
}
