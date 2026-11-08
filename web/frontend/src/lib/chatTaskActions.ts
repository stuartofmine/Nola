import type { TimelinePart } from './chatTimeline.ts';

export interface TaskOwner { conversationId: string; runId: string; hostId: string }
export interface TaskControl { taskId: string; identity: string; guidance: boolean; cancellation: boolean; cancellationObserved: boolean; terminal: boolean }
export type TaskIntentLock = <T>(name: string, operation: () => Promise<T>) => Promise<T>;
const browserIntentLock: TaskIntentLock = async (name, operation) => {
    if (!globalThis.navigator?.locks) throw new Error('Task intent coordination is unavailable');
    return navigator.locks.request(name, operation);
};
type IntentStore = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;
export type TaskActionState = 'ready' | 'sending' | 'queued' | 'cancelRequested' | 'unknown' | 'rejected' | 'delivered';
const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === 'object' && !Array.isArray(value);

/** A historical Task keeps its original parent Run, never the current turn. */
export function journalTaskOwner(part: TimelinePart, source: Pick<TaskOwner,'hostId'|'conversationId'>): TaskOwner | null {
    if (part.source!=='journal' || !source.hostId || !source.conversationId) return null;
    let identity:unknown;
    try { identity=JSON.parse(part.journalOwner); } catch { return null; }
    if (!Array.isArray(identity) || identity.length!==4 || identity.some(value=>typeof value!=='string' || !value)
        || identity[1]!==source.conversationId) return null;
    return {hostId:source.hostId,conversationId:source.conversationId,runId:identity[2]};
}

/** Capability flags come only from an owned committed TaskControl publication. */
export function taskControl(part: TimelinePart, owner: TaskOwner): TaskControl | null {
    if (part.source !== 'journal' || part.value.kind !== 'task_control' || !owner.hostId || !owner.runId || !owner.conversationId) return null;
    let identity: unknown;
    try { identity = JSON.parse(part.journalOwner); } catch { return null; }
    if (!Array.isArray(identity) || identity.length !== 4 || identity.some(item => typeof item !== 'string' || !item)
        || identity[1] !== owner.conversationId || identity[2] !== owner.runId) return null;
    const value = part.value;
    if (typeof value.task_id !== 'string' || !value.task_id.trim() || value.task_id.trim() !== value.task_id
        || typeof value.cancel_requested !== 'boolean' || !['pending', 'running', 'waiting_for_permission', 'waiting_for_input', 'completed', 'sealed', 'cancelled', 'failed'].includes(String(value.state))) return null;
    const active = value.cancel_requested === false && ['pending', 'running', 'waiting_for_permission', 'waiting_for_input'].includes(String(value.state));
    const guidance = active && value.supports_guidance === true, cancellation = active && value.supports_cancellation === true;
    return {taskId: value.task_id, identity: JSON.stringify([owner.hostId, ...identity, value.task_id]), guidance, cancellation, cancellationObserved: value.cancel_requested, terminal: ['completed', 'sealed', 'cancelled', 'failed'].includes(String(value.state))};
}

