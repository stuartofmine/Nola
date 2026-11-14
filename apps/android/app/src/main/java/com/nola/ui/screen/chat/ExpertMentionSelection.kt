package com.nola.ui.screen.chat

import com.nola.data.repository.AgentDirectory
import com.nola.data.repository.PersistentAgent
import com.nola.data.repository.AgentRoomSnapshot

internal class ExpertSelectionChanged : IllegalStateException()
internal fun checkedExpertCandidate(directory: AgentDirectory, id: String, revision: Long): PersistentAgent =
    directory.agents.singleOrNull { it.id == id && it.revision == revision && it.availability == "active" }
        ?: throw ExpertSelectionChanged()

internal fun currentExpertParticipant(room: AgentRoomSnapshot, agent: PersistentAgent): com.nola.data.repository.RoomParticipant? {
    val member = room.room.participants.singleOrNull { it.agentId == agent.id && it.active }
    if (member != null && member.agentRevision != agent.revision) throw ExpertSelectionChanged()
    return member
}

internal fun rejoinedExpertParticipant(previous: AgentRoomSnapshot, next: AgentRoomSnapshot, agent: PersistentAgent): com.nola.data.repository.RoomParticipant {
    if (next.room.id != previous.room.id || next.revision <= previous.revision) throw ExpertSelectionChanged()
    val old = previous.room.participants.singleOrNull { it.agentId == agent.id && it.active } ?: throw ExpertSelectionChanged()
    val member = currentExpertParticipant(next, agent) ?: throw ExpertSelectionChanged()
    val retained = next.room.participants.singleOrNull { it.id == old.id }
    if ((retained != null && (retained.active || retained.contextConversationId != old.contextConversationId || retained.agentRevision != old.agentRevision)) ||
        member.id == old.id || member.contextConversationId == old.contextConversationId || previous.room.participants.any { it.id == member.id }) {
        throw ExpertSelectionChanged()
    }
    return member
}

/** Choosing a parent model is unnecessary for an explicit expert-only submission. */
internal fun requiresPrimaryModelForComposer(hasPrimaryModel: Boolean, selectedExpertIds: Set<String>): Boolean =
    !hasPrimaryModel && selectedExpertIds.isEmpty()

/** Existing room snapshots stay usable after directory edits. A changed child
 * owner still requires a fresh explicit selection; never silently swap context. */
internal fun selectedExpertParticipant(room: AgentRoomSnapshot, agentId: String, observedRevision: Long?,
    observedMember: com.nola.data.repository.RoomParticipant?): com.nola.data.repository.RoomParticipant? {
    val member = room.room.participants.singleOrNull { it.agentId == agentId && it.active }
    if (observedMember != null && (member == null || member.id != observedMember.id ||
        member.agentRevision != observedMember.agentRevision || member.contextConversationId != observedMember.contextConversationId)) throw ExpertSelectionChanged()
    if (member != null && observedRevision != null && member.agentRevision != observedRevision) throw ExpertSelectionChanged()
    return member
}

/** Chips belong to the pinned member snapshot; directory availability is independent. */
internal fun selectedExpertLabel(agentId: String, member: com.nola.data.repository.RoomParticipant?,
    directory: AgentDirectory?, fallback: String): String =
    member?.takeIf { it.agentId == agentId }?.displayName?.takeIf { it.isNotBlank() }
        ?: directory?.agents?.singleOrNull { it.id == agentId }?.name?.takeIf { it.isNotBlank() }
        ?: fallback
