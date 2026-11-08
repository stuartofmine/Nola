import type { TimelineMessage, TimelinePart } from './chatTimeline.ts';

export interface ChatBrowserPage { index: number; title: string; url: string; active: boolean }
export interface ChatBrowserSnapshot {
    target: 'managed' | 'attached';
    connected: boolean;
    pages: ChatBrowserPage[];
    currentUrl: string;
    previewUrl: string;
}

export function applyManagedBrowserPages(snapshot: ChatBrowserSnapshot | null, value: unknown): ChatBrowserSnapshot {
    if (!record(value) || !Array.isArray(value.pages) || typeof value.running !== 'boolean'
        || typeof value.previewUrl !== 'string' || typeof value.currentUrl !== 'string') throw new Error('Invalid browser page receipt');
    const nextPages = value.pages.map((item: unknown) => {
        if (!record(item) || !Number.isSafeInteger(item.index) || (item.index as number) < 0
            || typeof item.url !== 'string' || !item.url.trim() || typeof item.title !== 'string'
            || typeof item.active !== 'boolean') throw new Error('Invalid browser page');
        return { index: item.index as number, title: item.title, url: item.url, active: item.active };
    });
    if (nextPages.length > 20) throw new Error('Too many browser pages');
    return {
        target: 'managed', connected: value.running,
        pages: nextPages,
        currentUrl: value.currentUrl || nextPages.find(page => page.active)?.url || nextPages[0]?.url || '',
        previewUrl: value.previewUrl || snapshot?.previewUrl || '',
    };
}

/** A preview server bound to the host loopback cannot be reached from a LAN client. */
export function localBrowserPreviewURL(value: string, pageOrigin: string): string | null {
    try {
        const page = new URL(pageOrigin);
        const preview = new URL(value);
        const loopback = new Set(['localhost', '127.0.0.1', '[::1]']);
        if (!loopback.has(page.hostname) || !loopback.has(preview.hostname) || preview.protocol !== 'http:'
            || preview.username || preview.password || preview.hash) return null;
        return preview.href;
    } catch { return null; }
}

export function chatBrowserActivityKey(messages: readonly TimelineMessage[]): string {
    let latest = '';
    for (const message of messages) {
        if (message.role !== 'assistant') continue;
        for (const part of message.parts) {
            const value = part.value;
            if (value.kind === 'tool' && typeof value.capability_id === 'string'
                && (value.capability_id.startsWith('browser_') || value.capability_id.startsWith('chrome_native_'))
                && value.state === 'completed') latest = `${message.id}:${part.id}:${value.revision}`;
        }
    }
    return latest;
}

const record = (value: unknown): value is Record<string, unknown> => !!value && typeof value === 'object' && !Array.isArray(value);

function embedded(value: unknown): unknown {
    if (typeof value !== 'string' || value.length > 65536) return value;
    try { return JSON.parse(value); } catch { return value; }
}

function field(value: unknown, key: string, depth = 0): unknown {
    if (depth > 6) return undefined;
    const item = embedded(value);
    if (record(item)) {
        if (Object.hasOwn(item, key)) return item[key];
        for (const child of Object.values(item)) {
            const found = field(child, key, depth + 1);
            if (found !== undefined) return found;
        }
    }
    return undefined;
}

function pages(value: unknown): ChatBrowserPage[] | null {
    const items = field(value, 'pages');
    if (!Array.isArray(items)) return null;
    return items.slice(0, 20).flatMap((item, offset) => {
        if (!record(item) || typeof item.url !== 'string' || !item.url.trim()) return [];
        return [{ index: Number.isSafeInteger(item.index) && (item.index as number) >= 0 ? item.index as number : offset,
            title: typeof item.title === 'string' ? item.title : '', url: item.url, active: item.active === true }];
    });
}

/** Derive display state from owned, completed tool Parts, not tool-name guesses in user text. */
export function chatBrowserSnapshot(messages: readonly TimelineMessage[]): ChatBrowserSnapshot | null {
    let snapshot: ChatBrowserSnapshot | null = null;
    for (const message of messages) {
        if (message.role !== 'assistant') continue;
        for (const part of message.parts) {
            const value: TimelinePart['value'] = part.value;
            if (value.kind !== 'tool' || typeof value.capability_id !== 'string'
                || !(value.capability_id.startsWith('browser_') || value.capability_id.startsWith('chrome_native_'))
                || value.state !== 'completed') continue;
            const name = value.capability_id;
            if (name === 'browser_stop') { snapshot = null; continue; }
            const previous = snapshot as ChatBrowserSnapshot | null;
            const output = value.output;
            const input = value.input;
            const target = field(input, 'target') ?? field(output, 'target');
            const resolvedTarget: ChatBrowserSnapshot['target'] = target === 'attached' || target === 'managed' ? target
                : previous?.target ?? (name.startsWith('chrome_native_') ? 'attached' : 'managed');
            const nextPages = pages(output);
            const url = field(output, 'final_url') ?? field(output, 'url') ?? field(input, 'url');
            const currentUrl: string = typeof url === 'string' ? url : nextPages?.find(page => page.active)?.url ?? nextPages?.[0]?.url ?? (nextPages === null ? previous?.currentUrl : '') ?? '';
            const running = field(output, 'running');
            const preview = field(output, 'preview_url');
            snapshot = {
                target: resolvedTarget,
                connected: typeof running === 'boolean' ? running : true,
                pages: nextPages ?? previous?.pages ?? (currentUrl ? [{ index: 0, title: '', url: currentUrl, active: true }] : []),
                currentUrl,
                previewUrl: typeof preview === 'string' ? preview : previous?.previewUrl ?? '',
            };
        }
    }
    return snapshot;
}
