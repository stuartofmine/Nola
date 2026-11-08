<script lang="ts">
    import { workspacePageApi } from '../lib/workspacePageLocation';
    const api = workspacePageApi(unscopedApi);
    import { workspaceUnloadScope } from '../lib/workspacePageLocation';
    const acceptsWorkspaceUnload = workspaceUnloadScope();
    import { onMount } from 'svelte';
    import { api as unscopedApi } from '../lib/api';
    import { locale, t } from '../lib/i18n';
    import { memoryDefault, memoryDefaultReceipt, type MemoryDefault } from '../lib/memoryDefault';
    import ConfirmationDialog from './ConfirmationDialog.svelte';
    let confirmation: ConfirmationDialog;
    let snapshot = $state<MemoryDefault>();
    let draft = $state(false), loading = $state(false), saving = $state(false), uncertain = $state(false);
    let error = $state('');
    let disposed = false;
    const controller = new AbortController();
    async function reload() {
        if (loading || saving) return;
        loading = true;
        if (uncertain && !await confirmation.ask({ title: t($locale, 'agentAccess.reloadTitle'), description: t($locale, 'agentAccess.reloadBody'), confirmLabel: t($locale, 'agentAccess.reloadConfirm'), destructive: true })) { loading = false; return; }
        if (disposed) return;
        error = '';
        try {
            const next = memoryDefault(await api('/api/assistant/memory-default', { signal: controller.signal }));
            if (!disposed) { snapshot = next; draft = next.enabled; uncertain = false; }
        } catch { if (!disposed) error = 'agentAccess.loadFailed'; }
        finally { if (!disposed) loading = false; }
    }
    async function change(event: Event) {
        if (!snapshot || loading || saving || uncertain) return;
        const enabled = (event.currentTarget as HTMLInputElement).checked;
        if (enabled && !snapshot.hostAllowed) return;
        draft = enabled; saving = true; error = '';
        try {
            const next = memoryDefaultReceipt(await api('/api/assistant/memory-default', { method: 'PUT', body: JSON.stringify({ enabled }) }), enabled);
            if (!disposed) { snapshot = next; draft = next.enabled; }
        } catch { if (!disposed) { uncertain = true; error = 'agentAccess.saveFailed'; } }
        finally { if (!disposed) saving = false; }
    }
    onMount(() => { void reload(); return () => { disposed = true; controller.abort(); }; });
</script>
<svelte:window onbeforeunload={event => { if (!acceptsWorkspaceUnload(event)) return; if (saving || uncertain) { event.preventDefault(); event.returnValue = ''; } }} />
<ConfirmationDialog bind:this={confirmation} />
<section class="card" aria-labelledby="memory-default-title" aria-busy={loading || saving}>
    <header><h2 id="memory-default-title">{t($locale, 'settings.memory.title')}</h2><button disabled={loading || saving} onclick={reload}>{t($locale, 'agentAccess.reload')}</button></header>
    <p class="muted">{t($locale, 'settings.memory.scope')}</p>
    {#if loading}<p role="status">{t($locale, 'chatPage.loading')}</p>{/if}
    {#if error}<p role="alert">{t($locale, error)}</p>{/if}
    {#if snapshot}
        <label><input type="checkbox" checked={draft} onchange={change} disabled={loading || saving || uncertain || (!snapshot.hostAllowed && !draft)} />{t($locale, 'settings.memory.enabled')}</label>
        {#if !snapshot.hostAllowed}<p role="status">{t($locale, 'settings.memory.denied')}</p>{/if}
        {#if saving}<p role="status">{t($locale, 'agentAccess.saving')}</p>{/if}
    {/if}
</section>
<style>
    section{display:grid;gap:14px;margin-bottom:20px}header{display:flex;align-items:center;justify-content:space-between;gap:12px;flex-wrap:wrap}h2,p{margin:0}label{display:flex;align-items:center;gap:8px}input{width:auto;flex-shrink:0}
</style>
