<script lang="ts">
    import { onMount, untrack } from 'svelte';
    import ConfirmationDialog from './ConfirmationDialog.svelte';
    let removalConfirmation: ConfirmationDialog;
    import { api } from '../lib/api';
    import { loadConversationPdf } from '../lib/chatPdfPreview';
    import { locale, t } from '../lib/i18n';
    import { conversationPdfUrl, conversationResourcesUrl, parseConversationPdfResource, parseConversationPdfResources, pdfFileBase64 } from '../lib/chatResources';
    import type { ConversationPdfResource } from '../lib/chatResources';

    let { conversationId, generation, busy = false, onopen, onremove }: { conversationId: string; generation: number; busy?: boolean; onopen?: (resource: ConversationPdfResource) => void; onremove?: (resourceId: string) => void } = $props();
    let resources = $state<ConversationPdfResource[]>([]);
    let selected = $state<string | null>(null);
    let previewUrl = $state('');
    let previewLoading = $state(false);
    let loading = $state(false);
    let mutating = $state(false);
    let error = $state('');
    let mounted = false;
    let disposed = false;
    let request = 0;
    let controller: AbortController | null = null;
    let previewController: AbortController | null = null;
    let previewRequest = 0;
    const current = (id: string, epoch: number) => !disposed && conversationId === id && generation === epoch;
    const selectedResource = $derived(resources.find(item => item.resourceId === selected) ?? null);
    function clearPreview() {
        ++previewRequest;
        previewController?.abort(); previewController = null;
        if (previewUrl) URL.revokeObjectURL(previewUrl);
        previewUrl = ''; previewLoading = false;
    }
    async function select(resource: ConversationPdfResource) {
        if (selected === resource.resourceId) { selected = null; clearPreview(); return; }
        clearPreview(); selected = resource.resourceId;
        const id = conversationId, epoch = generation, sequence = previewRequest;
        const next = new AbortController(); previewController = next;
        previewLoading = true; error = '';
        try {
            const blob = await loadConversationPdf(resource, next.signal);
            if (!current(id, epoch) || selected !== resource.resourceId || sequence !== previewRequest) return;
            previewUrl = URL.createObjectURL(blob);
        } catch { if (current(id, epoch) && sequence === previewRequest && !next.signal.aborted) error = 'chatResource.previewFailed'; }
        finally { if (current(id, epoch) && sequence === previewRequest) previewLoading = false; }
    }

    async function load() {
        const id = conversationId, epoch = generation, sequence = ++request;
        controller?.abort(); const next = new AbortController(); controller = next;
        loading = true; error = '';
        try {
            const list = parseConversationPdfResources(await api(conversationResourcesUrl(id), { signal: next.signal }), id);
            if (!current(id, epoch) || request !== sequence) return;
            resources = list;
            if (selected && !list.some(item => item.resourceId === selected)) { selected = null; clearPreview(); }
        } catch {
            if (current(id, epoch) && request === sequence && !next.signal.aborted) error = 'chatResource.loadFailed';
        } finally { if (current(id, epoch) && request === sequence) loading = false; }
    }
    $effect(() => {
        conversationId; generation;
        if (mounted) untrack(() => { selected = null; clearPreview(); resources = []; mutating = false; void load(); });
    });
    onMount(() => { mounted = true; void load(); return () => { disposed = true; controller?.abort(); clearPreview(); }; });

    async function upload(event: Event) {
        const input = event.currentTarget as HTMLInputElement;
        const file = input.files?.[0]; input.value = '';
        if (!file || mutating || busy) return;
        const id = conversationId, epoch = generation;
        mutating = true; error = '';
        try {
            const dataBase64 = await pdfFileBase64(file);
            if (!current(id, epoch)) return;
            const descriptor = parseConversationPdfResource(await api(conversationResourcesUrl(id), {
                method: 'POST', body: JSON.stringify({ displayName: file.name, mimeType: 'application/pdf', dataBase64 }),
            }), id);
            if (!current(id, epoch)) return;
            await load();
            if (current(id, epoch)) await select(descriptor);
        } catch (cause) { if (current(id, epoch)) {
            if (cause instanceof Error && cause.message === 'chatResource.invalidFile') error = 'chatResource.invalidFile';
            else { await load(); error = 'chatResource.uploadFailed'; }
        } }
        finally { if (current(id, epoch)) mutating = false; }
    }
    async function remove(resource: ConversationPdfResource) {
        if (mutating || busy || resource.conversationId !== conversationId) return;
        const id = conversationId, epoch = generation;
        if (!await removalConfirmation.ask({title: t($locale, 'chatResource.remove'),
            description: t($locale, 'chatResource.confirmRemove', {name: resource.displayName}),
            confirmLabel: t($locale, 'chatResource.remove'), destructive: true})) return;
        if (!current(id, epoch) || mutating || busy) return;
        mutating = true; error = '';
        try {
            await api(conversationPdfUrl(id, resource.resourceId), { method: 'DELETE' });
            if (!current(id, epoch)) return;
            if (selected === resource.resourceId) { selected = null; clearPreview(); }
            onremove?.(resource.resourceId);
            await load();
        } catch { if (current(id, epoch)) error = 'chatResource.deleteFailed'; }
        finally { if (current(id, epoch)) mutating = false; }
    }
