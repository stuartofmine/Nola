import { api, apiResponse, ApiError } from './api';
import {
    parseChatConversation,
    parseChatConversations,
    conversationUrl,
    type ChatConversation,
} from './chatConversations';
import type { UiInteraction, UiSnapshot, UiSubmissionOutcome } from './generativeUi';

export type V2PermissionProfile = 'cautious' | 'allow_all';
export type V2PermissionScope = 'once' | 'session';
export type V2ReasoningEffort = 'automatic' | 'none' | 'low' | 'medium' | 'high';

export function normalizeV2PermissionProfile(value: unknown): V2PermissionProfile {
    return value === 'allow_all' ? 'allow_all' : 'cautious';
}

export function normalizeV2ReasoningEffort(value: unknown): V2ReasoningEffort {
    if (value === 'none' || value === 'low' || value === 'medium' || value === 'high') {
        return value;
    }
    return 'automatic';
}

export interface V2ToolCall {
    callId: string;
    name: string;
    arguments?: unknown;
    status: 'running' | 'completed' | 'failed';
    summary?: string;
}

export interface V2Attachment {
    name: string;
    mimeType: string;
    text?: string;
    dataUrl?: string;
}

export interface V2PermissionPrompt {
    requestId: string;
    tool: string;
    reason: string;
    arguments: unknown;
}

export interface V2UserQuestion {
    requestId: string;
    question: string;
    options: string[];
}

export interface V2TokenUsage {
    input_tokens: number;
    output_tokens: number;
    cache_read_tokens: number;
    cache_write_tokens: number;
    cache_hit_rate: number;
}

export function emptyV2TokenUsage(): V2TokenUsage {
    return {
        input_tokens: 0,
        output_tokens: 0,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        cache_hit_rate: 0,
    };
}

export function parseV2TokenUsage(raw: unknown): V2TokenUsage | null {
    if (!raw || typeof raw !== 'object') return null;
    const o = raw as Record<string, unknown>;
    const input = Number(o.input_tokens ?? o.inputTokens ?? 0);
    const output = Number(o.output_tokens ?? o.outputTokens ?? 0);
    const cacheRead = Number(o.cache_read_tokens ?? o.cacheReadTokens ?? 0);
    const cacheWrite = Number(o.cache_write_tokens ?? o.cacheWriteTokens ?? 0);
    if (![input, output, cacheRead, cacheWrite].every(Number.isFinite)) return null;
    const explicitRate = typeof o.cache_hit_rate === 'number' && Number.isFinite(o.cache_hit_rate)
        ? o.cache_hit_rate
        : typeof o.cache_hit_rate_bps === 'number' && Number.isFinite(o.cache_hit_rate_bps)
          ? o.cache_hit_rate_bps / 100
          : input > 0
            ? Math.min(100, (cacheRead / input) * 100)
            : 0;
    return {
        input_tokens: Math.max(0, Math.round(input)),
        output_tokens: Math.max(0, Math.round(output)),
        cache_read_tokens: Math.max(0, Math.round(cacheRead)),
        cache_write_tokens: Math.max(0, Math.round(cacheWrite)),
        cache_hit_rate: Math.max(0, Math.min(100, explicitRate)),
    };
}

export function mergeV2TokenUsage(prev: V2TokenUsage, delta: V2TokenUsage): V2TokenUsage {
    const input = prev.input_tokens + delta.input_tokens;
    const output = prev.output_tokens + delta.output_tokens;
    const cacheRead = prev.cache_read_tokens + delta.cache_read_tokens;
    const cacheWrite = prev.cache_write_tokens + delta.cache_write_tokens;
    const cacheHitRate = input > 0 ? Math.min(100, (cacheRead / input) * 100) : 0;
    return {
        input_tokens: input,
        output_tokens: output,
        cache_read_tokens: cacheRead,
        cache_write_tokens: cacheWrite,
        cache_hit_rate: cacheHitRate,
    };
}

export interface V2UiActionPayload {
    actionId: string;
    surfaceId: string;
    dataModel: Record<string, unknown>;
    summary: string;
}

export interface V2Expert {
    id: string;
    name: string;
    description?: string;
    role?: string;
}

export interface V2ConversationGoal {
    id?: string;
    title: string;
    description?: string;
    status?: string;
    updatedAt?: string;
}

export interface V2WorkspaceCheckpoint {
    id: string;
    label: string;
    summary?: string;
    createdAt?: string;
}

export interface V2WorkspaceDiffSnapshot {
    root: string;
    isGitRepository: boolean;
    entries: Array<{ path: string; index: string; worktree: string }>;
    files: Array<{ path: string; size: number; isDir: boolean }>;
    stagedText: string;
    unstagedText: string;
    rawDiffText?: string;
}

