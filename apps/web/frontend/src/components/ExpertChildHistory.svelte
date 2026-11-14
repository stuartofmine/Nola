<script lang="ts">
    import { onDestroy } from 'svelte';
    import { api } from '../lib/api';
    import { roomCommandAtSource } from '../lib/agentRooms';
    import { parseExpertChildPage,expertChildHistoryPayload,expertChildContinuationUrl,type ExpertChildConversation } from '../lib/expertChildConversations';
    import { createChatTimeline,timelineRequest,reduceChatTimeline } from '../lib/chatTimeline';
    import { navigateWorkspace } from '../lib/workspaceNavigation';
    import { locale,t } from '../lib/i18n';
    import ChatMessageContent from './ChatMessageContent.svelte';
    import ChatReadOnlySurfacePart from './ChatReadOnlySurfacePart.svelte';
    let {parentId,disabled=false}:{parentId:string;disabled?:boolean}=$props();
    let expanded=$state(false),loading=$state(false),error=$state(''),host=$state('');
    let children=$state<ExpertChildConversation[]>([]),next=$state<number|null>(0),roomId=$state('');
    let sourceEpoch=$state<string|null>(null);
    let sourceIdentity=$state(''),senders=$state<Record<string,{name:string;agentId:string}>>({});
    let selected=$state<ExpertChildConversation|null>(null),historyLoading=$state(false),historyError=$state('');
    let timeline=$state<ReturnType<typeof createChatTimeline>|null>(null);
    let dialog:HTMLDialogElement,closeButton:HTMLButtonElement;
    let alive=true,serial=0,historySerial=0;const controller=new AbortController();let opener:HTMLElement|undefined;
    onDestroy(()=>{alive=false;++serial;++historySerial;controller.abort();if(dialog?.open)dialog.close();});
    async function sessionHost(){const session=await api<{hostId:string}>('/api/web/session',{signal:controller.signal});if(!session.hostId)throw Error('Invalid Host');return session.hostId;}
    async function load(reset=false){
        if(loading||!reset&&next===null)return;const request=++serial,parent=parentId,offset=reset?0:next!;loading=true;error='';
        try{const source=await sessionHost();if(host&&host!==source)throw Error('Host changed');
            const scoped=await roomCommandAtSource({type:'child_conversations',conversation_id:parent,offset,limit:40},sourceEpoch,controller.signal);
            const page=parseExpertChildPage(scoped.result,parent,offset);
            if(await sessionHost()!==source)throw Error('Host changed');
            if(!alive||request!==serial||parent!==parentId)return;
            if(!reset&&page.children.some(c=>children.some(old=>old.conversation_id===c.conversation_id)))throw Error('History changed; reopen the list');
            if(roomId&&roomId!==page.room_id)throw Error('Room changed');
            host=source;sourceEpoch=scoped.sourceEpoch;roomId=page.room_id??'';children=reset?page.children:[...children,...page.children];next=page.next_offset;
        }catch{if(alive&&request===serial)error=t($locale,'rooms.childLoadFailed');}finally{if(alive&&request===serial)loading=false;}
    }
    async function open(child:ExpertChildConversation,event:MouseEvent){
        opener=event.currentTarget as HTMLElement;selected=child;timeline=createChatTimeline({conversationId:child.conversation_id,generation:++historySerial});historyError='';sourceIdentity='';senders={};
        dialog.showModal();closeButton.focus();await readHistory();
    }
    function close(){++historySerial;selected=null;timeline=null;historyLoading=false;dialog.close();opener?.focus();}
    async function readHistory(){
        if(!selected||!timeline||historyLoading)return;
        const child=selected,request=historySerial,state=timeline;historyLoading=true;historyError='';
        try{if(await sessionHost()!==host)throw Error('Host changed');
            const window=state.window;const scoped=await roomCommandAtSource({type:'child_history',conversation_id:parentId,child_conversation_id:child.conversation_id,
                before_ordinal:window?.before??null,limit:50,content_byte_limit:262144},sourceEpoch,controller.signal);
            if(scoped.sourceEpoch!==sourceEpoch)throw Error('Expert source changed');const raw=scoped.result;
            const decoded=expertChildHistoryPayload(raw,parentId,child,roomId,host),payload=decoded.payload;
            if(sourceIdentity&&sourceIdentity!==decoded.sourceIdentity||await sessionHost()!==host)throw Error('Host changed');
            if(!alive||request!==historySerial||selected!==child)return;
            const result=reduceChatTimeline(state,{type:'history',request:timelineRequest(state),mode:window?'prepend':'replace',payload});
            if(result.ignored)throw Error(result.ignored);timeline=result.state;sourceIdentity=decoded.sourceIdentity;senders={...senders,...decoded.senders};
        }catch{if(alive&&request===historySerial)historyError=t($locale,'rooms.childLoadFailed');}finally{if(alive&&request===historySerial)historyLoading=false;}
    }
    function restartHistory(){if(!selected||historyLoading)return;timeline=createChatTimeline({conversationId:selected.conversation_id,generation:++historySerial});sourceIdentity='';senders={};void readHistory();}
