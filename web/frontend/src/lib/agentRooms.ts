import { api } from './api.ts';

export type RoomParticipant = { id: string; agent_id: string; agent_revision: number; display_name: string; context_conversation_id: string; active: boolean; description: string };
export type RoomSnapshot = { revision: number; room: { id: string; title: string; participants: RoomParticipant[] } };
export type PersistentAgent = { id: string; name: string; revision: number; availability: string; defaults: Record<string, unknown>; description: string };
export type AgentDirectory = { revision: number | null; agents: PersistentAgent[] };
export type RoomSubmissionInput = {room_id: string; submission_id: string; expected_room_revision: number; text: string; participant_id: string | null; at_ms: number; parent_message_id?: string | null; conversation_id?: string | null};
export type RoomSubmission = {input: RoomSubmissionInput; state: 'accepted' | 'selected' | 'executing' | 'completed' | 'incomplete' | 'blocked' | 'cancelled' | 'failed'; context_conversation_id: string | null; selected_participant_id: string | null; revision: number; failure_code: string | null; execution_id: string | null; terminal?: unknown};
export type RoomDraft = {text: string; participantId: string | null; pending?: RoomSubmissionInput; error?: boolean};

/** Retry the original uncertain submission, never a newly edited draft or a new ID. */
export function pendingRoomRetry(roomId: string, draft: RoomDraft): RoomSubmissionInput | null {
    return draft.error && draft.pending?.room_id === roomId ? draft.pending : null;
}

/** Polls and send acknowledgements can arrive out of order; immutable input owns the receipt. */
export function mergeRoomSubmissions(roomId: string, current: RoomSubmission[], incoming: RoomSubmission[]): RoomSubmission[] {
    const values = new Map(current.filter(value => value.input.room_id === roomId).map(value => [value.input.submission_id, value]));
    for (const value of incoming) {
        if (value.input.room_id !== roomId) continue;
        const previous = values.get(value.input.submission_id);
        if (previous) {
            if (value.revision <= previous.revision) continue;
            if (Object.keys(previous.input).some(key => previous.input[key as keyof RoomSubmissionInput] !== value.input[key as keyof RoomSubmissionInput])) continue;
        }
        values.set(value.input.submission_id, value);
    }
    return [...values.values()].sort((a, b) => a.input.at_ms - b.input.at_ms || a.input.submission_id.localeCompare(b.input.submission_id));
}

export function mentionQuery(text: string): string | null {
    const match = /(?:^|\s)@([^\s@]*)$/u.exec(text);
    return match ? match[1] : null;
}
export function removeMentionQuery(text: string): string { return text.replace(/(^|\s)@[^\s@]*$/u, '$1'); }

export function roomCommand<T>(command: Record<string, unknown>, signal?: AbortSignal): Promise<T> {
    return api<T>('/api/agent-rooms', {method: 'POST', body: JSON.stringify(command), signal});
}

/** Completion choices bind opaque IDs. Names and ordinary message words never route a turn. */
export function mentionCandidates(room: RoomSnapshot, query: string): RoomParticipant[] {
    const needle = query.toLocaleLowerCase();
    return room.room.participants.filter(member => member.active && member.display_name.toLocaleLowerCase().includes(needle));
}

export function selectedRecipient(room: RoomSnapshot, id: string | null): RoomParticipant | undefined {
    return room.room.participants.find(member => member.id === id && member.active);
}
