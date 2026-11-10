import { ApiError, api as requestAPI } from './api.ts';
import { sameEditedResendInput, parseEditedResendEnvelope, confirmEditedResendAdmission } from './editedResend.ts';
import type { EditedResendDraft, EditedResendInput, EditedResendEnvelope, PreparedEditedResend, EditedResendSourceSnapshot } from './editedResend.ts';
import { parseSourceReceipt } from './sourceSubmission.ts';
import type { SourceSubmissionReceipt } from './sourceSubmission.ts';

export interface ExpertParticipantReference {participant_id:string;agent_id:string;agent_revision:number;context_conversation_id:string}
export interface AdmitExpertMessageInput {
    conversation_id:string;expected_revision:number;room_id:string;expected_room_revision:number;
    submission_id:string;user_message_id:string;text:string;selected_participants:ExpertParticipantReference[];created_at_ms:number;
}
export interface ExpertMessageEntry {
    participant:ExpertParticipantReference;display_name:string;description:string;defaults:Record<string,unknown>;
    model_declaration:{profile_id:string;profile_revision:number;provider:string;model_name:string;runtime:string;parameters:Record<string,unknown>};
    context_configuration_revision:number;child_submission_id:string;child_user_message_id:string;child_assistant_message_id:string;
    expected_prompt_definition:unknown;model_witness:unknown;instruction_witness:unknown;
}
export interface ExpertMessageManifest {version:number;input:AdmitExpertMessageInput;entries:ExpertMessageEntry[]}
export interface PreparedExpertResend {
    input:EditedResendInput;source_submission_id:string;room_id:string;parent_message_id:string;
    entries:{original:ExpertMessageEntry;participant:ExpertParticipantReference;prepared:PreparedEditedResend}[];
}
export interface EditedExpertResendEnvelope {
    host_id:string;client_id:string;prepared:PreparedExpertResend;target:EditedResendEnvelope['target'];
}
export interface EditedExpertResendCompletion {envelope:EditedExpertResendEnvelope;receipts:SourceSubmissionReceipt[]}
type Owner={hostId:string;clientId:string};
const object=(v:unknown):v is Record<string,unknown>=>!!v&&typeof v==='object'&&!Array.isArray(v);
const positive=(v:unknown):v is number=>Number.isSafeInteger(v)&&Number(v)>0;
const id=(v:unknown):v is string=>typeof v==='string'&&v.trim()===v&&v.length>0&&v.length<=512&&!/[\u0000-\u001f]/.test(v);
function sameData(a:unknown,b:unknown):boolean {
    if(Array.isArray(a)&&Array.isArray(b))return a.length===b.length&&a.every((v,i)=>sameData(v,b[i]));
    if(object(a)&&object(b)){const keys=Object.keys(a);return keys.length===Object.keys(b).length&&keys.every(k=>Object.hasOwn(b,k)&&sameData(a[k],b[k]));}
    return a===b;
}
function participant(v:unknown):v is ExpertParticipantReference {
    return object(v)&&Object.keys(v).length===4&&id(v.participant_id)&&id(v.agent_id)&&positive(v.agent_revision)&&id(v.context_conversation_id);
}
function entry(v:unknown):v is ExpertMessageEntry {
    return object(v)&&Object.keys(v).length===12&&participant(v.participant)&&typeof v.display_name==='string'
        &&typeof v.description==='string'&&object(v.defaults)&&object(v.model_declaration)&&positive(v.context_configuration_revision)
        &&id(v.child_submission_id)&&v.child_user_message_id===`${v.child_submission_id}:user`
        &&v.child_assistant_message_id===`${v.child_submission_id}:assistant`
        &&Object.hasOwn(v,'expected_prompt_definition')&&Object.hasOwn(v,'model_witness')&&Object.hasOwn(v,'instruction_witness');
}
function owner(value:unknown,expected:Owner):asserts value is Record<string,unknown> {
    if(!object(value)||value.host_id!==expected.hostId||value.client_id!==expected.clientId)throw new Error('Expert Host changed');
}
function manifest(value:unknown):ExpertMessageManifest {
    if(!object(value)||value.version!==1||!object(value.input)||!Array.isArray(value.entries)||!value.entries.length
        ||!Array.isArray(value.input.selected_participants)||value.entries.length!==value.input.selected_participants.length
        ||!value.entries.every(entry)||!value.input.selected_participants.every(participant)
        ||value.entries.some((v,i)=>!sameData(v.participant,(value.input as unknown as AdmitExpertMessageInput).selected_participants[i]))) {
        throw new Error('Incomplete expert manifest');
    }
    return structuredClone(value) as unknown as ExpertMessageManifest;
}
function query(o:Owner){return new URLSearchParams({expected_host_id:o.hostId,client_id:o.clientId});}
export async function loadExpertManifest(source:EditedResendSourceSnapshot,request:typeof requestAPI=requestAPI):Promise<ExpertMessageManifest|null>{
    const value:unknown=await request(`/api/chat/conversations/${encodeURIComponent(source.conversationId)}/expert-messages/${encodeURIComponent(source.messageId)}?${query(source)}`);
    owner(value,source);
    if(value.manifest===null)return null;
    const plan=manifest(value.manifest);
    if(plan.input.conversation_id!==source.conversationId||plan.input.user_message_id!==source.messageId||plan.input.text!==source.originalText)throw new Error('Expert source changed');
    return plan;
}
export async function admitExpertMessage(input:AdmitExpertMessageInput,o:Owner,request:typeof requestAPI=requestAPI):Promise<ExpertMessageManifest>{
    const value=await request('/api/chat/expert-messages/admit',{method:'POST',body:JSON.stringify({expected_host_id:o.hostId,client_id:o.clientId,input})});
    owner(value,o);const plan=manifest(value.manifest);
    // This is an atomic plan admission, never a claim that child Runs completed.
    if(!sameData(plan.input,input))throw new Error('Expert admission intent changed');
    return plan;
}
function expertEnvelope(value:unknown,draft:EditedResendDraft):EditedExpertResendEnvelope{
    owner(value,draft.source);
    if(Object.keys(value).length!==4||!object(value.prepared)||!object(value.target))throw new Error('Missing expert preparation');
    const p=value.prepared,t=value.target;
    if(Object.keys(p).length!==5||!object(p.input)||!sameEditedResendInput(p.input as unknown as EditedResendInput,draft.input)
        ||!id(p.source_submission_id)||!id(p.room_id)||p.parent_message_id!==`${draft.input.operation_id}:user`
        ||t.id!==draft.input.target_conversation_id||typeof t.title!=='string'||!positive(t.revision)||!positive(t.configuration_revision)
        ||!Array.isArray(p.entries)||!p.entries.length||p.entries.some(v=>!object(v)||!entry(v.original)||!participant(v.participant)||!object(v.prepared)
            ||!object(v.prepared.input)||v.prepared.input.target_conversation_id!==v.participant.context_conversation_id))throw new Error('Expert preparation owner mismatch');
    return structuredClone(value) as unknown as EditedExpertResendEnvelope;
}
export async function completeEditedExpertResend(draft:EditedResendDraft,options:{
    controller:(envelope:EditedResendEnvelope)=>Promise<{submitPreparedEditedResend(envelope:EditedResendEnvelope,language:string):Promise<SourceSubmissionReceipt>}>;
    language:string;request?:typeof requestAPI;isCurrent?:()=>boolean;
}):Promise<EditedExpertResendCompletion>{
    const request=options.request??requestAPI;
    const check=()=>{if(options.isCurrent&&!options.isCurrent())throw new Error('Expert edit owner changed');};
    check();const source=await loadExpertManifest(draft.source,request);
    if(!source)throw new Error('Complete expert source is unavailable');
    check();const envelope=expertEnvelope(await request('/api/chat/edited-expert-resend/prepare',{method:'POST',body:JSON.stringify({
        expected_host_id:draft.source.hostId,client_id:draft.source.clientId,input:draft.input})}),draft);
    if(envelope.prepared.source_submission_id!==source.input.submission_id||envelope.prepared.entries.length!==source.entries.length
        ||envelope.prepared.entries.some((child,i)=>!sameData(child.original,source.entries[i])
            ||child.participant.agent_id!==source.entries[i].participant.agent_id
            ||child.participant.agent_revision!==source.entries[i].participant.agent_revision
            ||!sameData(child.prepared.source,source.entries[i].model_witness)
            ||child.prepared.input.edited_text!==draft.input.edited_text
            ||child.prepared.input.source_conversation_id!==source.entries[i].participant.context_conversation_id
            ||child.prepared.input.source_message_id!==source.entries[i].child_user_message_id)
        ||new Set(envelope.prepared.entries.map(child=>child.prepared.run_id)).size!==source.entries.length
        ||new Set(envelope.prepared.entries.map(child=>child.participant.context_conversation_id)).size!==source.entries.length) {
        throw new Error('Prepared expert collection differs from the complete original source');
    }
    const receipts:SourceSubmissionReceipt[]=[];
    const childOwner={...draft.source,clientId:`room:${envelope.prepared.room_id}`};
    try{
        for(const child of envelope.prepared.entries){
            check();const prepared=child.prepared;
            const raw=await request(`/api/chat/conversations/${encodeURIComponent(prepared.input.target_conversation_id)}/edited-resend?${query(childOwner)}`);
            const exact=parseEditedResendEnvelope(raw,childOwner,prepared.input);
            if(exact.prepared.run_id!==prepared.run_id||exact.prepared.user_message_id!==prepared.user_message_id||exact.prepared.assistant_message_id!==prepared.assistant_message_id)throw new Error('Expert child changed');
            check();const controller=await options.controller(exact);check();
            const receipt=await controller.submitPreparedEditedResend(exact,options.language);
            const childDraft:EditedResendDraft={source:childOwner,input:prepared.input};
            const current=parseEditedResendEnvelope(await request(`/api/chat/conversations/${encodeURIComponent(prepared.input.target_conversation_id)}/edited-resend?${query(childOwner)}`),childOwner,prepared.input);
            confirmEditedResendAdmission(childDraft,current,receipt);receipts.push(receipt);
        }
        const current=expertEnvelope(await request(`/api/chat/conversations/${encodeURIComponent(draft.input.target_conversation_id)}/edited-expert-resend?${query(draft.source)}`),draft);
        if(current.prepared.entries.length!==envelope.prepared.entries.length||current.prepared.entries.some((child,i)=>{
            const original=envelope.prepared.entries[i];return !sameData(child.original,original.original)||!sameData(child.participant,original.participant)||!sameEditedResendInput(child.prepared.input,original.prepared.input)
                ||child.prepared.run_id!==original.prepared.run_id||child.prepared.admitted_run_id!==original.prepared.run_id;
        }))throw new Error('Complete expert admission is not confirmed');
        return {envelope:current,receipts};
    }catch(cause){throw new Error(`Expert admission is partial (${receipts.length}/${envelope.prepared.entries.length}). Retry this edit to resume the same intent. ${cause instanceof Error?cause.message:String(cause)}`);}
}

