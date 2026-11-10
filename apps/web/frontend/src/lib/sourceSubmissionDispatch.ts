import { acceptSourceReceipt, bindSourceDigest, sourceSubmissionBody } from './sourceSubmission.ts';
import type { PendingSourceSubmission, SourceSubmissionStorage } from './sourceSubmission.ts';

/** Only emitted before any original stream dispatch, after retiring the unsent intent. */
export class SourceSubmissionNotDispatchedError extends Error {
    constructor() { super('Source submission was not dispatched'); }
}

/** The workspace, not a particular mounted page, owns the original dispatch.
 * UI callbacks are observers. Durable source receipts remain available to a new
 * page even when the original observer has gone away. Never retries dispatch. */
export async function dispatchSourceSubmission(pending: PendingSourceSubmission, options: {
    storage: SourceSubmissionStorage;
    signal: AbortSignal;
    prepare: (pending: PendingSourceSubmission, signal: AbortSignal) => Promise<unknown>;
    stream: (body: Record<string, unknown>, signal: AbortSignal) => AsyncIterable<Record<string, unknown>>;
    bound?: (pending: PendingSourceSubmission) => void | Promise<void>;
    onReceipt?: (pending: PendingSourceSubmission) => void | Promise<void>;
    event?: (event: Record<string, unknown>) => Promise<void>;
}): Promise<void> {
    const current = () => {
        const id = pending.identity;
        const value = options.storage.read(id.source_host_id, id.client_id, id.conversation_id);
        return value && JSON.stringify(value.identity) === JSON.stringify(id)
            && JSON.stringify(value.request) === JSON.stringify(pending.request) ? value : null;
    };
    options.signal.throwIfAborted();
    let value = current();
    if (!value || value.sent || value.receipt) return;
    let body: Record<string, unknown>;
    try {
        const response = await options.prepare(value, options.signal);
        options.signal.throwIfAborted();
        value = current();
        // Cancellation, a successor, or another dispatcher may have won while preparing.
        if (!value || value.sent || value.receipt) return;
        value = bindSourceDigest(value, response);
        body = sourceSubmissionBody(value);
    } catch (cause) {
        const latest = current();
        if (latest && !latest.sent && !latest.receipt) {
            options.storage.clear(latest);
            throw new SourceSubmissionNotDispatchedError();
        }
        throw cause;
    }
    value = {...value, sent: true};
    options.storage.save(value);
    await options.bound?.(value);
    options.signal.throwIfAborted();
    for await (const event of options.stream(body, options.signal)) {
        options.signal.throwIfAborted();
        const latest = current();
        if (!latest) return;
        if (event.type === 'submission_receipt') {
            const accepted = acceptSourceReceipt(latest, event.receipt);
            options.storage.save(accepted);
            await options.onReceipt?.(accepted);
        }
        await options.event?.(event);
    }
}

/** Hydrate journals in order while allowing permission/control frames through. */
export function journalEventQueue(apply: (event: Record<string, unknown>) => Promise<void>) {
    let pending = Promise.resolve(); let failure: unknown = null;
    return {
        async push(event: Record<string, unknown>) {
            if (event.type === 'assistant_journal_committed_v1') {
                pending = pending.then(() => apply(event)).catch(error => { failure = error; });
            } else { await apply(event); }
        },
        async flush() { await pending; if (failure) throw failure; },
    };
}
