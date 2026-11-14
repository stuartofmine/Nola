import { BrowserRecoveryIntentStore } from './unknownEffectRecovery.ts';
import { uiIdentityKey } from './generativeUi.ts';
import type { UiEdit, UiInteraction } from './generativeUi';

export interface UiPendingAttempt { input: UiInteraction; requestId: string; submittedAtMs: number }
export interface UiDraftState {
    drafts: Record<string, UiEdit>;
    revision: number | null;
    unresolved: UiPendingAttempt | null;
}

/** Recover the exact unacknowledged edits as well as the request identity.
 * A later rejection must not erase values that survived a cold page reload. */
export function draftStateFromAttempt(attempt: UiPendingAttempt): UiDraftState {
    return {
        drafts: Object.fromEntries(attempt.input.edits.map(edit =>
            [JSON.stringify([edit.instance_id, edit.binding]), structuredClone(edit)])),
        revision: attempt.input.expected_surface_revision,
        unresolved: structuredClone(attempt),
    };
}
/** Tab-lifetime presentation state only. Core remains the committed authority.
 * Full authority identities isolate Hosts, participants and generations. Values
 * are not written to disk or supplied to a model outside the normal submission. */
const entries = new Map<string, UiDraftState>();
export function readUiDraftState(identity: string): UiDraftState | undefined {
    const value = entries.get(identity);
    return value ? structuredClone(value) : undefined;
}
export function saveUiDraftState(identity: string, state: UiDraftState): void {
    if (!Object.keys(state.drafts).length && state.unresolved === null) entries.delete(identity);
    else entries.set(identity, JSON.parse(JSON.stringify(state)) as UiDraftState);
}

const durable = new BrowserRecoveryIntentStore();
export async function restoreUiAttempt(identity: string): Promise<UiPendingAttempt | null> {
    const wire = await durable.readUiAttempt(identity);
    if (wire === null) return null;
    const value = JSON.parse(wire) as UiPendingAttempt;
    if (!value?.input?.identity || uiIdentityKey(value.input.identity) !== identity || !value.requestId?.trim()
        || !Number.isSafeInteger(value.submittedAtMs) || value.submittedAtMs < 0
        || !Number.isSafeInteger(value.input.expected_surface_revision) || value.input.expected_surface_revision <= 0
        || !Array.isArray(value.input.edits)) throw new Error('Invalid persisted UI submission');
    return value;
}
export async function persistUiAttempt(identity: string, value: UiPendingAttempt): Promise<void> {
    await durable.writeUiAttempt(identity, JSON.stringify(value));
}
export async function retireUiAttempt(identity: string, value: UiPendingAttempt): Promise<void> {
    await durable.writeUiAttempt(identity, JSON.stringify(value), true);
}
