import { parseSource } from './dataSources.ts';
import type { DataSource } from './dataSources.ts';

export type ConversationDataSource = {
    source: DataSource;
    conversationOverride: boolean | null;
    resolution: { enabled: boolean; usable: boolean; reason: string };
};

const reasons = new Set([
    'ready', 'disabled_for_conversation', 'conversation_opt_in_required',
    'consent_required', 'blocked', 'contract_missing', 'contract_invalid',
    'validation_failed', 'stale',
]);

function object(value: unknown): Record<string, unknown> {
    if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('chatDataSources.invalidResponse');
    return value as Record<string, unknown>;
}

export function conversationDataSourcesUrl(conversationId: string): string {
    return `/api/data-sources?conversationId=${encodeURIComponent(conversationId)}`;
}

export function conversationDataSourceUrl(conversationId: string, sourceId: string): string {
    return `/api/chat/conversations/${encodeURIComponent(conversationId)}/data-sources/${encodeURIComponent(sourceId)}`;
}

export function parseConversationDataSource(value: unknown): ConversationDataSource {
    const row = object(value);
    const source = parseSource(row.source);
    const resolution = object(row.resolution);
    if (row.conversationOverride !== null && typeof row.conversationOverride !== 'boolean'
        || typeof resolution.enabled !== 'boolean' || typeof resolution.usable !== 'boolean'
        || typeof resolution.reason !== 'string' || !reasons.has(resolution.reason)
        || resolution.usable && (!resolution.enabled || resolution.reason !== 'ready')
        || resolution.reason === 'ready' && !resolution.usable) throw new Error('chatDataSources.invalidResponse');
    return { source, conversationOverride: row.conversationOverride as boolean | null,
        resolution: { enabled: resolution.enabled, usable: resolution.usable, reason: resolution.reason } };
}

export function parseConversationDataSources(value: unknown, conversationId: string): ConversationDataSource[] {
    const body = object(value);
    if (body.conversationId !== conversationId || !Array.isArray(body.sources)) throw new Error('chatDataSources.invalidResponse');
    const rows = body.sources.map(parseConversationDataSource);
    if (new Set(rows.map(row => row.source.id)).size !== rows.length) throw new Error('chatDataSources.invalidResponse');
    return rows;
}

export function confirmedConversationDataSource(value: unknown, sourceId: string, requested: boolean | null): ConversationDataSource {
    const row = parseConversationDataSource(object(value).source);
    if (row.source.id !== sourceId || row.conversationOverride !== requested) throw new Error('chatDataSources.invalidResponse');
    return row;
}