/** Record uncertainty before dispatch. Remount/reload must not silently resend a guidance message. */
type GuidanceIntent = {hostId: string; conversationId: string; parentRunId: string; taskId: string; messageId: string; submittedAtMs: number; message: string};
export class TaskActionSubmission {
    state: TaskActionState = 'ready';
    private readonly key: string;
    private intent: GuidanceIntent | null = null;
    private cancellationPending = false;
    private savedIntent: string | null = null;
    private inFlight = false;
    readonly owner: TaskOwner;
    readonly control: TaskControl;
    private readonly storage: IntentStore;
    get canRecover(): boolean { return (this.state === 'unknown' || this.state === 'cancelRequested') && (this.intent !== null || this.cancellationPending); }
    get pendingGuidanceText(): string | null { return this.intent?.message ?? null; }
    private readonly lock: TaskIntentLock;
    constructor(owner: TaskOwner, control: TaskControl, storage: IntentStore, lock: TaskIntentLock = browserIntentLock) {
        this.lock = lock;
        this.owner = {...owner}; this.control = {...control}; this.storage = storage;
        this.key = `nola.web.task-intent.v1:${control.identity}`;
        try {
            const saved = storage.getItem(this.key);
            this.savedIntent = saved;
            if (saved) {
                this.state = 'unknown';
                const value = JSON.parse(saved);
                if (object(value) && value.kind === 'cancellation' && value.hostId === owner.hostId
                    && value.conversationId === owner.conversationId && value.parentRunId === owner.runId
                    && value.taskId === control.taskId) {
                    this.cancellationPending = true;
                    if (value.acknowledged === true) this.state = 'cancelRequested';
                }
                if (object(value) && value.hostId === owner.hostId && value.conversationId === owner.conversationId
                    && value.parentRunId === owner.runId && value.taskId === control.taskId
                    && typeof value.messageId === 'string' && value.messageId.trim()
                    && Number.isSafeInteger(value.submittedAtMs) && Number(value.submittedAtMs) >= 0
                    && typeof value.message === 'string' && value.message.trim() && value.message.length <= 4096)
                    this.intent = value as GuidanceIntent;
            }
        } catch { this.state = 'unknown'; }
        this.refreshControl(control);
    }
    refreshControl(control: TaskControl): void {
        if (control.identity !== this.control.identity) return;
        Object.assign(this.control, control);
        if (!this.cancellationPending) return;
        // Only the owned committed Core publication settles an uncertain stop.
        // Neither a timeout nor a missing capability is evidence of completion.
        if (control.terminal) {
            // Synchronous rendering never mutates cross-tab storage. Cleanup
            // belongs to the next lock-protected receipt/submission operation.
            this.cancellationPending = false; this.state = 'ready';
        } else if (control.cancellationObserved && this.state !== 'unknown') {
            this.state = 'cancelRequested';
        }
    }
    private removeExactIntent(): void {
        if (this.savedIntent === null || this.storage.getItem(this.key) !== this.savedIntent)
            throw new Error('Task intent changed in another tab');
        this.storage.removeItem(this.key); this.savedIntent = null;
    }
    private persistIntent(value: object): void {
        if (this.storage.getItem(this.key) !== this.savedIntent)
            throw new Error('Task intent changed in another tab');
        const encoded = JSON.stringify(value);
        this.storage.setItem(this.key, encoded); this.savedIntent = encoded;
    }
    private reloadIntent(): void {
        const current = new TaskActionSubmission(this.owner, this.control, this.storage, this.lock);
        this.intent = current.intent; this.cancellationPending = current.cancellationPending;
        this.savedIntent = current.savedIntent; this.state = current.state;
        if (this.control.terminal && this.savedIntent !== null) {
            const saved = JSON.parse(this.savedIntent);
            if (object(saved) && saved.kind === 'cancellation') this.removeExactIntent();
        }
    }
    private cancellationIntent(acknowledged: boolean): object {
        return {kind: 'cancellation', hostId: this.owner.hostId, conversationId: this.owner.conversationId,
            parentRunId: this.owner.runId, taskId: this.control.taskId, acknowledged};
    }
    private acceptReceipt(result: unknown): boolean {
        const intent = this.intent;
        if (!intent || !object(result) || result.hostId !== intent.hostId || result.conversationId !== intent.conversationId
            || result.taskId !== intent.taskId || result.messageId !== intent.messageId || !Object.hasOwn(result, 'guidance')) throw new Error('Mismatched guidance receipt');
        if (result.guidance === null) return false;
        const record = result.guidance;
        if (!object(record) || record.task_id !== intent.taskId || record.message_id !== intent.messageId
            || record.owner_session_id !== intent.conversationId || record.content !== intent.message
            || record.submitted_at_ms !== intent.submittedAtMs
            || !['submitted', 'accepted', 'delivered', 'rejected'].includes(String(record.status))) throw new Error('Invalid guidance record');
        this.removeExactIntent(); this.intent = null;
        this.state = record.status === 'rejected' ? 'rejected' : record.status === 'delivered' ? 'delivered' : 'queued';
        return true;
    }
    private cancellationOwner(): object {
        return {hostId: this.owner.hostId, conversationId: this.owner.conversationId,
            parentRunId: this.owner.runId, taskId: this.control.taskId};
    }
    private acceptCancellation(result: unknown, requestCompleted: boolean): boolean {
        if (!object(result) || result.hostId !== this.owner.hostId || result.conversationId !== this.owner.conversationId
            || result.parentRunId !== this.owner.runId || result.taskId !== this.control.taskId
            || result.applicationPending !== false || typeof result.cancelRequested !== 'boolean'
            || typeof result.terminal !== 'boolean') throw new Error('Unconfirmed cancellation owner');
        const states = result.terminal ? ['sealed', 'failed', 'cancelled']
            : ['queued', 'running', 'retry_waiting', 'waiting_permission', 'suspended', 'awaiting_reconciliation'];
        if (!states.includes(String(result.state))) throw new Error('Invalid task cancellation fact');
        if (result.terminal) {
            this.removeExactIntent(); this.cancellationPending = false; this.state = 'ready'; return true;
        }
        if (result.cancelRequested) {
            this.persistIntent(this.cancellationIntent(true));
            this.state = 'cancelRequested'; return requestCompleted;
        }
        return false;
    }
    private async reconcileCancellation(send: (path: string, body: object) => Promise<unknown>): Promise<void> {
        const owner = this.cancellationOwner();
        if (this.acceptCancellation(await send('/api/chat/agent/v2/task/cancel/receipt', owner), false)) return;
        if (!this.acceptCancellation(await send('/api/chat/agent/v2/task/cancel', owner), true))
            throw new Error('Cancellation was not durably requested');
    }
    async recover(send: (path: string, body: object) => Promise<unknown>): Promise<void> {
        if (this.inFlight) return;
        this.inFlight = true; this.state = 'sending';
        try { await this.lock(this.key, async () => {
            this.reloadIntent(); await this.recoverLocked(send);
        }); } catch { this.state = 'unknown'; } finally { this.inFlight = false; }
    }
    async submit(action: 'guidance' | 'cancel', message: string, send: (path: string, body: object) => Promise<unknown>): Promise<void> {
        if (this.inFlight) return;
        this.inFlight = true; this.state = 'sending';
        try { await this.lock(this.key, async () => {
            this.reloadIntent(); await this.submitLocked(action, message, send);
        }); } catch { this.state = 'unknown'; } finally { this.inFlight = false; }
    }
    private async recoverLocked(send: (path: string, body: object) => Promise<unknown>): Promise<void> {
        if (!this.canRecover) return;
        this.state = 'sending';
        try {
            if (this.cancellationPending) { await this.reconcileCancellation(send); return; }
            const intent = this.intent!;
            if (this.acceptReceipt(await send('/api/chat/agent/v2/task/message/receipt', intent))) return;
            if (!this.acceptReceipt(await send('/api/chat/agent/v2/task/message', intent))) throw new Error('Missing submitted guidance');
        } catch { this.state = 'unknown'; }
    }
    private async submitLocked(action: 'guidance' | 'cancel', message: string, send: (path: string, body: object) => Promise<unknown>): Promise<void> {
        if (!['ready', 'queued', 'rejected', 'delivered'].includes(this.state) || (action === 'guidance' ? !this.control.guidance : !this.control.cancellation)) return;
        const text = message.trim();
        if (action === 'guidance' && (!text || text.length > 4096)) return;
        try {
            if (action === 'guidance') {
                this.intent = {hostId:this.owner.hostId, conversationId:this.owner.conversationId, parentRunId:this.owner.runId,
                    taskId:this.control.taskId, messageId:crypto.randomUUID(), submittedAtMs:Date.now(), message:text};
                this.persistIntent(this.intent);
            } else {
                this.cancellationPending = true;
                this.persistIntent(this.cancellationIntent(false));
            }
        } catch { this.state = 'unknown'; this.intent = null; return; }
        this.state = 'sending';
        try {
            if (action === 'guidance') {
                if (!this.acceptReceipt(await send('/api/chat/agent/v2/task/message', this.intent!))) throw new Error('Missing guidance receipt');
            } else {
                await this.reconcileCancellation(send);
            }
        } catch { this.state = action === 'cancel' && !this.cancellationPending && this.control.terminal ? 'ready' : 'unknown'; }
        if (action === 'cancel' && this.cancellationPending) this.refreshControl(this.control);
    }
}
