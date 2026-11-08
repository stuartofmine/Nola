<script lang="ts">
    import { locale, t } from '../lib/i18n';
    import { messagePresentation, executionProcessExpanded, isExecutionPresentation, liveReasoningParts } from '../lib/chatMessagePresentation';
    import { timelinePartText, timelineToolsNeedAttention } from '../lib/chatTimeline';
    import type { TimelinePart } from '../lib/chatTimeline';
    import Markdown from './Markdown.svelte';
    import ChatToolPart from './ChatToolPart.svelte';
    import ChatToolGroup from './ChatToolGroup.svelte';

    let { parts, live, choices, onexpandedchange, expandedChoice, onprocessexpandedchange }: { parts: TimelinePart[]; live: boolean;
        choices: Record<string, boolean>; onexpandedchange: (id: string, expanded: boolean) => void;
        expandedChoice: boolean | null; onprocessexpandedchange: (expanded: boolean) => void } = $props();
    const needsAttention = $derived(timelineToolsNeedAttention(parts));
    const currentReasoning = $derived(live ? liveReasoningParts(parts) : []);
    const readingToolDetails = $derived(parts.some(part => part.value.kind === 'tool' && choices[part.id] === true));
    const expanded = $derived(executionProcessExpanded(expandedChoice, needsAttention || currentReasoning.length > 0 || readingToolDetails));
    const thinkingOnly = $derived(parts.every(part => part.value.kind === 'reasoning'
        || part.value.kind === 'historical' && isExecutionPresentation({id: part.id, kind: 'part', part})));
    const groups = $derived(messagePresentation(parts).filter(isExecutionPresentation));
</script>

{#if needsAttention && !expanded && !live}<p class="process-attention" role="status">{t($locale, 'chatPage.processAttention')}</p>{/if}
<details class="execution-process" open={expanded}>
    <summary onclick={event => { event.preventDefault(); onprocessexpandedchange(!expanded); }}>
        <svg class="chevron" class:expanded viewBox="0 0 16 16" aria-hidden="true"><path d="m6 3 5 5-5 5" /></svg>
        <span>{t($locale, thinkingOnly ? 'chatPage.thinking' : 'chatPage.executionProcess')}</span>
        {#if currentReasoning.length || live && parts.some(part => ['pending', 'running'].includes(String(part.value.state)))}<span class="activity-light" aria-hidden="true"></span>{/if}
    </summary>
    <div class="process-content">
        {#each groups as group (group.id)}
            {#if group.kind === 'tools'}
                <ChatToolGroup parts={group.parts} completed {choices} {onexpandedchange} />
            {:else if group.kind === 'part'}
                {@const part = group.part}
                {#if part.value.kind === 'tool'}
                    <ChatToolPart {part} expandedChoice={Object.hasOwn(choices, part.id) ? choices[part.id] : undefined} onexpandedchange={expanded => onexpandedchange(part.id, expanded)} />
                {:else if part.value.kind === 'reasoning' || timelinePartText(part, 'reasoning')}
                    <section class="reasoning" aria-label={t($locale, 'chatPage.thinking')}>
                        {#if !thinkingOnly}<small>{t($locale, 'chatPage.thinking')}</small>{/if}
                        <Markdown source={timelinePartText(part, 'reasoning')} />
                    </section>
                {/if}
            {/if}
        {/each}
    </div>
</details>

<style>
    .execution-process { margin-bottom: 12px; min-width: 0; }
    .execution-process[open] { background: var(--panel); border-radius: 12px; }
    .process-attention { color: var(--danger); font-size: 12px; }
    summary { display: flex; align-items: center; gap: 8px; min-height: max(32px, var(--control-hit-size)); box-sizing: border-box; padding: 8px 0; cursor: pointer; color: var(--muted); border-radius: 12px; list-style: none; font-size: .875rem; font-weight: 500; transition: background-color 140ms ease, color 140ms ease; }
    summary::-webkit-details-marker { display: none; }
    summary:hover { background: color-mix(in srgb, currentColor 5%, transparent); color: var(--content); }
    .chevron { flex: none; width: 14px; height: 14px; fill: none; stroke: currentColor; stroke-width: 1.5; stroke-linecap: round; stroke-linejoin: round; transition: transform 160ms ease-out; }
    .chevron.expanded { transform: rotate(90deg); }
    .activity-light { width: 6px; height: 6px; flex: none; border-radius: 50%; margin-inline-start: auto; background: var(--accent); animation: quiet-status 2.4s ease-in-out infinite; }
    @keyframes quiet-status { 0%, 100% { opacity: .65; } 50% { opacity: 1; } }
    summary:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; border-radius: 12px; }
    .process-content { display: grid; gap: 12px; padding: 4px 12px 14px 18px; }
    .reasoning { padding-inline-start: 12px; border-inline-start: 2px solid var(--border); overflow-wrap: anywhere; }
    .reasoning > small { display: block; margin-block-end: 4px; }
    small { color: var(--muted); }
    @media (pointer: coarse) { summary { min-height: 44px; } }
    @media (prefers-contrast: more) { .execution-process { border: 1px solid currentColor; } }
    @media (prefers-reduced-motion: reduce) { summary, .chevron { transition: none; } .activity-light { animation: none; } }
</style>
