// SPDX-License-Identifier: Apache-2.0
package com.nola.data.repository

import com.nola.data.local.AndroidApplicationRuntime
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.decodeFromJsonElement
import kotlinx.serialization.json.put
import java.util.UUID

@Serializable
data class RoomParticipant(
    val id: String,
    @SerialName("agent_id") val agentId: String,
    @SerialName("agent_revision") val agentRevision: Long,
    @SerialName("display_name") val displayName: String,
    @SerialName("context_conversation_id") val contextConversationId: String,
    val active: Boolean,
    val description: String,
)

@Serializable
data class AgentRoom(val id: String, val title: String, val participants: List<RoomParticipant>)

@Serializable
data class AgentRoomSnapshot(val revision: Long, val room: AgentRoom)

@Serializable
data class ConversationRoomAssociation(
    @SerialName("conversation_id") val conversationId: String,
    @SerialName("room_id") val roomId: String,
    val revision: Long,
    val room: AgentRoomSnapshot,
)

@Serializable
data class PersistentAgent(
    val id: String, val name: String, val revision: Long,
    val availability: String, val defaults: JsonObject, val description: String,
)

@Serializable
data class AgentDirectory(val revision: Long?, val agents: List<PersistentAgent>)

@Serializable
data class RoomSubmissionInput(
    @SerialName("room_id") val roomId: String,
    @SerialName("submission_id") val submissionId: String,
    @SerialName("expected_room_revision") val expectedRoomRevision: Long,
    val text: String,
    @SerialName("participant_id") val participantId: String?,
    @SerialName("at_ms") val atMs: Long,
    @SerialName("parent_message_id") val parentMessageId: String? = null,
    @SerialName("conversation_id") val conversationId: String? = null,
) {
    init { require((parentMessageId == null) == (conversationId == null)) { "Canonical parent and conversation owner must be supplied together" } }
}

internal fun canonicalExpertSubmissionId(conversationId: String, parentMessageId: String, participantId: String): String =
    UUID.nameUUIDFromBytes("$conversationId/$parentMessageId/$participantId".toByteArray()).toString()

@Serializable
data class RoomSubmission(val input: RoomSubmissionInput, val state: String,
    @SerialName("context_conversation_id") val contextConversationId: String?,
    @SerialName("selected_participant_id") val selectedParticipantId: String?, val revision: Long,
    @SerialName("failure_code") val failureCode: String?,
    @SerialName("execution_id") val executionId: String?, val terminal: JsonElement? = null)

@Serializable
data class RoomExecutionClaim(val submission: RoomSubmission, @SerialName("dispatch_permitted") val dispatchPermitted: Boolean)

/** Canonical Application transport only: no Room/SharedPreferences shadow database. */
class AgentRoomRepository(private val execute: (JsonObject) -> JsonElement) {
    internal var remoteHostId: String? = null
    internal var observedSourceEpoch: String? = null
    internal fun confirmRemoteSource(host: String, epoch: String) {
        require(host.isNotBlank() && epoch.isNotBlank() && (remoteHostId == null || remoteHostId == host) && (observedSourceEpoch == null || observedSourceEpoch == epoch))
        remoteHostId = host; observedSourceEpoch = epoch
    }
    constructor(runtime: AndroidApplicationRuntime) : this({ runtime.execute(it) })

    private inline fun <reified T> command(body: JsonObject, decoder: Json = Json): T = decoder.decodeFromJsonElement(
        execute(buildJsonObject { put("type", "agent_room"); put("command", body) })
    )

    fun childConversations(conversationId: String, offset: Int = 0, limit: Int = 40): ExpertChildConversationPage = command<ExpertChildConversationPage>(buildJsonObject {
        put("type", "child_conversations"); put("conversation_id", conversationId); put("offset", offset); put("limit", limit)
    }).validated(conversationId, offset)

    fun directory(): AgentDirectory = command(buildJsonObject { put("type", "directory") })

    fun roleTemplates(): ExpertRoleCatalog = command<ExpertRoleCatalog>(buildJsonObject { put("type", "role_templates") }, Json { ignoreUnknownKeys = true }).validated()

    fun ensureForConversation(conversationId: String, atMs: Long = System.currentTimeMillis()): ConversationRoomAssociation =
        command(buildJsonObject {
            put("type", "ensure_for_conversation"); put("conversation_id", conversationId); put("at_ms", atMs)
        })

    fun forConversation(conversationId: String): ConversationRoomAssociation? = command(buildJsonObject {
        put("type", "for_conversation"); put("conversation_id", conversationId)
    })

    fun submit(input: RoomSubmissionInput): RoomSubmission = command(buildJsonObject {
        put("type", "submit"); put("input", Json.encodeToJsonElement(RoomSubmissionInput.serializer(), input))
    })

    fun submission(roomId: String, submissionId: String): RoomSubmission? = command(buildJsonObject {
        put("type", "submission"); put("room_id", roomId); put("submission_id", submissionId)
    })

    fun submissions(roomId: String): List<RoomSubmission> = command(buildJsonObject {
        put("type", "submissions"); put("room_id", roomId); put("limit", 100); put("offset", 0)
    })

