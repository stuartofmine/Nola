// SPDX-License-Identifier: Apache-2.0
package com.nola.data.local
import kotlinx.serialization.json.*

/** Stable retry identity only; Core validates the actual admitted Host effect. */
internal fun expertHostIntentIdentity(operation: JsonObject): JsonElement = when (operation["kind"]?.jsonPrimitive?.content) {
    "admit", "admit_peer", "prepare_edited_resend" -> operation.getValue("input")
    "prepare_peer_task", "abort_peer_task_preparation", "peer_task_action" -> {
        val request = operation.getValue("request").jsonObject
        buildJsonObject {
            for (field in listOf("effect_id", "session_id", "turn_id")) put(field, request.getValue(field))
            operation["exchange_id"]?.let { put("exchange_id", it) }
            if (operation["kind"] == JsonPrimitive("peer_task_action")) put("input", request.getValue("payload").jsonObject.getValue("input"))
        }
    }
    else -> error("Unknown native expert mutation family")
}
internal fun validateExpertPeerHostReceipt(operation: JsonObject, result: JsonElement) {
    val value = result.jsonObject
    when (operation.getValue("kind").jsonPrimitive.content) {
        "admit_peer" -> {
            val input = operation.getValue("input").jsonObject; val view = value.getValue("view").jsonObject
            require(view.getValue("source").jsonObject["context_conversation_id"] == input["source_context_id"] &&
                view["source_run_id"] == input["source_run_id"] && view["source_call_id"] == input["call_id"] &&
                view.getValue("target").jsonObject["participant_id"] == input["target_participant_id"] && view["text"] == input["text"])
        }
        "prepare_peer_task" -> {
            val id = operation.getValue("exchange_id")
            require(value.getValue("view").jsonObject["exchange_id"] == id && value.getValue("prepared").jsonObject.getValue("task").jsonObject["id"] == id)
            require(value.getValue("prepared").jsonObject.getValue("task").jsonObject["child_session_id"] ==
                value.getValue("view").jsonObject.getValue("target").jsonObject["context_conversation_id"])
        }
        "peer_task_action" -> require(value.getValue("response").jsonObject["call_id"] == operation.getValue("request").jsonObject.getValue("payload").jsonObject["call_id"])
        "abort_peer_task_preparation" -> require(value["preparation_aborted"] is JsonPrimitive)
        else -> error("Unknown native peer response")
    }
}
