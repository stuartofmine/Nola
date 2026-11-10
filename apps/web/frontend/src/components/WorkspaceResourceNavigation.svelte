<script lang="ts">
    import { locale, t } from '../lib/i18n';
    let {onopen, selectedPath = ''}: {onopen: (path: string) => void; selectedPath?: string} = $props();
    const entries = [
        ['/data-sources', 'shell.nav.data'], ['/models', 'shell.nav.models'], ['/capabilities', 'shell.nav.capabilities'],
        ['/data-sources?tab=memory', 'data_sources.tab.memory'], ['/data-sources?tab=plans', 'data_sources.tab.plans'],
        ['/activity', 'chatInspector.list.activity'],
    ];
    const selectedEntry = $derived.by(() => {
        if (!selectedPath) return '';
        const current = new URL(selectedPath, 'http://nola.invalid');
        return entries.filter(([path]) => {
            const entry = new URL(path, current.origin);
            return entry.pathname === current.pathname
                && [...entry.searchParams].every(([key, value]) => current.searchParams.get(key) === value);
        }).at(-1)?.[0] ?? '';
    });
    function open(event: MouseEvent, path: string) {
        if (event.button !== 0 || event.ctrlKey || event.metaKey || event.shiftKey || event.altKey) return;
        event.preventDefault(); onopen(path);
    }
</script>
<nav class="resource-navigation" aria-label={t($locale, 'chatInspector.list.resources')}>
    {#each entries as [path, title]}<a href={path} aria-current={selectedEntry === path ? 'page' : undefined} onclick={event => open(event, path)}>{t($locale, title)}</a>{/each}
</nav>
<style>
    .resource-navigation { display: grid; padding: 8px 0; }
    a { display: flex; align-items: center; min-height: 44px; padding: 8px 12px; border-radius: 0; border-bottom: 1px solid var(--border); }
    .resource-navigation a[aria-current], .resource-navigation a[aria-current]:hover { background: var(--panel); border-bottom-color: var(--border); }
    a:hover { background: var(--panel); }
</style>
