import { journalNumber, parseJournalCursor, journalIdentityKey } from './assistantJournal.ts';
import type { AssistantJournalReader, JournalCursor, JournalIdentity, JournalTransport, JournalUpdate } from './assistantJournal.ts';
import type { ChatTurn } from './chatTurn.ts';
import type { AdmittedMessage } from './messageAdmissions.ts';
import { parseRunResult, sameResultJSON } from './runResult.ts';
import type { RunResult } from './runResult.ts';

export type TerminalMessageState = 'completed' | 'failed' | 'cancelled';

export interface TerminalSeal {
    protocol_version: 1; identity: JournalIdentity; core_session_id: string; transition_id: string;
    checkpoint_revision: string; core_revision: string; event_cursor: string; result: RunResult; result_digest: string;
    completed_at_ms: number; entry_count: string; heads_count: string; child_receipt_count: string;
    manifest_digest: string; seal_digest: string;
    admission: JournalCursor;
}
export interface VerifiedTerminal {
    readonly owner: ChatTurn['owner']; readonly submissionId: string; readonly seal: Readonly<TerminalSeal>;
    readonly heads: readonly JournalCursor[];
    readonly messages: readonly { readonly through: JournalCursor; readonly state: TerminalMessageState; readonly completed_at_ms: number }[];
    readonly declarations: readonly AdmittedMessage[];
}
const verified = new WeakSet<object>();
const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === 'object' && !Array.isArray(value);
const id = (value: unknown): value is string => typeof value === 'string' && value.trim().length > 0;
const hash = (value: unknown): value is string => typeof value === 'string' && /^[0-9a-f]{64}$/.test(value);
function freezeEvidence(value: unknown): void {
    if (!value || typeof value !== 'object' || Object.isFrozen(value)) return;
    Object.freeze(value);
    for (const child of Object.values(value)) freezeEvidence(child);
}
export function isVerifiedTerminal(value: unknown): value is VerifiedTerminal { return object(value) && verified.has(value); }
function sameSeal(value: unknown, seal: TerminalSeal): boolean {
    return object(value) && object(value.identity)
        && journalIdentityKey(value.identity as unknown as JournalIdentity) === journalIdentityKey(seal.identity)
        && Object.keys(value).length === Object.keys(seal).length
        && Object.entries(seal).every(([key, field]) => {
            if (key === 'identity') return true;
            if (key === 'result') return sameResultJSON(value.result, field);
            if (key !== 'admission') return value[key] === field;
            const at = parseJournalCursor(value.admission);
            return journalIdentityKey(at.identity) === journalIdentityKey(seal.admission.identity)
                && at.sequence === seal.admission.sequence && at.record_digest === seal.admission.record_digest;
        });
}

/** This is a local authenticated Store read, not a remote replica installation.
 * Store validates seal/manifest digests. UI additionally fences the exact owner,
 * fixed seal, complete page frontier and every materialized assistant head. */
