<script lang="ts">
    import { onDestroy, onMount, tick, untrack, type Snippet } from 'svelte';
    import { api as unscopedApi, errorMessage } from '../lib/api';
    import { workspacePageApi } from '../lib/workspacePageLocation';
    import { latestRequest } from '../lib/latestRequest';
    import { t, locale } from '../lib/i18n';
    import { downloadActive, engineName, sourceName, modelByteSize, modelMemorySize, targetContextTokens, contextLimitation, repositoryUrl, unslothQuantization, type LocalHardware, type ModelCandidate, type LocalEngine, type ModelDownload, type ModelSource } from '../lib/modelAcquisition';

    let { engine = $bindable(), source = $bindable(), job, onJob, engineControls }: {
        engine: LocalEngine; source: ModelSource; job?: ModelDownload | null;
        onJob: (next: ModelDownload) => void; engineControls?: Snippet;
    } = $props();

    const api = workspacePageApi(unscopedApi);
    type Suggestions = { hardware: LocalHardware; candidates: ModelCandidate[]; recommendationSource: string; reason?: string };
    const reads = latestRequest<Suggestions>();
    const cached = new Map<string, { received: number; value: Suggestions }>();
    let hardware = $state<LocalHardware>();
    let hardwareError = $state('');
    let noMatchReason = $state('');
    let recommendationSource = $state('');
    const hardwareRequest = latestRequest<{hardware:LocalHardware}>();
    async function loadHardware() {
        hardwareError = '';
        try {
            const result = await hardwareRequest.run(signal => api('/api/local-models/hardware', {signal}));
            if (result) hardware = result.hardware;
        } catch (error) { if (alive) hardwareError = errorMessage(error); }
    }
    onMount(() => { void loadHardware(); });
    let query = $state('');
    let results = $state<ModelCandidate[]>();
    let looking = $state(false);
    let mutating = $state(false);
    let searchError = $state('');
    let actionError = $state('');
    let alive = true;
    let searchVersion = 0;
    let pending: ReturnType<typeof setTimeout> | undefined;
    let previousSelection = '';
    const hardwareReady = $derived(hardware !== undefined);
    $effect(() => {
        const key = engine + '/' + source;
        if (hardwareReady) untrack(() => {
            if (previousSelection && previousSelection !== key) clearSearch();
            previousSelection = key;
            recommendWhenEmpty();
        });
    });
    onDestroy(() => { alive = false; clearTimeout(pending); reads.cancel(); hardwareRequest.cancel(); });
    const canStart = $derived(!mutating && !downloadActive(job));

    function clearSearch() {
        clearTimeout(pending);
        ++searchVersion;
        reads.cancel();
        results = undefined;
        looking = false;
        searchError = '';
    }

    function recommendWhenEmpty() {
        if (!query.trim() && hardwareReady && !looking) {
            clearTimeout(pending);
            pending = setTimeout(() => { void lookup(true); }, 150);
        }
    }

    function editQuery(event: Event) {
        query = (event.currentTarget as HTMLInputElement).value;
        clearSearch();
        recommendWhenEmpty();
    }

    function show(value: Suggestions) {
        results = value.candidates;
        hardware = value.hardware;
        recommendationSource = value.recommendationSource;
        noMatchReason = value.reason || '';
    }

    function search(event: SubmitEvent) {
        event.preventDefault();
        if (!looking) void lookup(false);
    }

    async function lookup(automatic: boolean) {
        clearTimeout(pending);
        const selection = { engine, source, query: query.trim() };
        const key = engine + '/' + source;
        const previous = cached.get(key);
        if (automatic && previous && Date.now() - previous.received < 5 * 60_000) {
            show(previous.value);
            return;
        }
        const version = ++searchVersion;
        looking = true; searchError = ''; results = undefined;
        try {
            const value = await reads.run(signal => api('/api/local-models/recommend', {
                method: 'POST', body: JSON.stringify(selection), signal,
            }));
            if (value) {
                if (!selection.query) cached.set(key, {received: Date.now(), value});
                show(value);
            }
        } catch (error) {
            if (alive) searchError = errorMessage(error);
        } finally {
            if (alive && searchVersion === version) looking = false;
        }
    }

    async function reviewDownload(download: ModelDownload) {
        clearSearch();
        actionError = '';
        engine = download.engine;
        source = download.source;
        query = download.repository;
        // Allow the engine/source effect to invalidate the previous search first.
        await tick();
        if (alive && engine === download.engine && source === download.source && query === download.repository) {
            await lookup(false);
        }
    }

    async function update(action: 'install-plan' | 'cancel', body: object) {
        if (mutating) return;
        mutating = true; actionError = '';
        try {
            const result = await api<{job: ModelDownload}>('/api/local-models/' + action, {
                method: 'POST', body: JSON.stringify(body),
            });
            if (alive) {
                if (action === 'install-plan') results = undefined;
                onJob(result.job);
            }
        } catch (error) {
            if (alive) actionError = errorMessage(error);
        } finally {
            if (alive) mutating = false;
        }
    }
