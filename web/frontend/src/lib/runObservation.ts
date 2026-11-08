import { ApiError } from './api.ts';
import { parseJournalCursor } from './assistantJournal.ts';
import type { JournalCursor } from './assistantJournal.ts';
import { conversationExecutionOwnership } from './chatTaskDetails.ts';
import type { ConversationExecutionOwnership } from './chatTaskDetails.ts';
import { parseRunResult } from './runResult.ts';

export interface RunObservationOwner { conversationId: string; hostId: string; clientId: string }
export interface GoalExecutionOwner { goal_id: string; contract_id: string; run_id: string; control_token: string }
export interface GoalControlObservation { goal_id: string; control_token: string; status: string; can_submit: boolean; can_pause: boolean; can_resume: boolean; can_cancel: boolean }
export interface RunObservation {
    protocol_version: 1; conversation_id: string; client_id: string; run_id: string; assistant_message_id: string; status: string;
    worker: { state: 'running' | 'absent' | 'starting' | 'unknown'; generation: string | null };
    recovery: 'observe' | 'resume_required' | 'terminal' | 'inconsistent';
    admission: JournalCursor | null; terminal_seal: Record<string, unknown> | null;
    permissions: Record<string, unknown>[]; can_submit: boolean;
    submission_receipt: unknown | null;
    goal_execution: GoalExecutionOwner | null;
    task_execution?: Extract<ConversationExecutionOwnership, {kind: 'task_child'}> | null;
    // Display facts are validated separately, never control authority.
    context_usage: unknown | null;
}
const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === 'object' && !Array.isArray(value);
const id = (value: unknown): value is string => typeof value === 'string' && value.trim() === value && value.length > 0 && value.length <= 512;
export function parseGoalControl(value: unknown, owner: RunObservationOwner): GoalControlObservation | null {
    if (!object(value) || value.source_host_id !== owner.hostId || value.conversation_id !== owner.conversationId || value.client_id !== owner.clientId || !Object.hasOwn(value, 'goal_control')) throw new Error('Missing Goal control observation');
    const goal = value.goal_control;
    if (goal === null) return null;
    if (!object(goal) || !id(goal.goal_id) || !id(goal.control_token)
        || !['running', 'paused', 'needs_user', 'completed', 'failed', 'cancelled'].includes(String(goal.status))
        || ['can_pause', 'can_resume', 'can_cancel'].some(key => typeof goal[key] !== 'boolean')
        || typeof goal.can_submit !== 'boolean' || goal.can_submit && !['completed', 'cancelled', 'failed'].includes(String(goal.status)))
        throw new Error('Invalid Goal control observation');
    return goal as unknown as GoalControlObservation;
}
export function parseRunObservation(value: unknown, owner: RunObservationOwner, expected?: Pick<RunObservation, 'run_id' | 'assistant_message_id'>): RunObservation | null {
    if (!object(value) || value.source_host_id !== owner.hostId || value.conversation_id !== owner.conversationId || value.client_id !== owner.clientId || !Object.hasOwn(value, 'result')) throw new Error('Observation source changed');
    const result = value.result;
    if (result === null && !expected) return null;
    if (!object(result) || result.protocol_version !== 1 || result.conversation_id !== owner.conversationId || result.client_id !== owner.clientId
        || !id(result.run_id) || !id(result.assistant_message_id) || !['running', 'sealed'].includes(String(result.status))
        || expected && (result.run_id !== expected.run_id || result.assistant_message_id !== expected.assistant_message_id)
        || !object(result.worker) || !['running', 'absent', 'starting', 'unknown'].includes(String(result.worker.state))
        || !(result.worker.generation === null || typeof result.worker.generation === 'string' && /^[0-9]+$/.test(result.worker.generation))
        || !['observe', 'resume_required', 'terminal', 'inconsistent'].includes(String(result.recovery))
        || typeof result.can_submit !== 'boolean' || !Array.isArray(result.permissions) || result.permissions.length > 16
        || !(result.terminal_seal === null || object(result.terminal_seal))
        || !Object.hasOwn(result, 'submission_receipt') || !(result.submission_receipt === null || object(result.submission_receipt))
        || !Object.hasOwn(result, 'context_usage') || !(result.context_usage === null || object(result.context_usage))) throw new Error('Invalid Run observation');
    if (!Object.hasOwn(result, 'goal_execution')) throw new Error('Missing execution ownership');
    if (result.goal_execution !== null) {
        const goal = result.goal_execution;
        if (!object(goal) || !id(goal.goal_id) || !id(goal.contract_id) || goal.run_id !== result.run_id
            || !id(goal.control_token)
            || result.submission_receipt !== null) throw new Error('Invalid Goal execution ownership');
    }
    if (result.task_execution != null) {
        const task = conversationExecutionOwnership(result.task_execution, owner.conversationId);
        if (task.kind !== 'task_child' || result.submission_receipt !== null || result.goal_execution !== null
            || result.can_submit !== false || result.permissions.length !== 0
            || !['observe','terminal'].includes(String(result.recovery))) throw new Error('Invalid read-only task observation');
    }
    const admission = result.admission === null ? null : parseJournalCursor(result.admission);
    if (object(result.terminal_seal)) {
        if (result.status !== 'sealed' || result.recovery !== 'terminal' || result.terminal_seal.protocol_version !== 2)
            throw new Error('Invalid terminal observation');
        parseRunResult(result.terminal_seal.result, owner.conversationId, result.run_id as string);
    } else if (result.status === 'sealed' || result.recovery === 'terminal' || result.can_submit) {
        throw new Error('Terminal observation has no source seal');
    }
    if (admission && (admission.identity.conversation_id !== owner.conversationId || admission.identity.run_id !== result.run_id
        || admission.identity.assistant_message_id !== result.assistant_message_id)) throw new Error('Observation admission owner changed');
    for (const permission of result.permissions) {
        if (!object(permission) || permission.host_id !== owner.hostId || permission.client_id !== (result.goal_execution ? 'web-goal' : owner.clientId)
            || permission.session_id !== owner.conversationId || permission.canonical_run_id !== result.run_id
            || !id(permission.nonce) || !id(permission.request_id) || permission.protocol_version !== 4 || typeof permission.allow_conversation !== 'boolean'
            || typeof permission.expires_at !== 'number' || permission.expires_at <= Date.now() / 1000
            || result.worker.state !== 'running') throw new Error('Invalid observed permission owner');
    }
    return { ...result, admission } as unknown as RunObservation;
}