</script>
<div class="child-tree">
    <button class="disclosure" {disabled} aria-expanded={expanded} onclick={()=>{expanded=!expanded;if(expanded&&!children.length)void load();}}>{t($locale,'rooms.childHistory')} <span aria-hidden="true">{expanded?'−':'+'}</span></button>
    {#if expanded}
        <div class="children" aria-busy={loading}>
            {#each children as child(child.conversation_id)}<button {disabled} class="child" onclick={event=>open(child,event)}><span>{child.title||t($locale,'rooms.experts')}</span><small>{t($locale,child.is_current?'rooms.childCurrent':'rooms.childArchived')} · {child.message_count}</small></button>{/each}
            {#if error}<p role="alert">{error}</p><button {disabled} onclick={()=>load(true)}>{t($locale,'rooms.refresh')}</button>
            {:else if next!==null}<button disabled={disabled||loading} onclick={()=>load()}>{loading?t($locale,'rooms.loading'):t($locale,'rooms.childMore')}</button>
            {:else if !children.length}<p>{t($locale,'rooms.childEmpty')}</p>{/if}
        </div>
    {/if}
</div>
<dialog bind:this={dialog} aria-label={t($locale,'rooms.childHistory')} oncancel={event=>{event.preventDefault();close();}}>
    <header><h2>{selected?.title||t($locale,'rooms.childHistory')}</h2><button disabled={historyLoading} onclick={restartHistory}>{t($locale,'rooms.refresh')}</button><button bind:this={closeButton} onclick={close}>{t($locale,'rooms.close')}</button></header>
    <p class="muted">{t($locale,'rooms.childReadOnly')}</p>
    {#if selected?.is_current&&selected.active&&selected.agent_revision!==null}<button disabled={disabled||historyLoading||!timeline?.window||!!historyError} onclick={()=>{if(selected)navigateWorkspace(expertChildContinuationUrl(parentId,host,selected,sourceEpoch??undefined));}}>{t($locale,'rooms.childContinue')}</button>{/if}
    <!-- svelte-ignore a11y_no_noninteractive_tabindex (Keyboard users must reach the bounded transcript.) -->
    <section class="transcript" tabindex="0" aria-label={t($locale,'rooms.childHistory')}>
        {#if historyError}<p role="alert">{historyError}</p><button onclick={restartHistory}>{t($locale,'rooms.refresh')}</button>{/if}
        {#if timeline?.window?.hasMore&&!historyError}<button disabled={historyLoading} onclick={readHistory}>{t($locale,'rooms.childMore')}</button>{/if}
        {#each timeline?.messages??[] as message(message.id)}<article class:user={message.role==='user'}>{#if Object.hasOwn(senders,message.id)}<small class="muted" title={senders[message.id].agentId}>{senders[message.id].name||t($locale,'rooms.experts')}</small>{/if}<ChatMessageContent {message} ownerKey={JSON.stringify([host,selected?.conversation_id])}>
            {#snippet surface(part)}<ChatReadOnlySurfacePart {part} hostId={host} conversationId={selected!.conversation_id} messageId={message.id} active={true}/>{/snippet}
        </ChatMessageContent></article>{/each}
        {#if historyLoading}<p role="status">{t($locale,'rooms.loading')}</p>{:else if !historyError&&timeline?.window&&!timeline.messages.length}<p>{t($locale,'rooms.childEmpty')}</p>{/if}
    </section>
</dialog>
<style>
.child-tree{padding-inline-start:20px;min-width:0}.disclosure,.child{border:0;background:transparent;text-align:start;min-height:44px;width:100%;padding:8px 12px}.disclosure{display:flex;justify-content:space-between;color:var(--muted);font-size:12px}.children{border-inline-start:1px solid var(--border);margin:0 8px 8px}.child{display:grid;gap:4px}.child span{overflow-wrap:anywhere}.child small,.muted{color:var(--muted);font-size:12px}p{padding:8px 12px;margin:0}dialog{position:fixed;inset:0;box-sizing:border-box;width:min(880px,calc(100vw - 32px));height:85dvh;max-height:none;background:var(--panel);color:inherit;border:1px solid var(--border);border-radius:var(--radius);padding:16px}dialog[open]{display:flex;flex-direction:column;gap:12px}dialog::backdrop{background:rgb(0 0 0/.4)}header{display:flex;align-items:center;gap:12px}h2{flex:1;min-width:0;overflow-wrap:anywhere;font-size:16px;margin:0}button{min-height:44px}.transcript{min-height:0;overflow:auto;overscroll-behavior:contain;flex:1}article{padding:16px 8px;min-width:0;overflow-wrap:anywhere}.user{background:var(--canvas);border-radius:var(--radius)}@media(max-width:600px){dialog{width:100%;height:100dvh;max-width:none;margin:0;border-radius:0;padding:max(12px,env(safe-area-inset-top)) max(12px,env(safe-area-inset-right)) max(12px,env(safe-area-inset-bottom)) max(12px,env(safe-area-inset-left))}}
</style>