export interface V2DataSourceItem {
    id: string;
    name: string;
    kind: string;
    location: string;
    accessPolicy: string;
    enabled: boolean;
}

export interface V2ConversationConfiguration {
    permissionProfile: V2PermissionProfile;
    reasoningEffort: V2ReasoningEffort;
    knowledge: string;
    selectedModelId?: string;
    configurationRevision?: number;
}

export interface V2Message {
    id: string;
    seq?: number;
    role: 'user' | 'assistant' | 'system';
    content: string;
    reasoning?: string;
    toolCalls?: V2ToolCall[];
    uiSurfaces?: UiSnapshot[];
    attachments?: V2Attachment[];
    createdAtMs?: number;
}

export interface V2ModelChoice {
    id: string;
    displayName: string;
    modelName: string;
    endpointId: string;
    selected: boolean;
}

export interface V2SendOptions {
    modelId?: string;
    permissionProfile?: V2PermissionProfile;
    reasoningEffort?: Exclude<V2ReasoningEffort, 'automatic'>;
    attachments?: V2Attachment[];
    selectedExperts?: string[];
    uiAction?: V2UiActionPayload;
}

export interface V2StreamCallbacks {
    onSessionStarted?: (sessionId: string) => void;
    onTextDelta?: (delta: string) => void;
    onReasoningDelta?: (delta: string) => void;
    onToolStarted?: (callId: string, name: string, argumentsJson?: unknown) => void;
    onToolFinished?: (callId: string, name: string, ok: boolean, summary: string) => void;
    onUiSurface?: (snapshot: UiSnapshot) => void;
    onPermissionRequested?: (prompt: V2PermissionPrompt) => void;
    onUserQuestion?: (question: V2UserQuestion) => void;
    onCompacted?: (summary: string) => void;
    onTurnCompleted?: (finalText?: string, usage?: V2TokenUsage) => void;
    onError?: (message: string) => void;
}

export function formatPermissionSummary(prompt: V2PermissionPrompt): {
    headline: string;
    target?: string;
} {
    const args = prompt.arguments && typeof prompt.arguments === 'object' && !Array.isArray(prompt.arguments)
        ? (prompt.arguments as Record<string, unknown>)
        : null;
    if (!args) {
        return {
            headline: prompt.tool,
            target: typeof prompt.arguments === 'string' && prompt.arguments.trim() ? prompt.arguments.trim() : undefined,
        };
    }

    const pickString = (...keys: string[]): string | undefined => {
        for (const k of keys) {
            const v = args[k];
            if (typeof v === 'string' && v.trim()) return v.trim();
        }
        return undefined;
    };

    const command = pickString('command', 'cmd', 'script');
    if (command) {
        return { headline: `${prompt.tool} — run command`, target: command };
    }
    const filePath = pickString('path', 'file_path', 'filePath', 'target_path', 'target', 'cwd', 'directory');
    if (filePath) {
        return { headline: `${prompt.tool} — ${filePath}`, target: filePath };
    }
    const url = pickString('url', 'uri', 'endpoint', 'href');
    if (url) {
        return { headline: `${prompt.tool} — ${url}`, target: url };
    }
    const query = pickString('query', 'pattern', 'search', 'prompt');
    if (query) {
        return { headline: `${prompt.tool} — ${query}`, target: query };
    }

    const previewPairs = Object.entries(args)
        .slice(0, 3)
        .map(([k, v]) => {
            const rendered = typeof v === 'string' ? v : JSON.stringify(v);
            const clipped = rendered.length > 80 ? `${rendered.slice(0, 77)}…` : rendered;
            return `${k}: ${clipped}`;
        });
    return {
        headline: prompt.tool,
        target: previewPairs.length ? previewPairs.join(' · ') : undefined,
    };
}

export async function listV2Conversations(signal?: AbortSignal): Promise<ChatConversation[]> {
    return parseChatConversations(await api('/api/chat/conversations', { signal }));
}

export async function createV2Conversation(
    body: { title?: string; selectedModelSource?: string; selectedModelId?: string } = {},
    signal?: AbortSignal,
): Promise<ChatConversation> {
    return parseChatConversation(
        await api('/api/chat/conversations', {
            method: 'POST',
            body: JSON.stringify(body),
            signal,
        }),
    );
}

export async function getV2Conversation(id: string, signal?: AbortSignal): Promise<ChatConversation> {
    return parseChatConversation(await api(conversationUrl(id), { signal }));
}