    fun selectRecipient(value: RoomSubmission, participantId: String): RoomSubmission = command(buildJsonObject {
        put("type", "select_recipient"); put("room_id", value.input.roomId); put("submission_id", value.input.submissionId)
        put("expected_revision", value.revision); put("participant_id", participantId); put("at_ms", System.currentTimeMillis())
    })

    fun claimExecution(value: RoomSubmission): RoomExecutionClaim = command(buildJsonObject {
        put("type", "claim_execution"); put("room_id", value.input.roomId); put("submission_id", value.input.submissionId)
        put("expected_revision", value.revision); put("at_ms", System.currentTimeMillis())
    })
    fun executionIntent(value: RoomSubmission): JsonObject? = command(buildJsonObject {
        put("type", "execution_intent"); put("room_id", value.input.roomId); put("submission_id", value.input.submissionId)
    })
    fun prepareExecution(value: RoomSubmission, intent: JsonObject): RoomSubmission = command(buildJsonObject {
        put("type", "prepare_execution"); put("room_id", value.input.roomId); put("submission_id", value.input.submissionId)
        put("expected_revision", value.revision); put("intent", intent); put("at_ms", System.currentTimeMillis())
    })
    fun hasExecution(runId: String): Boolean = execute(buildJsonObject { put("type", "load_run_identity"); put("run_id", runId) }) != JsonNull
    fun cancelPending(value: RoomSubmission): RoomSubmission = command(buildJsonObject {
        put("type", "cancel_pending"); put("room_id", value.input.roomId); put("submission_id", value.input.submissionId)
        put("expected_revision", value.revision); put("at_ms", System.currentTimeMillis())
    })

    fun observeExecution(value: RoomSubmission): RoomSubmission = command(buildJsonObject {
        put("type", "reconcile_execution"); put("room_id", value.input.roomId); put("submission_id", value.input.submissionId)
        put("at_ms", System.currentTimeMillis())
    })

    fun fail(value: RoomSubmission, code: String): RoomSubmission = command(buildJsonObject {
        put("type", "fail_submission"); put("room_id", value.input.roomId); put("submission_id", value.input.submissionId)
        put("expected_revision", value.revision); put("code", code); put("at_ms", System.currentTimeMillis())
    })

    fun list(limit: Int = 50, offset: Int = 0): List<AgentRoomSnapshot> = command(buildJsonObject {
        put("type", "list"); put("limit", limit); put("offset", offset)
    })

    fun load(roomId: String): AgentRoomSnapshot? = command(buildJsonObject {
        put("type", "read"); put("room_id", roomId)
    })

    fun create(title: String): AgentRoomSnapshot = command(buildJsonObject {
        put("type", "create"); put("title", title); put("at_ms", System.currentTimeMillis())
    })

    fun join(agent: PersistentAgent, room: AgentRoomSnapshot): AgentRoomSnapshot = command(buildJsonObject {
        put("type", "join")
        put("input", buildJsonObject {
            put("room_id", room.room.id); put("expected_room_revision", room.revision)
            put("agent_id", agent.id); put("expected_agent_revision", agent.revision)
            put("at_ms", System.currentTimeMillis())
        })
    })

    fun rejoin(agent: PersistentAgent, room: AgentRoomSnapshot): AgentRoomSnapshot = command(buildJsonObject {
        put("type", "rejoin")
        put("input", buildJsonObject {
            put("room_id", room.room.id); put("expected_room_revision", room.revision)
            put("agent_id", agent.id); put("expected_agent_revision", agent.revision)
            put("at_ms", System.currentTimeMillis())
        })
    })

    fun deactivate(participantId: String, room: AgentRoomSnapshot): AgentRoomSnapshot = command(buildJsonObject {
        put("type", "deactivate"); put("room_id", room.room.id)
        put("participant_id", participantId); put("expected_revision", room.revision)
        put("at_ms", System.currentTimeMillis())
    })

    fun createAgent(name: String, defaults: JsonObject, directory: AgentDirectory, description: String, id: String = UUID.randomUUID().toString()): AgentDirectory = command(buildJsonObject {
        put("type", "configure_agent")
        put("input", buildJsonObject {
            put("id", id); put("name", name)
            put("description", description)
            put("expected_directory_revision", directory.revision?.let(::JsonPrimitive) ?: JsonNull)
            put("expected_agent_revision", JsonNull); put("defaults", defaults)
            put("availability", "active"); put("updated_at_ms", System.currentTimeMillis())
        })
    })
    fun updateAgent(agent: PersistentAgent, name: String, description: String, directory: AgentDirectory): AgentDirectory = command(buildJsonObject {
        put("type", "configure_agent")
        put("input", buildJsonObject {
            put("id", agent.id); put("name", name); put("description", description)
            put("expected_directory_revision", directory.revision?.let(::JsonPrimitive) ?: JsonNull)
            put("expected_agent_revision", agent.revision); put("defaults", agent.defaults)
            put("availability", agent.availability); put("updated_at_ms", System.currentTimeMillis())
        })
    })

}
