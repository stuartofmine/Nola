import { uiSurfaceReference } from './generativeUi.ts';
/** Authenticated, fixed-watermark journal reader. Notifications are hints, not
 * content or terminal authority. Only changed heads and appended UTF-8 ranges
 * are fetched after the initial bounded snapshot. Counters stay decimal strings. */
import { MessageAdmissionReader } from './messageAdmissions.ts';
import type { AdmittedMessage } from './messageAdmissions.ts';
export interface JournalIdentity { source_store_epoch: string; conversation_id: string; run_id: string; assistant_message_id: string }
export interface JournalCursor { identity: JournalIdentity; sequence: string; record_digest: string }
interface Snapshot { snapshot_id: string; at: JournalCursor }
interface Content { field: string; content_generation: string; utf8_bytes: string; chunk_count: string; content_root: string }
interface Head {
    identity: JournalIdentity; part_id: string; generation: string; revision: string; metadata_revision: string;
    order_key: { section: string; position: string }; metadata: { kind: string; data: Record<string, unknown> }; bodies: Content[];
}
export interface JournalPartValue { kind: string; part_id: string; revision: string; [field: string]: unknown }
export interface JournalTimelinePart {
    source: 'journal'; id: string; value: JournalPartValue; journalOwner: string;
    order: { section: string; position: string };
}
export interface JournalUpdate {
    sourceHostId: string; through: JournalCursor; initial: boolean;
    changes: { partId: string; part: JournalTimelinePart | null }[];
}
export type JournalTransport = (request: Record<string, unknown>, signal?: AbortSignal) => Promise<unknown>;
const object = (v: unknown): v is Record<string, unknown> => !!v && typeof v === 'object' && !Array.isArray(v);
const id = (v: unknown): v is string => typeof v === 'string' && v.trim().length > 0 && new TextEncoder().encode(v).length <= 1024 && !/[\p{Cc}]/u.test(v);
const digest = (v: unknown): v is string => typeof v === 'string' && /^[0-9a-f]{64}$/.test(v);
export function journalNumber(v: unknown): string {
    if (typeof v !== 'string' || !/^(0|[1-9][0-9]{0,19})$/.test(v) || BigInt(v) > 18446744073709551615n) throw new Error('Invalid journal counter');
    return v;
}
export function journalIdentityKey(v: JournalIdentity): string { return JSON.stringify([v.source_store_epoch, v.conversation_id, v.run_id, v.assistant_message_id]); }
export function parseJournalCursor(v: unknown): JournalCursor {
    if (!object(v) || !object(v.identity) || !['source_store_epoch', 'conversation_id', 'run_id', 'assistant_message_id'].every(k => id(v.identity && (v.identity as Record<string, unknown>)[k])) || !digest(v.record_digest)) throw new Error('Invalid journal cursor');
    journalNumber(v.sequence);
    return structuredClone(v) as unknown as JournalCursor;
}
const sameCursor = (a: JournalCursor, b: JournalCursor) => journalIdentityKey(a.identity) === journalIdentityKey(b.identity) && a.sequence === b.sequence && a.record_digest === b.record_digest;
const sections = ['transcript', 'tasks', 'skills', 'model_attempts'];
export function compareJournalParts(a: JournalTimelinePart, b: JournalTimelinePart): number {
    const section = sections.indexOf(a.order.section) - sections.indexOf(b.order.section);
    return section || (BigInt(a.order.position) < BigInt(b.order.position) ? -1 : BigInt(a.order.position) > BigInt(b.order.position) ? 1 : a.id.localeCompare(b.id));
}
function parseHead(v: unknown, cursor: JournalCursor): Head {
    if (!object(v) || !object(v.identity) || journalIdentityKey(v.identity as unknown as JournalIdentity) !== journalIdentityKey(cursor.identity)
        || !id(v.part_id) || !object(v.order_key) || !sections.includes(String(v.order_key.section)) || !object(v.metadata) || !id(v.metadata.kind) || !object(v.metadata.data) || !Array.isArray(v.bodies) || v.bodies.length > 16) throw new Error('Invalid journal head owner');
    for (const n of [v.generation, v.revision, v.metadata_revision, v.order_key.position]) journalNumber(n);
    const fields = new Set<string>();
    for (const body of v.bodies) {
        if (!object(body) || !id(body.field) || fields.has(body.field) || !digest(body.content_root)) throw new Error('Invalid journal content');
        fields.add(body.field);
        for (const n of [body.content_generation, body.utf8_bytes, body.chunk_count]) journalNumber(n);
    }
    return structuredClone(v) as unknown as Head;
}
interface CachedBody { reference: Content; text: string }
interface CachedPart { head: Head; bodies: Map<string, CachedBody> }
interface ReadState { at: JournalCursor; parts: Map<string, CachedPart> }

