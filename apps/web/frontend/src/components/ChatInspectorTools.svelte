<script lang="ts">
    import { onMount } from 'svelte';
    import { api } from '../lib/api';
    import { locale, t } from '../lib/i18n';
    import { parseChatBrowserConnection, parseChatToolCatalog } from '../lib/chatTools';
    import type { ChatBrowserConnection, ChatToolCatalog } from '../lib/chatTools';

    let catalog = $state<ChatToolCatalog | null>(null);
    let browser = $state<ChatBrowserConnection | null>(null);
    let loading = $state(false);
    let error = $state(false);
    let controller: AbortController | null = null;

    async function load() {
        controller?.abort();
        const request = new AbortController();
        controller = request;
        loading = true;
        error = false;
        const [catalogResult, browserResult] = await Promise.allSettled([
            api('/api/agent/tools', { signal: request.signal }),
            api('/api/native-browser/status', { signal: request.signal }),
        ]);
        if (request.signal.aborted) return;
        try { catalog = catalogResult.status === 'fulfilled' ? parseChatToolCatalog(catalogResult.value) : null; }
        catch { catalog = null; }
        try { browser = browserResult.status === 'fulfilled' ? parseChatBrowserConnection(browserResult.value) : null; }
        catch { browser = null; }
        error = catalog === null || browser === null;
        loading = false;
    }
    onMount(() => { void load(); return () => controller?.abort(); });
</script>

<section class="tool-inspector" aria-label={t($locale, 'chatTools.title')}>
    <div class="heading"><h3>{t($locale, 'chatTools.title')}</h3><button type="button" onclick={load} disabled={loading}>{t($locale, 'chatTools.refresh')}</button></div>
    {#if loading}<p class="muted" role="status">{t($locale, 'chatTools.loading')}</p>{/if}
    {#if error}<p class="error" role="alert">{t($locale, 'chatTools.loadFailed')}</p>{/if}
    {#if catalog}<div class="status-row"><span>{t($locale, 'chatTools.catalog')}</span><strong>{t($locale, 'chatTools.catalogCount', { count: catalog.names.length })}</strong></div>{/if}
    {#if browser}<div class="status-row"><span>{t($locale, 'chatTools.browser')}</span><strong class:connected={browser.connected}>{t($locale, browser.connected ? 'chatTools.connected' : 'chatTools.disconnected')}</strong></div>{/if}
    <p class="muted caveat">{t($locale, 'chatTools.catalogHint')}</p>
    <nav class="management" aria-label={t($locale, 'chatTools.manage')}>
        <a href="/settings">{t($locale, 'chatTools.systemAccess')} <span aria-hidden="true">↗</span></a>
        <a href="/capabilities">{t($locale, 'chatTools.skillsMcp')} <span aria-hidden="true">↗</span></a>
    </nav>
</section>

<style>
    .tool-inspector { display: grid; gap: 8px; padding: 12px; border: 1px solid var(--border); border-radius: 12px; background: var(--canvas); }
    .heading, .status-row, .management a { display: flex; align-items: center; justify-content: space-between; gap: 10px; }
    .heading h3 { margin: 0; font-size: 13px; }
    .heading button { font-size: 11px; }
    .status-row { min-height: 32px; font-size: 12px; }
    .status-row strong { font-size: 11px; font-weight: 600; color: var(--muted); }
    .status-row .connected { color: #19865d; }
    .caveat { margin: 0; font-size: 11px; line-height: 1.45; }
    .management { display: grid; gap: 2px; border-top: 1px solid var(--border); padding-top: 8px; }
    .management a { padding: 7px 2px; text-decoration: none; color: inherit; font-size: 12px; }
    .management a:hover { color: var(--accent); }
    .management span { color: var(--muted); }
</style>
