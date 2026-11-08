package com.nola.data.local

import com.nola.data.remote.api.RemoteRunKey
import com.nola.data.remote.api.RemoteSubmissionIdentity
import kotlinx.serialization.json.*

/** Current Application facts; null in a UI projection means unconfirmed. */
sealed interface PermissionConfigurationOwner {
    data object Inactive : PermissionConfigurationOwner
    data class Local(val key: RemoteRunKey) : PermissionConfigurationOwner
    data class Remote(val identity: RemoteSubmissionIdentity) : PermissionConfigurationOwner
}

internal class PermissionOwnerHydration {
    private var completed: RemoteSubmissionIdentity? = null
    private var inFlight: RemoteSubmissionIdentity? = null
    private var generation = 0L

    fun begin(identity: RemoteSubmissionIdentity): Long? {
        if (completed == identity || inFlight == identity) return null
        inFlight = identity
        return ++generation
    }

    fun finish(identity: RemoteSubmissionIdentity, request: Long, succeeded: Boolean): Boolean {
        if (inFlight != identity || generation != request) return false
        inFlight = null
        if (succeeded) completed = identity
        return true
    }

    fun invalidate(forgetSuccess: Boolean = false) {
        ++generation
        inFlight = null
        if (forgetSuccess) completed = null
    }

    fun confirm(identity: RemoteSubmissionIdentity) {
        invalidate()
        completed = identity
    }
}

internal fun readPermissionConfigurationOwner(conversationId: String, hostId: String?, clientId: String?,
    unresolvedRunId: String?, sourceUnconfirmed: Boolean, execute: (JsonObject) -> JsonElement): PermissionConfigurationOwner {
    val snapshot = execute(journalCommand("read_permission_configuration") {
        put("conversation_id", conversationId)
        put("source_host_id", hostId?.let(::JsonPrimitive) ?: JsonNull)
        put("client_id", clientId?.let(::JsonPrimitive) ?: JsonNull)
        put("unresolved_run_ids", JsonArray(listOfNotNull(unresolvedRunId?.let(::JsonPrimitive))))
        put("source_unconfirmed", sourceUnconfirmed)
    }).jsonObject
    require(snapshot.journalString("conversation_id") == conversationId)
    val owner = snapshot.getValue("owner").jsonObject
    return when (owner.journalString("kind")) {
        "inactive" -> PermissionConfigurationOwner.Inactive
        "local" -> PermissionConfigurationOwner.Local(RemoteRunKey(conversationId,
            owner.journalString("run_id"), owner.journalString("assistant_message_id")))
        "remote" -> PermissionConfigurationOwner.Remote(Json.decodeFromJsonElement<RemoteSubmissionIdentity>(
            owner.getValue("identity")).also { it.validate() })
        else -> error("Invalid permission configuration owner")
    }
}
