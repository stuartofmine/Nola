<script lang="ts">
    import { onMount } from 'svelte';
    import { api } from '../lib/api';
    import { parseInitializationSession } from '../lib/chatInitialization';
    import { workspacePageLocation } from '../lib/workspacePageLocation';
    import { parseWorkspaceReview, type WorkspaceReview } from '../lib/workspaceReview';
    import { locale, t } from '../lib/i18n';
    let {conversationId, hostId, onopen}: {conversationId?: string; hostId?: string; onopen?: () => void} = $props();
    const location = workspacePageLocation();
    const selected = $derived(conversationId ?? location.read().searchParams.get('conversation') ?? '');
    let review = $state<WorkspaceReview | null>(null), loading = $state(false), failed = $state(false);
    let mounted = $state(false);
    let request: AbortController | null = null;
    let sequence = 0;
    async function load() {
        request?.abort(); const controller = new AbortController(); request = controller;
        const id = selected, host = hostId, epoch = ++sequence;
        const current = () => mounted && !controller.signal.aborted && epoch === sequence && id === selected && host === hostId;
        review = null; loading = true; failed = false;
        try {
            if (!id) throw new Error('review.owner');
            const ownerHost = host || parseInitializationSession(await api('/api/web/session', {signal: controller.signal})).hostId;
            const result = parseWorkspaceReview(await api(`/api/chat/conversations/${encodeURIComponent(id)}/workspace-review`, {signal: controller.signal}), ownerHost, id);
            if (current()) review = result;
        } catch { if (current()) failed = true; }
        finally { if (current()) loading = false; }
    }
    onMount(() => { mounted = true; return () => { mounted = false; ++sequence; request?.abort(); }; });
    $effect(() => { selected; hostId; if (mounted) void load(); });
</script>
<section class="workspace-review" aria-label={t($locale, 'chatInspector.review')}>
    <header><h2>{t($locale, 'chatInspector.review')}</h2><button type="button" onclick={load} disabled={loading}>{t($locale, 'chatPage.refresh')}</button></header>
    {#if onopen}<button type="button" onclick={onopen}>{t($locale, 'chatResource.openTab')}</button>{/if}
    {#if loading}<p role="status">{t($locale, 'chatPage.loading')}</p>{/if}
    {#if failed}<p role="alert">{t($locale, 'chatInspector.reviewUnavailable')}</p>{/if}
    {#if review}
        <p class="workspace-root">{t($locale, 'chatInspector.hostWorkspace')}<code>{review.root}</code></p>
        {#if review.entries.length}<ul>{#each review.entries as entry}<li><code>{entry.index}{entry.worktree}</code><span>{entry.path}</span></li>{/each}</ul>
        {:else}<p>{t($locale, 'chatInspector.noChanges')}</p>{/if}
        {#if review.truncated}<p role="status">{t($locale, 'chatInspector.reviewTruncated')}</p>{/if}
        {#each [{label: 'unstaged', value: review.unstaged}, {label: 'staged', value: review.staged}] as section}
            <details open={!!section.value.text}><summary>{t($locale, `chatInspector.${section.label}`)}</summary>
                {#if section.value.failed}<p role="alert">{t($locale, 'chatInspector.diffUnavailable')}</p>
                {:else if section.value.text}
                    <!-- svelte-ignore a11y_no_noninteractive_tabindex (Scrollable diff supports keyboard reading.) -->
                    <pre tabindex="0" role="region" aria-label={t($locale, `chatInspector.${section.label}`)}>{section.value.text}</pre>
                {:else}<p>{t($locale, 'chatInspector.noChanges')}</p>{/if}
            </details>
        {/each}
    {/if}
</section>
<style>
    .workspace-review { min-width: 0; display: grid; gap: 12px; }
    header { display: flex; justify-content: space-between; align-items: center; gap: 12px; } h2 { margin: 0; font-size: 1rem; }
    p { margin: 0; color: var(--muted); font-size: .875rem; line-height: 1.6; } .workspace-root code { display: block; overflow-wrap: anywhere; }
    ul { list-style: none; margin: 0; padding: 0; } li { display: flex; gap: 8px; border-bottom: 1px solid var(--border); padding-block: 10px; font-size: .875rem; } li span { overflow-wrap: anywhere; min-width: 0; } li code { white-space: pre; flex: none; }
    summary { min-height: 44px; padding-block: 10px; cursor: pointer; font-size: .875rem; }
    pre { max-height: min(60dvh, 600px); overflow: auto; padding: 12px; border: 1px solid var(--border); border-radius: 10px; font: 12px/1.65 var(--font-mono, monospace); white-space: pre; }
    button { min-height: var(--control-hit-size); }
</style>
