<script module lang="ts">
    import type { LocalHardware, ModelCandidate } from '../lib/modelAcquisition';
    type Suggestions = { hardware: LocalHardware; candidates: ModelCandidate[]; recommendationSource: string; reason?: string };
    const cached = new Map<string, { received: number; value: Suggestions }>();
    let cachedHardware: LocalHardware | undefined;
</script>

<script lang="ts">
    import { onDestroy, onMount, tick, untrack, type Snippet } from 'svelte';
    import { api as unscopedApi, errorMessage } from '../lib/api';
    import { workspacePageApi } from '../lib/workspacePageLocation';
    import { latestRequest } from '../lib/latestRequest';
    import { t, locale } from '../lib/i18n';
    import {
        downloadActive, engineName, sourceName, modelByteSize, modelMemorySize,
        targetContextTokens, contextLimitation, repositoryUrl, unslothQuantization,
        parseDirectModelInput, cleanModelName, formatUnifiedGb, modelFitLevel, formatEta,
        type LocalEngine, type ModelDownload, type ModelSource
    } from '../lib/modelAcquisition';

    let { engine = $bindable(), source = $bindable(), job, onJob, nativeRuntime = '', engineControls }: {
        engine: LocalEngine; source: ModelSource; job?: ModelDownload | null;
        onJob: (next: ModelDownload) => void; nativeRuntime?: string; engineControls?: Snippet;
    } = $props();

    const api = workspacePageApi(unscopedApi);
    const reads = latestRequest<Suggestions>();
    let hardware = $state<LocalHardware | undefined>(cachedHardware);
    let hardwareError = $state('');
    let noMatchReason = $state('');
    let recommendationSource = $state('');
    const hardwareRequest = latestRequest<{hardware:LocalHardware}>();
    async function loadHardware() {
        hardwareError = '';
        try {
            const result = await hardwareRequest.run(signal => api('/api/local-models/hardware', {signal}));
            if (result) { hardware = result.hardware; cachedHardware = result.hardware; }
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

    // Live download speed & ETA tracking
    let lastJobId = '';
    let lastSampleBytes = 0;
    let lastSampleTime = 0;
    let bytesPerSecond = $state(0);

    $effect(() => {
        if (!job || !downloadActive(job)) {
            lastJobId = '';
            lastSampleBytes = 0;
            lastSampleTime = 0;
            bytesPerSecond = 0;
            return;
        }
        const now = Date.now();
        if (job.id !== lastJobId) {
            lastJobId = job.id;
            lastSampleBytes = job.downloadedBytes;
            lastSampleTime = now;
            bytesPerSecond = 0;
        } else if (now - lastSampleTime >= 500 && job.downloadedBytes >= lastSampleBytes) {
            const deltaBytes = job.downloadedBytes - lastSampleBytes;
            const deltaSec = (now - lastSampleTime) / 1000;
            if (deltaSec > 0) {
                const instant = deltaBytes / deltaSec;
                bytesPerSecond = bytesPerSecond > 0 ? bytesPerSecond * 0.65 + instant * 0.35 : instant;
            }
            lastSampleBytes = job.downloadedBytes;
            lastSampleTime = now;
        }
    });

    const downloadPercent = $derived(job && job.totalBytes > 0 ? Math.min(100, Math.round((job.downloadedBytes / job.totalBytes) * 100)) : 0);
    const downloadEta = $derived.by(() => {
        if (!job || !downloadActive(job) || job.totalBytes <= 0 || bytesPerSecond <= 1024) return '';
        const remaining = (job.totalBytes - job.downloadedBytes) / bytesPerSecond;
        return formatEta(remaining, $locale);
    });
    const downloadSpeedText = $derived.by(() => {
        if (bytesPerSecond <= 1024) return '';
        return `${modelByteSize(bytesPerSecond, $locale)}/s`;
    });

    const hostSummaryText = $derived.by(() => {
        if (!hardware) return '';
        const parts: string[] = [];
        if (hardware.memoryBytes != null) {
            parts.push(t($locale, 'local.hostSummary', { memory: formatUnifiedGb(hardware.memoryBytes, $locale) }));
        }
        if (nativeRuntime) {
            parts.push(nativeRuntime);
        } else if (hardware.gpu) {
            parts.push(hardware.gpu);
        } else if (hardware.cpu) {
            parts.push(hardware.cpu);
        }
        return parts.join(' · ');
    });

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
        const raw = (event.currentTarget as HTMLInputElement).value;
        const parsed = parseDirectModelInput(raw);
        if (parsed?.source && parsed.source !== source) {
            source = parsed.source;
            query = parsed.repository;
            clearSearch();
            void tick().then(() => { if (alive) void lookup(false); });
            return;
        }
        query = raw;
        clearSearch();
        recommendWhenEmpty();
    }

    function show(value: Suggestions) {
        results = value.candidates;
        hardware = value.hardware;
        cachedHardware = value.hardware;
        recommendationSource = value.recommendationSource;
        noMatchReason = value.reason || '';
    }

    function search(event: SubmitEvent) {
        event.preventDefault();
        const parsed = parseDirectModelInput(query);
        if (parsed) {
            if (parsed.source && parsed.source !== source) source = parsed.source;
            query = parsed.repository;
        }
        if (!looking) void lookup(false);
    }

    async function lookup(automatic: boolean) {
        clearTimeout(pending);
        const parsed = parseDirectModelInput(query);
        const normalizedQuery = parsed ? parsed.repository : query.trim();
        const selection = { engine, source, query: normalizedQuery };
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

    async function retryDownload(download: ModelDownload) {
        if (!canStart) return;
        clearSearch();
        actionError = '';
        engine = download.engine;
        source = download.source;
        query = download.repository;
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
                onJob(result.job);
            }
        } catch (error) {
            if (alive) actionError = errorMessage(error);
        } finally {
            if (alive) mutating = false;
        }
    }

    function formatContextShort(tokens: number) {
        return tokens >= 1024 && tokens % 1024 === 0 ? `${tokens / 1024}K` : tokens.toLocaleString($locale);
    }
