import { RunObserver } from './runObservation.ts';
import { reduceChatTurn } from './chatTurn.ts';
import type { ChatTurn } from './chatTurn.ts';

/** A new SSE challenge is a hint to read the complete permission authority.
 * Do not retire an acknowledged challenge based on event arrival order. */
export function refreshStreamPermissions(
    initial: ChatTurn,
    request: (body: Record<string, unknown>, signal: AbortSignal) => Promise<unknown>,
    current: () => ChatTurn | null,
    accept: (turn: ChatTurn, challenge: Record<string, unknown> | null) => void,
    failed: () => void,
): RunObserver | null {
    if (!initial.runId) return null;
    const observer = new RunObserver();
    const nonce = initial.permission?.nonce;
    const owned = () => {
        const value = current();
        return value && value.owner.generation === initial.owner.generation
            && value.owner.conversationId === initial.owner.conversationId
            && value.owner.hostId === initial.owner.hostId && value.owner.clientId === initial.owner.clientId
            && value.runId === initial.runId && value.permission?.nonce === nonce ? value : null;
    };
    void observer.run(initial.owner, request, async snapshot => {
        const value = owned();
        if (!value || !snapshot) return false;
        const result = reduceChatTurn(value, { type: 'permissions_observed', owner: initial.owner,
            challenges: snapshot.permissions, nowSeconds: Math.floor(Date.now() / 1000) });
        if (result.ignored) throw new Error('Invalid permission snapshot');
        const challenge = snapshot.permissions.find(item => item.nonce === result.state.permission?.nonce) ?? null;
        accept(result.state, challenge);
        return result.state.permission?.nonce === nonce && snapshot.recovery !== 'terminal';
    }, () => { if (owned()) failed(); },
    {run_id: initial.runId, assistant_message_id: initial.identity.assistantMessageId}, async () => {
        if (owned()) return true;
        observer.stop(); return false;
    });
    return observer;
}
