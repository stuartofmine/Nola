import { ApiError } from './api.ts';

export const channelKinds = ['telegram', 'whatsapp', 'wechat_official', 'we_com'] as const;
export type WorkerSettings = { enabled: boolean; backend_url: string; has_access_token: boolean; execution_backend: string; turn_timeout_seconds: number; poll_seconds: number; project_root: string | null };
export type WorkerDraft = { enabled: boolean; backend_url: string; execution_backend: string; turn_timeout_seconds: string; poll_seconds: string; project_root: string };
export type Channel = { connection_id: string; kind: string; display_name: string; account_id: string; credential_ref: string; webhook_secret_ref: string; verify_token_ref: string; provider_config: Record<string, unknown>; enabled: boolean; state: string; last_received_at?: string };
export type ChannelDraft = Omit<Channel, 'state' | 'last_received_at'>;
export class MessagingError extends Error { key: string; constructor(key: string) { super(key); this.key = key; } }
function invalid(): never { throw new MessagingError('messaging.invalidResponse'); }
function object(value: unknown): Record<string, unknown> { return value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : invalid(); }
function string(value: unknown): string { return typeof value === 'string' ? value : invalid(); }
function bool(value: unknown): boolean { return value === true || value === 1 ? true : value === false || value === 0 ? false : invalid(); }
function integer(value: unknown, min: number, max: number): number { return Number.isSafeInteger(value) && Number(value) >= min && Number(value) <= max ? Number(value) : invalid(); }
export function workerSnapshot(value: unknown): WorkerSettings {
    const data = object(value);
    return { enabled: bool(data.enabled), backend_url: string(data.backend_url), has_access_token: bool(data.has_access_token), execution_backend: string(data.execution_backend),
        turn_timeout_seconds: integer(data.turn_timeout_seconds, 30, 14400), poll_seconds: integer(data.poll_seconds, 1, 300), project_root: data.project_root === null ? null : string(data.project_root) };
}
export function workerDraft(value: WorkerSettings): WorkerDraft { return { enabled: value.enabled, backend_url: value.backend_url, execution_backend: value.execution_backend,
    turn_timeout_seconds: String(value.turn_timeout_seconds), poll_seconds: String(value.poll_seconds), project_root: value.project_root ?? '' }; }
