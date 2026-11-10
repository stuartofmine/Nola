<script lang="ts">
    import { onDestroy, untrack } from 'svelte';
    import { api } from '../lib/api';
    import { locale, t } from '../lib/i18n';
    import { recallHits, recallUrl } from '../lib/chatRecall';
    import type { RecallHit } from '../lib/chatRecall';
    let {conversationId, generation}: {conversationId: string; generation: number} = $props();
    let query = $state(''), hits = $state<RecallHit[]>([]);
    let loading = $state(false), failed = $state(false), loaded = $state(false);
    let sequence = 0, disposed = false;
    let timer: ReturnType<typeof setTimeout> | undefined, reader: AbortController | undefined;
    function reset() { ++sequence; clearTimeout(timer); reader?.abort(); hits = []; loading = false; failed = false; loaded = false; }
    $effect(() => { conversationId; generation; untrack(() => { reset(); query = ''; }); });
    onDestroy(() => { disposed = true; reset(); });
    function changed(value: string) {
        reset(); query = value;
        if (value.trim()) { loading = true; timer = setTimeout(search, 180); }
    }
    async function search() {
        clearTimeout(timer); reader?.abort();
        const text = query.trim(), id = conversationId, epoch = generation, token = ++sequence;
        if (!text) { reset(); return; }
        const controller = new AbortController(); reader = controller;
        loading = true; failed = false; loaded = false; hits = [];
        const current = () => !disposed && id === conversationId && epoch === generation && token === sequence && query.trim() === text;
        try {
            const result = recallHits(await api(recallUrl(id, text), {signal: controller.signal}), id, text);
            if (current()) { hits = result; loaded = true; }
        } catch { if (current()) failed = true; }
        finally { if (current()) loading = false; }
    }
</script>
<section class="conversation-recall" aria-busy={loading}>
    <h3>{t($locale, 'chatKnowledge.recall')}</h3>
    <p>{t($locale, 'chatKnowledge.recallHint')}</p>
    <form onsubmit={event => {event.preventDefault(); void search();}}>
        <label>{t($locale, 'chatKnowledge.search')}<input type="search" value={query} oninput={event => changed(event.currentTarget.value)} maxlength="500" /></label>
        <button type="submit" disabled={!query.trim()}>{t($locale, 'chatKnowledge.search')}</button>
    </form>
    {#if loading}<p role="status">{t($locale, 'chatKnowledge.searching')}</p>{/if}
    {#if failed}<p role="alert">{t($locale, 'chatKnowledge.searchFailed')}</p>{/if}
    {#if loaded && !hits.length}<p role="status">{t($locale, 'chatKnowledge.noMatches')}</p>{/if}
    <ol>{#each hits as hit (JSON.stringify([hit.message_id, hit.part_id, hit.start_character]))}<li><small>#{hit.message_ordinal} · {hit.role}</small><p class="snippet">{hit.snippet}</p></li>{/each}</ol>
</section>
<style>
    section, form, label { display: grid; gap: 10px; min-width: 0; }
    section { border: 1px solid var(--border); border-radius: 10px; padding: 12px; }
    h3, p { margin: 0; } h3 { font-size: 13px; } p, label, small { font-size: 12px; }
    p, small { color: var(--muted); } .snippet { color: inherit; white-space: pre-wrap; overflow-wrap: anywhere; }
    ol { list-style: none; margin: 0; padding: 0; display: grid; gap: 12px; } li { border-top: 1px solid var(--border); padding-top: 10px; }
    button { min-height: 44px; } [role="alert"] { color: var(--danger); }
</style>
