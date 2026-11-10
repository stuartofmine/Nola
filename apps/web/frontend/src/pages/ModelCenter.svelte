<script lang="ts">
    import { untrack } from 'svelte';
    import { workspacePageApi } from '../lib/workspacePageLocation';
    const api = workspacePageApi(unscopedApi);
    import { workspaceElements } from '../lib/workspacePageLocation';
    const pageElements = workspaceElements();
    import { workspaceDialog } from '../lib/workspacePageLocation';
    const ownDialog = workspaceDialog();
    import { workspaceUnloadScope } from '../lib/workspacePageLocation';
    const acceptsWorkspaceUnload = workspaceUnloadScope();
    import { workspacePageLocation } from '../lib/workspacePageLocation';
    const pageLocation = workspacePageLocation();
    $effect(() => { if (pageLocation.active()) untrack(() => { void resume(); }); });
    import { onMount, tick } from 'svelte';
    import { locale, t } from '../lib/i18n';
    import { api as unscopedApi, errorMessage } from '../lib/api';
    import { ModelValidationError, modelValidationText, type ModelValidationIssue } from '../lib/modelValidation';
    import ApiModels from './ApiModels.svelte';
    import ConfirmationDialog from '../components/ConfirmationDialog.svelte';
    import type { ConfirmationHandle } from '../lib/confirmation';
    import { latestRequest } from '../lib/latestRequest';
    import { localModelEditorSnapshot } from '../lib/localModelEditor';
    let confirmation: ConfirmationHandle;

    type Parameters = { contextTokens: number; temperature: number; topP: number; topK: number; minP: number; batchSize: number; threads: number; gpuLayers: number; flashAttention: boolean };
    type Model = { id: string; name?: string; displayName?: string; subtitle?: string; description?: string; tier?: string; quantization?: string; downloadBytes?: number; estimatedRuntimeBytes?: number; recommendedMemoryBytes?: number; contextWindowTokens?: number; installed: boolean; running: boolean; selected: boolean; deviceCompatible: boolean; configurationRevision: number; parameters: Parameters; download?: { status: string; downloadedBytes: number; totalBytes: number; error?: string } };
    type Catalog = { binaryAvailable: boolean; installHint?: string; physicalMemoryBytes?: number; recommendedModelId?: string; diskUsageBytes?: number; configurationError?: string; models: Model[] };
    let tab = $state<'local' | 'api'>('api');
    let linkedModelId = $state('');
    let catalog = $state<Catalog>();
    let error = $state('');
    let editorError = $state<string | ModelValidationIssue>('');
    let busy = $state('');
    let editing = $state<Model>();
    let draft = $state<Parameters>();
    let dialog: HTMLDialogElement;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let disposed = false;
    const catalogReads = latestRequest<Catalog>();
    const dirty = $derived(!!editing && JSON.stringify(draft) !== JSON.stringify(editing.parameters));
    const recommendedModel = $derived(catalog?.models.find(model => model.id === catalog?.recommendedModelId));
    const runningModel = $derived(catalog?.models.find(model => model.running));
    function formatBytes(value?: number) {
        if (value == null || !Number.isFinite(value) || value < 0) return t($locale, 'local.unavailable');
        return `${new Intl.NumberFormat($locale, { maximumFractionDigits: 1 }).format(value / 1_000_000_000)} GB`;
    }
    function formatContext(value?: number) {
        return value && Number.isFinite(value) ? `${Math.round(value / 1024)}K` : t($locale, 'local.unavailable');
    }
    const numericFields: { key: Exclude<keyof Parameters, 'flashAttention'>; label: string; min: number; max: number; step: number }[] = [
        { key: 'contextTokens', label: 'models.context', min: 65536, max: 4000000, step: 1 },
        { key: 'temperature', label: 'models.temperature', min: 0, max: 2, step: 0.05 },
        { key: 'topP', label: 'modelDisplay.topP', min: 0, max: 1, step: 0.01 },
        { key: 'topK', label: 'modelDisplay.topK', min: 1, max: 200, step: 1 },
        { key: 'minP', label: 'modelDisplay.minP', min: 0, max: 1, step: 0.01 },
        { key: 'batchSize', label: 'models.batch', min: 32, max: 4096, step: 32 },
        { key: 'threads', label: 'models.threads', min: 0, max: 256, step: 1 },
        { key: 'gpuLayers', label: 'models.gpu', min: 0, max: 999, step: 1 },
    ];
    async function refresh() {
        if (disposed || !pageLocation.active()) return;
        clearTimeout(timer);
        try {
            const result = await catalogReads.run(signal => api<Catalog>('/api/local-models', { signal }));
            if (disposed || !result) return;
            catalog = result;
            error = '';
            if (tab === 'local' && linkedModelId) {
                const linked = result.models.find(model => model.id === linkedModelId && model.installed);
                if (linked) {
                    linkedModelId = '';
                    configure(linked);
                }
            }
            if (pageLocation.active() && !document.hidden && tab === 'local' && result.models.some(model => model.download?.status === 'downloading')) timer = setTimeout(refresh, 1000);
            return result;
        } catch (cause) { if (!disposed) error = errorMessage(cause); }
    }
    function resume() { if (!disposed && pageLocation.active() && !document.hidden && tab === 'local' && !busy) void refresh(); }
    function selectTab(value: 'local' | 'api') {
        tab = value;
        clearTimeout(timer);
        if (tab === 'api' && !busy) catalogReads.cancel();
        else resume();
    }
    function visibilityChanged() {
        if (document.hidden) { clearTimeout(timer); if (!busy) catalogReads.cancel(); }
        else resume();
    }
    onMount(() => {
        const parameters = pageLocation.read().searchParams;
        tab = parameters.get('tab') === 'local' || parameters.has('configure') ? 'local' : 'api';
        linkedModelId = parameters.get('configure') || '';
        void refresh();
        return () => { disposed = true; clearTimeout(timer); catalogReads.cancel(); };
    });
    async function action(model: Model, name: string) {
        if (busy) return;
        if (name === 'installation' && !await confirmation.ask({ title: t($locale, 'models.deleteTitle'), description: t($locale, 'models.deleteBody', { name: model.displayName || model.name || model.id }), confirmLabel: t($locale, 'models.deleteConfirm'), destructive: true })) return;
        busy = model.id; error = '';
        catalogReads.cancel(); clearTimeout(timer);
        try {
            await api(`/api/local-models/${encodeURIComponent(model.id)}/${name}`, { method: name === 'installation' ? 'DELETE' : 'POST' });
            await refresh();
        } catch (cause) { error = errorMessage(cause); }
        finally { busy = ''; }
    }
    async function configure(model: Model) {
        editing = model; draft = { ...model.parameters }; editorError = '';
        // Let the form mount before native dialog focus chooses its first field.
        await tick();
        if (!disposed && editing === model && !dialog.open) dialog.showModal();
    }
    async function reloadEditor() {
        if (busy || !editing) return;
        if (dirty && !await confirmation.ask({ title: t($locale, 'models.reloadTitle'), description: t($locale, 'models.reloadBody'), confirmLabel: t($locale, 'models.reloadConfirm'), cancelLabel: t($locale, 'models.continueEditing'), destructive: true })) return;
        const id = editing.id;
        busy = id; editorError = '';
        try {
            const snapshot = await refresh();
            if (!snapshot) { editorError = error || t($locale, 'models.reloadFailed'); return; }
            const current = localModelEditorSnapshot(snapshot.models, id);
            editing = current.model;
            draft = current.parameters;
        } catch (cause) { editorError = cause instanceof ModelValidationError ? cause.issue : errorMessage(cause); }
        finally { busy = ''; }
    }
    async function closeEditor(event?: Event) {
        event?.preventDefault();
        if (busy) return;
        if (dirty && !await confirmation.ask({ title: t($locale, 'models.discardTitle'), description: t($locale, 'models.discardBody'), confirmLabel: t($locale, 'models.discard'), cancelLabel: t($locale, 'models.continueEditing'), destructive: true })) return;
        dialog.close(); editing = undefined; draft = undefined;
    }
    async function save(event: SubmitEvent) {
        event.preventDefault();
        if (busy || !editing || !draft || !Number.isSafeInteger(editing.configurationRevision)) return;
        busy = editing.id; editorError = '';
        catalogReads.cancel(); clearTimeout(timer);
        try {
            await api(`/api/local-models/${encodeURIComponent(editing.id)}/configuration`, {
                method: 'POST', body: JSON.stringify({ expectedRevision: editing.configurationRevision, parameters: draft }),
            });
            dialog.close(); editing = undefined; draft = undefined; await refresh();
        } catch (cause) { editorError = cause instanceof ModelValidationError ? cause.issue : errorMessage(cause); }
        finally { busy = ''; }
    }
    function tabKey(event: KeyboardEvent) {
        if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
        event.preventDefault();
        selectTab(event.key === 'Home' ? 'api' : event.key === 'End' ? 'local' : tab === 'local' ? 'api' : 'local');
        pageElements.element(`tab-${tab}`)?.focus();
    }
    function beforeUnload(event: BeforeUnloadEvent) {
        if (!acceptsWorkspaceUnload(event)) return;
        if (editing && JSON.stringify(draft) !== JSON.stringify(editing.parameters)) {
            event.preventDefault(); event.returnValue = '';
        }
    }
