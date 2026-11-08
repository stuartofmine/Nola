import { api } from './api.ts';
/** Core-owned projections. No A2UI wire parsing or expression evaluation here. */
export interface UiSurfaceOwner {
    authority_id: string;
    conversation_id: string;
    participant_id: string;
    context_id: string;
    originating_run_id: string;
    message_id: string;
    part_id: string;
}
export interface UiSurfaceIdentity { owner: UiSurfaceOwner; surface_id: string; generation: number }
export interface UiActionRef { instance_id: string; action_id: string }
export interface UiNodeSnapshot {
    instance_id: string;
    component_id: string;
    component: string;
    props: Record<string, unknown>;
    slots: Record<string, string[]>;
    bindings: Record<string, string>;
    actions: Record<string, UiActionRef>;
    validation_errors: string[];
}
export interface UiSnapshot {
    identity: UiSurfaceIdentity;
    revision: number;
    catalog_id: string;
    catalog_digest: string;
    phase: 'pending' | 'ready' | 'read_only' | 'deleted';
    root_instance_id: string | null;
    nodes: Record<string, UiNodeSnapshot>;
    summary: string;
}
export type UiEdit =
    | { type: 'set'; instance_id: string; binding: string; value: unknown }
    | { type: 'delete'; instance_id: string; binding: string };
export interface UiInteraction {
    identity: UiSurfaceIdentity;
    expected_surface_revision: number;
    edits: UiEdit[];
    action: UiActionRef | null;
}
export function uiIdentityKey(identity: UiSurfaceIdentity): string {
    const owner = identity.owner;
    return JSON.stringify([owner.authority_id, owner.conversation_id, owner.participant_id,
        owner.context_id, owner.originating_run_id, owner.message_id, owner.part_id,
        identity.surface_id, identity.generation]);
}

export type UiSubmissionOutcome =
    | { status: 'committed' }
    | { status: 'rejected' | 'conflict'; message: string }
    | { status: 'unknown'; message: string };

export interface UiSurfaceRevisionRef { identity: UiSurfaceIdentity; revision: number }

/** A user action references the original Surface, not its new user-message ID.
 * The envelope belongs to Core/model context and is never a display fallback. */
export function validateUiActionPart(value: Record<string, unknown>, conversationId: string): void {
    if (value.kind !== 'ui_action') throw new Error('Invalid UI action kind');
    const reference = value.surface as UiSurfaceRevisionRef | undefined;
    uiSurfaceReference({kind: 'ui_surface', surface: reference}, conversationId,
        reference?.identity?.owner?.message_id ?? '');
    if (typeof value.summary !== 'string' || !value.summary.trim()
        || value.envelope === null || typeof value.envelope !== 'object' || Array.isArray(value.envelope)) {
        throw new Error('Invalid UI action projection');
    }
}

export async function readUiSurfaceRevision(reference: UiSurfaceRevisionRef, signal?: AbortSignal): Promise<UiSnapshot | null> {
    if (!Number.isSafeInteger(reference.revision) || reference.revision <= 0) throw new Error('Invalid UI revision');
    const result = await api<{ snapshot: UiSnapshot | null }>(
        `/api/chat/conversations/${encodeURIComponent(reference.identity.owner.conversation_id)}/ui-surface-revision`,
        { method: 'POST', body: JSON.stringify(reference), signal },
    );
    const snapshot = result.snapshot;
    if (snapshot !== null && (uiIdentityKey(snapshot.identity) !== uiIdentityKey(reference.identity)
        || snapshot.revision !== reference.revision || snapshot.phase === 'ready'
        || Object.values(snapshot.nodes).some(node => Object.keys(node.actions).length || Object.keys(node.bindings).length))) {
        throw new Error('Historical UI projection identity mismatch');
    }
    return snapshot;
}

/** Typed message anchor; part revision and surface revision are independent clocks. */
export function uiSurfaceReference(value: Record<string, unknown>, conversationId: string, messageId: string): UiSurfaceRevisionRef | null {
    if (value.kind === 'historical') {
        const projection = value.projection as Record<string, any> | undefined;
        if (projection?.schema !== 'message_part_view' || projection.canonical_kind !== 'ui_surface'
            || projection.binding !== 'canonical' || projection.status?.state !== 'supported'
            || projection.fields?.surface?.availability !== 'complete' || projection.fields?.summary?.availability !== 'complete') return null;
        if (projection.owner?.conversation_id !== conversationId || projection.owner?.message_id !== messageId
            || projection.owner?.part_id !== value.part_id) throw new Error('Historical UI owner mismatch');
        return uiSurfaceReference({kind: 'ui_surface', surface: projection.fields.surface.value}, conversationId, messageId);
    }
    if (value.kind !== 'ui_surface') return null;
    const reference = value.surface as UiSurfaceRevisionRef | undefined;
    const identity = reference?.identity;
    const owner = identity?.owner;
    if (!owner || !Number.isSafeInteger(reference?.revision) || reference!.revision <= 0
        || !Number.isSafeInteger(identity.generation) || identity.generation <= 0
        || typeof identity.surface_id !== 'string' || !identity.surface_id.trim()
        || ['authority_id', 'conversation_id', 'participant_id', 'context_id', 'originating_run_id', 'message_id', 'part_id']
            .some(key => typeof owner[key as keyof UiSurfaceOwner] !== 'string' || !owner[key as keyof UiSurfaceOwner].trim())
        || owner.conversation_id !== conversationId || owner.message_id !== messageId) {
        throw new Error('Invalid UI surface message reference');
    }
    return reference!;
}

