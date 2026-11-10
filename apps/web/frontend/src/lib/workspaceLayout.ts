import { writable } from 'svelte/store';

export const layoutDefaults = {sidebar: 224, inspector: 360};
export type PanelWidths = typeof layoutDefaults;
export const workspaceWidths = writable<PanelWidths>({...layoutDefaults});
export const layoutStorageKey = 'nola.workspace.panel-widths.v1';
const limits = {sidebar: [220, 360], inspector: [320, 480]} as const;
export function panelWidth(panel: keyof PanelWidths, value: unknown): number {
    return typeof value === 'number' && Number.isFinite(value)
        ? Math.round(Math.max(limits[panel][0], Math.min(limits[panel][1], value))) : layoutDefaults[panel];
}
export function loadWorkspaceWidths(storage: Pick<Storage, 'getItem'>): void {
    try {
        const value = JSON.parse(storage.getItem(layoutStorageKey) ?? '{}');
        workspaceWidths.set({sidebar: panelWidth('sidebar', value?.sidebar), inspector: panelWidth('inspector', value?.inspector)});
    } catch { workspaceWidths.set({...layoutDefaults}); }
}
export function setWorkspaceWidth(panel: keyof PanelWidths, width: number): void {
    workspaceWidths.update(current => {
        const next = {...current, [panel]: panelWidth(panel, width)};
        try { localStorage.setItem(layoutStorageKey, JSON.stringify(next)); } catch { /* Layout still works when storage is unavailable. */ }
        return next;
    });
}
/** Clamp the rendered layout, not the saved preference, when a window narrows. */
export function visiblePanelWidths(viewport: number, saved: PanelWidths, chat: boolean) {
    const inspectorVisible = chat && viewport > 1250;
    const sidebarMax = Math.max(220, Math.min(360, viewport - 420 - 56 - (inspectorVisible ? 332 : 0)));
    const sidebar = Math.min(panelWidth('sidebar', saved.sidebar), sidebarMax);
    const inspectorMax = Math.max(320, Math.min(viewport <= 1000 ? 380 : 480, viewport - sidebar - 420 - 56));
    return {sidebar, inspector: Math.min(panelWidth('inspector', saved.inspector), inspectorMax), sidebarMax, inspectorMax};
}
