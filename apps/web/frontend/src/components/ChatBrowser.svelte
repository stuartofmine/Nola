<script lang="ts">
    import { onMount, untrack } from 'svelte';
    import { api } from '../lib/api';
    import { applyManagedBrowserPages, chatBrowserActivityKey, chatBrowserSnapshot, localBrowserPreviewURL } from '../lib/chatBrowser';
    import type { ChatBrowserSnapshot } from '../lib/chatBrowser';
    import type { TimelineMessage } from '../lib/chatTimeline';
    import { parseChatBrowserConnection } from '../lib/chatTools';
    import type { ChatBrowserConnection } from '../lib/chatTools';
    import { locale, t } from '../lib/i18n';

    let { messages, conversationId, generation }: { messages: TimelineMessage[]; conversationId: string; generation: number } = $props();
    let connection = $state<ChatBrowserConnection | null>(null);
    let loading = $state(false);
    let error = $state(false);
    let controller: AbortController | null = null;
    let pagesController: AbortController | null = null;
    let pageState = $state<{ conversationId: string; generation: number; activityKey: string; snapshot: ChatBrowserSnapshot } | null>(null);
    let switching = $state(false);
    let pageError = $state(false);
    let expanded = $state(false);
    let previewDialog = $state<HTMLDialogElement>();
    let previewToggle = $state<HTMLButtonElement>();
    function collapsePreview(event?: Event) {
        event?.preventDefault(); event?.stopPropagation();
        if (!expanded || !previewDialog) return;
        previewDialog.close(); expanded = false; previewDialog.open = true;
        previewToggle?.focus({ preventScroll: true });
    }
    function expandPreview() {
        if (!previewDialog) return;
        if (expanded) { collapsePreview(); return; }
        previewDialog.close(); previewDialog.showModal(); expanded = true;
        previewToggle?.focus({ preventScroll: true });
    }
    const snapshot = $derived(chatBrowserSnapshot(messages));
    const activityKey = $derived(chatBrowserActivityKey(messages));
    const current = $derived(pageState && pageState.conversationId === conversationId && pageState.generation === generation && pageState.activityKey === activityKey ? pageState.snapshot : snapshot);
    const preview = $derived(current?.previewUrl && typeof window !== 'undefined'
        ? localBrowserPreviewURL(current.previewUrl, window.location.href) : null);

    async function loadPages(id: string, ownerGeneration: number) {
        pagesController?.abort();
        const request = new AbortController(); pagesController = request;
        pageState = null; pageError = false;
        const sourceKey = activityKey;
        try {
            const result = await api(`/api/chat/conversations/${encodeURIComponent(id)}/browser`, { signal: request.signal });
            if (!request.signal.aborted && conversationId === id && generation === ownerGeneration && activityKey === sourceKey)
                pageState = { conversationId: id, generation: ownerGeneration, activityKey: sourceKey, snapshot: applyManagedBrowserPages(snapshot, result) };
        } catch { /* A historical browser may no longer be running; keep its completed snapshot. */ }
    }
    async function switchPage(index: number) {
        if (switching || !Number.isSafeInteger(index) || index < 0 || current?.target !== 'managed') return;
        const id = conversationId; const ownerGeneration = generation;
        const sourceKey = activityKey;
        switching = true; pageError = false;
        try {
            const result = await api(`/api/chat/conversations/${encodeURIComponent(id)}/browser/switch-page`, {
                method: 'POST', body: JSON.stringify({ index }),
            });
            if (conversationId === id && generation === ownerGeneration && activityKey === sourceKey)
                pageState = { conversationId: id, generation: ownerGeneration, activityKey: sourceKey, snapshot: applyManagedBrowserPages(current, result) };
        } catch { if (conversationId === id && generation === ownerGeneration) pageError = true; }
        finally { switching = false; }
    }
    $effect(() => { const id = conversationId; const ownerGeneration = generation; untrack(() => { void loadPages(id, ownerGeneration); }); return () => pagesController?.abort(); });

    async function load() {
        controller?.abort();
        const request = new AbortController(); controller = request;
        loading = true; error = false;
        try {
            const response = await api('/api/native-browser/status', { signal: request.signal });
            if (!request.signal.aborted) connection = parseChatBrowserConnection(response);
        } catch { if (!request.signal.aborted) error = true; }
        finally { if (!request.signal.aborted) loading = false; }
    }
    function refresh() { void load(); void loadPages(conversationId, generation); }
    onMount(() => { void load(); return () => { controller?.abort(); pagesController?.abort(); }; });
</script>

