import { refreshStreamPermissions } from './streamPermissionRefresh.ts';
import { queuedUiDispatch } from './queuedUiDispatch.ts';
import type { QueuedUiHead } from './queuedUiDispatch.ts';
import { get } from 'svelte/store';
import { api as requestAPI, ApiError } from './api.ts';
import { locale, t } from './i18n.ts';
import { browserNotifications } from './browserNotifications.ts';
import { createChatTimeline, reduceChatTimeline, timelineRequest } from './chatTimeline.ts';
import type { ChatTimeline, TimelineOwner } from './chatTimeline.ts';
import { AssistantJournalReader, parseJournalCursor } from './assistantJournal.ts';
import type { JournalUpdate } from './assistantJournal.ts';
import { readTerminal } from './assistantTerminal.ts';
import type { VerifiedTerminal } from './assistantTerminal.ts';
import { RunObserver, parseRunObservation, parseGoalControl } from './runObservation.ts';
import type { EditedResendEnvelope } from './editedResend.ts';
import type { GoalExecutionOwner, GoalControlObservation } from './runObservation.ts';
import { dispatchGoalControlIntent, GoalControlIntentStore } from './goalControlIntent.ts';
import { RunContextUsageProjection, readHistoricalRunContextUsage, readCurrentRunContextUsage } from './runContextUsage.ts';
import type { CommittedRunContextUsage } from './runContextUsage.ts';
import { createChatTurn, reduceChatTurn, chatTurnIdentity, canReleaseChatTurn, buildChatResumeRequest } from './chatTurn.ts';
import type { ChatTurn, ChatTurnOwner } from './chatTurn.ts';
import { SourceSubmissionStorage, createSourceSubmission, sourceIdentity, acceptSourceReceipt, sourceSubmissionQuery,
    exactSubmissionCancel, settledSourceSubmission, parseObservedSourceReceipt, admittedReceiptCancel, parseSourceReceipt } from './sourceSubmission.ts';
import type { PendingSourceSubmission, SourceSubmissionReceipt } from './sourceSubmission.ts';
import { dispatchSourceSubmission, SourceSubmissionNotDispatchedError, journalEventQueue } from './sourceSubmissionDispatch.ts';
import { freezeHostToolPlan } from './hostToolPlan.ts';
import { chatEventStream as transportStream } from './chatTransport.ts';
import { attachmentModelContent } from './chatTextAttachments.ts';
import type { ChatTextAttachment } from './chatTextAttachments.ts';
import { knowledgePending } from './chatKnowledge.ts';
import { parseChatPermission, permissionIdentity } from './chatPermission.ts';
import { messageWindowUrl } from './chatConversations.ts';
import type { ChatConversation } from './chatConversations.ts';
import type { WorkspaceDrafts } from './workspaceDrafts.ts';
import { BrowserRecoveryIntentStore, type RecoveryOwner } from './unknownEffectRecovery.ts';

/** Route recovery only from the same authoritative Goal observation. */
export function observedGoalResumeRoute(owner: GoalExecutionOwner | null, control: GoalControlObservation | null): 'run' | 'goal' | 'refresh' {
    if (!owner) return 'run';
    if (!control || control.goal_id !== owner.goal_id || control.control_token !== owner.control_token) return 'refresh';
    if (control.status === 'running') return 'run';
    if (control.can_resume) return 'goal';
    return 'refresh';
}

export interface SubmissionInput { text: string; attachments: ChatTextAttachment[]; language: string; submissionId?: string }
export interface ConversationSubmissionSnapshot {
    timeline: ChatTimeline | null; turn: ChatTurn | null; challenge: unknown; reviewText: string;
    pendingSubmission: PendingSourceSubmission | null; observedCanSubmit: boolean; observedRecovery: string;
    contextUsage: CommittedRunContextUsage | null; historyLoading: boolean; error: string; notice: string;
    pendingText: string; pendingAttachmentNames: string[];
    queuedDispatch: SourceSubmissionReceipt | null; queuedCancelling: boolean;
    historicalRecoveryOwners: RecoveryOwner[];
    draft: string; attachments: ChatTextAttachment[];
}
export interface ConversationPresentation {
    cacheChanged?(): void;
    captureHistory?(reason?: 'prepend' | 'metadata'): unknown;
    restoreHistory?(anchor: unknown, isCurrent?: () => boolean, reason?: 'metadata'): Promise<void>;
}
export interface ConversationSubmissionController {
    readonly owner: ChatTurnOwner;
    attach(listener: (snapshot: ConversationSubmissionSnapshot) => void, presentation?: ConversationPresentation): void;
    detach(): void; dispose(): void;
    initialize(): Promise<void>; loadHistory(mode: 'replace' | 'prepend'): Promise<void>;
    submitPreparedEditedResend(envelope: EditedResendEnvelope, language: string): Promise<SourceSubmissionReceipt>;
    submit(input: SubmissionInput): Promise<void>; submitQueued(commandId: string): Promise<void>; resume(): Promise<void>; stop(): Promise<void>;
    cancelQueued(): Promise<void>; checkQueued(): Promise<void>;
    observe(): void; setForeground(value: boolean): void;
    updateConversation(value: ChatConversation): void;
    acceptReceipt(value: unknown): Promise<void>;
    permissionAccepted(identity: string): void;
}
let nextGeneration = 0;

/** Application-owned owner-scoped protocol and projection subscription.
 * Mounting selects a listener; it never creates, cancels or redispatches a Run. */
