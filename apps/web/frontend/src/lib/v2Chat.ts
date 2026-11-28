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
    systemPrompt?: string;
    modelOverride?: string;
    allowedTools?: string[];
    color?: string;
    enabled?: boolean;
}

export interface V2AgentRoom {
    id: string;
    name: string;
    workspaceRoot?: string;
    participantExpertIds: string[];
    sessionId?: string;
    createdAtMs?: number;
    updatedAtMs?: number;
}

export interface V2AgentRoomsDirectory {
    hostId: string;
    rooms: V2AgentRoom[];
    experts: V2Expert[];
    roleTemplates: Array<{ id: string; name: string; description?: string; role?: string }>;
}

export interface V2ExpertReply {
    expertId?: string;
    expertName: string;
    role?: string;
    content: string;
    status?: string;
    color?: string;
    createdAtMs?: number;
}

export interface V2Artifact {
    id: string;
    title: string;
    kind: string;
    path?: string;
    mimeType?: string;
    size?: number;
    content?: string;
    preview?: string;
    createdAt?: string;
    messageId?: string;
}

export interface V2RecallHit {
    messageId: string;
    seq?: number;
    ordinal?: number;
    role: string;
    snippet: string;
    timestamp?: string;
}

export interface V2DocumentSearchHit {
    id: string;
    kind: 'document' | 'memory' | 'file';
    title: string;
    snippet?: string;
    path?: string;
    sourceId?: string;
    score?: number;
}

export interface V2ConversationPlanStep {
    id?: string;
    title: string;
    status?: string;
    detail?: string;
}

export interface V2ConversationPlan {
    enabled: boolean;
    status: 'idle' | 'draft' | 'proposed' | 'pending' | 'awaiting_approval' | 'approved' | 'rejected' | 'executing' | 'completed';
    summary?: string;
    markdown?: string;
    steps: V2ConversationPlanStep[];
    updatedAt?: string;
}

export interface V2SubagentTask {
    id: string;
    parentId?: string;
    title: string;
    status: 'queued' | 'running' | 'completed' | 'failed' | 'cancelled' | 'waiting';
    kind?: string;
    agent?: string;
    detail?: string;
    progress?: number;
    updatedAt?: string;
}

export interface V2GitWorktree {
    id: string;
    path: string;
    branch: string;
    head?: string;
    isMain: boolean;
    detached?: boolean;
    locked?: boolean;
    prunable?: boolean;
    dirty?: boolean;
    status?: string;
}

export interface V2TurnObservation {
    active: boolean;
    needsResponse?: boolean;
    status?: string;
    runId?: string;
    text?: string;
    reasoning?: string;
    toolCalls?: V2ToolCall[];
    pendingPermission?: V2PermissionPrompt | null;
    pendingQuestion?: V2UserQuestion | null;
    usage?: V2TokenUsage | null;
    step?: string;
}

export interface V2BrowserState {
    managedAvailable: boolean;
    managedConnected: boolean;
    nativeConnected: boolean;
    previewUrl?: string;
    pages: Array<{ id: string; title: string; url: string; active?: boolean }>;
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
    blocked: boolean;
}

export interface V2ConversationConfiguration {
    permissionProfile: V2PermissionProfile;
    reasoningEffort: V2ReasoningEffort;
    knowledge: string;
    keyPointsMarkdown: string;
    wikiMarkdown: string;
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
    expertId?: string;
    expertName?: string;
    expertColor?: string;
    expertReplies?: V2ExpertReply[];
    createdAtMs?: number;
}

export interface V2ModelChoice {
    id: string;
    displayName: string;
    modelName: string;
    endpointId: string;
    selected: boolean;
    source?: 'remote' | 'local';
}

