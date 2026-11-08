// Static import targets keep bundling predictable; URL input never becomes a module path.
const pages = {
    '/models': () => import('../pages/ModelCenter.svelte'),
    '/model-test': () => import('../pages/ModelTest.svelte'),
    '/login': () => import('../pages/Login.svelte'),
    '/capabilities': () => import('../pages/Capabilities.svelte'),
    '/activity': () => import('../pages/Activity.svelte'),
    '/results': () => import('../pages/Results.svelte'),
    '/dashboard': () => import('../pages/Dashboard.svelte'),
    '/data-sources': () => import('../pages/DataSources.svelte'),
    '/settings': () => import('../pages/Settings.svelte'),
    '/scheduled-actions': () => import('../pages/Automations.svelte'),
    '/workspace-review': () => import('../pages/WorkspaceReview.svelte'),
    '/today': () => import('../pages/Today.svelte'),
    '/conversations': () => import('../pages/Conversations.svelte'),
    '/chat': () => import('../pages/Chat.svelte'),
};

export function pageLoader(path: string) {
    return Object.hasOwn(pages, path) ? pages[path as keyof typeof pages] : undefined;
}
