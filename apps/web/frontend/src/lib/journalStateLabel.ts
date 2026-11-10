/** Read-only lifecycle labels. Sealed describes execution settlement, never
 * acceptance of a user requirement. Unknown states do not imply success. */
const labels: Record<string, string> = {
    pending: 'chatPage.toolPending',
    queued: 'chatPage.stateQueued',
    running: 'chatPage.toolRunning',
    waiting: 'chatPage.stateWaiting',
    waiting_for_input: 'chatPage.stateWaitingInput',
    waiting_for_permission: 'chatPage.stateWaitingPermission',
    suspended: 'chatPage.stateSuspended',
    retry_waiting: 'chatPage.stateRetryWaiting',
    sealed: 'chatPage.stateSealed',
    completed: 'chatPage.toolCompleted',
    failed: 'chatPage.toolFailed',
    cancelled: 'chatPage.stateCancelled',
};

export function journalStateLabelKey(state: unknown): string | null {
    if (typeof state !== 'string' || state.length === 0) return null;
    return Object.hasOwn(labels, state) ? labels[state] : 'chatPage.stateUnknown';
}

/** An Error Part is itself a failure fact; it need not also carry Activity.state. */
export function journalPartStateLabelKey(kind: unknown, state: unknown): string | null {
    return kind === 'error' ? 'chatPage.toolFailed' : journalStateLabelKey(state);
}