export async function listV2ModelChoices(signal?: AbortSignal): Promise<V2ModelChoice[]> {
    const res = await api<{
        profiles?: Array<{
            id?: string;
            displayName?: string;
            modelName?: string;
            enabled?: boolean;
            selected?: boolean;
            capabilities?: string[];
            endpoint?: { id?: string; enabled?: boolean };
        }>;
    }>('/api/model-profiles', { signal });
    if (!Array.isArray(res.profiles)) return [];
    return res.profiles
        .filter(
            p =>
                p.id &&
                p.enabled !== false &&
                p.endpoint?.enabled !== false &&
                (!p.capabilities || p.capabilities.includes('chat')),
        )
        .map(p => ({
            id: String(p.id),
            displayName: String(p.displayName || p.modelName || p.id),
            modelName: String(p.modelName || p.id),
            endpointId: String(p.endpoint?.id || ''),
            selected: p.selected === true,
        }));
}

function extractUsageFromEvents(events: unknown[]): V2TokenUsage {
    let total = emptyV2TokenUsage();
    // WebHost emits `chat_assistant` right after `AgentEngine`'s `turn_completed`
    // for the same turn. Deduplicate paired events so turn tokens are never counted twice.
    let lastUsageKind: 'turn_completed' | 'chat_assistant' | null = null;
    for (const raw of events) {
        if (!raw || typeof raw !== 'object') continue;
        const ev = raw as Record<string, unknown>;
        const kind = String(ev.kind ?? ev.type ?? '');
        if (kind === 'user_message' || kind === 'chat_user' || kind === 'turn_started') {
            lastUsageKind = null;
            continue;
        }
        const payload = (ev.payload && typeof ev.payload === 'object' ? ev.payload : ev) as Record<string, unknown>;
        const usage = parseV2TokenUsage(payload.usage ?? ev.usage);
        if (!usage) continue;
        if (kind === 'turn_completed') {
            if (lastUsageKind === 'chat_assistant') {
                lastUsageKind = null;
                continue;
            }
            total = mergeV2TokenUsage(total, usage);
            lastUsageKind = 'turn_completed';
        } else if (kind === 'chat_assistant' || kind === 'assistant_message') {
            if (lastUsageKind === 'turn_completed') {
                lastUsageKind = null;
                continue;
            }
            total = mergeV2TokenUsage(total, usage);
            lastUsageKind = 'chat_assistant';
        } else {
            total = mergeV2TokenUsage(total, usage);
            lastUsageKind = null;
        }
    }
    return total;
}

export async function loadV2MessagesWithUsage(
    id: string,
    signal?: AbortSignal,
): Promise<{ messages: V2Message[]; usage: V2TokenUsage }> {
    const res = await api<{ messages?: unknown[]; events?: unknown[]; usage?: unknown }>(
        `${conversationUrl(id)}/messages?limit=200`,
        { signal },
    );
    const messages = Array.isArray(res.messages)
        ? res.messages.map((raw, idx) => {
              const m = (raw && typeof raw === 'object' ? raw : {}) as Record<string, unknown>;
              const role: V2Message['role'] =
                  m.role === 'user' || m.role === 'assistant' || m.role === 'system' ? m.role : 'assistant';
              const rawTools = Array.isArray(m.toolCalls) ? m.toolCalls : [];
              const toolCalls: V2ToolCall[] = rawTools.map((t, ti) => {
                  const o = (t && typeof t === 'object' ? t : {}) as Record<string, unknown>;
                  return {
                      callId: typeof o.callId === 'string' ? o.callId : `tool-${idx}-${ti}`,
                      name: typeof o.name === 'string' ? o.name : 'tool',
                      arguments: o.arguments,
                      status: o.ok === false ? 'failed' : 'completed',
                      summary: typeof o.summary === 'string' ? o.summary : '',
                  };
              });
              const uiSurfaces = Array.isArray(m.uiSurfaces) ? (m.uiSurfaces as UiSnapshot[]) : [];
              const attachments = Array.isArray(m.attachments) ? (m.attachments as V2Attachment[]) : [];
              return {
                  id: typeof m.id === 'string' ? m.id : `msg-${idx}`,
                  seq: typeof m.seq === 'number' ? m.seq : undefined,
                  role,
                  content: typeof m.content === 'string' ? m.content : '',
                  reasoning: typeof m.reasoning === 'string' && m.reasoning ? m.reasoning : undefined,
                  toolCalls: toolCalls.length ? toolCalls : undefined,
                  uiSurfaces: uiSurfaces.length ? uiSurfaces : undefined,
                  attachments: attachments.length ? attachments : undefined,
                  createdAtMs: typeof m.createdAtMs === 'number' ? m.createdAtMs : undefined,
              };
          })
        : [];
    const directUsage = parseV2TokenUsage(res.usage);
    const usage = directUsage ?? (Array.isArray(res.events) ? extractUsageFromEvents(res.events) : emptyV2TokenUsage());
    return { messages, usage };
}

