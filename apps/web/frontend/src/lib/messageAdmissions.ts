import { journalIdentityKey, journalNumber, parseJournalCursor } from './assistantJournal.ts';
import type { JournalCursor, JournalIdentity, JournalTransport } from './assistantJournal.ts';

export interface AdmittedMessage {
    ordinal: number;
    input: { id: string; run_id: string; role: 'user' | 'assistant'; status: string; created_at_ms: number;
        completed_at_ms: number | null; initial_parts: { id: string; kind: string; content_text: string | null }[] };
}
const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === 'object' && !Array.isArray(value);
const same = (a: JournalCursor, b: JournalCursor) => journalIdentityKey(a.identity) === journalIdentityKey(b.identity) && a.sequence === b.sequence && a.record_digest === b.record_digest;
async function digest(text: string): Promise<string> {
    return Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(text))), byte => byte.toString(16).padStart(2, '0')).join('');
}

/** Browser is a source-local read client. Fixed authenticated declarations only
 * supply message shells; they never supply assistant completion authority. */
export class MessageAdmissionReader {
    private readonly host: string;
    private readonly conversation: string;
    private readonly transport: JournalTransport;
    private readonly current: () => boolean;
    private readonly runs = new Map<string, { at: JournalCursor; messages: AdmittedMessage[] }>();
    constructor(host: string, conversation: string, transport: JournalTransport, current: () => boolean) {
        this.host = host; this.conversation = conversation; this.transport = transport; this.current = current;
    }
    async read(root: JournalIdentity, required?: JournalCursor): Promise<readonly AdmittedMessage[]> {
        const query = async (request: Record<string, unknown>) => {
            if (!this.current()) throw new Error('Stale message admission owner');
            const response = await this.transport(request);
            if (!this.current() || !object(response) || response.source_host_id !== this.host) throw new Error('Message admission source mismatch');
            return response.result;
        };
        if (root.conversation_id !== this.conversation) throw new Error('Message admission conversation mismatch');
        const through = required ?? parseJournalCursor(await query({ operation: 'admission_head', conversation_id: this.conversation, run_id: root.run_id }));
        if (journalIdentityKey(through.identity) !== journalIdentityKey(root)) throw new Error('Message admission root mismatch');
        const key = journalIdentityKey(root); const cached = this.runs.get(key);
        if (cached && BigInt(cached.at.sequence) >= BigInt(through.sequence)) {
            if (cached.at.sequence === through.sequence && !same(cached.at, through)) throw new Error('Message admission fork');
            return cached.messages;
        }
        let after: JournalCursor | null = cached?.at ?? null;
        const messages = [...(cached?.messages ?? [])];
        let pending: { id: string; text: string; size: bigint; total: bigint; digest: string } | null = null;
        do {
            const page = await query({ operation: 'admissions', after, through, max_records: 32, max_encoded_bytes: 262144 });
            if (!object(page) || !Array.isArray(page.records) || page.records.length > 32) throw new Error('Invalid message admission page');
            const start = parseJournalCursor(page.after); const next = parseJournalCursor(page.next);
            if (!same(parseJournalCursor(page.through), through) || after && !same(start, after)
                || !after && (start.sequence !== '0' || journalIdentityKey(start.identity) !== key)) throw new Error('Message admission frontier mismatch');
            let cursor = start;
            for (const value of page.records) {
                if (!object(value) || typeof value.message_id !== 'string' || !value.message_id || typeof value.text !== 'string'
                    || typeof value.declaration_digest !== 'string' || !/^[0-9a-f]{64}$/.test(value.declaration_digest)) throw new Error('Invalid message declaration fragment');
                const at = parseJournalCursor(value.cursor); const offset = BigInt(journalNumber(value.offset)); const total = BigInt(journalNumber(value.total_utf8_bytes));
                const bytes = new TextEncoder().encode(value.text).length;
                if (journalIdentityKey(at.identity) !== key || BigInt(at.sequence) !== BigInt(cursor.sequence) + 1n
                    || value.previous_digest !== cursor.record_digest || bytes < 1 || bytes > 16384 || offset + BigInt(bytes) > total) throw new Error('Invalid message admission chain');
                if (!pending) {
                    if (offset !== 0n) throw new Error('Missing declaration prefix');
                    pending = { id: value.message_id, text: '', size: 0n, total, digest: value.declaration_digest };
                }
                if (pending.id !== value.message_id || pending.size !== offset || pending.total !== total || pending.digest !== value.declaration_digest) throw new Error('Message declaration fragments differ');
                pending.text += value.text; pending.size += BigInt(bytes); cursor = at;
                if (pending.size === pending.total) {
                    if (await digest(pending.text) !== pending.digest || !this.current()) throw new Error('Message declaration digest mismatch');
                    const declaration = JSON.parse(pending.text);
                    if (!object(declaration) || !Number.isSafeInteger(declaration.ordinal) || (declaration.ordinal as number) < 0 || !object(declaration.input)) throw new Error('Invalid message declaration');
                    const input = declaration.input;
                    if (input.id !== pending.id || input.run_id !== root.run_id || !['user', 'assistant'].includes(String(input.role))
                        || !Number.isSafeInteger(input.created_at_ms) || !Array.isArray(input.initial_parts)
                        || input.role === 'assistant' && (input.status !== 'streaming' || input.initial_parts.length !== 0)
                        || input.role === 'user' && input.status !== 'completed') throw new Error('Invalid admitted message');
                    for (const part of input.initial_parts) {
                        if (!object(part) || typeof part.id !== 'string' || !part.id || typeof part.kind !== 'string'
                            || !(part.content_text === null || typeof part.content_text === 'string')) throw new Error('Invalid admitted user part');
                        Object.freeze(part);
                    }
                    Object.freeze(input.initial_parts); Object.freeze(input); Object.freeze(declaration);
                    if (messages.some(message => message.input.id === input.id || message.ordinal === declaration.ordinal)) throw new Error('Duplicate admitted message');
                    messages.push(declaration as unknown as AdmittedMessage); pending = null;
                }
            }
            if (!same(cursor, next) || BigInt(next.sequence) > BigInt(through.sequence) || page.complete !== same(next, through)
                || !page.complete && BigInt(next.sequence) <= BigInt(start.sequence)) throw new Error('Message admission page did not advance');
            after = next;
        } while (!same(after, through));
        if (pending) throw new Error('Incomplete message declaration');
        this.runs.set(key, { at: through, messages });
        return messages;
    }
}
