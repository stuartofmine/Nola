import { uiSurfaceReference, validateUiActionPart } from './generativeUi.ts';
/** UI-neutral adapter for verified journal parts and canonical history.
 * Sources: cli/src/agent_parts/emitter.rs, core/nola-domain/src/projection.rs,
 * core/nola-host-protocol/src/lib.rs and GET /api/conversations/:id/messages.
 * Provider deltas and trace terminal events are deliberately not transcript input.
 */
import { compareJournalParts, journalIdentityKey, parseJournalCursor } from './assistantJournal.ts';
import type { JournalTimelinePart, JournalUpdate } from './assistantJournal.ts';
import { isVerifiedTerminal } from './assistantTerminal.ts';
import type { VerifiedTerminal } from './assistantTerminal.ts';
import type { AdmittedMessage } from './messageAdmissions.ts';
import { displayTextAttachmentMessage } from './chatTextAttachments.ts';
import { parseToolExecutionSource } from './toolExecutionSource.ts';
import type { ToolExecutionSource } from './toolExecutionSource.ts';
export interface TimelineOwner { conversationId: string; generation: number }
export interface CanonicalPart { kind: string; part_id: string; revision: number; [field: string]: unknown }
export type TimelinePart = { source: 'canonical'; id: string; value: CanonicalPart } | { source: 'optimistic'; id: string; value: { kind: 'text'; markdown: string; [field: string]: unknown } } | JournalTimelinePart;
export interface CanonicalMessage {
    message_id: string; role: 'user' | 'assistant' | 'system'; state: 'pending' | 'running' | 'completed' | 'cancelled' | 'failed';
    revision: number; created_at_ms: number; completed_at_ms?: number | null; parts: CanonicalPart[];
}
export interface CanonicalProjection {
    contract_version: number; conversation_id: string; revision: number; messages: CanonicalMessage[];
    surface?: Record<string, unknown>; [field: string]: unknown;
}
export interface CanonicalMessageSnapshot { readonly sourceHostId:string; readonly conversationId:string; readonly revision:number }
export interface TimelineMessage {
    id: string; role: CanonicalMessage['role']; state: CanonicalMessage['state'];
    canonical: CanonicalMessage | null; canonicalSnapshot?: Readonly<CanonicalMessageSnapshot> | null; parts: TimelinePart[];
}
export interface TimelineWindow { projectionRevision: number; eventSequence: number; before: number | null; hasMore: boolean; totalCount: number }
export interface ChatTimeline {
    owner: TimelineOwner; version: number; messages: TimelineMessage[];
    /** Loaded canonical message cache, not an assertion that omitted history was deleted.
     * Projection metadata/surface belong to the latest snapshot; streaming overlays
     * live only in `messages` until their canonical commit arrives. */
    projection: CanonicalProjection | null; window: TimelineWindow | null;
    committedSubmissions: string[];
    /** Durable conversation frontier; optimistic rows and cached projection never advance it. */
    submissionParentMessageId: string | null;
    /** A fixed terminal seal's complete assistant frontier was materialized.
     * chatTurn independently verifies the same exact owner before unlocking. */
    durable: boolean;
    activeSubmissionId: string | null;
    journalCursors: Record<string, { sequence: string; digest: string; sourceHostId: string }>;
}
export interface TimelineRequest { owner: TimelineOwner; version: number; window: TimelineWindow | null }
export type TimelineAction =
    | { type: 'beginSubmission'; owner: TimelineOwner; submissionId: string; userMessageId?: string; userText?: string }
    | { type: 'terminal'; owner: TimelineOwner; evidence: VerifiedTerminal }
    | { type: 'terminal_witness'; request: TimelineRequest; sourceHostId:string; evidence:VerifiedTerminal; payload:unknown }
    | { type: 'admissions'; owner: TimelineOwner; messages: readonly AdmittedMessage[] }
    | { type: 'history'; request: TimelineRequest; mode: 'replace' | 'prepend'; payload: unknown }
    | { type: 'journal'; owner: TimelineOwner; sourceHostId: string; runId: string; update: JournalUpdate };
