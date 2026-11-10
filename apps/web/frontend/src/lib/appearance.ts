import { derived, get, writable } from 'svelte/store';

export type Appearance = 'system' | 'light' | 'dark';
const key = 'nola.appearance';
const systemDark = writable(typeof matchMedia === 'function' && matchMedia('(prefers-color-scheme: dark)').matches);
export const appearance = writable<Appearance>('system');
export const darkAppearance = derived([appearance, systemDark], ([value, dark]) => value === 'dark' || value === 'system' && dark);
export function initializeAppearance() {
    if (typeof document === 'undefined' || typeof matchMedia !== 'function') return () => {};
    try {
        const saved = localStorage.getItem(key);
        if (saved === 'system' || saved === 'light' || saved === 'dark') appearance.set(saved);
    } catch { /* Browser storage is optional. */ }
    const media = matchMedia('(prefers-color-scheme: dark)');
    const changed = () => systemDark.set(media.matches);
    changed(); media.addEventListener('change', changed);
    const unsubscribe = appearance.subscribe(value => {
        if (value === 'system') delete document.documentElement.dataset.appearance;
        else document.documentElement.dataset.appearance = value;
        try { localStorage.setItem(key, value); } catch { /* Keep the current page preference. */ }
    });
    return () => { unsubscribe(); media.removeEventListener('change', changed); };
}
export function toggleAppearance() { appearance.set(get(darkAppearance) ? 'light' : 'dark'); }
