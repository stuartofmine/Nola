import { parseJournalCursor } from './assistantJournal.ts';
import type { JournalCursor } from './assistantJournal.ts';
import type { ChatTurnOwner, ChatTurnIdentity } from './chatTurn.ts';

export interface SourceSubmissionIdentity {
    source_host_id: string; client_id: string; conversation_id: string; submission_id: string;
    user_message_id: string; assistant_message_id: string;
}
export interface SourceSubmissionReceipt {
    protocol_version: 1; identity: SourceSubmissionIdentity; request_digest: string;
    phase: 'prepared' | 'admitted' | 'rejected' | 'cancelled'; admission: JournalCursor | null; rejection_code: string | null;
}
export interface PendingSourceSubmission {
    identity: SourceSubmissionIdentity; request: Record<string, unknown>; request_digest: string | null;
    sent: boolean; receipt: SourceSubmissionReceipt | null;
}
const object = (v: unknown): v is Record<string, unknown> => !!v && typeof v === 'object' && !Array.isArray(v);
const id = (v: unknown): v is string => typeof v === 'string' && v.trim() === v && v.length > 0 && v.length <= 512;
const digest = (v: unknown): v is string => typeof v === 'string' && /^[0-9a-f]{64}$/.test(v);
const fields = ['source_host_id', 'client_id', 'conversation_id', 'submission_id', 'user_message_id', 'assistant_message_id'] as const;
export function sourceIdentity(owner: ChatTurnOwner, turn: ChatTurnIdentity): SourceSubmissionIdentity {
    return { source_host_id: owner.hostId, client_id: owner.clientId, conversation_id: owner.conversationId,
        submission_id: turn.submissionId, user_message_id: turn.userMessageId, assistant_message_id: turn.assistantMessageId };
}
function sameIdentity(a: unknown, b: SourceSubmissionIdentity): boolean {
    return object(a) && Object.keys(a).length === fields.length && fields.every(field => id(a[field]) && a[field] === b[field]);
}
function validateRequest(identity: SourceSubmissionIdentity, request: Record<string, unknown>) {
    if (!fields.every(field => id(identity[field])) || identity.user_message_id === identity.assistant_message_id
        || Object.hasOwn(request, 'request_digest') || Object.hasOwn(request, 'session_id')
        || request.conversationId !== identity.conversation_id || request.expected_host_id !== identity.source_host_id
        || request.client_id !== identity.client_id || request.submission_id !== identity.submission_id
        || request.user_message_id !== identity.user_message_id || request.assistant_message_id !== identity.assistant_message_id)
        throw new Error('Invalid frozen source request');
}
export function createSourceSubmission(identity: SourceSubmissionIdentity, request: Record<string, unknown>): PendingSourceSubmission {
    const frozen = JSON.parse(JSON.stringify(request)) as Record<string, unknown>;
    validateRequest(identity, frozen);
    return { identity: { ...identity }, request: frozen, request_digest: null, sent: false, receipt: null };
}
export function bindSourceDigest(pending: PendingSourceSubmission, value: unknown): PendingSourceSubmission {
    if (!object(value) || value.source_host_id !== pending.identity.source_host_id || !object(value.result)
        || !sameIdentity(value.result.identity, pending.identity) || !digest(value.result.request_digest)) throw new Error('Invalid source digest preparation');
    if (pending.request_digest !== null && pending.request_digest !== value.result.request_digest) throw new Error('Frozen source request changed');
    return { ...pending, request_digest: value.result.request_digest };
}
export function parseSourceReceipt(value: unknown, identity: SourceSubmissionIdentity, requestDigest: string): SourceSubmissionReceipt {
    if (!object(value) || value.protocol_version !== 1 || !sameIdentity(value.identity, identity) || !digest(requestDigest)
        || value.request_digest !== requestDigest || !['prepared', 'admitted', 'rejected', 'cancelled'].includes(String(value.phase))
        || !Object.hasOwn(value, 'admission')) throw new Error('Invalid source submission receipt');
    const admission = value.admission === null ? null : parseJournalCursor(value.admission);
    if (value.phase === 'admitted' ? !admission || admission.identity.conversation_id !== identity.conversation_id
        || admission.identity.run_id !== identity.submission_id || admission.identity.assistant_message_id !== identity.assistant_message_id
        || BigInt(admission.sequence) === 0n : admission !== null)
        throw new Error('Submission admission owner changed');
    const rejection = value.rejection_code ?? null;
    if (!(rejection === null || typeof rejection === 'string' && /^[a-z0-9_]{1,128}$/.test(rejection))) throw new Error('Invalid rejection code');
    return { protocol_version: 1, identity: { ...identity }, request_digest: requestDigest,
        phase: value.phase as SourceSubmissionReceipt['phase'], admission, rejection_code: rejection as string | null };
}
export function acceptSourceReceipt(pending: PendingSourceSubmission, value: unknown): PendingSourceSubmission {
    if (pending.request_digest === null) throw new Error('Source request is not bound');
    const receipt = parseSourceReceipt(value, pending.identity, pending.request_digest);
    if (pending.receipt && (pending.receipt.phase === 'admitted' && receipt.phase !== 'admitted'
        || ['rejected', 'cancelled'].includes(pending.receipt.phase) && JSON.stringify(receipt) !== JSON.stringify(pending.receipt)))
        throw new Error('Source receipt cannot replace settled authority');
    return { ...pending, receipt };
}
export function sourceSubmissionQuery(pending: PendingSourceSubmission, cancel = false) {
    if (pending.request_digest === null) throw new Error('No bound request to reconcile');
    return { operation: cancel ? 'cancel' : 'observe', identity: pending.identity, request_digest: pending.request_digest };
}
export function sourceSubmissionBody(pending: PendingSourceSubmission): Record<string, unknown> {
    if (!digest(pending.request_digest) || pending.sent) throw new Error('Only one original source dispatch is allowed');
    return { ...pending.request, request_digest: pending.request_digest };
}
export function exactSubmissionCancel(pending: PendingSourceSubmission) {
    if (!digest(pending.request_digest) || pending.receipt?.phase !== 'admitted') throw new Error('No admitted cancellation authority');
    return { identity: pending.identity, request_digest: pending.request_digest };
}
export function settledSourceSubmission(pending: PendingSourceSubmission) { return pending.receipt?.phase === 'rejected' || pending.receipt?.phase === 'cancelled'; }