export type TimelineResult = { state: ChatTimeline; ignored?: 'owner' | 'submission' | 'stale_request' | 'snapshot_changed' | 'duplicate' | 'stale_event' | 'terminal_message' | 'unsupported' | 'invalid' };

const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === 'object' && !Array.isArray(value);
const integer = (value: unknown): value is number => typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;
const identity = (value: unknown): value is string => typeof value === 'string' && value.trim().length > 0;
export function sameTimelineOwner(a: TimelineOwner, b: TimelineOwner): boolean { return a.conversationId === b.conversationId && a.generation === b.generation; }
export function createChatTimeline(owner: TimelineOwner): ChatTimeline {
    if (!identity(owner.conversationId) || !integer(owner.generation)) throw new Error('Invalid timeline owner');
    return { owner: { ...owner }, version: 0, messages: [], projection: null, window: null, committedSubmissions: [], submissionParentMessageId: null, durable: false, activeSubmissionId: null, journalCursors: {} };
}
/** Display-only selection; historical content never gains live tool/permission authority. */
export function timelinePartText(part: TimelinePart, kind: 'text' | 'reasoning' = 'text'): string {
    const value = part.value;
    const string = (input: unknown) => typeof input === 'string' ? input : '';
    if (kind === 'text' && value.kind === 'ui_action') return string(value.summary);
    if (part.source === 'journal' && value.kind === 'historical' && value.canonical_kind === kind) return string(value.preview);
    if (value.kind === kind) {
        if (kind === 'text' && value.review != null && value.review !== 'approved') return '';
        return string(value[kind === 'text' ? 'markdown' : 'summary']);
    }
    if (part.source !== 'canonical' || value.kind !== 'historical' || !object(value.projection)) return '';
    const projection = value.projection;
    if (kind === 'text' && projection.canonical_kind === 'ui_action') {
        if (projection.schema !== 'message_part_view' || !object(projection.status)
            || !['supported', 'deferred'].includes(String(projection.status.state)) || !object(projection.fields)) return '';
        const summary = projection.fields.summary;
        return object(summary) && summary.availability === 'complete' ? string(summary.value) : '';
    }
    if (projection.canonical_kind !== kind || !object(projection.status)) return '';
    const status = projection.status.state;
    if (!['supported', 'deferred', 'unsupported', 'not_loaded'].includes(String(status))) return '';
    const field = projection.schema === 'agent_part' ? (kind === 'text' ? 'text' : 'detail') : projection.schema === 'message_part_view' ? (kind === 'text' ? 'markdown' : 'summary') : null;
    if ((status === 'supported' || status === 'deferred') && field && object(projection.fields)) {
        const semantic = projection.fields[field];
        if (object(semantic) && semantic.availability === 'complete' && typeof semantic.value === 'string') return semantic.value;
    }
    return object(projection.canonical_text) ? string(projection.canonical_text.text) : '';
}
export function timelineMessageText(message: TimelineMessage): string { return message.parts.map(part => timelinePartText(part, 'text')).join(''); }
export function timelineMessageReasoning(message: TimelineMessage): string { return message.parts.map(part => timelinePartText(part, 'reasoning')).join(''); }

/** Placeholder is live presentation, not a property of stale history state. */
export function timelineShowsWorkingPlaceholder(message: TimelineMessage, liveMessageId: unknown, hasRunStatus: boolean): boolean {
    if (hasRunStatus || message.role !== 'assistant' || message.id !== liveMessageId
        || !['running', 'pending'].includes(message.state)) return false;
    if (timelineMessageText(message).trim() || timelineMessageReasoning(message).trim()) return false;
    return !message.parts.some(part => ['tool', 'error', 'task', 'task_control', 'skill', 'ui_surface'].includes(part.value.kind)
        || part.value.state === 'failed');
}

/** Empty progress belongs to the live status surface, not a history disclosure. */
export function timelineReasoningParts(message: TimelineMessage): TimelinePart[] {
    return message.parts.filter(part => part.value.kind === 'reasoning'
        && (timelinePartText(part, 'reasoning').trim().length > 0 || part.value.state === 'failed'));
}