export async function loadV2Messages(id: string, signal?: AbortSignal): Promise<V2Message[]> {
    const loaded = await loadV2MessagesWithUsage(id, signal);
    return loaded.messages;
}

export async function getV2ConversationConfiguration(
    conversationId: string,
    signal?: AbortSignal,
): Promise<V2ConversationConfiguration> {
    const res = await api<Record<string, unknown>>(`${conversationUrl(conversationId)}/configuration`, { signal });
    const rawKnowledge = res.knowledge;
    let knowledge = '';
    if (typeof rawKnowledge === 'string') {
        knowledge = rawKnowledge;
    } else if (rawKnowledge && typeof rawKnowledge === 'object') {
        const kObj = rawKnowledge as Record<string, unknown>;
        const nonEmpty = (value: unknown): string | undefined =>
            typeof value === 'string' && value.length > 0 ? value : undefined;
        knowledge =
            nonEmpty(kObj.keyPointsMarkdown) ??
            nonEmpty(kObj.wikiMarkdown) ??
            nonEmpty(kObj.summary) ??
            nonEmpty(kObj.points) ??
            nonEmpty(kObj.wiki) ??
            '';
    }
    const rawModelId = res.selectedModelId ?? res.selected_model_id;
    return {
        permissionProfile: normalizeV2PermissionProfile(res.permissionProfile ?? res.permission_profile),
        reasoningEffort: normalizeV2ReasoningEffort(res.reasoningEffort ?? res.reasoning_effort),
        knowledge,
        selectedModelId: typeof rawModelId === 'string' && rawModelId ? rawModelId : undefined,
        configurationRevision:
            typeof res.configurationRevision === 'number' ? res.configurationRevision : undefined,
    };
}

export async function patchV2ConversationConfiguration(
    conversationId: string,
    patch: {
        permissionProfile?: V2PermissionProfile;
        reasoningEffort?: V2ReasoningEffort;
        knowledge?: string | { keyPointsMarkdown?: string; wikiMarkdown?: string; summary?: string };
        selectedModelId?: string;
    },
    signal?: AbortSignal,
): Promise<{ status: string; snapshot?: Record<string, unknown> }> {
    return api<{ status: string; snapshot?: Record<string, unknown> }>(
        `${conversationUrl(conversationId)}/configuration`,
        {
            method: 'PATCH',
            body: JSON.stringify(patch),
            signal,
        },
    );
}

export async function getV2ConversationGoal(
    conversationId: string,
    signal?: AbortSignal,
): Promise<V2ConversationGoal | null> {
    const res = await api<{ goal?: unknown }>(`${conversationUrl(conversationId)}/goal`, { signal });
    const raw = res.goal;
    if (!raw || typeof raw !== 'object') return null;
    const g = raw as Record<string, unknown>;
    const title = String(g.title ?? g.text ?? g.goal ?? '').trim();
    const description = typeof g.description === 'string' ? g.description : '';
    if (!title && !description) return null;
    return {
        id: typeof g.id === 'string' ? g.id : undefined,
        title: title || description,
        description: description || undefined,
        status: typeof g.status === 'string' ? g.status : 'active',
        updatedAt: typeof g.updatedAt === 'string' ? g.updatedAt : undefined,
    };
}

export async function saveV2ConversationGoal(
    conversationId: string,
    goal: { title: string; description?: string; status?: string },
    signal?: AbortSignal,
): Promise<V2ConversationGoal | null> {
    const payload = JSON.stringify({
        title: goal.title.trim(),
        description: goal.description?.trim() ?? '',
        status: goal.status ?? 'active',
    });
    const url = `${conversationUrl(conversationId)}/goal`;
    const res = await api<{ goal?: unknown }>(url, { method: 'POST', body: payload, signal });
    const raw = res.goal;
    if (!raw || typeof raw !== 'object') {
        return { title: goal.title.trim(), description: goal.description?.trim() || undefined, status: goal.status ?? 'active' };
    }
    const g = raw as Record<string, unknown>;
    return {
        id: typeof g.id === 'string' ? g.id : undefined,
        title: String(g.title ?? goal.title).trim(),
        description: typeof g.description === 'string' && g.description ? g.description : undefined,
        status: typeof g.status === 'string' ? g.status : 'active',
        updatedAt: typeof g.updatedAt === 'string' ? g.updatedAt : undefined,
    };
}

