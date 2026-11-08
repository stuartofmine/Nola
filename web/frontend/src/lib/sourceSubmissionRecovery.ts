import { acceptSourceReceipt, settledSourceSubmission, sourceSubmissionQuery } from './sourceSubmission.ts';
import type { PendingSourceSubmission, SourceSubmissionStorage } from './sourceSubmission.ts';

function wait(signal: AbortSignal): Promise<void> {
    return new Promise(resolve => {
        const finish = () => { clearTimeout(timer); signal.removeEventListener('abort', finish); resolve(); };
        const timer = setTimeout(finish, 5000);
        signal.addEventListener('abort', finish, { once: true });
        if (signal.aborted) finish();
    });
}

/** Observe the original receipt only. Missing ACKs never authorize redispatch. */
export function createSourceSubmissionRecovery(options: {
    storage(): SourceSubmissionStorage;
    observe(body: Record<string, unknown>, signal: AbortSignal): Promise<unknown>;
    admitted(pending: PendingSourceSubmission): void;
    wait?: (signal: AbortSignal) => Promise<void>;
}) {
    const active = new Map<string, { conversationId: string; controller: AbortController; done: Promise<void> }>();
    return {
        watch(pending: PendingSourceSubmission): Promise<void> {
            if (!pending.sent || !pending.request_digest || settledSourceSubmission(pending)) return Promise.resolve();
            const original = structuredClone(pending);
            const id = original.identity;
            const key = JSON.stringify([id.source_host_id, id.client_id, id.conversation_id, id.submission_id, id.user_message_id, id.assistant_message_id]);
            const existing = active.get(key);
            if (existing) return existing.done;
            const controller = new AbortController(); const { signal } = controller;
            const current = () => {
                if (signal.aborted) return null;
                const value = options.storage().read(id.source_host_id, id.client_id, id.conversation_id);
                return value?.sent && value.request_digest === original.request_digest
                    && JSON.stringify(value.identity) === JSON.stringify(id)
                    && JSON.stringify(value.request) === JSON.stringify(original.request) ? value : null;
            };
            const run = async () => {
                while (!signal.aborted) {
                    try {
                        let latest = current();
                        if (!latest || settledSourceSubmission(latest)) return;
                        if (latest.receipt?.phase !== 'admitted') {
                            const response = await options.observe(sourceSubmissionQuery(latest), signal);
                            latest = current();
                            if (!latest || settledSourceSubmission(latest)) return;
                            if (latest.receipt?.phase !== 'admitted') {
                                if (!response || typeof response !== 'object' || !('source_host_id' in response)
                                    || response.source_host_id !== id.source_host_id || !('result' in response)) throw new Error('Receipt source changed');
                                if (response.result !== null) {
                                    latest = acceptSourceReceipt(latest, response.result);
                                    options.storage().save(latest);
                                }
                            }
                        }
                        if (latest.receipt?.phase === 'admitted') { options.admitted(latest); return; }
                        if (settledSourceSubmission(latest)) return;
                    } catch { /* Keep uncertainty and retry only the receipt read. */ }
                    if (!signal.aborted) await (options.wait ?? wait)(signal);
                }
            };
            const done = run().catch(() => {}).finally(() => {
                if (active.get(key)?.controller === controller) active.delete(key);
            });
            active.set(key, { conversationId: id.conversation_id, controller, done });
            return done;
        },
        forget(conversationId: string) {
            for (const [key, entry] of active) if (entry.conversationId === conversationId) {
                entry.controller.abort(); active.delete(key);
            }
        },
        clear() { for (const entry of active.values()) entry.controller.abort(); active.clear(); },
    };
}
