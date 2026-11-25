export const expertBubbleColors = ['red', 'orange', 'yellow', 'green', 'teal', 'blue', 'purple', 'pink'] as const;
export type ExpertBubbleColor = typeof expertBubbleColors[number];

export const expertBubblePalette: Record<ExpertBubbleColor, readonly [string, string]> = {
    red: ['#F8F1F1', '#2B1919'],
    orange: ['#F8F4EF', '#2B2117'],
    yellow: ['#F7F6ED', '#272515'],
    green: ['#F1F6F2', '#17261C'],
    teal: ['#EFF6F6', '#152527'],
    blue: ['#F2F6F8', '#172128'],
    purple: ['#F6F3F7', '#231C2A'],
    pink: ['#F8F2F5', '#2A1A22'],
};

export const expertAccentPalette: Record<ExpertBubbleColor, string> = {
    red: '#D64545',
    orange: '#D97724',
    yellow: '#C29B1B',
    green: '#2E8B57',
    teal: '#1F8A8A',
    blue: '#2F6FD6',
    purple: '#7E57C2',
    pink: '#C94B88',
};

const storageKey = 'nola.experts.bubble-colors';
const saved = new Map<string, ExpertBubbleColor>();
const listeners = new Set<() => void>();
let loaded = false;

const key = (host: string, agent: string) => JSON.stringify([host || 'local', agent]);
const valid = (value: unknown): value is ExpertBubbleColor =>
    typeof value === 'string' && expertBubbleColors.includes(value as ExpertBubbleColor);

function load() {
    if (loaded || typeof localStorage === 'undefined') return;
    loaded = true;
    try {
        const value: unknown = JSON.parse(localStorage.getItem(storageKey) ?? '{}');
        if (!value || typeof value !== 'object' || Array.isArray(value)) return;
        for (const [id, color] of Object.entries(value)) {
            if (valid(color)) saved.set(id, color);
        }
    } catch {
        /* Local display preference only. */
    }
}

export function expertBubbleColor(host: string, agent: string, fallback?: string): ExpertBubbleColor {
    load();
    const stored = saved.get(key(host, agent));
    if (stored) return stored;
    if (valid(fallback)) return fallback;
    // Stable hash fallback so different experts automatically get distinct colors
    let hash = 0;
    for (let i = 0; i < agent.length; i++) {
        hash = ((hash << 5) - hash + agent.charCodeAt(i)) | 0;
    }
    return expertBubbleColors[Math.abs(hash) % expertBubbleColors.length];
}

export function saveExpertBubbleColor(host: string, agent: string, color: ExpertBubbleColor): boolean {
    if (!agent || !valid(color)) return false;
    load();
    saved.set(key(host, agent), color);
    let persisted = false;
    try {
        localStorage.setItem(storageKey, JSON.stringify(Object.fromEntries(saved)));
        persisted = true;
    } catch {
        /* Session-local preference still applies. */
    }
    for (const listener of listeners) listener();
    return persisted;
}

export function observeExpertBubbleColors(listener: () => void): () => void {
    listeners.add(listener);
    return () => listeners.delete(listener);
}