export async function readTerminal(
    turn: ChatTurn, reader: AssistantJournalReader, transport: JournalTransport,
    apply: (update: JournalUpdate | null) => void, current: () => boolean,
    expectedSealDigest?: string,
    candidateRunId?: string,
): Promise<VerifiedTerminal | null> {
    const owner = turn.owner;
    const runId = turn.runId ?? candidateRunId ?? turn.identity.submissionId;
    if (!id(runId)) throw new Error('Invalid terminal Run candidate');
    if (reader.sourceHostId !== owner.hostId || reader.conversationId !== owner.conversationId || reader.generation !== owner.generation) throw new Error('Terminal reader owner mismatch');
    const request = async (body: Record<string, unknown>) => {
        if (!current()) throw new Error('Stale terminal owner');
        const response = await transport(body);
        if (!current()) throw new Error('Stale terminal owner');
        if (!object(response) || response.source_host_id !== owner.hostId || !Object.hasOwn(response, 'result')) throw new Error('Terminal source mismatch');
        return response.result;
    };
    const raw = await request({ operation: 'terminal_seal', conversation_id: owner.conversationId, run_id: runId });
    if (raw === null) return null;
    if (!object(raw) || raw.protocol_version !== 1 || !id(raw.core_session_id) || !id(raw.transition_id)
        || !hash(raw.result_digest) || !hash(raw.manifest_digest) || !hash(raw.seal_digest)
        || !Number.isSafeInteger(raw.completed_at_ms) || (raw.completed_at_ms as number) < 0) throw new Error('Invalid terminal seal');
    const sealIdentity = parseJournalCursor({ identity: raw.identity, sequence: '0', record_digest: raw.seal_digest }).identity;
    if (sealIdentity.conversation_id !== owner.conversationId || sealIdentity.run_id !== runId || sealIdentity.assistant_message_id !== turn.identity.assistantMessageId) throw new Error('Terminal Run owner mismatch');
    const result = parseRunResult(raw.result, owner.conversationId, runId);
    for (const field of ['checkpoint_revision', 'core_revision', 'event_cursor', 'entry_count', 'heads_count', 'child_receipt_count']) journalNumber(raw[field]);
    if (BigInt(result.decided_at_revision) > BigInt(raw.core_revision as string) ||
        BigInt(result.decision_event_cursor) > BigInt(raw.event_cursor as string)) throw new Error('Result exceeds seal frontier');
    if (raw.checkpoint_revision === '0' || raw.heads_count === '0'
        || BigInt(raw.entry_count as string) !== BigInt(raw.heads_count as string) + BigInt(raw.child_receipt_count as string)) throw new Error('Invalid terminal counts');
    const seal = structuredClone(raw) as unknown as TerminalSeal;
    const admission = parseJournalCursor(raw.admission);
    if (journalIdentityKey(admission.identity) !== journalIdentityKey(seal.identity)) throw new Error('Terminal admission owner mismatch');
    if (expectedSealDigest !== undefined && (!hash(expectedSealDigest) || seal.seal_digest !== expectedSealDigest)) throw new Error('Terminal notification watermark mismatch');
    const declarations = await reader.admit(seal.identity, admission);
    if (!current()) throw new Error('Stale terminal owner');
    let after = '0'; let receipts = 0n;
    const heads: JournalCursor[] = []; const seen = new Set<string>(); const receiptIds = new Set<string>(); const taskIds = new Set<string>();
    const messages: { through: JournalCursor; state: TerminalMessageState; completed_at_ms: number }[] = [];
    do {
        const page = await request({ operation: 'terminal_manifest', seal, after_entry: after, max_records: 64, max_encoded_bytes: 131072 });
        if (!object(page) || !sameSeal(page.seal, seal)
            || page.after_entry !== after || typeof page.complete !== 'boolean' || !Array.isArray(page.entries) || page.entries.length > 64) throw new Error('Invalid terminal manifest page');
        const next = journalNumber(page.next_entry);
        if (BigInt(next) !== BigInt(after) + BigInt(page.entries.length) || BigInt(next) > BigInt(seal.entry_count)
            || page.complete !== (next === seal.entry_count) || (!page.complete && next === after)) throw new Error('Terminal manifest did not advance');
        for (const entry of page.entries) {
            if (!object(entry)) throw new Error('Invalid terminal entry');
            if (entry.kind === 'assistant') {
                if (!['completed', 'failed', 'cancelled'].includes(String(entry.state))
                    || !Number.isSafeInteger(entry.completed_at_ms) || (entry.completed_at_ms as number) < 0
                    || !Number.isSafeInteger(entry.created_at_ms) || (entry.completed_at_ms as number) < (entry.created_at_ms as number)) throw new Error('Invalid terminal message state');
                const through = parseJournalCursor(entry.through);
                if (!declarations.some(message => message.input.id === through.identity.assistant_message_id && message.input.role === 'assistant')) throw new Error('Terminal assistant lacks message admission');
                if (through.identity.source_store_epoch !== seal.identity.source_store_epoch || through.identity.conversation_id !== owner.conversationId
                    || through.identity.run_id !== runId || seen.has(journalIdentityKey(through.identity))) throw new Error('Terminal manifest owner mismatch');
                seen.add(journalIdentityKey(through.identity));
                const update = await reader.read(through, runId);
                if (!current()) throw new Error('Stale terminal owner');
                apply(update); heads.push(through);
                messages.push({ through, state: entry.state as TerminalMessageState, completed_at_ms: entry.completed_at_ms as number });
            } else if (entry.kind === 'child_receipt') {
                if (!id(entry.task_id) || !id(entry.receipt_id) || !id(entry.parent_turn_id)
                    || !id(entry.child_session_id) || !id(entry.child_run_id) || entry.status !== 'sealed'
                    || !hash(entry.receipt_digest) || !hash(entry.result_digest)
                    || receiptIds.has(entry.receipt_id) || taskIds.has(entry.task_id)) throw new Error('Invalid terminal receipt');
                receiptIds.add(entry.receipt_id); taskIds.add(entry.task_id); receipts++;
            } else throw new Error('Unsupported terminal entry');
        }
        after = next;
    } while (after !== seal.entry_count);
    if (BigInt(heads.length) !== BigInt(seal.heads_count) || receipts !== BigInt(seal.child_receipt_count)
        || !heads.some(head => journalIdentityKey(head.identity) === journalIdentityKey(seal.identity))) throw new Error('Incomplete terminal manifest');
    freezeEvidence(seal);
    for (const head of heads) { Object.freeze(head.identity); Object.freeze(head); }
    const proof = Object.freeze({ owner: Object.freeze({ ...owner }), submissionId: turn.identity.submissionId, seal, heads: Object.freeze(heads), messages: Object.freeze(messages.map(message => Object.freeze(message))), declarations });
    verified.add(proof);
    return proof;
}
