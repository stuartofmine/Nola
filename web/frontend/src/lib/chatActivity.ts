import { conversationUrl } from './chatConversations.ts';

export interface ChatTraceSummary {
    traceId: string;
    runId: string;
    status: string;
    lastSequence: number;
    startedAt: string;
    finishedAt: string | null;
}
export interface ChatGoalSummary {
    controlToken: string;
    canSubmit: boolean;
    canPause: boolean;
    canResume: boolean;
    canCancel: boolean;
    id: string;
    executionBlocker: 'configuration_unavailable' | null;
    revision: number;
    objective: string;
    status: string;
    completedRequirements: number;
    totalRequirements: number;
    requirements: { id: string; description: string; status: string }[];
    nextWakeAtEpochMs: number | null;
}

/** Goal control and execution traces are independent committed projections.
 * A cancelled/paused Goal receipt does not yet confirm that its Run stopped. */
export function chatActivityNeedsPolling(goal: ChatGoalSummary | null, traces: ChatTraceSummary[], controlPending: boolean): boolean {
    return controlPending || goal?.status === 'running' || goal?.nextWakeAtEpochMs != null
        || traces.some(trace => ['pending', 'running', 'waiting'].includes(trace.status));
}
const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === 'object' && !Array.isArray(value);
const count = (value: unknown): value is number => typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;

export function chatActivityUrl(conversationId: string, kind: 'traces' | 'goal' | 'plan'): string {
    const path = `${conversationUrl(conversationId)}/${kind}`;
    return kind === 'traces' ? `${path}?summaryOnly=1` : path;
}

export interface ChatPlanSummary { revision: number; todos: { id: string; content: string; status: string }[] }
export function parseChatPlan(value: unknown, conversationId: string): ChatPlanSummary | null {
    if (!object(value) || value.conversationId !== conversationId || value.authoritative !== true) throw new Error('chatActivity.invalidResponse');
    if (value.plan === null) return null;
    const plan = value.plan;
    if (!object(plan) || plan.conversation_id !== conversationId || !count(plan.revision) || !Array.isArray(plan.todos)) throw new Error('chatActivity.invalidResponse');
    const ids = new Set<string>();
    const todos = plan.todos.map(item => {
        if (!object(item) || typeof item.id !== 'string' || !item.id.trim() || ids.has(item.id)
            || typeof item.content !== 'string' || !item.content.trim() || typeof item.status !== 'string'
            || !['pending', 'in_progress', 'completed', 'cancelled'].includes(item.status)) throw new Error('chatActivity.invalidResponse');
        ids.add(item.id);
        return { id: item.id, content: item.content, status: item.status };
    });
    return { revision: plan.revision, todos };
}

/** Trace events may contain tool input and sensitive diagnostics; this read
 * model intentionally discards them rather than rendering arbitrary events. */
export function parseChatTraces(value: unknown, conversationId: string): ChatTraceSummary[] {
    if (!object(value) || value.conversationId !== conversationId || !Array.isArray(value.traces)) throw new Error('chatActivity.invalidResponse');
    const ids = new Set<string>();
    return value.traces.map((item: unknown) => {
        if (!object(item) || typeof item.traceId !== 'string' || !item.traceId.trim() || ids.has(item.traceId)
            || typeof item.runId !== 'string' || !item.runId.trim() || typeof item.status !== 'string' || !item.status.trim()
            || !count(item.lastSequence) || typeof item.startedAt !== 'string' || !(item.events === undefined || Array.isArray(item.events))
            || !(item.finishedAt === null || typeof item.finishedAt === 'string')) throw new Error('chatActivity.invalidResponse');
        ids.add(item.traceId);
        return { traceId: item.traceId, runId: item.runId, status: item.status, lastSequence: item.lastSequence,
            startedAt: item.startedAt, finishedAt: item.finishedAt };
    }).sort((left, right) => left.startedAt.localeCompare(right.startedAt) || left.traceId.localeCompare(right.traceId));
}

