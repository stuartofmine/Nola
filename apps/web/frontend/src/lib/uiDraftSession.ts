import { uiIdentityKey } from './generativeUi.ts';
import type { UiEdit, UiInteraction } from './generativeUi';

export interface UiPendingAttempt { input: UiInteraction; requestId: string; submittedAtMs: number }
export interface UiDraftState {
    drafts: Record<string, UiEdit>;
    revision: number | null;
    unresolved: UiPendingAttempt | null;
}

export function draftStateFromAttempt(attempt: UiPendingAttempt): UiDraftState {
    return {
        drafts: Object.fromEntries(attempt.input.edits.map(edit =>
            [JSON.stringify([edit.instance_id, edit.binding]), structuredClone(edit)])),
        revision: attempt.input.expected_surface_revision,
        unresolved: structuredClone(attempt),
    };
}

const entries = new Map<string, UiDraftState>();
const attempts = new Map<string, string>();
const attemptKey = (identity: string) => `nola.v2.ui_attempt:${identity}`;

export function readUiDraftState(identity: string): UiDraftState | undefined {
    const value = entries.get(identity);
    return value ? structuredClone(value) : undefined;
}
export function saveUiDraftState(identity: string, state: UiDraftState): void {
    if (!Object.keys(state.drafts).length && state.unresolved === null) entries.delete(identity);
    else entries.set(identity, JSON.parse(JSON.stringify(state)) as UiDraftState);
}

export async function restoreUiAttempt(identity: string): Promise<UiPendingAttempt | null> {
    let wire: string | null = attempts.get(identity) ?? null;
    if (wire === null && typeof sessionStorage !== 'undefined') {
        try { wire = sessionStorage.getItem(attemptKey(identity)); } catch { wire = null; }
    }
    if (wire === null) return null;
    const value = JSON.parse(wire) as UiPendingAttempt;
    if (!value?.input?.identity || uiIdentityKey(value.input.identity) !== identity || !value.requestId?.trim()
        || !Number.isSafeInteger(value.submittedAtMs) || value.submittedAtMs < 0
        || !Number.isSafeInteger(value.input.expected_surface_revision) || value.input.expected_surface_revision <= 0
        || !Array.isArray(value.input.edits)) throw new Error('Invalid persisted UI submission');
    return value;
}

export async function persistUiAttempt(identity: string, value: UiPendingAttempt): Promise<void> {
    const wire = JSON.stringify(value);
    attempts.set(identity, wire);
    if (typeof sessionStorage !== 'undefined') {
        try { sessionStorage.setItem(attemptKey(identity), wire); } catch { /* optional */ }
    }
}

export async function retireUiAttempt(identity: string, _value: UiPendingAttempt): Promise<void> {
    attempts.delete(identity);
    if (typeof sessionStorage !== 'undefined') {
        try { sessionStorage.removeItem(attemptKey(identity)); } catch { /* optional */ }
    }
}
