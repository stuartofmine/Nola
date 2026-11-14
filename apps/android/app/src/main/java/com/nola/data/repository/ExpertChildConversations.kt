// SPDX-License-Identifier: Apache-2.0
package com.nola.data.repository

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

@Serializable
data class ExpertChildConversation(
    @SerialName("conversation_id") val conversationId: String,
    val revision: Long, val title: String,
    @SerialName("participant_id") val participantId: String,
    @SerialName("agent_id") val agentId: String,
    @SerialName("agent_revision") val agentRevision: Long?,
    @SerialName("is_current") val isCurrent: Boolean,
    val active: Boolean,
    @SerialName("created_at_ms") val createdAtMs: Long,
    @SerialName("updated_at_ms") val updatedAtMs: Long,
    @SerialName("message_count") val messageCount: Long,
)

@Serializable
data class ExpertChildConversationPage(
    @kotlinx.serialization.Transient val sourceIdentity: String = "",
    val version: Int,
    @SerialName("conversation_id") val conversationId: String,
    @SerialName("room_id") val roomId: String?,
    @SerialName("room_revision") val roomRevision: Long?,
    val children: List<ExpertChildConversation>,
    @SerialName("has_more") val hasMore: Boolean,
    @SerialName("next_offset") val nextOffset: Int?,
) {
    fun validated(parent: String, offset: Int): ExpertChildConversationPage = apply {
        require(version == 1 && conversationId == parent && offset >= 0)
        require(roomId == null || roomId.isNotBlank())
        require(roomRevision == null || roomRevision >= 0)
        require(children.map { it.conversationId }.distinct().size == children.size)
        children.forEach {
            require(it.conversationId.isNotBlank() && it.conversationId != parent && it.participantId.isNotBlank() && it.agentId.isNotBlank())
            require(it.revision >= 0 && (it.agentRevision == null || it.agentRevision >= 0) && it.messageCount >= 0)
        }
        require(if (hasMore) children.isNotEmpty() && nextOffset == offset + children.size else nextOffset == null)
    }
}

/** Historical contexts never inherit the current directory definition. */
fun currentExpertChildMember(room: AgentRoomSnapshot, child: ExpertChildConversation): RoomParticipant? =
    if (!child.active || !child.isCurrent || child.agentRevision == null) null
    else room.room.participants.singleOrNull {
        it.active && it.id == child.participantId && it.agentId == child.agentId && it.agentRevision == child.agentRevision && it.contextConversationId == child.conversationId
    }

/** Canonical exchange IDs, never message text or the live expert directory, establish authorship. */
internal data class ExpertHistorySender(val name: String, val agentId: String)

internal fun expertChildExchangeSenders(value: kotlinx.serialization.json.JsonElement?, parent: String, child: ExpertChildConversation): Map<String, ExpertHistorySender> {
    if (value == null) return emptyMap()
    val peers = value as? kotlinx.serialization.json.JsonArray ?: error("Invalid expert exchange provenance")
    val result = mutableMapOf<String, ExpertHistorySender>()
    fun kotlinx.serialization.json.JsonObject.string(name: String) = (get(name) as? kotlinx.serialization.json.JsonPrimitive)?.content
    for (item in peers) {
        val peer = item as? kotlinx.serialization.json.JsonObject ?: error("Invalid expert exchange")
        val target = peer["target"] as? kotlinx.serialization.json.JsonObject ?: error("Missing expert target")
        val source = peer["source"] as? kotlinx.serialization.json.JsonObject ?: error("Missing expert source")
        val id = checkNotNull(peer.string("exchange_id")).also { require(it.isNotBlank()) }
        require(peer.string("parent_conversation_id") == parent && target.string("context_conversation_id") == child.conversationId &&
            target.string("participant_id") == child.participantId && target.string("agent_id") == child.agentId)
        val main = source.string("kind") == "main_agent"
        require(source.string("kind") == null || main)
        require(if (main) source.string("context_conversation_id") == parent else !source.string("agent_id").isNullOrBlank())
        require(result.put("$id:user", ExpertHistorySender(peer.string("source_display_name").orEmpty().ifBlank { if (main) "Nola" else "" }, if (main) parent else checkNotNull(source.string("agent_id")))) == null)
    }
    return result
}
