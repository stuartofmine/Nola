/** Display-only projection of the canonical conversation request ledger.
 * Never aggregate browser events: replay, reload, and multiple Runs must not
 * double count requests. A missing provider report is not a zero cache hit. */
import { parseCumulativeCacheUsage } from './runContextUsage.ts';
import type { CommittedRunContextUsage } from './runContextUsage.ts';

export type ConversationCacheUsage = CommittedRunContextUsage['run_cache_usage'];

export function parseConversationCacheUsage(raw: unknown, conversationId: string): ConversationCacheUsage {
    if (!raw || typeof raw !== 'object' || Array.isArray(raw)) throw new Error('Invalid conversation usage');
    const value = raw as Record<string, unknown>;
    if (value.conversation_id !== conversationId || !value.usage || typeof value.usage !== 'object' || Array.isArray(value.usage))
        throw new Error('Conversation usage owner mismatch');
    const usage = value.usage as Record<string, unknown>;
    return parseCumulativeCacheUsage(usage.conversation_cache_usage ?? null);
}
