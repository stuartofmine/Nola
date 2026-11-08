<script lang="ts">
    import { onMount, untrack } from 'svelte';
    import { api } from '../lib/api';
    import { locale, t } from '../lib/i18n';
    import { confirmedConversationDataSource, conversationDataSourceUrl, conversationDataSourcesUrl, parseConversationDataSources } from '../lib/chatDataSources';
    import type { ConversationDataSource } from '../lib/chatDataSources';

    let { conversationId, generation, busy = false }: { conversationId: string; generation: number; busy?: boolean } = $props();
    let sources = $state<ConversationDataSource[]>([]);
    let loading = $state(false);
    let changingId = $state<string | null>(null);
    let error = $state('');
    let mounted = false;
    let disposed = false;
    let request = 0;
    let controller: AbortController | null = null;
    const current = (id: string, epoch: number) => !disposed && conversationId === id && generation === epoch;

    async function load() {
        const id = conversationId, epoch = generation, sequence = ++request;
        controller?.abort(); const next = new AbortController(); controller = next;
        loading = true; error = '';
        try {
            const rows = parseConversationDataSources(await api(conversationDataSourcesUrl(id), { signal: next.signal }), id);
            if (current(id, epoch) && sequence === request) sources = rows;
        } catch {
            if (current(id, epoch) && sequence === request && !next.signal.aborted) error = 'chatDataSources.loadFailed';
        } finally {
            if (current(id, epoch) && sequence === request) loading = false;
        }
    }
    $effect(() => {
        conversationId; generation;
        if (mounted) untrack(() => { sources = []; changingId = null; error = ''; void load(); });
    });
    onMount(() => { mounted = true; void load(); return () => { disposed = true; controller?.abort(); }; });

    async function change(sourceId: string, event: Event) {
        const select = event.currentTarget as HTMLSelectElement;
        const row = sources.find(item => item.source.id === sourceId);
        const requested = select.value === 'inherit' ? null : select.value === 'on' ? true : select.value === 'off' ? false : undefined;
        select.value = row?.conversationOverride === null ? 'inherit' : row?.conversationOverride ? 'on' : 'off';
        if (!row || requested === undefined || row.source.accessPolicy === 'blocked' || busy || changingId || loading || requested === row.conversationOverride) return;
        const id = conversationId, epoch = generation;
        changingId = sourceId; error = '';
        try {
            const receipt = confirmedConversationDataSource(await api(conversationDataSourceUrl(id, sourceId), {
                method: 'PATCH', body: JSON.stringify({ enabled: requested }),
            }), sourceId, requested);
            if (current(id, epoch)) sources = sources.map(item => item.source.id === sourceId ? receipt : item);
        } catch {
            if (current(id, epoch)) { await load(); if (current(id, epoch)) error = 'chatDataSources.saveFailed'; }
        } finally {
            if (current(id, epoch)) changingId = null;
        }
    }
</script>

<section class="conversation-sources" aria-label={t($locale, 'chatDataSources.title')}>
    <div class="heading"><h2>{t($locale, 'chatDataSources.title')}</h2><button type="button" onclick={load} disabled={loading || !!changingId}>{t($locale, 'chatDataSources.refresh')}</button></div>
    <p class="muted">{t($locale, 'chatDataSources.scope')}</p>
    {#if error}<p class="error" role="alert">{t($locale, error)}</p>{/if}
    {#if loading}<p class="muted" role="status">{t($locale, 'chatDataSources.loading')}</p>{/if}
    {#each sources as row (row.source.id)}
        <div class="source">
            <div class="identity"><strong>{row.source.name}</strong><small>{t($locale, `chatDataSources.${row.resolution.reason}`)}</small></div>
            <label><span class="sr-only">{row.source.name}</span>
                <select value={row.conversationOverride === null ? 'inherit' : row.conversationOverride ? 'on' : 'off'} onchange={event => { void change(row.source.id, event); }} disabled={busy || !!changingId || loading || row.source.accessPolicy === 'blocked'}>
                    <option value="inherit">{t($locale, 'chatDataSources.inherit')}</option>
                    <option value="on">{t($locale, 'chatDataSources.enable')}</option>
                    <option value="off">{t($locale, 'chatDataSources.disable')}</option>
                </select>
            </label>
            {#if changingId === row.source.id}<small role="status">{t($locale, 'chatDataSources.saving')}</small>{/if}
        </div>
    {:else}{#if !loading && !error}<p class="muted">{t($locale, 'chatDataSources.empty')}</p>{/if}{/each}
    <a href="/data-sources">{t($locale, 'chatDataSources.manage')}</a>
</section>

<style>
    .conversation-sources { display: grid; gap: 8px; min-width: 0; padding: 12px; border: 1px solid var(--border); border-radius: 12px; background: var(--canvas); }
    .heading, .source { display: flex; align-items: center; justify-content: space-between; gap: 8px; min-width: 0; }
    h2 { margin: 0; font-size: 13px; } .heading button, select { font-size: 11px; }
    .source { flex-wrap: wrap; padding: 8px 0; border-top: 1px solid var(--border); }
    .identity { display: grid; min-width: 0; gap: 2px; overflow-wrap: anywhere; font-size: 12px; }
    .identity small { color: var(--muted); font-size: 11px; }
    .source select { max-width: 100%; }
    .muted, .error, a { margin: 0; font-size: 11px; line-height: 1.4; }
    .muted { color: var(--muted); } .error { color: var(--danger); }
    .sr-only { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap; }
</style>
