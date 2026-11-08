/** Committed source facts only. This read model never grants execution,
 * permission, resume, or submission authority. Counters stay lossless u64. */
import { journalIdentityKey, journalNumber, parseJournalCursor } from './assistantJournal.ts';
import type { JournalIdentity, JournalCursor } from './assistantJournal.ts';
import type { ChatTurnOwner } from './chatTurn.ts';
import { parseRunResult } from './runResult.ts';

export interface PromptCacheUsage {
    input_tokens: number | null; read_tokens: number | null; write_tokens: number | null;
    hit_rate_bps: number | null; is_compaction: boolean;
}
export interface CommittedRunContextUsage {
    protocol_version: 3; identity: JournalIdentity; admission: JournalCursor; event_sequence: string;
    attempt_sequence: string;
    run_cache_usage: (PromptCacheUsage & { reported_requests: number; unreported_requests: number }) | null;
    usage: { measurement: ContextMeasurement | null; prompt_cache_measurement: ContextMeasurement | null;
        prompt_cache_diagnostic: PromptCacheDiagnostic | null; cache_usage?: PromptCacheUsage;
        used_tokens: string; context_window_tokens: string; prompt_token_limit: string; estimated: boolean };
    record_digest: string;
}
export interface ContextMeasurement { request_id: string; attempt_id: string | null; context_generation: number | null }
export interface PromptCacheDiagnostic {
    cacheUsage?: PromptCacheUsage; schemaVersion: number; status: string; identityStable: boolean;
    identitySha256?: string; reasons: string[]; fragmentChanges?: { fragmentId: string; kind: string }[];
    cacheReadInputTokens: number | null; cacheWriteInputTokens: number | null;
}
const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === 'object' && !Array.isArray(value);
const fields = (value: Record<string, unknown>, names: readonly string[]) => Object.keys(value).length === names.length && names.every(name => Object.hasOwn(value, name));
const identityWire = (value: JournalIdentity) => ({ source_store_epoch: value.source_store_epoch, conversation_id: value.conversation_id,
    run_id: value.run_id, assistant_message_id: value.assistant_message_id });
const cursorWire = (value: JournalCursor) => ({ identity: identityWire(value.identity), sequence: value.sequence, record_digest: value.record_digest });

function measurementWire(raw: unknown): ContextMeasurement | null {
    if (raw === null) return null;
    if (!object(raw) || !fields(raw, ['request_id', 'attempt_id', 'context_generation']) ||
        typeof raw.request_id !== 'string' || !raw.request_id.trim() ||
        raw.attempt_id !== null && (typeof raw.attempt_id !== 'string' || !raw.attempt_id.trim()) ||
        raw.context_generation !== null && (!Number.isSafeInteger(raw.context_generation) || Number(raw.context_generation) < 0))
        throw new Error('Invalid context measurement identity');
    return { request_id: raw.request_id, attempt_id: raw.attempt_id as string | null, context_generation: raw.context_generation as number | null };
}

