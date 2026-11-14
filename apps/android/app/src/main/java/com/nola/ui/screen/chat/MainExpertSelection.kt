// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import com.nola.data.repository.RoomParticipant
import kotlinx.serialization.json.*

/** An explicit UI selection accompanies the original Main user input; it never rewrites that input. */
internal fun mainExpertSelection(ids: Set<String>, revisions: Map<String, Long>, members: Map<String, RoomParticipant>): JsonObject? {
    if (ids.isEmpty()) return null
    return buildJsonObject {
        put("version", 1)
        put("selected_experts", buildJsonArray {
            ids.sorted().forEach { id ->
                val revision = checkNotNull(revisions[id]); require(id.isNotBlank() && revision > 0)
                val member = members[id]
                require(member == null || member.active && member.agentId == id && member.agentRevision == revision && member.id.isNotBlank() && member.contextConversationId.isNotBlank())
                add(buildJsonObject {
                    put("agent_id", id); put("agent_revision", revision)
                    put("participant_id", member?.id?.let(::JsonPrimitive) ?: JsonNull)
                    put("context_conversation_id", member?.contextConversationId?.let(::JsonPrimitive) ?: JsonNull)
                })
            }
        })
    }
}

/** These are fixed Core protocol error identifiers, never user-text routing. */
internal fun mainExpertRequiresTools(code: String?): Boolean = code in setOf(
    "main_expert_selection_requires_tool_capable_main_model",
    "main_expert_selection_requires_enabled_expert_dispatch",
)
