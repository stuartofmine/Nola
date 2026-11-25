import { api, apiResponse, ApiError } from './api';
import {
    parseChatConversation,
    parseChatConversations,
    conversationUrl,
    type ChatConversation,
} from './chatConversations';

export interface V2ToolCall {
    callId: string;
    name: string;
    arguments?: unknown;
    status: 'running' | 'completed' | 'failed';
    summary?: string;
}

export interface V2Message {
    id: string;
    role: 'user' | 'assistant' | 'system';
    content: string;
    reasoning?: string;
    toolCalls?: V2ToolCall[];
    createdAtMs?: number;
}

export interface V2StreamCallbacks {
    onSessionStarted?: (sessionId: string) => void;
    onTextDelta?: (delta: string) => void;
    onReasoningDelta?: (delta: string) => void;
    onToolStarted?: (callId: string, name: string, argumentsJson?: unknown) => void;
    onToolFinished?: (callId: string, name: string, ok: boolean, summary: string) => void;
    onCompacted?: (summary: string) => void;
    onTurnCompleted?: (finalText?: string) => void;
    onError?: (message: string) => void;
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

export async function loadV2Messages(id: string, signal?: AbortSignal): Promise<V2Message[]> {
    const res = await api<{ messages?: unknown[] }>(`${conversationUrl(id)}/messages?limit=200`, { signal });
    if (!Array.isArray(res.messages)) return [];
    return res.messages.map((raw, idx) => {
        const m = (raw && typeof raw === 'object' ? raw : {}) as Record<string, unknown>;
        const role = m.role === 'user' || m.role === 'assistant' || m.role === 'system' ? m.role : 'assistant';
        const rawTools = Array.isArray(m.toolCalls) ? m.toolCalls : [];
        const toolCalls: V2ToolCall[] = rawTools.map((t, ti) => {
            const o = (t && typeof t === 'object' ? t : {}) as Record<string, unknown>;
            return {
                callId: typeof o.callId === 'string' ? o.callId : `tool-${idx}-${ti}`,
                name: typeof o.name === 'string' ? o.name : 'tool',
                status: o.ok === false ? 'failed' : 'completed',
                summary: typeof o.summary === 'string' ? o.summary : '',
            };
        });
        return {
            id: typeof m.id === 'string' ? m.id : `msg-${idx}`,
            role,
            content: typeof m.content === 'string' ? m.content : '',
            reasoning: typeof m.reasoning === 'string' && m.reasoning ? m.reasoning : undefined,
            toolCalls: toolCalls.length ? toolCalls : undefined,
            createdAtMs: typeof m.createdAtMs === 'number' ? m.createdAtMs : undefined,
        };
    });
}

export async function cancelV2Turn(conversationId: string): Promise<void> {
    await api(`${conversationUrl(conversationId)}/cancel`, { method: 'POST', body: '{}' });
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
): Promise<void> {
    const response = await apiResponse(`${conversationUrl(conversationId)}/messages`, {
        method: 'POST',
        body: JSON.stringify({ content }),
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
            case 'compacted':
                callbacks.onCompacted?.(String(ev.summary ?? ''));
                break;
            case 'turn_completed':
                callbacks.onTurnCompleted?.(typeof ev.final_text === 'string' ? ev.final_text : undefined);
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
