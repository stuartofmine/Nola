// SPDX-License-Identifier: Apache-2.0
package com.nola.data.local

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject

/** One committed Core read. Opaque permission/wait payloads are display evidence,
 * never a capability to answer, authorize, resume, or combine separate pages. */
@Serializable
internal data class TodayFactsSnapshot(
    val conversations: List<TodayConversationFact>,
    @SerialName("needs_attention") val needsAttention: Boolean,
    @SerialName("unrepresented_expert_contexts") val unrepresentedExpertContexts: List<TodayExpertContextFact>,
    @SerialName("recent_conversations") val recentConversations: List<TodayConversationFact>,
    @SerialName("has_more_conversations") val hasMoreConversations: Boolean,
    @SerialName("next_conversation_offset") val nextConversationOffset: Int?,
    @SerialName("queued_instances") val queuedInstances: List<TodayQueuedInstance>,
    @SerialName("has_more_queued_instances") val hasMoreQueuedInstances: Boolean,
)

@Serializable
internal data class TodayConversationFact(
    @SerialName("conversation_id") val conversationId: String,
    val title: String,
    val revision: Long,
    @SerialName("updated_at_ms") val updatedAtMs: Long,
    @SerialName("needs_response") val needsResponse: Boolean,
    @SerialName("active_run") val activeRun: TodayRunOwner?,
    @SerialName("permission_projection") val permissionProjection: JsonElement?,
    @SerialName("open_waits") val openWaits: List<JsonObject>,
    @SerialName("owned_task_waits") val ownedTaskWaits: List<JsonObject>,
    @SerialName("owned_task_permissions") val ownedTaskPermissions: List<JsonObject>,
    @SerialName("expert_contexts") val expertContexts: List<TodayExpertContextFact>,
)

@Serializable
internal data class TodayRunOwner(
    @SerialName("run_id") val runId: String,
    @SerialName("assistant_message_id") val assistantMessageId: String,
)

@Serializable
internal data class TodayQueuedInstance(
    @SerialName("occurrence_id") val occurrenceId: String,
    @SerialName("automation_id") val automationId: String,
    @SerialName("definition_revision") val definitionRevision: Long,
    val title: String?,
    @SerialName("scheduled_for_ms") val scheduledForMs: Long,
)

@Serializable
internal data class TodayExpertContextFact(
    @SerialName("room_id") val roomId: String,
    @SerialName("room_revision") val roomRevision: Long,
    @SerialName("association_revision") val associationRevision: Long?,
    @SerialName("canonical_conversation_id") val canonicalConversationId: String?,
    @SerialName("participant_id") val participantId: String,
    @SerialName("agent_id") val agentId: String,
    @SerialName("agent_revision") val agentRevision: Long,
    @SerialName("display_name") val displayName: String,
    @SerialName("context_conversation_id") val contextConversationId: String,
    @SerialName("context_revision") val contextRevision: Long,
    val fact: TodayConversationFact,
)
