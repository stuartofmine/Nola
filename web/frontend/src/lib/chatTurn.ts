import { isVerifiedTerminal } from './assistantTerminal.ts';
import type { VerifiedTerminal } from './assistantTerminal.ts';
import type { RunResultValue } from './runResult.ts';

export type ChatTurnPhase = 'sending' | 'streaming' | 'awaiting_permission' | 'recovering' | 'committed' | 'cancel_requested';
export type ChatTurnOutcome = 'pending' | 'uncertain' | 'rejected' | RunResultValue['kind'];
export interface ChatTurnOwner { conversationId: string; generation: number; hostId: string; clientId: string }
export interface ChatTurnIdentity { submissionId: string; userMessageId: string; assistantMessageId: string }
export interface TurnPermission {
    protocol_version: 1; allow_conversation: boolean; nonce: string; request_id: string; host_id: string; client_id: string;
    session_id: string; tool_name: string; expires_at: number;
}
export interface ChatTurn {
    readonly owner: Readonly<ChatTurnOwner>;
    readonly identity: Readonly<ChatTurnIdentity>;
    readonly phase: ChatTurnPhase;
    readonly outcome: ChatTurnOutcome;
    readonly runId: string | null;
    readonly permissionClientId: string;
    readonly permission: Readonly<TurnPermission> | null;
    readonly permissionAcknowledged: boolean;
    readonly committedRevision: string | null;
}
export type ChatTurnAction = { owner: ChatTurnOwner } & (
    | { type: 'stream_opened' }
    /** Caller supplies the validated canonical run binding, never a provider session ID. */
    | { type: 'run_bound'; runId: string; permissionClientId?: string }
    | { type: 'permission_pending'; challenge: unknown; nowSeconds: number }
    /** Complete permission list from the authenticated observation of this Run. */
    | { type: 'permissions_observed'; challenges: unknown[]; nowSeconds: number }
    | { type: 'permission_acknowledged'; nonce: string }
    /** An acknowledgement is not resolution; only the owning canonical state can resume streaming. */
    | { type: 'permission_resolved'; nonce: string }
    | { type: 'cancel_requested' }
    | { type: 'submission_not_admitted'; outcome: 'rejected' | 'cancelled' }
    | { type: 'transport_interrupted'; reason: 'eof' | 'done' | 'error' | 'abort' | 'trace_terminal' }
    | { type: 'terminal_verified'; evidence: VerifiedTerminal }
    | { type: 'event'; payload: unknown }
);
export type ChatTurnResult = { state: ChatTurn; ignored?: 'owner' | 'invalid' | 'unsupported' | 'phase' | 'run' | 'permission' | 'committed' };

const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === 'object' && !Array.isArray(value);
const counter = (value: unknown): value is number => typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;
const validId = (value: unknown, limit = 256): value is string => typeof value === 'string' && value.trim() === value && value.length > 0
    && !/[\p{Cc}]/u.test(value) && new TextEncoder().encode(value).length <= limit;

/** Deterministic IDs: retry/recovery never generates another user input. Persist before sending. */
export function chatTurnIdentity(submissionId: string): ChatTurnIdentity {
    const identity = { submissionId, userMessageId: `${submissionId}:user`, assistantMessageId: `${submissionId}:assistant` };
    if (!Object.values(identity).every(value => validId(value))) throw new Error('chat.turn.invalid_identity');
    return identity;
}

export function createChatTurn(owner: ChatTurnOwner, identity: ChatTurnIdentity | string): ChatTurn {
    if (!validId(owner.conversationId, 512) || !validId(owner.hostId, 512) || !validId(owner.clientId, 512) || !counter(owner.generation)) throw new Error('chat.turn.invalid_owner');
    const ids = typeof identity === 'string' ? chatTurnIdentity(identity) : { submissionId: identity.submissionId, userMessageId: identity.userMessageId, assistantMessageId: identity.assistantMessageId };
    if (![ids.submissionId, ids.userMessageId, ids.assistantMessageId].every(value => validId(value)) || ids.userMessageId === ids.assistantMessageId) throw new Error('chat.turn.invalid_identity');
    return { owner: Object.freeze({ conversationId: owner.conversationId, generation: owner.generation, hostId: owner.hostId, clientId: owner.clientId }), identity: Object.freeze(ids), phase: 'sending', outcome: 'pending', runId: null, permissionClientId: owner.clientId, permission: null, permissionAcknowledged: false, committedRevision: null };
}

export function canReleaseChatTurn(state: ChatTurn): boolean { return state.phase === 'committed'; }

function sameOwner(a: ChatTurnOwner, b: ChatTurnOwner): boolean {
    return a.conversationId === b.conversationId && a.generation === b.generation && a.hostId === b.hostId && a.clientId === b.clientId;
}

