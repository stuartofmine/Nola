import { t, type Locale } from './i18n.ts';

export type ResultSummary = {
    conversationId?: string | null; runId?: string | null;
    resultId: string; sourceId: string; sourceName: string; sourceUrl: string; sourceFormat: string;
    status: string; scheduledAt: string; createdAt: string; previewText: string; ackCount: number; expired: boolean;
};
export type ResultDetail = ResultSummary & {
    fetchedAt: string; llmSummaryText: string; failureReason?: string;
    events: Record<string, unknown>[]; htmlSnapshot?: Record<string, unknown> | null;
};
export function executionConversationLink(conversationId: unknown, runId: unknown): string | undefined {
    if (typeof conversationId !== 'string' || !conversationId.trim() || typeof runId !== 'string' || !runId.trim()) return;
    return `/chat?conversation=${encodeURIComponent(conversationId)}&run=${encodeURIComponent(runId)}`;
}
export function resultStatus(value: string, locale: Locale): string {
    return ['answered', 'action_completed', 'delegation_accepted', 'incomplete', 'blocked', 'cancelled', 'failed', 'running', 'waiting_input', 'queued'].includes(value) ? t(locale, `activity.status.${value}`) : value;
}
export function resultFormat(value: string, locale: Locale): string {
    if (value === 'rss' || value === 'json') return value.toUpperCase();
    return ['html', 'search', 'script', 'file', 'folder', 'icalendar'].includes(value) ? t(locale, `activity.format.${value}`) : value;
}
export const successfulResult = (result: ResultSummary) => ['answered', 'action_completed'].includes(result.status);
export const attentionResult = (result: ResultSummary) => ['incomplete', 'blocked', 'failed'].includes(result.status);
export function activeResult(result: ResultSummary) { return ['running', 'waiting_input', 'queued'].includes(result.status); }
export function sourceResults(results: ResultSummary[], sourceId: string | null): ResultSummary[] {
    // IDs are opaque. Never use substring matching or reinterpret them as URLs.
    return sourceId === null ? results : results.filter(result => result.sourceId === sourceId);
}
export function filterResults(results: ResultSummary[], query: string, status: string, locale: Locale = 'en') {
    const needle = query.trim().toLocaleLowerCase();
    return results.filter(result => (status === 'all' || status === 'success' && successfulResult(result) || status === 'attention' && attentionResult(result) || result.status === status) &&
        `${result.sourceName} ${result.sourceId} ${result.sourceUrl} ${result.previewText} ${resultFormat(result.sourceFormat, locale)}`.toLocaleLowerCase().includes(needle));
}
export function resultLink(value: unknown): string | undefined {
    if (typeof value !== 'string' || !value.trim()) return;
    try { const url = new URL(value); return ['http:', 'https:'].includes(url.protocol) && !url.username && !url.password ? url.href : undefined; } catch { return; }
}
export function resultTime(value: string | null | undefined, locale: Locale = 'en'): string {
    if (!value) return '—';
    const date = new Date(value);
    return Number.isFinite(date.getTime()) ? date.toLocaleString(locale) : '—';
}
