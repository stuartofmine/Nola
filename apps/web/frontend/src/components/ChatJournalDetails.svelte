<script lang="ts">
    import { replyDisclosure } from '../lib/replyDisclosure';
    import { onMount } from 'svelte';
    import { api } from '../lib/api';
    import { taskDetailsOwner, loadTaskConversation } from '../lib/chatTaskDetails';
    import { locale, t } from '../lib/i18n';
    import type { TimelinePart } from '../lib/chatTimeline';
    import Markdown from './Markdown.svelte';
    import { journalPartStateLabelKey } from '../lib/journalStateLabel';
    let { part, taskNumber = 1, showSummary = true, live = true }: { part: TimelinePart; taskNumber?: number; showSummary?: boolean; live?: boolean } = $props();
    const value = $derived(part.value);
    const taskOwner = $derived(taskDetailsOwner(part));
    const taskKey = $derived(taskOwner ? JSON.stringify([part.id, part.source === 'journal' ? part.journalOwner : null, taskOwner, value.state]) : '');
    let mounted = $state(false);
    let loadedKey = $state('');
    let childHref = $state<string | null>(null);
    let childLoading = $state(false);
    let childFailed = $state(false);
    let childRead: AbortController | null = null;
    onMount(() => { mounted = true; return () => { mounted = false; childRead?.abort(); }; });
    async function loadChild() {
        const capturedOwner = taskOwner, capturedKey = taskKey;
        if (!capturedOwner) return;
        childRead?.abort();
        const controller = new AbortController(); childRead = controller;
        loadedKey = capturedKey; childLoading = true; childFailed = false; childHref = null;
        try {
            const href = await loadTaskConversation(capturedOwner, path => api(path, {signal: controller.signal}));
            if (!controller.signal.aborted && mounted && taskKey === capturedKey) childHref = href;
        } catch { if (!controller.signal.aborted && mounted && taskKey === capturedKey) childFailed = true; }
        finally { if (!controller.signal.aborted && mounted && taskKey === capturedKey) childLoading = false; }
    }
    $effect(() => { if (mounted && (expanded || !showSummary) && taskKey && loadedKey !== taskKey) void loadChild(); });
    let userExpanded = $state<boolean | null>(null);
    const expanded = $derived(userExpanded ?? (value.kind === 'error' || value.kind !== 'model_attempt' && value.state === 'failed'));
    const terminal = $derived(['sealed', 'completed', 'cancelled', 'failed'].includes(String(value.state)));
    const hasSuccessor = $derived(value.kind === 'model_attempt' && typeof value.fallback_attempt_id === 'string' && !!value.fallback_attempt_id.trim());
    const title = $derived(['task', 'task_control'].includes(value.kind) ? t($locale, 'chatPage.backgroundTaskNumber', {number: taskNumber}) : value.kind === 'model_attempt' ? t($locale, hasSuccessor ? 'chatPage.requestRestarted' : 'chatPage.modelAttempt') :
        value.kind === 'compaction' ? t($locale, 'chatPage.compactionRecord') : value.kind === 'historical' ? t($locale, 'chatPage.historicalRecord') :
        [value.title, value.display_name, value.name]
        .find(item => typeof item === 'string' && item.length) as string | undefined);
    const failure = $derived(typeof value.failure === 'string' ? value.failure : value.failure && typeof value.failure === 'object' && !Array.isArray(value.failure) && 'message' in value.failure ? value.failure.message : null);
    const detail = $derived([terminal ? null : value.current_step, value.snippet, failure, value.preview, value.kind === 'compaction' ? value.summary : null]
        .filter(item => typeof item === 'string' && item.length).join('\n\n'));
    const stateKey = $derived(value.kind === 'skill' && value.state === 'running' && !live ? 'chatPage.stateIncomplete' : journalPartStateLabelKey(value.kind, value.state));
    const skillTitle = $derived(value.skill_id === 'conduct-deep-research' && (!title || title === value.skill_id) ? t($locale, 'chatPage.deepResearch') : title);
    const seconds = $derived(typeof value.elapsed_millis === 'number' || typeof value.elapsed_millis === 'string' ? Number(value.elapsed_millis) / 1000 : NaN);
    const stateLabel = $derived(stateKey && !hasSuccessor ? t($locale, stateKey) : '');
    const diagnostics = $derived(Object.fromEntries(['task_id', 'skill_id', 'run_id', 'mode', 'model_turns', 'tool_calls', 'elapsed_millis', 'result_ref', 'attempt_id', 'provider', 'model', 'fallback_attempt_id', 'fallback_provider', 'fallback_model', 'source_fingerprint', 'retained_item_ids', 'original_token_count', 'compacted_token_count']
        .filter(key => value[key] !== undefined && value[key] !== null).map(key => [key, value[key]])));