export function createConversationSubmissionController(options: {
    conversation: ChatConversation; hostId: string; clientId: string;
    workspace: ReturnType<WorkspaceDrafts['forHost']>;
    storage?: SourceSubmissionStorage;
    goalStorage?: GoalControlIntentStore;
    request?: typeof requestAPI;
    stream?: typeof transportStream;
    recoveryOwners?: () => Promise<RecoveryOwner[]>;
}): ConversationSubmissionController {
    let active = options.conversation;
    const hostId = options.hostId, clientId = options.clientId, generation = ++nextGeneration;
    const identity: ChatTurnOwner = {hostId, clientId, conversationId: active.conversationId, generation};
    const workspaceHost = options.workspace;
    const api = options.request ?? requestAPI;
    const chatEventStream = options.stream ?? transportStream;
    let disposed = false, foreground = true, initialized = false, initializationPrepared = false;
    let initialLoad: Promise<void> | null = null;
    let listener: ((snapshot: ConversationSubmissionSnapshot) => void) | null = null;
    let presentation: ConversationPresentation | null = null;
    let journalReader: AssistantJournalReader | null = null, observer: RunObserver | null = null;
    let permissionRefresh: RunObserver | null = null;
    let streamController: AbortController | null = null, historyController: AbortController | null = null;
    let witnessController: AbortController | null = null, witnessAttempt = '', historyEpoch = 0;
    let terminalWitness: VerifiedTerminal | null = null;
    function stopWitnessRead() { witnessController?.abort(); witnessController = null; witnessAttempt = ''; }
    const dispatchControllers = new Set<AbortController>();
    const editedRuns = new Set<string>();
    const editedAdmissions = new Map<string, {resolve:(receipt:SourceSubmissionReceipt)=>void; reject:(cause:unknown)=>void}>();
    const isEditedPending = (value: PendingSourceSubmission | null) => !!value && value.request.edited_resend_operation_id === value.identity.submission_id;
    const isEditedTurn = () => !!state.turn && (editedRuns.has(state.turn.identity.submissionId) || isEditedPending(state.pendingSubmission));
    function editedReceipt(value: PendingSourceSubmission) {
        if (!isEditedPending(value) || !value.receipt) return;
        const waiting = editedAdmissions.get(value.identity.submission_id);
        if (!waiting) return;
        if (value.receipt.phase === 'admitted') { editedAdmissions.delete(value.identity.submission_id); waiting.resolve(value.receipt); }
        else if (['rejected','cancelled'].includes(value.receipt.phase)) {
            editedAdmissions.delete(value.identity.submission_id); waiting.reject(new Error(value.receipt.rejection_code ?? 'Edited resend was not admitted'));
        }
    }
    function failEdited(run:string, cause:unknown) { const pending=editedAdmissions.get(run); editedAdmissions.delete(run); pending?.reject(cause); }
    let observedReceipt: SourceSubmissionReceipt | null = null, usageReading = false;
    let observedGoal: GoalExecutionOwner | null = null;
    let observedTaskReadOnly = false;
    let observedGoalControl: GoalControlObservation | null = null;
    const usageProjection = new RunContextUsageProjection();
    // Recover directly from Core; no browser-local duplicate of the durable FIFO.
    let queuedWakeController: AbortController | null = null;
    let queuedWakeTimer: ReturnType<typeof setTimeout> | null = null;
    function stopQueuedWake() {
        if (queuedWakeTimer !== null) clearTimeout(queuedWakeTimer);
        queuedWakeTimer = null;
        queuedWakeController?.abort();
        queuedWakeController = null;
    }
    function retryQueuedWake(delay: number) {
        if (disposed || !foreground || !presentation || queuedWakeTimer !== null) return;
        queuedWakeTimer = setTimeout(() => {
            queuedWakeTimer = null;
            void wakeQueuedAction();
        }, Math.max(250, Math.min(delay, 30_000)));
    }
    async function readQueuedDispatch(signal?: AbortSignal) {
        const query = new URLSearchParams({expected_host_id: hostId, client_id: clientId});
        return api<{head: QueuedUiHead | null; dispatch: SourceSubmissionReceipt | null}>(
            `/api/chat/conversations/${encodeURIComponent(identity.conversationId)}/queued-input?${query}`, {signal});
    }
    async function retainQueuedDispatch(receipt: SourceSubmissionReceipt) {
        state.queuedDispatch = receipt;
        if (receipt.phase === 'rejected' || receipt.phase === 'cancelled') {
            if (state.pendingSubmission?.identity.submission_id === receipt.identity.submission_id) {
                await workspaceHost.abandonSubmission(receipt.identity.submission_id);
                submissionStorage().clear(state.pendingSubmission);
                state.pendingSubmission = null;
            }
            if (state.turn?.identity.submissionId === receipt.identity.submission_id) {
                state.turn = reduceChatTurn(state.turn, {type: 'submission_not_admitted', owner: identity, outcome: receipt.phase}).state;
                state.pendingText = ''; state.pendingAttachmentNames = []; state.challenge = null;
                state.error = ''; state.notice = '';
            }
        }
    }
    async function checkQueued() {
        const result = await readQueuedDispatch();
        if (disposed) return;
        if (!result.head || result.head.user_message_id !== `ui:${result.head.command_id}:user`) {
            state.queuedDispatch = null; return;
        }
        const receipt = queuedUiDispatch(result.dispatch, result.head, identity);
        if (receipt) await retainQueuedDispatch(receipt);
        else state.queuedDispatch = null;
    }
    async function cancelQueued() {
        const receipt = state.queuedDispatch;
        if (!receipt || state.queuedCancelling || !['rejected', 'cancelled'].includes(receipt.phase)) return;
        state.queuedCancelling = true;
        try {
            const result = await api<{cancelled: boolean; command_id: string}>(
                `/api/chat/conversations/${encodeURIComponent(identity.conversationId)}/queued-input/cancel`,
                {method: 'POST', body: JSON.stringify({expected_host_id:hostId, client_id:clientId,
                    command_id:receipt.identity.submission_id, request_digest:receipt.request_digest})});
            if (!result.cancelled || result.command_id !== receipt.identity.submission_id) throw new Error('Queued cancellation mismatch');
            if (disposed) return;
            state.queuedDispatch = null;
            await loadHistory('replace');
        } catch { if (!disposed) state.error = 'chatPage.uncertain'; }
        finally { if (!disposed) state.queuedCancelling = false; }
    }
    async function wakeQueuedAction() {
        if (disposed || !initialized || !foreground || !presentation || queuedWakeController || state.queuedDispatch ||
            !state.observedCanSubmit || state.pendingSubmission || streamController ||
            state.turn && !canReleaseChatTurn(state.turn) ||
            knowledgePending(workspaceHost.knowledge.get(identity.conversationId))) return;
        const controller = new AbortController();
        queuedWakeController = controller;
        try {
            const result = await readQueuedDispatch(controller.signal);
            if (disposed || !foreground || !presentation || controller.signal.aborted || queuedWakeController !== controller ||
                !state.observedCanSubmit || state.pendingSubmission || streamController ||
                state.turn && !canReleaseChatTurn(state.turn)) return;
            const head = result.head;
            if (!head || head.kind !== 'user_input' || head.user_message_id !== `ui:${head.command_id}:user`) return;
            const dispatch = queuedUiDispatch(result.dispatch, head, identity);
            if (dispatch) { await retainQueuedDispatch(dispatch); return; }
            if (head.lease_until_ms !== null) {
                if (!Number.isSafeInteger(head.lease_until_ms)) throw new Error('Invalid queue lease deadline');
                if (head.lease_until_ms > Date.now()) {
                    retryQueuedWake(head.lease_until_ms - Date.now() + 25);
                    return;
                }
            }
            if (knowledgePending(workspaceHost.knowledge.get(identity.conversationId))) {
                retryQueuedWake(500);
                return;
            }
            await submit({text: '表单操作', attachments: [], language: get(locale)}, head.command_id);
        } catch {
            if (!disposed && !controller.signal.aborted && queuedWakeController === controller) {
                state.error = 'chatPage.uncertain';
                retryQueuedWake(5_000);
            }
        } finally { if (queuedWakeController === controller) queuedWakeController = null; }
    }
    let publishPending = false;
    function publish() {
        if (publishPending || disposed) return;
        publishPending = true;
        queueMicrotask(() => {
            publishPending = false;
            if (!disposed) listener?.({...state, observedCanSubmit: initialized && state.observedCanSubmit, draft: workspaceHost.text.read(identity.conversationId),
                attachments: workspaceHost.attachments.read(identity.conversationId)});
        });
    }
    const state = new Proxy<Omit<ConversationSubmissionSnapshot, 'draft' | 'attachments'>>({
        timeline: createChatTimeline(identity), turn: null, challenge: null, reviewText: '', pendingSubmission: null,
        observedCanSubmit: false, observedRecovery: 'observe', contextUsage: null, historyLoading: false,
        error: '', notice: '', pendingText: '', pendingAttachmentNames: [], historicalRecoveryOwners: [], queuedDispatch: null, queuedCancelling: false,
    }, {set(target, property, value) { Reflect.set(target, property, value); publish(); return true; }});
    function owner(): TimelineOwner { return identity; }
    function turnOwner(): ChatTurnOwner { return identity; }
    function sameActive(value: TimelineOwner): boolean {
        return !disposed && value.conversationId === identity.conversationId && value.generation === generation;
    }
    async function refreshRunUsage(identity: ChatTurnOwner) {
        const receipt = state.pendingSubmission?.receipt?.phase === 'admitted' ? state.pendingSubmission.receipt : observedReceipt;
        if (!receipt?.admission || !sameActive(identity) || !state.turn || state.turn.runId !== receipt.admission.identity.run_id || usageReading) return;
        const submission = state.turn.identity.submissionId;
        const current = () => sameActive(identity) && state.turn?.identity.submissionId === submission && state.turn.runId === receipt.admission!.identity.run_id;
        usageReading = true;
        try {
            const value = await readCurrentRunContextUsage(identity, receipt.admission.identity, usageProjection,
                body => api('/api/chat/agent/v2/journal', { method: 'POST', body: JSON.stringify(body) }), current);
            if (current()) state.contextUsage = value;
        } catch { if (current()) state.contextUsage = null; }
        finally { usageReading = false; }
    }
    const submissionStorage = () => options.storage ?? new SourceSubmissionStorage(localStorage);
    let submittedDraftUnavailable = false;
    async function acceptSubmission(value: unknown) {
        if (!state.pendingSubmission || !state.turn) throw new Error('Missing original submission');
        state.pendingSubmission = acceptSourceReceipt(state.pendingSubmission, value);
        submissionStorage().save(state.pendingSubmission);
        const receipt = state.pendingSubmission.receipt!;
        editedReceipt(state.pendingSubmission);
        try { if (!isEditedPending(state.pendingSubmission)) await workspaceHost.updateSubmission(state.pendingSubmission); }
        catch (cause) {
            if (receipt.phase !== 'admitted') throw cause;
            // The immutable source receipt still authorizes observation. Missing local
            // draft bookkeeping must neither invent a ticket nor strand the live Run.
            submittedDraftUnavailable = true;
            state.notice = 'chatPage.draftStorageFailed';
        }
        if (receipt.phase === 'admitted') {
            await consumeSubmittedDraft(state.turn.owner);
            state.turn = reduceChatTurn(state.turn, { type: 'run_bound', owner: state.turn.owner, runId: receipt.admission!.identity.run_id }).state;
            void workspaceHost?.completions.watch(state.turn);
        }
        else if (settledSourceSubmission(state.pendingSubmission)) {
            await workspaceHost.abandonSubmission(state.pendingSubmission.identity.submission_id);
            state.turn = reduceChatTurn(state.turn, { type: 'submission_not_admitted', owner: state.turn.owner, outcome: receipt.phase as 'rejected' | 'cancelled' }).state;
            state.pendingText = ''; state.pendingAttachmentNames = []; state.challenge = null;
            submissionStorage().clear(state.pendingSubmission); state.pendingSubmission = null;
            replyNotice(state.turn, receipt.phase === 'rejected' ? 'notSent' : null);
            state.error = '';
        }
    }
    async function reconcileSubmission(signal: AbortSignal): Promise<boolean> {
        if (state.pendingSubmission) {
            const id = state.pendingSubmission.identity;
            const latest = submissionStorage().read(id.source_host_id, id.client_id, id.conversation_id);
            // The workspace dispatcher can finish preparation after this page was remounted.
            // Only its exact local no-dispatch proof may release the restored unsent turn.
            if (!latest && state.turn?.identity.submissionId === id.submission_id && !state.pendingSubmission.sent
                && workspaceHost?.replyNotices.get(id.conversation_id)?.some(item => item.submissionId === id.submission_id && item.kind === 'notSent')) {
                await releaseUnsent(state.turn);
                return true;
            }
            if (latest?.identity.submission_id === id.submission_id) {
                state.pendingSubmission = latest;
                if (latest.receipt) await acceptSubmission(latest.receipt);
            }
        }
        const captured = state.pendingSubmission;
        if (!captured || captured.receipt?.phase === 'admitted') return true;
        state.observedCanSubmit = false;
        if (captured.request_digest === null) return false;
        const response = await api<{ source_host_id: string; result: unknown }>('/api/chat/agent/v2/submission', {
            method: 'POST', signal, body: JSON.stringify(sourceSubmissionQuery(captured)) });
        if (signal.aborted || state.pendingSubmission?.identity.submission_id !== captured.identity.submission_id) return false;
        if (response.source_host_id !== captured.identity.source_host_id) throw new Error('Source submission host changed');
        if (response.result === null) return false;
        await acceptSubmission(response.result);
        return !state.pendingSubmission || state.pendingSubmission.receipt?.phase === 'admitted';
    }
    function stopObservation() {
        if (observer) {
            observer.stop(); observer = null;
            // The observer also owns its in-flight journal reads.
            journalReader?.dispose(); journalReader = null;
        }
    }
    function observeSelected() {
        stopObservation();
        if (disposed || !presentation || !foreground || streamController) return;
        const identity = turnOwner();
        if (!identity) return;
        state.observedCanSubmit = false;
        const runObserver = new RunObserver(); observer = runObserver;
        const current = () => observer === runObserver && sameActive(identity);
        void runObserver.run(identity,
            (body, signal) => api('/api/chat/agent/v2/observe', { method: 'POST', body: JSON.stringify(body), signal }),
            async (value, goalControl) => {
                if (!current() || !state.timeline) return false;
                if (state.error === 'chatPage.controlUnconfirmed') state.error = '';
                observedGoalControl = goalControl;
                if (!value) { observedGoal = null; state.observedRecovery = 'observe'; state.observedCanSubmit = !observedTaskReadOnly && (goalControl?.can_submit ?? true); void wakeQueuedAction(); return current(); }
                observedTaskReadOnly = !!value.task_execution;
                observedGoal = value.goal_execution;
                state.observedCanSubmit = value.can_submit && (goalControl?.can_submit ?? true); state.observedRecovery = value.recovery;
                if (!value.admission) return true;
                if (!state.pendingSubmission) observedReceipt = value.goal_execution || value.task_execution ? null : parseObservedSourceReceipt(value.submission_receipt, identity, value.run_id, value.assistant_message_id);
                const declarations = await journals().admit(value.admission.identity, value.admission);
                if (!current() || !state.timeline) return false;
                state.timeline = reduceChatTimeline(state.timeline, { type: 'admissions', owner: identity, messages: declarations }).state;
                if (!state.turn || state.turn.runId !== value.run_id) {
                    stopWitnessRead(); terminalWitness = null;
                    const user = declarations.find(message => message.input.role === 'user');
                    if (!user) throw new Error('Observed Run lacks admitted user');
                    state.turn = createChatTurn(identity, { submissionId: value.run_id, userMessageId: user.input.id, assistantMessageId: value.assistant_message_id });
                    state.turn = reduceChatTurn(state.turn, { type: 'run_bound', owner: identity, runId: value.run_id, permissionClientId: value.goal_execution ? 'web-goal' : identity.clientId }).state;
                }
                if (!value.terminal_seal && !observedTaskReadOnly) void workspaceHost?.completions.watch(state.turn);
                // Current permission authority is independent of historical body hydration.
                const permissions = reduceChatTurn(state.turn, { type: 'permissions_observed', owner: identity,
                    challenges: value.permissions, nowSeconds: Math.floor(Date.now() / 1000) });
                if (!permissions.ignored) {
                    state.turn = permissions.state;
                    const pending = value.permissions.find(permission => permission.nonce === state.turn?.permission?.nonce);
                    state.challenge = pending ?? null;
                    state.reviewText = pending && typeof pending.safe_review_text === 'string' ? pending.safe_review_text : '';
                }
                // Recovery controls must remain usable while journal bodies hydrate.
                if (value.recovery === 'resume_required' || value.recovery === 'inconsistent') {
                    state.turn = reduceChatTurn(state.turn, { type: 'transport_interrupted', owner: identity, reason: 'eof' }).state;
                }
                try {
                    usageProjection.bind(identity, value.admission);
                    const usage = await usageProjection.accept(value.context_usage, identity, value.admission, current);
                    if (current()) state.contextUsage = usage;
                } catch { if (current()) state.contextUsage = null; }
                let after: string | null = null;
                for (let page = 0; page < 64; page++) {
                    const response = await api<{ source_host_id: string; result: unknown[] }>('/api/chat/agent/v2/journal', {
                        method: 'POST', signal: runObserver.signal, body: JSON.stringify({ operation: 'heads', conversation_id: identity.conversationId,
                            run_id: value.run_id, after_message_id: after, limit: 32 }) });
                    if (!current()) return false;
                    if (response.source_host_id !== identity.hostId || !Array.isArray(response.result) || response.result.length > 32) throw new Error('Invalid observed heads');
                    for (const raw of response.result) {
                        const cursor = parseJournalCursor(raw);
                        if (cursor.identity.run_id !== value.run_id || cursor.identity.conversation_id !== identity.conversationId) throw new Error('Observed head owner changed');
                        const update = await journals().read(cursor, value.run_id);
                        if (!current()) return false;
                        applyJournal(update, identity);
                        after = cursor.identity.assistant_message_id;
                    }
                    if (response.result.length < 32) break;
                    if (page === 63) throw new Error('Observed head budget exceeded');
                }
                if (!current() || !state.turn) return false;
                if (value.terminal_seal) {
                    if (!await verifyTerminal(identity, String(value.terminal_seal.seal_digest), value.run_id)) throw new Error('Unverified observed terminal');
                    void wakeQueuedAction();
                    return true;
                }
                return current();
            }, cause => { if (current()) { state.observedCanSubmit = false; if (cause instanceof ApiError && cause.code === 'run_control_unconfirmed') { state.challenge = null; state.reviewText = ''; state.observedRecovery = 'control_unconfirmed'; state.error = 'chatPage.controlUnconfirmed'; return; } if (state.turn && !canReleaseChatTurn(state.turn)) replyNotice(state.turn, 'uncertain'); else state.error = 'chatPage.uncertain'; } },
            state.turn?.runId && !canReleaseChatTurn(state.turn) ? { run_id: state.turn.runId, assistant_message_id: state.turn.identity.assistantMessageId } : undefined,
            () => reconcileSubmission(runObserver.signal), () => state.turn?.runId && !canReleaseChatTurn(state.turn) ? { run_id: state.turn.runId, assistant_message_id: state.turn.identity.assistantMessageId } : undefined);
    }
    function journals() {
        if (!active) throw new Error('Missing journal conversation');
        if (!journalReader) journalReader = new AssistantJournalReader(hostId, active.conversationId, generation,
            (body, signal) => api('/api/chat/agent/v2/journal', { method: 'POST', body: JSON.stringify(body), signal }));
        return journalReader;
    }
    function applyJournal(update: JournalUpdate | null, identity: TimelineOwner) {
        if (!update || !sameActive(identity) || !state.timeline) return;
        const result = reduceChatTimeline(state.timeline, { type: 'journal', owner: identity, sourceHostId: hostId,
            runId: update.through.identity.run_id, update });
        if (result.ignored && result.ignored !== 'duplicate') throw new Error('Journal timeline admission failed');
        state.timeline = result.state;
    }
    async function consumeSubmittedDraft(identity: ChatTurnOwner) {
        if (submittedDraftUnavailable || isEditedTurn()) return;
        if (!state.turn || state.turn.owner.conversationId !== identity.conversationId) return;
        try { await workspaceHost.retireSubmission(state.turn.identity.submissionId); }
        catch {
            submittedDraftUnavailable = true;
            state.notice = 'chatPage.draftStorageFailed';
            return;
        }
        workspaceHost.submissions.delete(identity.conversationId);
        publish();
    }
    async function verifyTerminal(identity: ChatTurnOwner, sealDigest?: string, candidateRunId?: string): Promise<boolean> {
        if (state.pendingSubmission && state.pendingSubmission.receipt?.phase !== 'admitted') return false;
        if (!state.turn || !state.timeline || !sameActive(identity)) return false;
        if (canReleaseChatTurn(state.turn)) { if (terminalWitness) void readTerminalWitness(terminalWitness); return true; }
        const captured = state.turn;
        const current = () => sameActive(identity) && !!state.turn && state.turn.identity.submissionId === captured.identity.submissionId;
        const evidence = await readTerminal(captured, journals(),
            request => api('/api/chat/agent/v2/journal', { method: 'POST', body: JSON.stringify(request) }),
            update => applyJournal(update, identity), current, sealDigest, candidateRunId);
        if (!evidence || !current() || !state.turn || !state.timeline) return false;
        const rendered = reduceChatTimeline(state.timeline, { type: 'terminal', owner: identity, evidence });
        if (rendered.ignored && rendered.ignored !== 'duplicate') throw new Error('Terminal journal not materialized: ' + rendered.ignored);
        const accepted = reduceChatTurn(state.turn, { type: 'terminal_verified', owner: identity, evidence });
        if (accepted.ignored && accepted.ignored !== 'committed') throw new Error('Terminal owner changed');
        // A complete Core terminal manifest supplies the current admission and
        // submission parent even when the initial history window was unavailable.
        // Historical rendering must not keep a verified completed Run busy.
        if (initializationPrepared) initialized = true;
        state.timeline = rendered.state; state.turn = accepted.state;
        terminalWitness = evidence;
        state.error = '';
        if (!observedTaskReadOnly) {
            replyNotice(state.turn, null);
            browserNotifications.completed(evidence, identity, t(get(locale), 'settings.notifications.completedTitle'), t(get(locale), 'settings.notifications.completedBody'));
        }
        presentation?.cacheChanged?.();
        if (state.pendingSubmission?.receipt?.phase === 'admitted') { submissionStorage().clear(state.pendingSubmission); state.pendingSubmission = null; }
        state.challenge = null; state.reviewText = ''; state.pendingText = ''; state.pendingAttachmentNames = [];
        if (!observedTaskReadOnly) await consumeSubmittedDraft(identity);
        if (current()) void readTerminalWitness(evidence);
        return canReleaseChatTurn(state.turn);
    }
    /** Enrich the exact user row from a real source snapshot. Never replace the
     * loaded history, replay a journal, or make terminal release depend on IO. */
    async function readTerminalWitness(evidence: VerifiedTerminal) {
        if (!state.timeline || !state.turn || !canReleaseChatTurn(state.turn) || !foreground || !presentation || state.historyLoading
            || state.turn.identity.submissionId !== evidence.submissionId || state.turn.runId !== evidence.seal.identity.run_id) return;
        const missing = evidence.declarations.some(({input}) => input.role === 'user'
            && state.timeline!.messages.some(message => message.id === input.id && (!message.canonical || !message.canonicalSnapshot)));
        if (!missing) return;
        const key = JSON.stringify([evidence.submissionId, evidence.seal.identity.run_id, evidence.seal.seal_digest]);
        if (witnessAttempt === key) return;
        stopWitnessRead(); witnessAttempt = key;
        const controller = new AbortController(); witnessController = controller;
        const request = timelineRequest(state.timeline), hooks = presentation, history = historyEpoch;
        const owned = () => witnessController === controller && !controller.signal.aborted && sameActive(evidence.owner)
            && foreground && presentation === hooks && historyEpoch === history && terminalWitness === evidence
            && !!state.turn && state.turn.identity.submissionId === evidence.submissionId
            && state.turn.runId === evidence.seal.identity.run_id && canReleaseChatTurn(state.turn);
        const current = () => owned() && state.timeline?.version === request.version;
        try {
            const payload = await api<unknown>(messageWindowUrl(identity.conversationId), {signal: controller.signal});
            if (!current() || !state.timeline) { if (witnessController === controller) witnessAttempt = ''; return; }
            const result = reduceChatTimeline(state.timeline, {type: 'terminal_witness', request, sourceHostId: hostId, evidence, payload});
            if (result.ignored || result.state === state.timeline) return;
            const anchor = hooks.captureHistory?.('metadata');
            state.timeline = result.state;
            if (anchor && witnessController === controller && presentation === hooks) {
                publish(); await hooks.restoreHistory?.(anchor, () => owned() && state.timeline === result.state, 'metadata');
            }
        } catch {
            // Missing metadata leaves editing unavailable. The committed seal,
            // user draft and submission authority remain untouched.
        } finally { if (witnessController === controller) witnessController = null; }
    }
    function replyNotice(snapshot: ChatTurn, kind: 'notSent' | 'uncertain' | null, workspace = workspaceHost) {
        if (!workspace || snapshot.owner.hostId !== hostId || disposed) return;
        const id = snapshot.owner.conversationId;
        const remaining = (workspace.replyNotices.get(id) ?? []).filter(item => item.submissionId !== snapshot.identity.submissionId);
        workspace.replyNotices.set(id, kind ? [...remaining, {...snapshot.identity, kind}] : remaining);
    }
    async function releaseUnsent(snapshot: ChatTurn) {
        if (!sameActive(snapshot.owner) || state.turn?.identity.submissionId !== snapshot.identity.submissionId || state.turn.runId) return;
        await workspaceHost.abandonSubmission(snapshot.identity.submissionId);
        state.turn = reduceChatTurn(state.turn, {type: 'submission_not_admitted', owner: snapshot.owner, outcome: 'rejected'}).state;
        state.pendingSubmission = null; state.pendingText = ''; state.pendingAttachmentNames = []; state.challenge = null;
        state.observedCanSubmit = true; state.error = '';
        if (!editedRuns.has(snapshot.identity.submissionId)) workspaceHost?.submissions.delete(snapshot.owner.conversationId);
    }
    async function loadHistory(mode: 'replace' | 'prepend') {
        if (!active || !state.timeline || state.historyLoading) return;
        const current = owner()!;
        const request = timelineRequest(state.timeline);
        const window = state.timeline.window;
        if (mode === 'prepend' && (!window?.hasMore || window.before === null)) return;
        ++historyEpoch; stopWitnessRead();
        const restartObserver = mode === 'replace';
        if (restartObserver) stopObservation();
        let observationStarted = false;
        let historicalReader: AssistantJournalReader | null = null;
        historyController?.abort(); const controller = new AbortController(); historyController = controller;
        state.historyLoading = true; state.error = '';
        try {
            const path = mode === 'prepend' ? messageWindowUrl(current.conversationId, window!.before, 48, window!) : messageWindowUrl(current.conversationId);
            const payload = await api<unknown>(path, { signal: controller.signal });
            if (!sameActive(current) || !state.timeline || controller.signal.aborted) return;
            const captured = mode === 'prepend' ? presentation?.captureHistory?.() : null;
            const result = reduceChatTimeline(state.timeline, { type: 'history', request, mode, payload });
            if (result.ignored) { if (result.ignored === 'snapshot_changed') state.notice = 'chatPage.historyChanged'; return; }
            state.timeline = result.state;
            // Parent revision/admission and restored drafts are sufficient for submission.
            // Historical Part bodies are presentation data, not a send prerequisite.
            if (initializationPrepared) initialized = true;
            // The live observer and history replay have independent cancellation/read queues.
            historicalReader = new AssistantJournalReader(hostId, current.conversationId, generation,
                (body, signal) => api('/api/chat/agent/v2/journal', { method: 'POST', body: JSON.stringify(body), signal: signal ? AbortSignal.any([signal, controller.signal]) : controller.signal }));
            if (restartObserver) { observeSelected(); observationStarted = true; }
            const journalWindow = payload as { sourceHostId?: unknown; assistantJournalHeads?: unknown };
            if (Array.isArray(journalWindow.assistantJournalHeads) && journalWindow.assistantJournalHeads.length) {
                if (journalWindow.sourceHostId !== hostId) throw new Error('History journal source mismatch');
                for (const head of journalWindow.assistantJournalHeads) {
                    const cursor = parseJournalCursor(head);
                    if (!sameActive(current) || controller.signal.aborted) return;
                    applyJournal(await historicalReader.read(cursor, cursor.identity.run_id), current);
                }
            }
            if (!sameActive(current) || controller.signal.aborted) return;
            if (mode === 'replace' && state.turn && !canReleaseChatTurn(state.turn)) await verifyTerminal(state.turn.owner);
            if (mode === 'replace' && (!state.turn || canReleaseChatTurn(state.turn)) && Array.isArray(journalWindow.assistantJournalHeads)) {
                const latest = state.timeline?.projection?.messages.filter(message => message.role === 'assistant').at(-1);
                const heads = journalWindow.assistantJournalHeads.map(parseJournalCursor);
                const member = latest && ['completed', 'failed', 'cancelled'].includes(latest.state)
                    ? heads.find(head => head.identity.assistant_message_id === latest.message_id) : undefined;
                const historicalOwner = turnOwner();
                const historicalCurrent = () => sameActive(current) && !controller.signal.aborted && (!state.turn || canReleaseChatTurn(state.turn));
                if (member && historicalOwner && historicalCurrent()) {
                    try {
                        const usage = await readHistoricalRunContextUsage(historicalOwner, member, usageProjection,
                            body => api('/api/chat/agent/v2/journal', { method: 'POST', body: JSON.stringify(body), signal: controller.signal }), historicalCurrent);
                        if (historicalCurrent()) state.contextUsage = usage;
                    } catch { if (historicalCurrent()) state.contextUsage = null; }
                }
            }
            if (!sameActive(current) || controller.signal.aborted) return;
            if (captured) { publish(); await presentation?.restoreHistory?.(captured); }
        } catch (cause) {
            if (controller.signal.aborted || !sameActive(current)) return;
            if (mode === 'prepend' && cause instanceof ApiError && cause.status === 409) {
                state.notice = 'chatPage.historyChanged';
                state.historyLoading = false;
                await loadHistory('replace');
            } else state.error = 'chatPage.loadFailed';
        } finally {
            historicalReader?.dispose();
            if (historyController === controller) { historyController = null; state.historyLoading = false; }
            if (restartObserver && !observationStarted && sameActive(current) && !controller.signal.aborted) observeSelected();
            if (terminalWitness) void readTerminalWitness(terminalWitness);
        }
    }
    async function applyEvent(value: Record<string, unknown>, identity: ChatTurnOwner) {
        if (!sameActive(identity) || !state.timeline || !state.turn || state.turn.owner.conversationId !== identity.conversationId || state.turn.owner.generation !== identity.generation) return;
        if (value.type === 'submission_receipt') {
            if (state.pendingSubmission) await acceptSubmission(value.receipt);
            else if (observedReceipt) observedReceipt = parseSourceReceipt(value.receipt, observedReceipt.identity, observedReceipt.request_digest);
            else throw new Error('Missing source submission owner');
            void refreshRunUsage(identity);
            return;
        }
        if (state.pendingSubmission && state.pendingSubmission.receipt?.phase !== 'admitted') {
            if (['assistant_journal_committed_v1', 'assistant_run_terminal_committed_v1', 'permission_request'].includes(String(value.type)))
                throw new Error('Event preceded source submission admission');
            return;
        }
        if (value.type === 'context_usage') {
            if (value.source_host_id === identity.hostId && value.session_id === identity.conversationId) void refreshRunUsage(identity);
            return;
        }
        if (value.type === 'assistant_run_terminal_committed_v1') {
            if (value.source_host_id === identity.hostId && value.session_id === identity.conversationId
                && (value.submission_id === undefined || value.submission_id === state.turn.identity.submissionId)) {
                if (typeof value.run_id !== 'string' || state.turn.runId !== null && value.run_id !== state.turn.runId || typeof value.seal_digest !== 'string') return;
                await verifyTerminal(identity, value.seal_digest, value.run_id);
            }
            return; // A bounded notification is only a query hint.
        }
        if (value.type === 'assistant_journal_committed_v1') {
            const submission = state.turn.identity.submissionId;
            const through = parseJournalCursor(value.through);
            if (value.source_host_id !== hostId || value.session_id !== identity.conversationId || state.turn.runId !== null && through.identity.run_id !== state.turn.runId) throw new Error('Journal admission owner mismatch');
            const root = { ...through.identity, assistant_message_id: state.turn.identity.assistantMessageId };
            const declarations = journals().knownAdmission(root, through.identity.assistant_message_id) ?? await journals().admit(root);
            if (!declarations.some(message => message.input.id === through.identity.assistant_message_id && message.input.role === 'assistant')) throw new Error('Journal assistant lacks message admission');
            if (!sameActive(identity) || !state.turn || !state.timeline || state.turn.identity.submissionId !== submission) return;
            state.turn = reduceChatTurn(state.turn, { type: 'run_bound', owner: state.turn.owner, runId: root.run_id }).state;
            state.timeline = reduceChatTimeline(state.timeline, { type: 'admissions', owner: identity, messages: declarations }).state;
            const update = await journals().notification(value, state.turn.runId ?? submission);
            if (!sameActive(identity) || !state.turn || state.turn.identity.submissionId !== submission) return;
            applyJournal(update, identity);
            return; // A committed journal watermark is never terminal authority.
        }
        if (value.type === 'permission_request') {
            const decision = reduceChatTurn(state.turn, { type: 'permission_pending', owner: identity, challenge: value, nowSeconds: Math.floor(Date.now() / 1000) });
            if (!decision.ignored) { state.turn = decision.state; state.challenge = value; state.reviewText = typeof value.safe_review_text === 'string' ? value.safe_review_text : ''; }
            else if (decision.ignored === 'permission') {
                permissionRefresh?.stop();
                const stream = streamController;
                permissionRefresh = refreshStreamPermissions(state.turn,
                    (body, signal) => api('/api/chat/agent/v2/observe', {method: 'POST', body: JSON.stringify(body), signal}),
                    () => !disposed && foreground && streamController === stream && !stream?.signal.aborted ? state.turn : null,
                    (turn, challenge) => {
                        state.turn = turn; state.challenge = challenge;
                        state.reviewText = typeof challenge?.safe_review_text === 'string' ? challenge.safe_review_text : '';
                    }, () => { state.error = 'chatPage.uncertain'; });
                const refresh = permissionRefresh;
                stream?.signal.addEventListener('abort', () => refresh?.stop(), {once: true});
            }
        }
        const accepted = reduceChatTurn(state.turn, { type: 'event', owner: identity, payload: value });
        if (!accepted.ignored) {
            state.turn = accepted.state;
            if (canReleaseChatTurn(state.turn)) {
                state.challenge = null; state.reviewText = ''; state.pendingText = ''; state.pendingAttachmentNames = [];
                await consumeSubmittedDraft(identity);
            }
        }
    }
    async function consume(path: string, body: Record<string, unknown>, identity: ChatTurnOwner) {
        if (!state.turn) return;
        stopObservation();
        const controller = new AbortController(); streamController = controller;
        state.turn = reduceChatTurn(state.turn, { type: 'stream_opened', owner: identity }).state;
        // Body hydration is ordered, but must never hold up subsequent permission/control frames.
        const journalQueue = journalEventQueue(async event => {
            if (streamController === controller && sameActive(identity)) await applyEvent(event, identity);
        });
        try {
            for await (const event of chatEventStream(path, { method: 'POST', body: JSON.stringify(body), signal: controller.signal })) {
                if (!sameActive(identity) || !state.turn || canReleaseChatTurn(state.turn)) break;
                await journalQueue.push(event);
            }
            await journalQueue.flush();
            if (sameActive(identity) && state.turn && !canReleaseChatTurn(state.turn) && !await verifyTerminal(identity)) {
                state.turn = reduceChatTurn(state.turn, { type: 'transport_interrupted', owner: identity, reason: 'eof' }).state;
                replyNotice(state.turn, 'uncertain');
            }
        } catch {
            if (sameActive(identity) && state.turn && !canReleaseChatTurn(state.turn)) {
                try { if (await verifyTerminal(identity)) return; } catch { /* Keep uncertainty if the authenticated frontier is unavailable. */ }
                if (!sameActive(identity) || !state.turn) return;
                state.turn = reduceChatTurn(state.turn, { type: 'transport_interrupted', owner: identity, reason: 'error' }).state;
                replyNotice(state.turn, 'uncertain');
            }
        } finally {
            if (streamController === controller) { streamController = null; if (sameActive(identity)) observeSelected(); }
        }
    }
    async function submit(input: SubmissionInput, queuedCommandId?: string, edited?: EditedResendEnvelope) {
        if (disposed || !initialized) { if (edited) throw new Error('Edited resend controller is not ready'); return; }
        const text = input.text;
        const attachments = input.attachments;
        const language = input.language;
        const identity = turnOwner();
        if (!identity || !state.timeline || !state.observedCanSubmit || state.turn && !canReleaseChatTurn(state.turn) || (!text.trim() && !attachments.length)) {
            if (edited) throw new Error('Edited resend target is busy or unavailable'); return;
        }
        // Recheck the owned draft synchronously for keyboard/programmatic submit,
        // including the debounce interval before a write has even started.
        if (!edited && knowledgePending(workspaceHost?.knowledge.get(identity.conversationId))) return;
        const captured = attachments.map(item => ({ ...item }));
        let modelContent: string;
        try { modelContent = attachmentModelContent(text, captured, identity.conversationId); }
        catch (cause) { state.error = 'chatPage.sendFailed'; if (edited) throw cause; return; }
        const submissionId = queuedCommandId ?? input.submissionId ?? crypto.randomUUID();
        stopWitnessRead(); terminalWitness = null;
        if (!queuedCommandId && !edited) workspaceHost?.submissions.set(identity.conversationId, {submissionId, text, attachmentIds: captured.map(item => item.id)});
        stopObservation();
        const turnIdentity = edited ? {submissionId:edited.prepared.run_id,userMessageId:edited.prepared.user_message_id,
            assistantMessageId:edited.prepared.assistant_message_id} : chatTurnIdentity(submissionId);
        if (queuedCommandId) turnIdentity.userMessageId = `ui:${submissionId}:user`;
        state.turn = createChatTurn(identity, turnIdentity);
        usageProjection.reset(); state.contextUsage = null;
        state.timeline = reduceChatTimeline(state.timeline, { type: 'beginSubmission', owner: identity, submissionId,
            userMessageId: state.turn.identity.userMessageId, userText: modelContent }).state;
        state.pendingText = ''; state.pendingAttachmentNames = captured.map(item => item.name); state.error = ''; state.notice = '';
        const request = {
            conversationId: identity.conversationId, client_id: identity.clientId,
            expected_host_id: identity.hostId,
            submission_id: submissionId, user_message_id: state.turn.identity.userMessageId, assistant_message_id: state.turn.identity.assistantMessageId,
            messages: [{ role: 'user', content: modelContent }], current_user_display_text: text,
            timezone: Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC', language,
            execution_backend: active.executionBackend,
            expected_configuration_revision: edited ? edited.target.configuration_revision : active.configurationRevision,
            expected_parent_message_id: edited ? null : state.timeline.submissionParentMessageId,
            ...(edited ? {edited_resend_operation_id:edited.prepared.input.operation_id} : {}),
            ...(queuedCommandId ? {queued_user_command_id: queuedCommandId} : {}),
        };
        try {
            const frozenRequest = await freezeHostToolPlan(request, api);
            if (!sameActive(identity) || state.turn?.identity.submissionId !== submissionId) {
                if (edited) throw new Error('Edited resend target owner changed'); return;
            }
            state.pendingSubmission = createSourceSubmission(sourceIdentity(identity, state.turn.identity), frozenRequest);
            if (!edited) await workspaceHost.stageSubmission(state.pendingSubmission);
            submissionStorage().save(state.pendingSubmission);
            state.observedCanSubmit = false;
            const captured = state.pendingSubmission;
            const submittedTurn = state.turn;
            const submittedWorkspace = workspaceHost;
            const completionObserver = workspaceHost!.completions;
            const sourceRecovery = workspaceHost!.sources;
            const transport = new AbortController();
            const transports = workspaceHost!.transports;
            transports.add(transport);
            dispatchControllers.add(transport);
            const submittedJournal = journalEventQueue(async event => {
                if (sameActive(identity) && state.turn?.identity.submissionId === captured.identity.submission_id) await applyEvent(event, identity);
            });
            void dispatchSourceSubmission(captured, {
                storage: submissionStorage(), signal: transport.signal,
                onReceipt: async value => {
                    if (!edited) await workspaceHost.updateSubmission(value);
                    editedReceipt(value);
                    if (!edited && value.receipt?.phase === 'admitted') await workspaceHost.retireSubmission(value.identity.submission_id);
                    if (value.receipt?.phase !== 'admitted') return;
                    const admitted = reduceChatTurn(submittedTurn, { type: 'run_bound', owner: identity,
                        runId: value.receipt.admission!.identity.run_id }).state;
                    void completionObserver.watch(admitted);
                },
                prepare: (value, signal) => api('/api/chat/agent/v2/submission', { method: 'POST', signal, body: JSON.stringify({
                    operation: 'prepare', identity: value.identity, request: value.request }) }),
                stream: (body, signal) => chatEventStream('/api/chat/agent/v2/stream', {method: 'POST', signal, body: JSON.stringify(body)}),
                bound: async value => {
                    if (!edited) await workspaceHost.updateSubmission(value);
                    void sourceRecovery.watch(value);
                    if (!sameActive(identity) || state.pendingSubmission?.identity.submission_id !== captured.identity.submission_id || !state.turn) return;
                    state.pendingSubmission = value;
                    state.turn = reduceChatTurn(state.turn, {type: 'stream_opened', owner: identity}).state;
                },
                event: event => submittedJournal.push(event),
            }).then(async () => {
                await submittedJournal.flush();
                if (edited && editedAdmissions.has(submissionId)) failEdited(submissionId, new Error('Edited resend admission is unconfirmed; observe the same operation before retrying'));
                if (sameActive(identity) && state.turn?.identity.submissionId === captured.identity.submission_id && !canReleaseChatTurn(state.turn)) {
                    state.turn = reduceChatTurn(state.turn, {type: 'transport_interrupted', owner: identity, reason: 'eof'}).state;
                    replyNotice(state.turn, 'uncertain');
                }
            }).catch(async cause => {
                if (edited) failEdited(submissionId, cause);
                if (cause instanceof SourceSubmissionNotDispatchedError) {
                    replyNotice(submittedTurn, 'notSent', submittedWorkspace);
                    try { await releaseUnsent(submittedTurn); } catch { if (sameActive(identity)) state.error = 'chatPage.draftStorageFailed'; }
                } else if (sameActive(identity) && state.turn?.identity.submissionId === captured.identity.submission_id && !canReleaseChatTurn(state.turn)) {
                    state.turn = reduceChatTurn(state.turn, {type: 'transport_interrupted', owner: identity, reason: 'error'}).state;
                    replyNotice(state.turn, 'uncertain');
                }
            })
                .finally(() => {
                    transports.delete(transport);
                    dispatchControllers.delete(transport);
                    if (sameActive(identity) && state.turn?.identity.submissionId === captured.identity.submission_id) observeSelected();
                });
        } catch (cause) {
            if (edited) failEdited(submissionId, cause);
            // No dispatcher exists yet: local preparation/storage failure cannot have sent a Run.
            if (state.turn?.identity.submissionId === submissionId && !state.turn.runId) {
                const snapshot = state.turn;
                if (state.pendingSubmission) { try { submissionStorage().clear(state.pendingSubmission); } catch { /* Preserve unavailable storage for diagnosis. */ } }
                replyNotice(snapshot, 'notSent');
                try { await releaseUnsent(snapshot); } catch { state.error = 'chatPage.draftStorageFailed'; }
            }
        }
    }
    async function submitPreparedEditedResend(envelope: EditedResendEnvelope, language: string): Promise<SourceSubmissionReceipt> {
        const prepared=envelope.prepared;
        if (envelope.host_id!==hostId || envelope.client_id!==clientId || envelope.target.id!==identity.conversationId
            || prepared.input.target_conversation_id!==identity.conversationId || prepared.run_id!==prepared.input.operation_id
            || prepared.user_message_id!==`${prepared.run_id}:user` || prepared.assistant_message_id!==`${prepared.run_id}:assistant`) {
            throw new Error('Edited resend controller owner mismatch');
        }
        editedRuns.add(prepared.run_id);
        await initialize();
        if (disposed || !initialized || !state.timeline) throw new Error('Edited resend history is unavailable');
        if (state.pendingSubmission) {
            if (!isEditedPending(state.pendingSubmission) || state.pendingSubmission.identity.submission_id!==prepared.run_id) throw new Error('Another target submission is pending');
            await reconcileSubmission(new AbortController().signal);
            const pending=state.pendingSubmission;
            if (pending?.receipt?.phase==='admitted') return pending.receipt;
            // A stream may already have reached the Host. Never manufacture another dispatch.
            if (pending?.sent || pending?.request_digest) throw new Error('Original edited resend admission remains unconfirmed');
            if (pending) throw new Error('Original edited resend preparation remains pending');
        }
        const raw=await api('/api/chat/agent/v2/observe',{method:'POST',body:JSON.stringify({operation:'discover',
            conversation_id:identity.conversationId,expected_host_id:hostId,client_id:clientId})});
        if (disposed) throw new Error('Edited resend owner disposed');
        const observed=parseRunObservation(raw,identity), goal=parseGoalControl(raw,identity);
        if (observed) {
            if (observed.run_id!==prepared.run_id || observed.assistant_message_id!==prepared.assistant_message_id) throw new Error('Edited resend target already has another Run');
            return parseObservedSourceReceipt(observed.submission_receipt,identity,prepared.run_id,prepared.assistant_message_id);
        }
        if (prepared.admitted_run_id!==null || goal && !goal.can_submit) throw new Error('Prepared admission must be observed, never redispatched');
        if (state.timeline.projection?.messages.length) throw new Error('Edited resend target is not empty');
        state.observedCanSubmit=true;
        return new Promise<SourceSubmissionReceipt>((resolve,reject)=>{
            if (editedAdmissions.has(prepared.run_id)) {reject(new Error('Edited resend dispatch already in progress'));return;}
            editedAdmissions.set(prepared.run_id,{resolve,reject});
            void submit({text:prepared.input.edited_text,attachments:[],language,submissionId:prepared.run_id},undefined,envelope)
                .catch(cause=>failEdited(prepared.run_id,cause));
        });
    }
    async function controlObservedGoal(action: 'pause' | 'resume') {
        const goal = observedGoal;
        if (!goal || goal.run_id !== state.turn?.runId) throw new Error('Goal execution owner changed');
        await dispatchGoalControlIntent(identity.conversationId, {
            operationId: crypto.randomUUID(), action, expectedGoalId: goal.goal_id, expectedControlToken: goal.control_token,
        }, api, options.goalStorage ?? new GoalControlIntentStore(localStorage, hostId));
        // Command admission is not a stop confirmation. Continue reading Core.
        observeSelected();
    }
    async function resume() {
        if (disposed || observedTaskReadOnly) return;
        if (!state.turn || state.turn.phase !== 'recovering' || state.observedRecovery !== 'resume_required') return;
        const route = observedGoalResumeRoute(observedGoal, observedGoalControl);
        if (route === 'refresh') { state.error = 'chatPage.uncertain'; observeSelected(); return; }
        if (route === 'goal') {
            try { await controlObservedGoal('resume'); } catch { state.error = 'chatPage.uncertain'; observeSelected(); }
            return;
        }
        const body = buildChatResumeRequest(state.turn);
        if (!body) { state.error = 'chatPage.uncertain'; return; }
        await consume('/api/chat/agent/v2/resume', body, state.turn.owner);
    }
    async function stop() {
        if (disposed || observedTaskReadOnly) return;
        if (!state.turn || !active || state.turn.phase === 'committed' || state.turn.phase === 'cancel_requested') return;
        const identity = state.turn.owner;
        try {
            if (observedGoal) { await controlObservedGoal('pause'); return; }
            if (state.pendingSubmission) {
                const id = state.pendingSubmission.identity;
                const latest = submissionStorage().read(id.source_host_id, id.client_id, id.conversation_id);
                if (latest?.identity.submission_id === id.submission_id) {
                    state.pendingSubmission = latest;
                    if (latest.receipt) await acceptSubmission(latest.receipt);
                }
            }
            if (canReleaseChatTurn(state.turn)) { observeSelected(); return; }
            if (state.pendingSubmission && state.pendingSubmission.receipt?.phase !== 'admitted') {
                const captured = state.pendingSubmission;
                if (captured.request_digest === null && !captured.sent) {
                    await workspaceHost.abandonSubmission(captured.identity.submission_id);
                    submissionStorage().clear(captured); state.pendingSubmission = null;
                    state.turn = reduceChatTurn(state.turn, { type: 'submission_not_admitted', owner: identity, outcome: 'cancelled' }).state;
                    observeSelected(); return;
                }
                const response = await api<{ source_host_id: string; result: unknown }>('/api/chat/agent/v2/submission', {
                    method: 'POST', body: JSON.stringify(sourceSubmissionQuery(captured, true)) });
                if (!sameActive(identity) || state.pendingSubmission?.identity.submission_id !== captured.identity.submission_id) return;
                if (response.source_host_id !== identity.hostId || response.result === null) { state.error = 'chatPage.uncertain'; return; }
                await acceptSubmission(response.result);
                if (!state.pendingSubmission) { observeSelected(); return; }
            }
            const cancel = state.pendingSubmission ? exactSubmissionCancel(state.pendingSubmission) : observedReceipt ? admittedReceiptCancel(observedReceipt) : null;
            if (!cancel) throw new Error('No exact source cancellation authority');
            const result = await api<{ cancelRequested?: boolean; terminationConfirmed?: boolean }>('/api/chat/agent/v2/cancel', { method: 'POST', body: JSON.stringify(cancel) });
            if (!sameActive(identity) || !state.turn) return;
            if (result.cancelRequested || result.terminationConfirmed) state.turn = reduceChatTurn(state.turn, { type: 'cancel_requested', owner: identity }).state;
            else state.error = 'chatPage.uncertain';
        } catch { if (sameActive(identity)) state.error = 'chatPage.uncertain'; }
    }
    function permissionAccepted(identity: string) {
        const parsed = parseChatPermission(state.challenge);
        if (!parsed || permissionIdentity(parsed) !== identity || !state.turn?.permission || state.turn.permission.nonce !== parsed.nonce) return;
        state.turn = reduceChatTurn(state.turn, { type: 'permission_acknowledged', owner: state.turn.owner, nonce: parsed.nonce }).state;
    }

    function initialize(): Promise<void> { return initialLoad ??= initializeCore(); }
    async function initializeCore() {
        if (disposed) return;
        try {
            await workspaceHost.load(identity.conversationId);
            if (!workspaceHost.isLoaded(identity.conversationId)) throw new Error('Draft unavailable');
            state.historicalRecoveryOwners = await (options.recoveryOwners?.()
                ?? new BrowserRecoveryIntentStore().listOwners(hostId, clientId, identity.conversationId));
            for (const pending of await workspaceHost.restoreSubmissions(clientId)) {
                if (pending.identity.conversation_id !== identity.conversationId) continue;
                const latest = submissionStorage().read(hostId, clientId, identity.conversationId);
                if (!latest) submissionStorage().save(pending);
            }
            state.pendingSubmission = submissionStorage().read(hostId, clientId, identity.conversationId);
            if (state.pendingSubmission) {
                const saved = state.pendingSubmission.identity;
                state.turn = createChatTurn(identity, {submissionId: saved.submission_id,
                    userMessageId: saved.user_message_id, assistantMessageId: saved.assistant_message_id});
                state.pendingText = typeof state.pendingSubmission.request.current_user_display_text === 'string'
                    ? state.pendingSubmission.request.current_user_display_text : '';
                if (state.pendingSubmission.receipt) await acceptSubmission(state.pendingSubmission.receipt);
            }
            // Queue recovery is independent of reading the existing transcript.
            // A foreign/temporarily unavailable queue must not blank the conversation.
            try { await checkQueued(); }
            catch { if (!disposed) state.error = 'chatPage.uncertain'; }
            initializationPrepared = true;
            await loadHistory('replace');
        } catch { state.error = 'chatPage.uncertain'; state.observedCanSubmit = false; }
        publish();
    }
    return {
        owner: identity,
        attach(value, hooks = {}) {
            listener = value; presentation = hooks; publish();
            if (initialized) void loadHistory('replace').then(observeSelected);
        },
        detach() { stopWitnessRead(); listener = null; presentation = null; stopObservation(); stopQueuedWake(); },
        dispose() { stopWitnessRead(); for (const [run] of editedAdmissions) failEdited(run, new Error('Edited resend controller disposed')); disposed = true; stopQueuedWake(); listener = null; presentation = null; stopObservation(); streamController?.abort(); historyController?.abort(); for (const transport of dispatchControllers) transport.abort(); dispatchControllers.clear(); },
        initialize, loadHistory, submit, submitPreparedEditedResend, cancelQueued, checkQueued,
        submitQueued: async _commandId => { await wakeQueuedAction(); },
        resume, stop, observe: observeSelected,
        setForeground(value) { foreground = value; if (value) { observeSelected(); void wakeQueuedAction(); } else { stopWitnessRead(); stopObservation(); stopQueuedWake(); } },
        updateConversation(value) { if (value.conversationId !== identity.conversationId) throw new Error('Conversation owner changed'); active = value; },
        acceptReceipt: acceptSubmission, permissionAccepted,
    };
}