<section class="browser-inspector" aria-label={t($locale, 'chatBrowser.title')}>
    <div class="heading"><h2>{t($locale, 'chatBrowser.title')}</h2><button type="button" onclick={refresh} disabled={loading}>{t($locale, 'chatBrowser.refresh')}</button></div>
    <div class="status"><span>{t($locale, 'chatBrowser.managed')}</span><strong>{t($locale, current?.target === 'managed' && current.connected ? 'chatBrowser.active' : 'chatBrowser.available')}</strong></div>
    {#if current?.target === 'managed' && current.pages.length}
        <ol class="pages" aria-label={t($locale, 'chatBrowser.pages')}>
            {#each current.pages as page (page.index)}
                <li class:active={page.active}>
                    <button type="button" aria-current={page.active ? 'page' : undefined} disabled={page.active || switching} onclick={() => { void switchPage(page.index); }}><strong>{page.title || t($locale, 'chatBrowser.untitled')}</strong></button>
                    <span title={page.url}>{page.url}</span>
                </li>
            {/each}
        </ol>
    {:else}<p class="muted">{t($locale, 'chatBrowser.noPages')}</p>{/if}
    {#if preview && current?.target === 'managed'}
        <dialog open bind:this={previewDialog} class:expanded class="preview-shell" aria-label={t($locale, 'chatBrowser.previewTitle')} oncancel={collapsePreview}>
            <div class="preview-actions"><span title={current.currentUrl}>{current.currentUrl}</span><button bind:this={previewToggle} type="button" onclick={expandPreview} aria-expanded={expanded}>{t($locale, expanded ? 'chatBrowser.collapse' : 'chatBrowser.expand')}</button></div>
            <iframe src={preview} title={t($locale, 'chatBrowser.previewTitle')} sandbox="allow-scripts allow-same-origin" referrerpolicy="no-referrer" loading="lazy"></iframe>
        </dialog>
    {:else if current?.previewUrl && current.target === 'managed'}<p class="muted">{t($locale, 'chatBrowser.previewUnavailable')}</p>
    {:else}<p class="muted">{t($locale, 'chatBrowser.previewPending')}</p>{/if}
    {#if pageError}<p class="error" role="alert">{t($locale, 'chatBrowser.switchFailed')}</p>{/if}
    <div class="status"><span>{t($locale, 'chatBrowser.native')}</span><strong>{connection ? t($locale, connection.connected ? 'chatBrowser.connected' : 'chatBrowser.disconnected') : '—'}</strong></div>
    {#if loading}<p class="muted" role="status">{t($locale, 'chatBrowser.loading')}</p>{/if}
    {#if error}<p class="error" role="alert">{t($locale, 'chatBrowser.loadFailed')}</p>{/if}
</section>

<style>
    .browser-inspector { display: grid; gap: 12px; min-width: 0; padding: 13px; border: 1px solid var(--border); border-radius: 11px; background: var(--panel); }
    .heading,.status { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
    h2,p { margin: 0; } h2 { font-size: 13px; }
    .heading button,.status { font-size: 11px; }
    .status strong { color: var(--muted); font-weight: 600; }
    .pages { display: grid; gap: 5px; margin: 0; padding: 0; list-style: none; }
    .pages li { display: grid; gap: 2px; padding: 8px; border: 1px solid var(--border); border-radius: 8px; min-width: 0; font-size: 11px; }
    .pages li.active { background: var(--accent-subtle); }
    .pages button { width: 100%; padding: 0; text-align: start; border: 0; background: transparent; cursor: pointer; }
    .pages button:disabled { cursor: default; }
    .pages strong,.pages span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .pages span,.muted { color: var(--muted); } .muted,.error { font-size: 11px; line-height: 1.45; }
    .error { color: var(--danger); }
    .preview-shell { position: static; margin: 0; padding: 0; width: 100%; max-width: none; max-height: none; box-shadow: none; min-width: 0; overflow: hidden; border: 1px solid var(--border); border-radius: 9px; background: var(--canvas); }
    .preview-actions { display: flex; align-items: center; justify-content: space-between; gap: 8px; padding: 5px 8px; font-size: 10px; }
    .preview-actions span { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--muted); }
    .preview-actions button { flex: none; font-size: 10px; }
    iframe { display: block; width: 100%; height: 350px; border: 0; }
    .preview-shell.expanded { position: fixed; inset: 0; margin: auto; width: calc(100vw - 24px); height: calc(100dvh - 24px); max-height: calc(100dvh - 24px); display: flex; flex-direction: column; box-shadow: 0 18px 70px #0008; }
    .preview-shell::backdrop { background: rgb(0 0 0 / .4); }
    .expanded iframe { flex: 1; min-height: 0; height: auto; }
    .preview-actions { flex-shrink: 0; }
</style>