</script>

<section class="acquisition" aria-label={t($locale, 'local.find')}>
    <section class="hardware" aria-label={t($locale, 'local.hardware')}>
        <h2>{t($locale, 'local.hardware')}</h2>
        {#if hardware}
            <dl>
                <div><dt>CPU</dt><dd>{hardware.cpu || t($locale, 'local.unavailable')}{hardware.cores ? ' · ' + t($locale, 'local.cores', {count: String(hardware.cores)}) : ''}</dd></div>
                <div><dt>GPU</dt><dd>{hardware.gpu || t($locale, 'local.unavailable')}</dd></div>
                <div><dt>{t($locale, hardware.unifiedMemory ? 'local.unifiedMemory' : 'local.memory')}</dt><dd>{hardware.memoryBytes != null ? modelMemorySize(hardware.memoryBytes, $locale) : t($locale, 'local.unavailable')}</dd></div>
                {#if !hardware.unifiedMemory && hardware.gpuMemoryBytes != null}<div><dt>{t($locale, 'local.graphicsMemory')}</dt><dd>{modelMemorySize(hardware.gpuMemoryBytes, $locale)}</dd></div>{/if}
                <div><dt>{t($locale, 'local.diskFree')}</dt><dd>{hardware.freeDiskBytes != null ? modelByteSize(hardware.freeDiskBytes, $locale) : t($locale, 'local.unavailable')}</dd></div>
            </dl>
        {:else if hardwareError}<p role="alert">{hardwareError} <button onclick={loadHardware}>{t($locale, 'models.retry')}</button></p>
        {:else}<p role="status">{t($locale, 'models.loading')}</p>{/if}
    </section>
    <div class="find-heading">
        <h2>{t($locale, 'local.find')}</h2>
        <span>{t($locale, 'local.contextTarget', { tokens: targetContextTokens.toLocaleString($locale) })}</span>
    </div>
    <form onsubmit={search}>
        <div class="choices">
            <label>{t($locale, 'local.engine')}
                <select bind:value={engine}><option value="llama.cpp">llama.cpp</option><option value="vllm">vLLM</option></select>
            </label>
            <label>{t($locale, 'local.source')}
                <select bind:value={source}><option value="huggingface">Hugging Face</option><option value="modelscope">ModelScope</option></select>
            </label>
        </div>
        <label>{t($locale, 'local.modelName')}
            <input type="search" value={query} oninput={editQuery} maxlength="256" placeholder={t($locale, 'local.optionalModel')} />
        </label>
        <button class="primary search" disabled={looking}>{t($locale, looking ? 'local.searching' : query.trim() ? 'local.search' : results ? 'local.refreshRecommendations' : 'local.recommend')}</button>
    </form>
    {@render engineControls?.()}
    {#if searchError}<p class="error" role="alert">{searchError}</p>{/if}
    {#if results}
        {#if recommendationSource === 'modelfit' || recommendationSource === 'mixed'}
            <p class="attribution"><a href="https://modelfit.io/data/" rel="noopener noreferrer" target="_blank">ModelFit</a> · <a href="https://creativecommons.org/licenses/by/4.0/" rel="noopener noreferrer" target="_blank">CC BY 4.0</a>{#if recommendationSource === 'mixed'} · <a href={source === 'modelscope' ? 'https://modelscope.cn/models' : 'https://huggingface.co/models'} rel="noopener noreferrer" target="_blank">{sourceName(source)}</a>{/if}</p>
        {:else if recommendationSource === 'hub'}
            <p class="attribution">{t($locale, 'local.catalogFallback', { source: sourceName(source) })}</p>
        {/if}
        {#if results.length}
            <ul>
                {#each results as candidate (candidate.repository)}
                    {@const limitation = contextLimitation(candidate)}
                    {@const edition = unslothQuantization(candidate)}
                    <li>
                        <div class="candidate"><a class="repository" href={repositoryUrl(source, candidate.repository)} target="_blank" rel="noopener noreferrer">{candidate.repository}</a>
                            {#if candidate.variant}<small class="muted">{edition === 'dynamic' ? 'Unsloth Dynamic · ' : edition === 'quantized' ? t($locale, 'local.unslothQuant') + ' · ' : ''}{candidate.variant}</small>{/if}
                            <dl>
                                {#if candidate.plan.device === 'cpu' || candidate.plan.device === 'gpu' || candidate.plan.device === 'unified'}
                                    <div><dt>{t($locale, 'local.compute')}</dt><dd>{candidate.plan.device === 'unified' ? t($locale, 'local.unifiedMemory') : candidate.plan.device.toUpperCase()}</dd></div>
                                {/if}
                                <div><dt>{t($locale, 'local.download_size')}</dt><dd>{modelByteSize(candidate.downloadBytes, $locale)}</dd></div>
                                <div><dt>{t($locale, 'local.runtime')}</dt><dd>{modelMemorySize(candidate.estimatedMemoryBytes, $locale)}</dd></div>
                                <div><dt>{t($locale, 'local.plannedContext')}</dt><dd>{candidate.contextTokens.toLocaleString($locale)}</dd></div>
                            </dl>
                            {#if limitation}<small class="muted">{t($locale, limitation, { tokens: candidate.nativeContextTokens?.toLocaleString($locale) || '' })}</small>{/if}
                        </div>
                        <button disabled={!canStart} aria-label={t($locale, 'local.confirmDownload') + ' · ' + candidate.repository} onclick={() => update('install-plan', { engine, source, repository: candidate.repository, plan: candidate.plan })}>{t($locale, 'local.confirmDownload')}</button>
                    </li>
                {/each}
            </ul>
        {:else}<p class="muted" role="status">{noMatchReason || t($locale, 'local.noMatches', { engine: engineName(engine) })}</p>{/if}
    {/if}
    {#if job && job.status !== 'installed'}
        <div class="download">
            <div class="details"><strong>{job.repository}</strong><span class="muted">{engineName(job.engine)} · {sourceName(job.source)}</span></div>
            {#if downloadActive(job)}
                <div class="details">
                    <span role="status">{t($locale, job.status === 'cancelling' ? 'local.cancelling' : 'local.downloading')}</span>
                    <span>{job.totalBytes > 0 ? modelByteSize(job.downloadedBytes, $locale) + ' / ' + modelByteSize(job.totalBytes, $locale) : ''}</span>
                </div>
                <progress aria-label={t($locale, 'models.downloadProgress')} max={job.totalBytes || 1} value={job.totalBytes > 0 ? job.downloadedBytes : undefined}></progress>
                <button disabled={mutating || job.status === 'cancelling'} onclick={() => update('cancel', { id: job.id })}>{t($locale, 'models.cancel')}</button>
            {:else if job.status === 'failed' || job.status === 'cancelled'}
                <p role="status">{job.error || t($locale, 'local.cancelled')}</p>
                <button disabled={!canStart || looking} onclick={() => reviewDownload(job!)}>{t($locale, 'local.reviewDownloadPlan')}</button>
            {/if}
        </div>
    {/if}
    {#if actionError}<p class="error" role="alert">{actionError}</p>{/if}
</section>
<style>
    .acquisition { padding-block: 16px 24px; border-bottom: 1px solid var(--border); }
    h2 { margin: 0 0 12px; font-size: 1rem; }
    .attribution { margin-block: 12px 0; font-size: .8125rem; color: var(--muted); }
    .attribution a { color: inherit; }
    .hardware { margin-bottom: 24px; }
    dl { display: flex; flex-wrap: wrap; gap: 10px 24px; margin: 0; }
    dl div { display: grid; gap: 3px; }
    dt { color: var(--muted); font-size: .75rem; }
    dd { margin: 0; font-size: .875rem; }
    .candidate dl { margin-block: 8px 2px; }
    .repository { font-weight: 600; color: inherit; width: fit-content; text-decoration: none; }
    .repository:hover, .repository:focus-visible { text-decoration: underline; }
    .find-heading { display: flex; flex-wrap: wrap; align-items: baseline; gap: 4px 14px; margin-bottom: 12px; }
    .find-heading h2 { margin: 0; }
    .find-heading span { font-size: .8125rem; color: var(--muted); }
    form { display: flex; align-items: end; flex-wrap: wrap; gap: 10px; }
    .choices { display: flex; gap: 10px; flex: 1 1 255px; }
    label { display: grid; gap: 4px; min-width: 0; flex: 1 1 165px; font-size: .8125rem; }
    select, input { width: 100%; min-height: 40px; }
    .search { min-height: 40px; }
    ul { list-style: none; padding: 0; margin-bottom: 0; }
    li, .details { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
    li { padding: 6px 0; border-top: 1px solid var(--border); }
    .candidate { display: grid; gap: 2px; min-width: 0; overflow-wrap: anywhere; }
    .details strong, p { min-width: 0; overflow-wrap: anywhere; }
    .download { display: grid; gap: 8px; margin-top: 12px; }
    .download button { justify-self: start; }
    .details { flex-wrap: wrap; font-size: .875rem; }
    .download progress { width: 100%; }
</style>
