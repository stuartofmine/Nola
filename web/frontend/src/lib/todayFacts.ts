export interface TodayConversationFact {conversation_id: string; title: string; updated_at_ms: number; needs_response: boolean; active_run: {run_id: string; assistant_message_id: string} | null}
export interface TodayQueuedFact {occurrence_id: string; automation_id: string; definition_revision: number; title: string | null; scheduled_for_ms: number}
export interface TodayFacts {conversations: TodayConversationFact[]; recent_conversations: TodayConversationFact[]; queued_instances: TodayQueuedFact[]; needs_attention: boolean; unrepresented_expert_contexts: {display_name: string; context_conversation_id: string}[]; has_more_conversations: boolean; next_conversation_offset: number | null; has_more_queued_instances: boolean}
const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === 'object' && !Array.isArray(value);
const id = (value: unknown): value is string => typeof value === 'string' && value.trim().length > 0;
const timestamp = (value: unknown): value is number => typeof value === 'number' && Number.isSafeInteger(value) && Number.isFinite(new Date(value).getTime());
/** Validate display facts only; the Core's needs_response is never recomputed or used to authorize. */
export function parseTodayFacts(value: unknown, host: string, offset = 0): TodayFacts {
    if (!host || !object(value) || value.source_host_id !== host || !object(value.snapshot)) throw new Error('today.invalid');
    const snapshot = value.snapshot;
    if (!Array.isArray(snapshot.conversations) || !Array.isArray(snapshot.recent_conversations) || !Array.isArray(snapshot.queued_instances)
        || typeof snapshot.needs_attention !== 'boolean' || !Array.isArray(snapshot.unrepresented_expert_contexts)
        || typeof snapshot.has_more_conversations !== 'boolean' || typeof snapshot.has_more_queued_instances !== 'boolean') throw new Error('today.invalid');
    const unrepresented_expert_contexts = snapshot.unrepresented_expert_contexts.map(context => {
        if (!object(context) || typeof context.display_name !== 'string' || !id(context.context_conversation_id)) throw new Error('today.invalid');
        return {display_name: context.display_name, context_conversation_id: context.context_conversation_id};
    });
    const next = snapshot.next_conversation_offset;
    if (snapshot.has_more_conversations ? !Number.isSafeInteger(next) || Number(next) <= offset : next !== null) throw new Error('today.invalid');
    const parseConversation = (row: unknown): TodayConversationFact => {
        if (!object(row) || !id(row.conversation_id) || typeof row.title !== 'string' || !timestamp(row.updated_at_ms)
            || typeof row.needs_response !== 'boolean' || (row.active_run !== null && (!object(row.active_run) || !id(row.active_run.run_id) || !id(row.active_run.assistant_message_id)))) throw new Error('today.invalid');
        return {conversation_id: row.conversation_id, title: row.title, updated_at_ms: row.updated_at_ms, needs_response: row.needs_response,
            active_run: row.active_run as TodayConversationFact['active_run']};
    };
    const conversations = snapshot.conversations.map(parseConversation);
    const recent_conversations = snapshot.recent_conversations.map(parseConversation);
    if (recent_conversations.length > 5 || new Set(recent_conversations.map(row => row.conversation_id)).size !== recent_conversations.length) throw new Error('today.invalid');
    const queued_instances = snapshot.queued_instances.map(row => {
        if (!object(row) || !id(row.occurrence_id) || !id(row.automation_id) || !timestamp(row.scheduled_for_ms)
            || !Number.isSafeInteger(row.definition_revision) || Number(row.definition_revision) < 1 || (row.title !== null && typeof row.title !== 'string')) throw new Error('today.invalid');
        return {occurrence_id: row.occurrence_id, automation_id: row.automation_id, definition_revision: Number(row.definition_revision), title: row.title as string | null, scheduled_for_ms: row.scheduled_for_ms};
    });
    if (new Set(conversations.map(row => row.conversation_id)).size !== conversations.length || new Set(queued_instances.map(row => row.occurrence_id)).size !== queued_instances.length) throw new Error('today.invalid');
    return {conversations, recent_conversations, queued_instances, needs_attention: snapshot.needs_attention, unrepresented_expert_contexts, has_more_conversations: snapshot.has_more_conversations, next_conversation_offset: next as number | null, has_more_queued_instances: snapshot.has_more_queued_instances};
}
