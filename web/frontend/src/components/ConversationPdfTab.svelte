<script lang="ts">
    import { onMount } from 'svelte';
    import { loadConversationPdf } from '../lib/chatPdfPreview';
    import type { ConversationPdfResource } from '../lib/chatResources';
    import { locale, t } from '../lib/i18n';
    let {resource}: {resource: ConversationPdfResource} = $props();
    let url = $state(''), error = $state(false), loading = $state(true);
    let controller: AbortController | null = null;
    let request = 0;
    async function load() {
        const sequence = ++request;
        controller?.abort(); controller = new AbortController();
        const signal = controller.signal;
        loading = true; error = false;
        try {
            const blob = await loadConversationPdf(resource, signal);
            if (signal.aborted || sequence !== request) return;
            if (url) URL.revokeObjectURL(url);
            url = URL.createObjectURL(blob);
        } catch { if (!signal.aborted && sequence === request) error = true; }
        finally { if (!signal.aborted && sequence === request) loading = false; }
    }
    onMount(() => { void load(); return () => { ++request; controller?.abort(); if (url) URL.revokeObjectURL(url); }; });
</script>
<div class="document-tab">
    {#if loading}<p role="status">{t($locale, 'chatResource.loadingPreview')}</p>{/if}
    {#if error}<div role="alert"><p>{t($locale, 'chatResource.previewFailed')}</p><button onclick={load}>{t($locale, 'chatResource.refresh')}</button></div>{/if}
    {#if url}<div class="preview-actions"><p>{t($locale, 'chatResource.fallback')}</p><a href={url} target="_blank" rel="noopener noreferrer">{t($locale, 'chatResource.openSeparate')}</a><a href={url} download={resource.displayName}>{t($locale, 'chatResource.download')}</a></div><iframe sandbox="allow-scripts" src={url} title={t($locale, 'chatResource.previewTitle', {name: resource.displayName})}></iframe>{/if}
</div>
<style>
    .document-tab { display: flex; flex: 1; flex-direction: column; min-width: 0; min-height: 0; height: 100%; padding: 16px; gap: 12px; }
    .preview-actions { display:flex; flex-wrap:wrap; gap:8px 16px; font-size:13px; }
    .preview-actions p { width:100%; margin:0; color:var(--muted); }
    .preview-actions a { min-height:44px; display:inline-flex; align-items:center; }
    iframe { width: 100%; flex: 1; min-height: 240px; border: 0; background: var(--canvas); }
</style>
