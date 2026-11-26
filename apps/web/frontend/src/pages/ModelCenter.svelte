<script lang="ts">
    import { untrack, setContext } from 'svelte';
    setContext('nola.modelCenterToolbar', true);
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
    import LocalEngineSetup from '../components/LocalEngineSetup.svelte';
    import LocalModelSearch from '../components/LocalModelSearch.svelte';
    import {
        downloadActive, engineName, modelByteSize, modelMemorySize,
        defaultModelSource, friendlyRuntimeLabel, formatUnifiedGb, cleanModelName,
        type EngineStatus, type LocalEngine, type ModelDownload, type ModelSource
    } from '../lib/modelAcquisition';
    import { localModelEditorSnapshot } from '../lib/localModelEditor';
    let confirmation: ConfirmationHandle;

    type Parameters = { contextTokens: number; temperature: number; topP: number; topK: number; minP: number; batchSize: number; threads: number; gpuLayers: number; flashAttention: boolean };
    type Model = { id: string; runtime?: string; format?: string; minimumContextWindowTokens?: number; nativeContextTokens?: number; binaryAvailable?: boolean; installHint?: string; name?: string; displayName?: string; subtitle?: string; description?: string; tier?: string; quantization?: string; downloadBytes?: number; estimatedRuntimeBytes?: number; recommendedMemoryBytes?: number; contextWindowTokens?: number; installed: boolean; running: boolean; selected: boolean; deviceCompatible: boolean; configurationRevision: number; parameters: Parameters; download?: { status: string; downloadedBytes: number; totalBytes: number; error?: string } };
    type Catalog = { acquisition?: ModelDownload | null; runtimes?: { id: LocalEngine; binaryAvailable: boolean; installHint?: string }[]; binaryAvailable: boolean; installHint?: string; physicalMemoryBytes?: number; recommendedModelId?: string; diskUsageBytes?: number; configurationError?: string; models: Model[] };
    const initialParameters = untrack(() => pageLocation.read().searchParams);
    let tab = $state<'local' | 'api'>(initialParameters.get('tab') === 'local' ||
        (!initialParameters.has('tab') && initialParameters.has('configure')) ? 'local' : 'api');
    let localPageOpened = $state(false);
    let apiPageOpened = $state(false);
    $effect(() => {
        if (tab === 'local') localPageOpened = true;
        else apiPageOpened = true;
    });
    let engine = $state<LocalEngine>('llama.cpp');
    let source = $state<ModelSource>(defaultModelSource());
    let preferencesReady = $state(false);
    $effect(() => {
        if (preferencesReady) {
            try { localStorage.setItem('nola.localModel.engine', engine); localStorage.setItem('nola.localModel.source', source); }
            catch { /* Storage is optional in private browser contexts. */ }
        }
    });
    let linkedModelId = $state(initialParameters.get('configure') || '');
    let requestedEndpointName = $state('');
    let catalog = $state<Catalog>();
    let error = $state('');
    let actionError = $state<{name: string; message: string}>();
    let editorError = $state<string | ModelValidationIssue>('');
    let busy = $state('');
    let editing = $state<Model>();
    let draft = $state<Parameters>();
    let dialog: HTMLDialogElement;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let disposed = false;
    const catalogReads = latestRequest<Catalog>();
    const dirty = $derived(!!editing && JSON.stringify(draft) !== JSON.stringify(editing.parameters));
    function isModelDownloadActive(status?: string) {
        return status === 'queued' || status === 'downloading' || status === 'verifying' || status === 'cancelling';
    }
    const visibleModels = $derived(
        (catalog?.models.filter(model => model.installed || isModelDownloadActive(model.download?.status)) ?? [])
            .sort((a, b) => Number(b.running) - Number(a.running) || Number(b.selected) - Number(a.selected))
    );
    function runtimeAvailable(id: LocalEngine) {
        return catalog?.runtimes?.find(item => item.id === id)?.binaryAvailable
            ?? (id === 'llama.cpp' && catalog?.binaryAvailable);
    }
    async function showEngineSetup(id: LocalEngine) {
        engine = id;
        await tick();
        engineSetup?.reveal();
    }
    function hasActiveDownload(value?: Catalog) {
        return downloadActive(value?.acquisition) || value?.models.some(model => isModelDownloadActive(model.download?.status));
    }
    let engineSetup = $state<{reveal(): void}>();
    let readiness = $state<Partial<Record<LocalEngine, boolean>>>({});
    const alternative = $derived((['llama.cpp','vllm'] as LocalEngine[]).find(id => id !== engine &&
        (readiness[id] ?? catalog?.runtimes?.find(item => item.id === id)?.binaryAvailable)));
    function updateEngine(status: EngineStatus) {
        readiness = {...readiness, [status.engine]:status.ready};
    }
    const runningModel = $derived(visibleModels.find(model => model.running));
    function formatBytes(value?: number) {
        if (value == null || !Number.isFinite(value) || value < 0) return t($locale, 'local.unavailable');
        return modelByteSize(value, $locale);
    }
    function formatContext(value?: number) {
        if (!value || !Number.isFinite(value)) return t($locale, 'local.unavailable');
        return value >= 1024 && value % 1024 === 0 ? `${value / 1024}K` : new Intl.NumberFormat($locale).format(value);
    }
    const numericFields: { key: Exclude<keyof Parameters, 'flashAttention'>; label: string; min: number; max: number; step: number }[] = [
        { key: 'contextTokens', label: 'models.context', min: 2048, max: 4000000, step: 1 },
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
            if (pageLocation.active() && !document.hidden && tab === 'local' && hasActiveDownload(result)) timer = setTimeout(refresh, 1000);
            return result;
        } catch (cause) {
            if (!disposed) {
                error = errorMessage(cause);
                if (pageLocation.active() && !document.hidden && tab === 'local' && hasActiveDownload(catalog)) timer = setTimeout(refresh, 3000);
            }
        }
    }
    function trackDownload(next: ModelDownload) {
        if (catalog) catalog = { ...catalog, acquisition: next };
        void refresh();
    }
    function resume() { if (!disposed && pageLocation.active() && !document.hidden && tab === 'local' && !busy) void refresh(); }
    function selectTab(value: 'local' | 'api') {
        if (value === tab) return;
        linkedModelId = '';
        tab = value;
        if (value === 'local') requestedEndpointName = '';
        const url = pageLocation.read();
        url.searchParams.set('tab', value);
        url.searchParams.delete('configure');
        pageLocation.replace(url);
        clearTimeout(timer);
        if (tab === 'api' && !busy) catalogReads.cancel();
        else resume();
    }
    function visibilityChanged() {
        if (document.hidden) { clearTimeout(timer); if (!busy) catalogReads.cancel(); }
        else resume();
    }
    let desktopRuntime = $state('');
    onMount(() => {
        try {
            if (localStorage.getItem('nola.localModel.engine') === 'vllm') engine = 'vllm';
            const savedSource = localStorage.getItem('nola.localModel.source');
            if (savedSource === 'modelscope' || savedSource === 'huggingface') source = savedSource;
            else source = defaultModelSource($locale);
        } catch {
            source = defaultModelSource($locale);
        }
        preferencesReady = true;
        const desktop = (window as Window & { NolaDesktop?: { nativeRuntime?: () => Promise<{ backend: string; architecture: string }> } }).NolaDesktop;
        desktop?.nativeRuntime?.().then(runtime => {
            if (!disposed && (runtime.backend || runtime.architecture))
                desktopRuntime = friendlyRuntimeLabel(runtime.backend || '', runtime.architecture || '', $locale);
        }).catch(() => {});
        return () => { disposed = true; clearTimeout(timer); catalogReads.cancel(); };
    });
    async function changeModel(model: Model, change: () => Promise<void>) {
        if (busy || disposed) return;
        busy = model.id;
        actionError = undefined;
        catalogReads.cancel();
        clearTimeout(timer);
        try {
            await change();
        } catch (cause) {
            if (!disposed) actionError = { name: model.displayName || model.name || model.id, message: errorMessage(cause) };
        } finally {
            // Selection can succeed before startup fails. Re-read the durable state for either outcome.
            if (!disposed) {
                await refresh();
                window.dispatchEvent(new CustomEvent('nola:models-updated'));
                busy = '';
            }
        }
    }
    function switchAndUse(model: Model) {
        return changeModel(model, async () => {
            if (!model.selected) await api(`/api/local-models/${encodeURIComponent(model.id)}/select`, { method: 'POST' });
            if (!disposed && !model.running) await api(`/api/local-models/${encodeURIComponent(model.id)}/start`, { method: 'POST' });
        });
    }
    async function action(model: Model, name: string) {
        if (busy) return;
        if (name === 'installation' && !await confirmation.ask({ title: t($locale, 'models.deleteTitle'), description: t($locale, 'models.deleteBody', { name: model.displayName || model.name || model.id }), confirmLabel: t($locale, 'models.deleteConfirm'), destructive: true })) return;
        await changeModel(model, async () => {
            await api(`/api/local-models/${encodeURIComponent(model.id)}/${name}`, { method: name === 'installation' ? 'DELETE' : 'POST' });
        });
    }
    async function configure(model: Model) {
        engine = model.runtime === 'vllm' ? 'vllm' : 'llama.cpp';
        editing = model; draft = { ...model.parameters }; editorError = '';
        // Let the form mount before native dialog focus chooses its first field.
        await tick();
        if (!disposed && editing === model && !dialog.open) dialog.showModal();
    }
    async function reloadEditor() {
        if (busy || !editing) return;
        if (dirty && !await confirmation.ask({ title: t($locale, 'models.reloadTitle'), description: t($locale, 'models.reloadBody'), confirmLabel: t($locale, 'models.reloadConfirm'), cancelLabel: t($locale, 'models.continueEditing'), destructive: true })) return;
        if (disposed || !editing) return;
        const id = editing.id;
        busy = id; editorError = '';
        try {
            const snapshot = await refresh();
            if (disposed) return;
            if (!snapshot) { editorError = error || t($locale, 'models.reloadFailed'); return; }
            const current = localModelEditorSnapshot(snapshot.models, id);
            editing = current.model;
            draft = current.parameters;
        } catch (cause) { if (!disposed) editorError = cause instanceof ModelValidationError ? cause.issue : errorMessage(cause); }
        finally { if (!disposed) busy = ''; }
    }
    async function closeEditor(event?: Event) {
        event?.preventDefault();
        if (busy) return;
        if (dirty && !await confirmation.ask({ title: t($locale, 'models.discardTitle'), description: t($locale, 'models.discardBody'), confirmLabel: t($locale, 'models.discard'), cancelLabel: t($locale, 'models.continueEditing'), destructive: true })) return;
        if (disposed) return;
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
            if (disposed) return;
            dialog.close(); editing = undefined; draft = undefined; await refresh();
            window.dispatchEvent(new CustomEvent('nola:models-updated'));
        } catch (cause) { if (!disposed) editorError = cause instanceof ModelValidationError ? cause.issue : errorMessage(cause); }
        finally { if (!disposed) busy = ''; }
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
    <div class="models-toolbar">
        <div class="tabs" role="tablist" aria-label={t($locale, 'models.type')}>
            {#each ['api', 'local'] as value}
                <button type="button" id="tab-{value}" role="tab" aria-controls="panel-{value}" aria-selected={tab === value} tabindex={tab === value ? 0 : -1} onclick={() => selectTab(value as 'local' | 'api')} onkeydown={tabKey}>{t($locale, value === 'local' ? 'models.local' : 'models.api')}</button>
            {/each}
        </div>
        {#if tab === 'api'}
            <button type="button" class="primary" onclick={() => { requestedEndpointName = '__new__'; }}>{t($locale, 'remote.add_endpoint_model')}</button>
        {/if}
    </div>
    <div id="panel-local" role="tabpanel" aria-labelledby="tab-local" hidden={tab !== 'local'}>
        {#if error}<div class="notice error" role="alert">{error}{catalog ? t($locale, 'models.stale') : ''} <button type="button" onclick={refresh} disabled={!!busy}>{t($locale, 'models.retry')}</button></div>{/if}
        {#if actionError}<div class="notice error" role="alert"><strong>{actionError.name}</strong> {actionError.message}</div>{/if}
        {#if !catalog}{#if !error}<p role="status">{t($locale, 'models.loading')}</p>{/if}
        {:else}
            {#if catalog.configurationError}<p class="notice error" role="alert">{t($locale, 'local.configuration_error', { detail: catalog.configurationError })}</p>{/if}
            {#if visibleModels.length}
                <section class="installed-section" aria-label={t($locale, 'local.myModels')}>
                    <div class="section-heading">
                        <h2>{t($locale, 'local.myModels')}</h2>
                        {#if catalog.diskUsageBytes != null}<span class="muted">{formatBytes(catalog.diskUsageBytes)}</span>{/if}
                    </div>
                    <div class="models">
                        {#each visibleModels as model (model.id)}
                            {@const modelEngine = (model.runtime === 'vllm' ? 'vllm' : 'llama.cpp') as LocalEngine}
                            {@const modelTitle = model.displayName || cleanModelName(model.name || model.id)}
                            {@const canRun = model.running || (readiness[modelEngine] ?? model.binaryAvailable ?? runtimeAvailable(modelEngine))}
                            <article class="model-row" aria-busy={busy === model.id}>
                                <div class="row between">
                                    <div class="model-title">
                                        <h3>{modelTitle}</h3>
                                        <span class="status-pill" data-running={model.running || undefined}>{t($locale, model.running ? 'models.running' : model.download?.status === 'downloading' ? 'local.downloading' : model.selected ? 'models.selected' : model.installed ? 'models.installed' : 'models.notDownloaded')}</span>
                                    </div>
                                    <span class="muted tabular">{model.downloadBytes != null ? formatUnifiedGb(model.downloadBytes, $locale) : ''}</span>
                                </div>
                                {#if model.name && model.name !== modelTitle}<small class="muted">{model.name}</small>{/if}
                                {#if model.description || model.subtitle}<p class="muted">{model.description || model.subtitle}</p>{/if}
                                <p class="model-details muted tabular">
                                    {#if model.quantization || model.format}<span>{model.quantization || model.format}</span>{/if}
                                    <span>{t($locale, 'local.context')} · {formatContext(model.parameters?.contextTokens ?? model.contextWindowTokens)}</span>
                                    {#if model.estimatedRuntimeBytes}<span>{t($locale, 'local.runtime')} · {formatUnifiedGb(model.estimatedRuntimeBytes, $locale)}</span>{/if}
                                    <span>{engineName(modelEngine)}</span>
                                </p>
                                {#if model.download?.status === 'downloading'}
                                    <progress aria-label={t($locale, 'models.downloadProgress')} value={model.download.totalBytes > 0 ? model.download.downloadedBytes : undefined} max={model.download.totalBytes || undefined}></progress>
                                {/if}
                                {#if model.download?.error}<p class="error">{model.download.error}</p>{/if}
                                <div class="actions">
                                    {#if model.installed}
                                        {#if model.running}
                                            <button type="button" class="primary" onclick={() => action(model, 'stop')} disabled={!!busy}>{t($locale, 'models.stop')}</button>
                                        {:else if canRun}
                                            <button type="button" class="primary" onclick={() => switchAndUse(model)} disabled={!!busy}>{t($locale, 'local.switchAndUse')}</button>
                                        {:else}
                                            <button type="button" onclick={() => showEngineSetup(modelEngine)}>{t($locale, 'local.setup.openSetup')}</button>
                                        {/if}
                                        <button type="button" onclick={() => configure(model)} disabled={!!busy}>{t($locale, 'models.configure')}</button>
                                        <button type="button" class="danger" onclick={() => action(model, 'installation')} disabled={!!busy || model.running}>{t($locale, 'models.delete')}</button>
                                    {/if}
                                </div>
                            </article>
                        {/each}
                    </div>
                </section>
            {/if}
            {#if localPageOpened}<LocalModelSearch bind:engine bind:source job={catalog.acquisition} onJob={trackDownload} nativeRuntime={desktopRuntime}>
                {#snippet engineControls()}
                    <LocalEngineSetup bind:this={engineSetup} bind:engine {alternative} runningName={runningModel?.displayName || runningModel?.name || runningModel?.id}
                        nativeRuntime={desktopRuntime} onStatus={updateEngine} openConnections={() => { selectTab('api'); requestedEndpointName = 'vLLM'; }} />
                {/snippet}
            </LocalModelSearch>{/if}
            {#if !visibleModels.length && catalog.diskUsageBytes}
                <section class="card storage-card"><h2>{t($locale, 'local.storage')}</h2><p class="muted">{formatBytes(catalog.diskUsageBytes)}</p></section>
            {/if}
        {/if}
    </div>
    <div id="panel-api" role="tabpanel" aria-labelledby="tab-api" hidden={tab !== 'api'}>
        {#if apiPageOpened}<ApiModels bind:requestedEndpointName openProfileId={tab === 'api' ? linkedModelId : ''} />{/if}
    </div>
</div>
<dialog use:ownDialog bind:this={dialog} oncancel={closeEditor} onclick={event => { if (event.target === dialog && !busy) void closeEditor(); }} aria-label={t($locale, 'models.configureTitle', { name: editing?.displayName || editing?.name || editing?.id || '' })}>
    {#if draft && editing}
        <form onsubmit={save} aria-busy={!!busy}>
            <div><h2>{editing.displayName || editing.name || editing.id}</h2><p class="muted">{t($locale, 'models.nextStart')}</p></div>
            {#if editorError}<div class="notice error" role="alert">{modelValidationText($locale, editorError)} <button type="button" onclick={reloadEditor} disabled={!!busy}>{t($locale, 'models.reload')}</button></div>{/if}
            <div class="fields">
                {#each numericFields.filter(field => editing?.runtime !== 'vllm' || !['batchSize', 'gpuLayers', 'threads'].includes(field.key)) as field}
                    <label>{t($locale, field.label)}<input type="number" bind:value={draft[field.key]} min={field.key === 'contextTokens' ? editing.minimumContextWindowTokens ?? field.min : field.min} max={field.key === 'contextTokens' ? editing.nativeContextTokens || editing.contextWindowTokens || field.max : field.max} step={field.step < 1 ? 'any' : field.step} disabled={!!busy} required /></label>
                {/each}
            </div>
            {#if editing.runtime !== 'vllm'}<label class="row"><input class="check" type="checkbox" role="switch" bind:checked={draft.flashAttention} disabled={!!busy} />{t($locale, 'modelDisplay.flashAttention')}</label>{/if}
            <div class="actions"><button type="button" onclick={() => closeEditor()} disabled={!!busy}>{t($locale, 'models.cancel')}</button><button class="primary" disabled={!!busy || !dirty}>{t($locale, busy ? 'models.saving' : 'models.save')}</button></div>
        </form>
    {/if}
</dialog>
<style>
    .models-header { max-width: 1120px; margin: auto; border-bottom: 0; padding: 40px 32px 24px; align-items: start; }
    .models-header h1 { font-size: 1.75rem; letter-spacing: -.025em; }
    .models-header p { margin-top: 8px; }
    .models-content { max-width: 1120px; padding: 0 32px 48px; gap: 20px; }
    .models-toolbar { display: flex; justify-content: space-between; align-items: center; flex-wrap: wrap; gap: 12px; }
    .tabs button { padding-inline: 16px; }
    #panel-api :global(.profile-settings) { margin-block: 0 24px; }
    .installed-section { display: grid; gap: 12px; margin-bottom: 20px; padding-bottom: 16px; border-bottom: 1px solid var(--border); }
    .section-heading { display: flex; align-items: baseline; justify-content: space-between; gap: 12px; }
    .section-heading h2 { font-size: 1rem; font-weight: 600; margin: 0; }
    .model-title { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
    .model-title h3 { font-size: .9375rem; font-weight: 600; margin: 0; }
    .status-pill { font-size: .75rem; padding: 2px 8px; border-radius: 999px; background: var(--panel); color: var(--muted); }
    .status-pill[data-running] { background: var(--accent-subtle); color: var(--content); font-weight: 600; }
    .tabular { font-variant-numeric: tabular-nums; }
    .models { display: grid; gap: 10px; }
    .model-row { display: grid; grid-template-columns: minmax(0, 1fr) auto; column-gap: 20px; row-gap: 6px; padding: 14px 16px; border: 1px solid var(--border); border-radius: 14px; background: var(--surface); min-width: 0; }
    .model-row > :not(.actions) { grid-column: 1; min-width: 0; overflow-wrap: anywhere; }
    .model-row > .actions { grid-column: 2; grid-row: 1 / span 5; align-content: center; align-items: center; max-width: 320px; }
    .model-row .actions button:not(.primary) { border-color: transparent; background: transparent; }
    .storage-card { background: transparent; border: 0; border-radius: 0; padding: 16px 0; gap: 8px; }
    .model-details { display: flex; flex-wrap: wrap; gap: 4px 16px; margin: 0; font-size: .8125rem; }
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
