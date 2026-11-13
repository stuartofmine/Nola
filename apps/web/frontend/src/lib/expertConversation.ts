import type { PersistentAgent, RoomParticipant, RoomSubmission } from './agentRooms.ts';

export interface AttributedExpertSubmission {
    submission: RoomSubmission;
    participant: RoomParticipant;
}

/** An expert can have multiple turns; separate contexts must never be merged. */
export function groupExpertSessions(items: readonly AttributedExpertSubmission[]) {
    const sessions = new Map<string, { participant: RoomParticipant; replies: AttributedExpertSubmission[] }>();
    for (const item of items) {
        const contextId = item.participant.context_conversation_id;
        const session = sessions.get(contextId);
        if (session) session.replies.push(item);
        else sessions.set(contextId, { participant: item.participant, replies: [item] });
    }
    return [...sessions.values()];
}

export function expertDirectoryMatches(agents: readonly PersistentAgent[], query: string, selectedAgentIds: ReadonlySet<string>) {
    const needle = query.normalize('NFKC').toLocaleLowerCase();
    return agents.filter(agent => agent.availability === 'active' && !selectedAgentIds.has(agent.id)
        && agent.name.normalize('NFKC').toLocaleLowerCase().includes(needle));
}

/** Project only submissions bound by the Host to a visible canonical user
 * message and the exact participant context recorded in the associated Room. */
export function projectExpertSubmissions(
    roomId: string,
    conversationId: string,
    participants: readonly RoomParticipant[],
    submissions: readonly RoomSubmission[],
    visibleUserMessageIds: ReadonlySet<string>,
): AttributedExpertSubmission[] {
    const byId = new Map(participants.map(participant => [participant.id, participant]));
    return submissions.flatMap(submission => {
        const parent = submission.input.parent_message_id;
        const participant = submission.selected_participant_id ? byId.get(submission.selected_participant_id) : undefined;
        if (submission.input.conversation_id !== conversationId || submission.input.room_id !== roomId || !parent || !visibleUserMessageIds.has(parent)
            || !participant || submission.context_conversation_id !== participant.context_conversation_id) return [];
        return [{submission, participant}];
    }).sort((a, b) => a.submission.input.at_ms - b.submission.input.at_ms
        || a.submission.input.submission_id.localeCompare(b.submission.input.submission_id));
}

/** Selection is keyed by persisted participant IDs, never directory agent IDs. */
export function toggleExpertParticipant(selected: string[], participantId: string): string[] {
    return selected.includes(participantId)
        ? selected.filter(id => id !== participantId)
        : [...selected, participantId];
}
