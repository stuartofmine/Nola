<script lang="ts">
    import {untrack} from 'svelte';
    import {api} from '../lib/api';
    import {locale,t} from '../lib/i18n';
    import {conversationHref} from '../lib/conversations';
    import type {TimelineMessage} from '../lib/chatTimeline';
    import {timelineMessageText} from '../lib/chatTimeline';
    import {RunObserver} from '../lib/runObservation';
    import {createRoomReplyProjection} from '../lib/roomReplyProjection';
    import ChatMessageContent from './ChatMessageContent.svelte';
    import ChatReadOnlySurfacePart from './ChatReadOnlySurfacePart.svelte';
    import ChatPermissionPanel from './ChatPermissionPanel.svelte';
    import Markdown from './Markdown.svelte';
    let {contextId,executionId,running,hostId,roomId,hideContext=false,active=true,compact=false,permissionInteractive=true,message=$bindable<TimelineMessage|null>(null)}:{contextId:string;executionId:string;running:boolean;hostId:string;roomId:string;hideContext?:boolean;active?:boolean;compact?:boolean;permissionInteractive?:boolean;message?:TimelineMessage|null}=$props();
    let permissions=$state<Record<string,unknown>[]>([]);
    let failed=$state(false);
    let visible=$state(typeof document==='undefined'||!document.hidden);
    let displayOwner='';
    let generation=0;
    $effect(()=>{
        const key=JSON.stringify([hostId,roomId,contextId,executionId]);
        const isRunning=running;
        if(key!==untrack(()=>displayOwner)){displayOwner=key;message=null;}
        permissions=[];
        if(!active||!visible||!hostId||!contextId||!executionId)return;
        const owner={hostId,clientId:`room:${roomId}`,conversationId:contextId,generation:++generation,
            runId:executionId,assistantMessageId:`${executionId}:assistant`};
        const observer=new RunObserver();
        const projection=createRoomReplyProjection(owner,api,value=>{
            if(!observer.signal.aborted){message=value;failed=false;}
        },untrack(()=>message));
        failed=false;
        // Initial bounded history is optional presentation hydration. Live
        // admissions and journal deltas do not wait for a long history scan.
        void projection.history().catch(()=>{if(!observer.signal.aborted&&!isRunning)failed=true;});
        void observer.run(owner,(body,signal)=>api('/api/chat/agent/v2/observe',{
            method:'POST',body:JSON.stringify(body),signal}),async value=>{
                if(observer.signal.aborted)return false;
                if(!value)throw new Error('Expert Run is unavailable');
                permissions=value.permissions;
                const keepObserving=await projection.accept(value);
                if(!observer.signal.aborted){failed=false;if(!keepObserving){permissions=[];observer.stop();projection.dispose();}}
                return keepObserving;
            },()=>{if(!observer.signal.aborted){permissions=[];failed=true;}},
            {run_id:owner.runId,assistant_message_id:owner.assistantMessageId});
        return()=>{observer.stop();projection.dispose();};
    });
</script>
<svelte:document onvisibilitychange={()=>visible=!document.hidden} />
{#if message && compact}<Markdown source={timelineMessageText(message)} />
{:else if message}{@const renderedMessage = message}{#key JSON.stringify([hostId, contextId, executionId, renderedMessage.id])}<ChatMessageContent message={renderedMessage} ownerKey={JSON.stringify([hostId,contextId,executionId])} readingActive={active}>
    {#snippet surface(part)}
        <ChatReadOnlySurfacePart {part} {hostId} conversationId={contextId} messageId={renderedMessage.id} active={active && visible} />
    {/snippet}
</ChatMessageContent>{/key}{/if}
{#if permissionInteractive}{#each permissions as challenge (`${challenge.request_id}:${challenge.nonce}`)}
    <ChatPermissionPanel {challenge} runId={executionId} active={active&&visible&&running}
        reviewText={typeof challenge.safe_review_text==='string'?challenge.safe_review_text:''} />
{/each}{/if}
{#if failed}<p role="status">{t($locale,'rooms.failed')}</p>{/if}
{#if !hideContext}<a href={conversationHref(contextId)}>{t($locale,'rooms.context')}</a>{/if}
