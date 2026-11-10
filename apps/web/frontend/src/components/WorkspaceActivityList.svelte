<script lang="ts">
    import { onMount, untrack, tick } from 'svelte';
    import { api } from '../lib/api';
    import { locale, t } from '../lib/i18n';
    import { resultStatus } from '../lib/results';
    import { activityErrorKey } from '../lib/activityErrors';
    import { activityPath, activitySourcePath, type ActivityCursor, type ActivityEntry, type ActivityPage, type ActivityDetail, type ActivityOwner } from '../lib/activity';
    import { workspaceDialog } from '../lib/workspacePageLocation';
    import Markdown from './Markdown.svelte';
    const ownDialog = workspaceDialog();
    let {onopen, full = false, requestedOwner}: {onopen: (path: string) => void; full?: boolean; requestedOwner?: ActivityOwner} = $props();
    let entries = $state<ActivityEntry[]>([]), cursor = $state<ActivityCursor | null>(null);
    let error = $state(''), loading = $state(false), query = $state(''), source = $state(''), needsResponse = $state(false);
    let detail = $state<ActivityDetail | null>(null), detailError = $state(''), detailLoading = $state(false);
    let selected = $state<ActivityOwner | null>(null);
    let dialog = $state<HTMLDialogElement>();
    let controller: AbortController | undefined, detailController: AbortController | undefined;
    let disposed = false;
    const time = (ms: number) => new Date(ms).toLocaleString($locale);
    async function load(more = false) {
        controller?.abort(); const request = new AbortController(); controller = request;
        loading = true; error = '';
        if (!more) { entries = []; cursor = null; }
        try {
            const response = await api<ActivityPage>('/api/activity', {method: 'POST', signal: request.signal,
                body: JSON.stringify({source: source || null, needs_response: needsResponse, query: query.trim(), cursor: more ? cursor : null, limit: 40})});
            if (!disposed && !request.signal.aborted) { entries = more ? [...entries, ...response.entries] : response.entries; cursor = response.next_cursor; }
        } catch (cause) { if (!disposed && !request.signal.aborted) error = activityErrorKey(cause); }
        finally { if (!disposed && !request.signal.aborted) loading = false; }
    }
    $effect(() => {
        query; source; needsResponse;
        const timer = setTimeout(() => { void untrack(() => load()); }, 200);
        return () => { clearTimeout(timer); controller?.abort(); };
    });
    async function showDetail(owner: ActivityOwner) {
        if (!full) { onopen(activityPath(owner)); return; }
        detailController?.abort(); const request = new AbortController(); detailController = request;
        selected = owner; detail = null; detailError = ''; detailLoading = true;
        await tick(); if (!disposed && dialog && !dialog.open) dialog.showModal();
        try {
            const value = await api<ActivityDetail | null>('/api/activity/detail', {method: 'POST', signal: request.signal, body: JSON.stringify(owner)});
            if (!disposed && !request.signal.aborted) { detail = value; if (!value) detailError = 'activity.runUnavailable'; }
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
        return () => { disposed = true; controller?.abort(); detailController?.abort(); };
    });