export interface V2SendOptions {
    modelId?: string;
    modelSource?: string;
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
    onExpertReply?: (reply: V2ExpertReply) => void;
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
        return { headline: prompt.tool, target: command };
    }
    const filePath = pickString('path', 'file_path', 'filePath', 'target_path', 'target', 'cwd', 'directory');
    if (filePath) {
        return { headline: prompt.tool, target: filePath };
    }
    const url = pickString('url', 'uri', 'endpoint', 'href');
    if (url) {
        return { headline: prompt.tool, target: url };
    }
    const query = pickString('query', 'pattern', 'search', 'prompt');
    if (query) {
        return { headline: prompt.tool, target: query };
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
    body: {
        title?: string;
        selectedModelSource?: string;
        selectedModelId?: string;
        permissionProfile?: V2PermissionProfile;
        reasoningEffort?: V2ReasoningEffort;
    } = {},
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
    const [remoteRes, localRes] = await Promise.allSettled([
        api<{
            profiles?: Array<{
                id?: string;
                displayName?: string;
                modelName?: string;
                enabled?: boolean;
                selected?: boolean;
                capabilities?: string[];
                endpoint?: { id?: string; enabled?: boolean };
            }>;
        }>('/api/model-profiles', { signal }),
        api<{
            models?: Array<{
                id?: string;
                displayName?: string;
                name?: string;
                runtime?: string;
                installed?: boolean;
                selected?: boolean;
            }>;
        }>('/api/local-models', { signal }),
    ]);

    const choices: V2ModelChoice[] = [];
    const seen = new Set<string>();

    if (remoteRes.status === 'fulfilled' && Array.isArray(remoteRes.value.profiles)) {
        for (const p of remoteRes.value.profiles) {
            if (
                !p.id ||
                p.enabled === false ||
                p.endpoint?.enabled === false ||
                (p.capabilities && !p.capabilities.includes('chat'))
            ) {
                continue;
            }
            const id = String(p.id);
            if (seen.has(id)) continue;
            seen.add(id);
            choices.push({
                id,
                displayName: String(p.displayName || p.modelName || p.id),
                modelName: String(p.modelName || p.id),
                endpointId: String(p.endpoint?.id || ''),
                selected: p.selected === true,
                source: 'remote',
            });
        }
    }

    if (localRes.status === 'fulfilled' && Array.isArray(localRes.value.models)) {
        for (const m of localRes.value.models) {
            if (!m.id || m.installed !== true) continue;
            const id = String(m.id);
            if (seen.has(id)) continue;
            seen.add(id);
            choices.push({
                id,
                displayName: String(m.displayName || m.name || m.id),
                modelName: String(m.name || m.id),
                endpointId: String(m.runtime || 'local'),
                selected: m.selected === true,
                source: 'local',
            });
        }
    }

    return choices;
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
              const rawExpertReplies = Array.isArray(m.expertReplies)
                  ? m.expertReplies
                  : Array.isArray(m.expert_replies)
                    ? m.expert_replies
                    : Array.isArray(m.roomReplies)
                      ? m.roomReplies
                      : Array.isArray(m.room_replies)
                        ? m.room_replies
                        : [];
              const expertReplies: V2ExpertReply[] = rawExpertReplies
                  .filter((r): r is Record<string, unknown> => !!r && typeof r === 'object')
                  .map(r => ({
                      expertId: typeof r.expertId === 'string' ? r.expertId : typeof r.expert_id === 'string' ? r.expert_id : undefined,
                      expertName: String(r.expertName ?? r.expert_name ?? r.name ?? r.speaker ?? 'Expert'),
                      role: typeof r.role === 'string' ? r.role : undefined,
                      content: String(r.content ?? r.text ?? r.reply ?? ''),
                      status: typeof r.status === 'string' ? r.status : undefined,
                      color: typeof r.color === 'string' ? r.color : undefined,
                      createdAtMs: typeof r.createdAtMs === 'number' ? r.createdAtMs : typeof r.created_at_ms === 'number' ? r.created_at_ms : undefined,
                  }))
                  .filter(r => r.content || r.expertName);
              const expertId = typeof m.expertId === 'string' ? m.expertId : typeof m.expert_id === 'string' ? m.expert_id : undefined;
              const expertName = typeof m.expertName === 'string' ? m.expertName : typeof m.expert_name === 'string' ? m.expert_name : undefined;
              const expertColor = typeof m.expertColor === 'string' ? m.expertColor : typeof m.expert_color === 'string' ? m.expert_color : undefined;
              return {
                  id: typeof m.id === 'string' ? m.id : `msg-${idx}`,
                  seq: typeof m.seq === 'number' ? m.seq : undefined,
                  role,
                  content: typeof m.content === 'string' ? m.content : '',
                  reasoning: typeof m.reasoning === 'string' && m.reasoning ? m.reasoning : undefined,
                  toolCalls: toolCalls.length ? toolCalls : undefined,
                  uiSurfaces: uiSurfaces.length ? uiSurfaces : undefined,
                  attachments: attachments.length ? attachments : undefined,
                  expertId,
                  expertName,
                  expertColor,
                  expertReplies: expertReplies.length ? expertReplies : undefined,
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
    let keyPointsMarkdown = '';
    let wikiMarkdown = '';
    if (typeof rawKnowledge === 'string') {
        keyPointsMarkdown = rawKnowledge;
    } else if (rawKnowledge && typeof rawKnowledge === 'object') {
        const kObj = rawKnowledge as Record<string, unknown>;
        keyPointsMarkdown =
            typeof kObj.keyPointsMarkdown === 'string'
                ? kObj.keyPointsMarkdown
                : typeof kObj.summary === 'string'
                  ? kObj.summary
                  : typeof kObj.points === 'string'
                    ? kObj.points
                    : '';
        wikiMarkdown =
            typeof kObj.wikiMarkdown === 'string'
                ? kObj.wikiMarkdown
                : typeof kObj.wiki === 'string'
                  ? kObj.wiki
                  : '';
    }
    const rawModelId = res.selectedModelId ?? res.selected_model_id;
    return {
        permissionProfile: normalizeV2PermissionProfile(res.permissionProfile ?? res.permission_profile),
        reasoningEffort: normalizeV2ReasoningEffort(res.reasoningEffort ?? res.reasoning_effort),
        knowledge: keyPointsMarkdown,
        keyPointsMarkdown,
        wikiMarkdown,
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
        selectedModelSource?: string;
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
    conversationId: string,
    signal?: AbortSignal,
): Promise<V2WorkspaceCheckpoint[]> {
    const res = await api<{ checkpoints?: unknown[] }>(
        `${conversationUrl(conversationId)}/workspace/checkpoints`,
        { signal },
    );
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
    conversationId: string,
    signal?: AbortSignal,
): Promise<{ ok: boolean; message?: string }> {
    const res = await api<Record<string, unknown>>(`${conversationUrl(conversationId)}/workspace/undo`, {
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
    conversationId: string,
    signal?: AbortSignal,
): Promise<{ ok: boolean; message?: string }> {
    const res = await api<Record<string, unknown>>(`${conversationUrl(conversationId)}/workspace/redo`, {
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
    const disabledSet = new Set<string>();
    const convRecord = convRes as Record<string, unknown>;
    const topDisabled = convRecord.disabledIds ?? convRecord.disabled_ids;
    if (Array.isArray(topDisabled)) {
        for (const entry of topDisabled) {
            if (typeof entry === 'string' && entry) disabledSet.add(entry);
        }
    }
    const rawBound = convRecord.sources ?? convRecord.enabledIds ?? [];
    if (Array.isArray(rawBound)) {
        for (const entry of rawBound) {
            if (typeof entry === 'string') {
                enabledSet.add(entry);
            } else if (entry && typeof entry === 'object') {
                const o = entry as Record<string, unknown>;
                const id = String(o.id ?? o.sourceId ?? o.source_id ?? '');
                if (!id) continue;
                if (o.enabled === false) disabledSet.add(id);
                else enabledSet.add(id);
            }
        }
    } else if (rawBound && typeof rawBound === 'object') {
        const o = rawBound as Record<string, unknown>;
        const nestedDisabled = o.disabledIds ?? o.disabled_ids;
        if (Array.isArray(nestedDisabled)) {
            for (const entry of nestedDisabled) {
                if (typeof entry === 'string' && entry) disabledSet.add(entry);
            }
        }
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
                    const id = String(item.id ?? item.sourceId ?? item.source_id ?? '');
                    if (!id) continue;
                    if (item.enabled === false) disabledSet.add(id);
                    else enabledSet.add(id);
                }
            }
        } else {
            for (const [k, v] of Object.entries(o)) {
                if (v === true) enabledSet.add(k);
                else if (v === false) disabledSet.add(k);
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
            const accessPolicy = String(src.accessPolicy ?? src.access_policy ?? 'conversationOptIn');
            const blocked = accessPolicy === 'blocked';
            const enabled = blocked
                ? false
                : accessPolicy === 'defaultEnabled'
                  ? !disabledSet.has(id)
                  : enabledSet.has(id);
            return {
                id,
                name: String(src.name ?? src.title ?? id),
                kind: String(src.kind ?? 'localFile'),
                location: String(src.location ?? src.url ?? ''),
                accessPolicy,
                enabled,
                blocked,
            };
        })
        .filter(item => item.id);
}

export async function saveV2ConversationDataSources(
    conversationId: string,
    enabledIds: string[],
    disabledIds: string[] = [],
    signal?: AbortSignal,
): Promise<void> {
    const url = `${conversationUrl(conversationId)}/data-sources`;
    const body = JSON.stringify({ enabledIds, disabledIds, sources: enabledIds });
    await api(url, { method: 'POST', body, signal });
}

export async function parseV2Attachment(
    payload: { filename: string; mimeType: string; dataBase64: string },
    signal?: AbortSignal,
): Promise<{ text: string }> {
    const res = await api<{ text?: string; content?: string }>('/api/attachments/parse', {
        method: 'POST',
        body: JSON.stringify({
            filename: payload.filename,
            mimeType: payload.mimeType,
            dataBase64: payload.dataBase64,
        }),
        signal,
    });
    return { text: typeof res.text === 'string' ? res.text : typeof res.content === 'string' ? res.content : '' };
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
            ...(options.modelSource !== undefined
                ? { selectedModelSource: options.modelSource, modelSource: options.modelSource }
                : {}),
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
                        reason: typeof ev.reason === 'string' ? ev.reason : '',
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
            case 'expert_reply':
            case 'room_reply': {
                const payloadObj = (ev.reply && typeof ev.reply === 'object' ? ev.reply : ev) as Record<string, unknown>;
                const content = String(payloadObj.content ?? payloadObj.text ?? payloadObj.reply ?? '');
                const expertName = String(payloadObj.expertName ?? payloadObj.expert_name ?? payloadObj.name ?? 'Expert');
                if (content || expertName) {
                    callbacks.onExpertReply?.({
                        expertId: typeof payloadObj.expertId === 'string' ? payloadObj.expertId : typeof payloadObj.expert_id === 'string' ? payloadObj.expert_id : undefined,
                        expertName,
                        role: typeof payloadObj.role === 'string' ? payloadObj.role : undefined,
                        content,
                        status: typeof payloadObj.status === 'string' ? payloadObj.status : 'completed',
                        color: typeof payloadObj.color === 'string' ? payloadObj.color : undefined,
                        createdAtMs: Date.now(),
                    });
                }
                break;
            }
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

function parseExpertItem(raw: unknown): V2Expert | null {
    if (!raw || typeof raw !== 'object') return null;
    const o = raw as Record<string, unknown>;
    const id = String(o.id ?? o.expert_id ?? o.agentId ?? o.agent_id ?? '').trim();
    const name = String(o.name ?? o.displayName ?? o.display_name ?? o.title ?? id).trim();
    if (!id || !name) return null;
    return {
        id,
        name,
        description: typeof o.description === 'string' ? o.description : typeof o.responsibilities === 'string' ? o.responsibilities : undefined,
        role: typeof o.role === 'string' ? o.role : undefined,
        systemPrompt: typeof o.systemPrompt === 'string' ? o.systemPrompt : typeof o.system_prompt === 'string' ? o.system_prompt : undefined,
        modelOverride: typeof o.modelOverride === 'string' ? o.modelOverride : typeof o.model_override === 'string' ? o.model_override : undefined,
        allowedTools: Array.isArray(o.allowedTools)
            ? o.allowedTools.map(String)
            : Array.isArray(o.allowed_tools)
              ? o.allowed_tools.map(String)
              : undefined,
        color: typeof o.color === 'string' ? o.color : undefined,
        enabled: o.enabled !== false,
    };
}

function parseRoomItem(raw: unknown): V2AgentRoom | null {
    if (!raw || typeof raw !== 'object') return null;
    const o = raw as Record<string, unknown>;
    const id = String(o.id ?? o.room_id ?? o.roomId ?? '').trim();
    const name = String(o.name ?? o.title ?? id).trim();
    if (!id) return null;
    const rawIds = Array.isArray(o.participant_expert_ids)
        ? o.participant_expert_ids
        : Array.isArray(o.participantExpertIds)
          ? o.participantExpertIds
          : Array.isArray(o.participants)
            ? o.participants.map(p => (typeof p === 'string' ? p : String((p as Record<string, unknown>)?.id ?? '')))
            : [];
    return {
        id,
        name: name || id,
        workspaceRoot: typeof o.workspace_root === 'string' ? o.workspace_root : typeof o.workspaceRoot === 'string' ? o.workspaceRoot : undefined,
        participantExpertIds: rawIds.map(String).filter(Boolean),
        sessionId: typeof o.session_id === 'string' ? o.session_id : typeof o.sessionId === 'string' ? o.sessionId : undefined,
        createdAtMs: typeof o.created_at_ms === 'number' ? o.created_at_ms : typeof o.createdAtMs === 'number' ? o.createdAtMs : undefined,
        updatedAtMs: typeof o.updated_at_ms === 'number' ? o.updated_at_ms : typeof o.updatedAtMs === 'number' ? o.updatedAtMs : undefined,
    };
}

export function invalidateV2ExpertsCache(): void {
    cachedExperts = null;
}

export async function fetchV2AgentRoomsDirectory(signal?: AbortSignal): Promise<V2AgentRoomsDirectory> {
    const res = await api<Record<string, unknown>>('/api/agent-rooms', { signal });
    const hostId = String(res.host_id ?? res.hostId ?? 'local');
    const rooms = (Array.isArray(res.rooms) ? res.rooms : []).map(parseRoomItem).filter((r): r is V2AgentRoom => !!r);
    const experts = (Array.isArray(res.experts) ? res.experts : []).map(parseExpertItem).filter((e): e is V2Expert => !!e);
    const rawTemplates = Array.isArray(res.roleTemplates)
        ? res.roleTemplates
        : Array.isArray(res.role_templates)
          ? res.role_templates
          : [];
    const roleTemplates = rawTemplates
        .filter((t): t is Record<string, unknown> => !!t && typeof t === 'object')
        .map(t => ({
            id: String(t.id ?? ''),
            name: String(t.name ?? t.title ?? t.id ?? ''),
            description: typeof t.description === 'string' ? t.description : undefined,
            role: typeof t.role === 'string' ? t.role : undefined,
        }))
        .filter(t => t.id);
    cachedExperts = experts;
    return { hostId, rooms, experts, roleTemplates };
}

export async function upsertV2Expert(
    input: {
        id?: string;
        name: string;
        role?: string;
        description?: string;
        systemPrompt?: string;
        modelOverride?: string;
        allowedTools?: string[];
        color?: string;
    },
    signal?: AbortSignal,
): Promise<V2AgentRoomsDirectory> {
    const payload = {
        command: {
            type: 'configure_agent',
            input: {
                ...(input.id ? { id: input.id, expert_id: input.id } : {}),
                name: input.name.trim(),
                role: input.role?.trim() || 'specialist',
                description: input.description?.trim() ?? '',
                system_prompt: input.systemPrompt?.trim() ?? '',
                ...(input.modelOverride?.trim() ? { model_override: input.modelOverride.trim() } : {}),
                ...(input.allowedTools ? { allowed_tools: input.allowedTools } : {}),
                ...(input.color ? { color: input.color } : {}),
            },
        },
    };
    await api('/api/agent-rooms', { method: 'POST', body: JSON.stringify(payload), signal });
    invalidateV2ExpertsCache();
    return fetchV2AgentRoomsDirectory(signal);
}

export async function deleteV2Expert(expertId: string, signal?: AbortSignal): Promise<V2AgentRoomsDirectory> {
    try {
        await api(`/api/agent-rooms/experts/${encodeURIComponent(expertId)}`, { method: 'DELETE', signal });
    } catch (cause) {
        if (cause instanceof ApiError && (cause.status === 404 || cause.status === 405)) {
            await api('/api/agent-rooms', {
                method: 'POST',
                body: JSON.stringify({
                    command: { type: 'delete_agent', input: { id: expertId, expert_id: expertId } },
                }),
                signal,
            });
        } else {
            throw cause;
        }
    }
    invalidateV2ExpertsCache();
    return fetchV2AgentRoomsDirectory(signal);
}

export async function createV2AgentRoom(
    input: { name: string; participantExpertIds: string[]; sessionId?: string },
    signal?: AbortSignal,
): Promise<V2AgentRoomsDirectory> {
    await api('/api/agent-rooms', {
        method: 'POST',
        body: JSON.stringify({
            command: {
                type: 'create_room',
                input: {
                    name: input.name.trim(),
                    participant_expert_ids: input.participantExpertIds,
                    ...(input.sessionId ? { session_id: input.sessionId } : {}),
                },
            },
            name: input.name.trim(),
            participant_expert_ids: input.participantExpertIds,
            ...(input.sessionId ? { session_id: input.sessionId } : {}),
        }),
        signal,
    });
    invalidateV2ExpertsCache();
    return fetchV2AgentRoomsDirectory(signal);
}

export async function updateV2AgentRoom(
    roomId: string,
    input: { name?: string; participantExpertIds?: string[]; sessionId?: string },
    signal?: AbortSignal,
): Promise<V2AgentRoomsDirectory> {
    const restBody = {
        ...(input.name !== undefined ? { name: input.name.trim() } : {}),
        ...(input.participantExpertIds !== undefined ? { participant_expert_ids: input.participantExpertIds } : {}),
        ...(input.sessionId !== undefined ? { session_id: input.sessionId } : {}),
    };
    await api('/api/agent-rooms', {
        method: 'POST',
        body: JSON.stringify({
            command: {
                type: 'update_room',
                input: { id: roomId, room_id: roomId, ...restBody },
            },
        }),
        signal,
    });
    invalidateV2ExpertsCache();
    return fetchV2AgentRoomsDirectory(signal);
}

export async function deleteV2AgentRoom(roomId: string, signal?: AbortSignal): Promise<V2AgentRoomsDirectory> {
    try {
        await api(`/api/agent-rooms/${encodeURIComponent(roomId)}`, { method: 'DELETE', signal });
    } catch (cause) {
        if (cause instanceof ApiError && (cause.status === 404 || cause.status === 405)) {
            await api('/api/agent-rooms', {
                method: 'POST',
                body: JSON.stringify({
                    command: { type: 'delete_room', input: { id: roomId, room_id: roomId } },
                }),
                signal,
            });
        } else {
            throw cause;
        }
    }
    invalidateV2ExpertsCache();
    return fetchV2AgentRoomsDirectory(signal);
}

export async function attachV2ConversationToRoom(
    conversationId: string,
    roomId: string | null,
    signal?: AbortSignal,
    currentRoomId?: string | null,
): Promise<V2AgentRoom | null> {
    if (!roomId) {
        let targetRoomId = currentRoomId ?? null;
        if (!targetRoomId) {
            const dir = await fetchV2AgentRoomsDirectory(signal);
            targetRoomId = dir.rooms.find(r => r.sessionId === conversationId)?.id ?? null;
        }
        if (!targetRoomId) return null;
        await api<Record<string, unknown>>('/api/agent-rooms', {
            method: 'POST',
            body: JSON.stringify({
                command: {
                    type: 'update_room',
                    input: { id: targetRoomId, detach: true, session_id: null },
                },
            }),
            signal,
        });
        invalidateV2ExpertsCache();
        return null;
    }
    const res = await api<Record<string, unknown>>('/api/agent-rooms', {
        method: 'POST',
        body: JSON.stringify({
            command: {
                type: 'update_room',
                input: { id: roomId, room_id: roomId, session_id: conversationId, conversation_id: conversationId },
            },
        }),
        signal,
    });
    invalidateV2ExpertsCache();
    const result = (res.result && typeof res.result === 'object' ? res.result : res) as Record<string, unknown>;
    return parseRoomItem(result.room ?? res.item);
}

export async function sendV2RoomMessage(
    roomId: string,
    content: string,
    targetRecipient?: string,
    signal?: AbortSignal,
): Promise<V2ExpertReply[]> {
    const res = await api<{ replies?: unknown[] }>(`/api/agent-rooms/${encodeURIComponent(roomId)}/messages`, {
        method: 'POST',
        body: JSON.stringify({
            content,
            ...(targetRecipient ? { to: targetRecipient } : {}),
        }),
        signal,
    });
    const raw = Array.isArray(res.replies) ? res.replies : [];
    return raw
        .filter((item): item is Record<string, unknown> => !!item && typeof item === 'object')
        .map(item => ({
            expertId: typeof item.expert_id === 'string' ? item.expert_id : typeof item.expertId === 'string' ? item.expertId : undefined,
            expertName: String(item.expert_name ?? item.expertName ?? item.name ?? item.speaker ?? 'Expert'),
            role: typeof item.role === 'string' ? item.role : undefined,
            content: String(item.content ?? item.reply ?? item.text ?? ''),
            status: typeof item.status === 'string' ? item.status : 'completed',
            createdAtMs: typeof item.created_at_ms === 'number' ? item.created_at_ms : Date.now(),
        }))
        .filter(r => r.content || r.expertName);
}

export async function listV2ConversationArtifacts(
    conversationId: string,
    signal?: AbortSignal,
): Promise<V2Artifact[]> {
    try {
        const res = await api<Record<string, unknown>>(`${conversationUrl(conversationId)}/artifacts`, { signal });
        const raw = Array.isArray(res.artifacts)
            ? res.artifacts
            : Array.isArray(res.items)
              ? res.items
              : Array.isArray(res)
                ? (res as unknown[])
                : [];
        return raw
            .filter((entry): entry is Record<string, unknown> => !!entry && typeof entry === 'object')
            .map((entry, idx) => {
                const item = (
                    entry.item && typeof entry.item === 'object' && !Array.isArray(entry.item)
                        ? entry.item
                        : entry
                ) as Record<string, unknown>;
                const createdMs =
                    typeof item.createdAtMs === 'number'
                        ? item.createdAtMs
                        : typeof item.created_at_ms === 'number'
                          ? item.created_at_ms
                          : typeof entry.createdAtMs === 'number'
                            ? entry.createdAtMs
                            : typeof entry.created_at_ms === 'number'
                              ? entry.created_at_ms
                              : undefined;
                const pathVal =
                    typeof item.path === 'string'
                        ? item.path
                        : typeof entry.path === 'string'
                          ? entry.path
                          : undefined;
                const identityObj =
                    item.identity && typeof item.identity === 'object'
                        ? (item.identity as Record<string, unknown>)
                        : null;
                return {
                    id: String(
                        entry.id ??
                            item.id ??
                            item.artifact_id ??
                            item.artifactId ??
                            identityObj?.surface_id ??
                            (entry.seq != null ? `art-seq-${entry.seq}-${idx}` : `art-${idx}`),
                    ),
                    title: String(
                        item.title ??
                            item.name ??
                            item.filename ??
                            item.path ??
                            entry.title ??
                            entry.name ??
                            entry.path ??
                            item.summary ??
                            `Artifact ${idx + 1}`,
                    ),
                    kind: String(entry.kind ?? item.kind ?? item.type ?? 'file'),
                    path: pathVal,
                    mimeType:
                        typeof item.mimeType === 'string'
                            ? item.mimeType
                            : typeof item.mime_type === 'string'
                              ? item.mime_type
                              : typeof entry.mimeType === 'string'
                                ? entry.mimeType
                                : typeof entry.mime_type === 'string'
                                  ? entry.mime_type
                                  : undefined,
                    size:
                        typeof item.size === 'number'
                            ? item.size
                            : typeof item.byte_count === 'number'
                              ? item.byte_count
                              : typeof item.size_bytes === 'number'
                                ? item.size_bytes
                                : typeof entry.size === 'number'
                                  ? entry.size
                                  : undefined,
                    content:
                        typeof item.content === 'string'
                            ? item.content
                            : typeof item.text === 'string'
                              ? item.text
                              : typeof entry.content === 'string'
                                ? entry.content
                                : undefined,
                    preview:
                        typeof item.preview === 'string'
                            ? item.preview
                            : typeof item.summary === 'string'
                              ? item.summary
                              : typeof entry.preview === 'string'
                                ? entry.preview
                                : typeof entry.summary === 'string'
                                  ? entry.summary
                                  : undefined,
                    createdAt:
                        typeof item.createdAt === 'string'
                            ? item.createdAt
                            : typeof item.created_at === 'string'
                              ? item.created_at
                              : typeof entry.createdAt === 'string'
                                ? entry.createdAt
                                : typeof entry.created_at === 'string'
                                  ? entry.created_at
                                  : createdMs !== undefined
                                    ? new Date(createdMs).toLocaleString()
                                    : undefined,
                    messageId:
                        typeof item.messageId === 'string'
                            ? item.messageId
                            : typeof item.message_id === 'string'
                              ? item.message_id
                              : typeof entry.messageId === 'string'
                                ? entry.messageId
                                : typeof entry.message_id === 'string'
                                  ? entry.message_id
                                  : undefined,
                };
            });
    } catch (cause) {
        if (cause instanceof ApiError && cause.status === 404) return [];
        throw cause;
    }
}

export async function searchV2ConversationRecall(
    conversationId: string,
    query: string,
    signal?: AbortSignal,
): Promise<V2RecallHit[]> {
    const trimmed = query.trim();
    if (!trimmed) return [];
    const res = await api<Record<string, unknown>>(
        `${conversationUrl(conversationId)}/recall`,
        {
            method: 'POST',
            body: JSON.stringify({ query: trimmed }),
            signal,
        },
    );
    const raw = Array.isArray(res.hits)
        ? res.hits
        : Array.isArray(res.matches)
          ? res.matches
          : Array.isArray(res.events)
            ? res.events
            : Array.isArray(res.results)
              ? res.results
              : [];
    return raw
        .filter((item): item is Record<string, unknown> => !!item && typeof item === 'object')
        .map((item, idx) => {
            const payload = (item.payload && typeof item.payload === 'object' ? item.payload : item) as Record<string, unknown>;
            const seq = typeof item.seq === 'number' ? item.seq : undefined;
            const createdMs =
                typeof item.createdAtMs === 'number'
                    ? item.createdAtMs
                    : typeof item.created_at_ms === 'number'
                      ? item.created_at_ms
                      : undefined;
            return {
                messageId: String(
                    item.messageId ??
                        item.message_id ??
                        item.id ??
                        (seq != null ? `msg-${seq}` : `hit-${idx}`),
                ),
                seq,
                ordinal: typeof item.message_ordinal === 'number' ? item.message_ordinal : seq !== undefined ? seq : idx + 1,
                role: String(item.role ?? payload.role ?? item.kind ?? 'message'),
                snippet: String(item.snippet ?? item.text ?? payload.text ?? payload.content ?? payload.summary ?? ''),
                timestamp: typeof item.createdAt === 'string'
                    ? item.createdAt
                    : typeof item.created_at === 'string'
                      ? item.created_at
                      : createdMs !== undefined
                        ? new Date(createdMs).toLocaleString()
                        : undefined,
            };
        })
        .filter(h => h.snippet);
}

export async function searchV2Documents(query: string, signal?: AbortSignal): Promise<V2DocumentSearchHit[]> {
    const trimmed = query.trim();
    if (!trimmed) return [];
    const res = await api<Record<string, unknown>>('/api/documents/search', {
        method: 'POST',
        body: JSON.stringify({ query: trimmed }),
        signal,
    });
    const hits: V2DocumentSearchHit[] = [];
    const seen = new Set<string>();
    const pushDocs = (arr: unknown, defaultKind: V2DocumentSearchHit['kind']) => {
        if (!Array.isArray(arr)) return;
        for (let i = 0; i < arr.length; i++) {
            const item = arr[i];
            if (!item || typeof item !== 'object') continue;
            const o = item as Record<string, unknown>;
            const kind: V2DocumentSearchHit['kind'] =
                o.kind === 'memory' || o.kind === 'file' || o.kind === 'document' ? o.kind : defaultKind;
            const id = String(o.id ?? o.chunk_id ?? o.path ?? `${kind}-${i}`);
            const dedupeKey = `${kind}:${id}`;
            if (seen.has(dedupeKey)) continue;
            seen.add(dedupeKey);
            hits.push({
                id,
                kind,
                title: String(o.title ?? o.source_name ?? o.name ?? o.path ?? `Match ${i + 1}`),
                snippet: typeof o.snippet === 'string'
                    ? o.snippet
                    : typeof o.content === 'string'
                      ? o.content
                      : typeof o.text === 'string'
                        ? o.text
                        : undefined,
                path: typeof o.path === 'string' ? o.path : typeof o.location === 'string' ? o.location : undefined,
                sourceId: typeof o.source_id === 'string' ? o.source_id : typeof o.sourceId === 'string' ? o.sourceId : undefined,
                score: typeof o.score === 'number' ? o.score : undefined,
            });
        }
    };
    pushDocs(res.documents, 'document');
    pushDocs(res.chunks, 'document');
    pushDocs(res.results, 'document');
    pushDocs(res.memory, 'memory');
    pushDocs(res.files, 'file');
    return hits;
}

export async function indexV2DataSource(
    sourceId: string,
    signal?: AbortSignal,
): Promise<{ ok: boolean; indexedChunks?: number; message?: string }> {
    const res = await api<Record<string, unknown>>(`/api/data-sources/${encodeURIComponent(sourceId)}/index`, {
        method: 'POST',
        body: '{}',
        signal,
    });
    return {
        ok: res.ok !== false,
        indexedChunks:
            typeof res.indexed_chunks === 'number'
                ? res.indexed_chunks
                : typeof res.indexedChunks === 'number'
                  ? res.indexedChunks
                  : typeof res.chunks === 'number'
                    ? res.chunks
                    : undefined,
        message: typeof res.message === 'string' ? res.message : undefined,
    };
}

function parsePlanResponse(raw: unknown): V2ConversationPlan {
    const o = (raw && typeof raw === 'object' ? raw : {}) as Record<string, unknown>;
    const inner = (o.plan && typeof o.plan === 'object' ? o.plan : o) as Record<string, unknown>;
    const enabled = Boolean(o.enabled ?? o.planMode ?? o.plan_mode ?? inner.enabled ?? false);
    const statusRaw = String(inner.status ?? o.status ?? (enabled ? 'draft' : 'idle')).toLowerCase();
    const status: V2ConversationPlan['status'] =
        statusRaw === 'draft' ||
        statusRaw === 'proposed' ||
        statusRaw === 'pending' ||
        statusRaw === 'awaiting_approval' ||
        statusRaw === 'pending_approval' ||
        statusRaw === 'approved' ||
        statusRaw === 'rejected' ||
        statusRaw === 'executing' ||
        statusRaw === 'completed'
            ? (statusRaw === 'pending_approval' ? 'awaiting_approval' : statusRaw)
            : 'idle';
    const rawSteps = Array.isArray(inner.steps) ? inner.steps : Array.isArray(o.steps) ? o.steps : [];
    const steps: V2ConversationPlanStep[] = rawSteps
        .map((s, idx): V2ConversationPlanStep | null => {
            if (typeof s === 'string') return { id: `step-${idx}`, title: s };
            if (!s || typeof s !== 'object') return null;
            const so = s as Record<string, unknown>;
            const title = String(so.title ?? so.description ?? so.step ?? so.content ?? '').trim();
            if (!title) return null;
            return {
                id: typeof so.id === 'string' ? so.id : `step-${idx}`,
                title,
                status: typeof so.status === 'string' ? so.status : undefined,
                detail: typeof so.detail === 'string' ? so.detail : undefined,
            };
        })
        .filter((s): s is V2ConversationPlanStep => s !== null);
    return {
        enabled,
        status,
        summary:
            typeof inner.summary === 'string'
                ? inner.summary
                : typeof inner.title === 'string'
                  ? inner.title
                  : typeof o.summary === 'string'
                    ? o.summary
                    : typeof o.title === 'string'
                      ? o.title
                      : undefined,
        markdown: typeof inner.markdown === 'string'
            ? inner.markdown
            : typeof inner.content === 'string'
              ? inner.content
              : typeof o.markdown === 'string'
                ? o.markdown
                : undefined,
        steps,
        updatedAt: typeof inner.updatedAt === 'string'
            ? inner.updatedAt
            : typeof inner.updated_at === 'string'
              ? inner.updated_at
              : undefined,
    };
}

export async function getV2ConversationPlan(
    conversationId: string,
    signal?: AbortSignal,
): Promise<V2ConversationPlan> {
    try {
        const res = await api<Record<string, unknown>>(`${conversationUrl(conversationId)}/plan`, { signal });
        return parsePlanResponse(res);
    } catch (cause) {
        if (cause instanceof ApiError && cause.status === 404) {
            return { enabled: false, status: 'idle', steps: [] };
        }
        throw cause;
    }
}

export async function updateV2ConversationPlan(
    conversationId: string,
    payload: {
        enabled?: boolean;
        action?: 'enable' | 'disable' | 'approve' | 'reject' | 'clear';
        feedback?: string;
    },
    signal?: AbortSignal,
): Promise<V2ConversationPlan> {
    const res = await api<Record<string, unknown>>(`${conversationUrl(conversationId)}/plan`, {
        method: 'POST',
        body: JSON.stringify({
            ...(payload.enabled !== undefined ? { enabled: payload.enabled, plan_mode: payload.enabled } : {}),
            ...(payload.action ? { action: payload.action } : {}),
            ...(payload.feedback ? { feedback: payload.feedback } : {}),
        }),
        signal,
    });
    return parsePlanResponse(res);
}

function normalizeSubagentTaskStatus(rawStatus: unknown, fallback: V2SubagentTask['status'] = 'queued'): V2SubagentTask['status'] {
    const st = String(rawStatus ?? '').trim().toLowerCase();
    if (!st) return fallback;
    if (st === 'completed' || st === 'done' || st === 'succeeded' || st === 'finished') return 'completed';
    if (st === 'failed' || st === 'error') return 'failed';
    if (st === 'cancelled' || st === 'canceled' || st === 'stopped') return 'cancelled';
    if (st === 'queued' || st === 'pending' || st === 'todo' || st === 'open') return 'queued';
    if (st === 'waiting' || st === 'waiting_input' || st === 'waiting_for_input') return 'waiting';
    if (st === 'running' || st === 'in_progress' || st === 'active' || st === 'working' || st === 'started') return 'running';
    return fallback;
}

function parseSubagentTask(raw: unknown, idx: number): V2SubagentTask | null {
    if (!raw || typeof raw !== 'object') return null;
    const o = raw as Record<string, unknown>;
    const rawId = o.id ?? o.task_id ?? o.taskId ?? o.subagent_id ?? o.call_id ?? o.callId;
    const id = String(rawId ?? `task-${idx}`).trim();
    const rawTitle = o.title ?? o.label ?? o.name ?? o.content ?? o.subject ?? o.task ?? o.goal ?? o.prompt;
    const title = String(rawTitle ?? '').trim();
    if (!id || !title) return null;
    const status = normalizeSubagentTaskStatus(o.status ?? o.state, 'queued');
    return {
        id,
        parentId: typeof o.parent_id === 'string' ? o.parent_id : typeof o.parentId === 'string' ? o.parentId : undefined,
        title,
        status,
        kind: typeof o.kind === 'string' ? o.kind : typeof o.type === 'string' ? o.type : undefined,
        agent: typeof o.agent === 'string' ? o.agent : typeof o.expert === 'string' ? o.expert : undefined,
        detail: typeof o.detail === 'string' ? o.detail : typeof o.summary === 'string' ? o.summary : typeof o.message === 'string' ? o.message : undefined,
        progress: typeof o.progress === 'number' ? o.progress : undefined,
        updatedAt: typeof o.updated_at === 'string'
            ? o.updated_at
            : typeof o.updatedAt === 'string'
              ? o.updatedAt
              : typeof o.updated_at_ms === 'number'
                ? new Date(o.updated_at_ms).toLocaleTimeString()
                : typeof o.updatedAtMs === 'number'
                  ? new Date(o.updatedAtMs).toLocaleTimeString()
                  : undefined,
    };
}

function parseTaskEventFrame(ev: Record<string, unknown>): { tasks?: V2SubagentTask[]; update?: V2SubagentTask } | null {
    const kind = String(ev.kind ?? '').trim().toLowerCase();
    const payload = (ev.payload && typeof ev.payload === 'object' && !Array.isArray(ev.payload)
        ? ev.payload
        : {}) as Record<string, unknown>;
    const nestedList = Array.isArray(payload.tasks)
        ? payload.tasks
        : Array.isArray(payload.todos)
          ? payload.todos
          : null;
    if (nestedList) {
        return { tasks: nestedList.map(parseSubagentTask).filter((t): t is V2SubagentTask => !!t) };
    }
    const isTrackedKind =
        kind.startsWith('tool_') ||
        kind.startsWith('subagent_') ||
        kind.startsWith('task_');
    if (!isTrackedKind) return null;

    const rawId =
        payload.task_id ??
        payload.taskId ??
        payload.subagent_id ??
        payload.call_id ??
        payload.callId ??
        payload.id;
    const id = rawId != null ? String(rawId).trim() : '';
    const title = String(
        payload.title ??
            payload.label ??
            payload.name ??
            payload.tool ??
            payload.tool_name ??
            payload.content ??
            payload.goal ??
            '',
    ).trim();
    if (!id || !title) return null;

    let inferredStatus: V2SubagentTask['status'] | null = null;
    if (payload.status !== undefined || payload.state !== undefined) {
        inferredStatus = normalizeSubagentTaskStatus(payload.status ?? payload.state, 'queued');
    } else if (kind.endsWith('_finished') || kind.endsWith('_completed') || kind.endsWith('_succeeded')) {
        inferredStatus = payload.is_error === true || payload.ok === false ? 'failed' : 'completed';
    } else if (kind.endsWith('_failed') || kind.endsWith('_error')) {
        inferredStatus = 'failed';
    } else if (kind.endsWith('_cancelled') || kind.endsWith('_canceled') || kind.endsWith('_stopped')) {
        inferredStatus = 'cancelled';
    } else if (kind.endsWith('_started') || kind.endsWith('_running') || kind.endsWith('_progress')) {
        inferredStatus = 'running';
    } else if (kind.endsWith('_queued') || kind.endsWith('_created')) {
        inferredStatus = 'queued';
    }
    if (!inferredStatus) return null;

    const createdMs =
        typeof ev.created_at_ms === 'number'
            ? ev.created_at_ms
            : typeof ev.createdAtMs === 'number'
              ? ev.createdAtMs
              : undefined;
    return {
        update: {
            id,
            parentId:
                typeof payload.parent_id === 'string'
                    ? payload.parent_id
                    : typeof payload.parentId === 'string'
                      ? payload.parentId
                      : undefined,
            title,
            status: inferredStatus,
            kind: kind.startsWith('tool_') ? 'tool' : kind.startsWith('subagent_') ? 'subagent' : 'task',
            agent:
                typeof payload.agent === 'string'
                    ? payload.agent
                    : typeof payload.expert === 'string'
                      ? payload.expert
                      : undefined,
            detail:
                typeof payload.summary === 'string'
                    ? payload.summary
                    : typeof payload.detail === 'string'
                      ? payload.detail
                      : typeof payload.message === 'string'
                        ? payload.message
                        : undefined,
            progress: typeof payload.progress === 'number' ? payload.progress : undefined,
            updatedAt: createdMs !== undefined ? new Date(createdMs).toLocaleTimeString() : undefined,
        },
    };
}

export async function subscribeV2ConversationTasks(
    conversationId: string,
    callbacks: {
        onTasks: (tasks: V2SubagentTask[]) => void;
        onTaskUpdate?: (task: V2SubagentTask) => void;
        onError?: (message: string) => void;
    },
    signal?: AbortSignal,
): Promise<void> {
    const response = await apiResponse(`${conversationUrl(conversationId)}/tasks/events`, {
        method: 'GET',
        signal,
        headers: { Accept: 'text/event-stream, application/json' },
    });
    if (!response.ok) {
        if (response.status === 404) return;
        throw new ApiError(response.status, `HTTP ${response.status}`);
    }
    const contentType = response.headers.get('content-type') ?? '';
    if (contentType.includes('application/json')) {
        const data = (await response.json().catch(() => ({}))) as Record<string, unknown>;
        const arr = Array.isArray(data.tasks)
            ? data.tasks
            : Array.isArray(data.todos)
              ? data.todos
              : Array.isArray(data.items)
                ? data.items
                : [];
        callbacks.onTasks(arr.map(parseSubagentTask).filter((t): t is V2SubagentTask => !!t));
        return;
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
        const evType = typeof ev.type === 'string' ? ev.type : '';
        const list = Array.isArray(ev.tasks)
            ? ev.tasks
            : Array.isArray(ev.todos)
              ? ev.todos
              : null;
        if (list) {
            callbacks.onTasks(list.map(parseSubagentTask).filter((t): t is V2SubagentTask => !!t));
            return;
        }
        if (evType === 'task_snapshot') {
            return;
        }
        if (evType === 'task_projection') {
            const proj = (ev.taskProjection ?? ev.task_projection ?? ev.projection) as Record<string, unknown> | null | undefined;
            const projList = proj && typeof proj === 'object'
                ? (Array.isArray(proj.tasks)
                    ? proj.tasks
                    : Array.isArray(proj.todos)
                      ? proj.todos
                      : Array.isArray(proj.items)
                        ? proj.items
                        : null)
                : null;
            if (projList) {
                callbacks.onTasks(projList.map(parseSubagentTask).filter((t): t is V2SubagentTask => !!t));
            }
            return;
        }
        if (evType === 'task_event') {
            const normalized = parseTaskEventFrame(ev);
            if (normalized?.tasks) {
                callbacks.onTasks(normalized.tasks);
            } else if (normalized?.update) {
                callbacks.onTaskUpdate?.(normalized.update);
            }
            return;
        }
        if (ev.task && typeof ev.task === 'object') {
            const parsed = parseSubagentTask(ev.task, 0);
            if (parsed) callbacks.onTaskUpdate?.(parsed);
        } else if (!evType && (typeof ev.id === 'string' || typeof ev.task_id === 'string') && (ev.status !== undefined || ev.state !== undefined)) {
            const parsed = parseSubagentTask(ev, 0);
            if (parsed) callbacks.onTaskUpdate?.(parsed);
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

export async function stopV2ConversationTask(
    conversationId: string,
    taskId: string,
    signal?: AbortSignal,
): Promise<void> {
    try {
        await api(`${conversationUrl(conversationId)}/tasks/${encodeURIComponent(taskId)}/stop`, {
            method: 'POST',
            body: JSON.stringify({ task_id: taskId }),
            signal,
        });
    } catch (cause) {
        if (cause instanceof ApiError && (cause.status === 404 || cause.status === 405)) {
            await cancelV2Turn(conversationId);
        } else {
            throw cause;
        }
    }
}

function parseWorktreeItem(raw: unknown, idx: number): V2GitWorktree | null {
    if (!raw || typeof raw !== 'object') return null;
    const o = raw as Record<string, unknown>;
    const path = String(o.path ?? o.worktree ?? '').trim();
    const rawBranch =
        typeof o.branch === 'string'
            ? o.branch.trim()
            : typeof o.ref === 'string'
              ? o.ref.trim()
              : typeof o.name === 'string'
                ? o.name.trim()
                : '';
    const detached =
        o.detached === true ||
        !rawBranch ||
        rawBranch === '(detached)' ||
        rawBranch === 'detached';
    const branch = rawBranch || '(detached)';
    if (!path && !rawBranch) return null;
    return {
        id: String(o.id ?? path ?? `${branch}-${idx}`),
        path: path || branch,
        branch,
        head: typeof o.head === 'string' ? o.head : typeof o.commit === 'string' ? o.commit : undefined,
        isMain: o.is_main === true || o.isMain === true || o.main === true,
        detached,
        locked: o.locked === true,
        prunable: o.prunable === true,
        dirty: o.dirty === true,
        status: typeof o.status === 'string' ? o.status : undefined,
    };
}

export async function listV2WorkspaceWorktrees(signal?: AbortSignal): Promise<V2GitWorktree[]> {
    try {
        const res = await api<Record<string, unknown>>('/api/workspace/worktrees', { signal });
        const raw = Array.isArray(res.worktrees)
            ? res.worktrees
            : Array.isArray(res.items)
              ? res.items
              : Array.isArray(res)
                ? (res as unknown[])
                : [];
        return raw.map(parseWorktreeItem).filter((w): w is V2GitWorktree => !!w);
    } catch (cause) {
        if (cause instanceof ApiError && cause.status === 404) return [];
        throw cause;
    }
}

export async function createV2WorkspaceWorktree(
    payload: { branch: string; path?: string; baseRef?: string },
    signal?: AbortSignal,
): Promise<{ ok: boolean; message?: string; worktree?: V2GitWorktree | null }> {
    const res = await api<Record<string, unknown>>('/api/workspace/worktrees', {
        method: 'POST',
        body: JSON.stringify({
            action: 'create',
            branch: payload.branch.trim(),
            ...(payload.path?.trim() ? { path: payload.path.trim() } : {}),
            ...(payload.baseRef?.trim() ? { base_ref: payload.baseRef.trim(), baseRef: payload.baseRef.trim() } : {}),
        }),
        signal,
    });
    return {
        ok: res.ok !== false,
        message: typeof res.message === 'string' ? res.message : undefined,
        worktree: parseWorktreeItem(res.worktree, 0),
    };
}

export async function integrateV2WorkspaceWorktree(
    worktree: Pick<V2GitWorktree, 'id' | 'path' | 'branch'>,
    signal?: AbortSignal,
): Promise<{ ok: boolean; message?: string }> {
    const res = await api<Record<string, unknown>>('/api/workspace/worktrees', {
        method: 'POST',
        body: JSON.stringify({
            action: 'integrate',
            id: worktree.id,
            path: worktree.path,
            branch: worktree.branch,
        }),
        signal,
    });
    return {
        ok: res.ok !== false,
        message: typeof res.message === 'string' ? res.message : typeof res.stdout === 'string' ? res.stdout : undefined,
    };
}

export async function removeV2WorkspaceWorktree(
    worktree: Pick<V2GitWorktree, 'id' | 'path' | 'branch'>,
    force = false,
    signal?: AbortSignal,
): Promise<{ ok: boolean; message?: string }> {
    const res = await api<Record<string, unknown>>('/api/workspace/worktrees', {
        method: 'POST',
        body: JSON.stringify({
            action: 'remove',
            id: worktree.id,
            path: worktree.path,
            branch: worktree.branch,
            force,
        }),
        signal,
    });
    return {
        ok: res.ok !== false,
        message: typeof res.message === 'string' ? res.message : typeof res.stdout === 'string' ? res.stdout : undefined,
    };
}

function firstRecord(value: unknown): Record<string, unknown> | null {
    if (Array.isArray(value)) {
        const first = value.find(item => !!item && typeof item === 'object' && !Array.isArray(item));
        return first ? (first as Record<string, unknown>) : null;
    }
    if (value && typeof value === 'object' && !Array.isArray(value)) {
        return value as Record<string, unknown>;
    }
    return null;
}

function parseTurnObservation(raw: unknown): V2TurnObservation {
    const outer = (raw && typeof raw === 'object' ? raw : {}) as Record<string, unknown>;
    const o = (outer.result && typeof outer.result === 'object' && !Array.isArray(outer.result)
        ? outer.result
        : outer) as Record<string, unknown>;
    const status = String(o.status ?? o.state ?? o.phase ?? '').toLowerCase();
    const rawTools = Array.isArray(o.toolCalls) ? o.toolCalls : Array.isArray(o.tool_calls) ? o.tool_calls : [];
    const toolCalls: V2ToolCall[] = rawTools
        .filter((t): t is Record<string, unknown> => !!t && typeof t === 'object')
        .map((t, i) => ({
            callId: String(t.callId ?? t.call_id ?? `obs-tool-${i}`),
            name: String(t.name ?? 'tool'),
            arguments: t.arguments,
            status: t.status === 'running' ? 'running' : t.ok === false || t.status === 'failed' ? 'failed' : 'completed',
            summary: typeof t.summary === 'string' ? t.summary : undefined,
        }));
    const rawPerm =
        firstRecord(o.pendingPermission) ??
        firstRecord(o.pending_permission) ??
        firstRecord(o.pendingPermissions) ??
        firstRecord(o.pending_permissions) ??
        firstRecord(o.permissions);
    const permId = rawPerm ? (rawPerm.requestId ?? rawPerm.request_id ?? rawPerm.id) : undefined;
    const pendingPermission: V2PermissionPrompt | null =
        rawPerm && typeof permId === 'string' && permId.trim()
            ? {
                  requestId: permId,
                  tool: String(rawPerm.tool ?? rawPerm.tool_name ?? rawPerm.name ?? 'tool'),
                  reason:
                      typeof rawPerm.reason === 'string'
                          ? rawPerm.reason
                          : typeof rawPerm.message === 'string'
                            ? rawPerm.message
                            : '',
                  arguments: rawPerm.arguments ?? rawPerm.args,
              }
            : null;
    const rawQ =
        firstRecord(o.pendingQuestion) ??
        firstRecord(o.pending_question) ??
        firstRecord(o.pendingQuestions) ??
        firstRecord(o.pending_questions) ??
        firstRecord(o.questions);
    const qId = rawQ ? (rawQ.requestId ?? rawQ.request_id ?? rawQ.id) : undefined;
    const pendingQuestion: V2UserQuestion | null =
        rawQ && typeof qId === 'string' && qId.trim()
            ? {
                  requestId: qId,
                  question: String(rawQ.question ?? rawQ.prompt ?? rawQ.text ?? ''),
                  options: Array.isArray(rawQ.options) ? rawQ.options.map(String) : [],
              }
            : null;
    const needsResponse =
        o.needsResponse === true ||
        o.needs_response === true ||
        Boolean(pendingPermission) ||
        Boolean(pendingQuestion);
    const active =
        o.active === true ||
        o.running === true ||
        o.in_progress === true ||
        o.inProgress === true ||
        needsResponse ||
        status === 'running' ||
        status === 'streaming' ||
        status === 'working' ||
        status === 'waiting_permission' ||
        status === 'waiting_input' ||
        status === 'waiting_for_input';
    return {
        active,
        needsResponse,
        status: status || undefined,
        runId: typeof o.run_id === 'string' ? o.run_id : typeof o.runId === 'string' ? o.runId : undefined,
        text: typeof o.text === 'string' ? o.text : typeof o.partial_text === 'string' ? o.partial_text : typeof o.content === 'string' ? o.content : undefined,
        reasoning: typeof o.reasoning === 'string' ? o.reasoning : typeof o.partial_reasoning === 'string' ? o.partial_reasoning : undefined,
        toolCalls: toolCalls.length ? toolCalls : undefined,
        pendingPermission,
        pendingQuestion,
        usage: parseV2TokenUsage(o.usage),
        step: typeof o.step === 'string' ? o.step : typeof o.current_step === 'string' ? o.current_step : undefined,
    };
}

export async function observeV2ConversationProgress(
    conversationId: string,
    signal?: AbortSignal,
): Promise<V2TurnObservation> {
    let progressObs: V2TurnObservation | null = null;
    try {
        const res = await api<Record<string, unknown>>(`${conversationUrl(conversationId)}/progress`, { signal });
        progressObs = parseTurnObservation(res);
        if (!progressObs.needsResponse || progressObs.pendingPermission || progressObs.pendingQuestion) {
            return progressObs;
        }
    } catch (cause) {
        if (signal?.aborted) throw cause;
        if (!(cause instanceof ApiError && cause.status === 404)) {
            progressObs = null;
        }
    }
    try {
        const res = await api<Record<string, unknown>>(`${conversationUrl(conversationId)}/observe`, { signal });
        const observeObs = parseTurnObservation(res);
        if (!progressObs) return observeObs;
        return {
            ...progressObs,
            ...observeObs,
            active: progressObs.active || observeObs.active,
            needsResponse: progressObs.needsResponse || observeObs.needsResponse,
            pendingPermission: observeObs.pendingPermission ?? progressObs.pendingPermission ?? null,
            pendingQuestion: observeObs.pendingQuestion ?? progressObs.pendingQuestion ?? null,
            toolCalls: observeObs.toolCalls?.length ? observeObs.toolCalls : progressObs.toolCalls,
            text: observeObs.text ?? progressObs.text,
            reasoning: observeObs.reasoning ?? progressObs.reasoning,
            usage: observeObs.usage ?? progressObs.usage,
        };
    } catch (cause) {
        if (signal?.aborted) throw cause;
    }
    return progressObs ?? { active: false };
}

export async function streamV2ConversationObserve(
    conversationId: string,
    callbacks: V2StreamCallbacks & { onSnapshot?: (obs: V2TurnObservation) => void },
    signal?: AbortSignal,
): Promise<void> {
    const response = await apiResponse(`${conversationUrl(conversationId)}/observe`, {
        method: 'GET',
        signal,
        headers: { Accept: 'text/event-stream, application/json' },
    });
    if (!response.ok) {
        if (response.status === 404) return;
        throw new ApiError(response.status, `HTTP ${response.status}`);
    }
    const contentType = response.headers.get('content-type') ?? '';
    if (contentType.includes('application/json')) {
        // Polling fallback when /observe returns JSON snapshots instead of SSE
        let obs = parseTurnObservation(await response.json().catch(() => ({})));
        callbacks.onSnapshot?.(obs);
        while (obs.active && !signal?.aborted) {
            await new Promise<void>((resolve, reject) => {
                const timer = setTimeout(resolve, 900);
                signal?.addEventListener('abort', () => { clearTimeout(timer); reject(new DOMException('Aborted', 'AbortError')); }, { once: true });
            });
            if (signal?.aborted) break;
            obs = await observeV2ConversationProgress(conversationId, signal);
            callbacks.onSnapshot?.(obs);
        }
        if (!obs.active) {
            callbacks.onTurnCompleted?.(obs.text, obs.usage ?? undefined);
        }
        return;
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
        if (type === 'snapshot' || type === 'progress' || (!type && ('active' in ev || 'status' in ev))) {
            callbacks.onSnapshot?.(parseTurnObservation(ev));
            return;
        }
        if (type === 'text_delta' && typeof ev.delta === 'string') callbacks.onTextDelta?.(ev.delta);
        else if (type === 'reasoning_delta' && typeof ev.delta === 'string') callbacks.onReasoningDelta?.(ev.delta);
        else if (type === 'tool_started') callbacks.onToolStarted?.(String(ev.call_id ?? ''), String(ev.name ?? 'tool'), ev.arguments);
        else if (type === 'tool_finished') callbacks.onToolFinished?.(String(ev.call_id ?? ''), String(ev.name ?? 'tool'), ev.is_error !== true && ev.ok !== false, String(ev.summary ?? ''));
        else if (type === 'permission_requested' && typeof ev.request_id === 'string') {
            callbacks.onPermissionRequested?.({ requestId: ev.request_id, tool: String(ev.tool ?? 'tool'), reason: typeof ev.reason === 'string' ? ev.reason : '', arguments: ev.arguments });
        } else if (type === 'user_question' && typeof ev.request_id === 'string') {
            callbacks.onUserQuestion?.({ requestId: ev.request_id, question: String(ev.question ?? ''), options: Array.isArray(ev.options) ? ev.options.map(String) : [] });
        } else if (type === 'turn_completed') {
            callbacks.onTurnCompleted?.(typeof ev.final_text === 'string' ? ev.final_text : undefined, parseV2TokenUsage(ev.usage) ?? undefined);
        } else if (type === 'error') {
            callbacks.onError?.(String(ev.message ?? ev.error ?? 'Unknown error'));
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

export async function getV2BrowserState(signal?: AbortSignal): Promise<V2BrowserState> {
    const fallback: V2BrowserState = { managedAvailable: false, managedConnected: false, nativeConnected: false, pages: [] };
    try {
        const res = await api<Record<string, unknown>>('/api/native-browser/status', { signal });
        const rawPages = Array.isArray(res.pages) ? res.pages : Array.isArray(res.tabs) ? res.tabs : [];
        return {
            managedAvailable: Boolean(res.managed_available ?? res.managedAvailable ?? res.available ?? false),
            managedConnected: Boolean(res.managed_connected ?? res.managedConnected ?? res.running ?? false),
            nativeConnected: Boolean(res.native_connected ?? res.nativeConnected ?? res.connected ?? false),
            previewUrl: typeof res.preview_url === 'string' ? res.preview_url : typeof res.previewUrl === 'string' ? res.previewUrl : undefined,
            pages: rawPages
                .filter((p): p is Record<string, unknown> => !!p && typeof p === 'object')
                .map((p, idx) => ({
                    id: String(p.id ?? p.page_id ?? `page-${idx}`),
                    title: String(p.title ?? p.url ?? 'Page'),
                    url: String(p.url ?? ''),
                    active: p.active === true,
                })),
        };
    } catch {
        return fallback;
    }
}