export function parseObservedSourceReceipt(value: unknown, owner: ChatTurnOwner, runId: string, assistantMessageId: string): SourceSubmissionReceipt {
    if (!object(value) || !object(value.identity) || !digest(value.request_digest)) throw new Error('Observed source receipt missing');
    const identity = value.identity as unknown as SourceSubmissionIdentity;
    if (identity.source_host_id !== owner.hostId || identity.client_id !== owner.clientId || identity.conversation_id !== owner.conversationId
        || identity.submission_id !== runId || identity.assistant_message_id !== assistantMessageId) throw new Error('Observed source owner changed');
    const receipt = parseSourceReceipt(value, identity, value.request_digest);
    if (receipt.phase !== 'admitted') throw new Error('Observed Run was not admitted');
    return receipt;
}
export function admittedReceiptCancel(receipt: SourceSubmissionReceipt) {
    if (receipt.phase !== 'admitted') throw new Error('No admitted cancellation authority');
    return { identity: receipt.identity, request_digest: receipt.request_digest };
}

/** Browser source mode stores no replica intent and never writes local Runs.
 * Persistence must succeed before preparation/dispatch; no ephemeral fallback. */
export class SourceSubmissionStorage {
    private storage: Pick<Storage, 'getItem' | 'setItem' | 'removeItem' | 'key' | 'length'>;
    private namespace: string;
    constructor(storage: Pick<Storage, 'getItem' | 'setItem' | 'removeItem' | 'key' | 'length'>, namespace = 'chat') {
        this.storage = storage;
        this.namespace = namespace;
    }
    private key(host: string, client: string, conversation: string) { return `nola.source-submission.${this.namespace}.${JSON.stringify([host, client, conversation])}`; }
    save(value: PendingSourceSubmission) { this.storage.setItem(this.key(value.identity.source_host_id, value.identity.client_id, value.identity.conversation_id), JSON.stringify(value)); }
    read(host: string, client: string, conversation: string): PendingSourceSubmission | null {
        const raw = this.storage.getItem(this.key(host, client, conversation)); if (!raw) return null;
        const value: unknown = JSON.parse(raw);
        if (!object(value) || !object(value.identity) || !object(value.request) || typeof value.sent !== 'boolean') throw new Error('Invalid persisted submission');
        const identity = value.identity as unknown as SourceSubmissionIdentity;
        if (identity.source_host_id !== host || identity.client_id !== client || identity.conversation_id !== conversation) throw new Error('Persisted source owner changed');
        const pending = createSourceSubmission(identity, value.request);
        if (!(value.request_digest === null || digest(value.request_digest))) throw new Error('Invalid persisted digest');
        let restored: PendingSourceSubmission = { ...pending, request_digest: value.request_digest as string | null, sent: value.sent };
        if (value.receipt !== null) restored = acceptSourceReceipt(restored, value.receipt);
        return restored;
    }
    recoverable(host: string, client: string): PendingSourceSubmission[] {
        const prefix = `nola.source-submission.${this.namespace}.`;
        const result: PendingSourceSubmission[] = [];
        for (let index = 0; index < this.storage.length; index++) {
            const key = this.storage.key(index);
            if (!key?.startsWith(prefix)) continue;
            try {
                const scope: unknown = JSON.parse(key.slice(prefix.length));
                if (!Array.isArray(scope) || scope.length !== 3 || scope[0] !== host || scope[1] !== client || !id(scope[2])) continue;
                const pending = this.read(host, client, scope[2]);
                if (pending?.sent && pending.request_digest !== null && !settledSourceSubmission(pending)) result.push(pending);
            } catch { /* A corrupt local record is not admission authority; preserve it for diagnosis. */ }
        }
        return result;
    }
    clear(value: PendingSourceSubmission) {
        const key = this.key(value.identity.source_host_id, value.identity.client_id, value.identity.conversation_id);
        const raw = this.storage.getItem(key);
        if (raw === null) return;
        const current: unknown = JSON.parse(raw);
        // A delayed cleanup cannot erase a newer submission in this scope.
        if (object(current) && sameIdentity(current.identity, value.identity)) this.storage.removeItem(key);
    }
}
