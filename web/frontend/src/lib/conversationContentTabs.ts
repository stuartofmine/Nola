import type { ConversationPdfResource } from './chatResources';

export type PdfContentTab = { key: string; kind: 'pdf'; resource: ConversationPdfResource };
export type PageContentTab = {key: string; kind: 'page'; path: string; titleKey: string; viewport: {top: number}};
export type ContentTab = PdfContentTab | PageContentTab;
export type ConversationContentTabs = { selected: string; items: readonly ContentTab[] };
const pageTitles: Record<string, string> = {
    '/today': 'chatInspector.today', '/conversations': 'chatInspector.list.conversations',
    '/workspace-review': 'chatInspector.review',
    '/models': 'shell.nav.models', '/model-test': 'shell.nav.models', '/capabilities': 'shell.nav.capabilities',
    '/activity': 'chatInspector.list.activity', '/results': 'activity.results', '/dashboard': 'activity.overview', '/data-sources': 'shell.information',
    '/settings': 'shell.settings', '/scheduled-actions': 'activity.automations',
};
export function managementContentPath(href: string, origin: string): {path: string; titleKey: string} | null {
    try {
        const url = new URL(href, origin);
        if (url.origin !== new URL(origin).origin || url.username || url.password || !Object.hasOwn(pageTitles, url.pathname)) return null;
        const dataTitles: Record<string, string> = {knowledge: 'chatKnowledge.knowledgePoints',
            sources: 'data_sources.tab.sources', memory: 'data_sources.tab.memory', plans: 'data_sources.tab.plans',
            activity: 'shell.informationActivity', usage: 'data_sources.tab.usage'};
        const titleKey = url.pathname === '/data-sources' ? dataTitles[url.searchParams.get('tab') ?? ''] ?? pageTitles[url.pathname] : pageTitles[url.pathname];
        return {path: url.pathname + url.search + url.hash, titleKey};
    } catch { return null; }
}
export function openPageContentTab(state: ConversationContentTabs, path: string, titleKey: string): ConversationContentTabs {
    const key = JSON.stringify(['page', path]);
    // An editor can change its section without changing its object identity.
    // Opening its original address must select that editor, never duplicate its key.
    const existing = state.items.find(item => item.kind === 'page' && (item.path === path || item.key === key));
    if (existing) return {...state, selected: existing.key};
    return {selected: key, items: [...state.items, {key, kind: 'page', path, titleKey, viewport: {top: 0}}]};
}
export function updatePageContentLocation(state: ConversationContentTabs, key: string, path: string): ConversationContentTabs {
    return {...state, items: state.items.map(item => item.kind === 'page' && item.key === key ? {...item, path, titleKey: managementContentPath(path, 'http://nola.invalid')?.titleKey ?? item.titleKey} : item)};
}
export const conversationTabKey = 'conversation';
export const emptyContentTabs = (): ConversationContentTabs => ({selected: conversationTabKey, items: []});
export const pdfTabKey = (id: string) => JSON.stringify(['pdf', id]);
export function openPdfContentTab(state: ConversationContentTabs, conversationId: string, resource: ConversationPdfResource): ConversationContentTabs {
    if (resource.conversationId !== conversationId) throw new Error('Content tab belongs to another conversation');
    const key = pdfTabKey(resource.resourceId);
    const existing = state.items.find(item => item.key === key);
    return {selected: key, items: existing ? state.items.map(item => item.key === key && item.kind === 'pdf' ? {...item, resource: {...resource}} : item)
        : [...state.items, {key, kind: 'pdf', resource: {...resource}}]};
}
export function selectContentTab(state: ConversationContentTabs, key: string): ConversationContentTabs {
    if (key !== conversationTabKey && !state.items.some(item => item.key === key)) return state;
    return {...state, selected: key};
}
export function closeContentTab(state: ConversationContentTabs, key: string): ConversationContentTabs {
    const index = state.items.findIndex(item => item.key === key);
    if (index < 0) return state; // The permanent conversation tab cannot close.
    return {items: state.items.filter(item => item.key !== key), selected: state.selected !== key ? state.selected
        : index === 0 ? conversationTabKey : state.items[index - 1].key};
}

export function contentTabDomId(kind: 'tab' | 'panel', owner: string, key: string): string {
    return `content-${kind}-${encodeURIComponent(JSON.stringify([owner, key]))}`;
}