export interface UiChangeSet {
    identity: UiSurfaceIdentity; base_revision: number; revision: number; catalog_digest: string;
    phase: UiSnapshot['phase']; root_instance_id: string | null;
    upserted_nodes: UiNodeSnapshot[]; removed_instance_ids: string[]; summary: string;
}
export type UiSurfaceUpdate = {type: 'snapshot'; snapshot: UiSnapshot} | {type: 'changes'; changes: UiChangeSet}
    | {type: 'unchanged'; identity: UiSurfaceIdentity; revision: number} | {type: 'missing'; identity: UiSurfaceIdentity};
/** Apply only Core-resolved nodes. No binding or protocol evaluation on the client. */
export function applyUiSurfaceUpdate(identity: UiSurfaceIdentity, baseline: UiSnapshot | undefined, update: UiSurfaceUpdate): UiSnapshot | null {
    const received = update.type === 'snapshot' ? update.snapshot.identity : update.type === 'changes' ? update.changes.identity : update.identity;
    if (uiIdentityKey(received) !== uiIdentityKey(identity) || baseline && uiIdentityKey(baseline.identity) !== uiIdentityKey(identity)) throw new Error('UI update owner mismatch');
    if (update.type === 'missing') return null;
    if (update.type === 'snapshot') {
        if (baseline && update.snapshot.revision < baseline.revision) throw new Error('UI snapshot regressed');
        return update.snapshot;
    }
    if (update.type === 'unchanged') {
        if (!baseline || baseline.revision !== update.revision) throw new Error('UI update baseline missing');
        return baseline;
    }
    const change = update.changes;
    if (!baseline || baseline.revision !== change.base_revision || baseline.catalog_digest !== change.catalog_digest
        || !Number.isSafeInteger(change.revision) || change.revision <= change.base_revision) throw new Error('UI delta baseline mismatch');
    const nodes = {...baseline.nodes};
    for (const id of change.removed_instance_ids) delete nodes[id];
    for (const node of change.upserted_nodes) nodes[node.instance_id] = node;
    if (change.root_instance_id !== null && !nodes[change.root_instance_id]) throw new Error('UI root is missing');
    return {...baseline, revision: change.revision, phase: change.phase, root_instance_id: change.root_instance_id, nodes, summary: change.summary};
}
export async function readUiSurfaceChanges(identity: UiSurfaceIdentity, baseline?: UiSnapshot, signal?: AbortSignal): Promise<UiSnapshot | null> {
    const {update} = await api<{update: UiSurfaceUpdate}>(
        `/api/chat/conversations/${encodeURIComponent(identity.owner.conversation_id)}/ui-surface-changes`,
        {method: 'POST', body: JSON.stringify({identity, base_revision: baseline?.revision ?? null}), signal});
    return applyUiSurfaceUpdate(identity, baseline, update);
}

export interface UiDraftPreviewInput {
    identity: UiSurfaceIdentity;
    expected_surface_revision: number;
    edits: UiEdit[];
    draft_generation: number;
}
export interface UiDraftPreview {
    identity: UiSurfaceIdentity;
    base_revision: number;
    draft_generation: number;
    snapshot: UiSnapshot;
}
export function validateUiDraftPreview(input: UiDraftPreviewInput, preview: UiDraftPreview): UiSnapshot {
    if (uiIdentityKey(preview.identity) !== uiIdentityKey(input.identity)
        || uiIdentityKey(preview.snapshot.identity) !== uiIdentityKey(input.identity)
        || preview.base_revision !== input.expected_surface_revision
        || preview.snapshot.revision !== input.expected_surface_revision
        || preview.draft_generation !== input.draft_generation || preview.snapshot.phase !== 'ready') {
        throw new Error('UI draft preview owner or generation mismatch');
    }
    return preview.snapshot;
}
export async function previewUiSurface(input: UiDraftPreviewInput, signal?: AbortSignal): Promise<UiDraftPreview> {
    const {preview} = await api<{preview: UiDraftPreview}>(
        `/api/chat/conversations/${encodeURIComponent(input.identity.owner.conversation_id)}/ui-surface-preview`,
        {method:'POST', body:JSON.stringify(input), signal});
    validateUiDraftPreview(input, preview);
    return preview;
}
