<script lang="ts">
    import type { ChatConversation } from '../lib/chatConversations';
    import { committedContextOverview } from '../lib/chatOverview';
    import type { CommittedRunContextUsage } from '../lib/runContextUsage';
    import type { ConversationCacheUsage } from '../lib/conversationCacheUsage';
    import { locale, t } from '../lib/i18n';

    let { compact = false, conversation, busy = false, selectedModelName = '', usage = null, cache = null, cacheFailed = false }: {
        compact?: boolean; conversation: ChatConversation; busy?: boolean; selectedModelName?: string; usage?: CommittedRunContextUsage | null;
        cache?: ConversationCacheUsage; cacheFailed?: boolean;
    } = $props();
    const overview = $derived(committedContextOverview(usage));
    const coverage = $derived(cache && cache.reported_requests + cache.unreported_requests > 0
        ? cache.reported_requests / (cache.reported_requests + cache.unreported_requests) : null);
    const modelLabel = $derived(conversation.selectedModelSource === 'auto' || !conversation.selectedModelId
        ? t($locale, 'chatOverview.automatic') : selectedModelName || '—');

</script>

<section class="overview" aria-label={t($locale, 'chatOverview.title')}>
    {#if !compact}<h2>{t($locale, 'chatOverview.title')}</h2>
    <div class="status"><span class:running={busy} class="dot"></span><span>{t($locale, busy ? 'chatOverview.running' : 'chatOverview.ready')}</span><span class="host">{t($locale, 'chatOverview.localHost')}</span></div>{/if}
    {#if !compact}<div class="model">{conversation.executionBackend === 'nola' ? modelLabel : conversation.executionBackend}</div>{/if}
    <div class="context">
        <div class="context-head"><strong>{t($locale, 'chatOverview.context')}</strong><span>{overview
            ? `${BigInt(overview.usedTokens).toLocaleString($locale)} / ${BigInt(overview.contextWindowTokens).toLocaleString($locale)}` : '—'}</span></div>
        {#if overview?.fraction !== null && overview?.fraction !== undefined}<progress value={overview.fraction} max="1" aria-label={t($locale, 'chatOverview.context')}></progress>{/if}
        <p class="muted">{t($locale, conversation.executionBackend !== 'nola' ? 'chatOverview.externalUnit' : overview ? 'chatOverview.lastRun' : 'chatOverview.notRecorded')}{#if overview?.estimated} · {t($locale, 'chatOverview.estimated')}{/if}</p>
    </div>
    <div class="context">
        <div class="context-head"><strong>{t($locale, 'chatOverview.cacheRate')}</strong><span>{cache?.hit_rate_bps != null
            ? new Intl.NumberFormat($locale, { style: 'percent', minimumFractionDigits: 1, maximumFractionDigits: 1 }).format(cache.hit_rate_bps / 10000)
            : t($locale, cacheFailed ? 'chatOverview.cacheUnavailable' : 'chatOverview.cacheUnknown')}</span></div>
        {#if cache?.read_tokens != null && cache.input_tokens != null}<p class="muted">{cache.read_tokens.toLocaleString($locale)} / {cache.input_tokens.toLocaleString($locale)} tokens</p>{/if}
        {#if cache}<p class="muted">{t($locale, 'chatOverview.cacheReported')}: {cache.reported_requests.toLocaleString($locale)} · {t($locale, 'chatOverview.cacheUnreported')}: {cache.unreported_requests.toLocaleString($locale)}</p>{/if}
        {#if coverage !== null}<p class="muted">{t($locale, 'chatOverview.cacheCoverage')}: {new Intl.NumberFormat($locale, { style: 'percent', maximumFractionDigits: 1 }).format(coverage)}</p>{/if}
    </div>
    {#if usage?.usage.prompt_cache_measurement && usage.usage.prompt_cache_diagnostic}
        <details class="context">
            <summary>{t($locale, 'chatOverview.cacheDiagnostic')}</summary>
            <p class="muted">{t($locale, 'chatOverview.request')}: {usage.usage.prompt_cache_measurement.request_id}</p>
            {#if usage.usage.prompt_cache_measurement.attempt_id}<p class="muted">{t($locale, 'chatOverview.attempt')}: {usage.usage.prompt_cache_measurement.attempt_id}</p>{/if}
            <p class="muted">{usage.usage.prompt_cache_diagnostic.status}</p>
            {#each usage.usage.prompt_cache_diagnostic.reasons as reason}<p class="muted">{reason}</p>{/each}
            {#each usage.usage.prompt_cache_diagnostic.fragmentChanges ?? [] as change}<p class="muted">{change.fragmentId}: {change.kind}</p>{/each}
        </details>
    {/if}
</section>

<style>
    .overview { display: grid; gap: 12px; min-width: 0; padding: 0; }
    h2,p { margin: 0; } h2 { font-size: 13px; font-weight: 650; }
    .status { display: flex; align-items: center; gap: 7px; min-width: 0; color: var(--muted); font-size: .8125rem; }
    .host { margin-inline-start: auto; white-space: nowrap; }
    .dot { width: 6px; height: 6px; flex: none; border-radius: 50%; background: var(--muted); }
    .dot.running { background: var(--accent); }
    .model { font-size: 15px; font-weight: 600; overflow-wrap: anywhere; }
    .context { display: grid; gap: 5px; padding-top: 10px; border-top: 1px solid var(--border); }
    .context-head { display: flex; justify-content: space-between; align-items: center; gap: 8px; font-size: .8125rem; }
    .context-head span { color: var(--accent); font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: .8125rem; }
    progress { width: 100%; height: 5px; accent-color: var(--accent); }
    .muted { color: var(--muted); font-size: .8125rem; line-height: 1.6; overflow-wrap: anywhere; }
</style>