export function validBackendURL(value: string): boolean {
    try { const url = new URL(value); return ['http:', 'https:'].includes(url.protocol) && !url.username && !url.password && !url.search && !url.hash; } catch { return false; }
}
export function workerPayload(draft: WorkerDraft, baseline: WorkerSettings, token: string, clearToken = false) {
    const backend = draft.backend_url.trim(); const secret = token.trim();
    if (backend && !validBackendURL(backend) || draft.enabled && !backend) throw new MessagingError('messaging.invalidURL');
    if (secret && !/^[\x21-\x7e]{1,8192}$/.test(secret)) throw new MessagingError('messaging.invalidToken');
    if (secret && clearToken) throw new MessagingError('messaging.invalidToken');
    if (draft.enabled && !secret && (!baseline.has_access_token || clearToken)) throw new MessagingError('messaging.tokenRequired');
    // Never silently reuse saved account credentials for a different service.
    if (backend !== baseline.backend_url.trim() && baseline.has_access_token && !secret && !clearToken) throw new MessagingError('messaging.newTokenRequired');
    const timeout = Number(draft.turn_timeout_seconds), poll = Number(draft.poll_seconds);
    if (!Number.isSafeInteger(timeout) || timeout < 30 || timeout > 14400 || !Number.isSafeInteger(poll) || poll < 1 || poll > 300) throw new MessagingError('messaging.invalidRange');
    if (!draft.execution_backend.trim()) throw new MessagingError('messaging.invalidBackend');
    return { enabled: draft.enabled, backend_url: backend, execution_backend: draft.execution_backend.trim(), turn_timeout_seconds: timeout, poll_seconds: poll,
        project_root: draft.project_root.trim() || null, ...(secret ? { access_token: secret } : {}), ...(clearToken ? { clear_access_token: true } : {}) };
}
export function channelSnapshot(value: unknown): Channel {
    const row = object(value);
    return { connection_id: string(row.connection_id), kind: string(row.kind), display_name: string(row.display_name), account_id: string(row.account_id),
        credential_ref: string(row.credential_ref), webhook_secret_ref: string(row.webhook_secret_ref), verify_token_ref: string(row.verify_token_ref),
        provider_config: structuredClone(object(row.provider_config)), enabled: bool(row.enabled), state: string(row.state),
        ...(typeof row.last_received_at === 'string' ? { last_received_at: row.last_received_at } : {}) };
}
export function channelsSnapshot(value: unknown): Channel[] {
    const rows = object(value).channels; if (!Array.isArray(rows)) return invalid();
    const channels = rows.map(channelSnapshot);
    if (new Set(channels.map(channel => channel.connection_id)).size !== channels.length) return invalid();
    return channels;
}
export function channelDraft(value?: Channel, kind = 'telegram'): ChannelDraft {
    return value ? { connection_id: value.connection_id, kind: value.kind, display_name: value.display_name, account_id: value.account_id,
        credential_ref: value.credential_ref, webhook_secret_ref: value.webhook_secret_ref, verify_token_ref: value.verify_token_ref, provider_config: structuredClone(value.provider_config), enabled: value.enabled }
        : { connection_id: '', kind, display_name: '', account_id: '', credential_ref: '', webhook_secret_ref: '', verify_token_ref: '', provider_config: {}, enabled: false };
}
export const providerFields: Record<string, readonly string[]> = { telegram: [], whatsapp: ['phone_number_id', 'graph_version'], wechat_official: ['app_id'], we_com: ['corp_id', 'agent_id'] };
export function channelPayload(draft: ChannelDraft, baseline?: Channel): ChannelDraft {
    if (!channelKinds.includes(draft.kind as typeof channelKinds[number]) || !/^[A-Za-z0-9._-]+$/.test(draft.connection_id)) throw new MessagingError('messaging.invalidIdentity');
    if (baseline && (draft.connection_id !== baseline.connection_id || draft.kind !== baseline.kind)) throw new MessagingError('messaging.changedIdentity');
    for (const ref of [draft.credential_ref, draft.webhook_secret_ref]) if (!/^[A-Za-z0-9._-]+$/.test(ref.trim())) throw new MessagingError('messaging.invalidRef');
    if (draft.verify_token_ref && !/^[A-Za-z0-9._-]+$/.test(draft.verify_token_ref.trim())) throw new MessagingError('messaging.invalidRef');
    // The server treats an empty verify ref as preserve, not clear. Do not report a false clear.
    if (baseline?.verify_token_ref && !draft.verify_token_ref.trim()) throw new MessagingError('messaging.cannotClearRef');
    return { ...structuredClone(draft), display_name: draft.display_name.trim(), account_id: draft.account_id.trim(), credential_ref: draft.credential_ref.trim(), webhook_secret_ref: draft.webhook_secret_ref.trim(), verify_token_ref: draft.verify_token_ref.trim() };
}
export function channelReceipt(value: unknown, request: ChannelDraft): Channel {
    const row = object(value);
    if (row.connection_id !== request.connection_id || row.kind !== request.kind) return invalid();
    return { ...structuredClone(request), display_name: string(row.display_name), account_id: string(row.account_id), enabled: bool(row.enabled), state: string(row.state) };
}
export function callbackURL(settings: WorkerSettings | undefined, channel: ChannelDraft | undefined): string {
    if (!settings || !channel || !validBackendURL(settings.backend_url) || !channelKinds.includes(channel.kind as typeof channelKinds[number]) || !/^[A-Za-z0-9._-]+$/.test(channel.connection_id)) return '';
    return `${settings.backend_url.replace(/\/$/, '')}/v1/channels/webhook/${channel.kind}/${encodeURIComponent(channel.connection_id)}`;
}
// Provider failures can echo credentials or URLs. Never render their raw bodies.
export function messagingErrorKey(error: unknown): string { return error instanceof MessagingError ? error.key : error instanceof ApiError && [401, 403].includes(error.status) ? 'messaging.authError' : 'messaging.requestError'; }
