import type { RoomSnapshot } from './agentRooms.ts';
export interface ExpertChildConversation {
    conversation_id:string; revision:number; title:string; participant_id:string; agent_id:string;
    agent_revision:number|null; is_current:boolean; active:boolean; created_at_ms:number; updated_at_ms:number; message_count:number;
}
export interface ExpertChildPage { version:1; conversation_id:string; room_id:string|null; room_revision:number|null; children:ExpertChildConversation[]; has_more:boolean; next_offset:number|null }
const object=(v:unknown):v is Record<string,unknown>=>!!v&&typeof v==='object'&&!Array.isArray(v);
const id=(v:unknown):v is string=>typeof v==='string'&&v.trim().length>0;
const integer=(v:unknown):v is number=>Number.isSafeInteger(v)&&Number(v)>=0;
/** Never attribute a historical context to today's directory definition. */
export function parseExpertChildPage(value:unknown,parent:string,offset:number):ExpertChildPage {
    if(!object(value)||value.version!==1||value.conversation_id!==parent||!Array.isArray(value.children)
        ||!(value.room_id===null||id(value.room_id))||!(value.room_revision===null||integer(value.room_revision))
        ||typeof value.has_more!=='boolean'||!(value.next_offset===null||integer(value.next_offset)))throw new Error('Invalid expert history');
    const seen=new Set<string>();
    for(const child of value.children){
        if(!object(child)||!['conversation_id','participant_id','agent_id'].every(k=>id(child[k]))||child.conversation_id===parent
            ||typeof child.title!=='string'||!['revision','created_at_ms','updated_at_ms','message_count'].every(k=>integer(child[k]))
            ||!(child.agent_revision===null||integer(child.agent_revision))||typeof child.active!=='boolean'||typeof child.is_current!=='boolean'
            ||seen.has(child.conversation_id as string))throw new Error('Invalid expert history');
        seen.add(child.conversation_id as string);
    }
    if(value.has_more&&(value.children.length===0||value.next_offset!==offset+value.children.length)
        ||!value.has_more&&value.next_offset!==null)throw new Error('Invalid expert history page');
    return value as unknown as ExpertChildPage;
}
export function currentExpertChildMember(room:RoomSnapshot,child:ExpertChildConversation){
    return child.active&&child.is_current&&child.agent_revision!==null
        ?room.room.participants.find(p=>p.active&&p.id===child.participant_id&&p.agent_id===child.agent_id
            &&p.agent_revision===child.agent_revision&&p.context_conversation_id===child.conversation_id):undefined;
}
export function expertChildContinuationUrl(parent:string,host:string,child:ExpertChildConversation,sourceEpoch?:string):string {
    if(!child.active||!child.is_current||child.agent_revision===null)throw new Error('Historical expert context is read-only');
    return '/chat?'+new URLSearchParams({conversation:parent,sourceHost:host,expert_context:child.conversation_id,
        expert_participant:child.participant_id,expert_agent:child.agent_id,expert_revision:String(child.agent_revision),...(sourceEpoch?{expert_source_epoch:sourceEpoch}:{})});
}

/** Map explicit exchange provenance; ordinary user text never selects an author. */
export function expertChildExchangeSenders(value:unknown,child:ExpertChildConversation,parent:string):Record<string,{name:string;agentId:string}> {
    if(value===undefined)return {};
    if(!Array.isArray(value))throw new Error('Invalid expert exchange provenance');
    const authors:Record<string,{name:string;agentId:string}>={};
    for(const peer of value){
        if(!object(peer)||!id(peer.exchange_id)||peer.parent_conversation_id!==parent||!object(peer.target)
            ||peer.target.context_conversation_id!==child.conversation_id||peer.target.participant_id!==child.participant_id
            ||peer.target.agent_id!==child.agent_id||!object(peer.source))throw new Error('Expert exchange owner changed');
        const main = peer.source.kind === 'main_agent';
        if(peer.source.kind!==undefined&&!main)throw new Error('Unknown expert source');
        if(main ? peer.source.context_conversation_id!==parent : !id(peer.source.agent_id))throw new Error('Expert source changed');
        const messageId=`${peer.exchange_id}:user`;
        if(Object.hasOwn(authors,messageId))throw new Error('Duplicate expert exchange');
        authors[messageId]={name:typeof peer.source_display_name==='string'&&peer.source_display_name?peer.source_display_name:main?'Nola':'',agentId:main?parent:peer.source.agent_id as string};
    }
    return authors;
}

export function expertChildHistoryPayload(value:unknown,parent:string,child:ExpertChildConversation,roomId:string,hostId:string){
    if(!object(value)||value.version!==1||value.conversation_id!==parent||value.child_conversation_id!==child.conversation_id
        ||value.room_id!==roomId||!object(value.source_owner)||!object(value.window))throw new Error('Expert history owner changed');
    const source=value.source_owner;
    if(source.kind!=='local'&&!(source.kind==='remote'&&id(source.source_host_id)&&id(source.source_store_epoch)))throw new Error('Expert history source unavailable');
    const window=value.window;
    return {sourceIdentity:JSON.stringify(source),senders:expertChildExchangeSenders(value.expert_exchanges,child,parent),payload:{
        sourceHostId:hostId,conversationProjection:window.projection,
        messageWindow:{projectionRevision:object(window.projection)?window.projection.revision:null,eventSequence:window.event_sequence,
            before:window.before_ordinal,hasMore:window.has_more,totalCount:window.total_message_count},
    }};
}