</script>

<ConfirmationDialog bind:this={removalConfirmation} />
<section class="pdf-resources" aria-label={t($locale, 'chatResource.title')}>
    <h2>{t($locale, 'chatResource.title')}</h2>
    <p class="muted">{t($locale, 'chatResource.previewOnly')}</p>
    <label class="upload" class:disabled={busy || mutating}>{t($locale, 'chatResource.upload')}<input type="file" accept="application/pdf,.pdf" onchange={upload} disabled={busy || mutating} /></label>
    <button type="button" onclick={load} disabled={loading || mutating}>{t($locale, 'chatResource.refresh')}</button>
    {#if error}<p class="error" role="alert">{t($locale, error)}</p>{/if}
    {#if loading}<p role="status">{t($locale, 'chatResource.loading')}</p>{/if}
    {#if !loading && !resources.length}<p class="muted">{t($locale, 'chatResource.empty')}</p>{/if}
    {#if resources.length}<ul>{#each resources as resource (resource.resourceId)}<li>
        <button type="button" class:chosen={selected === resource.resourceId} onclick={() => select(resource)}>{resource.displayName}</button>
        <span class="muted">{Math.ceil(resource.byteCount / 1024).toLocaleString($locale)} KiB</span>
        <button type="button" class="remove" onclick={() => remove(resource)} disabled={busy || mutating} aria-label={t($locale, 'chatResource.remove')}>{t($locale, 'chatResource.remove')}</button>
    </li>{/each}</ul>{/if}
    {#if previewLoading}<p role="status">{t($locale, 'chatResource.loadingPreview')}</p>{/if}
    {#if selectedResource && previewUrl}<div class="preview"><iframe sandbox="allow-scripts" src={previewUrl} title={t($locale, 'chatResource.previewTitle', { name: selectedResource.displayName })}></iframe>
        {#if onopen}<button type="button" onclick={() => selectedResource && onopen?.(selectedResource)}>{t($locale, 'chatResource.openTab')}</button>{/if}
        <p class="muted">{t($locale, 'chatResource.fallback')}</p><a href={previewUrl} download={selectedResource.displayName}>{t($locale, 'chatResource.download')}</a><a href={previewUrl} target="_blank" rel="noopener noreferrer">{t($locale, 'chatResource.openSeparate')}</a></div>{/if}
</section>

<style>
    .pdf-resources { display: grid; gap: 10px; min-width: 0; }
    h2,p { margin: 0; } h2 { font-size: 1rem; } .muted { color: var(--muted); font-size: 12px; line-height: 1.45; }
    .upload { position: relative; display: inline-flex; width: fit-content; max-width: 100%; padding: 5px 8px; border: 1px solid var(--border); border-radius: 7px; font-size: 12px; font-weight: 600; cursor: pointer; }
    .upload.disabled { color: var(--muted); background: var(--panel); cursor: not-allowed; }
    .upload input { position: absolute; width: 1px; height: 1px; overflow: hidden; opacity: 0; }
    .upload:focus-within { outline: 2px solid var(--accent); outline-offset: 2px; }
    ul { display: grid; gap: 5px; list-style: none; padding: 0; margin: 0; }
    li { display: flex; align-items: center; gap: 6px; min-width: 0; }
    li > button:first-child { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; text-align: start; }
    .remove { flex: none; font-size: 11px; } .chosen { border-color: var(--accent); }
    .preview { display: grid; gap: 8px; min-width: 0; } .preview iframe { width: 100%; height: min(60dvh, 480px); min-height: 180px; border: 1px solid var(--outline); border-radius: 8px; background: var(--surface); }
    .preview a { font-size: .875rem; color: var(--accent); text-decoration: underline; text-underline-offset: 3px; padding-block: 8px; } .error { color: var(--danger); font-size: 12px; }
</style>
