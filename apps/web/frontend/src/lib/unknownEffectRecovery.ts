export interface RecoveryOwner {
    expected_host_id: string; client_id: string; conversation_id: string; run_id: string; assistant_message_id: string;
}
export interface RecoveryTarget {
    effect_id: string; request_id: string; attempt_id: string; worker_generation: string;
    lease_owner: string; lease_token: string; expected_attempt_revision: number;
}
export interface RecoveryInspection {
    conversation_id: string; run_id: string; assistant_message_id: string; invocation_config_digest: string;
    targets: RecoveryTarget[]; independent_pending_effects: number; retry_allowed: boolean;
}
export type RecoveryChoice = 'authorize_risk_retry' | 'keep_unknown_blocked';
export interface RecoveryDecision {
    owner: RecoveryOwner; request_id: string;
    input: { conversation_id: string; run_id: string; invocation_config_digest: string; resolution_id: string;
        targets: RecoveryTarget[]; choice: { kind: RecoveryChoice; reason: string }; resolved_at_ms: number };
}
export type RecoveryResult = 'pending' | 'retry_authorized' | 'unknown_preserved_blocked' | 'rejected' | 'conflict';
const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === 'object' && !Array.isArray(value);
const bytes = (value: string) => new TextEncoder().encode(value).length;
const id = (value: unknown): value is string => typeof value === 'string' && value.length > 0 && value.trim() === value
    && bytes(value) <= 512 && !/[\u0000-\u001f\u007f-\u009f]/u.test(value);
const count = (value: unknown): value is number => Number.isSafeInteger(value) && Number(value) >= 0;
function keys(value: unknown, fields: string[]): asserts value is Record<string, unknown> {
    if (!object(value) || Object.keys(value).length !== fields.length || fields.some(key => !Object.hasOwn(value, key))) throw new Error('Invalid recovery contract');
}
const ownerFields = ['expected_host_id', 'client_id', 'conversation_id', 'run_id', 'assistant_message_id'];
export function recoveryKey(owner: RecoveryOwner): string {
    keys(owner, ownerFields);
    if (Object.values(owner).some(value => !id(value))) throw new Error('Invalid recovery owner');
    return JSON.stringify(ownerFields.map(field => owner[field as keyof RecoveryOwner]));
}
function target(value: unknown): RecoveryTarget {
    const fields = ['effect_id', 'request_id', 'attempt_id', 'worker_generation', 'lease_owner', 'lease_token'];
    keys(value, [...fields, 'expected_attempt_revision']);
    if (fields.some(key => !id(value[key])) || !count(value.expected_attempt_revision) || value.expected_attempt_revision < 1) throw new Error('Invalid recovery target');
    return value as unknown as RecoveryTarget;
}
function inspection(value: unknown, owner: RecoveryOwner): RecoveryInspection {
    keys(value, ['conversation_id', 'run_id', 'assistant_message_id', 'invocation_config_digest', 'targets', 'independent_pending_effects', 'retry_allowed']);
    if (value.conversation_id !== owner.conversation_id || value.run_id !== owner.run_id || value.assistant_message_id !== owner.assistant_message_id
        || !id(value.invocation_config_digest) || typeof value.retry_allowed !== 'boolean' || !count(value.independent_pending_effects) || !Array.isArray(value.targets)
        || value.targets.length > 256) throw new Error('Recovery inspection owner changed');
    const targets = value.targets.map(target);
    if (new Set(targets.map(item => item.effect_id)).size !== targets.length) throw new Error('Duplicate recovery effect');
    return { ...value, targets } as unknown as RecoveryInspection;
}
function envelope(value: unknown, owner: RecoveryOwner): Record<string, unknown> {
    keys(value, ['source_host_id', 'owner', 'result']);
    if (value.source_host_id !== owner.expected_host_id || !object(value.owner)
        || recoveryKey(value.owner as unknown as RecoveryOwner) !== recoveryKey(owner) || !object(value.result)) throw new Error('Recovery response owner changed');
    return value.result;
}
export function parseRecoveryInspection(value: unknown, owner: RecoveryOwner): RecoveryInspection | null {
    const result = envelope(value, owner); keys(result, ['inspection']);
    if (result.inspection === null) return null;
    const parsed = inspection(result.inspection, owner);
    return parsed.targets.length ? parsed : null;
}
export function parseRecoveryDecision(wire: string, owner: RecoveryOwner): RecoveryDecision {
    if (bytes(wire) > 8 * 1024 * 1024) throw new Error('Recovery intent too large');
    const value: unknown = JSON.parse(wire); keys(value, ['owner', 'request_id', 'input']);
    if (!object(value.owner) || recoveryKey(value.owner as unknown as RecoveryOwner) !== recoveryKey(owner) || !id(value.request_id)) throw new Error('Recovery intent owner changed');
    const input = value.input;
    keys(input, ['conversation_id', 'run_id', 'invocation_config_digest', 'resolution_id', 'targets', 'choice', 'resolved_at_ms']);
    if (input.conversation_id !== owner.conversation_id || input.run_id !== owner.run_id || !id(input.invocation_config_digest)
        || !id(input.resolution_id) || !count(input.resolved_at_ms) || !Array.isArray(input.targets)
        || !input.targets.length || input.targets.length > 256) throw new Error('Invalid recovery intent');
    keys(input.choice, ['kind', 'reason']);
    if (!['authorize_risk_retry', 'keep_unknown_blocked'].includes(String(input.choice.kind)) || typeof input.choice.reason !== 'string'
        || !input.choice.reason.trim() || bytes(input.choice.reason) > 4096) throw new Error('Invalid recovery choice');
    const targets = input.targets.map(target);
    if (new Set(targets.map(item => item.effect_id)).size !== targets.length
        || input.choice.kind === 'authorize_risk_retry' && targets.length !== 1) throw new Error('Invalid recovery selection');
    return value as unknown as RecoveryDecision;
}
export function createRecoveryDecision(owner: RecoveryOwner, inspected: RecoveryInspection, choice: RecoveryChoice,
    targetIndex: number, reason: string, requestId: string, resolutionId: string, now: number): string {
    const captured = inspection(inspected, owner);
    if (choice === 'authorize_risk_retry' && !captured.retry_allowed) throw new Error('Only preserving unknown is permitted');
    if (choice === 'keep_unknown_blocked' && captured.independent_pending_effects !== 0) throw new Error('Independent work still pending');
    if (choice === 'authorize_risk_retry' && (!count(targetIndex) || !captured.targets[targetIndex])) throw new Error('Select one effect');
    const wire = JSON.stringify({ owner, request_id: requestId, input: {
        conversation_id: owner.conversation_id, run_id: owner.run_id, invocation_config_digest: captured.invocation_config_digest,
        resolution_id: resolutionId, targets: choice === 'authorize_risk_retry' ? [captured.targets[targetIndex]] : captured.targets,
        choice: { kind: choice, reason }, resolved_at_ms: now,
    } });
    parseRecoveryDecision(wire, owner);
    return wire;
}
function receipt(value: unknown, owner: RecoveryOwner, decision: RecoveryDecision): RecoveryResult | null {
    const result = envelope(value, owner); keys(result, ['request_id', 'receipt']);
    if (result.request_id !== decision.request_id) throw new Error('Recovery request changed');
    if (result.receipt === null) return null;
    const row = result.receipt;
    if (!object(row)) throw new Error('Invalid recovery receipt');
    if (row.status === 'accepted') { keys(row, ['status']); return 'pending'; }
    if (row.status === 'rejected' || row.status === 'conflict') { keys(row, ['status']); return row.status; }
    keys(row, ['status', 'result']);
    if (row.status !== 'committed' || !object(row.result)) throw new Error('Invalid recovery receipt');
    const reply = row.result;
    if (reply.outcome === 'rejected' || reply.outcome === 'conflict') {
        keys(reply, ['outcome', 'code']); if (!id(reply.code)) throw new Error('Invalid recovery rejection'); return reply.outcome;
    }
    keys(reply, ['outcome', 'run_id', 'resolution_id']);
    const expected = decision.input.choice.kind === 'authorize_risk_retry' ? 'retry_authorized' : 'unknown_preserved_blocked';
    if (reply.run_id !== owner.run_id || reply.resolution_id !== decision.input.resolution_id || reply.outcome !== expected) throw new Error('Recovery result changed');
    return expected;
}