export async function clearV2ConversationGoal(conversationId: string, signal?: AbortSignal): Promise<void> {
    await api(`${conversationUrl(conversationId)}/goal`, { method: 'DELETE', signal });
}

export async function getV2WorkspaceDiff(
    conversationId: string,
    signal?: AbortSignal,
): Promise<V2WorkspaceDiffSnapshot> {
    const raw = await api<Record<string, unknown>>(`${conversationUrl(conversationId)}/workspace-review`, { signal });
    const root = typeof raw.workspace_root === 'string' ? raw.workspace_root : typeof raw.root === 'string' ? raw.root : '';
    const isGitRepository = raw.is_git_repository !== false && raw.isGitRepository !== false;
    const review = (raw.review && typeof raw.review === 'object' ? raw.review : raw) as Record<string, unknown>;
    const inventory = (review.inventory && typeof review.inventory === 'object' ? review.inventory : {}) as Record<string, unknown>;
    const rawEntries = Array.isArray(inventory.entries) ? inventory.entries : Array.isArray(raw.entries) ? raw.entries : [];
    const entries = rawEntries
        .filter((e): e is Record<string, unknown> => !!e && typeof e === 'object')
        .map(e => ({
            path: String(e.path ?? ''),
            index: String(e.index_status ?? e.index ?? ''),
            worktree: String(e.worktree_status ?? e.worktree ?? ''),
        }))
        .filter(e => e.path);
    const rawFiles = Array.isArray(raw.files) ? raw.files : [];
    const files = rawFiles
        .filter((f): f is Record<string, unknown> => !!f && typeof f === 'object')
        .map(f => ({
            path: String(f.path ?? ''),
            size: Number(f.size ?? 0),
            isDir: f.is_dir === true || f.isDir === true,
        }))
        .filter(f => f.path);

    const readDiffText = (field: unknown): string => {
        if (typeof field === 'string') return field;
        if (field && typeof field === 'object' && typeof (field as Record<string, unknown>).text === 'string') {
            return String((field as Record<string, unknown>).text);
        }
        return '';
    };
    const stagedText = readDiffText(review.staged);
    const unstagedText = readDiffText(review.unstaged);
    const rawDiffText = typeof raw.diff === 'string' ? raw.diff : undefined;

    return { root, isGitRepository, entries, files, stagedText, unstagedText, rawDiffText };
}

export async function listV2WorkspaceCheckpoints(
    _conversationId: string,
    signal?: AbortSignal,
): Promise<V2WorkspaceCheckpoint[]> {
    const res = await api<{ checkpoints?: unknown[] }>('/api/workspace/checkpoints', { signal });
    const items = Array.isArray(res.checkpoints) ? res.checkpoints : [];
    return items.map((raw, idx) => {
        const o = (raw && typeof raw === 'object' ? raw : {}) as Record<string, unknown>;
        const id = String(o.id ?? o.checkpoint_id ?? `cp-${idx}`);
        const label = String(o.label ?? o.title ?? o.message ?? o.name ?? id);
        const summary = typeof o.summary === 'string' ? o.summary : typeof o.commit === 'string' ? o.commit : undefined;
        const createdAt =
            typeof o.createdAt === 'string'
                ? o.createdAt
                : typeof o.created_at === 'string'
                  ? o.created_at
                  : typeof o.created_at_ms === 'number'
                    ? new Date(o.created_at_ms).toLocaleString()
                    : undefined;
        return { id, label, summary, createdAt };
    });
}

export async function undoV2Workspace(
    _conversationId: string,
    signal?: AbortSignal,
): Promise<{ ok: boolean; message?: string }> {
    const res = await api<Record<string, unknown>>('/api/workspace/undo', {
        method: 'POST',
        body: '{}',
        signal,
    });
    return {
        ok: res.ok !== false,
        message: typeof res.stdout === 'string' ? res.stdout.trim() : typeof res.message === 'string' ? res.message : undefined,
    };
}

export async function redoV2Workspace(
    _conversationId: string,
    signal?: AbortSignal,
): Promise<{ ok: boolean; message?: string }> {
    const res = await api<Record<string, unknown>>('/api/workspace/redo', {
        method: 'POST',
        body: '{}',
        signal,
    });
    return {
        ok: res.ok !== false,
        message: typeof res.stdout === 'string' ? res.stdout.trim() : typeof res.message === 'string' ? res.message : undefined,
    };
}

