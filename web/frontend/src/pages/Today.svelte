<script lang="ts">
    import { onMount } from 'svelte';
    import { api } from '../lib/api';
    import { parseInitializationSession } from '../lib/chatInitialization';
    import { parseTodayFacts, type TodayFacts } from '../lib/todayFacts';
    import { locale, t } from '../lib/i18n';
    const pageOffset = Number(new URL(window.location.href).searchParams.get('offset') ?? 0);
    const offset = Number.isSafeInteger(pageOffset) && pageOffset >= 0 && pageOffset <= 1_000_000 ? pageOffset : 0;
    let facts = $state<TodayFacts | null>(null);
    let host = $state('');
    let loading = $state(true);
    let failed = $state(false);
    let request: AbortController | null = null;
    let disposed = false;
    const pending = $derived(facts?.conversations.filter(row => row.needs_response) ?? []);
    const recent = $derived(facts?.recent_conversations ?? []);
    const conversationHref = (id: string) => `/chat?conversation=${encodeURIComponent(id)}&sourceHost=${encodeURIComponent(host)}`;
    const time = (ms: number) => new Intl.DateTimeFormat($locale, {dateStyle: 'medium', timeStyle: 'short'}).format(ms);
    async function refresh() {
        request?.abort(); const controller = new AbortController(); request = controller;
        loading = true; failed = false;
        try {
            const session = parseInitializationSession(await api('/api/web/session', {signal: controller.signal}));
            const snapshot = parseTodayFacts(await api(`/api/workspace/today?offset=${offset}`, {signal: controller.signal}), session.hostId, offset);
            if (controller.signal.aborted || disposed) return;
            host = session.hostId; facts = snapshot;
        } catch { if (!controller.signal.aborted && !disposed) { failed = true; facts = null; } }
        finally { if (!controller.signal.aborted && !disposed) loading = false; }
    }
    onMount(() => { void refresh(); return () => { disposed = true; request?.abort(); }; });
