// Read-only adapter over the same admissions/journal/terminal consumers as Chat.
// It has no submission, recovery, queue, draft or permission-decision side effects.
import {AssistantJournalReader, parseJournalCursor, type JournalTransport, type JournalUpdate} from './assistantJournal.ts';
import {readTerminal} from './assistantTerminal.ts';
import {createChatTimeline, reduceChatTimeline, timelineRequest, type ChatTimeline, type TimelineMessage} from './chatTimeline.ts';
import {createChatTurn, reduceChatTurn, type ChatTurn, type ChatTurnOwner} from './chatTurn.ts';
import type {RunObservation} from './runObservation.ts';
import {messageWindowUrl} from './chatConversations.ts';

export interface RoomReplyReadOwner extends ChatTurnOwner {runId:string; assistantMessageId:string}
export function createRoomReplyProjection(owner:RoomReplyReadOwner,
    request:<T>(path:string,options?:RequestInit)=>Promise<T>, publish:(message:TimelineMessage)=>void, seed:TimelineMessage|null=null) {
    const lifetime=new AbortController();
    const current=()=>!lifetime.signal.aborted;
    const journal:JournalTransport=(body,signal)=>request('/api/chat/agent/v2/journal',{
        method:'POST',body:JSON.stringify(body),signal:signal?AbortSignal.any([signal,lifetime.signal]):lifetime.signal});
    const reader=new AssistantJournalReader(owner.hostId,owner.conversationId,owner.generation,journal);
    let timeline:ChatTimeline=createChatTimeline(owner);
    let turn:ChatTurn|null=null;
    let retained=seed;
    function changed(next:ChatTimeline,authenticatedParts=false) {
        if (!current()) return;
        timeline=next;
        const message=timeline.messages.find(item=>item.id===owner.assistantMessageId);
        // Admission shells/history previews must not blank a hydrated message
        // while a returning owner reacquires its exact journal watermark.
        if(message&&(!retained?.parts.length||authenticatedParts)) {retained=message;publish(message);}
    }
    function apply(update:JournalUpdate|null) {
        if(!update||!current())return;
        const result=reduceChatTimeline(timeline,{type:'journal',owner,sourceHostId:owner.hostId,runId:owner.runId,update});
        if(result.ignored&&result.ignored!=='duplicate')throw new Error('Expert journal owner was rejected');
        changed(result.state,true);
    }
    async function history() {
        let before:number|null=null;
        let snapshot:ChatTimeline['window']=null;
        const visited=new Set<number>();
        for(let page=0;page<64&&current();page++) {
            const captured=timelineRequest(timeline);
            const payload=await request<unknown>(messageWindowUrl(owner.conversationId,before,page?200:48,snapshot??undefined),{signal:lifetime.signal});
            if(!current())return;
            const envelope=payload as {sourceHostId?:unknown;assistantJournalHeads?:unknown};
            if(envelope.sourceHostId!==owner.hostId)throw new Error('Expert history source changed');
            const result=reduceChatTimeline(timeline,{type:'history',request:captured,mode:page?'prepend':'replace',payload});
            if(result.ignored==='stale_request')return;
            if(result.ignored)throw new Error('Expert history snapshot changed');
            changed(result.state);
            if(Array.isArray(envelope.assistantJournalHeads)) {
                for(const raw of envelope.assistantJournalHeads) {
                    const cursor=parseJournalCursor(raw);
                    if(cursor.identity.conversation_id!==owner.conversationId)throw new Error('Expert history head conversation changed');
                    if(cursor.identity.run_id===owner.runId&&cursor.identity.assistant_message_id===owner.assistantMessageId) {
                        apply(await reader.read(cursor,owner.runId));
                        if(!current())return;
                    }
                }
            }
            const window=timeline.window;
            if(timeline.messages.some(item=>item.id===owner.assistantMessageId)||!window?.hasMore)return;
            if(window.before===null||visited.has(window.before))throw new Error('Expert history cursor did not advance');
            visited.add(window.before);before=window.before;snapshot=window;
        }
        if(current())throw new Error('Expert history read budget exceeded');
    }
    async function accept(value:RunObservation):Promise<boolean> {
        if(!current())return false;
        if(value.conversation_id!==owner.conversationId||value.run_id!==owner.runId||value.assistant_message_id!==owner.assistantMessageId||!value.admission)
            throw new Error('Expert exact Run admission is unavailable');
        const declarations=await reader.admit(value.admission.identity,value.admission);
        if(!current())return false;
        changed(reduceChatTimeline(timeline,{type:'admissions',owner,messages:declarations}).state);
        if(!turn) {
            const user=declarations.find(item=>item.input.role==='user');
            if(!user)throw new Error('Expert Run has no admitted input');
            turn=createChatTurn(owner,{submissionId:owner.runId,userMessageId:user.input.id,assistantMessageId:owner.assistantMessageId});
            turn=reduceChatTurn(turn,{type:'run_bound',owner,runId:owner.runId}).state;
        }
        let after:string|null=null;
        const seen=new Set<string>();
        for(let page=0;page<64;page++) {
            const response=await journal({operation:'heads',conversation_id:owner.conversationId,run_id:owner.runId,after_message_id:after,limit:32});
            if(!current())return false;
            const envelope=response as {source_host_id?:unknown;result?:unknown};
            if(envelope.source_host_id!==owner.hostId||!Array.isArray(envelope.result)||envelope.result.length>32)throw new Error('Expert journal source changed');
            for(const raw of envelope.result) {
                const cursor=parseJournalCursor(raw);
                if(cursor.identity.conversation_id!==owner.conversationId||cursor.identity.run_id!==owner.runId||seen.has(cursor.identity.assistant_message_id))throw new Error('Expert head owner/cursor changed');
                seen.add(cursor.identity.assistant_message_id);
                apply(await reader.read(cursor,owner.runId));
                if(!current())return false;
                after=cursor.identity.assistant_message_id;
            }
            if(envelope.result.length<32)break;
            if(page===63)throw new Error('Expert head read budget exceeded');
        }
        if(value.terminal_seal) {
            const digest=value.terminal_seal.seal_digest;
            if(typeof digest!=='string')throw new Error('Expert terminal seal is unavailable');
            const evidence=await readTerminal(turn,reader,journal,apply,current,digest,owner.runId);
            if(!current())return false;
            if(!evidence)throw new Error('Expert terminal is not verified');
            const terminal=reduceChatTimeline(timeline,{type:'terminal',owner,evidence});
            if(terminal.ignored&&terminal.ignored!=='duplicate')throw new Error('Expert terminal frontier is incomplete');
            changed(terminal.state,true);
            turn=reduceChatTurn(turn,{type:'terminal_verified',owner,evidence}).state;
            return false;
        }
        return true;
    }
    return {history,accept,dispose(){lifetime.abort();reader.dispose();}};
}