export function parseChatGoal(value: unknown, conversationId: string): ChatGoalSummary | null {
    if (!object(value) || value.conversationId !== conversationId || value.authoritative !== true
        || !(value.goalProjection === null || object(value.goalProjection))) throw new Error('chatActivity.invalidResponse');
    const projection = value.goalProjection;
    if (projection === null) return null;
    if (!count(projection.revision) || !(projection.active === null || projection.active === undefined || object(projection.active))) throw new Error('chatActivity.invalidResponse');
    if (projection.active === null || projection.active === undefined) return null;
    const goal = projection.active;
    if (typeof projection.control_token !== 'string' || !projection.control_token.trim()) throw new Error('chatActivity.invalidResponse');
    if (['can_submit', 'can_pause', 'can_resume', 'can_cancel'].some(key => typeof projection[key] !== 'boolean'))
        throw new Error('chatActivity.invalidResponse');
    if (typeof goal.id !== 'string' || !goal.id.trim() || typeof goal.objective !== 'string' || !goal.objective.trim() || typeof goal.status !== 'string' || !goal.status.trim()
        || !object(goal.requirements) || !(goal.next_wake_at_epoch_ms === null || goal.next_wake_at_epoch_ms === undefined
            || Number.isSafeInteger(goal.next_wake_at_epoch_ms))) throw new Error('chatActivity.invalidResponse');
    const blocker = goal.execution_blocker;
    if (blocker !== null && (!object(blocker) || blocker.code !== 'configuration_unavailable' || typeof blocker.message !== 'string'))
        throw new Error('chatActivity.invalidResponse');
    const requirements = Object.entries(goal.requirements).map(([id, item]) => {
        if (!object(item) || item.id !== id || typeof item.description !== 'string' || !item.description.trim()
            || typeof item.status !== 'string') throw new Error('chatActivity.invalidResponse');
        return { id, description: item.description, status: item.status };
    });
    return { controlToken: projection.control_token, canSubmit: projection.can_submit as boolean,
        canPause: projection.can_pause as boolean, canResume: projection.can_resume as boolean, canCancel: projection.can_cancel as boolean, id: goal.id, executionBlocker: blocker === null ? null : 'configuration_unavailable', revision: projection.revision, objective: goal.objective, status: goal.status,
        totalRequirements: requirements.length, completedRequirements: requirements.filter(item => item.status === 'completed').length, requirements,
        nextWakeAtEpochMs: typeof goal.next_wake_at_epoch_ms === 'number' ? goal.next_wake_at_epoch_ms : null };
}

export type ChatGoalAction = 'start' | 'pause' | 'resume' | 'cancel';
export type ChatGoalRequest =
    | { operationId: string; action: 'start'; objective: string }
    | { operationId: string; action: Exclude<ChatGoalAction, 'start'>; expectedGoalId: string; expectedControlToken: string };
export function validateGoalOperationId(operationId: string): void {
    if (typeof operationId !== 'string' || !operationId || operationId.trim() !== operationId || operationId.length > 256 || /[\u0000-\u001f\u007f]/.test(operationId))
        throw new Error('chatActivity.invalidResponse');
}
export function goalActionRequest(action: ChatGoalAction, objective = '', goal?: ChatGoalSummary | null, operationId: string = crypto.randomUUID()): ChatGoalRequest {
    validateGoalOperationId(operationId);
    if (action === 'start') {
        const text = objective.trim();
        if (!text || text.length > 20_000) throw new Error('chatActivity.invalidObjective');
        return { operationId, action, objective: text };
    }
    if (!goal || !goal.id || !goal.controlToken?.trim()) throw new Error('chatActivity.invalidResponse');
    return { operationId, action, expectedGoalId: goal.id, expectedControlToken: goal.controlToken };
}

export function goalAvailableActions(goal: ChatGoalSummary | null): ChatGoalAction[] {
    if (!goal) return ['start'];
    const actions: ChatGoalAction[] = [];
    if (goal.canSubmit) actions.push('start');
    if (goal.canPause) actions.push('pause');
    if (goal.canResume) actions.push('resume');
    if (goal.canCancel) actions.push('cancel');
    return actions;
}

/** A queued control is not an applied Goal transition. */
export function pendingGoalControl(value: unknown, conversationId: string): string | null {
    if (!object(value) || value.controlPending !== true) return null;
    if (value.conversationId !== conversationId || value.authoritative !== false || typeof value.operationId !== 'string'
        || !value.operationId.trim()) throw new Error('chatActivity.invalidResponse');
    return value.operationId;
}
export function goalControlOutcome(value: unknown, conversationId: string, operationId: string): 'pending' | 'applied' | 'rejected' {
    if (object(value) && value.conversationId === conversationId && value.authoritative === true && value.goalProjection === null) return 'pending';
    if (!object(value) || value.conversationId !== conversationId || value.authoritative !== true
        || !object(value.goalProjection) || !object(value.goalProjection.applied_operations)) throw new Error('chatActivity.invalidResponse');
    const operation = value.goalProjection.applied_operations[operationId];
    if (operation === undefined) return 'pending';
    if (!object(operation) || !count(operation.goal_revision) || !(operation.rejection === null || typeof operation.rejection === 'string'))
        throw new Error('chatActivity.invalidResponse');
    return operation.rejection === null ? 'applied' : 'rejected';
}
