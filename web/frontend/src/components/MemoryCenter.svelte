<script lang="ts">
    import { workspacePageLocation } from '../lib/workspacePageLocation';
    const pageLocation = workspacePageLocation();
    import { workspacePageApi } from '../lib/workspacePageLocation';
    const api = workspacePageApi(unscopedApi);
    import { workspaceUnloadScope } from '../lib/workspacePageLocation';
    const acceptsWorkspaceUnload = workspaceUnloadScope();
    import { onMount, tick } from 'svelte';
    import { api as unscopedApi } from '../lib/api';
    import { locale, t } from '../lib/i18n';
    import { memoryRecords, memoryView, memoryDeleteUrl, memoryDeletionConfirmed } from '../lib/memoryCenter';
    import ConfirmationDialog from './ConfirmationDialog.svelte';
    import MemoryEditor from './MemoryEditor.svelte';
    let editing = $state(false), editRecord = $state<MemoryRecord>();
    let editorOpener: HTMLButtonElement | undefined;
    function edit(record: MemoryRecord | undefined, opener: HTMLButtonElement) {
        if (loading || removing || editing) return;
        editRecord = record ? $state.snapshot(record) : undefined; editorOpener = opener; editing = true;
    }
    async function closeEditor() {editing = false; await tick(); if (disposed || !pageLocation.active()) return; if (editorOpener?.isConnected) editorOpener.focus(); else refreshButton?.focus();}
    let confirmation: ConfirmationDialog;
    let refreshButton: HTMLButtonElement;
    let removing = $state(false), deletionFailed = $state(false);
    import type { MemoryRecord } from '../lib/memoryCenter';
    let records = $state<MemoryRecord[]>([]), query = $state('');
    let loading = $state(false), failed = $state(false), loaded = $state(false);
    let disposed = false, controller: AbortController | undefined;
    const visible = $derived(memoryView(records, query, $locale));
    async function load() {
        if (loading || disposed) return;
        loading = true; controller = new AbortController();
        try {
            const next = memoryRecords(await api('/api/memory', {signal: controller.signal}));
            if (!disposed) {records = next; loaded = true; failed = false;}
        } catch {if (!disposed) failed = true;}
        finally {if (!disposed) loading = false;}
    }
    onMount(() => {void load(); return () => {disposed = true; controller?.abort();};});
    async function remove(id: string, opener: HTMLButtonElement) {
        if (removing || loading || !records.some(record => record.id === id)) return;
        removing = true; deletionFailed = false;
        try {
            const approved = await confirmation.ask({title: t($locale, 'memoryCenter.delete'), description: t($locale, 'memoryCenter.deleteBody'), confirmLabel: t($locale, 'memoryCenter.delete'), destructive: true});
            if (!approved || disposed) return;
            try {
                const receipt = await api(memoryDeleteUrl(id), {method: 'DELETE'});
                if (disposed) return;
                if (!memoryDeletionConfirmed(receipt, id)) throw new Error('Unconfirmed deletion');
                records = records.filter(record => record.id !== id);
            } catch {if (!disposed) deletionFailed = true;}
            if (!disposed) await load();
        } finally {
            if (!disposed) {
                removing = false; await tick();
                if (!pageLocation.active()) return;
                if (opener.isConnected) opener.focus(); else refreshButton?.focus();
            }
        }
    }
</script>
<svelte:window onbeforeunload={event => { if (!acceptsWorkspaceUnload(event)) return;if (removing) {event.preventDefault(); event.returnValue = '';}}} />
<ConfirmationDialog bind:this={confirmation} />
{#if editing}<MemoryEditor record={editRecord} onclose={closeEditor} onrefresh={load} />{/if}
<section class="memory-center card" aria-busy={loading}>
    <div class="row memory-actions"><button bind:this={refreshButton} disabled={loading || removing} onclick={load}>{t($locale, 'data_sources.action.refresh')}</button></div>
    <p class="muted">{t($locale, 'memoryCenter.hint')}</p>
    <button disabled={loading || removing || editing || !loaded} onclick={event => edit(undefined, event.currentTarget)}>{t($locale, 'memoryCenter.create')}</button>
    <label>{t($locale, 'memoryCenter.search')}<input type="search" bind:value={query} /></label>
    {#if loading}<p role="status">{t($locale, 'memoryCenter.loading')}</p>{/if}
    {#if failed}<p role="alert" class="error">{t($locale, 'memoryCenter.failed')}{#if loaded} {t($locale, 'data_sources.stale')}{/if}</p>{/if}
    {#if deletionFailed}<p role="alert" class="error">{t($locale, 'memoryCenter.deleteFailed')}</p>{/if}
    {#if loaded}
        <p class="muted">{t($locale, 'memoryCenter.limit')}</p>
        {#each visible as {record, superseded} (record.id)}
            <article class:superseded><div class="row between"><strong>{['global', 'workspace', 'conversation', 'turn'].includes(record.scope) ? t($locale, `memoryCenter.scope.${record.scope}`) : record.scope} · {['preference', 'identity', 'decision', 'project', 'feedback', 'reference', 'other'].includes(record.kind) ? t($locale, `memoryCenter.kind.${record.kind}`) : record.kind}</strong>{#if superseded}<span>{t($locale, 'memoryCenter.superseded')}</span>{/if}</div>
                <p class="memory-content">{record.content}</p>
                {#if record.summary}<p>{record.summary}</p>{/if}
                <p class="muted">{t($locale, 'memoryCenter.source')}: {record.source} · {t($locale, 'memoryCenter.confidence')}: {new Intl.NumberFormat($locale, {style: 'percent', maximumFractionDigits: 0}).format(record.confidence)}</p>
                {#if record.tags.length}<p class="muted">{record.tags.join(' · ')}</p>{/if}
                <button disabled={loading || removing || editing} onclick={event => edit(record, event.currentTarget)}>{t($locale, 'memoryCenter.revise')}</button>
                <button class="danger" disabled={loading || removing} onclick={event => remove(record.id, event.currentTarget)}>{t($locale, 'memoryCenter.delete')}</button>
            </article>
        {:else}<p role="status">{t($locale, 'memoryCenter.empty')}</p>{/each}
    {/if}
</section>
<style>
    .memory-actions { justify-content: flex-end; }
    section, article, label { display: grid; gap: 12px; min-width: 0; }
    input { width: 100%; min-width: 0; box-sizing: border-box; }
    section { overflow-wrap: anywhere; }
    article { border-top: 1px solid var(--border); padding-top: 16px; overflow-wrap: anywhere; }
    .memory-content { white-space: pre-wrap; user-select: text; } p { margin: 0; }
    .superseded .memory-content { color: var(--muted); }
</style>
