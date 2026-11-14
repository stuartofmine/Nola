import { api } from './api.ts';
import type { EditedResendDraft } from './editedResend.ts';
import type { ExpertParticipantReference } from './expertMessage.ts';
import type { SourceSubmissionReceipt } from './sourceSubmission.ts';
export interface InPlaceResendInput {
    operation_id:string; conversation_id:string; message_id:string; expected_revision:number;
    expected_message_revision:number; edited_text:string; created_at_ms:number;
}
export interface InPlaceResendEnvelope {
    host_id:string; client_id:string;
    prepared:{input:InPlaceResendInput;run_id:string;user_message_id:string;experts:ExpertParticipantReference[]};
    target:{id:string;title:string;revision:number;configuration_revision:number};
}
export interface InPlaceResendCompletion { inPlace:true; envelope:InPlaceResendEnvelope; receipt?:SourceSubmissionReceipt }
export function inPlaceInput(draft:EditedResendDraft):InPlaceResendInput {
    const i=draft.input;
    return {operation_id:i.operation_id,conversation_id:i.source_conversation_id,message_id:i.source_message_id,
        expected_revision:i.expected_source_revision,expected_message_revision:i.expected_source_message_revision,
        edited_text:i.edited_text,created_at_ms:i.created_at_ms};
}
export async function prepareInPlaceResend(draft:EditedResendDraft, request:typeof api=api):Promise<InPlaceResendEnvelope> {
    const input=inPlaceInput(draft);
    const result=await request<InPlaceResendEnvelope>('/api/chat/in-place-resend/prepare',{method:'POST',body:JSON.stringify({
        expected_host_id:draft.source.hostId,client_id:draft.source.clientId,input})});
    const p=result?.prepared;
    if(result?.host_id!==draft.source.hostId||result.client_id!==draft.source.clientId||result.target?.id!==input.conversation_id
        ||!p||!p.input||Object.keys(input).some(k=>input[k as keyof InPlaceResendInput]!==p.input[k as keyof InPlaceResendInput])
        ||p.run_id!==input.operation_id||!Array.isArray(p.experts)
        ||p.user_message_id!==(p.experts.length?input.operation_id:`${input.operation_id}:user`))throw new Error('In-place resend owner changed');
    return result;
}

export function confirmInPlaceCompletion(draft:EditedResendDraft, result:InPlaceResendCompletion):void {
    const input=inPlaceInput(draft), p=result.envelope.prepared;
    if(result.envelope.target.id!==input.conversation_id||p.run_id!==input.operation_id
        ||Object.keys(input).some(k=>input[k as keyof InPlaceResendInput]!==p.input[k as keyof InPlaceResendInput])
        ||(!p.experts.length && (result.receipt?.phase!=='admitted'||result.receipt.identity.conversation_id!==input.conversation_id
            ||result.receipt.identity.submission_id!==p.run_id||result.receipt.identity.user_message_id!==p.user_message_id)))
        throw new Error('In-place resend admission is not confirmed');
}