function diagnosticWire(raw: unknown): PromptCacheDiagnostic | null {
    if (raw === null) return null;
    if (!object(raw) || !fields(raw, [...(Object.hasOwn(raw, 'cacheUsage') ? ['cacheUsage'] : []),
        'schemaVersion', 'status', 'identityStable', ...(Object.hasOwn(raw, 'identitySha256') ? ['identitySha256'] : []),
        'reasons', ...(Object.hasOwn(raw, 'fragmentChanges') ? ['fragmentChanges'] : []), 'cacheReadInputTokens', 'cacheWriteInputTokens']) ||
        raw.schemaVersion !== 1 || !['disabled', 'invalid_identity', 'first_request', 'hit', 'write', 'miss', 'stable_candidate', 'unknown'].includes(String(raw.status)) ||
        typeof raw.identityStable !== 'boolean' || raw.identitySha256 !== undefined && typeof raw.identitySha256 !== 'string' ||
        !Array.isArray(raw.reasons) || !raw.reasons.every(reason => typeof reason === 'string')) throw new Error('Invalid cache diagnostic');
    for (const key of ['cacheReadInputTokens', 'cacheWriteInputTokens'])
        if (raw[key] !== null && (!Number.isSafeInteger(raw[key]) || Number(raw[key]) < 0)) throw new Error('Invalid diagnostic counter');
    if (raw.fragmentChanges !== undefined && (!Array.isArray(raw.fragmentChanges) || !raw.fragmentChanges.every(change =>
        object(change) && fields(change, ['fragmentId', 'kind']) && typeof change.fragmentId === 'string' &&
        ['added', 'removed', 'changed'].includes(String(change.kind))))) throw new Error('Invalid diagnostic fragments');
    return { ...(Object.hasOwn(raw, 'cacheUsage') ? { cacheUsage: cacheUsageWire(raw.cacheUsage) } : {}),
        schemaVersion: 1, status: raw.status as string, identityStable: raw.identityStable,
        ...(raw.identitySha256 !== undefined ? { identitySha256: raw.identitySha256 as string } : {}), reasons: raw.reasons as string[],
        ...(raw.fragmentChanges !== undefined ? { fragmentChanges: (raw.fragmentChanges as { fragmentId: string; kind: string }[])
            .map(change => ({ fragmentId: change.fragmentId, kind: change.kind })) } : {}),
        cacheReadInputTokens: raw.cacheReadInputTokens as number | null, cacheWriteInputTokens: raw.cacheWriteInputTokens as number | null };
}

function usageWire(raw: CommittedRunContextUsage['usage']): CommittedRunContextUsage['usage'] {
    const measurement = measurementWire(raw.measurement), promptCacheMeasurement = measurementWire(raw.prompt_cache_measurement);
    const diagnostic = diagnosticWire(raw.prompt_cache_diagnostic);
    if (diagnostic !== null && promptCacheMeasurement === null) throw new Error('Cache diagnostic has no source request');
    return { measurement, prompt_cache_measurement: promptCacheMeasurement, prompt_cache_diagnostic: diagnostic,
        ...(Object.hasOwn(raw, 'cache_usage') ? { cache_usage: cacheUsageWire(raw.cache_usage) } : {}),
        used_tokens: raw.used_tokens, context_window_tokens: raw.context_window_tokens,
        prompt_token_limit: raw.prompt_token_limit, estimated: raw.estimated };
}

function cacheUsageWire(raw: unknown): PromptCacheUsage {
    if (!object(raw) || !fields(raw, ['input_tokens', 'read_tokens', 'write_tokens', 'hit_rate_bps', 'is_compaction']) || typeof raw.is_compaction !== 'boolean')
        throw new Error('Invalid cache usage');
    for (const key of ['input_tokens', 'read_tokens', 'write_tokens', 'hit_rate_bps'])
        if (raw[key] !== null && (!Number.isSafeInteger(raw[key]) || Number(raw[key]) < 0)) throw new Error('Invalid cache counter');
    const input = raw.input_tokens as number | null, read = raw.read_tokens as number | null;
    const expected = input !== null && read !== null && input > 0 && read <= input ? Number(BigInt(read) * 10000n / BigInt(input)) : null;
    if (raw.hit_rate_bps !== expected) throw new Error('Invalid cache rate');
    return { input_tokens: input, read_tokens: read, write_tokens: raw.write_tokens as number | null, hit_rate_bps: expected, is_compaction: raw.is_compaction };
}

