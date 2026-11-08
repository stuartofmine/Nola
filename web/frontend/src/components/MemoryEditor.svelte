<script lang="ts">
    import { workspacePageApi } from '../lib/workspacePageLocation';
    const api = workspacePageApi(unscopedApi);
    import { workspaceDialog } from '../lib/workspacePageLocation';
    const ownDialog = workspaceDialog();
    import { workspaceUnloadScope } from '../lib/workspacePageLocation';
    const acceptsWorkspaceUnload = workspaceUnloadScope();
    import {onMount, tick, untrack} from 'svelte';
    import {api as unscopedApi} from '../lib/api';
    import {locale, t} from '../lib/i18n';
    import {memoryDraft, memoryWriteReceipt} from '../lib/memoryCenter';
    import type {MemoryRecord} from '../lib/memoryCenter';
    import ConfirmationDialog from './ConfirmationDialog.svelte';
    let {record, onclose, onrefresh}: {record?: MemoryRecord; onclose: () => void; onrefresh: () => Promise<void>} = $props();
    let draft = $state(untrack(() => memoryDraft(record))), tags = $state(untrack(() => record?.tags.join(', ') ?? ''));
    const baseline = untrack(() => JSON.stringify({draft: memoryDraft(record), tags: record?.tags.join(', ') ?? ''}));
    let saving = $state(false), uncertain = $state(false);
    let dialog: HTMLDialogElement, confirmation: ConfirmationDialog;
    let disposed = false;
    const dirty = $derived(JSON.stringify({draft, tags}) !== baseline);
    onMount(() => {dialog.showModal(); return () => {disposed = true;};});
    async function close(event?: Event) {
        event?.preventDefault(); if (saving) return;
        if ((dirty || uncertain) && !await confirmation.ask({title: t($locale, 'memoryCenter.discard'), description: t($locale, 'memoryCenter.discardBody'), confirmLabel: t($locale, 'data_sources.discard.confirm'), destructive: true})) return;
        if (!disposed) {dialog.close(); onclose();}
    }
    async function save() {
        if (saving || uncertain || !draft.content.trim()) return;
        const payload = {...draft, tags: tags.split(',').map(tag => tag.trim()).filter(Boolean)};
        saving = true;
        try {
            const response = await api('/api/memory', {method: 'POST', body: JSON.stringify(record ? {operation: 'revise', id: record.id, draft: payload} : {operation: 'create', draft: payload})});
            if (disposed) return;
            memoryWriteReceipt(response, payload, record?.id);
            await onrefresh();
            if (!disposed) {dialog.close(); onclose();}
        } catch {
            if (!disposed) {uncertain = true; await onrefresh(); await tick();}
        } finally {if (!disposed) saving = false;}
    }
</script>
<svelte:window onbeforeunload={event => { if (!acceptsWorkspaceUnload(event)) return;if (dirty || saving || uncertain) {event.preventDefault(); event.returnValue = '';}}} />
<ConfirmationDialog bind:this={confirmation} />
<dialog use:ownDialog bind:this={dialog} oncancel={close} aria-labelledby="memory-editor-title">
    <form onsubmit={event => {event.preventDefault(); void save();}} aria-busy={saving}>
        <h2 id="memory-editor-title">{t($locale, record ? 'memoryCenter.revise' : 'memoryCenter.create')}</h2>
        {#if uncertain}<p role="alert">{t($locale, 'memoryCenter.uncertain')}</p>{/if}
        <fieldset disabled={saving || uncertain}>
            <label>{t($locale, 'memoryCenter.content')}<textarea bind:value={draft.content} rows="6" required maxlength="100000"></textarea></label>
            <label>{t($locale, 'memoryCenter.summary')}<textarea bind:value={draft.summary} rows="2"></textarea></label>
            <label>{t($locale, 'memoryCenter.scope')}<select bind:value={draft.scope}>{#if !['global', 'workspace', 'conversation', 'turn'].includes(draft.scope)}<option value={draft.scope}>{draft.scope}</option>{/if}{#each ['global', 'workspace', 'conversation', 'turn'] as value}<option {value}>{t($locale, `memoryCenter.scope.${value}`)}</option>{/each}</select></label>
            <label>{t($locale, 'memoryCenter.kind')}<select bind:value={draft.kind}>{#each ['preference', 'identity', 'decision', 'project', 'feedback', 'reference', 'other'] as value}<option {value}>{t($locale, `memoryCenter.kind.${value}`)}</option>{/each}</select></label>
            <label>{t($locale, 'memoryCenter.tags')}<input bind:value={tags} /></label>
            <label>{t($locale, 'memoryCenter.confidence')}<input type="number" min="0" max="1" step="any" bind:value={draft.confidence} required /></label>
        </fieldset>
        <div class="actions"><button type="button" disabled={saving} onclick={() => close()}>{t($locale, 'shell.cancel')}</button><button type="submit" disabled={saving || uncertain || !draft.content.trim()}>{t($locale, 'memoryCenter.save')}</button></div>
    </form>
</dialog>
<style>
    dialog { max-width: 640px; } form, fieldset, label { display: grid; gap: 12px; min-width: 0; }
    fieldset { padding: 0; border: 0; } textarea, input, select { width: 100%; min-width: 0; box-sizing: border-box; }
    textarea { padding: 10px; border: 1px solid var(--border); border-radius: 8px; background: var(--canvas); color: inherit; font: inherit; resize: vertical; }
    .actions { display: flex; flex-wrap: wrap; gap: 8px; justify-content: flex-end; } button { min-height: 44px; } [role="alert"] { color: var(--danger); }
</style>
