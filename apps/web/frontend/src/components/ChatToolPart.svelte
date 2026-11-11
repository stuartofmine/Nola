<script lang="ts">
    import { toolTextPreview } from '../lib/toolTextPreview';
    import { replyDisclosure } from '../lib/replyDisclosure';
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
    const inputPreview = $derived(input === null ? null : toolTextPreview(input));
    const resultPreview = $derived(result === null ? null : toolTextPreview(result));
    const outputPreview = $derived(tool?.outputPreview == null ? null : toolTextPreview(tool.outputPreview));
</script>

{#key owner}
{#if tool}
    <details use:replyDisclosure class="tool-part" data-tool-id={part.id} data-reading-part={readingPartKey(part)} data-reading-ticket={reading?.identity??''} data-state={tool.state} open={expanded}>
        <summary onclick={event => { event.preventDefault(); toggle(); }}>
            <span class="tool-indicator" aria-hidden="true"></span>
            <span class="tool-heading"><span class="tool-title" title={title}>{title}</span>{#if !expanded && tool.summary}<span class="tool-brief">{tool.summary}</span>{/if}</span>
            <span class="tool-state">{t($locale, stateLabels[tool.state])}</span>
            <span class="tool-chevron" aria-hidden="true">›</span>
        </summary>
        <div class="tool-details">
            {#if tool.summary !== null}<p class="tool-summary">{tool.summary}</p>{/if}
            {#each toolArtifacts(tool.artifacts) as reference}<ChatReferenceCard {reference} />{/each}
            {#if tool.resources.length}
                <section class="tool-resources" aria-label={t($locale, 'chatPage.toolResources')}>
                    <h4>{t($locale, 'chatPage.toolResources')}</h4>
                    <ul>{#each tool.resources as resource}<li><span class="resource-meta">{resource.type} · {resource.operation}</span><code>{resource.uri}</code></li>{/each}</ul>
                </section>
            {/if}
            {#if tool.outputPreview !== null}
                <section class="tool-output" aria-label={t($locale, 'chatPage.toolPreview')}>
                <div class="field-heading"><h4>{t($locale, 'chatPage.toolPreview')}</h4>
                {#if outputPreview?.truncated}{#key owner}<ChatTextReader {part} field="preview" text={tool.outputPreview} title={title + ' · ' + t($locale, 'chatPage.toolPreview')} />{/key}{/if}</div>
                <pre class="tool-preview" aria-label={t($locale, 'chatPage.toolPreview')}>{outputPreview?.text}</pre>
                </section>
            {/if}
            {#if tool.origin || tool.hasInput || tool.permission || tool.hasStructuredResult}
            <details use:replyDisclosure class="tool-technical" open={technicalExpanded}>
                <summary onclick={event=>{event.preventDefault();chooseField('technical',!technicalExpanded);}}>{t($locale, 'chatPage.toolTechnicalDetails')}</summary>
            <div class="technical-content">
            {#if tool.origin}<section class="tool-origin"><h4>{t($locale, 'chatPage.toolSource')}</h4><p>{tool.origin}</p></section>{/if}
            {#if input !== null}<details use:replyDisclosure class="tool-input" open={inputExpanded}><summary onclick={event=>{event.preventDefault();chooseField('input',!inputExpanded);}}>{t($locale, 'chatPage.toolInput')}</summary>
                {#if inputPreview?.truncated}{#key owner}<ChatTextReader {part} field="input" text={input} title={title + ' · ' + t($locale, 'chatPage.toolInput')} />{/key}{/if}
                <pre>{inputPreview?.text}</pre>
            </details>{/if}
            {#if tool.permission}
                <section class="tool-permission" aria-label={t($locale, 'chatPage.toolPermissionRecord')}>
                    <h4>{t($locale, 'chatPage.toolPermissionRecord')}</h4>
                    <dl class="permission-facts">
                        <dt>{t($locale, 'chatPage.toolPermissionDomain')}</dt><dd>{tool.permission.domain}</dd>
                        <dt>{t($locale, 'chatPage.toolPermissionReason')}</dt><dd>{tool.permission.rationale}</dd>
                        {#if tool.permission.decision !== null}<dt>{t($locale, 'chatPage.toolPermissionDecision')}</dt><dd>{tool.permission.decision}</dd>{/if}
                    </dl>
                    <p class="muted">{t($locale, 'chatPage.toolPermissionReadOnly')}</p>
                </section>
            {/if}
            {#if result !== null}
                <section aria-label={t($locale, 'chatPage.toolResult')}><div class="field-heading"><h4>{t($locale, 'chatPage.toolResult')}</h4>
                {#if resultPreview?.truncated}{#key owner}<ChatTextReader {part} field="result" text={result} title={title + ' · ' + t($locale, 'chatPage.toolResult')} />{/key}{/if}</div>
                <pre class="tool-result" aria-label={t($locale, 'chatPage.toolResult')}>{resultPreview?.text}</pre>
                </section>
            {/if}
            </div>
            </details>
            {/if}
        </div>
    </details>
{/if}
{/key}

<style>
    h4 { margin: 0; color: var(--muted); font-size: 12px; font-weight: 500; line-height: 1.5; }
    .tool-resources, .tool-permission { min-width: 0; overflow-wrap: anywhere; }
    .tool-resources ul { list-style: none; margin: 8px 0 0; padding: 0; border: 1px solid var(--border); border-radius: 8px; overflow: hidden; }
    .tool-resources li { display: grid; grid-template-columns: minmax(88px, 24%) minmax(0, 1fr); gap: 6px 12px; padding: 10px; align-items: baseline; }
    .tool-resources li + li { border-top: 1px solid var(--border); }
    .resource-meta { color: var(--muted); font-size: 12px; }
    .tool-resources code { min-width: 0; white-space: pre-wrap; overflow-wrap: anywhere; font-size: 12px; line-height: 1.5; }
    .field-heading { display: flex; flex-wrap: wrap; justify-content: space-between; align-items: center; gap: 4px 12px; min-width: 0; }
    .field-heading :global(.reader-open) { margin: 0; padding: 4px 8px; font-size: 12px; }
    .permission-facts { display: grid; grid-template-columns: minmax(88px, 24%) minmax(0, 1fr); gap: 8px 12px; font-size: 12px; line-height: 1.5; margin: 8px 0; }
    .permission-facts dt { color: var(--muted); }
    .permission-facts dd { margin: 0; min-width: 0; white-space: pre-wrap; }
    .tool-permission .muted { margin: 8px 0 0; color: var(--muted); font-size: 12px; line-height: 1.5; }
    .technical-content { display: grid; gap: 16px; padding-bottom: 4px; min-width: 0; }
    .technical-content > * { min-width: 0; }
    .tool-origin p { margin: 6px 0 0; font-size: 12px; line-height: 1.5; overflow-wrap: anywhere; }
    .tool-part { container-type: inline-size; --tool-tone: var(--muted); min-width: 0; margin: 6px 0; border: 1px solid var(--border); border-radius: var(--radius); background: var(--panel); box-shadow: 0 1px 2px color-mix(in srgb, var(--muted) 5%, transparent); }
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
    .tool-brief { display: -webkit-box; line-clamp: 2; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; white-space: pre-wrap; color: var(--muted); font-size: 12px; font-weight: 400; margin-top: 4px; }
    .tool-part[data-state='failed'] .tool-brief { color: var(--danger); }
    .tool-technical { border-top: 1px solid var(--border); margin-top: 0; }
    .tool-technical > summary, .tool-input > summary { color: var(--muted); font-size: 12px; padding-inline: 0; }
    .tool-technical > summary::before, .tool-input > summary::before { content: '›'; width: 12px; flex: 0 0 12px; text-align: center; font-size: 18px; }
    .tool-technical[open] > summary::before, .tool-input[open] > summary::before { transform: rotate(90deg); }
    summary:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
    .tool-state { color: var(--tool-tone); font-size: 12px; max-width: 45%; overflow-wrap: anywhere; }
    .tool-indicator { width: 7px; height: 7px; flex: 0 0 7px; border-radius: 50%; background: var(--tool-tone); }
    .tool-chevron { color: var(--muted); font-size: 20px; transition: transform var(--motion-base) var(--motion-disclosure); }
    details[open] > summary .tool-chevron { transform: rotate(90deg); }
    .tool-details { display: grid; gap: 16px; min-width: 0; padding: 0 12px 12px 27px; }
    .tool-details > * { min-width: 0; }
    .tool-summary { margin: 0; white-space: pre-wrap; overflow-wrap: anywhere; line-height: 1.55; font-size: 13px; }
    @media (max-width: 700px) {
        .tool-details { padding-inline: 12px; }
        .tool-resources li, .permission-facts { grid-template-columns: minmax(0, 1fr); gap: 4px; }
        .permission-facts dd + dt { margin-top: 8px; }
        .tool-part > summary { display: grid; grid-template-columns: 7px minmax(0, 1fr) 12px; gap: 2px 8px; font-size: 14px; line-height: 20px; }
        .tool-indicator { grid-column: 1; grid-row: 1; }
        .tool-heading { grid-column: 2; grid-row: 1; }
        .tool-state { grid-column: 2; grid-row: 2; max-width: 100%; }
        .tool-chevron { grid-column: 3; grid-row: 1 / 3; }
    }
    @container (max-width: 420px) {
        .tool-details { padding-inline: 12px; }
        .tool-resources li, .permission-facts { grid-template-columns: minmax(0, 1fr); gap: 4px; }
        .permission-facts dd + dt { margin-top: 8px; }
        .field-heading { align-items: flex-start; }
    }
    pre { max-height: min(320px, 45dvh); box-sizing: border-box; min-width: 0; max-width: 100%; margin: 8px 0 0; padding: 10px; border-radius: 6px; background: var(--canvas); overflow: auto; white-space: pre-wrap; overflow-wrap: anywhere; line-height: 1.5; font-size: 12px; }
    @media (prefers-reduced-motion: no-preference) {
        .tool-part[data-state='running'] .tool-indicator { animation: working 1.4s ease-in-out infinite; }
    }
    @keyframes working { 50% { opacity: .35; } }
</style>
