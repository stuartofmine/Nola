import type { TimelinePart } from './chatTimeline.ts';
import { messageWindowUrl } from './chatConversations.ts';
const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === 'object' && !Array.isArray(value);
const id = (value: unknown): value is string => typeof value === 'string' && !!value.trim() && value.trim() === value;
export interface TaskDetailsOwner { conversationId: string; runId: string; taskId: string }
export function taskDetailsOwner(part: TimelinePart): TaskDetailsOwner | null {
    if (part.source !== 'journal' || !['task', 'task_control'].includes(part.value.kind) || !id(part.value.task_id)) return null;
    let identity: unknown;
    try { identity = JSON.parse(part.journalOwner); } catch { return null; }
    if (!Array.isArray(identity) || identity.length !== 4 || !identity.every(id)) return null;
    return {conversationId: identity[1], runId: identity[2], taskId: part.value.task_id};
}
/** Navigation uses Core's task lineage, never the opaque journal result URI. */
export async function loadTaskConversation(owner: TaskDetailsOwner, read: (path: string) => Promise<unknown>): Promise<string | null> {
    const response = await read(messageWindowUrl(owner.conversationId, null, 1));
    if (!object(response) || !object(response.taskProjection) || !object(response.taskProjection.tasks)) throw new Error('Task projection unavailable');
    const task = response.taskProjection.tasks[owner.taskId];
    if (!object(task) || task.id !== owner.taskId || task.owner_session_id !== owner.conversationId || task.owner_turn_id !== owner.runId) throw new Error('Task owner mismatch');
    if (!object(task.lineage)) throw new Error('Task lineage unavailable');
    const child = task.lineage.child_session_id;
    if (child === null) return null;
    if (!id(child) || child === owner.conversationId) throw new Error('Invalid child conversation');
    return `/chat?conversation=${encodeURIComponent(child)}`;
}

export type ConversationExecutionOwnership = {kind:'user'} | {kind:'unavailable'} | {
    kind:'task_child';parent_conversation_id:string;parent_run_id:string;task_id:string;child_conversation_id:string;
};
export function conversationExecutionOwnership(value: unknown, conversationId: string): ConversationExecutionOwnership {
    if (!object(value)) return {kind:'unavailable'};
    if (value.kind === 'user' && Object.keys(value).length === 1) return {kind:'user'};
    if (value.kind === 'unavailable' && Object.keys(value).length === 1) return {kind:'unavailable'};
    if (value.kind !== 'task_child' || !id(value.parent_conversation_id) || !id(value.parent_run_id)
        || !id(value.task_id) || value.child_conversation_id !== conversationId || value.parent_conversation_id === conversationId) return {kind:'unavailable'};
    return value as ConversationExecutionOwnership;
}
