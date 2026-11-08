export type ActivityCursor = {created_at_ms: number; run_id: string};
export type ActivityOwner = {conversation_id: string; run_id: string; assistant_message_id: string};
export type ActivityEntry = ActivityOwner & {
    source: 'agent' | 'automation' | 'conversation'; title: string; created_at_ms: number;
    completed_at_ms: number | null; status: string; needs_response: boolean; is_current: boolean;
};
export type ActivityPage = {entries: ActivityEntry[]; next_cursor: ActivityCursor | null};
export type ActivityDetail = {entry: ActivityEntry; explanation: string | null; result_text: string | null; result_has_more: boolean;
    source_conversation_id: string | null; review_in_source: boolean; execution_host_id: string | null;
    context_scope: 'conversation' | 'task' | 'unavailable'};
export const activityPath = (entry: ActivityOwner) => `/activity?${new URLSearchParams({
    conversation_id: entry.conversation_id, run_id: entry.run_id, assistant_message_id: entry.assistant_message_id})}`;
export function activityOwner(url: URL): ActivityOwner | undefined {
    const conversation_id = url.searchParams.get('conversation_id'), run_id = url.searchParams.get('run_id'), assistant_message_id = url.searchParams.get('assistant_message_id');
    return conversation_id && run_id && assistant_message_id ? {conversation_id, run_id, assistant_message_id} : undefined;
}

/** UI navigation only. The destination still validates and resolves every request. */
export function activitySourcePath(detail: ActivityDetail): string | null {
    if (!detail.source_conversation_id) return null;
    const query = new URLSearchParams({conversation: detail.source_conversation_id, from_activity: activityPath(detail.entry)});
    if (detail.entry.needs_response || detail.review_in_source) {
        query.set('run', detail.entry.run_id);
        query.set('request_conversation', detail.entry.conversation_id);
        query.set('focus', 'request');
    }
    return `/chat?${query}`;
}
export function activityFocusOwner(url: URL): {conversationId: string; runId: string} | null {
    const conversationId = url.searchParams.get('request_conversation'), runId = url.searchParams.get('run');
    return url.pathname === '/chat' && url.searchParams.get('conversation') && url.searchParams.get('focus') === 'request'
        && conversationId && runId ? {conversationId, runId} : null;
}
export function activityReturnPath(url: URL): string | null {
    const raw = url.searchParams.get('from_activity');
    if (!raw) return null;
    try {
        const target = new URL(raw, url.origin), owner = activityOwner(target);
        return target.origin === url.origin && !target.username && !target.password && target.pathname === '/activity' && owner ? activityPath(owner) : null;
    } catch { return null; }
}
