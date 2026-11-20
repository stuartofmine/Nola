<script lang="ts">
    import { onMount } from 'svelte';
    import { locale, t } from '../lib/i18n';
    import { desktopPluginCatalog, desktopPluginPath, type DesktopPluginPage } from '../lib/desktopPluginPages';
    import PrototypeIcon from './PrototypeIcon.svelte';
    let {selectedPath, onpages}: {selectedPath?: string; onpages: (pages: DesktopPluginPage[], removed: Set<string>) => void} = $props();
    let pages = $state<DesktopPluginPage[]>([]);
    onMount(() => {
        let previous = new Set<string>();
        return desktopPluginCatalog.subscribe(snapshot => {
            pages = snapshot.pages;
            const current = new Set(pages.map(page => desktopPluginPath(page.id)));
            const removed = snapshot.ready ? new Set([...previous].filter(path => !current.has(path))) : new Set<string>();
            if (snapshot.ready) previous = current;
            onpages(pages, removed);
        });
    });
</script>
{#if pages.length}
    <nav class="plugins" aria-label={t($locale, 'plugin.pages')}>
        {#each pages as page (page.id)}
            {@const path = desktopPluginPath(page.id)}
            {@const duplicate = pages.some(other => other.id !== page.id && other.title === page.title)}
            <a href={path} aria-current={selectedPath === path ? 'page' : undefined} title={page.package}>
                <PrototypeIcon name="overview" size={17} /><span>{page.title}{#if duplicate}<small>{page.package}</small>{/if}</span>
            </a>
        {/each}
    </nav>
{/if}
<style>
    .plugins { display:grid; flex:0 1 auto; min-height:0; max-height:36%; overflow:auto; padding:4px 8px; border-top:1px solid var(--border); }
    a { display:flex; align-items:center; gap:8px; min-height:44px; padding:6px 10px; border-radius:8px; color:var(--content); text-decoration:none; }
    a[aria-current], a:hover { background:var(--panel); }
    a:focus-visible { outline:2px solid var(--accent); outline-offset:-2px; }
    span { min-width:0; overflow-wrap:anywhere; } small { display:block; color:var(--muted); }
</style>