</script>
<div class="activity-list" class:full>
    <p class="muted scope">{t($locale, 'activity.localScope')}</p>
    <div class="filters">
        <label>{t($locale, 'activity.source')}<select bind:value={source}>
            <option value="">{t($locale, 'activity.all')}</option><option value="agent">{t($locale, 'activity.agent')}</option><option value="automation">{t($locale, 'activity.plan')}</option>
        </select></label>
        <label class="attention"><input type="checkbox" bind:checked={needsResponse} />{t($locale, 'chatInspector.needsResponse')}</label>
    <label class="search">{t($locale, 'activity.search')}<input type="search" bind:value={query} maxlength="512" placeholder={t($locale, 'activity.canonicalSearch')} /></label>
    <button onclick={() => load()} disabled={loading}>{t($locale, 'activity.refresh')}</button>
    </div>
    {#if loading}<p role="status">{t($locale, 'shell.pageLoading')}</p>{/if}
    {#if error}<p role="alert">{t($locale, error)}</p>{/if}
    {#each entries as entry (entry.run_id)}
        <button class="activity-row" onclick={() => showDetail(entry)}>
            <strong>{entry.title || t($locale, 'chatConversations.untitled')}</strong>
            <small>{t($locale, entry.source === 'automation' ? 'activity.plan' : entry.source === 'agent' ? 'activity.agent' : 'activity.conversation')} · {resultStatus(entry.status, $locale)} · {time(entry.created_at_ms)}</small>
            {#if entry.needs_response}<span>{t($locale, 'chatInspector.needsResponse')}</span>{/if}
        </button>
    {/each}
    {#if !loading && !error && !entries.length}<p class="muted">{t($locale, needsResponse ? 'chatInspector.noResponse' : 'chatInspector.noActivity')}</p>{/if}
    {#if cursor}<button disabled={loading} onclick={() => load(true)}>{t($locale, 'activity.loadMore')}</button>{/if}
    {#if !full}<button onclick={() => onopen('/activity')}>{t($locale, 'chatInspector.list.activity')}</button>{/if}
</div>
{#if full}<dialog bind:this={dialog} use:ownDialog oncancel={event => {event.preventDefault(); close();}} onclose={() => { detailController?.abort(); selected = null; }} aria-label={t($locale, 'activity.detail')}>
    <div class="detail">
        <div class="detail-heading"><h2>{detail?.entry.title || t($locale, 'activity.detail')}</h2><button onclick={close}>{t($locale, 'activity.close')}</button></div>
        {#if detailLoading}<p role="status">{t($locale, 'activity.loading_detail')}</p>{/if}
        {#if detailError}<p role="alert">{t($locale, detailError)}</p>{#if selected}<button onclick={() => {if (selected) void showDetail(selected);}}>{t($locale, 'activity.retry')}</button>{/if}{/if}
        {#if detail}
            <section aria-label={t($locale, 'activity.execution')}>
                <h3>{t($locale, 'activity.execution')}</h3>
                <p>{resultStatus(detail.entry.status, $locale)} · {time(detail.entry.created_at_ms)}</p>
                <p class="scope">{t($locale, 'activity.executionBoundary')}</p>
            </section>
            <dl class="execution-facts">
                <div><dt>{t($locale, 'activity.executionHost')}</dt><dd>{detail.execution_host_id || t($locale, 'activity.hostUnknown')}</dd></div>
                <div><dt>{t($locale, 'activity.contextScope')}</dt><dd>{t($locale, detail.context_scope === 'task' ? 'activity.context.task' : detail.context_scope === 'conversation' ? 'activity.context.conversation' : 'activity.context.unavailable')}</dd></div>
            </dl>
            {#if detail.entry.needs_response}<p role="status">{t($locale, 'chatInspector.needsResponse')}</p>{/if}
            {#if detail.result_text}<Markdown source={detail.result_text} />{/if}
            {#if detail.result_has_more}<p class="muted">{t($locale, 'activity.moreInSource')}</p>{/if}
            {#if detail.explanation}<p class="explanation">{detail.explanation}</p>{/if}
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
    .activity-row { display: grid; gap: 6px; text-align: start; border-color: transparent; background: transparent; padding: 12px; }
    .activity-row strong, .activity-row span, .activity-row small { min-width: 0; overflow-wrap: anywhere; }
    .activity-row strong { display: -webkit-box; line-clamp: 2; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
    .activity-row small, .scope { color: var(--muted); font-size: 12px; }
    .detail { display: grid; gap: 16px; }
    .detail-heading { display: flex; align-items: start; justify-content: space-between; gap: 12px; }
    .detail-heading h2 { min-width: 0; margin: 0; overflow-wrap: anywhere; }
    .detail-heading > button { flex: none; white-space: nowrap; }
    .explanation { white-space: pre-wrap; overflow-wrap: anywhere; }
    dialog { width: min(760px, calc(100vw - 24px)); }
</style>
