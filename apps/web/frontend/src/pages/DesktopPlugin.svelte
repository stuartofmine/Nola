<script lang="ts">
    import { api as base, errorMessage } from '../lib/api';
    import { desktopPluginCatalog, desktopPluginsChanged, pluginFrameDocument, type DesktopPluginDocument } from '../lib/desktopPluginPages';
    import { locale, t } from '../lib/i18n';
    import { workspacePageApi, workspacePageLocation } from '../lib/workspacePageLocation';

    const location = workspacePageLocation();
    const api = workspacePageApi(base);
    const id = $derived(location.read().searchParams.get('id'));
    const page = $derived($desktopPluginCatalog.pages.find(item => item.id === id));
    let loaded = $state<DesktopPluginDocument>();
    let error = $state('');
    $effect(() => {
        const item = page;
        if (!item) { loaded = undefined; error = ''; return; }
        if (!location.active()) return;
        const controller = new AbortController();
        error = '';
        void api<DesktopPluginDocument>('/api/desktop-plugin-pages', {
            method: 'POST', body: JSON.stringify({id: item.id}), signal: controller.signal,
        }).then(document => {
            if (controller.signal.aborted) return;
            if (document.id !== item.id || typeof document.html !== 'string') throw Error(t($locale, 'plugin.unavailable'));
            loaded = document;
        }).catch(cause => {
            if (!controller.signal.aborted) { loaded = undefined; error = errorMessage(cause); }
        });
        return () => controller.abort();
    });
    const frameDocument = $derived(loaded && loaded.id === id && page ? pluginFrameDocument(loaded.html, loaded.title) : undefined);
    let frameURL = $state<string>();
    $effect(() => {
        if (!frameDocument) { frameURL = undefined; return; }
        const url = URL.createObjectURL(new Blob([frameDocument], {type: 'text/html;charset=UTF-8'}));
        frameURL = url;
        return () => URL.revokeObjectURL(url);
    });
</script>
{#if frameDocument && frameURL && loaded}
    <iframe title={loaded.title} src={frameURL} sandbox="allow-scripts" referrerpolicy="no-referrer"></iframe>
{:else if error || $desktopPluginCatalog.error || ($desktopPluginCatalog.ready && !page)}
    <section class="content" role="status">
        <p>{error || $desktopPluginCatalog.error || t($locale, 'plugin.unavailable')}</p>
        {#if error || $desktopPluginCatalog.error}<button onclick={() => window.dispatchEvent(new Event(desktopPluginsChanged))}>{t($locale,'plugin.refresh')}</button>{/if}
        <a href="/capabilities?tab=plugins">{t($locale,'plugin.title')}</a>
    </section>
{:else}
    <p class="content" role="status">{t($locale,'shell.pageLoading')}</p>
{/if}
<style>
    iframe { display:block; width:100%; height:100%; min-height:16rem; border:0; background:var(--canvas); }
    @media(max-width:700px) { iframe { min-height:calc(100dvh - 130px - env(safe-area-inset-top) - env(safe-area-inset-bottom)); } }
    section a { display:inline-flex; align-items:center; min-height:44px; }
</style>
