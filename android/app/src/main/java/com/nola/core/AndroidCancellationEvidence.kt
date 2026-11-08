package com.nola.core

import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.longOrNull

/** Current-process teardown facts are separate from tool success and recovery safety. */
internal class AndroidCancellationEvidence {
    private data class ReturnedEffect(
        val conversation: String,
        val host: String,
        val session: String,
        val run: String,
        val effect: String,
        val request: String,
    )

    private val returned = mutableMapOf<String, ReturnedEffect>()
    private val uncertain = mutableSetOf<String>()

    fun dispatching(request: JsonObject) {
        // A replay/new delivery cannot borrow an earlier operation's return.
        returned.remove(request.strictId("effect_id"))
    }

    fun returnedLocally(request: JsonObject, conversation: String) {
        // Maintenance without a formal Run has no reusable Run stop fact.
        if (request["turn_id"] == null || request["turn_id"] == JsonNull) return
        val identity = ReturnedEffect(
            conversation = conversation,
            host = request.strictId("host_id"),
            session = request.strictId("session_id"),
            run = request.strictId("turn_id"),
            effect = request.strictId("effect_id"),
            request = request.strictId("request_id"),
        )
        returned[identity.effect] = identity
    }

    /** Called only with a target returned by Core after exact owner/OS-proof validation. */
    fun ownerTerminationProven(request: JsonObject, conversation: String) {
        uncertain.remove(request.strictId("effect_id"))
        returnedLocally(request, conversation)
    }

    fun outcomeUnknown(request: JsonObject) {
        val effect = request.strictId("effect_id")
        returned.remove(effect)
        uncertain.add(effect)
    }

    fun requireCancellationProof(request: JsonObject, conversation: String) {
        val payload = request["payload"] as? JsonObject ?: error("Cancel has no payload")
        require(payload.keys == setOf("type", "target_effect_id", "target_not_started")) {
            "Invalid Cancel fields"
        }
        require(payload.strictId("type") == "cancel")
        val target = payload.strictId("target_effect_id")
        require(payload.containsKey("target_not_started")) { "Cancel is missing target_not_started" }
        val proof = payload.getValue("target_not_started")
        if (proof != JsonNull) validateNotStarted(proof as? JsonObject
            ?: error("Cancel target_not_started must be an object or null"), request, conversation, target)
        if (target in uncertain) throw HostEffectCancellationUncertain(target)
        if (proof != JsonNull) return
        val local = returned[target]
        if (local != null && local.conversation == conversation
            && local.host == request.strictId("host_id")
            && local.session == request.strictId("session_id")
            && local.run == request.strictId("turn_id")) return
        throw HostEffectCancellationUncertain(target)
    }

    private fun validateNotStarted(proof: JsonObject, request: JsonObject, conversation: String, target: String) {
        require(proof.keys == setOf(
            "conversation_id", "run_id", "core_session_id", "host_id", "cancellation_effect_id",
            "cancellation_request_id", "target_effect_id", "target_request_id", "target_payload_identity",
            "checkpoint_revision", "revoked_attempts",
        )) { "Invalid cancellation NotStarted fields" }
        require(proof.strictId("conversation_id") == conversation)
        require(proof.strictId("run_id") == request.strictId("turn_id"))
        require(proof.strictId("core_session_id") == request.strictId("session_id"))
        require(proof.strictId("host_id") == request.strictId("host_id"))
        require(proof.strictId("cancellation_effect_id") == request.strictId("effect_id"))
        require(proof.strictId("cancellation_request_id") == request.strictId("request_id"))
        require(proof.strictId("target_effect_id") == target)
        require(target != request.strictId("effect_id"))
        proof.strictId("target_request_id")
        val digest = proof.strictId("target_payload_identity")
        require(digest.length == 64 && digest.all { it in '0'..'9' || it in 'a'..'f' })
        require(proof.positiveInteger("checkpoint_revision") > 0)
        val attempts = proof["revoked_attempts"] as? JsonArray ?: error("Invalid revoked attempts")
        require(attempts.size <= 256)
        val ids = mutableSetOf<String>()
        for (entry in attempts) {
            val attempt = entry as? JsonObject ?: error("Invalid revoked attempt")
            require(attempt.keys == setOf("attempt_id", "revision"))
            require(ids.add(attempt.strictId("attempt_id"))) { "Duplicate revoked attempt" }
            require(attempt.positiveInteger("revision") > 0)
        }
    }
}

private fun JsonObject.strictId(key: String): String {
    val primitive = this[key] as? JsonPrimitive ?: error("Missing cancellation identity: $key")
    require(primitive.isString) { "Cancellation identity must be a string: $key" }
    val value = primitive.content
    require(value.isNotEmpty() && value == value.trim() && value.toByteArray(Charsets.UTF_8).size <= 1024
        && value.none { it.isISOControl() }) { "Invalid cancellation identity: $key" }
    return value
}

private fun JsonObject.positiveInteger(key: String): Long {
    val primitive = this[key] as? JsonPrimitive ?: error("Missing cancellation integer: $key")
    require(!primitive.isString)
    return primitive.longOrNull?.takeIf { it > 0 } ?: error("Invalid cancellation integer: $key")
}
