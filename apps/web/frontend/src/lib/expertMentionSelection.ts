import type {AgentDirectory, PersistentAgent, RoomSnapshot, RoomParticipant} from './agentRooms.ts';
export class ExpertSelectionChanged extends Error {}
/** The suggestion is an opaque ID+revision assertion, never its displayed name. */
export function checkedExpertCandidate(directory: AgentDirectory, id: string, revision: number): PersistentAgent {
    const agent = directory.agents.find(agent => agent.id === id);
    if (!agent || agent.availability !== 'active' || agent.revision !== revision) throw new ExpertSelectionChanged();
    return agent;
}
export function existingExpertParticipant(room: RoomSnapshot, agent: PersistentAgent): RoomParticipant | undefined {
    const member = room.room.participants.find(member => member.agent_id === agent.id && member.active);
    if (member && member.agent_revision !== agent.revision) throw new ExpertSelectionChanged();
    return member;
}
/** The roster receipt proves a new child owner; historical manifests remain Core-owned. */
export function rejoinedExpertParticipant(previous: RoomSnapshot, next: RoomSnapshot, agent: PersistentAgent): RoomParticipant {
    if (next.room.id !== previous.room.id || next.revision <= previous.revision) throw new ExpertSelectionChanged();
    const old = previous.room.participants.find(member => member.agent_id === agent.id && member.active);
    const member = existingExpertParticipant(next, agent);
    const retained = old && next.room.participants.find(value => value.id === old.id);
    if (!old || !member || (retained && (retained.active || retained.context_conversation_id !== old.context_conversation_id
        || retained.agent_revision !== old.agent_revision)) || member.id === old.id
        || member.context_conversation_id === old.context_conversation_id || previous.room.participants.some(value => value.id === member.id)) {
        throw new ExpertSelectionChanged();
    }
    return member;
}

/** Submission pins room participant identities, not today's directory revision. */
export function selectedExpertMembers(room: RoomSnapshot, participantIds: string[]): RoomParticipant[] {
    if (new Set(participantIds).size !== participantIds.length) throw new ExpertSelectionChanged();
    return participantIds.map(id => {
        const member = room.room.participants.find(member => member.id === id && member.active);
        if (!member) throw new ExpertSelectionChanged();
        return member;
    });
}
