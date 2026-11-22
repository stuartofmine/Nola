<script lang="ts">
    import { onMount, untrack, tick } from 'svelte';
    import { api } from '../lib/api';
    import { locale, t } from '../lib/i18n';
    import { resultStatus } from '../lib/results';
    import { activityQueryKey, sameActivityOwner } from '../lib/activityRequest';
    import { activityErrorKey } from '../lib/activityErrors';
    import { activityPath, activitySourcePath, type ActivityCursor, type ActivityEntry, type ActivityPage, type ActivityDetail, type ActivityOwner } from '../lib/activity';
    import { workspaceDialog } from '../lib/workspacePageLocation';
    import Markdown from './Markdown.svelte';
    const ownDialog = workspaceDialog();
    let {onopen, full = false, requestedOwner}: {onopen: (path: string) => void; full?: boolean; requestedOwner?: ActivityOwner} = $props();
    let entries = $state<ActivityEntry[]>([]), cursor = $state<ActivityCursor | null>(null);
    let error = $state(''), loading = $state(false), query = $state(''), source = $state(''), needsResponse = $state(false);
    let detail = $state<ActivityDetail | null>(null), detailError = $state(''), detailLoading = $state(false);
    let retryMore = $state(false);
    let searchInput: HTMLInputElement;
    const filtered = $derived(Boolean(source || needsResponse || query.trim()));
    function clearFilters() {
        source = ''; needsResponse = false; query = '';
        searchInput.focus();
    }
    let selected = $state<ActivityOwner | null>(null);
    let dialog = $state<HTMLDialogElement>();
    let controller: AbortController | undefined, detailController: AbortController | undefined;
    let disposed = false;
    let pendingRefresh: ReturnType<typeof setTimeout> | undefined;
    let snapshotQuery: string | undefined;
    let requestedQuery: string | undefined;
    const queryKey = () => activityQueryKey(source, needsResponse, query);
    const time = (ms: number) => new Date(ms).toLocaleString($locale);
    async function load(more = false) {
        const key = queryKey();
        // A click during the search debounce must never submit an old cursor
        // with a new filter or cancel the pending replacement refresh.
        if (more && (snapshotQuery !== key || loading)) return;
        clearTimeout(pendingRefresh);
        requestedQuery = key;
        controller?.abort(); const request = new AbortController(); controller = request;
        loading = true; error = '';
        if (snapshotQuery !== key) { entries = []; cursor = null; }
        try {
            const response = await api<ActivityPage>('/api/activity', {method: 'POST', signal: request.signal,
                body: JSON.stringify({source: source || null, needs_response: needsResponse, query: query.trim(), cursor: more ? cursor : null, limit: 40})});
            if (!disposed && !request.signal.aborted && queryKey() === key) {
                entries = more ? Array.from(new Map([...entries, ...response.entries].map(entry => [entry.run_id, entry])).values()) : response.entries;
                cursor = response.next_cursor; snapshotQuery = key;
            }
        } catch (cause) { if (!disposed && !request.signal.aborted && queryKey() === key) { error = activityErrorKey(cause); retryMore = more; } }
        finally { if (!disposed && !request.signal.aborted && requestedQuery === key) loading = false; }
    }
    $effect(() => {
        const key = activityQueryKey(source, needsResponse, query);
        // Clear a different query immediately, including its pagination, while
        // preserving the last successful snapshot for same-query refreshes.
        untrack(() => {
            if (snapshotQuery !== key) { entries = []; cursor = null; }
            error = ''; loading = true;
        });
        pendingRefresh = setTimeout(() => { void untrack(() => load()); }, 200);
        return () => { clearTimeout(pendingRefresh); controller?.abort(); };
    });
    async function showDetail(owner: ActivityOwner) {
        if (!full) { onopen(activityPath(owner)); return; }
        detailController?.abort(); const request = new AbortController(); detailController = request;
        selected = owner; detail = null; detailError = ''; detailLoading = true;
        await tick();
        if (disposed || request.signal.aborted) return;
        if (dialog && !dialog.open) dialog.showModal();
        try {
            const value = await api<ActivityDetail | null>('/api/activity/detail', {method: 'POST', signal: request.signal, body: JSON.stringify(owner)});
            if (!disposed && !request.signal.aborted) {
                if (value && !sameActivityOwner(owner, value.entry)) throw new Error('Activity owner mismatch');
                detail = value; if (!value) detailError = 'activity.runUnavailable';
            }
        } catch (cause) { if (!disposed && !request.signal.aborted) detailError = activityErrorKey(cause); }
        finally { if (!disposed && !request.signal.aborted) detailLoading = false; }
    }
    function close() { detailController?.abort(); dialog?.close(); detail = null; selected = null; detailLoading = false; }
    function openSource() {
        if (!detail?.source_conversation_id) return;
        const target = activitySourcePath(detail);
        if (!target) return;
        close(); onopen(target);
    }
    onMount(() => {
        if (requestedOwner) void showDetail(requestedOwner);
        return () => { disposed = true; clearTimeout(pendingRefresh); controller?.abort(); detailController?.abort(); };
    });
