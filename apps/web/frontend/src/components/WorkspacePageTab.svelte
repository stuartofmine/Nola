<script lang="ts">
    import { setContext, onMount, onDestroy, tick, untrack } from 'svelte';
    import { createWorkspacePageScope } from '../lib/workspacePageScope';
    import { pageLoader } from '../lib/pageRoutes';
    import { workspacePageLocationContext, type WorkspacePageLocation } from '../lib/workspacePageLocation';
    import { locale, t } from '../lib/i18n';
    let {path, ownerKey, viewport, active, onlocation}: {path: string; ownerKey: string; viewport: {top: number}; active: boolean; onlocation: (path: string) => void} = $props();
    let scroller: HTMLDivElement;
    const reading = untrack(() => viewport);
    const scope = createWorkspacePageScope(untrack(() => ownerKey));
    $effect(() => scope.setActive(active));
    let alive = true;
    let loaded = $state(false);
    onMount(() => { void Promise.resolve(page).then(async () => { await tick(); if (alive) loaded = true; }, () => {}); });
    $effect(() => {
        if (loaded && active) void tick().then(() => { if (alive && active && scroller) scroller.scrollTop = reading.top; });
    });
    onDestroy(() => { alive = false; scope.destroy(); });
    const initial = new URL(untrack(() => path), typeof window === 'undefined' ? 'http://nola.invalid/' : window.location.origin);
    // Keep one mounted editor per tab. Section/query changes update only its
    // own location, not the top-level route or the conversation's owner.
    const load = pageLoader(initial.pathname);
    const page = load?.();
    setContext<WorkspacePageLocation>(workspacePageLocationContext, {
        key: untrack(() => ownerKey),
        scope,
        read: () => new URL(path, typeof window === 'undefined' ? 'http://nola.invalid/' : window.location.origin),
        active: () => active,
        replace: url => {
            if (!alive || url.origin !== initial.origin || url.pathname !== initial.pathname) return;
            onlocation(url.pathname + url.search + url.hash);
        },
    });
</script>
<div class="page-tab" use:scope.mount bind:this={scroller} inert={!active} onscroll={() => { if (active && loaded) reading.top = scroller.scrollTop; }}>
    {#await page}<p role="status">{t($locale, 'shell.pageLoading')}</p>
    {:then module}{#if module}<module.default />{:else}<p role="alert">{t($locale, 'shell.pageLoadFailed')}</p>{/if}
    {:catch}<p role="alert">{t($locale, 'shell.pageLoadFailed')}</p>{/await}
</div>
<style>
    .page-tab { min-width: 0; width: 100%; flex: 1 1 auto; align-self: stretch; height: 100%; overflow: auto; }
    .page-tab :global(.page-header) { padding-top: 24px; }
</style>
