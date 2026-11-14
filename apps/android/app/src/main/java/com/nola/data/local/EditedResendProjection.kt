// SPDX-License-Identifier: Apache-2.0
package com.nola.data.local

import kotlinx.serialization.Serializable
import kotlinx.serialization.SerialName
import kotlinx.serialization.json.*

/** Closed current-version intent. No previous workspace, history or grants. */
@Serializable
data class AndroidPrepareEditedResend(
    @SerialName("operation_id") val operationId: String,
    @SerialName("source_conversation_id") val sourceConversationId: String,
    @SerialName("source_message_id") val sourceMessageId: String,
    @SerialName("expected_source_revision") val expectedSourceRevision: Long,
    @SerialName("expected_source_message_revision") val expectedSourceMessageRevision: Long,
    @SerialName("edited_text") val editedText: String,
    @SerialName("target_conversation_id") val targetConversationId: String,
    val title: String,
    @SerialName("created_at_ms") val createdAtMs: Long,
)

@Serializable
data class AndroidFrozenResendModel(
    val version: Int,
    val primary: JsonObject,
    val fallbacks: JsonArray,
    val generation: JsonObject,
    @SerialName("output_admission") val outputAdmission: JsonObject,
    @SerialName("model_capability") val modelCapability: JsonObject,
)

@Serializable
data class AndroidMessageSendWitness(
    @SerialName("conversation_id") val conversationId: String,
    @SerialName("message_id") val messageId: String,
    @SerialName("run_id") val runId: String,
    @SerialName("assistant_message_id") val assistantMessageId: String,
    @SerialName("body_sha256") val bodySha256: String,
    @SerialName("invocation_config_sha256") val invocationConfigSha256: String,
    val model: AndroidFrozenResendModel,
)

@Serializable
data class AndroidPreparedEditedResend(
    val input: AndroidPrepareEditedResend,
    val source: AndroidMessageSendWitness,
    @SerialName("user_message_id") val userMessageId: String,
    @SerialName("assistant_message_id") val assistantMessageId: String,
    @SerialName("run_id") val runId: String,
    @SerialName("admitted_run_id") val admittedRunId: String?,
)

internal fun AndroidApplicationRuntime.loadPreparedEditedResend(target: String): AndroidPreparedEditedResend? {
    val result = executeQuery(buildJsonObject { put("type", "load_prepared_edited_resend"); put("conversation_id", target) })
    return if (result == JsonNull) null else Json.decodeFromJsonElement<AndroidPreparedEditedResend>(result)
}

internal fun AndroidApplicationRuntime.prepareEditedResend(input: AndroidPrepareEditedResend): AndroidPreparedEditedResend =
    Json.decodeFromJsonElement(execute(buildJsonObject {
        put("type", "prepare_edited_resend"); put("input", Json.encodeToJsonElement(input))
    }))

/** Stable IDs and owner witnesses are checked again at the native admission boundary. */
internal fun AndroidPreparedEditedResend.requireIdentity() {
    require(input.operationId == runId && input.targetConversationId != input.sourceConversationId &&
        userMessageId == "$runId:user" && assistantMessageId == "$runId:assistant" &&
        source.conversationId == input.sourceConversationId && source.messageId == input.sourceMessageId &&
        source.runId != runId && (admittedRunId == null || admittedRunId == runId)) {
        "Edited resend owner witness changed"
    }
    java.util.UUID.fromString(runId)
    java.util.UUID.fromString(input.targetConversationId)
}

@Serializable
data class AndroidPrepareInPlaceResend(
    @SerialName("operation_id") val operationId: String,
    @SerialName("conversation_id") val conversationId: String,
    @SerialName("message_id") val messageId: String,
    @SerialName("expected_revision") val expectedRevision: Long,
    @SerialName("expected_message_revision") val expectedMessageRevision: Long,
    @SerialName("edited_text") val editedText: String,
    @SerialName("created_at_ms") val createdAtMs: Long,
)
@Serializable
data class AndroidPreparedInPlaceResend(val input: AndroidPrepareInPlaceResend,
    @SerialName("run_id") val runId: String,
    @SerialName("user_message_id") val userMessageId: String,
    val experts: List<AndroidExpertParticipantReference>)
internal fun AndroidPrepareEditedResend.inPlace() = AndroidPrepareInPlaceResend(operationId, sourceConversationId,
    sourceMessageId, expectedSourceRevision, expectedSourceMessageRevision, editedText, createdAtMs)
internal fun AndroidApplicationRuntime.prepareInPlaceResend(input: AndroidPrepareInPlaceResend): AndroidPreparedInPlaceResend =
    Json.decodeFromJsonElement(execute(buildJsonObject {
        put("type", "prepare_in_place_resend"); put("input", Json.encodeToJsonElement(input))
    }))

internal fun AndroidApplicationRuntime.loadRunIdentity(runId: String): JsonObject? = try {
    executeQuery(buildJsonObject { put("type", "load_run_identity"); put("run_id", runId) }).jsonObject
} catch (failure: Exception) {
    if (failure.message?.contains("run_not_found") == true) null else throw failure
}

internal fun AndroidApplicationRuntime.loadPreparedInPlaceResend(operationId: String): AndroidPreparedInPlaceResend? {
    val result = executeQuery(buildJsonObject { put("type", "load_prepared_in_place_resend"); put("operation_id", operationId) })
    return if (result == JsonNull) null else Json.decodeFromJsonElement<AndroidPreparedInPlaceResend>(result)
}