export async function listV2ConversationDataSources(
    conversationId: string,
    signal?: AbortSignal,
): Promise<V2DataSourceItem[]> {
    const [globalRes, convRes] = await Promise.all([
        api<Record<string, unknown>>('/api/data-sources', { signal }),
        api<Record<string, unknown>>(`${conversationUrl(conversationId)}/data-sources`, { signal }).catch(() => ({
            sources: [],
        })),
    ]);

    const enabledSet = new Set<string>();
    const rawBound = (convRes as Record<string, unknown>).sources ?? (convRes as Record<string, unknown>).enabledIds ?? [];
    if (Array.isArray(rawBound)) {
        for (const entry of rawBound) {
            if (typeof entry === 'string') {
                enabledSet.add(entry);
            } else if (entry && typeof entry === 'object') {
                const o = entry as Record<string, unknown>;
                const id = String(o.id ?? o.sourceId ?? o.source_id ?? '');
                if (id && o.enabled !== false) enabledSet.add(id);
            }
        }
    } else if (rawBound && typeof rawBound === 'object') {
        const o = rawBound as Record<string, unknown>;
        const arr = Array.isArray(o.enabledIds)
            ? o.enabledIds
            : Array.isArray(o.sources)
              ? o.sources
              : Array.isArray(o.ids)
                ? o.ids
                : null;
        if (arr) {
            for (const entry of arr) {
                if (typeof entry === 'string') enabledSet.add(entry);
                else if (entry && typeof entry === 'object') {
                    const item = entry as Record<string, unknown>;
                    const id = String(item.id ?? item.sourceId ?? '');
                    if (id && item.enabled !== false) enabledSet.add(id);
                }
            }
        } else {
            for (const [k, v] of Object.entries(o)) {
                if (v === true) enabledSet.add(k);
            }
        }
    }

    const rawList = Array.isArray(globalRes.data_sources)
        ? globalRes.data_sources
        : Array.isArray(globalRes.items)
          ? globalRes.items
          : Array.isArray(globalRes.sources)
            ? globalRes.sources
            : [];

    return rawList
        .map((entry, idx) => {
            const wrapped = entry && typeof entry === 'object' ? (entry as Record<string, unknown>) : {};
            const src = (wrapped.source && typeof wrapped.source === 'object'
                ? wrapped.source
                : wrapped) as Record<string, unknown>;
            const id = String(src.id ?? `ds-${idx}`);
            return {
                id,
                name: String(src.name ?? src.title ?? id),
                kind: String(src.kind ?? 'localFile'),
                location: String(src.location ?? src.url ?? ''),
                accessPolicy: String(src.accessPolicy ?? 'conversationOptIn'),
                enabled: enabledSet.has(id),
            };
        })
        .filter(item => item.id);
}

export async function saveV2ConversationDataSources(
    conversationId: string,
    enabledIds: string[],
    signal?: AbortSignal,
): Promise<void> {
    const url = `${conversationUrl(conversationId)}/data-sources`;
    const body = JSON.stringify({ enabledIds, sources: enabledIds });
    await api(url, { method: 'POST', body, signal });
}

let cachedExperts: V2Expert[] | null = null;
let pendingExpertsRequest: Promise<V2Expert[]> | null = null;

export async function listV2Experts(forceRefresh = false, signal?: AbortSignal): Promise<V2Expert[]> {
    if (!forceRefresh && cachedExperts !== null) {
        return cachedExperts;
    }
    if (!forceRefresh && pendingExpertsRequest) {
        return pendingExpertsRequest;
    }
    const loader = (async () => {
        const res = await api<Record<string, unknown>>('/api/agent-rooms', { signal });
        const byId = new Map<string, V2Expert>();
        const ingest = (raw: unknown) => {
            if (!raw || typeof raw !== 'object') return;
            const o = raw as Record<string, unknown>;
            const id = String(o.id ?? o.agentId ?? o.agent_id ?? o.participantId ?? o.roomId ?? '').trim();
            const name = String(o.name ?? o.displayName ?? o.display_name ?? o.title ?? id).trim();
            if (id && name) {
                if (!byId.has(id)) {
                    byId.set(id, {
                        id,
                        name,
                        description: typeof o.description === 'string' ? o.description : typeof o.responsibilities === 'string' ? o.responsibilities : undefined,
                        role: typeof o.role === 'string' ? o.role : undefined,
                    });
                }
            }
        };

        if (Array.isArray(res.experts)) {
            for (const exp of res.experts) ingest(exp);
        }
        if (Array.isArray(res.roleTemplates)) {
            for (const tmpl of res.roleTemplates) ingest(tmpl);
        }
        if (Array.isArray(res.role_templates)) {
            for (const tmpl of res.role_templates) ingest(tmpl);
        }
        if (Array.isArray(res.rooms)) {
            for (const room of res.rooms) {
                if (!room || typeof room !== 'object') continue;
                const r = room as Record<string, unknown>;
                const nested = [
                    ...(Array.isArray(r.participants) ? r.participants : []),
                    ...(Array.isArray(r.agents) ? r.agents : []),
                    ...(Array.isArray(r.members) ? r.members : []),
                    ...(Array.isArray(r.experts) ? r.experts : []),
                ];
                if (nested.length > 0) {
                    for (const member of nested) ingest(member);
                } else {
                    ingest(r);
                }
            }
        }
        const list = Array.from(byId.values());
        cachedExperts = list;
        return list;
    })();

    pendingExpertsRequest = loader;
    try {
        return await loader;
    } finally {
        if (pendingExpertsRequest === loader) pendingExpertsRequest = null;
    }
}