export function reduceChatTurn(state: ChatTurn, action: ChatTurnAction): ChatTurnResult {
    if (!sameOwner(state.owner, action.owner)) return { state, ignored: 'owner' };
    if (state.phase === 'committed') return { state, ignored: 'committed' };
    switch (action.type) {
        case 'stream_opened':
            return state.phase === 'sending' || state.phase === 'recovering'
                ? { state: { ...state, phase: state.permission ? 'awaiting_permission' : 'streaming' } }
                : { state, ignored: 'phase' };
        case 'run_bound':
            if (!validId(action.runId, 512)) return { state, ignored: 'invalid' };
            if (state.runId && state.runId !== action.runId) return { state, ignored: 'run' };
            if (action.permissionClientId !== undefined && (!validId(action.permissionClientId, 512)
                || state.runId !== null && action.permissionClientId !== state.permissionClientId)) return { state, ignored: 'owner' };
            return { state: { ...state, runId: action.runId, permissionClientId: action.permissionClientId ?? state.permissionClientId } };
        case 'permissions_observed': {
            // Validate the entire snapshot before retiring a displayed challenge. A stream
            // event alone cannot replace it: parallel requests can arrive out of order.
            const candidates: TurnPermission[] = [];
            for (const challenge of action.challenges) {
                const checked = reduceChatTurn({ ...state, permission: null, permissionAcknowledged: false },
                    { type: 'permission_pending', owner: action.owner, challenge, nowSeconds: action.nowSeconds });
                if (checked.ignored) return { state, ignored: checked.ignored };
                const permission = checked.state.permission!;
                if (candidates.some(candidate => candidate.nonce === permission.nonce)) return { state, ignored: 'invalid' };
                candidates.push(permission);
            }
            const current = candidates.find(candidate => candidate.nonce === state.permission?.nonce);
            if (current) return reduceChatTurn(state, { type: 'permission_pending', owner: action.owner,
                challenge: current, nowSeconds: action.nowSeconds });
            const resolved = state.permission ? reduceChatTurn(state, { type: 'permission_resolved',
                owner: action.owner, nonce: state.permission.nonce }).state : state;
            return candidates.length ? reduceChatTurn(resolved, { type: 'permission_pending', owner: action.owner,
                challenge: candidates[0], nowSeconds: action.nowSeconds }) : { state: resolved };
        }
        case 'permission_pending': {
            const value = action.challenge;
            if (state.phase === 'cancel_requested') return { state, ignored: 'phase' };
            if (!object(value) || value.protocol_version !== 1 || typeof value.allow_conversation !== 'boolean' || !counter(action.nowSeconds) || !counter(value.expires_at) || value.expires_at <= action.nowSeconds
                || !['nonce', 'request_id', 'host_id', 'client_id', 'session_id', 'tool_name'].every(key => validId(value[key], 512))) return { state, ignored: 'invalid' };
            if (value.host_id !== state.owner.hostId || value.client_id !== state.permissionClientId || value.session_id !== state.owner.conversationId) return { state, ignored: 'owner' };
            if (state.permission && state.permission.nonce !== value.nonce) return { state, ignored: 'permission' };
            // Store only ownership fields, not arbitrary event objects or mutable UI payloads.
            const permission: TurnPermission = { protocol_version: 1, allow_conversation: value.allow_conversation, nonce: value.nonce as string, request_id: value.request_id as string,
                host_id: value.host_id as string, client_id: value.client_id as string, session_id: value.session_id as string,
                tool_name: value.tool_name as string, expires_at: value.expires_at };
            if (state.permission && JSON.stringify(state.permission) !== JSON.stringify(permission)) return { state, ignored: 'permission' };
            return { state: { ...state, phase: 'awaiting_permission', permission: Object.freeze(permission) } };
        }
        case 'permission_acknowledged':
            return state.permission?.nonce === action.nonce
                ? { state: { ...state, permissionAcknowledged: true } } : { state, ignored: 'permission' };
        case 'permission_resolved':
            if (state.permission?.nonce !== action.nonce) return { state, ignored: 'permission' };
            return { state: { ...state, permission: null, permissionAcknowledged: false,
                phase: state.phase === 'awaiting_permission' ? 'streaming' : state.phase } };
        case 'cancel_requested':
            return { state: { ...state, phase: 'cancel_requested', outcome: 'uncertain' } };
        case 'submission_not_admitted':
            return state.runId === null ? { state: { ...state, phase: 'committed', outcome: action.outcome, permission: null } }
                : { state, ignored: 'run' };
        case 'transport_interrupted':
            return { state: { ...state, phase: state.phase === 'cancel_requested' ? 'cancel_requested' : 'recovering', outcome: 'uncertain' } };
        case 'terminal_verified': {
            const evidence = action.evidence;
            if (!isVerifiedTerminal(evidence)) return { state, ignored: 'invalid' };
            if (!sameOwner(state.owner, evidence.owner) || evidence.submissionId !== state.identity.submissionId
                || evidence.seal.identity.assistant_message_id !== state.identity.assistantMessageId
                || state.runId !== null && evidence.seal.identity.run_id !== state.runId) return { state, ignored: 'owner' };
            return { state: { ...state, phase: 'committed', outcome: evidence.seal.result.value.kind,
                runId: evidence.seal.identity.run_id, committedRevision: evidence.seal.checkpoint_revision, permission: null, permissionAcknowledged: false } };
        }
        case 'event': {
            const event = action.payload;
            if (!object(event)) return { state, ignored: 'invalid' };
            // All stream frames are hints, including the former full snapshot.
            return { state, ignored: 'unsupported' };
        }
    }
}

/** Existing-run recovery only. No arbitrary object spread, input, credentials or model fields. */
export function buildChatResumeRequest(state: ChatTurn): { conversation_id: string; expected_run_id: string; expected_host_id: string; client_id: string } | null {
    if (state.phase !== 'recovering' || !state.runId) return null;
    return { conversation_id: state.owner.conversationId, expected_run_id: state.runId, expected_host_id: state.owner.hostId, client_id: state.owner.clientId };
}
