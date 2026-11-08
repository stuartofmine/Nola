<script lang="ts">
    import {getContext} from 'svelte';
    import {MESSAGE_PART_READING,readingPartKey,type MessagePartReading,type DisclosureField} from '../lib/messagePartReading';
    import { locale, t } from '../lib/i18n';
    import { timelineToolPresentation } from '../lib/chatTimeline';
    import type { TimelinePart, TimelineToolState } from '../lib/chatTimeline';
    import { toolArtifacts } from '../lib/chatReferences';
    import ChatReferenceCard from './ChatReferenceCard.svelte';
    import ChatTextReader from './ChatTextReader.svelte';

    interface Props { part: TimelinePart; expandedChoice?: boolean | null; onexpandedchange?: (expanded: boolean) => void }
    let { part, expandedChoice, onexpandedchange }: Props = $props();
    const stateLabels: Record<TimelineToolState, string> = {
        pending: 'chatPage.toolPending', running: 'chatPage.toolRunning', completed: 'chatPage.toolCompleted', failed: 'chatPage.toolFailed',
        cancelled: 'chatPage.toolCancelled', waiting_for_permission: 'chatPage.toolWaitingForPermission',
        waiting_for_input: 'chatPage.toolWaitingForInput', unknown: 'chatPage.toolUnknown',
        sealed: 'chatPage.toolSealed',
    };
    const tool = $derived(timelineToolPresentation(part));
    const messageReading=getContext<MessagePartReading|undefined>(MESSAGE_PART_READING);
    const reading=$derived(messageReading?.bind(part));
    const owner = $derived((reading?reading.identity:(part.source === 'journal' ? part.journalOwner : part.source)) + ':' + part.id);
    let localChoices=$state<{owner:string;technical?:boolean;input?:boolean}|null>(null);
    const technicalExpanded=$derived(reading?.choice('technical')??(localChoices?.owner===owner?localChoices.technical??false:false));
    const inputExpanded=$derived(reading?.choice('input')??(localChoices?.owner===owner?localChoices.input??false:false));
    function chooseField(field:DisclosureField,expanded:boolean){
        if(reading)reading.choose(field,expanded);
        else localChoices={...(localChoices?.owner===owner?localChoices:{}),owner,[field]:expanded};
    }
    let userExpanded = $state<{ owner: string; expanded: boolean } | null>(null);
    const expanded = $derived(expandedChoice ?? (!messageReading&&userExpanded?.owner === owner ? userExpanded.expanded : false));
    function toggle() {
        const next=!expanded;
        if(!messageReading)userExpanded={owner,expanded:next};
        onexpandedchange?.(next);
    }
    const title = $derived(tool?.title || t($locale, 'chatPage.toolActivity'));
    function structured(value: unknown): string {
        return typeof value === 'string' ? value : JSON.stringify(value, null, 2) ?? '';
    }
    const result = $derived(tool?.hasStructuredResult ? structured(tool.structuredResult) : null);
    const input = $derived(tool?.hasInput ? structured(tool.input) : null);
    function preview(text: string): string { return text.length > 2000 ? text.slice(0, 2000) + '…' : text; }
</script>