export type TimelineToolState = 'pending' | 'running' | 'waiting_for_permission' | 'waiting_for_input'
    | 'completed' | 'sealed' | 'cancelled' | 'failed' | 'unknown';
const toolStates = new Set<string>(['pending', 'running', 'waiting_for_permission', 'waiting_for_input', 'completed', 'sealed', 'cancelled', 'failed']);
export interface TimelineToolPresentation {
    executionSource: ToolExecutionSource;
    state: TimelineToolState;
    title: string; summary: string | null; outputPreview: string | null;
    hasStructuredResult: boolean; structuredResult?: unknown;
    hasInput: boolean; input?: unknown;
    origin: string; artifacts: unknown;
    permission: { domain: string; rationale: string; decision: string | null } | null;
    resources: { type: string; operation: string; uri: string }[];
}

/** Failures and user waits must not be buried in a collapsed tool group. */
export function timelineToolsNeedAttention(parts: TimelinePart[]): boolean {
    return parts.some(part => {
        const state = timelineToolPresentation(part)?.state;
        return state === 'failed' || state === 'waiting_for_permission' || state === 'waiting_for_input';
    });
}
/** Render selection does not merge or discard independent source fields. */
export function timelineToolPresentation(part: TimelinePart): TimelineToolPresentation | null {
    const value = part.value;
    if (value.kind !== 'tool') return null;
    const resultField = 'output';
    const hasStructuredResult = Object.hasOwn(value, resultField);
    return {
        executionSource: parseToolExecutionSource(value.execution_source),
        state: typeof value.state === 'string' && toolStates.has(value.state) ? value.state as TimelineToolState : 'unknown',
        title: typeof value.title === 'string' ? value.title : '',
        summary: typeof value.summary === 'string' ? value.summary : null,
        outputPreview: typeof value.output_preview === 'string' ? value.output_preview : null,
        hasInput: Object.hasOwn(value, 'input'),
        ...(Object.hasOwn(value, 'input') ? {input: value.input} : {}),
        origin: object(value.origin) ? [value.origin.provider_title, value.origin.namespace, value.capability_id]
            .filter((item): item is string => typeof item === 'string' && !!item).join(' · ')
            : typeof value.capability_id === 'string' ? value.capability_id : '',
        artifacts: value.artifacts,
        permission: object(value.permission) && typeof value.permission.permission_domain === 'string' && typeof value.permission.rationale === 'string'
            ? { domain: value.permission.permission_domain, rationale: value.permission.rationale, decision: typeof value.permission.decision === 'string' ? value.permission.decision : null } : null,
        resources: Array.isArray(value.affected_resources) ? value.affected_resources.filter((resource): resource is Record<string, unknown> => object(resource)
            && typeof resource.resource_type === 'string' && typeof resource.operation === 'string' && typeof resource.uri === 'string')
            .map(resource => ({ type: resource.resource_type as string, operation: resource.operation as string, uri: resource.uri as string })) : [],
        hasStructuredResult,
        ...(hasStructuredResult ? { structuredResult: value[resultField] } : {}),
    };
}

