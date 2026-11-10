<script lang="ts">
    import {getContext} from 'svelte';
    import {locale,t} from '../lib/i18n';
    import {MESSAGE_PART_READING,readingPartKey,type ReadingField,type MessagePartReading,type MessageTextSnapshot} from '../lib/messagePartReading';
    import type {TimelinePart} from '../lib/chatTimeline';
    import ChatTextReaderDialog from './ChatTextReaderDialog.svelte';
    let {text,title,part,field}:{text:string;title:string;part:TimelinePart;field:ReadingField}=$props();
    const messageReading=getContext<MessagePartReading|undefined>(MESSAGE_PART_READING);
    const reading=$derived(messageReading?.bind(part));
    let localSnapshot=$state.raw<MessageTextSnapshot|null>(null);
    function open(event:MouseEvent){
        if(reading)reading.open(field,title,text,event.currentTarget as HTMLButtonElement);
        else localSnapshot=Object.freeze({partKey:readingPartKey(part),field,title,text});
    }
</script>

<button type="button" class="reader-open" data-reader-part={readingPartKey(part)} data-reader-field={field}
    aria-label={t($locale,'chatPage.openTextReader')+' · '+title} onclick={open}>{t($locale,'chatPage.openTextReader')}</button>
{#if localSnapshot}{@const shown=localSnapshot}{#key shown}<ChatTextReaderDialog snapshot={shown} onclose={()=>{if(localSnapshot===shown)localSnapshot=null;}} />{/key}{/if}
<style>.reader-open{margin-top:8px;min-height:44px;}</style>
