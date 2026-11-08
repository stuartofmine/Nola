import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'vite';
import { chatActivityNeedsPolling, chatActivityUrl, pendingGoalControl, goalControlOutcome, goalActionRequest, goalAvailableActions, parseChatGoal, parseChatTraces, parseChatPlan } from './chatActivity.ts';

test('plan read model binds both owners and keeps real todo states without inferred progress', () => {
    const response = { conversationId: 'a', authoritative: true, plan: { conversation_id: 'a', revision: 3,
        todos: [{ id: 'one', content: 'Verify evidence', status: 'in_progress', secret: 'discard' }] } };
    assert.deepEqual(parseChatPlan(response, 'a'), { revision: 3, todos: [{ id: 'one', content: 'Verify evidence', status: 'in_progress' }] });
    assert.equal(parseChatPlan({ conversationId: 'a', authoritative: true, plan: null }, 'a'), null);
    assert.throws(() => parseChatPlan(response, 'b'));
    assert.throws(() => parseChatPlan({ ...response, plan: { ...response.plan, conversation_id: 'b' } }, 'a'));
    assert.throws(() => parseChatPlan({ ...response, plan: { ...response.plan, todos: [...response.plan.todos, ...response.plan.todos] } }, 'a'));
    assert.throws(() => parseChatPlan({ ...response, authoritative: false }, 'a'));
});

const conversationId = 'conversation A/?';
const goal = { can_submit: false, can_pause: true, can_resume: false, can_cancel: true, control_token: 'core-token', revision: 4, active: { execution_blocker: null, id: 'goal-one', objective: 'Ship the app', status: 'running', next_wake_at_epoch_ms: null,
    requirements: { one: { id: 'one', description: 'Build', status: 'completed' }, two: { id: 'two', description: 'Verify', status: 'pending' } } } };
const trace = { traceId: 'trace-1', runId: 'run-1', status: 'waiting', lastSequence: 7, startedAt: '2026-09-23T00:00:00Z', finishedAt: null,
    events: [{ args: { token: 'SECRET_TOKEN' } }] };

test('cancelled Goal keeps observing a Run until the independently committed trace settles', () => {
    const cancelled = parseChatGoal({ conversationId, authoritative: true,
        goalProjection: { ...goal, active: { ...goal.active, status: 'cancelled' } } }, conversationId);
    assert.equal(chatActivityNeedsPolling(cancelled, [{ ...trace, status: 'running' }], false), true);
    assert.equal(chatActivityNeedsPolling(cancelled, [trace], false), true);
    assert.equal(chatActivityNeedsPolling(cancelled, [{ ...trace, status: 'cancelled', finishedAt: trace.startedAt }], false), false);
    assert.equal(chatActivityNeedsPolling(null, [trace], false), true);
    assert.equal(chatActivityNeedsPolling(cancelled, [], true), true);
});

test('activity URLs encode opaque conversation and parsers keep exact owner', () => {
    assert.equal(chatActivityUrl(conversationId, 'goal'), '/api/chat/conversations/conversation%20A%2F%3F/goal');
    assert.equal(chatActivityUrl(conversationId, 'traces'), '/api/chat/conversations/conversation%20A%2F%3F/traces?summaryOnly=1');
    assert.deepEqual(parseChatGoal({ conversationId, authoritative: true, goalProjection: goal }, conversationId),
        { canSubmit: false, canPause: true, canResume: false, canCancel: true, controlToken: 'core-token', id: 'goal-one', executionBlocker: null, revision: 4, objective: 'Ship the app', status: 'running', completedRequirements: 1, totalRequirements: 2, nextWakeAtEpochMs: null,
            requirements: [{ id: 'one', description: 'Build', status: 'completed' }, { id: 'two', description: 'Verify', status: 'pending' }] });
    assert.equal(parseChatGoal({ conversationId, authoritative: true, goalProjection: { revision: 5 } }, conversationId), null);
    assert.throws(() => parseChatGoal({ conversationId: 'other', authoritative: true, goalProjection: goal }, conversationId));
    assert.throws(() => parseChatGoal({ conversationId, authoritative: false, goalProjection: goal }, conversationId));
});

test('trace read model discards raw events and does not infer terminal state', () => {
    const parsed = parseChatTraces({ conversationId, traces: [trace] }, conversationId);
    assert.deepEqual(parsed, [{ traceId: 'trace-1', runId: 'run-1', status: 'waiting', lastSequence: 7, startedAt: trace.startedAt, finishedAt: null }]);
    assert.ok(!JSON.stringify(parsed).includes('SECRET_TOKEN'));
    assert.throws(() => parseChatTraces({ conversationId, traces: [trace, trace] }, conversationId));
    assert.throws(() => parseChatTraces({ conversationId: 'other', traces: [trace] }, conversationId));
    const older = { ...trace, traceId: 'trace-0', runId: 'run-0', startedAt: '2026-09-22T00:00:00Z' };
    assert.deepEqual(parseChatTraces({ conversationId, traces: [trace, older] }, conversationId).map(item => item.traceId), ['trace-0', 'trace-1']);
    const { events: _events, ...summary } = trace;
    assert.equal(parseChatTraces({ conversationId, traces: [summary] }, conversationId)[0].status, 'waiting');
});