export function parseCumulativeCacheUsage(raw: unknown): CommittedRunContextUsage['run_cache_usage'] {
    if (raw === null) return null;
    if (!object(raw) || !fields(raw, ['input_tokens', 'read_tokens', 'write_tokens', 'hit_rate_bps', 'is_compaction', 'reported_requests', 'unreported_requests']))
        throw new Error('Invalid cumulative cache usage');
    const { reported_requests, unreported_requests, ...totals } = raw;
    if (!Number.isSafeInteger(reported_requests) || Number(reported_requests) < 0 ||
        !Number.isSafeInteger(unreported_requests) || Number(unreported_requests) < 0 ||
        Number(reported_requests) + Number(unreported_requests) === 0 || totals.is_compaction !== false)
        throw new Error('Invalid cache coverage');
    const parsed = cacheUsageWire(totals);
    if (parsed.input_tokens !== null && parsed.read_tokens !== null && parsed.read_tokens > parsed.input_tokens)
        throw new Error('Invalid cumulative cache totals');
    return { ...parsed, reported_requests: reported_requests as number, unreported_requests: unreported_requests as number };
}

export function parseRunContextUsage(value: unknown, admission: JournalCursor): CommittedRunContextUsage {
    if (!object(value) || !fields(value, ['protocol_version', 'identity', 'admission', 'event_sequence', 'attempt_sequence', 'run_cache_usage', 'usage', 'record_digest']) ||
        value.protocol_version !== 3 || !object(value.identity) || !fields(value.identity, ['source_store_epoch', 'conversation_id', 'run_id', 'assistant_message_id']) ||
        !object(value.admission) || !fields(value.admission, ['identity', 'sequence', 'record_digest']) ||
        !object(value.admission.identity) || !fields(value.admission.identity, ['source_store_epoch', 'conversation_id', 'run_id', 'assistant_message_id']) || !object(value.usage) ||
        !fields(value.usage, ['measurement', 'prompt_cache_measurement', 'prompt_cache_diagnostic', ...(Object.hasOwn(value.usage, 'cache_usage') ? ['cache_usage'] : []), 'used_tokens', 'context_window_tokens', 'prompt_token_limit', 'estimated']) ||
        typeof value.usage.estimated !== 'boolean' || typeof value.record_digest !== 'string' || !/^[0-9a-f]{64}$/.test(value.record_digest))
        throw new Error('Invalid committed context usage');
    const bound = parseJournalCursor(value.admission);
    const recordIdentity = parseJournalCursor({ identity: value.identity, sequence: bound.sequence, record_digest: bound.record_digest }).identity;
    const expected = parseJournalCursor(admission);
    if (journalIdentityKey(recordIdentity) !== journalIdentityKey(expected.identity) ||
        journalIdentityKey(bound.identity) !== journalIdentityKey(expected.identity) || BigInt(bound.sequence) === 0n ||
        BigInt(bound.sequence) > BigInt(expected.sequence) || bound.sequence === expected.sequence && bound.record_digest !== expected.record_digest)
        throw new Error('Context usage admission owner/frontier changed');
    const event = journalNumber(value.event_sequence);
    const attempt = journalNumber(value.attempt_sequence);
    const cache = parseCumulativeCacheUsage(value.run_cache_usage);
    if (cache !== null && attempt === '0') throw new Error('Cumulative cache usage has no attempt frontier');
    if (event === '0') throw new Error('Context usage has no committed event');
    const used = journalNumber(value.usage.used_tokens), window = journalNumber(value.usage.context_window_tokens), prompt = journalNumber(value.usage.prompt_token_limit);
    if (BigInt(prompt) > BigInt(window)) throw new Error('Context prompt limit exceeds its window');
    return { protocol_version: 3, identity: identityWire(recordIdentity), admission: cursorWire(bound), event_sequence: event,
        attempt_sequence: attempt, run_cache_usage: cache,
        usage: usageWire({ ...value.usage, used_tokens: used, context_window_tokens: window, prompt_token_limit: prompt,
            estimated: value.usage.estimated } as CommittedRunContextUsage['usage']),
        record_digest: value.record_digest };
}