export function confirmEditedExpertResendCompletion(draft:EditedResendDraft,completion:EditedExpertResendCompletion):EditedExpertResendCompletion {
    const envelope=expertEnvelope(completion.envelope,draft);
    if(completion.receipts.length!==envelope.prepared.entries.length)throw new Error('Incomplete expert receipts');
    envelope.prepared.entries.forEach((child,i)=>{
        const p=child.prepared,receipt=completion.receipts[i];
        const accepted=parseSourceReceipt(receipt,{source_host_id:draft.source.hostId,client_id:`room:${envelope.prepared.room_id}`,
            conversation_id:p.input.target_conversation_id,submission_id:p.run_id,user_message_id:p.user_message_id,
            assistant_message_id:p.assistant_message_id},receipt.request_digest);
        if(accepted.phase!=='admitted'||p.admitted_run_id!==p.run_id)throw new Error('Expert child has not been admitted');
    });
    return completion;
}

/** UI intent persistence only. Core still owns admission, snapshots and all Runs. */
export function expertIntentKey(o:Owner,conversationId:string):string {
    return `nola.web.expert-intent/${encodeURIComponent(o.hostId)}/${encodeURIComponent(o.clientId)}/${encodeURIComponent(conversationId)}`;
}
export function loadExpertIntent(storage:Pick<Storage,'getItem'>,o:Owner,conversationId:string):AdmitExpertMessageInput|null {
    const raw=storage.getItem(expertIntentKey(o,conversationId));if(raw===null)return null;
    const input:unknown=JSON.parse(raw);
    if(!object(input)||Object.keys(input).length!==9||input.conversation_id!==conversationId||!positive(input.expected_revision)
        ||!id(input.room_id)||!positive(input.expected_room_revision)||!id(input.submission_id)||!id(input.user_message_id)
        ||typeof input.text!=='string'||!input.text.trim()||!Number.isSafeInteger(input.created_at_ms)||Number(input.created_at_ms)<0
        ||!Array.isArray(input.selected_participants)||!input.selected_participants.length||!input.selected_participants.every(participant))throw new Error('Stored expert intent is invalid');
    return input as unknown as AdmitExpertMessageInput;
}
export function saveExpertIntent(storage:Pick<Storage,'setItem'|'getItem'>,o:Owner,input:AdmitExpertMessageInput):void {
    const key=expertIntentKey(o,input.conversation_id),raw=JSON.stringify(input);storage.setItem(key,raw);
    if(storage.getItem(key)!==raw)throw new Error('Expert intent could not be saved');
}
export function clearExpertIntent(storage:Pick<Storage,'getItem'|'removeItem'>,o:Owner,input:AdmitExpertMessageInput):void {
    const stored=loadExpertIntent(storage,o,input.conversation_id);
    if(stored&&!sameData(stored,input))throw new Error('A different expert intent owns this draft');
    const key=expertIntentKey(o,input.conversation_id);storage.removeItem(key);
    if(storage.getItem(key)!==null)throw new Error('Expert intent acknowledgement could not be saved');
}

