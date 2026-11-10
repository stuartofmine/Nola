export const expertBubbleColors = ['red', 'orange', 'yellow', 'green', 'teal', 'blue', 'purple', 'pink'] as const;
export type ExpertBubbleColor = typeof expertBubbleColors[number];
export const expertBubblePalette: Record<ExpertBubbleColor, readonly [string, string]> = {
    red: ['#F8F1F1', '#1C1313'], orange: ['#F8F4EF', '#1C1712'], yellow: ['#F7F6ED', '#1A190F'],
    green: ['#F1F6F2', '#111A14'], teal: ['#EFF6F6', '#101A1B'], blue: ['#F2F6F8', '#11171B'],
    purple: ['#F6F3F7', '#18141C'], pink: ['#F8F2F5', '#1C1318'],
};
const storageKey = 'nola.experts.bubble-colors';
const saved = new Map<string, ExpertBubbleColor>();
const listeners = new Set<() => void>();
let loaded = false;
const key = (host: string, agent: string) => JSON.stringify([host, agent]);
const valid = (value: unknown): value is ExpertBubbleColor => typeof value === 'string' && expertBubbleColors.includes(value as ExpertBubbleColor);
function load() {
    if (loaded || typeof localStorage === 'undefined') return;
    loaded = true;
    try {
        const value: unknown = JSON.parse(localStorage.getItem(storageKey) ?? '{}');
        if (!value || typeof value !== 'object' || Array.isArray(value)) return;
        for (const [id, color] of Object.entries(value)) if (valid(color)) saved.set(id, color);
    } catch { /* A color is a local display preference, never execution authority. */ }
}
export function expertBubbleColor(host: string, agent: string): ExpertBubbleColor {
    load(); return saved.get(key(host, agent)) ?? 'blue';
}
export function saveExpertBubbleColor(host: string, agent: string, color: ExpertBubbleColor): boolean {
    if (!host || !agent || !valid(color)) return false;
    load(); saved.set(key(host, agent), color);
    let persisted = false;
    try { localStorage.setItem(storageKey, JSON.stringify(Object.fromEntries(saved))); persisted = true; } catch { /* Session-local preference still applies. */ }
    for (const listener of listeners) listener();
    return persisted;
}
export function observeExpertBubbleColors(listener: () => void): () => void {
    listeners.add(listener); return () => listeners.delete(listener);
}
