import { AssistantJournalReader } from './assistantJournal.ts';
import { readTerminal } from './assistantTerminal.ts';
import type { VerifiedTerminal } from './assistantTerminal.ts';
import type { ChatTurn } from './chatTurn.ts';

function pause(milliseconds: number, signal: AbortSignal): Promise<void> {
    return new Promise(resolve => {
        const finish = () => { clearTimeout(timer); signal.removeEventListener('abort', finish); resolve(); };
        const timer = setTimeout(finish, milliseconds);
        signal.addEventListener('abort', finish, { once: true });
        if (signal.aborted) finish();
    });
}

/** Read-only, workspace-owned observation. A stream EOF is never completion.
 * No retries of submissions, no resume/cancel writes, and no transcript storage. */
export function createWorkspaceCompletions(options: {
    journal(body: Record<string, unknown>, signal: AbortSignal): Promise<unknown>;
    completed(evidence: VerifiedTerminal): void;
    wait?: (milliseconds: number, signal: AbortSignal) => Promise<void>;
}) {
    const active = new Map<string, { conversationId: string; controller: AbortController; done: Promise<void> }>();
    const finished = new Set<string>();
    return {
        watch(turn: ChatTurn): Promise<void> {
            if (!turn.runId || turn.phase === 'committed') return Promise.resolve();
            const captured = { ...turn, owner: { ...turn.owner }, identity: { ...turn.identity } };
            const key = JSON.stringify([captured.owner.hostId, captured.owner.clientId, captured.owner.conversationId,
                captured.runId, captured.identity.assistantMessageId]);
            if (finished.has(key)) return Promise.resolve();
            const existing = active.get(key);
            if (existing) return existing.done;
            const controller = new AbortController();
            const { signal } = controller;
            const run = async () => {
                let failures = 0;
                while (!signal.aborted) {
                    const reader = new AssistantJournalReader(captured.owner.hostId, captured.owner.conversationId,
                        captured.owner.generation, body => options.journal(body, signal));
                    let evidence: VerifiedTerminal | null = null;
                    try {
                        evidence = await readTerminal(captured, reader, body => options.journal(body, signal),
                            () => {}, () => !signal.aborted);
                        failures = 0;
                    } catch { failures++; /* Unavailable or invalid evidence stays unconfirmed. */ }
                    finally { reader.dispose(); }
                    if (signal.aborted) return;
                    if (evidence) {
                        finished.add(key);
                        if (finished.size > 200) finished.delete(finished.values().next().value!);
                        options.completed(evidence);
                        return;
                    }
                    await (options.wait ?? pause)(Math.min(30_000, 5_000 * Math.max(1, failures)), signal);
                }
            };
            const done = run().catch(() => { /* A notification failure cannot alter the Run. */ }).finally(() => {
                if (active.get(key)?.controller === controller) active.delete(key);
            });
            active.set(key, { conversationId: captured.owner.conversationId, controller, done });
            return done;
        },
        forget(conversationId: string) {
            for (const [key, entry] of active) if (entry.conversationId === conversationId) {
                entry.controller.abort(); active.delete(key);
            }
        },
        clear() { for (const entry of active.values()) entry.controller.abort(); active.clear(); finished.clear(); },
    };
}
