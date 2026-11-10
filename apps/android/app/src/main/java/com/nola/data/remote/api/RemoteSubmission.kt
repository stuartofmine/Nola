package com.nola.data.remote.api

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.*

@Serializable
data class RemoteSubmissionIdentity(
    @SerialName("source_host_id") val sourceHostId: String,
    @SerialName("client_id") val clientId: String,
    @SerialName("conversation_id") val conversationId: String,
    @SerialName("submission_id") val submissionId: String,
    @SerialName("user_message_id") val userMessageId: String,
    @SerialName("assistant_message_id") val assistantMessageId: String,
) {
    fun wire(): JsonObject = Json.parseToJsonElement(Json.encodeToString(this)).jsonObject
    fun validate() {
        require(listOf(sourceHostId, clientId, conversationId, submissionId, userMessageId, assistantMessageId)
            .all { it.isNotBlank() && it.trim() == it && it.length <= 512 })
        require(userMessageId != assistantMessageId)
    }
}

enum class RemoteSubmissionPhase { Prepared, Bound, Admitted, Rejected, Cancelled }

data class RemoteSubmissionIntent(val identity: RemoteSubmissionIdentity, val requestDigest: String?, val phase: RemoteSubmissionPhase)

data class RemoteSubmissionReceipt(val identity: RemoteSubmissionIdentity, val requestDigest: String,
    val phase: RemoteSubmissionPhase, val admission: JsonObject?, val rejectionCode: String?, val wire: JsonObject)

private fun JsonObject.submissionText(field: String): String = getValue(field).jsonPrimitive.let {
    require(it.isString && it.content.isNotBlank()); it.content
}
private fun submissionPhase(value: String) = when (value) {
    "prepared" -> RemoteSubmissionPhase.Prepared
    "bound" -> RemoteSubmissionPhase.Bound
    "admitted" -> RemoteSubmissionPhase.Admitted
    "rejected" -> RemoteSubmissionPhase.Rejected
    "cancelled" -> RemoteSubmissionPhase.Cancelled
    else -> error("Invalid remote submission phase")
}
private fun submissionIdentity(value: JsonObject): RemoteSubmissionIdentity {
    require(value.keys == setOf("source_host_id", "client_id", "conversation_id", "submission_id", "user_message_id", "assistant_message_id"))
    return RemoteSubmissionIdentity(
        value.submissionText("source_host_id"), value.submissionText("client_id"), value.submissionText("conversation_id"),
        value.submissionText("submission_id"), value.submissionText("user_message_id"), value.submissionText("assistant_message_id"),
    ).also { it.validate() }
}
private fun requireSubmissionDigest(value: String) { require(value.matches(Regex("[0-9a-f]{64}"))) }

fun decodeRemoteSubmissionIntent(value: JsonObject): RemoteSubmissionIntent {
    val identity = submissionIdentity(value.getValue("identity").jsonObject)
    val digest = value.getValue("request_digest").takeUnless { it == JsonNull }?.jsonPrimitive?.let {
        require(it.isString); it.content.also(::requireSubmissionDigest)
    }
    val phase = submissionPhase(value.submissionText("phase"))
    require(phase != RemoteSubmissionPhase.Prepared || digest == null)
    require(phase !in setOf(RemoteSubmissionPhase.Bound, RemoteSubmissionPhase.Admitted, RemoteSubmissionPhase.Rejected) || digest != null)
    return RemoteSubmissionIntent(identity, digest, phase)
}

fun decodeRemoteSubmissionReceipt(value: JsonObject, identity: RemoteSubmissionIdentity,
    requestDigest: String): RemoteSubmissionReceipt {
    val version = value.getValue("protocol_version").jsonPrimitive
    require(!version.isString && version.int == 1)
    require(submissionIdentity(value.getValue("identity").jsonObject) == identity)
    val digest = value.submissionText("request_digest").also(::requireSubmissionDigest)
    require(digest == requestDigest)
    val phase = submissionPhase(value.submissionText("phase"))
    require(phase != RemoteSubmissionPhase.Bound)
    val admission = value.getValue("admission").takeUnless { it == JsonNull }?.jsonObject
    if (phase == RemoteSubmissionPhase.Admitted) {
        val owner = checkNotNull(admission).getValue("identity").jsonObject
        require(owner.submissionText("conversation_id") == identity.conversationId && owner.submissionText("run_id") == identity.submissionId
            && owner.submissionText("assistant_message_id") == identity.assistantMessageId)
        owner.submissionText("source_store_epoch")
        val sequence = admission.submissionText("sequence")
        require(sequence.matches(Regex("[1-9][0-9]{0,19}")) && sequence.toULongOrNull() != null)
        requireSubmissionDigest(admission.submissionText("record_digest"))
    } else require(admission == null)
    val rejection = value["rejection_code"]?.takeUnless { it == JsonNull }?.jsonPrimitive?.content
    require(rejection == null || rejection.matches(Regex("[a-z0-9_]{1,128}")))
    return RemoteSubmissionReceipt(identity, digest, phase, admission, rejection, value)
}

data class FrozenRemoteSubmission(val identity: RemoteSubmissionIdentity, val requestDigest: String, val request: JsonObject) {
    init {
        identity.validate(); requireSubmissionDigest(requestDigest)
        require(!request.containsKey("request_digest") && !request.containsKey("session_id"))
        require(request["client_id"]?.jsonPrimitive?.content == identity.clientId && request["expected_host_id"]?.jsonPrimitive?.content == identity.sourceHostId)
        require(request["submission_id"]?.jsonPrimitive?.content == identity.submissionId && request["user_message_id"]?.jsonPrimitive?.content == identity.userMessageId
            && request["assistant_message_id"]?.jsonPrimitive?.content == identity.assistantMessageId)
        require(request["conversationId"]?.jsonPrimitive?.content == identity.conversationId)
    }
    fun wireRequest() = JsonObject(request + ("request_digest" to JsonPrimitive(requestDigest)))
    fun reconciliationRequest(cancel: Boolean = false) = remoteSubmissionReconciliationRequest(identity, requestDigest, cancel)
}

/** Receipt reconciliation carries identity only, never rereads the frozen
 * model input. The full request is loaded only for an explicit original send. */
fun remoteSubmissionReconciliationRequest(identity: RemoteSubmissionIdentity, requestDigest: String,
    cancel: Boolean = false): JsonObject {
    identity.validate(); requireSubmissionDigest(requestDigest)
    return buildJsonObject {
        put("operation", if (cancel) "cancel" else "observe"); put("identity", identity.wire()); put("request_digest", requestDigest)
    }
}

fun decodeObservedSubmissionReceipt(value: JsonObject, hostId: String, clientId: String,
    key: RemoteRunKey): RemoteSubmissionReceipt {
    val identity = submissionIdentity(value.getValue("identity").jsonObject)
    require(identity.sourceHostId == hostId && identity.clientId == clientId && identity.conversationId == key.conversationId
        && identity.submissionId == key.runId && identity.assistantMessageId == key.assistantMessageId)
    val receipt = decodeRemoteSubmissionReceipt(value, identity, value.submissionText("request_digest"))
    require(receipt.phase == RemoteSubmissionPhase.Admitted)
    return receipt
}

class RemoteSubmissionUnresolved(val submission: FrozenRemoteSubmission, cause: Throwable? = null) :
    IllegalStateException("Remote submission outcome unresolved: ${submission.identity.submissionId}", cause)
