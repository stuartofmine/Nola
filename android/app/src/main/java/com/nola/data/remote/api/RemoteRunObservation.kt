package com.nola.data.remote.api

import kotlinx.serialization.json.*

data class RemoteRunKey(val conversationId: String, val runId: String, val assistantMessageId: String)

enum class RemoteRunRecovery { Observe, ResumeRequired, Terminal, Inconsistent }

data class RemoteRunObservation(
    val key: RemoteRunKey,
    val status: String,
    val workerState: String,
    val workerGeneration: String?,
    val recovery: RemoteRunRecovery,
    val admission: JsonObject?,
    val terminalSeal: JsonObject?,
    val permissions: List<JsonObject>,
    val canSubmit: Boolean,
    val submissionReceipt: JsonObject?,
    val contextUsage: JsonObject? = null,
)

private fun JsonObject.requiredText(key: String): String = getValue(key).jsonPrimitive.let {
    require(it.isString && it.content.isNotBlank()) { "Invalid remote observation $key" }
    it.content
}

fun remoteObservationRequest(hostId: String, clientId: String, conversationId: String,
    key: RemoteRunKey? = null): JsonObject = buildJsonObject {
    require(hostId.isNotBlank() && clientId.isNotBlank() && conversationId.isNotBlank())
    require(key == null || key.conversationId == conversationId)
    put("operation", if (key == null) "discover" else "observe")
    put("conversation_id", conversationId); put("expected_host_id", hostId); put("client_id", clientId)
    key?.let { put("expected_run_id", it.runId); put("assistant_message_id", it.assistantMessageId) }
}

fun remoteResumeRequest(hostId: String, clientId: String, key: RemoteRunKey): JsonObject = buildJsonObject {
    require(hostId.isNotBlank() && clientId.isNotBlank() && key.runId.isNotBlank())
    put("conversation_id", key.conversationId); put("expected_run_id", key.runId)
    put("expected_host_id", hostId); put("client_id", clientId)
}

/** Only authenticated discovery may establish absence. An exact observer
 * never changes Run/root identity or uses worker absence as execution authority. */
fun decodeRemoteRunObservation(response: JsonObject, hostId: String, clientId: String, conversationId: String,
    expected: RemoteRunKey? = null): RemoteRunObservation? {
    require(response.requiredText("source_host_id") == hostId
        && response.requiredText("conversation_id") == conversationId
        && response.requiredText("client_id") == clientId) { "Remote observation envelope owner mismatch" }
    val raw = response.getValue("result")
    if (raw == JsonNull) {
        require(expected == null) { "Exact remote Run observation disappeared" }
        return null
    }
    val value = raw.jsonObject
    require(value.requiredText("client_id") == clientId) { "Remote observation client mismatch" }
    val version = value.getValue("protocol_version").jsonPrimitive
    require(!version.isString && version.int == 1)
    val key = RemoteRunKey(value.requiredText("conversation_id"), value.requiredText("run_id"),
        value.requiredText("assistant_message_id"))
    require(key.conversationId == conversationId && (expected == null || key == expected))
    val worker = value.getValue("worker").jsonObject
    val workerState = worker.requiredText("state")
    require(workerState in setOf("running", "absent", "starting", "unknown"))
    val generation = worker.getValue("generation").takeUnless { it == JsonNull }?.jsonPrimitive?.let {
        require(it.isString && it.content.isNotBlank()); it.content
    }
    val recovery = when (value.requiredText("recovery")) {
        "observe" -> RemoteRunRecovery.Observe
        "resume_required" -> RemoteRunRecovery.ResumeRequired
        "terminal" -> RemoteRunRecovery.Terminal
        "inconsistent" -> RemoteRunRecovery.Inconsistent
        else -> error("Unknown remote recovery state")
    }
    val status = value.requiredText("status")
    require(status in setOf("pending", "running", "waiting_for_permission", "waiting_for_input", "cancellation_requested", "cancel_requested",
        "sealed"))
    fun evidence(field: String): JsonObject? = value.getValue(field).takeUnless { it == JsonNull }?.jsonObject?.also {
        val identity = it.getValue("identity").jsonObject
        require(identity.requiredText("conversation_id") == key.conversationId && identity.requiredText("run_id") == key.runId
            && identity.requiredText("assistant_message_id") == key.assistantMessageId)
        identity.requiredText("source_store_epoch")
    }
    val admission = evidence("admission")
    val seal = evidence("terminal_seal")
    val contextUsage = evidence("context_usage")
    require(contextUsage == null || contextUsage["identity"] == admission?.get("identity"))
    require(admission == null || seal == null || admission["identity"] == seal["identity"])
    val canSubmit = value.getValue("can_submit").jsonPrimitive.boolean
    require(!canSubmit || (recovery == RemoteRunRecovery.Terminal && seal != null))
    require(recovery != RemoteRunRecovery.Terminal || (status == "sealed" && seal != null))
    if (seal != null) {
        require(status == "sealed" && recovery == RemoteRunRecovery.Terminal)
        require(seal.getValue("protocol_version").jsonPrimitive.content == "1")
        val result = seal.getValue("result").jsonObject
        require(result.requiredText("conversation_id") == key.conversationId &&
            result.requiredText("run_id") == key.runId)
    }
    val permissions = value.getValue("permissions").jsonArray.map { it.jsonObject }
    require(permissions.size <= 64)
    require(!canSubmit || permissions.isEmpty())
    require(recovery != RemoteRunRecovery.ResumeRequired || workerState == "absent")
    val receipt = value.getValue("submission_receipt").takeUnless { it == JsonNull }?.jsonObject
    receipt?.let { decodeObservedSubmissionReceipt(it, hostId, clientId, key) }
    return RemoteRunObservation(key, status, workerState, generation, recovery, admission, seal, permissions, canSubmit, receipt, contextUsage)
}