export async function cancelV2Turn(conversationId: string): Promise<void> {
    await api(`${conversationUrl(conversationId)}/cancel`, { method: 'POST', body: '{}' });
}

export async function respondV2Permission(
    conversationId: string,
    requestId: string,
    allow: boolean,
    scope?: V2PermissionScope,
): Promise<void> {
    await api(`${conversationUrl(conversationId)}/permission`, {
        method: 'POST',
        body: JSON.stringify({
            request_id: requestId,
            allow,
            ...(allow && scope ? { scope } : {}),
        }),
    });
}

export async function respondV2UserInput(
    conversationId: string,
    requestId: string,
    answer: string | null,
): Promise<void> {
    await api(`${conversationUrl(conversationId)}/user-input`, {
        method: 'POST',
        body: JSON.stringify({ request_id: requestId, answer }),
    });
}

export async function truncateV2Conversation(conversationId: string, fromSeq: number): Promise<void> {
    await api(`${conversationUrl(conversationId)}/truncate`, {
        method: 'POST',
        body: JSON.stringify({ fromSeq }),
    });
}

export async function compactV2Conversation(conversationId: string): Promise<{ summary: string }> {
    return api<{ summary: string }>(`${conversationUrl(conversationId)}/compact`, {
        method: 'POST',
        body: '{}',
    });
}

export async function branchV2Conversation(conversationId: string): Promise<ChatConversation> {
    const res = await api<{ conversation: unknown }>(`${conversationUrl(conversationId)}/branch`, {
        method: 'POST',
        body: '{}',
    });
    return parseChatConversation(res.conversation);
}

export async function submitV2UiInteraction(
    conversationId: string,
    interaction: UiInteraction,
    requestId?: string,
    submittedAtMs?: number,
): Promise<{
    outcome: UiSubmissionOutcome;
    snapshot?: UiSnapshot;
    dataModel?: Record<string, unknown>;
    summary?: string;
    actionId?: string | null;
}> {
    try {
        const res = await api<{
            status?: string;
            snapshot?: UiSnapshot;
            dataModel?: Record<string, unknown>;
            data_model?: Record<string, unknown>;
            summary?: string;
            actionId?: string | null;
            error?: string;
        }>(`${conversationUrl(conversationId)}/ui-interactions`, {
            method: 'POST',
            body: JSON.stringify({
                ...interaction,
                ...(requestId ? { requestId, request_id: requestId } : {}),
                ...(typeof submittedAtMs === 'number' ? { submittedAtMs, submitted_at_ms: submittedAtMs } : {}),
            }),
        });
        if (res.status === 'committed') {
            return {
                outcome: { status: 'committed' },
                snapshot: res.snapshot,
                dataModel: res.dataModel ?? res.data_model ?? {},
                summary: res.summary,
                actionId: res.actionId,
            };
        }
        return {
            outcome: { status: 'rejected', message: res.error || 'UI interaction rejected' },
        };
    } catch (cause) {
        if (cause instanceof ApiError && cause.status === 409) {
            return { outcome: { status: 'conflict', message: cause.message } };
        }
        return {
            outcome: {
                status: 'rejected',
                message: cause instanceof Error ? cause.message : String(cause),
            },
        };
    }
}

export async function hasConfiguredChatModel(signal?: AbortSignal): Promise<boolean> {
    const [remote, local] = await Promise.allSettled([
        api<{ profiles?: Array<{ enabled?: boolean; endpoint?: { enabled?: boolean }; capabilities?: string[] }> }>(
            '/api/model-profiles',
            { signal },
        ),
        api<{ models?: Array<{ installed?: boolean }> }>('/api/local-models', { signal }),
    ]);
    const hasRemote =
        remote.status === 'fulfilled' &&
        Array.isArray(remote.value.profiles) &&
        remote.value.profiles.some(
            p => p.enabled !== false && p.endpoint?.enabled !== false && (!p.capabilities || p.capabilities.includes('chat')),
        );
    const hasLocal =
        local.status === 'fulfilled' &&
        Array.isArray(local.value.models) &&
        local.value.models.some(m => m.installed === true);
    return hasRemote || hasLocal;
}