{#key owner}
{#if tool}
    <details class="tool-part" data-tool-id={part.id} data-reading-part={readingPartKey(part)} data-reading-ticket={reading?.identity??''} data-state={tool.state} open={expanded}>
        <summary onclick={event => { event.preventDefault(); toggle(); }}>
            <span class="tool-indicator" aria-hidden="true"></span>
            <span class="tool-heading"><span class="tool-title" title={title}>{title}</span>{#if !expanded && tool.state === 'failed' && tool.summary}<span class="tool-failure-summary">{tool.summary}</span>{/if}</span>
            <span class="tool-state">{t($locale, stateLabels[tool.state])}</span>
            <span class="tool-chevron" aria-hidden="true">›</span>
        </summary>
        <div class="tool-details">
            {#if tool.summary !== null}<p class="tool-summary">{tool.summary}</p>{/if}
            {#each toolArtifacts(tool.artifacts) as reference}<ChatReferenceCard {reference} />{/each}
            {#if tool.origin || tool.hasInput || tool.permission || tool.resources.length || tool.outputPreview !== null || tool.hasStructuredResult}
            <details class="tool-technical" open={technicalExpanded}>
                <summary onclick={event=>{event.preventDefault();chooseField('technical',!technicalExpanded);}}>{t($locale, 'chatPage.toolTechnicalDetails')}</summary>
            {#if tool.origin}<p class="tool-origin">{tool.origin}</p>{/if}
            {#if input !== null}<details class="tool-input" open={inputExpanded}><summary onclick={event=>{event.preventDefault();chooseField('input',!inputExpanded);}}>{t($locale, 'chatPage.toolInput')}</summary>
                {#if input.length > 2000}{#key owner}<ChatTextReader {part} field="input" text={input} title={title + ' · ' + t($locale, 'chatPage.toolInput')} />{/key}{/if}
                <pre>{preview(input)}</pre>
            </details>{/if}
            {#if tool.permission}
                <section class="tool-permission" aria-label={t($locale, 'chatPage.toolPermissionRecord')}>
                    <h4>{t($locale, 'chatPage.toolPermissionRecord')}</h4>
                    <p>{tool.permission.domain}</p><p>{tool.permission.rationale}</p>
                    {#if tool.permission.decision !== null}<p>{t($locale, 'chatPage.toolPermissionDecision')}: {tool.permission.decision}</p>{/if}
                    <p class="muted">{t($locale, 'chatPage.toolPermissionReadOnly')}</p>
                </section>
            {/if}
            {#if tool.resources.length}
                <section class="tool-resources" aria-label={t($locale, 'chatPage.toolResources')}>
                    <h4>{t($locale, 'chatPage.toolResources')}</h4>
                    <ul>{#each tool.resources as resource}<li><span>{resource.type} · {resource.operation}</span><code>{resource.uri}</code></li>{/each}</ul>
                </section>
            {/if}
            {#if tool.outputPreview !== null}
                {#if tool.outputPreview.length > 2000}{#key owner}<ChatTextReader {part} field="preview" text={tool.outputPreview} title={title + ' · ' + t($locale, 'chatPage.toolPreview')} />{/key}{/if}
                <pre class="tool-preview" aria-label={t($locale, 'chatPage.toolPreview')}>{preview(tool.outputPreview)}</pre>
            {/if}
            {#if result !== null}
                {#if result.length > 2000}{#key owner}<ChatTextReader {part} field="result" text={result} title={title + ' · ' + t($locale, 'chatPage.toolResult')} />{/key}{/if}
                <pre class="tool-result" aria-label={t($locale, 'chatPage.toolResult')}>{preview(result)}</pre>
            {/if}
            </details>
            {/if}
        </div>
    </details>
{/if}
{/key}

<style>
    .tool-permission,.tool-resources{border:1px solid var(--border);border-radius:6px;padding:10px;margin-top:10px;overflow-wrap:anywhere;min-width:0}h4{margin:0;font-size:13px}.tool-permission p{white-space:pre-wrap;margin:6px 0;font-size:12px}.tool-resources ul{margin:8px 0 0;padding-left:18px}.tool-resources li{margin:8px 0}.tool-resources code{display:block;white-space:pre-wrap;overflow-wrap:anywhere;font-size:12px}
    .tool-part { --tool-tone: var(--muted); min-width: 0; margin: 6px 0; border: 1px solid var(--border); border-radius: var(--radius); background: var(--panel); }
    .tool-part[data-state='running'] { --tool-tone: var(--accent); }
    .tool-part[data-state='completed'] { --tool-tone: var(--success); }
    .tool-part[data-state='failed'] { --tool-tone: var(--danger); }
    .tool-part[data-state='waiting_for_permission'], .tool-part[data-state='waiting_for_input'] { --tool-tone: var(--warning); }
    summary { display: flex; align-items: center; gap: 8px; min-height: 44px; padding: 10px 12px; cursor: pointer; list-style: none; border-radius: inherit; }
    summary::-webkit-details-marker { display: none; }
    summary:hover { background: color-mix(in srgb, var(--muted) 7%, transparent); }
    .tool-heading { flex: 1; min-width: 0; overflow-wrap: anywhere; }
    .tool-title { font-weight: 600; display: -webkit-box; line-clamp: 2; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
    details[open] > summary .tool-title { display: block; }
    .tool-failure-summary { display: -webkit-box; line-clamp: 2; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; white-space: pre-wrap; color: var(--danger); font-size: 12px; font-weight: 400; margin-top: 4px; }
    .tool-technical > summary { color: var(--muted); font-size: 12px; padding-inline: 0; }
    .tool-technical > summary::before { content: '›'; font-size: 18px; }
    .tool-technical[open] > summary::before { transform: rotate(90deg); }
    summary:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
    .tool-state { color: var(--tool-tone); font-size: 12px; max-width: 45%; overflow-wrap: anywhere; }
    .tool-indicator { width: 7px; height: 7px; flex: 0 0 7px; border-radius: 50%; background: var(--tool-tone); }
    .tool-chevron { color: var(--muted); font-size: 20px; transition: transform var(--motion-base) var(--motion-disclosure); }
    details[open] .tool-chevron { transform: rotate(90deg); }
    .tool-details { min-width: 0; padding: 0 12px 12px; }
    .tool-summary { white-space: pre-wrap; line-height: 1.55; }
    @media (max-width: 700px) {
        .tool-part > summary { display: grid; grid-template-columns: 7px minmax(0, 1fr) 12px; gap: 2px 8px; font-size: 14px; line-height: 20px; }
        .tool-indicator { grid-column: 1; grid-row: 1; }
        .tool-heading { grid-column: 2; grid-row: 1; }
        .tool-state { grid-column: 2; grid-row: 2; max-width: 100%; }
        .tool-chevron { grid-column: 3; grid-row: 1 / 3; }
    }
    pre { max-width: 100%; margin: 8px 0 0; padding: 10px; border-radius: 6px; background: var(--canvas); overflow: auto; white-space: pre-wrap; overflow-wrap: anywhere; line-height: 1.5; font-size: 12px; }
    @media (prefers-reduced-motion: no-preference) {
        .tool-part[data-state='running'] .tool-indicator { animation: working 1.4s ease-in-out infinite; }
        details[open] .tool-details { animation: reveal var(--motion-base) var(--motion-disclosure); }
    }
    @keyframes working { 50% { opacity: .35; } }
    @keyframes reveal { from { opacity: 0; } to { opacity: 1; } }
</style>