test('goal actions follow Core capabilities independently of status and never manufacture progress', () => {
    const current = parseChatGoal({ conversationId, authoritative: true, goalProjection: goal }, conversationId)!;
    assert.deepEqual(goalAvailableActions(current), ['pause', 'cancel']);
    assert.deepEqual(goalAvailableActions({ ...current, status: 'paused', canPause:false, canResume:true }), ['resume', 'cancel']);
    assert.deepEqual(goalAvailableActions({ ...current, status: 'completed', canSubmit:true, canPause:false, canCancel:false }), ['start']);
    assert.deepEqual(goalAvailableActions({ ...current, status: 'unknown', canPause:false, canCancel:false }), []);
    assert.deepEqual(goalActionRequest('start', '  Review  ', null, 'start-op'), { operationId: 'start-op', action: 'start', objective: 'Review' });
    assert.deepEqual(goalActionRequest('resume', '', current, 'resume-op'), { operationId: 'resume-op', action: 'resume', expectedGoalId: 'goal-one', expectedControlToken: 'core-token' });
    assert.throws(() => goalActionRequest('resume'));
    assert.throws(() => goalActionRequest('start', ' '));
});

test('actual activity and goal Svelte component renders seven-language controls without exposing trace events', async () => {
    const server = await createServer({ server: { middlewareMode: true, ws: false }, appType: 'custom' });
    try {
        const { default: Component } = await server.ssrLoadModule('/src/components/ChatActivityGoal.svelte');
        const { locale, supportedLocales, t } = await server.ssrLoadModule('/src/lib/i18n.ts');
        const { render } = await server.ssrLoadModule('svelte/server');
        for (const language of supportedLocales) {
            locale.set(language);
            const html: string = render(Component, { props: { hostId: 'host', conversationId, generation: 0 } }).body;
            assert.ok(html.includes(t(language, 'chatActivity.title')));
            assert.ok(html.includes(t(language, 'chatActivity.goal')));
            assert.ok(html.includes(t(language, 'chatActivity.traces')));
            assert.ok(!html.includes('SECRET_TOKEN'));
            for (const status of ['running', 'paused', 'needs_user', 'completed', 'failed', 'cancelled']) {
                const key = `chatActivity.goal${status.charAt(0).toUpperCase()}${status.slice(1)}`;
                assert.notEqual(t(language, key), key);
            }
            for (const status of ['pending', 'running', 'waiting', 'succeeded', 'failed', 'cancelled', 'incomplete', 'interrupted']) {
                const key = `chatActivity.trace${status.charAt(0).toUpperCase()}${status.slice(1)}`;
                assert.notEqual(t(language, key), key);
            }
            assert.notEqual(t(language, 'chatActivity.invalidObjective'), 'chatActivity.invalidObjective');
        }
    } finally { await server.close(); }
});

test('queued Goal control waits for the exact durable operation outcome', () => {
    assert.equal(pendingGoalControl({conversationId, authoritative:false, controlPending:true, operationId:'op'}, conversationId), 'op');
    const projection = {conversationId, authoritative:true, goalProjection:{applied_operations:{}}};
    assert.equal(goalControlOutcome(projection, conversationId, 'op'), 'pending');
    for (const [rejection, expected] of [[null, 'applied'], ['stale goal', 'rejected']] as const) {
        assert.equal(goalControlOutcome({...projection, goalProjection:{applied_operations:{op:{goal_revision:8,rejection}}}}, conversationId, 'op'), expected);
    }
    assert.throws(() => pendingGoalControl({conversationId:'other', authoritative:false,controlPending:true,operationId:'op'}, conversationId));
});

test('Core execution blocker is explicit and arbitrary diagnostics are not displayed', () => {
    const blocked = {conversationId, authoritative:true, goalProjection:{...goal, can_resume:true, active:{...goal.active,
        status:'needs_user', execution_blocker:{code:'configuration_unavailable', message:'private diagnostic sentinel'}}}};
    const parsed = parseChatGoal(blocked, conversationId)!;
    assert.equal(parsed.executionBlocker, 'configuration_unavailable');
    assert.ok(!JSON.stringify(parsed).includes('private diagnostic sentinel'));
    assert.deepEqual(goalAvailableActions(parsed), ['pause', 'resume', 'cancel']);
    assert.throws(() => parseChatGoal({...blocked,goalProjection:{...goal,active:{...goal.active,execution_blocker:{code:'unknown',message:''}}}}, conversationId));
});

test('Goal controls consume the opaque Core token independently of audit revision', () => {
    const first = parseChatGoal({conversationId, authoritative: true, goalProjection: goal}, conversationId)!;
    const heartbeat = parseChatGoal({conversationId, authoritative: true, goalProjection: {...goal, revision: 99}}, conversationId)!;
    assert.deepEqual(goalActionRequest('pause', '', first, 'same-intent'), goalActionRequest('pause', '', heartbeat, 'same-intent'));
    const {control_token, ...missingToken} = goal;
    assert.throws(() => parseChatGoal({conversationId, authoritative:true, goalProjection:missingToken}, conversationId));
});


test('Goal request identity rejects malformed operations before durable dispatch', () => {
    for (const operationId of ['', ' trailing ', 'line\nbreak', 'x'.repeat(257)]) {
        assert.throws(() => goalActionRequest('start', 'Review', null, operationId));
    }
    const request = goalActionRequest('start', 'Review', null, 'fixed-operation');
    assert.deepEqual(request, { action: 'start', objective: 'Review', operationId: 'fixed-operation' });
});

 test('active Goal rejects missing Core capability fields instead of guessing from status', () => {
    for (const key of ['can_submit','can_pause','can_resume','can_cancel']) {
        const missing: Record<string, unknown> = {...goal}; delete missing[key];
        assert.throws(() => parseChatGoal({conversationId,authoritative:true,goalProjection:missing}, conversationId));
    }
});