/** Explicit cold recovery of a prepared expert parent; absence means an ordinary
 * expert message. No current composer or new identity is used. */
export async function resumePreparedExpertMessage(visible:ExpertMessageManifest,o:Owner,options:Parameters<typeof completeEditedExpertResend>[1]):Promise<boolean>{
    const request=options.request??requestAPI;let raw:unknown;
    try{raw=await request(`/api/chat/conversations/${encodeURIComponent(visible.input.conversation_id)}/edited-expert-resend?${query(o)}`);}
    catch(cause){if(cause instanceof ApiError&&cause.status===404&&cause.code==='expert_message_rejected'&&cause.message==='expert_message_not_found')return false;throw cause;}
    owner(raw,o);
    if(!object(raw.prepared)||!object(raw.prepared.input))throw new Error('Prepared expert recovery intent is invalid');
    const input=raw.prepared.input as unknown as EditedResendInput;
    const sourceRaw=await request(`/api/chat/conversations/${encodeURIComponent(input.source_conversation_id)}/expert-messages/${encodeURIComponent(input.source_message_id)}?${query(o)}`);
    owner(sourceRaw,o);const original=manifest(sourceRaw.manifest);
    const draft:EditedResendDraft={input,source:{...o,conversationId:input.source_conversation_id,conversationRevision:input.expected_source_revision,
        messageId:input.source_message_id,messageRevision:input.expected_source_message_revision,originalText:original.input.text}};
    const envelope=expertEnvelope(raw,draft);
    if(envelope.target.id!==visible.input.conversation_id||envelope.prepared.parent_message_id!==visible.input.user_message_id
        ||envelope.prepared.entries.length!==visible.entries.length||envelope.prepared.entries.some((child,i)=>
            child.prepared.run_id!==visible.entries[i].child_submission_id||!sameData(child.participant,visible.entries[i].participant)))throw new Error('Prepared expert recovery collection changed');
    await completeEditedExpertResend(draft,options);return true;
}
