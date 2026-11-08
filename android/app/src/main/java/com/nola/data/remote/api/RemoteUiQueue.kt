package com.nola.data.remote.api

import kotlinx.serialization.Serializable
import kotlinx.serialization.json.*

@Serializable
data class RemoteUiQueueHead(val command_id: String, val kind: String,
    val user_message_id: String? = null, val lease_owner: String? = null, val lease_until_ms: Long? = null)
data class RemoteUiQueueState(val head: RemoteUiQueueHead?, val dispatch: RemoteSubmissionReceipt?)
fun decodeRemoteUiQueue(value: JsonObject, host: String, client: String, conversation: String): RemoteUiQueueState {
    require(value.keys == setOf("head", "dispatch"))
    val head = Json.decodeFromString<RemoteUiQueueHead?>(value.getValue("head").toString())
    if (head != null) {
        require(head.command_id.isNotBlank() && (head.lease_owner == null) == (head.lease_until_ms == null))
        require(head.kind in setOf("user_input", "active_submission", "permission_policy", "task_terminal", "task_guidance"))
    }
    val raw = value.getValue("dispatch").takeUnless { it == JsonNull }?.jsonObject
    val receipt = raw?.let {
        val command = checkNotNull(head).command_id
        val identity = RemoteSubmissionIdentity(host, client, conversation, command, "ui:$command:user", "$command:assistant")
        require(head.user_message_id == identity.userMessageId)
        decodeRemoteSubmissionReceipt(it, identity, it.getValue("request_digest").jsonPrimitive.content)
    }
    return RemoteUiQueueState(head, receipt)
}

/** Settlement belongs to the originating conversation, independent of the visible tab. */
fun clearCancelledUiQueueOwner(receipt: RemoteSubmissionReceipt,
    rejected: MutableMap<String, RemoteSubmissionReceipt>, suppressed: MutableSet<String>) {
    val conversation = receipt.identity.conversationId
    if (rejected[conversation] == receipt) {
        rejected.remove(conversation)
        suppressed.remove(conversation)
    }
}
