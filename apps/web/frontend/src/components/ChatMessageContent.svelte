<script lang="ts">
    import { tick, untrack, onDestroy, setContext, type Snippet } from 'svelte';
    import {MESSAGE_PART_READING,MessageReadingScope,readingPartKey,type MessagePartReading,type MessageTextSnapshot} from '../lib/messagePartReading';
    import ChatTextReaderDialog from './ChatTextReaderDialog.svelte';
    import { locale, t } from '../lib/i18n';
    import { messagePresentation, messageFlowPresentation, executionProcessChoice, modelFailureHelpKey } from '../lib/chatMessagePresentation';
    import ChatExecutionProcess from './ChatExecutionProcess.svelte';
    import { timelinePartText, timelineReasoningParts } from '../lib/chatTimeline';
    import type { TimelineMessage, TimelinePart } from '../lib/chatTimeline';
    import {journalTaskOwner, type TaskOwner} from '../lib/chatTaskActions';
    import { timelineReference } from '../lib/chatReferences';
    import Markdown from './Markdown.svelte';
    import ChatToolPart from './ChatToolPart.svelte';
    import ChatToolGroup from './ChatToolGroup.svelte';
    import ChatReferenceCard from './ChatReferenceCard.svelte';
    import ChatReferenceFooter from './ChatReferenceFooter.svelte';
    import ChatJournalDetails from './ChatJournalDetails.svelte';
    import ChatTaskActions from './ChatTaskActions.svelte';
    import ChatReplyError from './ChatReplyError.svelte';
    import ChatCanonicalErrorPart from './ChatCanonicalErrorPart.svelte';

    let { message, ownerKey, readingActive = true, skipText = false, taskOwner, taskClientId, onTaskRecovered, surface }: {message: TimelineMessage; ownerKey:string; readingActive?:boolean; skipText?: boolean;
        taskOwner?: TaskOwner; taskClientId?:string; onTaskRecovered?:()=>void; surface?: Snippet<[TimelinePart]>} = $props();
    const groups = $derived(message.role === 'assistant' ? messageFlowPresentation(message.parts.filter(part => !['source', 'citation'].includes(part.value.kind))) : messagePresentation(message.parts));
    const latestProcessID = $derived([...groups].reverse().find(group => group.kind === 'process')?.id);
    const initialTexts = untrack(() => new Map((message.state === 'running' || message.state === 'pending' ? groups : []).flatMap(group => group.kind === 'text' ? [[group.id, group.text] as const] : [])));
    $effect(() => { if (message.state !== 'running' && message.state !== 'pending') initialTexts.clear(); });
    const reasoning = $derived(new Set(timelineReasoningParts(message).map(part => part.id)));
    const taskNumbers = $derived.by(() => {
        const ids = new Map<string, number>();
        for (const part of message.parts) {
            if (['task', 'task_control'].includes(part.value.kind) && typeof part.value.task_id === 'string'
                && !ids.has(part.value.task_id)) ids.set(part.value.task_id, ids.size + 1);
        }
        return ids;
    });
    let container: HTMLDivElement;
    const readingOwner=$derived.by(()=>{
        if(typeof ownerKey!=='string'||!ownerKey)throw new Error('Message reading owner is required');
        return JSON.stringify([ownerKey,message.id]);
    });
    const readingScope=new MessageReadingScope(untrack(()=>readingOwner),untrack(()=>message.parts.map(readingPartKey)),untrack(()=>readingActive));
    let knownReadingOwner=untrack(()=>readingOwner);
    let readingRevision=$state(0);
    // Identity is the modal residency/close CAS witness; do not proxy it anew
    // when a Part update republishes the same immutable snapshot.
    let textSnapshot=$state.raw<MessageTextSnapshot|null>(null);
    let readerOpener:HTMLButtonElement|null=null;
    function choices(field:'tool'|'process'):Record<string,boolean>{
        readingRevision;
        return Object.fromEntries(message.parts.flatMap(part=>{
            const choice=readingScope.bind(readingPartKey(part)).choice(field);
            return choice===null?[]:[[part.id,choice]];
        }));
    }
    const toolChoices=$derived(choices('tool'));
    const processChoices=$derived(choices('process'));
    function controls(parts:TimelinePart[]){
        readingRevision;
        const bound=parts.map(part=>({id:part.id,reading:readingScope.bind(readingPartKey(part))}));
        return {
            tool(id:string,expanded:boolean){
                if(!bound.find(value=>value.id===id)?.reading.choose('tool',expanded))return;
                if(expanded)for(const value of bound){value.reading.choose('process',true);if(value.id===id)value.reading.choose('group',true);}
                readingRevision++;
            },
            process(expanded:boolean){if(bound.map(value=>value.reading.choose('process',expanded)).some(Boolean))readingRevision++;},
        };
    }
    setContext<MessagePartReading>(MESSAGE_PART_READING,{
        bind(part){
            readingRevision;
            const bound=readingScope.bind(readingPartKey(part));
            return {
                identity:bound.identity,
                choice:bound.choice,
                choose(field,expanded){if(bound.choose(field,expanded))readingRevision++;},
                open(field,title,text,button){
                    const snapshot=bound.open(field,title,text);
                    if(!snapshot)return;
                    readerOpener=button;textSnapshot=snapshot;
                    bound.choose('tool',true);bound.choose('group',true);
                    const process=groups.find(group=>group.kind==='process'&&group.parts.some(value=>readingPartKey(value)===snapshot.partKey));
                    if(process?.kind==='process')for(const member of process.parts)readingScope.bind(readingPartKey(member)).choose('process',true);
                    readingRevision++;
                },
            };
        },
    });
    $effect.pre(()=>{
        if(knownReadingOwner!==readingOwner){
            knownReadingOwner=readingOwner;readerOpener=null;
        }
        if(readingScope.update(readingOwner,message.parts.map(readingPartKey),readingActive)){
            readingRevision++;textSnapshot=readingScope.selected;
            if(!readingScope.selected)readerOpener=null;
        }
    });
    onDestroy(()=>readingScope.dispose());
    async function closeTextReader(shown:MessageTextSnapshot){
        if(!readingScope.close(shown))return;
        const boundary=readingScope.bind(shown.partKey),opener=readerOpener;
        textSnapshot=null;readerOpener=null;
        await tick();
        if(!boundary.current()||readingScope.selected)return;
        if(opener?.isConnected)opener.focus();
        else [...container.querySelectorAll<HTMLButtonElement>('[data-reader-part]')]
            .find(button=>button.dataset.readerPart===shown.partKey&&button.dataset.readerField===shown.field)?.focus();
    }
    $effect.pre(() => {
        groups;
        const focused = document.activeElement;
        if (!(focused instanceof HTMLElement) || !container?.contains(focused)) return;
        const oldCard=focused.closest<HTMLElement>('.tool-part');
        const key=oldCard?.dataset.readingPart,ticket=oldCard?.dataset.readingTicket;
        if(!key||!ticket)return;
        const boundary=readingScope.bind(key);
        if(!boundary.current()||boundary.identity!==ticket)return;
        void tick().then(() => {
            if (!boundary.current() || focused.isConnected || !container?.isConnected || document.activeElement !== document.body) return;
            const card = [...container.querySelectorAll<HTMLElement>('.tool-part')]
                .find(node => node.dataset.readingPart===key&&node.dataset.readingTicket===ticket);
            const summary = card?.querySelector<HTMLElement>(':scope > summary');
            if (summary?.getClientRects().length) summary.focus();
            else card?.parentElement?.closest('details')?.querySelector<HTMLElement>(':scope > summary')?.focus();
        });
    });