/** Same fixed-order typed tuple and domain-separated byte length as Rust. */
export async function contextUsageDigest(value: Omit<CommittedRunContextUsage, 'record_digest'>): Promise<string> {
    const bytes = new TextEncoder().encode(JSON.stringify([value.protocol_version, identityWire(value.identity), cursorWire(value.admission),
        value.event_sequence, value.attempt_sequence, parseCumulativeCacheUsage(value.run_cache_usage), usageWire(value.usage)]));
    const tag = new TextEncoder().encode('NOLA-RUN-CONTEXT-USAGE-V3');
    const input = new Uint8Array(tag.length + 8 + bytes.length); input.set(tag);
    new DataView(input.buffer).setBigUint64(tag.length, BigInt(bytes.length), false); input.set(bytes, tag.length + 8);
    return Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', input)), byte => byte.toString(16).padStart(2, '0')).join('');
}

const scope = (owner: ChatTurnOwner, identity: JournalIdentity) => JSON.stringify([owner.hostId, owner.clientId, owner.conversationId,
    owner.generation, journalIdentityKey(identity)]);

export class RunContextUsageProjection {
    private owner: ChatTurnOwner | null = null;
    private admission: JournalCursor | null = null;
    private value: CommittedRunContextUsage | null = null;
    get record() { return this.value; }
    reset() { this.owner = null; this.admission = null; this.value = null; }
    bind(owner: ChatTurnOwner, admission: JournalCursor, historical = false): boolean {
        const exact = parseJournalCursor(admission);
        if (exact.identity.conversation_id !== owner.conversationId || exact.sequence === '0') throw new Error('Context usage lacks exact Run admission');
        if (this.owner && this.admission) {
            if (this.owner.generation === owner.generation && this.owner.conversationId === owner.conversationId &&
                (this.owner.hostId !== owner.hostId || this.owner.clientId !== owner.clientId)) throw new Error('Context source/client changed inside a load generation');
            if (scope(this.owner, this.admission.identity) === scope(owner, exact.identity)) {
                if (BigInt(exact.sequence) < BigInt(this.admission.sequence)) return false;
                if (exact.sequence === this.admission.sequence && exact.record_digest !== this.admission.record_digest) throw new Error('Context admission fork');
                this.admission = exact; return true;
            }
            if (this.owner.generation === owner.generation && this.owner.hostId === owner.hostId &&
                this.owner.conversationId === owner.conversationId && this.admission.identity.run_id === exact.identity.run_id)
                throw new Error('Context usage cannot replace the same Run root/store');
            // Only live exact-Run observation can supersede a visible owner.
            // Prepending/reloading older history is not a latest-Run grant.
            if (historical) return false;
        }
        this.owner = { ...owner }; this.admission = exact; this.value = null; return true;
    }
    async accept(raw: unknown | null, owner: ChatTurnOwner, admission: JournalCursor, current: () => boolean): Promise<CommittedRunContextUsage | null> {
        if (!current() || !this.owner || !this.admission || scope(this.owner, this.admission.identity) !== scope(owner, admission.identity)) return null;
        if (raw === null) return this.value;
        const captured = scope(this.owner, this.admission.identity);
        const record = parseRunContextUsage(raw, admission);
        if (await contextUsageDigest(record) !== record.record_digest) throw new Error('Context usage digest mismatch');
        if (!current() || !this.owner || !this.admission || scope(this.owner, this.admission.identity) !== captured) return null;
        if (this.value) {
            const event = BigInt(record.event_sequence) - BigInt(this.value.event_sequence);
            const attempt = BigInt(record.attempt_sequence) - BigInt(this.value.attempt_sequence);
            if ((event < 0n && attempt > 0n) || (event > 0n && attempt < 0n)) throw new Error('Context usage frontier fork');
            if (event < 0n || attempt < 0n) return this.value;
            if (event === 0n && JSON.stringify(record.usage) !== JSON.stringify(this.value.usage)) throw new Error('Context usage event fork');
            if (attempt === 0n && JSON.stringify(record.run_cache_usage) !== JSON.stringify(this.value.run_cache_usage)) throw new Error('Cache attempt fork');
            if (event === 0n && attempt === 0n) {
                if (record.record_digest !== this.value.record_digest) throw new Error('Context usage record fork');
                return this.value;
            }
        }
        this.value = record; return record;
    }
}