export class AssistantJournalReader {
    readonly sourceHostId: string;
    readonly conversationId: string;
    readonly generation: number;
    private transport: JournalTransport;
    private states = new Map<string, ReadState>();
    private epochs = new Map<string, string>();
    private controller = new AbortController();
    private pending: Promise<void> = Promise.resolve();
    private admissions: MessageAdmissionReader;
    private admittedMessages = new Map<string, readonly AdmittedMessage[]>();
    constructor(sourceHostId: string, conversationId: string, generation: number, transport: JournalTransport) {
        if (!id(sourceHostId) || !id(conversationId) || !Number.isSafeInteger(generation) || generation < 0) throw new Error('Invalid journal reader owner');
        this.sourceHostId = sourceHostId; this.conversationId = conversationId; this.generation = generation; this.transport = transport;
        this.admissions = new MessageAdmissionReader(sourceHostId, conversationId, transport, () => !this.controller.signal.aborted);
    }
    async admit(root: JournalIdentity, required?: JournalCursor): Promise<readonly AdmittedMessage[]> {
        const messages = await this.admissions.read(root, required);
        this.admittedMessages.set(journalIdentityKey(root), messages);
        return messages;
    }
    knownAdmission(root: JournalIdentity, messageId: string): readonly AdmittedMessage[] | null {
        const messages = this.admittedMessages.get(journalIdentityKey(root));
        return messages?.some(message => message.input.id === messageId) ? messages : null;
    }
    dispose() { this.controller.abort(); this.states.clear(); this.epochs.clear(); }
    private check() { if (this.controller.signal.aborted) throw new Error('Journal reader disposed'); }
    private async request(body: Record<string, unknown>, cleanup = false): Promise<unknown> {
        if (!cleanup) this.check();
        const result = await this.transport(body, cleanup ? undefined : this.controller.signal);
        if (!cleanup) this.check();
        if (!object(result) || result.source_host_id !== this.sourceHostId || !Object.hasOwn(result, 'result')) throw new Error('Journal source mismatch');
        return result.result;
    }
    async notification(event: Record<string, unknown>, expectedRunId: string): Promise<JournalUpdate | null> {
        if (event.type !== 'assistant_journal_committed_v1') return null;
        if (event.session_id !== this.conversationId || event.source_host_id !== this.sourceHostId) throw new Error('Journal notification owner mismatch');
        return this.read(event.through, expectedRunId);
    }
    /** History heads are supplied by the authenticated canonical message window;
     * stream heads must additionally match that submission's exact Run. */
    read(value: unknown, expectedRunId: string): Promise<JournalUpdate | null> {
        const result = this.pending.then(() => this.readAt(value, expectedRunId));
        this.pending = result.then(() => {}, () => {});
        return result;
    }
    private async readAt(value: unknown, expectedRunId: string): Promise<JournalUpdate | null> {
        this.check();
        const through = parseJournalCursor(value);
        const identity = through.identity;
        if (identity.conversation_id !== this.conversationId || identity.run_id !== expectedRunId) throw new Error('Journal Run mismatch');
        const owner = journalIdentityKey(identity);
        const scope = JSON.stringify([identity.conversation_id, identity.run_id, identity.assistant_message_id]);
        const epoch = this.epochs.get(scope);
        if (epoch && epoch !== identity.source_store_epoch) throw new Error('Journal source epoch changed');
        const previous = this.states.get(owner);
        if (previous && BigInt(through.sequence) <= BigInt(previous.at.sequence)) {
            if (through.sequence === previous.at.sequence && !sameCursor(through, previous.at)) throw new Error('Conflicting journal replay');
            return null;
        }
        const raw = await this.request({ operation: 'open_snapshot', at: through });
        if (!object(raw) || !id(raw.snapshot_id)) throw new Error('Invalid journal snapshot');
        const snapshot: Snapshot = { snapshot_id: raw.snapshot_id, at: through };
        try {
            if (!sameCursor(parseJournalCursor(raw.at), through)) throw new Error('Journal snapshot watermark mismatch');
            const parts = new Map(previous?.parts);
            const changes: JournalUpdate['changes'] = [];
            const seen = new Set<string>();
            let after: string | null = null;
            const visitedCursors = new Set<string>();
            do {
                const page = await this.request({ operation: previous ? 'changes' : 'parts', snapshot,
                    ...(previous ? { base: previous.at } : {}), after_part_id: after, max_records: 128, max_encoded_bytes: 262144 });
                if (!object(page) || !sameCursor(parseJournalCursor(page.at), through)
                    || previous && !sameCursor(parseJournalCursor(page.base), previous.at)
                    || !Object.hasOwn(page, 'next_part_id') || !(page.next_part_id === null || id(page.next_part_id))) throw new Error('Invalid journal page');
                const values = previous ? page.changes : page.parts;
                if (!Array.isArray(values) || values.length > 128) throw new Error('Invalid journal page entries');
                for (const entry of values) {
                    if (previous && (!object(entry) || !id(entry.part_id) || !Object.hasOwn(entry, 'head'))) throw new Error('Invalid journal retirement');
                    const head = previous && entry.head === null ? null : parseHead(previous ? entry.head : entry, through);
                    const partId: string = previous ? entry.part_id : head!.part_id;
                    if (head && partId !== head.part_id || seen.has(partId)) throw new Error('Conflicting journal Part');
                    seen.add(partId);
                    if (!head) { parts.delete(partId); changes.push({ partId, part: null }); continue; }
                    const old = parts.get(partId);
                    if (old && (BigInt(head.generation) < BigInt(old.head.generation) || BigInt(head.revision) < BigInt(old.head.revision))) throw new Error('Journal Part regressed');
                    const bodies = new Map<string, CachedBody>();
                    for (const content of head.bodies) {
                        if (head.metadata.kind === 'text' && head.metadata.data.visibility !== 'public') continue;
                        const cached = old && head.generation === old.head.generation ? old.bodies.get(content.field) : undefined;
                        bodies.set(content.field, await this.body(snapshot, head.part_id, content, cached));
                    }
                    parts.set(partId, { head, bodies });
                    changes.push({ partId, part: present(head, bodies) });
                }
                const next = page.next_part_id as string | null;
                if (next !== null) {
                    if (visitedCursors.has(next) || previous && after !== null && next <= after) throw new Error('Journal page did not advance');
                    visitedCursors.add(next);
                }
                after = next;
            } while (after !== null);
            this.check();
            this.states.set(owner, { at: through, parts });
            this.epochs.set(scope, identity.source_store_epoch);
            return { sourceHostId: this.sourceHostId, through, initial: !previous, changes };
        } finally {
            // Snapshot release is exact and idempotent; cancellation must not
            // prevent resource cleanup or reuse its handle for another owner.
            try { await this.request({ operation: 'release_snapshot', snapshot }, true); }
            catch (error) { this.states.delete(owner); throw error; }
        }
    }
    private async body(snapshot: Snapshot, partId: string, reference: Content, old?: CachedBody): Promise<CachedBody> {
        let offset = '0'; let text = '';
        if (old && old.reference.content_generation === reference.content_generation) {
            if (BigInt(reference.utf8_bytes) < BigInt(old.reference.utf8_bytes) || BigInt(reference.chunk_count) < BigInt(old.reference.chunk_count)) throw new Error('Journal body regressed');
            if (reference.utf8_bytes === old.reference.utf8_bytes && reference.content_root !== old.reference.content_root) throw new Error('Journal body changed without a generation');
            offset = old.reference.utf8_bytes; text = old.text;
        }
        while (BigInt(offset) < BigInt(reference.utf8_bytes)) {
            const page = await this.request({ operation: 'range', snapshot, part_id: partId, content: reference, offset_bytes: offset, limit_bytes: 32768 });
            if (!object(page) || page.offset_bytes !== offset || page.total_utf8_bytes !== reference.utf8_bytes || typeof page.text !== 'string' || typeof page.end_of_content !== 'boolean') throw new Error('Invalid journal text range');
            const next = journalNumber(page.next_offset_bytes);
            const bytes = new TextEncoder().encode(page.text).length;
            if (!bytes || bytes > 32768 || BigInt(next) !== BigInt(offset) + BigInt(bytes) || BigInt(next) > BigInt(reference.utf8_bytes)
                || page.end_of_content !== (next === reference.utf8_bytes)) throw new Error('Journal range did not advance');
            text += page.text; offset = next;
        }
        return { reference, text };
    }
}