</script>
<svelte:head><title>{t($locale, 'chatInspector.today')} — Nola</title></svelte:head>
<main id="page-main" tabindex="-1" class="today-page">
    <header><div><h1>{t($locale, 'chatInspector.today')}</h1><p class="today-hint">{t($locale, 'shell.todayHint')}</p></div><button type="button" aria-label={t($locale, 'chatPage.refresh')} title={t($locale, 'chatPage.refresh')} onclick={refresh} disabled={loading}><svg aria-hidden="true" viewBox="0 0 24 24"><path d="M20 7v5h-5M4 17v-5h5M5 8a8 8 0 0 1 14-2l1 6M4 12l1 6a8 8 0 0 0 14-2"/></svg></button></header>
    {#if loading}<p role="status" class="muted">{t($locale, 'chatPage.loading')}</p>{/if}
    {#if failed}<p role="alert">{t($locale, 'shell.pageLoadFailed')}</p>{/if}
    {#if facts}
        {#if facts.needs_attention}<aside role="status"><p>{t($locale, 'chatInspector.expertAttention')}</p><ul>{#each facts.unrepresented_expert_contexts as context (context.context_conversation_id)}<li>{context.display_name}</li>{/each}</ul></aside>{/if}
        <section><h2>{t($locale, 'chatInspector.needsResponse')}</h2>
            {#each pending as row (row.conversation_id)}<article class="pending-matter"><p class="matter-meta">{time(row.updated_at_ms)} · {t($locale, 'chatInspector.needsResponse')}</p><h3>{row.title || t($locale, 'chatConversations.untitled')}</h3>
                <p class="matter-summary">{t($locale, 'chatInspector.responseHint')}</p>
                <a class="matter-action" href={conversationHref(row.conversation_id)}>{t($locale, 'chatInspector.handleResponse')} <span aria-hidden="true">›</span></a>
                <a class="matter-source" href={conversationHref(row.conversation_id)}><svg aria-hidden="true" viewBox="0 0 24 24"><path d="M20 11a8 8 0 0 1-8 8H5l-3 3V11a9 9 0 0 1 18 0Z"/></svg><span>{t($locale, 'chatInspector.responseSource')} · {row.title || t($locale, 'chatConversations.untitled')}</span></a>
            </article>
            {:else}{#if !facts.needs_attention}<p class="muted">{t($locale, 'chatInspector.noResponse')}</p>{/if}{/each}
            {#if facts.has_more_conversations}<a href={`/today?offset=${facts.next_conversation_offset}`}>{t($locale, 'chatInspector.moreConversations')}</a>{/if}
        </section>
        <section><h2>{t($locale, 'chatInspector.queuedInstances')}</h2>
            {#each facts.queued_instances as row (row.occurrence_id)}<a class="fact-row" href="/scheduled-actions"><span><strong>{row.title ?? row.automation_id}</strong><small>{time(row.scheduled_for_ms)}</small></span><span aria-hidden="true">›</span></a>
            {:else}<p class="muted">{t($locale, 'chatInspector.noQueuedInstances')}</p>{/each}
            {#if facts.has_more_queued_instances}<a href="/scheduled-actions">{t($locale, 'activity.automations')}</a>{/if}
        </section>
        {#if offset === 0}<section><h2>{t($locale, 'chatConversations.recent')}</h2>
            {#each recent as row (row.conversation_id)}<a class="fact-row" href={conversationHref(row.conversation_id)}><span><strong>{row.title || t($locale, 'chatConversations.untitled')}</strong><small>{time(row.updated_at_ms)}</small></span><span aria-hidden="true">›</span></a>
            {:else}<p class="muted">{t($locale, 'chatPage.choose')}</p>{/each}
        </section>{/if}
    {/if}
    {#if offset > 0}<a href="/today">{t($locale, 'chatInspector.today')}</a>{/if}
    <nav aria-label={t($locale, 'shell.workspace')}><a href="/conversations">{t($locale, 'chatInspector.list.conversations')}</a><a href="/data-sources">{t($locale, 'chatInspector.list.resources')}</a><a href="/settings">{t($locale, 'shell.settings')}</a></nav>
</main>
<style>
    .today-page { max-width: 760px; min-height: calc(100dvh - 144px); margin-inline: auto; padding: 8px 20px max(20px, env(safe-area-inset-bottom)); background: var(--canvas); }
    header { display: flex; align-items: center; justify-content: space-between; gap: 16px; min-height: 52px; }
    h1 { font-size: 28px; line-height:36px; margin: 0; } h2 { font-size: 16px; margin: 0 0 12px; }
    section { margin-block: 32px; } .today-hint { font-size:15px; line-height:24px; color:var(--muted); margin-top:6px; } header button { border:0; background:transparent; width:44px; padding:10px; } header svg { width:20px; height:20px; stroke:currentColor; fill:none; stroke-width:1.5; } .muted, small { color: var(--muted); font-size: .875rem; line-height: 1.6; }
    .fact-row { display: flex; align-items: center; justify-content: space-between; gap: 16px; padding-block: 12px; border-bottom: 1px solid var(--border); min-height: 52px; }
    .fact-row > span { min-width: 0; } strong { font-weight: 500; overflow-wrap: anywhere; } small { display: block; margin-top: 4px; }
    a { color: inherit; text-decoration: none; } a:hover { color: var(--accent); } nav { display: flex; flex-wrap: wrap; gap: 16px; border-top: 1px solid var(--border); margin-top: 32px; } nav a { display: flex; align-items: center; min-height: 48px; }
    button { min-height: 44px; }
    @media(max-width:359px) { .today-page { padding-inline:16px; } }
    @media (max-width: 760px) { nav { display: none; } }
    .pending-matter { padding:22px 0; border-bottom:1px solid var(--border); }
    .matter-meta { color:var(--muted); font-size:12px; line-height:20px; margin:0 0 8px; }
    .pending-matter h3 { font-size:21px; line-height:31px; font-weight:550; letter-spacing:-.4px; margin:0; overflow-wrap:anywhere; }
    .matter-summary { margin:10px 0 0; font-size:15px; line-height:25px; color:var(--muted); }
    .matter-source { display:flex; align-items:center; gap:7px; min-height:44px; margin-top:4px; font-size:12px; line-height:19px; color:var(--muted); }
    .matter-source svg { width:16px; height:16px; flex-shrink:0; fill:none; stroke:currentColor; stroke-width:1.5; }
    .matter-source span { overflow-wrap:anywhere; }
    .matter-action { display:inline-flex; align-items:center; gap:12px; min-height:44px; margin-top:14px; padding:8px 14px; border-radius:8px; background:var(--primary); color:var(--on-primary); font-size:14px; }
    .matter-action:hover { color:var(--on-primary); background:var(--primary); }
</style>
