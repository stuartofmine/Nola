<script lang="ts">
    import { workspacePageApi } from '../lib/workspacePageLocation';
    const api = workspacePageApi(unscopedApi);
    import { workspaceUnloadScope } from '../lib/workspacePageLocation';
    const acceptsWorkspaceUnload = workspaceUnloadScope();
    import { onMount } from 'svelte';
    import { api as unscopedApi } from '../lib/api';
    import { locale, t } from '../lib/i18n';
    import { retrievalDefault, retrievalDefaultReceipt, type RetrievalDefault } from '../lib/retrievalDefault';
    import ConfirmationDialog from './ConfirmationDialog.svelte';
    let confirmation: ConfirmationDialog;
    let snapshot = $state<RetrievalDefault>();
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
            const next = retrievalDefault(await api('/api/assistant/retrieval-default', { signal: controller.signal }));
            if (!disposed) { snapshot = next; draft = next.enabled; uncertain = false; }
        } catch { if (!disposed) error = 'agentAccess.loadFailed'; }
        finally { if (!disposed) loading = false; }
    }
    async function change(event: Event) {
        if (!snapshot || loading || saving || uncertain) return;
        const enabled = (event.currentTarget as HTMLInputElement).checked;
        draft = enabled; saving = true; error = '';
        try {
            const next = retrievalDefaultReceipt(await api('/api/assistant/retrieval-default', { method: 'PUT', body: JSON.stringify({ enabled }) }), enabled);
            if (!disposed) { snapshot = next; draft = next.enabled; }
        } catch { if (!disposed) { uncertain = true; error = 'agentAccess.saveFailed'; } }
        finally { if (!disposed) saving = false; }
    }
    onMount(() => { void reload(); return () => { disposed = true; controller.abort(); }; });
</script>
<svelte:window onbeforeunload={event => { if (!acceptsWorkspaceUnload(event)) return; if (saving || uncertain) { event.preventDefault(); event.returnValue = ''; } }} />
<ConfirmationDialog bind:this={confirmation} />
<section class="card" aria-labelledby="retrieval-default-title" aria-busy={loading || saving}>
    <header><h2 id="retrieval-default-title">{t($locale, 'chatSession.retrieval')}</h2><button disabled={loading || saving} onclick={reload}>{t($locale, 'agentAccess.reload')}</button></header>
    <p class="muted">{t($locale, 'settings.retrieval.scope')}</p>
    <p class="muted">{t($locale, 'chatSession.retrievalScope')}</p>
    {#if loading}<p role="status">{t($locale, 'chatPage.loading')}</p>{/if}
    {#if error}<p role="alert">{t($locale, error)}</p>{/if}
    {#if snapshot}
        <label><input type="checkbox" checked={draft} onchange={change} disabled={loading || saving || uncertain} />{t($locale, 'settings.retrieval.enabled')}</label>
        {#if saving}<p role="status">{t($locale, 'agentAccess.saving')}</p>{/if}
    {/if}
</section>
<style>
    section{display:grid;gap:14px;margin-bottom:20px}header{display:flex;align-items:center;justify-content:space-between;gap:12px;flex-wrap:wrap}h2,p{margin:0}label{display:flex;align-items:center;gap:8px}input{width:auto;flex-shrink:0}
</style>
