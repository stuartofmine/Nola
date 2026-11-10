package com.nola.data.local

import kotlinx.serialization.json.*

/** Validate transport observations before allowing a pending intent to retire. */
internal object ApplicationReceiptValidation {
    const val MAX_FRAME_BYTES = 16 * 1024 * 1024

    fun recovered(request: JsonObject, receipt: JsonObject): JsonObject {
        try {
            validate(request, receipt)
        } catch (pending: ApplicationSubmissionJournal.PendingSubmission) {
            // Startup retries isolate this owner just like an uncertain direct response.
            throw AndroidApplicationRuntime.PendingMutation(pending.requestId)
        }
        return receipt
    }

    fun decode(request: JsonObject, response: ByteArray?, readOnly: Boolean = false): JsonObject {
        fun pending(): Nothing = throw ApplicationSubmissionJournal.PendingSubmission(
            request.getValue("request_id").jsonPrimitive.content, "No validated Application receipt")
        if (response == null || response.size > MAX_FRAME_BYTES) pending()
        val outer = try { Json.parseToJsonElement(response.decodeToString(throwOnInvalidSequence = true)) as? JsonObject }
            catch (_: Exception) { null } ?: pending()
        val ok = outer["ok"] as? JsonPrimitive
        // Even a nonrecoverable outer failure has no durable command identity.
        if (readOnly && ok?.isString == false && ok.booleanOrNull == false) {
            val error = outer["error"] as? JsonObject
            val message = (error?.get("message") as? JsonPrimitive)?.takeIf { it.isString }?.content
            if (!message.isNullOrBlank()) throw AndroidApplicationRuntime.RejectedMutation(message)
        }
        if (ok == null || ok.isString || ok.booleanOrNull != true) pending()
        val receipt = outer["result"] as? JsonObject ?: pending()
        validate(request, receipt)
        return receipt
    }

    fun validate(request: JsonObject, receipt: JsonObject) {
        fun pending(): Nothing = throw ApplicationSubmissionJournal.PendingSubmission(
            request.getValue("request_id").jsonPrimitive.content, "Invalid Application receipt")
        val common = setOf("protocol_version", "client_id", "request_id", "received_at", "status")
        if (listOf("client_id", "request_id", "protocol_version").any { receipt[it] != request[it] }
            || !text(receipt["client_id"]) || !text(receipt["request_id"])
            || unsigned(receipt["protocol_version"]) == null || !text(receipt["received_at"])) pending()
        val status = (receipt["status"] as? JsonPrimitive)?.takeIf { it.isString }?.content ?: pending()
        val allowed = when (status) {
            "accepted" -> common
            "committed" -> {
                if (!text(receipt["completed_at"]) || !receipt.containsKey("result")) pending()
                receipt["committed_revision"]?.let { if (unsigned(it)?.let { value -> value > 0uL } != true) pending() }
                common + setOf("completed_at", "committed_revision", "result")
            }
            "rejected" -> {
                val error = receipt["error"] as? JsonObject ?: pending()
                val code = (error["code"] as? JsonPrimitive)?.takeIf { it.isString }?.content ?: pending()
                val recoverable = error["recoverable"] as? JsonPrimitive ?: pending()
                if (!text(receipt["completed_at"]) || !Regex("[a-z][a-z0-9_]*").matches(code)
                    || !text(error["message"]) || recoverable.isString || recoverable.booleanOrNull == null
                    || error.keys.any { it !in setOf("code", "message", "recoverable", "details") }
                    || (error.containsKey("details") && error["details"] !is JsonObject)) pending()
                common + setOf("completed_at", "error")
            }
            "conflict" -> {
                val conflict = receipt["conflict"] as? JsonObject ?: pending()
                val expected = unsigned(conflict["expected_revision"]) ?: pending()
                val actual = unsigned(conflict["actual_revision"]) ?: pending()
                if (!text(receipt["completed_at"]) || !text(conflict["scope_id"]) || expected == actual
                    || conflict.keys != setOf("scope_id", "expected_revision", "actual_revision")) pending()
                common + setOf("completed_at", "conflict")
            }
            else -> pending()
        }
        if (receipt.keys.any { it !in allowed }) pending()
    }

    private fun text(value: JsonElement?): Boolean = (value as? JsonPrimitive)?.let {
        it.isString && it.content.isNotBlank() && it.content.none(Char::isISOControl)
    } == true

    private fun unsigned(value: JsonElement?): ULong? = (value as? JsonPrimitive)
        ?.takeUnless { it.isString }?.content?.toULongOrNull()
}