</script>

<div class="message-parts" bind:this={container}>
{#if message.role === 'assistant' && message.state === 'failed' && !message.parts.some(part => part.value.kind === 'error')}
    {@const help = modelFailureHelpKey(message.parts)}
    <ChatReplyError detail={help ? t($locale, help) : ''} />
{/if}
{#each groups as group (group.id)}
    {#if group.kind === 'process'}
        {@const reading=controls(group.parts)}
        <ChatExecutionProcess parts={group.parts} live={(message.state === 'running' || message.state === 'pending') && group.id === latestProcessID} choices={toolChoices} expandedChoice={executionProcessChoice(group.parts, processChoices)}
            onprocessexpandedchange={reading.process} onexpandedchange={reading.tool} />
    {:else if group.kind === 'text'}
        {#if !skipText}<Markdown source={group.text} initialText={initialTexts.get(group.id) ?? ''} streaming={message.role === 'assistant' && (message.state === 'running' || message.state === 'pending')} />{/if}
    {:else if group.kind === 'tools'}
        {@const reading=controls(group.parts)}
        <ChatToolGroup parts={group.parts} completed choices={toolChoices} onexpandedchange={reading.tool} />
    {:else}
        {@const part = group.part}
        {@const reference = timelineReference(part)}
        {@const historicalReasoning = part.value.kind === 'historical' ? timelinePartText(part, 'reasoning') : ''}
        {#if part.value.kind === 'ui_surface'}
            {#if surface}{@render surface(part)}{/if}
        {:else if part.value.kind === 'error'}
            {#if part.source === 'canonical'}<ChatCanonicalErrorPart part={part.value} />
            {:else}<ChatReplyError detail={typeof part.value.failure === 'string' ? part.value.failure : ''} />{/if}
        {:else if part.value.kind === 'tool'}
            {@const reading=controls([part])}
            <ChatToolPart {part} expandedChoice={Object.hasOwn(toolChoices, part.id) ? toolChoices[part.id] : undefined} onexpandedchange={expanded => reading.tool(part.id,expanded)} />
        {:else if reasoning.has(part.id) || historicalReasoning}
            <details class="standalone-reasoning">
                <summary>
                    <svg class="reasoning-chevron" viewBox="0 0 16 16" aria-hidden="true"><path d="m6 3 5 5-5 5" /></svg>
                    <span class="reasoning-title">{typeof part.value.title === 'string' && part.value.title.length ? part.value.title : t($locale, 'chatPage.thinking')}</span>
                </summary>
                <div class="reasoning-body"><Markdown source={historicalReasoning || timelinePartText(part, 'reasoning')} /></div>
            </details>
        {:else if reference}<ChatReferenceCard {reference} />
        {:else if part.source !== 'optimistic' && !['text', 'reasoning', 'ui_surface'].includes(part.value.kind)}
            <ChatJournalDetails {part} live={message.state === 'running' || message.state === 'pending'} taskNumber={taskNumbers.get(String(part.value.task_id)) ?? 1}
                showSummary={part.value.kind !== 'task_control' || !message.parts.some(item => item.value.kind === 'task' && item.value.task_id === part.value.task_id)} />{/if}
        {#if part.value.kind === 'historical' && surface}{@render surface(part)}{/if}
        {#if taskOwner && part.value.kind === 'task_control'}{@const historicalOwner=journalTaskOwner(part,taskOwner)}{#if historicalOwner}<ChatTaskActions {part} active={readingActive} owner={historicalOwner} clientId={taskClientId} onRecovered={onTaskRecovered} />{/if}{/if}
    {/if}
{/each}
{#if message.role === 'assistant'}{#key readingOwner}<ChatReferenceFooter parts={message.parts} />{/key}{/if}
{#if textSnapshot}{@const shown=textSnapshot}{#key shown}<ChatTextReaderDialog snapshot={shown} onclose={()=>void closeTextReader(shown)} />{/key}{/if}
</div>

<style>
    .message-parts { display: contents; }
    .standalone-reasoning { margin-bottom: 12px; min-width: 0; border-radius: 12px; transition: background-color var(--motion-base) var(--motion-disclosure); }
    .standalone-reasoning[open] { background: var(--panel); }
    .standalone-reasoning > summary { display: flex; align-items: center; gap: 8px; min-height: max(32px, var(--control-hit-size)); box-sizing: border-box; padding: 8px 10px; cursor: pointer; color: var(--muted); border-radius: 12px; list-style: none; font-size: .875rem; font-weight: 500; transition: background-color 140ms ease, color 140ms ease; }
    .standalone-reasoning > summary::-webkit-details-marker { display: none; }
    .standalone-reasoning > summary:hover { background: color-mix(in srgb, currentColor 5%, transparent); color: var(--content); }
    .standalone-reasoning > summary:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; border-radius: 12px; }
    .reasoning-chevron { flex: none; width: 14px; height: 14px; fill: none; stroke: currentColor; stroke-width: 1.6; stroke-linecap: round; stroke-linejoin: round; transition: transform var(--motion-base) var(--motion-disclosure); }
    .standalone-reasoning[open] > summary .reasoning-chevron { transform: rotate(90deg); }
    .reasoning-title { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .reasoning-body { margin: 0 12px 14px 18px; padding-inline-start: 12px; border-inline-start: 2px solid var(--border); overflow-wrap: anywhere; max-height: min(360px, 45dvh); overflow: auto; }
    @media (pointer: coarse) { .standalone-reasoning > summary { min-height: 44px; } }
    @media (prefers-reduced-motion: reduce) { .standalone-reasoning > summary, .reasoning-chevron { transition: none; } }
</style>