/** Bounded cold-history read. The source Store validates its persisted seal;
 * the client uses its actual root, never a child head or latest Run guess.
 * This does not create VerifiedTerminal/turn/permission authority. */
export async function readHistoricalRunContextUsage(owner: ChatTurnOwner, member: JournalCursor,
    projection: RunContextUsageProjection, transport: (request: Record<string, unknown>) => Promise<unknown>,
    current: () => boolean): Promise<CommittedRunContextUsage | null> {
    const query = async (request: Record<string, unknown>) => {
        if (!current()) return undefined;
        const raw = await transport(request);
        if (!current()) return undefined;
        if (!object(raw) || raw.source_host_id !== owner.hostId || !Object.hasOwn(raw, 'result')) throw new Error('Historical context source changed');
        return raw.result;
    };
    if (member.identity.conversation_id !== owner.conversationId) throw new Error('Historical context conversation changed');
    const seal = await query({ operation: 'terminal_seal', conversation_id: owner.conversationId, run_id: member.identity.run_id });
    if (seal === undefined || seal === null) return null;
    if (!object(seal) || seal.protocol_version !== 2 ||
        !object(seal.identity) || typeof seal.seal_digest !== 'string' || !/^[0-9a-f]{64}$/.test(seal.seal_digest)) throw new Error('Invalid historical context seal');
    parseRunResult(seal.result, owner.conversationId, member.identity.run_id);
    const admission = parseJournalCursor(seal.admission);
    if (admission.sequence === '0' || journalIdentityKey(admission.identity) !== journalIdentityKey(seal.identity as unknown as JournalIdentity) ||
        admission.identity.conversation_id !== owner.conversationId || admission.identity.run_id !== member.identity.run_id ||
        admission.identity.source_store_epoch !== member.identity.source_store_epoch) throw new Error('Historical context root differs');
    if (!current() || !projection.bind(owner, admission, true)) return projection.record;
    const usage = await query({ operation: 'context_usage', identity: identityWire(admission.identity) });
    if (usage === undefined) return null;
    return projection.accept(usage, owner, admission, current);
}

export function contextUsageFraction(record: CommittedRunContextUsage): number | null {
    const window = BigInt(record.usage.context_window_tokens);
    if (window === 0n) return null;
    const used = BigInt(record.usage.used_tokens);
    return Number((used > window ? window : used) * 10000n / window) / 10000;
}

export async function readCurrentRunContextUsage(owner: ChatTurnOwner, root: JournalIdentity,
    projection: RunContextUsageProjection, transport: (request: Record<string, unknown>) => Promise<unknown>,
    current: () => boolean): Promise<CommittedRunContextUsage | null> {
    const query = async (request: Record<string, unknown>) => {
        if (!current()) return undefined;
        const raw = await transport(request);
        if (!current()) return undefined;
        if (!object(raw) || raw.source_host_id !== owner.hostId || !Object.hasOwn(raw, 'result')) throw new Error('Live context source changed');
        return raw.result;
    };
    const rawAdmission = await query({ operation: 'admission_head', conversation_id: owner.conversationId, run_id: root.run_id });
    if (rawAdmission === undefined) return null;
    const admission = parseJournalCursor(rawAdmission);
    if (journalIdentityKey(admission.identity) !== journalIdentityKey(root)) throw new Error('Live context root changed');
    if (!current() || !projection.bind(owner, admission)) return null;
    const raw = await query({ operation: 'context_usage', identity: identityWire(root) });
    if (raw === undefined) return null;
    return projection.accept(raw, owner, admission, current);
}
