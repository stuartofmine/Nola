// SPDX-License-Identifier: Apache-2.0
@file:OptIn(kotlinx.serialization.ExperimentalSerializationApi::class)

package com.nola.data.local

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNames
import kotlinx.serialization.json.JsonObject

/** One committed Core read. Opaque permission/wait payloads are display evidence,
 * never a capability to answer, authorize, resume, or combine separate pages. */
@Serializable
internal data class TodayFactsSnapshot(
    val conversations: List<TodayConversationFact> = emptyList(),
    @SerialName("needs_attention") val needsAttention: Boolean = false,
    @SerialName("unrepresented_expert_contexts") val unrepresentedExpertContexts: List<TodayExpertContextFact> = emptyList(),
    @SerialName("recent_conversations") val recentConversations: List<TodayConversationFact> = emptyList(),
    @SerialName("has_more_conversations") val hasMoreConversations: Boolean = false,
    @SerialName("next_conversation_offset") val nextConversationOffset: Int? = null,
    @SerialName("queued_instances") val queuedInstances: List<TodayQueuedInstance> = emptyList(),
    @SerialName("has_more_queued_instances") val hasMoreQueuedInstances: Boolean = false,
)

@Serializable
internal data class TodayConversationFact(
    @SerialName("conversation_id")
    @JsonNames("conversationId", "id")
    val conversationId: String = "",
    val title: String = "",
    val revision: Long = 0L,
    @SerialName("updated_at_ms")
    @JsonNames("updatedAtMs")
    val updatedAtMs: Long = 0L,
    @SerialName("needs_response")
    @JsonNames("needsResponse")
    val needsResponse: Boolean = false,
    @SerialName("active_run")
    @JsonNames("activeRun")
    val activeRun: TodayRunOwner? = null,
    @SerialName("permission_projection")
    @JsonNames("permissionProjection")
    val permissionProjection: JsonElement? = null,
    @SerialName("open_waits")
    @JsonNames("openWaits")
    val openWaits: List<JsonObject> = emptyList(),
    @SerialName("owned_task_waits")
    @JsonNames("ownedTaskWaits")
    val ownedTaskWaits: List<JsonObject> = emptyList(),
    @SerialName("owned_task_permissions")
    @JsonNames("ownedTaskPermissions")
    val ownedTaskPermissions: List<JsonObject> = emptyList(),
    @SerialName("expert_contexts")
    @JsonNames("expertContexts")
    val expertContexts: List<TodayExpertContextFact> = emptyList(),
)

@Serializable
internal data class TodayRunOwner(
    @SerialName("run_id") val runId: String = "",
    @SerialName("assistant_message_id") val assistantMessageId: String = "",
)

@Serializable
internal data class TodayQueuedInstance(
    @SerialName("occurrence_id") val occurrenceId: String = "",
    @SerialName("automation_id") val automationId: String = "",
    @SerialName("definition_revision") val definitionRevision: Long = 0L,
    val title: String? = null,
    @SerialName("scheduled_for_ms") val scheduledForMs: Long = 0L,
)

@Serializable
internal data class TodayExpertContextFact(
    @SerialName("room_id") val roomId: String = "",
    @SerialName("room_revision") val roomRevision: Long = 0L,
    @SerialName("association_revision") val associationRevision: Long? = null,
    @SerialName("canonical_conversation_id") val canonicalConversationId: String? = null,
    @SerialName("participant_id") val participantId: String = "",
    @SerialName("agent_id") val agentId: String = "",
    @SerialName("agent_revision") val agentRevision: Long = 0L,
    @SerialName("display_name") val displayName: String = "",
    @SerialName("context_conversation_id") val contextConversationId: String = "",
    @SerialName("context_revision") val contextRevision: Long = 0L,
    val fact: TodayConversationFact = TodayConversationFact(),
)
