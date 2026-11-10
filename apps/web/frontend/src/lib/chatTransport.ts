import { ApiError, apiResponse } from './api.ts';
import { ChatTransportError, createSseJsonDecoder } from '../../../app/static/js/sse-json-decoder.mjs';
export { ChatTransportError, createSseJsonDecoder as createChatEventDecoder };

export type ChatEventEnvelope = Record<string, unknown>;

/** One request only. Retry/resume decisions belong to the turn state machine. */
export async function* chatEventStream(path: string, options: RequestInit): AsyncGenerator<ChatEventEnvelope> {
    const headers = new Headers(options.headers); headers.set('Accept', 'text/event-stream');
    const response = await apiResponse(path, { ...options, headers, redirect: 'error' });
    if (!response.ok) {
        await response.body?.cancel().catch(() => {});
        throw new ApiError(response.status, 'chat.transport.http');
    }
    if (response.headers.get('Content-Type')?.split(';')[0].trim().toLowerCase() !== 'text/event-stream') {
        await response.body?.cancel().catch(() => {});
        throw new ChatTransportError('invalid_content_type');
    }
    if (!response.body) throw new ChatTransportError('missing_body');
    const reader = response.body.getReader();
    const utf8 = new TextDecoder('utf-8', { fatal: true });
    const frames = createSseJsonDecoder();
    let ended = false;
    const decode = (value?: Uint8Array, stream = true) => {
        try { return utf8.decode(value, { stream }); } catch { throw new ChatTransportError('invalid_utf8'); }
    };
    try {
        while (true) {
            options.signal?.throwIfAborted();
            const { value, done } = await reader.read();
            options.signal?.throwIfAborted();
            if (done) { ended = true; break; }
            for (const event of frames.push(decode(value))) {
                options.signal?.throwIfAborted();
                yield event;
            }
        }
        for (const event of frames.push(decode(undefined, false))) yield event;
        frames.finish();
    } finally {
        if (!ended) await reader.cancel().catch(() => {});
        reader.releaseLock();
    }
}
