<script lang="ts">
    import { workspacePageApi } from '../lib/workspacePageLocation';
    const api = workspacePageApi(unscopedApi);
    import { workspaceDialog } from '../lib/workspacePageLocation';
    const ownDialog = workspaceDialog();
    import { workspacePageLocation } from '../lib/workspacePageLocation';
    const pageLocation = workspacePageLocation();
    $effect(() => { if (pageLocation.active()) untrack(() => { void load(); }); });
    import { onMount, untrack } from 'svelte';
    import { locale, t } from '../lib/i18n';
    import { api as unscopedApi } from '../lib/api';
    import { activityErrorKey } from '../lib/activityErrors';
    import { activeResult, successfulResult, attentionResult, filterResults, resultLink, resultTime, resultStatus, resultFormat, sourceResults, executionConversationLink } from '../lib/results';
    import type { ResultSummary, ResultDetail } from '../lib/results';
    import Markdown from '../components/Markdown.svelte';
    let results = $state<ResultSummary[]>([]);
    let loaded = $state(false);
    let loading = $state(false);
    let error = $state('');
    let query = $state('');
    let filter = $state('all');
    let selected = $state('');
    let detail = $state<ResultDetail>();
    let detailError = $state('');
    let detailLoading = $state(false);
    let dialog: HTMLDialogElement;
    let controller: AbortController | undefined;
    let detailController: AbortController | undefined;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let disposed = false;
    const sourceId = pageLocation.read().searchParams.get('sourceId');
    const scopedResults = $derived(sourceResults(results, sourceId));
    const active = $derived(scopedResults.filter(activeResult));
    const history = $derived(filterResults(scopedResults.filter(result => !activeResult(result)), query, filter, $locale));
    const successCount = $derived(scopedResults.filter(successfulResult).length);
    const attentionCount = $derived(scopedResults.filter(attentionResult).length);
    async function load() {
        if (loading || disposed || document.hidden || !pageLocation.active()) return;
        clearTimeout(timer); loading = true; controller = new AbortController();
        try {
            const response = await api<{ results: ResultSummary[] }>('/api/results', { signal: controller.signal });
            if (disposed) return;
            results = response.results; loaded = true; error = '';
        } catch (cause) { if (!disposed) error = activityErrorKey(cause); }
        finally {
            loading = false;
            if (!disposed && pageLocation.active() && !document.hidden && results.some(activeResult)) timer = setTimeout(load, 2500);
        }
    }
    function visibility() { clearTimeout(timer); if (!document.hidden) void load(); }
    onMount(() => {
        const initialResult = pageLocation.read().searchParams.get('resultId');
        void load().then(() => { if (initialResult && !disposed && pageLocation.active()
            && results.some(result => result.resultId === initialResult && (sourceId === null || result.sourceId === sourceId))) void showDetail(initialResult); });
        document.addEventListener('visibilitychange', visibility);
        return () => { disposed = true; clearTimeout(timer); controller?.abort(); detailController?.abort(); document.removeEventListener('visibilitychange', visibility); };
    });
    async function showDetail(id: string) {
        detailController?.abort(); const request = new AbortController(); detailController = request;
        selected = id; detail = undefined; detailError = ''; detailLoading = true;
        if (!dialog.open) dialog.showModal();
        try {
            const response = await api<ResultDetail>(`/api/results/${encodeURIComponent(id)}`, { signal: request.signal });
            if (!request.signal.aborted && !disposed) detail = response;
        } catch (cause) { if (!request.signal.aborted && !disposed) detailError = activityErrorKey(cause); }
        finally { if (detailController === request) detailLoading = false; }
    }
    function close(event?: Event) {
        event?.preventDefault(); detailController?.abort(); dialog.close(); selected = ''; detail = undefined; detailLoading = false;
    }
</script>

