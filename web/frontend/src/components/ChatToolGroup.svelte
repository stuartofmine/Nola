<script lang="ts">
    import {getContext} from 'svelte';
    import {MESSAGE_PART_READING,type MessagePartReading} from '../lib/messagePartReading';
    import {executionProcessChoice,executionProcessExpanded} from '../lib/chatMessagePresentation';
    import { locale, t } from '../lib/i18n';
    import { timelineToolsNeedAttention } from '../lib/chatTimeline';
    import type { TimelinePart } from '../lib/chatTimeline';
    import ChatToolPart from './ChatToolPart.svelte';

    let { parts, completed = false, choices = {}, onexpandedchange }: { parts: TimelinePart[]; completed?: boolean;
        choices?: Record<string, boolean>; onexpandedchange?: (id: string, expanded: boolean) => void } = $props();
    let userExpanded = $state<boolean | null>(null);
    const messageReading=getContext<MessagePartReading|undefined>(MESSAGE_PART_READING);
    const readings=$derived(parts.map(part=>messageReading?.bind(part)));
    const groupChoice=$derived(executionProcessChoice(parts,Object.fromEntries(parts.flatMap((part,index)=>{
        const choice=readings[index]?.choice('group');
        return choice==null?[]:[[part.id,choice]];
    }))));
    let hadExpandedTool = $state(false);
    const hasExpandedTool = $derived(parts.some(part => choices[part.id] === true));
    $effect(() => { if (!messageReading&&hasExpandedTool) hadExpandedTool = true; });
    const expanded = $derived(executionProcessExpanded(groupChoice??(messageReading?null:userExpanded),timelineToolsNeedAttention(parts)||(!messageReading&&hadExpandedTool)||hasExpandedTool));
    function choose(expanded:boolean){
        if(messageReading)for(const reading of readings)reading?.choose('group',expanded);
        else userExpanded=expanded;
    }
</script>

<details open={expanded}>
    <summary onclick={event => { event.preventDefault(); choose(!expanded); }}>
        {t($locale, completed ? 'chatPage.completedTools' : 'chatPage.toolActivity')} · {parts.length}
    </summary>
    <ul>{#each parts as part (part.id)}<li><ChatToolPart {part} expandedChoice={Object.hasOwn(choices, part.id) ? choices[part.id] : undefined} onexpandedchange={expanded => onexpandedchange?.(part.id, expanded)} /></li>{/each}</ul>
</details>

<style>
    details { min-width: 0; margin: 6px 0; }
    summary { min-height: 44px; box-sizing: border-box; padding: 12px 8px; cursor: pointer; color: var(--muted); font-size: 12px; }
    summary:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; border-radius: 6px; }
    ul { list-style: none; padding: 0 0 0 10px; margin: 0; border-inline-start: 1px solid var(--border); }
    li { min-width: 0; }
</style>