function validatePublicPart(part: Record<string, unknown>, messageId: string, conversationId: string): void {
    if (part.kind === 'ui_surface') uiSurfaceReference(part, conversationId, messageId);
    if (part.kind === 'ui_action') validateUiActionPart(part, conversationId);
    if (part.kind === 'error' && (typeof part.code !== 'string' || typeof part.display_key !== 'string'
        || typeof part.recoverable !== 'boolean' || Object.keys(part).some(key =>
            !['kind', 'part_id', 'revision', 'code', 'display_key', 'recoverable'].includes(key)))) {
        throw new Error('Invalid canonical Error Part');
    }
    if (part.kind === 'tool') {
        parseToolExecutionSource(part.execution_source);
        if (Object.hasOwn(part, 'raw_input') || Object.hasOwn(part, 'full_output_ref')) throw new Error('Private tool field in public projection');
        const reference = part.full_output;
        if (reference != null && (!object(reference) || Object.keys(reference).length !== 3
            || reference.message_id !== messageId || reference.part_id !== part.part_id
            || reference.tool_call_id !== part.tool_call_id)) throw new Error('Invalid public tool-output owner');
    }
    if (part.kind === 'citation') {
        if (!(part.location == null || object(part.location))) throw new Error('Invalid citation location');
        const location = object(part.location) ? part.location : {};
        for (const field of ['page', 'line_start', 'line_end', 'section']) {
            if ((part[field] ?? null) !== (location[field] ?? null)) throw new Error('Conflicting citation location');
        }
        if ((part.range_start == null) !== (part.range_end == null)) throw new Error('Incomplete citation range');
    }
}
function canonicalCache(projection: CanonicalProjection, messages: TimelineMessage[]): CanonicalProjection {
    return { ...projection, messages: messages.flatMap(message => message.canonical ? [message.canonical] : []) };
}
export function timelineRequest(state: ChatTimeline): TimelineRequest { return { owner: { ...state.owner }, version: state.version, window: state.window && { ...state.window } }; }
export function parseCanonicalProjection(value: unknown): CanonicalProjection {
    if (!object(value) || value.contract_version !== 1 || !identity(value.conversation_id) || !integer(value.revision) || !Array.isArray(value.messages)) throw new Error('Invalid canonical projection');
    const ids = new Set<string>();
    const partIds = new Set<string>();
    for (const message of value.messages) {
        if (!object(message) || !identity(message.message_id) || ids.has(message.message_id) || !['user', 'assistant', 'system'].includes(String(message.role)) || !['pending', 'running', 'completed', 'cancelled', 'failed'].includes(String(message.state)) || !integer(message.revision) || !Number.isSafeInteger(message.created_at_ms) || !Array.isArray(message.parts)) throw new Error('Invalid canonical message');
        ids.add(message.message_id);
        for (const part of message.parts) {
            if (!object(part) || !identity(part.part_id) || !identity(part.kind) || !integer(part.revision) || partIds.has(part.part_id)) throw new Error('Invalid canonical part');
            validatePublicPart(part, message.message_id, value.conversation_id);
            partIds.add(part.part_id);
        }
    }
    return structuredClone(value) as unknown as CanonicalProjection;
}
function canonicalMessage(message: CanonicalMessage, snapshot:Readonly<CanonicalMessageSnapshot>): TimelineMessage {
    return { id: message.message_id, role: message.role, state: message.state, canonical: message, canonicalSnapshot:snapshot, parts: message.parts.map(value => ({ source: 'canonical', id: value.part_id, value })) };
}
function parseWindow(value: unknown): TimelineWindow {
    if (!object(value) || !integer(value.projectionRevision) || !integer(value.eventSequence) || !integer(value.totalCount) || typeof value.hasMore !== 'boolean' || !(value.before === null || Number.isSafeInteger(value.before))) throw new Error('Invalid history window');
    return { projectionRevision: value.projectionRevision, eventSequence: value.eventSequence, totalCount: value.totalCount, before: value.before as number | null, hasMore: value.hasMore };
}
function sameSnapshot(a: TimelineWindow, b: TimelineWindow): boolean { return a.projectionRevision === b.projectionRevision && a.eventSequence === b.eventSequence; }
function admittedMessages(existing: TimelineMessage[], declarations: readonly AdmittedMessage[]): TimelineMessage[] {
    const ordered = [...declarations].sort((a, b) => a.ordinal - b.ordinal);
    const ids = new Set(ordered.map(message => message.input.id));
    const first = existing.findIndex(message => ids.has(message.id));
    const group = ordered.map(({ input }): TimelineMessage => {
        const old = existing.find(message => message.id === input.id);
        if (old && old.role !== input.role) throw new Error('Admission role mismatch');
        if (input.role === 'assistant') return old ?? { id: input.id, role: 'assistant', state: 'running', canonical: null, parts: [] };
        const textParts = input.initial_parts.filter(part => part.kind === 'text');
        const retained = old?.parts.length === textParts.length && old.parts.every((part,index) =>
            part.source === 'canonical' && part.id === textParts[index].id && part.value.kind === 'text'
            && part.value.markdown === (textParts[index].content_text ?? '')
            && (part.value.review == null || part.value.review === 'approved'));
        return { id: input.id, role: 'user', state: 'completed', canonical: old?.canonical ?? null, canonicalSnapshot:old?.canonicalSnapshot??null,
            parts: retained ? old!.parts : textParts.map(part => ({ source: 'canonical', id: part.id,
                value: { kind: 'text', part_id: part.id, revision: 0, markdown: part.content_text ?? '' } })) };
    });
    const remaining = existing.filter(message => !ids.has(message.id));
    remaining.splice(first < 0 ? remaining.length : first, 0, ...group);
    return remaining;
}