/** One cancellable observer owns discovery and subsequent exact Run reads.
 * Retrying reads never resumes execution, submits input, or issues a decision. */
export class RunObserver {
    private controller = new AbortController();
    get signal() { return this.controller.signal; }
    stop() { this.controller.abort(); }
    async run(owner: RunObservationOwner,
        request: (body: Record<string, unknown>, signal: AbortSignal) => Promise<unknown>,
        accept: (value: RunObservation | null, goal: GoalControlObservation | null) => Promise<boolean>,
        exhausted: (error: unknown) => void,
        initial?: Pick<RunObservation, 'run_id' | 'assistant_message_id'>,
        beforeRead: () => Promise<boolean> = async () => true,
        admitted: () => Pick<RunObservation, 'run_id' | 'assistant_message_id'> | undefined = () => undefined): Promise<void> {
        let expected = initial; let failures = 0;
        const signal = this.controller.signal;
        while (!signal.aborted) {
            try {
                if (!await beforeRead()) {
                    if (signal.aborted) return;
                    await new Promise<void>(resolve => {
                        const finish = () => { clearTimeout(timer); signal.removeEventListener('abort', finish); resolve(); };
                        const timer = setTimeout(finish, 1000); signal.addEventListener('abort', finish, { once: true });
                    });
                    continue;
                }
                if (signal.aborted) return;
                expected ??= admitted();
                const raw = await request({ operation: expected ? 'observe' : 'discover', conversation_id: owner.conversationId,
                    expected_host_id: owner.hostId, client_id: owner.clientId,
                    ...(expected ? { expected_run_id: expected.run_id, assistant_message_id: expected.assistant_message_id } : {}) }, signal);
                if (signal.aborted) return;
                const value = parseRunObservation(raw, owner, expected);
                const goal = parseGoalControl(raw, owner);
                if (value) expected = value;
                if (!await accept(value, goal) || signal.aborted) return;
                // A verified terminal callback continues with discovery: a
                // different Run may already own this conversation.
                if (value?.recovery === 'terminal') expected = undefined;
                failures = 0;
            } catch (error) {
                if (signal.aborted) return;
                if (error instanceof ApiError && error.code === 'run_control_unconfirmed') { exhausted(error); return; }
                if (++failures >= 6) { exhausted(error); return; }
            }
            await new Promise<void>(resolve => {
                const finish = () => { clearTimeout(timer); signal.removeEventListener('abort', finish); resolve(); };
                const timer = setTimeout(finish, Math.min(8000, 750 * 2 ** failures));
                signal.addEventListener('abort', finish, { once: true });
            });
        }
    }
}
