import { api as requestAPI } from './api.ts';
import { parseSourceReceipt } from './sourceSubmission.ts';
import type { SourceSubmissionReceipt } from './sourceSubmission.ts';
import {timelineMessageText,type TimelineMessage} from './chatTimeline.ts';
import { displayTextAttachmentMessage } from './chatTextAttachments.ts';

export interface EditedResendSourceSnapshot {
    hostId: string; clientId: string; conversationId: string; conversationRevision: number;
    messageId: string; messageRevision: number; originalText: string;
}
export interface EditedResendInput {
    operation_id: string; source_conversation_id: string; source_message_id: string;
    expected_source_revision: number; expected_source_message_revision: number; edited_text: string;
    target_conversation_id: string; title: string; created_at_ms: number;
}
export interface EditedResendDraft { source: Readonly<EditedResendSourceSnapshot>; input: Readonly<EditedResendInput> }
export interface PreparedEditedResend {
    input: EditedResendInput;
    source: {conversation_id: string; message_id: string; run_id: string; assistant_message_id: string;
        body_sha256: string; invocation_config_sha256: string; model: Record<string, unknown>};
    user_message_id: string; assistant_message_id: string; run_id: string; admitted_run_id: string | null;
}
export interface EditedResendEnvelope {
    host_id: string; client_id: string; prepared: PreparedEditedResend;
    target: {id: string; title: string; revision: number; configuration_revision: number};
}
export interface EditedResendCompletion { envelope: EditedResendEnvelope; receipt: SourceSubmissionReceipt }
const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === 'object' && !Array.isArray(value);
const id = (value: unknown): value is string => typeof value === 'string' && !!value.trim() && value.trim() === value && !/[\u0000-\u001f\u007f-\u009f]/.test(value) && value.length <= 512;
const positive = (value: unknown): value is number => Number.isSafeInteger(value) && Number(value) > 0;
const digest = (value: unknown): value is string => typeof value === 'string' && /^[a-f0-9]{64}$/.test(value);
const uuid = (value: unknown): value is string => typeof value === 'string' && /^[a-f0-9]{8}-(?:[a-f0-9]{4}-){3}[a-f0-9]{12}$/i.test(value);
/** Offer editing only for a disclosed canonical plain user boundary. Core still proves send provenance. */
export function editedResendSourceForMessage(message: TimelineMessage, owner: {
    hostId:string;clientId:string;conversationId:string;
}): EditedResendSourceSnapshot | null {
    const canonical=message.canonical,snapshot=message.canonicalSnapshot;
    if(!snapshot||snapshot.sourceHostId!==owner.hostId||snapshot.conversationId!==owner.conversationId||!positive(snapshot.revision))return null;
    if(message.role!=='user'||message.state!=='completed'||!canonical||canonical.role!=='user'||canonical.state!=='completed'||canonical.message_id!==message.id
        ||!positive(canonical.revision)||canonical.parts.length!==1)return null;
    const part=canonical.parts[0];
    let text:string;
    if(part.kind==='text'&&part.is_final===true&&typeof part.markdown==='string'&&(part.review==null||part.review==='approved'))text=part.markdown;
    else if(part.kind==='historical'&&object(part.projection)){
        const p=part.projection,owner=p.owner,preview=p.canonical_text;
        if(p.canonical_kind!=='text'||p.schema!=null||!object(owner)||owner.conversation_id!==snapshot.conversationId
            ||owner.message_id!==canonical.message_id||owner.part_id!==part.part_id||!object(preview)||typeof preview.text!=='string'
            ||preview.total_utf8_bytes!==new TextEncoder().encode(preview.text).length)return null;
        text=preview.text;
    }else return null;
    if(!positive(part.revision)||displayTextAttachmentMessage(text)!==null||message.parts.length!==1
        ||message.parts[0].id!==part.part_id||timelineMessageText(message)!==text)return null;
    return Object.freeze({...owner,conversationRevision:snapshot.revision,messageId:canonical.message_id,messageRevision:canonical.revision,originalText:text});
}
export function createEditedResendDraft(source: EditedResendSourceSnapshot, text = source.originalText,
    identity = {operationId: crypto.randomUUID(), targetId: crypto.randomUUID(), createdAtMs: Date.now()}): EditedResendDraft {
    if (![source.hostId, source.clientId, source.conversationId, source.messageId].every(id)
        || !positive(source.conversationRevision) || !positive(source.messageRevision)
        || !uuid(identity.operationId) || !uuid(identity.targetId) || !Number.isSafeInteger(identity.createdAtMs) || identity.createdAtMs < 0
        || identity.targetId === source.conversationId || typeof text !== 'string') throw new Error('Invalid edit source witness');
    const title = Array.from(text.replace(/[\u0000-\u001f\u007f-\u009f]/g, '').trim()).slice(0,80).join('') || 'Edited message';
    return {source: Object.freeze({...source}), input: Object.freeze({operation_id: identity.operationId,
        source_conversation_id: source.conversationId, source_message_id: source.messageId,
        expected_source_revision: source.conversationRevision, expected_source_message_revision: source.messageRevision,
        edited_text: text, target_conversation_id: identity.targetId, title, created_at_ms: identity.createdAtMs})};
}
/** Input changes deliberately create a new intent; retries of unchanged text retain all IDs/time. */
export function editResendText(draft: EditedResendDraft, text: string): EditedResendDraft {
    return text === draft.input.edited_text ? draft : createEditedResendDraft(draft.source, text);
}
export function sameEditedResendInput(a: EditedResendInput, b: EditedResendInput): boolean {
    return Object.keys(a).length === Object.keys(b).length
        && Object.keys(a).every(key => a[key as keyof EditedResendInput] === b[key as keyof EditedResendInput]);
}
export function parseEditedResendEnvelope(value: unknown, source: Pick<EditedResendSourceSnapshot, 'hostId' | 'clientId'>,
    expected?: EditedResendInput): EditedResendEnvelope {
    if (!object(value) || value.host_id !== source.hostId || value.client_id !== source.clientId
        || !object(value.prepared) || !object(value.target)) throw new Error('Edited resend Host changed');
    const p = value.prepared, target = value.target;
    if (Object.keys(value).length!==4 || Object.keys(p).length!==6 || Object.keys(target).length!==4
        || !object(p.input) || !object(p.source) || Object.keys(p.source).length!==7 || !object(p.source.model)
        || Object.keys(p.source.model).length!==6 || !positive(p.source.model.version) || !object(p.source.model.primary)
        || !object(p.source.model.model_capability)
        || !Array.isArray(p.source.model.fallbacks) || !object(p.source.model.generation) || !object(p.source.model.output_admission)) throw new Error('Edited resend witness missing');
    const input = p.input as unknown as EditedResendInput, sent = p.source;
    if (Object.keys(input).length !== 9 || !uuid(input.operation_id) || !uuid(input.target_conversation_id)
        || !id(input.source_conversation_id) || !id(input.source_message_id)
        || !positive(input.expected_source_revision) || !positive(input.expected_source_message_revision)
        || typeof input.edited_text !== 'string' || !input.edited_text.trim() || typeof input.title !== 'string' || !input.title.trim()
        || !Number.isSafeInteger(input.created_at_ms) || input.created_at_ms<0 || input.target_conversation_id===input.source_conversation_id
        || expected && !sameEditedResendInput(input, expected)
        || sent.conversation_id !== input.source_conversation_id || sent.message_id !== input.source_message_id
        || !id(sent.run_id) || sent.run_id===p.run_id || !id(sent.assistant_message_id) || !digest(sent.body_sha256) || !digest(sent.invocation_config_sha256)
        || p.run_id !== input.operation_id || p.user_message_id !== `${input.operation_id}:user`
        || p.assistant_message_id !== `${input.operation_id}:assistant`
        || !Object.hasOwn(p, 'admitted_run_id') || p.admitted_run_id !== null && p.admitted_run_id !== p.run_id
        || target.id !== input.target_conversation_id || typeof target.title !== 'string'
        || !positive(target.revision) || !positive(target.configuration_revision)) throw new Error('Edited resend identity or revision mismatch');
    return structuredClone(value) as unknown as EditedResendEnvelope;
}
export async function prepareEditedResend(draft: EditedResendDraft, request: typeof requestAPI = requestAPI): Promise<EditedResendEnvelope> {
    if (!draft.input.edited_text.trim()) throw new Error('Edited text is empty');
    return parseEditedResendEnvelope(await request('/api/chat/edited-resend/prepare', {method:'POST',body:JSON.stringify({
        expected_host_id:draft.source.hostId,client_id:draft.source.clientId,input:draft.input})}), draft.source, draft.input);
}
export async function loadPreparedEditedResend(draft: EditedResendDraft, request: typeof requestAPI = requestAPI): Promise<EditedResendEnvelope> {
    const query = new URLSearchParams({expected_host_id:draft.source.hostId, client_id:draft.source.clientId});
    return parseEditedResendEnvelope(await request(`/api/chat/conversations/${encodeURIComponent(draft.input.target_conversation_id)}/edited-resend?${query}`), draft.source, draft.input);
}
export function confirmEditedResendAdmission(draft: EditedResendDraft, envelope: EditedResendEnvelope,
    receipt: SourceSubmissionReceipt): EditedResendCompletion {
    const checked = parseEditedResendEnvelope(envelope, draft.source, draft.input), p = checked.prepared;
    const exact = {source_host_id:draft.source.hostId,client_id:draft.source.clientId,conversation_id:p.input.target_conversation_id,
        submission_id:p.run_id,user_message_id:p.user_message_id,assistant_message_id:p.assistant_message_id};
    const accepted = parseSourceReceipt(receipt, exact, receipt.request_digest);
    if (accepted.phase !== 'admitted' || p.admitted_run_id !== p.run_id) throw new Error('Edited resend has not been admitted');
    return {envelope:checked,receipt:accepted};
}
export async function completeEditedResend(draft: EditedResendDraft, options: {
    controller: (envelope: EditedResendEnvelope) => Promise<{submitPreparedEditedResend(envelope:EditedResendEnvelope, language:string):Promise<SourceSubmissionReceipt>}>;
    language: string; request?: typeof requestAPI; isCurrent?: () => boolean;
}): Promise<EditedResendCompletion> {
    if (options.isCurrent && !options.isCurrent()) throw new Error('Edit owner changed');
    const envelope = await prepareEditedResend(draft, options.request);
    if (options.isCurrent && !options.isCurrent()) throw new Error('Edit owner changed before dispatch');
    const controller = await options.controller(envelope);
    if (options.isCurrent && !options.isCurrent()) throw new Error('Edit owner changed before dispatch');
    const receipt = await controller.submitPreparedEditedResend(envelope, options.language);
    return confirmEditedResendAdmission(draft, await loadPreparedEditedResend(draft, options.request), receipt);
}