<svelte:window onfocus={() => void load()} />
<svelte:head>{#if pageLocation.active()}<title>{t($locale, 'activity.results')} — Nola</title>{/if}</svelte:head>
<header class="page-header"><div><h1>{t($locale, 'activity.results')}</h1><p class="muted">{t($locale, 'activity.results_hint')}</p></div><button onclick={load} disabled={loading}>{loading ? t($locale, 'activity.refreshing') : t($locale, 'activity.refresh')}</button></header>
<div class="content">
    <div class="card"><p class="muted">{t($locale, 'activity.legacyResultScope')}</p><a class="button" href="/activity">{t($locale, 'chatInspector.list.activity')}</a></div>
    {#if sourceId !== null}<div class="card row between"><p>{t($locale, 'activity.sourceScope', { source: scopedResults[0]?.sourceName || sourceId })}</p><a class="button" href="/results">{t($locale, 'activity.allSources')}</a></div>{/if}
    {#if error}<div class="notice error" role="alert">{t($locale, error)} <button onclick={load} disabled={loading}>{t($locale, 'activity.retry')}</button></div>{/if}
    {#if !loaded && loading}<p role="status">{t($locale, 'activity.loading_results')}</p>{/if}
    {#if loaded}
        <p class="muted">{t($locale, 'activity.result_counts', { total: scopedResults.length, success: successCount, attention: attentionCount })}</p>
        {#if active.length}<section class="result-section"><h2>{t($locale, 'activity.active')} · {active.length}</h2>{#each active as result (result.resultId)}
            <button class="card result" onclick={() => showDetail(result.resultId)}><span class="row between"><strong>{result.sourceName || result.sourceId}</strong><span>{resultStatus(result.status, $locale)}</span></span><span class="muted">{result.previewText || t($locale, 'activity.waiting')}</span></button>
        {/each}</section>{/if}
        <section class="result-section" aria-busy={loading}>
            <h2>{t($locale, 'activity.history')}</h2>
            <div class="fields"><label>{t($locale, 'activity.search')}<input type="search" bind:value={query} placeholder={t($locale, 'activity.search_hint')} /></label><label>{t($locale, 'activity.status')}<select bind:value={filter}><option value="all">{t($locale, 'activity.all')}</option><option value="success">{t($locale, 'activity.status.success')}</option><option value="attention">{t($locale, 'activity.attention')}</option></select></label></div>
            {#each history as result (result.resultId)}
                <button class="card result" onclick={() => showDetail(result.resultId)}>
                    <span class="row between"><strong>{result.sourceName || result.sourceId}</strong><span class:error={attentionResult(result)}>{resultStatus(result.status, $locale)}</span></span>
                    {#if result.previewText}<span class="preview muted">{result.previewText}</span>{/if}
                    <small>{resultFormat(result.sourceFormat, $locale)} · {t($locale, 'activity.scheduled', { time: resultTime(result.scheduledAt, $locale) })} · {t($locale, 'activity.recorded', { time: resultTime(result.createdAt, $locale) })}{result.expired ? ` · ${t($locale, 'activity.expired')}` : ''}</small>
                </button>
            {:else}<div class="card"><h2>{scopedResults.length ? t($locale, 'activity.no_matches') : t($locale, 'activity.no_results')}</h2><p class="muted">{active.length ? t($locale, 'activity.after_complete') : t($locale, 'activity.manage_hint')}</p>
                {#if query || filter !== 'all'}<button onclick={() => { query = ''; filter = 'all'; }}>{t($locale, 'activity.clear_filters')}</button>{:else}<a class="button" href="/scheduled-actions">{t($locale, 'activity.manage')}</a>{/if}
            </div>{/each}
        </section>
    {/if}
</div>
<dialog use:ownDialog bind:this={dialog} oncancel={close} aria-labelledby="result-detail-title">
    <div class="result-section" aria-busy={detailLoading}>
        <div class="row between"><h2 id="result-detail-title">{detail?.sourceName || t($locale, 'activity.detail')}</h2><button onclick={() => close()} aria-label={t($locale, 'activity.close_detail')}>{t($locale, 'activity.close')}</button></div>
        {#if detailLoading}<p role="status">{t($locale, 'activity.loading_detail')}</p>{/if}
        {#if detailError}<p class="error" role="alert">{t($locale, detailError)} <button onclick={() => showDetail(selected)}>{t($locale, 'activity.retry')}</button></p>{/if}
        {#if detail}
            {#if executionConversationLink(detail.conversationId, detail.runId)}<a class="button" href={executionConversationLink(detail.conversationId, detail.runId)}>{t($locale, 'modelTest.openConversation')}</a>{/if}
            {#if detail.previewText || detail.llmSummaryText}<section><h2>{t($locale, 'activity.conclusion')}</h2><Markdown source={detail.previewText || detail.llmSummaryText || ''} /></section>{/if}
            <dl>
                <dt>{t($locale, 'activity.status')}</dt><dd>{resultStatus(detail.status, $locale)}{detail.expired ? ` · ${t($locale, 'activity.expired')}` : ''}</dd>
                <dt>{t($locale, 'activity.task_type')}</dt><dd>{resultFormat(detail.sourceFormat, $locale)}</dd>
                <dt>{t($locale, 'activity.source')}</dt><dd>{detail.sourceName || detail.sourceId}</dd>
                <dt>{t($locale, 'activity.location')}</dt><dd>{#if resultLink(detail.sourceUrl)}<a href={resultLink(detail.sourceUrl)} target="_blank" rel="noopener noreferrer">{detail.sourceUrl}</a>{:else}{detail.sourceUrl || '—'}{/if}</dd>
                <dt>{t($locale, 'activity.scheduled_time')}</dt><dd>{resultTime(detail.scheduledAt, $locale)}</dd><dt>{t($locale, 'activity.completed_time')}</dt><dd>{resultTime(detail.fetchedAt, $locale)}</dd>
                <dt>{t($locale, 'activity.ack_count')}</dt><dd>{detail.ackCount}</dd><dt>{t($locale, 'activity.result_id')}</dt><dd>{detail.resultId}</dd>
            </dl>
            {#if detail.failureReason}<section><h2>{t($locale, 'activity.failure')}</h2><p class="error prose">{detail.failureReason}</p></section>{/if}
            {#if detail.llmSummaryText && detail.previewText && detail.llmSummaryText !== detail.previewText}<section><h2>{t($locale, 'activity.summary')}</h2><Markdown source={detail.llmSummaryText} /></section>{/if}
            {#if detail.htmlSnapshot}
                <section class="card"><h2>{String(detail.htmlSnapshot.title || t($locale, 'activity.snapshot'))}</h2><p class="prose">{String(detail.htmlSnapshot.summary || '')}</p><small>HTTP {String(detail.htmlSnapshot.statusCode ?? '—')} · {String(detail.htmlSnapshot.contentType || '')}</small>
                    {#if resultLink(detail.htmlSnapshot.url)}<a href={resultLink(detail.htmlSnapshot.url)} target="_blank" rel="noopener noreferrer">{t($locale, 'activity.view_source')}</a>{/if}
                </section>
            {/if}
            {#each detail.events || [] as event}
                {#if event.kind === 'script_run'}<section><h2>{t($locale, 'activity.logs')}</h2><dl><dt>{t($locale, 'activity.cwd')}</dt><dd>{String(event.cwd || '—')}</dd><dt>{t($locale, 'activity.exit_code')}</dt><dd>{String(event.exitCode ?? '—')}</dd></dl>
                    {#if event.stdout}<details><summary>{t($locale, 'activity.stdout')}</summary><pre>{String(event.stdout)}</pre></details>{/if}
                    {#if event.stderr}<details><summary>{t($locale, 'activity.stderr')}</summary><pre class="error">{String(event.stderr)}</pre></details>{/if}
                </section>
                {:else}<article><strong>{String(event.title || t($locale, 'activity.event'))}</strong>{#if event.due}<p>{String(event.due)}</p>{/if}{#if resultLink(event.url)}<p><a href={resultLink(event.url)} target="_blank" rel="noopener noreferrer">{t($locale, 'activity.view_source')}</a></p>{/if}</article>{/if}
            {/each}
        {/if}
    </div>
</dialog>
<style>
    .result-section { display: grid; gap: 16px; } .result { width: 100%; text-align: start; overflow-wrap: anywhere; }
    .preview { display: -webkit-box; -webkit-line-clamp: 3; line-clamp: 3; -webkit-box-orient: vertical; overflow: hidden; }
    .prose, pre { white-space: pre-wrap; overflow-wrap: anywhere; } pre { max-height: 400px; overflow: auto; padding: 12px; background: var(--panel); border-radius: 8px; }
    dl { display: grid; grid-template-columns: auto minmax(0, 1fr); gap: 8px 16px; margin: 0; } dt { color: var(--muted); } dd { margin: 0; overflow-wrap: anywhere; }
    dialog { max-width: 760px; } dialog a { color: var(--accent); text-decoration: underline; } summary { cursor: pointer; }
</style>