</script>

<!-- Both current projections share this read-only display. TaskControl
     authority is handled separately from these labels and diagnostic fields. -->
{#snippet content()}
    {#if showSummary && detail}<Markdown source={detail} />{/if}
    {#if showSummary && !terminal && typeof value.progress === 'number' && Number.isFinite(value.progress) && value.progress >= 0 && value.progress <= 1}<progress value={value.progress} max="1" aria-label={title || t($locale, 'chatPage.details')}></progress>{/if}
    {#if showSummary && typeof value.uri === 'string' && value.uri}<p class="reference">{value.uri}</p>{/if}
    {#if taskOwner && loadedKey === taskKey}
        {#if childLoading}<p role="status">{t($locale, 'chatPage.taskDetailsLoading')}</p>
        {:else if childFailed}<button onclick={loadChild}>{t($locale, 'chatPage.taskDetailsRetry')}</button>
        {:else if childHref}<a class="task-conversation" href={childHref}>{t($locale, 'chatPage.taskOpenConversation')}</a>{/if}
    {/if}
    {#if showSummary && Object.keys(diagnostics).length}
        <details use:replyDisclosure class="technical"><summary>{t($locale, 'chatPage.toolTechnicalDetails')}</summary><pre>{JSON.stringify(diagnostics, null, 2)}</pre></details>
    {/if}
{/snippet}

{#if showSummary && value.kind === 'skill'}
    <section class="skill-summary" data-part-id={part.id}>
        <span>{skillTitle || t($locale, 'chatPage.details')}</span>{#if stateLabel}<small>{stateLabel}</small>{/if}
        <details class="skill-info"><summary aria-label={t($locale, 'chatPage.details')}>ⓘ</summary><div>
            {#if value.model_turns != null}<p>{t($locale, 'chatPage.modelTurns')} {String(value.model_turns)}</p>{/if}
            {#if value.tool_calls != null}<p>{t($locale, 'chatPage.toolCallsCount')} {String(value.tool_calls)}</p>{/if}
            {#if Number.isFinite(seconds) && seconds >= 0}<p>{Math.floor(seconds / 60)}:{String(Math.floor(seconds % 60)).padStart(2, '0')}</p>{/if}
        </div></details>
        {#if detail}<div class="skill-body"><Markdown source={detail} /></div>{/if}
    </section>
{:else if showSummary}
    <details use:replyDisclosure class="journal-detail" data-part-id={part.id} open={expanded}>
        <summary class="part-summary" onclick={event => { event.preventDefault(); userExpanded = !expanded; }}>{title || t($locale, 'chatPage.details')}{#if stateLabel} · {stateLabel}{/if}</summary>
        {@render content()}
    </details>
{:else}
    <div class="task-control-details" data-part-id={part.id}>{@render content()}</div>
{/if}

<style>
    .skill-summary { display: flex; flex-wrap: wrap; align-items: baseline; gap: 8px; margin-block: 8px; }
    .skill-summary > small { margin-inline-start: auto; color: var(--muted); }
    .skill-body { flex-basis: 100%; }
    .skill-info { position: relative; color: var(--muted); }
    .skill-info > summary { list-style: none; min-width: 28px; min-height: 32px; padding: 4px; text-align: center; }
    .skill-info > summary::-webkit-details-marker { display: none; }
    .skill-info > div { position: absolute; z-index: 2; inset-inline-end: 0; min-width: 180px; padding: 12px; border: 1px solid var(--border); border-radius: 12px; background: var(--panel); font-size: .8125rem; }
    .skill-info p { margin: 4px 0; }
    @media (pointer: coarse) { .skill-info > summary { min-width: 44px; min-height: 44px; } }
    .journal-detail { margin: .55rem 0; }
    .task-conversation, button { display: inline-flex; align-items: center; min-height: 44px; }
    summary { cursor: pointer; overflow-wrap: anywhere; min-height: 44px; box-sizing: border-box; padding-block: 12px; }
    pre { white-space: pre-wrap; overflow-wrap: anywhere; font-size: .8125rem; }
    .reference { overflow-wrap: anywhere; font-size: .85em; opacity: .8; }
</style>