export interface RecoveryIntentStore {
    read(owner: RecoveryOwner): Promise<string | null>;
    create(owner: RecoveryOwner, wire: string): Promise<void>;
    clear(owner: RecoveryOwner, wire: string): Promise<void>;
}
/** IndexedDB's read-write transaction arbitrates competing tabs. No overwrite,
 * no in-memory fallback when durable storage is unavailable. */
export class BrowserRecoveryIntentStore implements RecoveryIntentStore {
    private open(): Promise<IDBDatabase> {
        return new Promise((resolve, reject) => {
            const request = indexedDB.open('nola.recovery-intents', 1);
            request.onupgradeneeded = () => request.result.createObjectStore('intents');
            request.onsuccess = () => resolve(request.result);
            request.onerror = () => reject(request.error);
            request.onblocked = () => reject(new Error('Recovery storage blocked'));
        });
    }
    /** Same durable intent store, separate namespace from recovery decisions. */
    async readUiAttempt(identity: string): Promise<string | null> {
        const db = await this.open();
        return new Promise((resolve, reject) => {
            const tx = db.transaction('intents', 'readonly');
            const request = tx.objectStore('intents').get(`ui-surface:${identity}`);
            tx.oncomplete = () => { db.close();
                if (request.result !== undefined && typeof request.result !== 'string') reject(new Error('Invalid UI intent'));
                else resolve(request.result ?? null);
            };
            tx.onabort = () => { db.close(); reject(tx.error); };
        });
    }
    async writeUiAttempt(identity: string, wire: string, clear = false): Promise<void> {
        const db = await this.open();
        return new Promise((resolve, reject) => {
            const tx = db.transaction('intents', 'readwrite', {durability: 'strict'});
            const store = tx.objectStore('intents');
            const key = `ui-surface:${identity}`;
            const read = store.get(key);
            read.onsuccess = () => {
                if (read.result !== undefined && read.result !== wire) { tx.abort(); return; }
                if (clear) store.delete(key); else store.put(wire, key);
            };
            tx.oncomplete = () => { db.close(); resolve(); };
            tx.onabort = () => { db.close(); reject(tx.error ?? new Error('Another UI submission is unresolved')); };
        });
    }
    async listOwners(host: string, client: string, conversation: string): Promise<RecoveryOwner[]> {
        if (![host, client, conversation].every(id)) throw new Error('Invalid recovery owner');
        const prefix = JSON.stringify([host, client, conversation]).slice(0, -1) + ',';
        const db = await this.open();
        return new Promise((resolve, reject) => {
            const tx = db.transaction('intents', 'readonly'); const owners: RecoveryOwner[] = [];
            const request = tx.objectStore('intents').openCursor(IDBKeyRange.bound(prefix, prefix + '\uffff'));
            let failure: unknown;
            request.onsuccess = () => {
                const cursor = request.result; if (!cursor) return;
                try {
                    if (typeof cursor.key !== 'string' || typeof cursor.value !== 'string' || owners.length >= 256) throw new Error('Invalid recovery intent list');
                    const fields: unknown = JSON.parse(cursor.key);
                    if (!Array.isArray(fields) || fields.length !== ownerFields.length || !fields.every(id)) throw new Error('Invalid recovery key');
                    const owner = Object.fromEntries(ownerFields.map((field, index) => [field, fields[index]])) as unknown as RecoveryOwner;
                    if (owner.expected_host_id !== host || owner.client_id !== client || owner.conversation_id !== conversation) throw new Error('Recovery scope changed');
                    parseRecoveryDecision(cursor.value, owner); owners.push(owner); cursor.continue();
                } catch (error) { failure = error; tx.abort(); }
            };
            tx.oncomplete = () => { db.close(); resolve(owners); };
            tx.onabort = () => { db.close(); reject(failure ?? tx.error); };
        });
    }
    async read(owner: RecoveryOwner): Promise<string | null> {
        const db = await this.open();
        return new Promise((resolve, reject) => {
            const tx = db.transaction('intents', 'readonly');
            const request = tx.objectStore('intents').get(recoveryKey(owner));
            tx.oncomplete = () => { db.close(); try {
                if (request.result === undefined) { resolve(null); return; }
                if (typeof request.result !== 'string') throw new Error('Invalid stored recovery intent');
                parseRecoveryDecision(request.result, owner); resolve(request.result);
            } catch (error) { reject(error); } };
            tx.onabort = () => { db.close(); reject(tx.error); };
        });
    }
    async create(owner: RecoveryOwner, wire: string): Promise<void> {
        parseRecoveryDecision(wire, owner);
        const db = await this.open();
        return new Promise((resolve, reject) => {
            const tx = db.transaction('intents', 'readwrite', { durability: 'strict' });
            tx.objectStore('intents').add(wire, recoveryKey(owner));
            tx.oncomplete = () => { db.close(); resolve(); };
            tx.onabort = () => { db.close(); reject(tx.error ?? new Error('Recovery intent already exists')); };
        });
    }
    async clear(owner: RecoveryOwner, wire: string): Promise<void> {
        const db = await this.open();
        return new Promise((resolve, reject) => {
            const tx = db.transaction('intents', 'readwrite', { durability: 'strict' });
            const store = tx.objectStore('intents'); const request = store.get(recoveryKey(owner));
            request.onsuccess = () => {
                if (request.result !== wire) { tx.abort(); return; }
                store.delete(recoveryKey(owner));
            };
            tx.oncomplete = () => { db.close(); resolve(); };
            tx.onabort = () => { db.close(); reject(tx.error ?? new Error('Recovery intent changed')); };
        });
    }
}

/** Network/protocol failure never retires intent. A null receipt permits only
 * the exact original command, not a fresh resolution ID or time. */
export async function recoverDecision(owner: RecoveryOwner, wire: string, store: RecoveryIntentStore,
    request: (body: string) => Promise<unknown>): Promise<RecoveryResult> {
    const decision = parseRecoveryDecision(wire, owner);
    if (await store.read(owner) !== wire) throw new Error('Recovery intent changed');
    let result = receipt(await request(JSON.stringify({ operation: 'receipt', decision })), owner, decision);
    if (result === null) result = receipt(await request(JSON.stringify({ operation: 'resolve', decision })), owner, decision);
    if (result === null || result === 'pending') return 'pending';
    await store.clear(owner, wire);
    return result;
}

/** Parent authority is independent from the child's read-only history. */
export interface OwnedTaskRecoveryContext {
    host_id: string; conversation_id: string; parent_run_id: string; task_id: string;
}