</script>

<section class="acquisition" aria-label={t($locale, 'local.find')}>
    {#if job && job.status !== 'installed'}
        <div class="download-banner" aria-live="polite">
            <div class="download-head">
                <div class="download-title">
                    <strong>{cleanModelName(job.repository)}</strong>
                    <span class="muted">{sourceName(job.source)} · {engineName(job.engine)}</span>
                </div>
                {#if downloadActive(job)}
                    <button type="button" disabled={mutating || job.status === 'cancelling'} onclick={() => update('cancel', { id: job.id })}>{t($locale, 'models.cancel')}</button>
                {:else if job.status === 'failed' || job.status === 'cancelled'}
                    <button type="button" class="primary" disabled={!canStart || looking} onclick={() => retryDownload(job!)}>{t($locale, 'local.reviewDownloadPlan')}</button>
                {/if}
            </div>
            {#if downloadActive(job)}
                <div class="download-metrics">
                    <span role="status">{t($locale, job.status === 'cancelling' ? 'local.cancelling' : 'local.downloading')}{#if job.totalBytes > 0} · {downloadPercent}%{/if}</span>
                    <span class="tabular">
                        {#if job.totalBytes > 0}{formatUnifiedGb(job.downloadedBytes, $locale)} / {formatUnifiedGb(job.totalBytes, $locale)}{/if}
                        {#if downloadSpeedText} · {downloadSpeedText}{/if}
                        {#if downloadEta} · {downloadEta}{/if}
                    </span>
                </div>
                <progress aria-label={t($locale, 'models.downloadProgress')} max={job.totalBytes || 1} value={job.totalBytes > 0 ? job.downloadedBytes : undefined}></progress>
            {:else if job.status === 'failed' || job.status === 'cancelled'}
                <p class="error" role="status">{job.error || t($locale, 'local.cancelled')}</p>
            {/if}
        </div>
    {/if}

    <form class="search-bar" onsubmit={search}>
        <label class="sr-only" for="local-model-query">{t($locale, 'local.modelName')}</label>
        <input id="local-model-query" type="search" value={query} oninput={editQuery} maxlength="256" placeholder={t($locale, 'local.searchPlaceholder')} />
        <button class="primary search-btn" disabled={looking}>{t($locale, looking ? 'local.searching' : query.trim() ? 'local.search' : results ? 'local.refreshRecommendations' : 'local.recommend')}</button>
    </form>

    <div class="info-bar">
        <div class="host-summary">
            {#if hardware}
                <details class="technical hardware" aria-label={t($locale, 'local.hardware')}>
                    <summary>{hostSummaryText || t($locale, 'local.hardware')}</summary>
                    <dl>
                        {#if hardware.cpu || hardware.cores}<div><dt>CPU</dt><dd>{[hardware.cpu, hardware.cores ? t($locale, 'local.cores', {count: hardware.cores}) : ''].filter(Boolean).join(' · ')}</dd></div>{/if}
                        {#if hardware.gpu}<div><dt>GPU</dt><dd>{hardware.gpu}</dd></div>{/if}
                        {#if hardware.memoryBytes != null}<div><dt>{t($locale, hardware.unifiedMemory ? 'local.unifiedMemory' : 'local.memory')}</dt><dd>{modelMemorySize(hardware.memoryBytes, $locale)}</dd></div>{/if}
                        {#if !hardware.unifiedMemory && hardware.gpuMemoryBytes != null}<div><dt>{t($locale, 'local.graphicsMemory')}</dt><dd>{modelMemorySize(hardware.gpuMemoryBytes, $locale)}</dd></div>{/if}
                        {#if hardware.freeDiskBytes != null}<div><dt>{t($locale, 'local.diskFree')}</dt><dd>{modelByteSize(hardware.freeDiskBytes, $locale)}</dd></div>{/if}
                    </dl>
                    {#if !hardware.cpu && !hardware.cores && !hardware.gpu && hardware.memoryBytes == null && hardware.gpuMemoryBytes == null && hardware.freeDiskBytes == null}<small>{t($locale, 'local.unavailable')}</small>{/if}
                </details>
            {:else if hardwareError}
                <span class="error" role="alert">{hardwareError} <button type="button" class="inline-retry" onclick={loadHardware}>{t($locale, 'models.retry')}</button></span>
            {:else}
                <span class="muted" role="status">{t($locale, 'models.loading')}</span>
            {/if}
        </div>
        <div class="source-controls">
            <div class="source-toggle" role="group" aria-label={t($locale, 'local.source')}>
                <button type="button" aria-pressed={source === 'modelscope'} onclick={() => { source = 'modelscope'; }}>ModelScope</button>
                <button type="button" aria-pressed={source === 'huggingface'} onclick={() => { source = 'huggingface'; }}>Hugging Face</button>
            </div>
            <label class="engine-select">
                <span class="sr-only">{t($locale, 'local.engine')}</span>
                <select bind:value={engine} aria-label={t($locale, 'local.engine')}>
                    <option value="llama.cpp">llama.cpp</option>
                    <option value="vllm">vLLM</option>
                </select>
            </label>
        </div>
    </div>

    {@render engineControls?.()}

    {#if searchError}<p class="error" role="alert">{searchError}</p>{/if}
    {#if actionError}<p class="error" role="alert">{actionError}</p>{/if}

    {#if results}
        {#if results.length}
            <p class="context-target">{t($locale, 'local.contextTarget', {tokens: targetContextTokens.toLocaleString($locale)})}</p>
            <ul class="candidate-list">
                {#each results as candidate (candidate.id || `${candidate.repository}:${candidate.quantization ?? candidate.variant ?? ''}`)}
                    {@const limitation = contextLimitation(candidate)}
                    {@const edition = unslothQuantization(candidate)}
                    {@const fit = modelFitLevel(candidate, hardware)}
                    <li class="candidate-card">
                        <div class="candidate-main">
                            <div class="candidate-title-row">
                                <strong>{cleanModelName(candidate.repository)}</strong>
                                <span class="fit-badge" data-fit={fit}>{t($locale, fit === 'best' ? 'local.fitBest' : fit === 'light' ? 'local.fitLight' : 'local.fitLarge')}</span>
                                <span class="size-pill tabular">{formatUnifiedGb(candidate.downloadBytes, $locale)}</span>
                            </div>
                            <a class="repository" href={repositoryUrl(source, candidate.repository)} target="_blank" rel="noopener noreferrer">{candidate.repository}</a>
                            <p class="candidate-meta muted tabular">
                                <span>{t($locale, 'local.runtime')} · {formatUnifiedGb(candidate.estimatedMemoryBytes, $locale)}</span>
                                <span>{t($locale, 'local.plannedContext')} · {formatContextShort(candidate.contextTokens)}</span>
                            </p>
                            <details class="technical">
                                <summary>{t($locale, 'local.technicalDetails')}</summary>
                                <div class="tech-body">
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
                            </details>
                        </div>
                        <button class="primary download-btn" disabled={!canStart} aria-label={t($locale, 'local.confirmDownload') + ' · ' + candidate.repository} onclick={() => update('install-plan', { engine, source, repository: candidate.repository, plan: candidate.plan })}>{t($locale, 'local.download')}</button>
                    </li>
                {/each}
            </ul>
        {:else}<p class="muted" role="status">{noMatchReason || t($locale, 'local.noMatches', { engine: engineName(engine) })}</p>{/if}
        {#if recommendationSource === 'modelfit' || recommendationSource === 'mixed'}
            <p class="attribution"><a href="https://modelfit.io/data/" rel="noopener noreferrer" target="_blank">ModelFit</a> · <a href="https://creativecommons.org/licenses/by/4.0/" rel="noopener noreferrer" target="_blank">CC BY 4.0</a>{#if recommendationSource === 'mixed'} · <a href={source === 'modelscope' ? 'https://modelscope.cn/models' : 'https://huggingface.co/models'} rel="noopener noreferrer" target="_blank">{sourceName(source)}</a>{/if}</p>
        {:else if recommendationSource === 'hub'}
            <p class="attribution">{t($locale, 'local.catalogFallback', { source: sourceName(source) })}</p>
        {/if}
    {:else if looking}
        <p class="muted" role="status">{t($locale, 'local.searching')}</p>
    {/if}
</section>

<style>
    .sr-only { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap; }
    .tabular { font-variant-numeric: tabular-nums; }
    .acquisition { display: grid; gap: 14px; padding-block: 8px 20px; }
    .download-banner { display: grid; gap: 10px; padding: 14px 16px; border: 1px solid var(--border); border-radius: 14px; background: var(--surface); }
    .download-head { display: flex; align-items: center; justify-content: space-between; gap: 12px; flex-wrap: wrap; }
    .download-title { display: flex; align-items: baseline; gap: 8px; flex-wrap: wrap; min-width: 0; }
    .download-title strong { font-size: .9375rem; }
    .download-metrics { display: flex; align-items: center; justify-content: space-between; gap: 12px; flex-wrap: wrap; font-size: .8125rem; color: var(--muted); }
    .search-bar { display: flex; gap: 8px; align-items: stretch; }
    .search-bar input { flex: 1; min-width: 0; min-height: 42px; }
    .search-btn { flex: none; min-height: 42px; padding-inline: 16px; }
    .info-bar { display: flex; align-items: center; justify-content: space-between; gap: 10px 16px; flex-wrap: wrap; font-size: .8125rem; }
    .host-summary { min-width: 0; overflow-wrap: anywhere; }
    .inline-retry { min-height: 28px; padding: 2px 8px; font-size: .75rem; margin-inline-start: 6px; }
    .source-controls { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
    .source-toggle { display: inline-flex; padding: 2px; border: 1px solid var(--border); border-radius: 10px; background: var(--panel); gap: 2px; }
    .source-toggle button { min-height: 30px; min-width: 0; padding: 4px 10px; font-size: .8125rem; border: 0; border-radius: 8px; background: transparent; color: var(--muted); box-shadow: none; }
    .source-toggle button[aria-pressed='true'] { background: var(--surface); color: var(--content); font-weight: 600; }
    .engine-select { display: inline-flex; margin: 0; }
    .engine-select select { min-height: 34px; padding: 4px 28px 4px 10px; font-size: .8125rem; width: auto; }
    .candidate-list { list-style: none; padding: 0; margin: 0; display: grid; gap: 10px; }
    .candidate-card { display: flex; align-items: flex-start; justify-content: space-between; gap: 16px; padding: 14px 16px; border: 1px solid var(--border); border-radius: 14px; background: var(--surface); }
    .candidate-main { display: grid; gap: 6px; min-width: 0; flex: 1; }
    .candidate-title-row { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
    .candidate-title-row strong { font-size: .9375rem; }
    .fit-badge { font-size: .75rem; font-weight: 550; padding: 2px 8px; border-radius: 999px; background: var(--accent-subtle); color: var(--content); }
    .fit-badge[data-fit='large'] { background: var(--panel); color: var(--muted); }
    .size-pill { font-size: .8125rem; color: var(--muted); }
    .candidate-meta { display: flex; flex-wrap: wrap; gap: 4px 14px; margin: 0; font-size: .8125rem; }
    .download-btn { flex: none; align-self: center; }
    .technical { margin-top: 2px; }
    .technical > summary { font-size: .75rem; color: var(--muted); padding: 2px 0; min-height: 28px; cursor: pointer; list-style: none; display: inline-flex; align-items: center; gap: 4px; }
    .technical > summary::-webkit-details-marker { display: none; }
    .technical > summary::before { content: '▸'; font-size: .75rem; transition: transform var(--motion-fast) ease; }
    .technical[open] > summary::before { transform: rotate(90deg); }
    .tech-body { display: grid; gap: 6px; padding: 8px 0 2px; font-size: .8125rem; }
    .context-target { margin: 0; font-size: .8125rem; color: var(--muted); }
    .hardware dl { padding-block: 6px; }
    .repository { font-size: .8125rem; color: var(--muted); width: fit-content; text-decoration: none; overflow-wrap: anywhere; }
    .repository:hover, .repository:focus-visible { text-decoration: underline; }
    dl { display: flex; flex-wrap: wrap; gap: 8px 20px; margin: 0; }
    dl div { display: grid; gap: 2px; }
    dt { color: var(--muted); font-size: .75rem; }
    dd { margin: 0; font-size: .8125rem; }
    .attribution { margin: 4px 0 0; font-size: .75rem; color: var(--muted); }
    .attribution a { color: inherit; text-decoration: underline; }
    @media (max-width: 760px) {
        .search-bar { flex-direction: column; }
        .candidate-card { flex-direction: column; }
        .download-btn { align-self: stretch; }
    }
</style>
