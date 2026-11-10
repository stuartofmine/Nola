// SPDX-License-Identifier: Apache-2.0
package com.nola.data.remote.api

import kotlinx.serialization.json.*

data class RemoteSubmissionFrontier(val configurationRevision: Long?, val parentMessageId: String?)

fun remoteSubmissionFrontierRequest(hostId: String, clientId: String, conversationId: String,
    owner: RemoteRunKey?): JsonObject = buildJsonObject {
    require(hostId.isNotBlank() && clientId.isNotBlank() && conversationId.isNotBlank())
    require(owner == null || owner.conversationId == conversationId && owner.runId.isNotBlank() && owner.assistantMessageId.isNotBlank())
    put("expected_host_id", hostId); put("client_id", clientId); put("conversation_id", conversationId)
    put("expected_run_id", owner?.runId?.let(::JsonPrimitive) ?: JsonNull)
    put("assistant_message_id", owner?.assistantMessageId?.let(::JsonPrimitive) ?: JsonNull)
}

/** Host revision and parent are one atomic observation; neither is derived from a local mirror. */
fun decodeRemoteSubmissionFrontier(response: JsonObject, hostId: String, clientId: String,
    conversationId: String, displayedParentMessageId: String?): RemoteSubmissionFrontier {
    require(response["source_host_id"] == JsonPrimitive(hostId)) { "Submission frontier source mismatch" }
    val value = response.getValue("result").jsonObject
    require(value.keys == setOf("protocol_version", "source_host_id", "client_id", "conversation_id",
        "configuration_revision", "parent_message_id")) { "Unknown submission frontier format" }
    require(value["protocol_version"] == JsonPrimitive(1) && value["source_host_id"] == JsonPrimitive(hostId) &&
        value["client_id"] == JsonPrimitive(clientId) && value["conversation_id"] == JsonPrimitive(conversationId)) {
        "Submission frontier owner mismatch"
    }
    val revision = value.getValue("configuration_revision").let { raw ->
        if (raw == JsonNull) null else raw.jsonPrimitive.let {
            require(!it.isString && it.long > 0); it.long
        }
    }
    val parent = value.getValue("parent_message_id").let { raw ->
        if (raw == JsonNull) null else raw.jsonPrimitive.let {
            require(it.isString && it.content.isNotBlank()); it.content
        }
    }
    require(revision != null || parent == null) { "Missing source conversation revision" }
    require(parent == displayedParentMessageId) {
        "Host conversation changed. Reload before submitting; your draft is retained."
    }
    return RemoteSubmissionFrontier(revision, parent)
}