export async function streamV2ChatMessage(
    conversationId: string,
    content: string,
    callbacks: V2StreamCallbacks,
    signal?: AbortSignal,
    options: V2SendOptions = {},
): Promise<void> {
    const response = await apiResponse(`${conversationUrl(conversationId)}/messages`, {
        method: 'POST',
        body: JSON.stringify({
            content,
            ...(options.modelId ? { modelId: options.modelId } : {}),
            ...(options.permissionProfile ? { permissionProfile: options.permissionProfile } : {}),
            ...(options.reasoningEffort ? { reasoningEffort: options.reasoningEffort } : {}),
            ...(options.attachments?.length ? { attachments: options.attachments } : {}),
            ...(options.selectedExperts?.length ? { selected_experts: options.selectedExperts } : {}),
            ...(options.uiAction ? { ui_action: options.uiAction } : {}),
        }),
        signal,
        headers: { Accept: 'text/event-stream' },
    });
    if (!response.ok) {
        const body = await response.json().catch(() => ({} as Record<string, unknown>));
        throw new ApiError(
            response.status,
            String(body.error || body.detail || `HTTP ${response.status}`),
            typeof body.code === 'string' ? body.code : undefined,
        );
    }
    const reader = response.body?.getReader();
    if (!reader) return;
    const decoder = new TextDecoder();
    let buffer = '';

    const handleFrame = (frame: string) => {
        const dataLines = frame
            .split(/\r?\n/)
            .filter(line => line.startsWith('data:'))
            .map(line => line.slice(5).trimStart());
        if (!dataLines.length) return;
        const payload = dataLines.join('\n').trim();
        if (!payload || payload === '[DONE]') return;
        let ev: Record<string, unknown>;
        try {
            ev = JSON.parse(payload) as Record<string, unknown>;
        } catch {
            return;
        }
        const type = String(ev.type ?? '');
        switch (type) {
            case 'session_started':
                if (typeof ev.session_id === 'string') callbacks.onSessionStarted?.(ev.session_id);
                break;
            case 'text_delta':
                if (typeof ev.delta === 'string') callbacks.onTextDelta?.(ev.delta);
                break;
            case 'reasoning_delta':
                if (typeof ev.delta === 'string') callbacks.onReasoningDelta?.(ev.delta);
                break;
            case 'tool_started':
                callbacks.onToolStarted?.(
                    String(ev.call_id ?? ''),
                    String(ev.name ?? 'tool'),
                    ev.arguments,
                );
                break;
            case 'tool_finished':
                callbacks.onToolFinished?.(
                    String(ev.call_id ?? ''),
                    String(ev.name ?? 'tool'),
                    ev.is_error !== true && ev.ok !== false,
                    String(ev.summary ?? ''),
                );
                break;
            case 'ui_surface':
                if (ev.snapshot && typeof ev.snapshot === 'object') {
                    callbacks.onUiSurface?.(ev.snapshot as UiSnapshot);
                }
                break;
            case 'permission_requested':
                if (typeof ev.request_id === 'string') {
                    callbacks.onPermissionRequested?.({
                        requestId: ev.request_id,
                        tool: String(ev.tool ?? 'tool'),
                        reason: String(ev.reason ?? 'Requires user confirmation'),
                        arguments: ev.arguments,
                    });
                }
                break;
            case 'user_question':
                if (typeof ev.request_id === 'string') {
                    callbacks.onUserQuestion?.({
                        requestId: ev.request_id,
                        question: String(ev.question ?? ''),
                        options: Array.isArray(ev.options) ? ev.options.map(String) : [],
                    });
                }
                break;
            case 'compacted':
                callbacks.onCompacted?.(String(ev.summary ?? ''));
                break;
            case 'turn_completed':
                callbacks.onTurnCompleted?.(
                    typeof ev.final_text === 'string' ? ev.final_text : undefined,
                    parseV2TokenUsage(ev.usage) ?? undefined,
                );
                break;
            case 'error':
                callbacks.onError?.(String(ev.message ?? ev.error ?? 'Unknown error'));
                break;
        }
    };

    try {
        while (true) {
            const { value, done } = await reader.read();
            if (done) break;
            buffer += decoder.decode(value, { stream: true });
            const parts = buffer.split(/\r?\n\r?\n/);
            buffer = parts.pop() ?? '';
            for (const part of parts) handleFrame(part);
        }
        if (buffer.trim()) handleFrame(buffer);
    } finally {
        reader.releaseLock();
    }
}
