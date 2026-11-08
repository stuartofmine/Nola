import { chatActivityUrl, goalControlOutcome, goalActionRequest, validateGoalOperationId } from './chatActivity.ts';

export type GoalControlRequest = ReturnType<typeof goalActionRequest>;
export type GoalIntentLock = <T>(name: string, operation: () => Promise<T>) => Promise<T>;
const browserLock: GoalIntentLock = async (name, operation) => {
    if (!globalThis.navigator?.locks) throw new Error('Goal intent coordination is unavailable');
    return navigator.locks.request(name, operation);
};
type StoragePort = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;
/** Origin-scoped durable intent, not a second Goal state machine. */
export class GoalControlIntentStore {
    private readonly storage: StoragePort;
    readonly hostId: string;
    constructor(storage: StoragePort, hostId: string) {
        this.storage = storage; this.hostId = hostId;
        if (!hostId.trim()) throw new Error('Goal intent requires a Host identity');
    }
    private key(conversationId: string) { return `nola.goal-intent:${encodeURIComponent(this.hostId)}:${encodeURIComponent(conversationId)}`; }
    async locked<T>(conversationId: string, operation: () => Promise<T>, lock: GoalIntentLock = browserLock): Promise<T> {
        return lock(this.key(conversationId), operation);
    }
    read(conversationId: string): GoalControlRequest | null {
        const raw = this.storage.getItem(this.key(conversationId));
        if (raw === null) return null;
        const value = JSON.parse(raw);
        if (!value || value.hostId !== this.hostId || value.conversationId !== conversationId || !value.request
            || typeof value.request.operationId !== 'string' || !value.request.operationId.trim()
            || !['start', 'pause', 'resume', 'cancel'].includes(value.request.action)) throw new Error('Invalid saved Goal intent');
        const request = value.request;
        // Use the same bounded request contract for disk recovery and new input.
        this.validate(request);
        return value.request;
    }
    private validate(request: GoalControlRequest): void {
        const fields = request.action === 'start' ? ['operationId', 'action', 'objective']
            : ['operationId', 'action', 'expectedGoalId', 'expectedControlToken'];
        const record = request as unknown as Record<string, unknown>;
        if (Object.keys(record).some(key => !fields.includes(key))
            || fields.some(key => typeof record[key] !== 'string' || !(record[key] as string).trim())) throw new Error('Invalid saved Goal intent');
        if (request.action === 'start') {
            const normalized = goalActionRequest('start', request.objective, null, request.operationId);
            if (normalized.action !== 'start' || normalized.objective !== request.objective) throw new Error('Invalid saved Goal objective');
        } else {
            if (!['pause', 'resume', 'cancel'].includes(request.action)
                || !request.expectedGoalId?.trim() || !request.expectedControlToken?.trim()) throw new Error('Invalid Goal control identity');
            validateGoalOperationId(request.operationId);
        }
    }
    save(conversationId: string, request: GoalControlRequest): void {
        this.validate(request);
        const current = this.read(conversationId);
        if (current && JSON.stringify(current) !== JSON.stringify(request)) throw new Error('An earlier Goal intent remains unresolved');
        this.storage.setItem(this.key(conversationId), JSON.stringify({ hostId: this.hostId, conversationId, request }));
    }
    settle(conversationId: string, request: GoalControlRequest, response: unknown): boolean {
        if ((response as { source_host_id?: string })?.source_host_id !== this.hostId) throw new Error('Goal receipt Host changed');
        if (goalControlOutcome(response, conversationId, request.operationId) === 'pending') return false;
        const current = this.read(conversationId);
        if (current && JSON.stringify(current) !== JSON.stringify(request)) throw new Error('Goal intent ownership changed');
        this.storage.removeItem(this.key(conversationId));
        return true;
    }
}

export async function dispatchGoalControlIntent(
    conversationId: string, request: GoalControlRequest,
    api: (url: string, options: { method: string; body: string; signal: AbortSignal }) => Promise<unknown>,
    store: GoalControlIntentStore,
    lock: GoalIntentLock = browserLock,
): Promise<unknown> {
    return store.locked(conversationId, async () => {
        store.save(conversationId, request); // A failed durable write must prevent dispatch.
        // A disconnected Host must not hold the cross-tab lock indefinitely.
        // Timeout leaves the saved intent intact for receipt lookup or exact retry.
        const response = await api(chatActivityUrl(conversationId, 'goal'), {
            method: 'POST', body: JSON.stringify({ ...request, expectedHostId: store.hostId }),
            signal: AbortSignal.timeout(30_000),
        });
        const receipt = response as { source_host_id?: string; conversationId?: string; operationId?: string; authoritative?: boolean; controlPending?: boolean };
        if (!receipt || receipt.source_host_id !== store.hostId || receipt.conversationId !== conversationId || receipt.operationId !== request.operationId
            || !(receipt.authoritative === true || (receipt.authoritative === false && receipt.controlPending === true))) {
            throw new Error('Goal control response identity mismatch');
        }
        if ((response as { authoritative?: boolean })?.authoritative === true) store.settle(conversationId, request, response);
        return response;
    }, lock);
}