</script>

<svelte:window onbeforeunload={beforeUnload} onfocus={resume} />
<svelte:document onvisibilitychange={visibilityChanged} />
<ConfirmationDialog bind:this={confirmation} />
<svelte:head>{#if pageLocation.active()}<title>{t($locale, 'models.title')} — Nola</title>{/if}</svelte:head>
<header class="page-header models-header"><div><h1>{t($locale, 'models.title')}</h1><p class="muted">{t($locale, 'models.subtitle')}</p></div><a class="button" href="/model-test">{t($locale, 'modelTest.title')}…</a></header>
<div class="content models-content">
    <div class="tabs" role="tablist" aria-label={t($locale, 'models.type')}>
        {#each ['api', 'local'] as value}
            <button id="tab-{value}" role="tab" aria-controls="panel-{value}" aria-selected={tab === value} tabindex={tab === value ? 0 : -1} onclick={() => selectTab(value as 'local' | 'api')} onkeydown={tabKey}>{t($locale, value === 'local' ? 'models.local' : 'models.api')}</button>
        {/each}
    </div>
    <div id="panel-local" role="tabpanel" tabindex="0" aria-labelledby="tab-local" hidden={tab !== 'local'}>
        {#if error}<div class="notice error" role="alert">{error}{catalog ? t($locale, 'models.stale') : ''} <button onclick={refresh} disabled={!!busy}>{t($locale, 'models.retry')}</button></div>{/if}
        {#if !catalog}{#if !error}<p role="status">{t($locale, 'models.loading')}</p>{/if}
        {:else}
            {#if catalog.configurationError}<p class="notice error" role="alert">{t($locale, 'local.configuration_error', { detail: catalog.configurationError })}</p>{/if}
            {#if recommendedModel}<section class="card summary-card" aria-label={t($locale, 'local.recommended')}>
                <div><h2>{t($locale, 'local.recommended')} <span class="accent">{recommendedModel.displayName || recommendedModel.name || recommendedModel.id}</span></h2>
                    <p class="summary-details"><span>{t($locale, 'local.memory')} · {formatBytes(catalog.physicalMemoryBytes)}</span><span>{t($locale, 'local.context')} · {formatContext(recommendedModel.contextWindowTokens)}</span><span>{t($locale, 'local.runtime')} · {formatBytes(recommendedModel.estimatedRuntimeBytes)}</span></p>
                </div>
            </section>{/if}
            <section class="card runtime-card" aria-label={t($locale, 'local.engine_status')}>
                <div><h2>{t($locale, 'local.engine_status')}</h2><p class="muted">{runningModel ? t($locale, 'local.running_model', { name: runningModel.displayName || runningModel.name || runningModel.id }) : catalog.binaryAvailable ? t($locale, 'models.engineReady') : t($locale, 'models.engineMissing')}</p></div>
            {#if !catalog.binaryAvailable && catalog.installHint}<details><summary>{t($locale, 'models.configure')}</summary><p>{catalog.installHint}</p></details>{/if}</section>
            <div class="models">
                {#each catalog.models as model (model.id)}
                    <article class="model-row" aria-busy={busy === model.id}>
                        <div class="row between"><h2>{model.displayName || model.name || model.id}</h2><span class="muted">{t($locale, model.running ? 'models.running' : model.selected ? 'models.selected' : model.installed ? 'models.installed' : 'models.notDownloaded')}</span></div>
                        {#if model.description || model.subtitle}<p class="muted">{model.description || model.subtitle}</p>{/if}
                        <p class="model-details muted"><span>{model.quantization || 'GGUF'}</span><span>{t($locale, 'local.context')} · {formatContext(model.contextWindowTokens)}</span><span>{t($locale, 'local.runtime')} · {formatBytes(model.estimatedRuntimeBytes)}</span><span>{t($locale, 'local.required_memory')} · {formatBytes(model.recommendedMemoryBytes)}</span><span>{t($locale, 'local.download_size')} · {formatBytes(model.downloadBytes)}</span></p>
                        {#if model.download?.status === 'downloading'}
                            <progress aria-label={t($locale, 'models.downloadProgress')} value={model.download.downloadedBytes} max={model.download.totalBytes || 1}></progress>
                        {/if}
                        {#if model.download?.error}<p class="error">{model.download.error}</p>{/if}
                        <div class="actions">
                            {#if model.installed}
                                <button onclick={() => configure(model)} disabled={!!busy}>{t($locale, 'models.configure')}</button>
                                <button onclick={() => action(model, 'select')} disabled={!!busy || model.selected}>{t($locale, 'models.select')}</button>
                                <button class="primary" onclick={() => action(model, model.running ? 'stop' : 'start')} disabled={!!busy || !catalog.binaryAvailable}>{t($locale, model.running ? 'models.stop' : 'models.start')}</button>
                                <button class="danger" onclick={() => action(model, 'installation')} disabled={!!busy || model.running}>{t($locale, 'models.delete')}</button>
                            {:else}<button onclick={() => action(model, 'download')} disabled={!!busy || model.deviceCompatible === false || model.download?.status === 'downloading'}>{t($locale, 'models.download')}</button>{/if}
                        </div>
                    </article>
                {:else}<div class="card"><h2>{t($locale, 'models.empty')}</h2><p class="muted">{t($locale, 'models.emptyHint')}</p></div>{/each}
            </div>
            <section class="card storage-card"><h2>{t($locale, 'local.storage')}</h2><p class="muted">{formatBytes(catalog.diskUsageBytes)}</p></section>
        {/if}
    </div>
    <div id="panel-api" role="tabpanel" tabindex="0" aria-labelledby="tab-api" hidden={tab !== 'api'}><ApiModels openProfileId={tab === 'api' ? linkedModelId : ''} /></div>
</div>
<dialog use:ownDialog bind:this={dialog} oncancel={closeEditor} aria-label={t($locale, 'models.configureTitle', { name: editing?.displayName || editing?.name || editing?.id || '' })}>
    {#if draft && editing}
        <form onsubmit={save} aria-busy={!!busy}>
            <div><h2>{editing.displayName || editing.name || editing.id}</h2><p class="muted">{t($locale, 'models.nextStart')}</p></div>
            {#if editorError}<div class="notice error" role="alert">{modelValidationText($locale, editorError)} <button type="button" onclick={reloadEditor} disabled={!!busy}>{t($locale, 'models.reload')}</button></div>{/if}
            <div class="fields">
                {#each numericFields as field}
                    <label>{t($locale, field.label)}<input type="number" bind:value={draft[field.key]} min={field.min} max={field.max} step={field.step < 1 ? 'any' : field.step} disabled={!!busy} required /></label>
                {/each}
            </div>
            <label class="row"><input class="check" type="checkbox" bind:checked={draft.flashAttention} disabled={!!busy} />{t($locale, 'modelDisplay.flashAttention')}</label>
            <div class="actions"><button type="button" onclick={() => closeEditor()} disabled={!!busy}>{t($locale, 'models.cancel')}</button><button class="primary" disabled={!!busy || !dirty}>{t($locale, busy ? 'models.saving' : 'models.save')}</button></div>
        </form>
    {/if}
</dialog>
<style>
    .models-header { max-width: 1120px; margin: auto; border-bottom: 0; padding: 40px 32px 24px; align-items: start; }
    .models-header h1 { font-size: 1.75rem; letter-spacing: -.025em; }
    .models-header p { margin-top: 8px; }
    .models-header > a { background: transparent; border-color: transparent; }
    .models-content { max-width: 1120px; padding: 0 32px 48px; gap: 24px; }
    .tabs { background: transparent; padding: 0; gap: 8px; border-radius: 0; }
    .tabs button { padding-inline: 20px; }
    .models { display: grid; margin-top: 24px; border-bottom: 1px solid var(--border); }
    .model-row { display: grid; grid-template-columns: minmax(0, 1fr) auto; column-gap: 24px; row-gap: 8px; padding: 24px 0; border-top: 1px solid var(--border); min-width: 0; }
    .model-row > :not(.actions) { grid-column: 1; min-width: 0; overflow-wrap: anywhere; }
    .model-row > .actions { grid-column: 2; grid-row: 1 / span 5; align-content: center; align-items: center; max-width: 320px; }
    .model-row h2 { font-size: 1rem; }
    .model-row .actions button:not(.primary) { border-color: transparent; background: transparent; }
    .summary-card, .runtime-card, .storage-card { background: transparent; border: 0; border-radius: 0; padding: 16px 0; gap: 8px; }
    .summary-card, .runtime-card { border-bottom: 1px solid var(--border); }
    .summary-card h2, .runtime-card h2 { font-size: .9375rem; margin: 0 0 6px; }
    .accent { color: var(--content); font-weight: 600; }
    .summary-details, .model-details { display: flex; flex-wrap: wrap; gap: 4px 16px; margin: 0; font-size: .8125rem; }
    .storage-card { display: flex; flex-wrap: wrap; justify-content: space-between; align-items: baseline; margin-top: 12px; }
    .storage-card h2 { font-size: .875rem; }
    .storage-card p { overflow-wrap: anywhere; }
    .check { width: auto; }
    dialog { max-width: 720px; padding: 32px; }
    dialog form > div:first-child h2 { font-size: 1.5rem; overflow-wrap: anywhere; }
    dialog .actions { border-top: 1px solid var(--border); padding-top: 20px; }
    @media (max-width: 900px) {
        .model-row { grid-template-columns: minmax(0, 1fr); }
        .model-row > .actions { grid-column: 1; grid-row: auto; justify-content: flex-start; max-width: none; margin-top: 4px; }
    }
    @media (max-width: 760px) {
        .models-header { padding: 24px 16px 20px; gap: 12px; }
        .models-header h1 { font-size: 1.5rem; }
        .models-content { padding: 0 16px 32px; }
        .model-row { padding-block: 20px; }
        dialog { padding: 20px 16px; }
    }
</style>
