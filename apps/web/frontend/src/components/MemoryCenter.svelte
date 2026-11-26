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
    import RefreshButton from './RefreshButton.svelte';
    import ConfirmationDialog from './ConfirmationDialog.svelte';
    import MemoryEditor from './MemoryEditor.svelte';
    let editing = $state(false), editRecord = $state<MemoryRecord>();
    let editorOpener: HTMLButtonElement | undefined;
    function edit(record: MemoryRecord | undefined, opener: HTMLButtonElement) {
        if (loading || removing || promoting || editing) return;
        editRecord = record ? $state.snapshot(record) : undefined; editorOpener = opener; editing = true;
    }
    async function closeEditor() {editing = false; await tick(); if (disposed || !pageLocation.active()) return; if (editorOpener?.isConnected) editorOpener.focus(); else refreshButton?.focus();}
    let confirmation: ConfirmationDialog;
    let refreshButton = $state<HTMLButtonElement>();
    let removing = $state(false), deletionFailed = $state(false);
    let promoting = $state(false), promotionFailed = $state(false);
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
    async function promote(record: MemoryRecord) {
        if (promoting || removing || loading || editing || record.scope === 'global') return;
        promoting = true; promotionFailed = false;
        try {
            await api('/api/memory', {method: 'POST', body: JSON.stringify({operation: 'promote', id: record.id})});
            if (!disposed) await load();
        } catch {
            if (!disposed) promotionFailed = true;
        } finally {
            if (!disposed) promoting = false;
        }
    }
    async function remove(id: string, opener: HTMLButtonElement) {
        if (removing || promoting || loading || !records.some(record => record.id === id)) return;
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
<svelte:window onbeforeunload={event => { if (!acceptsWorkspaceUnload(event)) return;if (removing || promoting) {event.preventDefault(); event.returnValue = '';}}} />
<ConfirmationDialog bind:this={confirmation} />
{#if editing}<MemoryEditor record={editRecord} onclose={closeEditor} onrefresh={load} />{/if}
<section class="memory-center card" aria-busy={loading || promoting}>
    <header class="memory-header">
        <div class="memory-heading">
            <h2>{t($locale, 'memoryCenter.title')}</h2>
            <p class="muted">{t($locale, 'memoryCenter.hint')}</p>
        </div>
        <div class="memory-actions">
            <button type="button" class="primary" disabled={loading || removing || promoting || editing || !loaded} onclick={event => edit(undefined, event.currentTarget)}>{t($locale, 'memoryCenter.create')}</button>
            <RefreshButton bind:element={refreshButton} disabled={removing || promoting} {loading} refresh={load} />
        </div>
    </header>
    <label>{t($locale, 'memoryCenter.search')}<input type="search" bind:value={query} /></label>
    {#if loading && !loaded}<p role="status">{t($locale, 'memoryCenter.loading')}</p>{/if}
    {#if failed}<p role="alert" class="error">{t($locale, 'memoryCenter.failed')}{#if loaded} {t($locale, 'data_sources.stale')}{/if}</p>{/if}
    {#if deletionFailed}<p role="alert" class="error">{t($locale, 'memoryCenter.deleteFailed')}</p>{/if}
    {#if promotionFailed}<p role="alert" class="error">{t($locale, 'memoryCenter.promoteFailed')}</p>{/if}
    {#if loaded}
        <p class="muted">{t($locale, 'memoryCenter.limit')}</p>
        {#each visible as {record, superseded} (record.id)}
            <article class:superseded><div class="row between"><strong>{['global', 'workspace', 'conversation', 'turn'].includes(record.scope) ? t($locale, `memoryCenter.scope.${record.scope}`) : record.scope} · {['preference', 'identity', 'decision', 'project', 'feedback', 'reference', 'other'].includes(record.kind) ? t($locale, `memoryCenter.kind.${record.kind}`) : record.kind}</strong>{#if superseded}<span>{t($locale, 'memoryCenter.superseded')}</span>{/if}</div>
                <p class="memory-content">{record.content}</p>
                {#if record.summary}<p>{record.summary}</p>{/if}
                <p class="muted">{t($locale, 'memoryCenter.source')}: {record.source} · {t($locale, 'memoryCenter.confidence')}: {new Intl.NumberFormat($locale, {style: 'percent', maximumFractionDigits: 0}).format(record.confidence)}</p>
                {#if record.tags.length}<p class="muted">{record.tags.join(' · ')}</p>{/if}
                <div class="actions">
                    {#if record.scope !== 'global'}
                        <button type="button" disabled={loading || removing || promoting || editing} onclick={() => void promote(record)}>{t($locale, 'memoryCenter.promote')}</button>
                    {/if}
                    <button type="button" disabled={loading || removing || promoting || editing} onclick={event => edit(record, event.currentTarget)}>{t($locale, 'memoryCenter.revise')}</button>
                    <button type="button" class="danger" disabled={loading || removing || promoting} onclick={event => remove(record.id, event.currentTarget)}>{t($locale, 'memoryCenter.delete')}</button>
                </div>
            </article>
        {:else}<p role="status">{t($locale, 'memoryCenter.empty')}</p>{/each}
    {/if}
</section>
<style>
    .memory-header { display: flex; flex-wrap: wrap; align-items: flex-start; justify-content: space-between; gap: 12px 16px; }
    .memory-heading { display: grid; gap: 4px; flex: 1 1 240px; min-width: 0; }
    .memory-heading h2 { margin: 0; font-size: 1.0625rem; }
    .memory-actions { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; flex-shrink: 0; }
    section, article, label { display: grid; gap: 12px; min-width: 0; }
    input { width: 100%; min-width: 0; box-sizing: border-box; }
    section { overflow-wrap: anywhere; }
    article { border-top: 1px solid var(--border); padding-top: 16px; overflow-wrap: anywhere; }
    .memory-content { white-space: pre-wrap; user-select: text; } p { margin: 0; }
    .superseded .memory-content { color: var(--muted); }
</style>