export function reduceChatTimeline(state: ChatTimeline, action: TimelineAction): TimelineResult {
    const owner = action.type === 'history' || action.type === 'terminal_witness' ? action.request.owner : action.owner;
    if (!sameTimelineOwner(state.owner, owner)) return { state, ignored: 'owner' };
    try {
        if (action.type === 'admissions') return { state: { ...state, version: state.version + 1, messages: admittedMessages(state.messages, action.messages) } };
        if (action.type === 'terminal') {
            const evidence = action.evidence;
            if (!isVerifiedTerminal(evidence) || !sameTimelineOwner(owner, evidence.owner)) return { state, ignored: 'owner' };
            if (state.activeSubmissionId !== null && evidence.submissionId !== state.activeSubmissionId) return { state, ignored: 'submission' };
            if (state.committedSubmissions.includes(evidence.submissionId)) return { state, ignored: 'duplicate' };
            for (const head of evidence.heads) {
                const cursor = state.journalCursors[journalIdentityKey(head.identity)];
                if (!cursor || cursor.sourceHostId !== evidence.owner.hostId || BigInt(cursor.sequence) < BigInt(head.sequence)
                    || cursor.sequence === head.sequence && cursor.digest !== head.record_digest) return { state, ignored: 'invalid' };
            }
            const messages = new Map(evidence.messages.map(message => [message.through.identity.assistant_message_id, message]));
            return { state: { ...state, version: state.version + 1, durable: true,
                committedSubmissions: [...state.committedSubmissions, evidence.submissionId],
                submissionParentMessageId: [...evidence.declarations].sort((a, b) => a.ordinal - b.ordinal).at(-1)?.input.id ?? state.submissionParentMessageId,
                messages: admittedMessages(state.messages, evidence.declarations).map(message => {
                    const terminal = messages.get(message.id);
                    return terminal ? { ...message, state: terminal.state, canonical: message.canonical ? {
                        ...message.canonical, state: terminal.state, completed_at_ms: terminal.completed_at_ms,
                    } : null } : message;
                }) } };
        }
        if (action.type === 'terminal_witness') {
            if(action.request.version!==state.version)return {state,ignored:'stale_request'};
            const evidence=action.evidence;
            if(!isVerifiedTerminal(evidence)||!sameTimelineOwner(owner,evidence.owner)
                ||action.sourceHostId!==evidence.owner.hostId)return {state,ignored:'owner'};
            if(!state.durable||!state.committedSubmissions.includes(evidence.submissionId)
                ||state.activeSubmissionId!==null&&state.activeSubmissionId!==evidence.submissionId)return {state,ignored:'submission'};
            for(const head of evidence.heads){
                const cursor=state.journalCursors[journalIdentityKey(head.identity)];
                if(!cursor||cursor.sourceHostId!==action.sourceHostId||BigInt(cursor.sequence)<BigInt(head.sequence)
                    ||cursor.sequence===head.sequence&&cursor.digest!==head.record_digest)return {state,ignored:'invalid'};
            }
            if(!object(action.payload)||action.payload.sourceHostId!==action.sourceHostId)return {state,ignored:'owner'};
            const projection=parseCanonicalProjection(action.payload.conversationProjection);
            const window=parseWindow(action.payload.messageWindow);
            if(projection.conversation_id!==owner.conversationId)return {state,ignored:'owner'};
            if(projection.revision!==window.projectionRevision)return {state,ignored:'invalid'};
            const knownRevision=state.messages.reduce((revision,message)=>
                message.canonicalSnapshot?.sourceHostId===action.sourceHostId&&message.canonicalSnapshot.conversationId===owner.conversationId
                    ?Math.max(revision,message.canonicalSnapshot.revision):revision,state.projection?.revision??0);
            if(projection.revision<knownRevision||state.window&&window.eventSequence<state.window.eventSequence)return {state,ignored:'stale_event'};
            const declared=new Map(evidence.declarations.filter(value=>value.input.role==='user').map(value=>[value.input.id,value.input]));
            const incoming=new Map(projection.messages.map(message=>[message.message_id,message]));
            const snapshot=Object.freeze({sourceHostId:action.sourceHostId,conversationId:owner.conversationId,revision:projection.revision});
            let changed=false;
            const messages=state.messages.map(message=>{
                const declaration=declared.get(message.id),canonical=incoming.get(message.id);
                if(!declaration||!canonical||message.canonical&&message.canonicalSnapshot)return message;
                if(message.role!=='user'||message.state!=='completed'||canonical.role!=='user'||canonical.state!=='completed'
                    ||canonical.revision<1||projection.revision<1||message.canonical&&canonical.revision<message.canonical.revision)
                    throw new Error('Terminal user witness identity/revision mismatch');
                const part=canonical.parts.length===1?canonical.parts[0]:null;
                const previousPart=part&&message.canonical?.parts.find(value=>value.part_id===part.part_id);
                if(part&&previousPart&&part.revision<previousPart.revision)throw new Error('Terminal user Part revision regressed');
                const admitted=declaration.initial_parts.length===1?declaration.initial_parts[0]:null;
                if(!part||part.kind!=='text'||part.is_final!==true||typeof part.markdown!=='string'
                    ||part.review!=null&&part.review!=='approved'||part.revision<1||displayTextAttachmentMessage(part.markdown)!==null
                    ||!admitted||admitted.kind!=='text'||admitted.id!==part.part_id||admitted.content_text!==part.markdown
                    ||message.parts.length!==1||message.parts[0].value.kind!=='text'||message.parts[0].id!==part.part_id
                    ||timelineMessageText(message)!==part.markdown)throw new Error('Terminal user witness body mismatch');
                changed=true;
                return {...message,canonical,canonicalSnapshot:snapshot};
            });
            // This is not a history replacement. Keep the exact loaded range,
            // pagination snapshot, visible Parts, journal and terminal authority.
            return changed?{state:{...state,version:state.version+1,messages}}:{state};
        }
        if (action.type === 'journal') {
            const update = action.update;
            const cursor = parseJournalCursor(update.through);
            if (action.sourceHostId !== update.sourceHostId || cursor.identity.conversation_id !== owner.conversationId || cursor.identity.run_id !== action.runId) return { state, ignored: 'owner' };
            const key = journalIdentityKey(cursor.identity);
            const previous = state.journalCursors[key];
            if (previous && previous.sourceHostId !== update.sourceHostId) return { state, ignored: 'owner' };
            if (previous && BigInt(cursor.sequence) <= BigInt(previous.sequence)) return { state, ignored: cursor.sequence === previous.sequence && cursor.record_digest !== previous.digest ? 'invalid' : 'duplicate' };
            const messageId = cursor.identity.assistant_message_id;
            const index = state.messages.findIndex(message => message.id === messageId);
            const old = state.messages[index];
            if (old && old.role !== 'assistant') return { state, ignored: 'invalid' };
            // Journal materialization may arrive after a terminal history read.
            // It updates content only; it cannot reopen or finish that message.
            let parts = old ? old.parts.filter(part => part.source === 'journal') : [];
            if (update.initial) parts = parts.filter(part => part.source !== 'journal' || part.journalOwner !== key);
            for (const change of update.changes) {
                if (change.part && (change.part.journalOwner !== key || change.part.id !== change.partId)) return { state, ignored: 'invalid' };
                if (state.messages.some(message => message.id !== messageId && message.parts.some(part => part.id === change.partId))) return { state, ignored: 'invalid' };
                const at = parts.findIndex(part => part.id === change.partId);
                if (at >= 0 && parts[at].source === 'journal' && parts[at].journalOwner !== key) return { state, ignored: 'owner' };
                if (change.part === null) { if (at >= 0) parts.splice(at, 1); }
                else if (at >= 0) parts[at] = change.part;
                else parts.push(change.part);
            }
            // Preserve earlier trusted Run groups during an internal handoff;
            // each owner has its own independently allocated order clock.
            const groups = [...new Set(parts.map(part => part.source === 'journal' ? part.journalOwner : ''))];
            parts.sort((a, b) => a.source === 'journal' && b.source === 'journal'
                ? groups.indexOf(a.journalOwner) - groups.indexOf(b.journalOwner) || compareJournalParts(a, b) : 0);
            const message: TimelineMessage = { id: messageId, role: 'assistant', state: old?.state ?? 'running', canonical: old?.canonical ?? null, canonicalSnapshot:old?.canonicalSnapshot??null, parts };
            const messages = [...state.messages];
            if (index < 0) messages.push(message); else messages[index] = message;
            return { state: { ...state, messages, version: state.version + 1, journalCursors: { ...state.journalCursors,
                [key]: { sequence: cursor.sequence, digest: cursor.record_digest, sourceHostId: update.sourceHostId } } } };
        }
        if (action.type === 'beginSubmission') {
            if (!identity(action.submissionId) || state.committedSubmissions.includes(action.submissionId)) return { state, ignored: 'invalid' };
            if (state.activeSubmissionId === action.submissionId) return { state, ignored: 'duplicate' };
            // Cursor identity includes the local HTTP submission. A subsequent
            // Host invocation may restart its Agent journal cursor; late callbacks
            // from the prior invocation must not poison or unlock the new one.
            const messages = [...state.messages];
            if (action.userText !== undefined) {
                if (!identity(action.userMessageId) || typeof action.userText !== 'string' || messages.some(message => message.id === action.userMessageId)) return { state, ignored: 'invalid' };
                // Optimistic display of the user's exact input, never persistence evidence.
                const partId = `${action.userMessageId}:local-text`;
                messages.push({ id: action.userMessageId, role: 'user', state: 'pending', canonical: null, parts: [{ source: 'optimistic', id: partId,
                    value: { kind: 'text', markdown: action.userText } }] });
            }
            return { state: { ...state, messages, version: state.version + 1, activeSubmissionId: action.submissionId, durable: false } };
        }
        if (action.type === 'history') {
            if (action.request.version !== state.version) return { state, ignored: 'stale_request' };
            if (!object(action.payload)) return { state, ignored: 'invalid' };
            const projection = parseCanonicalProjection(action.payload.conversationProjection);
            const window = parseWindow(action.payload.messageWindow);
            if (projection.conversation_id !== owner.conversationId) return { state, ignored: 'owner' };
            if (projection.revision !== window.projectionRevision) return { state, ignored: 'invalid' };
            if (state.projection && projection.revision < state.projection.revision) return { state, ignored: 'stale_event' };
            if (action.mode === 'prepend' && (!state.window || !action.request.window || !sameSnapshot(state.window, window) || !sameSnapshot(action.request.window, window))) return { state, ignored: 'snapshot_changed' };
            if (!identity(action.payload.sourceHostId)) return {state,ignored:'invalid'};
            const snapshot=Object.freeze({sourceHostId:action.payload.sourceHostId,conversationId:projection.conversation_id,revision:projection.revision});
            const incoming = projection.messages.map(message=>canonicalMessage(message,snapshot));
            const existingIds = new Set(state.messages.map(message => message.id));
            const messages = action.mode === 'prepend' ? [...incoming.filter(message => !existingIds.has(message.id)), ...state.messages] : incoming;
            return { state: { ...state, version: state.version + 1, messages, projection: canonicalCache(action.mode === 'prepend' ? state.projection! : projection, messages), window,
                submissionParentMessageId: action.mode === 'replace' ? projection.messages.at(-1)?.message_id ?? null : state.submissionParentMessageId,
                journalCursors: action.mode === 'replace' ? {} : state.journalCursors } };
        }
        return { state, ignored: 'unsupported' };
    } catch { return { state, ignored: 'invalid' }; }
}
