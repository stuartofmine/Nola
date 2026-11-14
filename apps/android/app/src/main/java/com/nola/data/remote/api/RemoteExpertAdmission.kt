// SPDX-License-Identifier: Apache-2.0
package com.nola.data.remote.api

import com.nola.data.local.AndroidExpertMessageManifest
import kotlinx.serialization.json.*

/** A remote receipt never becomes a locally invented user message or assistant Run. */
internal data class RemoteExpertAdmission(val manifest: AndroidExpertMessageManifest, val parentAdmission: JsonObject)
internal fun decodeRemoteExpertAdmission(response: JsonObject, host: String, client: String, parent: String, message: String): RemoteExpertAdmission? {
    require(response["host_id"]?.jsonPrimitive?.content == host && response["client_id"]?.jsonPrimitive?.content == client)
    val raw = response["manifest"] ?: error("Expert manifest response is incomplete")
    if (raw == JsonNull) { require(response["parent_admission"] == JsonNull); return null }
    val manifest = Json.decodeFromJsonElement<AndroidExpertMessageManifest>(raw)
    require(manifest.version == 1 && manifest.input.conversationId == parent && manifest.input.userMessageId == message &&
        manifest.entries.map { it.participant } == manifest.input.selectedParticipants)
    val admission = response.getValue("parent_admission").jsonObject
    require(admission["version"]?.jsonPrimitive?.int == 1 && admission["source_host_id"]?.jsonPrimitive?.content == host &&
        admission["source_store_epoch"]?.jsonPrimitive?.content?.isNotBlank() == true &&
        admission["conversation_id"]?.jsonPrimitive?.content == parent && admission["input"] == raw.jsonObject["input"])
    return RemoteExpertAdmission(manifest, admission)
}

internal fun definitiveRemoteExpertRejection(error: Exception, host: String, client: String, input: com.nola.data.local.AndroidAdmitExpertMessage): Boolean {
    val failure = error as? WebHostStatusException ?: return false
    val evidence = failure.expertRejection ?: return false
    return failure.statusCode == 409 && failure.errorCode == "expert_admission_rejected" &&
        evidence["definitive_rejection"] == JsonPrimitive(true) && evidence["host_id"] == JsonPrimitive(host) &&
        evidence["client_id"] == JsonPrimitive(client) && evidence["conversation_id"] == JsonPrimitive(input.conversationId) &&
        evidence["submission_id"] == JsonPrimitive(input.submissionId) && evidence["user_message_id"] == JsonPrimitive(input.userMessageId)
}