/** Preserve unsafe integer/fraction tokens as decimal strings in read-only
 * presentation, never round them through JavaScript Number. The cached body
 * bytes remain untouched and are the source for subsequent range offsets. */
export function parseJournalStructured(text: string, field: 'value' | 'resources' | 'artifacts' | 'evidence'): unknown {
    const safe = text.replace(/"(?:\\.|[^"\\])*"|-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?/g, token => {
        if (token.startsWith('"')) return token;
        return !/[.eE]/.test(token) && Number.isSafeInteger(Number(token)) ? token : JSON.stringify(token);
    });
    const envelope: unknown = JSON.parse(safe);
    if (!object(envelope) || envelope.protocol_version !== 1 || Object.keys(envelope).length !== 2 || !Object.hasOwn(envelope, field)) throw new Error('Invalid structured journal envelope');
    if ((field === 'resources' || field === 'artifacts') && !Array.isArray(envelope[field])) throw new Error('Invalid journal collection');
    return envelope[field];
}

function present(head: Head, bodies: Map<string, CachedBody>): JournalTimelinePart {
    const body = (field: string) => bodies.get(field)?.text ?? '';
    const text = (v: unknown): string => object(v) && v.kind === 'inline' && typeof v.value === 'string' ? v.value
        : object(v) && v.kind === 'field' && typeof v.value === 'string' ? body(v.value) : '';
    const structured = (field: string, payload: 'value' | 'resources' | 'artifacts' | 'evidence' = 'value'): unknown => {
        const value = body(field); return value ? parseJournalStructured(value, payload) : null;
    };
    const { kind, data } = head.metadata;
    const value: JournalPartValue = { ...data, kind, part_id: head.part_id, revision: head.revision };
    for (const field of ['title', 'display_name', 'uri', 'mime_type', 'name', 'operation', 'summary']) if (data[field] != null) value[field] = text(data[field]);
    if (kind === 'ui_surface') {
        value.summary = typeof data.summary === 'string' ? data.summary : '';
        uiSurfaceReference(value, head.identity.conversation_id, head.identity.assistant_message_id);
    }
    if (kind === 'text') value.markdown = data.visibility === 'public' && (data.review == null || data.review === 'approved') ? body('markdown') : '';
    if (kind === 'reasoning') value.summary = body('reasoning_summary');
    if (kind === 'tool') {
        value.input = structured('tool_input'); value.output = structured('tool_output');
        value.output_preview = body('tool_output_text'); value.artifacts = structured('tool_artifacts', 'artifacts');
        if (object(data.affected_resources)) value.affected_resources = data.affected_resources.kind === 'inline' ? data.affected_resources.value : structured('tool_resources', 'resources');
        if (object(data.permission)) value.permission = { ...data.permission, permission_domain: text(data.permission.permission_domain), rationale: text(data.permission.rationale), decision: data.permission.decision == null ? null : text(data.permission.decision) };
        if (object(data.origin)) value.origin = { ...data.origin,
            provider_id: data.origin.provider_id == null ? null : text(data.origin.provider_id),
            provider_title: data.origin.provider_title == null ? null : text(data.origin.provider_title),
            namespace: data.origin.namespace == null ? null : text(data.origin.namespace) };
    }
    if (kind === 'task_control' && object(data.task)) Object.assign(value, data.task, { kind: 'task_control' });
    if (object(value.progress) && 'fraction_bits' in value.progress) {
        const bytes = new DataView(new ArrayBuffer(8));
        bytes.setBigUint64(0, BigInt(journalNumber(value.progress.fraction_bits)));
        const progress = bytes.getFloat64(0);
        if (!Number.isFinite(progress) || progress < 0 || progress > 1) throw new Error('Invalid journal task progress');
        value.progress = progress;
    }
    if (kind === 'task' || kind === 'task_control' || kind === 'skill') {
        value.current_step = body('task_current_step'); value.result_ref = body('result_reference');
    }
    if (kind === 'source') { value.snippet = body('source_snippet'); value.evidence = structured('source_evidence', 'evidence'); }
    if (kind === 'citation' && object(data.location)) value.location = { ...data.location, section: data.location.section == null ? null : text(data.location.section) };
    if (kind === 'error' || kind === 'model_attempt' || kind === 'skill') value.failure = body('failure_message');
    if (kind === 'historical') value.preview = body('historical_preview');
    return { source: 'journal', id: head.part_id, value, journalOwner: journalIdentityKey(head.identity), order: { ...head.order_key } };
}
