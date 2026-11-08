<script lang="ts">
    import { onMount } from 'svelte';
    import { api } from '../lib/api';
    import { taskDetailsOwner, loadTaskConversation } from '../lib/chatTaskDetails';
    import { locale, t } from '../lib/i18n';
    import type { TimelinePart } from '../lib/chatTimeline';
    import Markdown from './Markdown.svelte';
    import { journalPartStateLabelKey } from '../lib/journalStateLabel';
    let { part, taskNumber = 1, showSummary = true }: { part: TimelinePart; taskNumber?: number; showSummary?: boolean } = $props();
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
    const stateKey = $derived(journalPartStateLabelKey(value.kind, value.state));
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
        <details class="technical"><summary>{t($locale, 'chatPage.toolTechnicalDetails')}</summary><pre>{JSON.stringify(diagnostics, null, 2)}</pre></details>
    {/if}
{/snippet}

{#if showSummary}
    <details class="journal-detail" data-part-id={part.id} open={expanded}>
        <summary class="part-summary" onclick={event => { event.preventDefault(); userExpanded = !expanded; }}>{title || t($locale, 'chatPage.details')}{#if stateLabel} · {stateLabel}{/if}</summary>
        {@render content()}
    </details>
{:else}
    <div class="task-control-details" data-part-id={part.id}>{@render content()}</div>
{/if}

<style>
    .journal-detail { margin: .55rem 0; }
    .task-conversation, button { display: inline-flex; align-items: center; min-height: 44px; }
    summary { cursor: pointer; overflow-wrap: anywhere; min-height: 44px; box-sizing: border-box; padding-block: 12px; }
    pre { white-space: pre-wrap; overflow-wrap: anywhere; font-size: .8125rem; }
    .reference { overflow-wrap: anywhere; font-size: .85em; opacity: .8; }
</style>