</script>
<div class="activity-list" class:full>
    <p class="muted scope">{t($locale, 'activity.localScope')}</p>
    <div class="filters">
        <label>{t($locale, 'activity.source')}<select bind:value={source}>
            <option value="">{t($locale, 'activity.all')}</option><option value="conversation">{t($locale, 'activity.conversation')}</option><option value="agent">{t($locale, 'activity.agent')}</option><option value="automation">{t($locale, 'activity.plan')}</option>
        </select></label>
        <label class="attention"><input type="checkbox" bind:checked={needsResponse} />{t($locale, 'chatInspector.needsResponse')}</label>
    <label class="search">{t($locale, 'activity.search')}<input type="search" bind:this={searchInput} bind:value={query} maxlength="512" placeholder={t($locale, 'activity.canonicalSearch')} /></label>
    <button onclick={() => load()} disabled={loading}>{t($locale, 'activity.refresh')}</button>
    </div>
    {#if loading}<p role="status">{t($locale, 'shell.pageLoading')}</p>{/if}
    {#if error}<p role="alert" class="error">{t($locale, error)}</p><div><button onclick={() => load(retryMore)}>{t($locale, 'activity.retry')}</button></div>{/if}
    {#each entries as entry (entry.run_id)}
        <button class="activity-row" onclick={() => showDetail(entry)}>
            <div class="row-top">
                <strong>{entry.title || t($locale, 'chatConversations.untitled')}</strong>
                <div class="row-pills">
                    {#if entry.needs_response}<span class="status-pill attention-pill">{t($locale, 'chatInspector.needsResponse')}</span>{/if}
                    <span class="status-pill" data-status={entry.status}>{resultStatus(entry.status, $locale)}</span>
                </div>
            </div>
            <small>{t($locale, entry.source === 'automation' ? 'activity.plan' : entry.source === 'agent' ? 'activity.agent' : 'activity.conversation')} · {time(entry.created_at_ms)}</small>
        </button>
    {/each}
    {#if !loading && !error && !entries.length}
        <p class="muted" role="status">{t($locale, filtered ? 'activity.no_matches' : 'chatInspector.noActivity')}</p>
        {#if filtered}<div><button onclick={clearFilters}>{t($locale, 'activity.clear_filters')}</button></div>{/if}
    {/if}
    {#if cursor && !error}<button disabled={loading} onclick={() => load(true)}>{t($locale, 'activity.loadMore')}</button>{/if}
    {#if !full}<button onclick={() => onopen('/activity')}>{t($locale, 'chatInspector.list.activity')}</button>{/if}
</div>
{#if full}<dialog bind:this={dialog} use:ownDialog oncancel={event => {event.preventDefault(); close();}} onclick={event => { if (event.target === dialog) close(); }} onclose={() => { detailController?.abort(); selected = null; }} aria-label={t($locale, 'activity.detail')}>
    <div class="detail">
        <div class="detail-heading"><h2>{detail?.entry.title || t($locale, 'activity.detail')}</h2><button onclick={close}>{t($locale, 'activity.close')}</button></div>
        {#if detailLoading}<p role="status">{t($locale, 'activity.loading_detail')}</p>{/if}
        {#if detailError}<p role="alert" class="error">{t($locale, detailError)}</p>{#if selected}<button onclick={() => {if (selected) void showDetail(selected);}}>{t($locale, 'activity.retry')}</button>{/if}{/if}
        {#if detail}
            <div class="detail-meta">
                <span class="status-pill" data-status={detail.entry.status}>{resultStatus(detail.entry.status, $locale)}</span>
                {#if detail.entry.needs_response}<span class="status-pill attention-pill" role="status">{t($locale, 'chatInspector.needsResponse')}</span>{/if}
                <span class="muted">{time(detail.entry.created_at_ms)}</span>
            </div>
            {#if detail.result_text}<Markdown source={detail.result_text} />{/if}
            {#if detail.result_has_more}<p class="muted">{t($locale, 'activity.moreInSource')}</p>{/if}
            {#if detail.explanation}<p class="explanation">{detail.explanation}</p>{/if}
            <details class="disclosure">
                <summary>{t($locale, 'activity.execution')}</summary>
                <div class="execution-details">
                    <p class="scope">{t($locale, 'activity.executionBoundary')}</p>
                    <dl class="execution-facts">
                        <div><dt>{t($locale, 'activity.executionHost')}</dt><dd title={detail.execution_host_id || undefined}>{detail.execution_host_id ? detail.execution_host_id.slice(0, 8) : t($locale, 'activity.hostUnknown')}</dd></div>
                        <div><dt>{t($locale, 'activity.contextScope')}</dt><dd>{t($locale, detail.context_scope === 'task' ? 'activity.context.task' : detail.context_scope === 'conversation' ? 'activity.context.conversation' : 'activity.context.unavailable')}</dd></div>
                    </dl>
                </div>
            </details>
            {#if detail.source_conversation_id}<button onclick={openSource}>{t($locale, 'activity.openSource')}</button>{:else}<p>{t($locale, 'activity.sourceUnavailable')}</p>{/if}
        {/if}
    </div>
</dialog>{/if}
<style>
    .execution-facts { display: grid; gap: 16px; }
    .execution-facts dt { color: var(--muted); font-size: 12px; margin-bottom: 6px; }
    .execution-facts dd { margin: 0; overflow-wrap: anywhere; }
    .activity-list { display: grid; gap: 8px; min-width: 0; }
    .full { width: 100%; max-width: 960px; margin: 0 auto; }
    .filters { display: flex; flex-wrap: wrap; gap: 12px; align-items: end; }
    label { display: grid; gap: 6px; }
    .search { flex: 1; min-width: min(220px, 100%); }
    .filters > button { flex: none; }
    .attention { align-self: center; }
    .attention { display: flex; align-items: center; gap: 8px; }
    .attention input { width: auto; }
    .activity-row { display: grid; gap: 6px; text-align: start; border: 0; border-bottom: 1px solid var(--border); border-radius: 0; background: transparent; box-shadow: none; padding: 12px 8px; }
    .activity-row:hover { background: var(--panel); }
    .row-top { display: flex; align-items: flex-start; justify-content: space-between; gap: 10px; min-width: 0; }
    .row-pills, .detail-meta { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; flex-shrink: 0; }
    .status-pill { display: inline-flex; align-items: center; padding: 2px 8px; border-radius: 999px; font-size: 11.5px; font-weight: 500; background: var(--panel); color: var(--muted); white-space: nowrap; }
    .status-pill[data-status="completed"], .status-pill[data-status="answered"], .status-pill[data-status="succeeded"] { color: var(--success); background: color-mix(in srgb, var(--success) 12%, transparent); }
    .status-pill[data-status="failed"], .status-pill[data-status="error"] { color: var(--danger); background: color-mix(in srgb, var(--danger) 12%, transparent); }
    .status-pill[data-status="running"], .status-pill[data-status="pending"] { color: var(--accent); background: var(--accent-subtle); }
    .attention-pill { color: var(--warning); background: color-mix(in srgb, var(--warning) 14%, transparent); font-weight: 600; }
    .execution-details { display: grid; gap: 12px; padding-top: 10px; }
    .activity-row strong, .activity-row span, .activity-row small { min-width: 0; overflow-wrap: anywhere; }
    .activity-row strong { flex: 1; display: -webkit-box; line-clamp: 2; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
    .activity-row small, .scope { color: var(--muted); font-size: 12px; }
    .detail { display: grid; gap: 16px; }
    .detail-heading { display: flex; align-items: start; justify-content: space-between; gap: 12px; }
    .detail-heading h2 { min-width: 0; margin: 0; overflow-wrap: anywhere; }
    .detail-heading > button { flex: none; white-space: nowrap; }
    .explanation { white-space: pre-wrap; overflow-wrap: anywhere; }
    dialog { width: min(760px, calc(100vw - 24px)); }
</style>
