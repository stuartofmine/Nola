import { ApiError } from './api.ts';
import test from 'node:test';
import assert from 'node:assert/strict';
import { parseRunObservation, parseGoalControl, RunObserver } from './runObservation.ts';

const owner = { conversationId: 'conversation', hostId: 'host', clientId: 'client' };
const result = { protocol_version: 1, conversation_id: 'conversation', client_id: 'client', run_id: 'run', assistant_message_id: 'assistant',
    status: 'running', worker: { state: 'absent', generation: null }, recovery: 'resume_required',
    admission: null, terminal_seal: null, permissions: [], can_submit: false, submission_receipt: null, goal_execution: null, context_usage: null };
test('Room observation exposes only the exact member execution permission', async () => {
    const roomOwner = { ...owner, clientId: 'room:room-a' };
    const permission = { protocol_version: 4, allow_conversation: true, nonce: 'nonce', request_id: 'request',
        host_id: owner.hostId, client_id: roomOwner.clientId, session_id: owner.conversationId,
        canonical_run_id: 'run', expires_at: Date.now() / 1000 + 60, safe_review_text: 'Write the selected file' };
    const observation = { ...result, client_id: roomOwner.clientId, recovery: 'observe',
        worker: { state: 'running', generation: '1' }, permissions: [permission] };
    const observer = new RunObserver();
    await observer.run(roomOwner, async body => {
        assert.equal(body.operation, 'observe');
        assert.equal(body.expected_run_id, 'run');
        assert.equal(body.client_id, 'room:room-a');
        return { source_host_id: 'host', conversation_id: owner.conversationId, client_id: roomOwner.clientId, goal_control: null, result: observation };
    }, async value => {
        assert.equal(value?.permissions[0]?.safe_review_text, 'Write the selected file');
        return false;
    }, () => assert.fail('Exact room permission was rejected'), { run_id: 'run', assistant_message_id: 'assistant' });
    for (const field of ['host_id', 'client_id', 'session_id', 'canonical_run_id']) {
        assert.throws(() => parseRunObservation({ source_host_id: 'host', conversation_id: owner.conversationId, client_id: owner.clientId, goal_control: null, result: {
            ...observation, permissions: [{ ...permission, [field]: 'other-owner' }]
        } }, roomOwner, { run_id: 'run', assistant_message_id: 'assistant' }));
    }
});
test('absence requests explicit recovery and never permits new input', () => {
    const value = parseRunObservation({ source_host_id: 'host', conversation_id: owner.conversationId, client_id: owner.clientId, goal_control: null, result }, owner)!;
    assert.equal(value.can_submit, false);
    assert.equal(value.recovery, 'resume_required');
    assert.throws(() => parseRunObservation({ source_host_id: 'foreign', result }, owner));
    assert.throws(() => parseRunObservation({ source_host_id: 'host', conversation_id: owner.conversationId, client_id: owner.clientId, goal_control: null, result }, owner, { run_id: 'other', assistant_message_id: 'assistant' }));
});
test('unknown generation cannot replay a permission nonce', () => {
    assert.throws(() => parseRunObservation({ source_host_id: 'host', conversation_id: owner.conversationId, client_id: owner.clientId, goal_control: null, result: { ...result,
        worker: { state: 'unknown', generation: '9007199254740993' }, permissions: [{ protocol_version: 4, allow_conversation: true,
            nonce: 'nonce', request_id: 'request', host_id: 'host', client_id: 'client', session_id: 'conversation',
            canonical_run_id: 'run', expires_at: Date.now() / 1000 + 60 }] } }, owner));
});
test('stop discards a late discovery and never submits or resumes', async () => {
    const observer = new RunObserver();
    let finish!: (value: unknown) => void;
    const calls: Record<string, unknown>[] = [];
    let updates = 0;
    const pending = observer.run(owner, body => { calls.push(body); return new Promise(resolve => { finish = resolve; }); },
        async () => { updates++; return true; }, () => assert.fail('cancelled observation failed'));
    await Promise.resolve();
    observer.stop(); finish({ source_host_id: 'host', conversation_id: owner.conversationId, client_id: owner.clientId, goal_control: null, result }); await pending;
    assert.equal(updates, 0);
    assert.deepEqual(calls.map(call => call.operation), ['discover']);
});

test('unresolved submission prevents Run discovery and cancellation cannot dispatch after admission wait', async () => {
    const observer = new RunObserver();
    let release!: (ready: boolean) => void;
    let reads = 0;
    const pending = observer.run(owner, async () => { reads++; return { source_host_id: 'host', conversation_id: owner.conversationId, client_id: owner.clientId, goal_control: null, result }; },
        async () => assert.fail('Unresolved submission produced Run authority'), () => assert.fail('Unexpected error'),
        undefined, () => new Promise(resolve => { release = resolve; }));
    assert.equal(reads, 0);
    observer.stop(); release(true); await pending;
    assert.equal(reads, 0);
});

test('source receipt reconciliation admits the original exact Run before the first observer read', async () => {
    const observer = new RunObserver();
    const calls: Record<string, unknown>[] = [];
    let admitted = false;
    await observer.run(owner, async body => { calls.push(body); return { source_host_id: 'host', conversation_id: owner.conversationId, client_id: owner.clientId, goal_control: null, result }; },
        async value => { assert.equal(value?.run_id, 'run'); return false; }, () => assert.fail('Unexpected error'),
        undefined, async () => { admitted = true; return true; },
        () => admitted ? { run_id: 'run', assistant_message_id: 'assistant' } : undefined);
    assert.deepEqual(calls, [{ operation: 'observe', conversation_id: 'conversation', expected_host_id: 'host', client_id: 'client',
        expected_run_id: 'run', assistant_message_id: 'assistant' }]);
    assert.throws(() => parseRunObservation({ source_host_id: 'host', conversation_id: owner.conversationId, client_id: owner.clientId, goal_control: null, result: { ...result, submission_receipt: undefined } }, owner));
});

test('a temporarily unavailable terminal manifest is observed again for the same Run without resubmission', async () => {
    const observer = new RunObserver();
    const calls: Record<string, unknown>[] = [];
    let materializations = 0;
    await observer.run(owner, async body => {
        calls.push(body);
        return { source_host_id: 'host', conversation_id: owner.conversationId, client_id: owner.clientId, goal_control: null, result };
    }, async value => {
        assert.equal(value?.run_id, 'run');
        if (++materializations === 1) throw new Error('Terminal manifest is not yet readable');
        return false;
    }, () => assert.fail('Transient materialization failure exhausted observation'),
    { run_id: 'run', assistant_message_id: 'assistant' });
    assert.equal(materializations, 2);
    assert.deepEqual(calls.map(call => [call.operation, call.expected_run_id]), [
        ['observe', 'run'], ['observe', 'run']
    ]);
});

 test('Goal observation binds an exact Core execution and cannot borrow source submission authority', () => {
    const goal = { goal_id: 'goal', contract_id: 'contract', run_id: 'run', control_token: 'core-token' };
    const observed = { ...result, goal_execution: goal };
    assert.deepEqual(parseRunObservation({ source_host_id: 'host', conversation_id: owner.conversationId, client_id: owner.clientId, goal_control: null, result: observed }, owner)?.goal_execution, goal);
    for (const changed of [{ ...goal, run_id: 'other' }, { ...goal, contract_id: '' }, { ...goal, control_token: '' }]) {
        assert.throws(() => parseRunObservation({ source_host_id: 'host', conversation_id: owner.conversationId, client_id: owner.clientId, goal_control: null, result: { ...observed, goal_execution: changed } }, owner));
    }
    assert.throws(() => parseRunObservation({ source_host_id: 'host', conversation_id: owner.conversationId, client_id: owner.clientId, goal_control: null, result: { ...observed, submission_receipt: {} } }, owner));
    const { goal_execution, ...missing } = observed;
    assert.throws(() => parseRunObservation({ source_host_id: 'host', conversation_id: owner.conversationId, client_id: owner.clientId, goal_control: null, result: missing }, owner));
});

test('Goal between rounds retains Core admission control without inventing a Run', () => {
    const goal = { goal_id: 'goal', control_token: 'core-token', status: 'running', can_submit: false, can_pause: true, can_resume: false, can_cancel: true };
    assert.deepEqual(parseGoalControl({ source_host_id: 'host', conversation_id: owner.conversationId, client_id: owner.clientId, goal_control: goal, result: null }, owner), goal);
    assert.throws(() => parseGoalControl({ source_host_id: 'host', conversation_id: owner.conversationId, client_id: owner.clientId, goal_control: {...goal, can_submit: true} }, owner));
    assert.throws(() => parseGoalControl({ source_host_id: 'host' }, owner));
    assert.throws(() => parseGoalControl({ source_host_id: 'other', goal_control: goal }, owner));
    const {can_pause, ...missingPause} = goal;
    assert.throws(() => parseGoalControl({source_host_id:'host',conversation_id:owner.conversationId,client_id:owner.clientId,goal_control:missingPause}, owner));
});

 test('unconfirmed control identity fails closed immediately without retrying or accepting idle', async () => {
    const observer = new RunObserver(); let reads = 0; let rejected: unknown;
    await observer.run(owner, async () => { reads++; throw new ApiError(409, 'unconfirmed', 'run_control_unconfirmed'); },
        async () => { assert.fail('must not accept unconfirmed authority'); }, error => { rejected = error; });
    assert.equal(reads, 1); assert.ok(rejected instanceof ApiError);
 });


test('idle Goal observation cannot unlock a different conversation or client', () => {
    const envelope = { source_host_id: owner.hostId, conversation_id: owner.conversationId,
        client_id: owner.clientId, goal_control: null, result: null };
    assert.equal(parseRunObservation(envelope, owner), null);
    assert.equal(parseGoalControl(envelope, owner), null);
    for (const changed of [{ ...envelope, conversation_id: 'other' }, { ...envelope, client_id: 'other' }]) {
        assert.throws(() => parseRunObservation(changed, owner));
        assert.throws(() => parseGoalControl(changed, owner));
    }
});

test('task-owned observation is read-only and rejects control authority or foreign lineage', () => {
    const task = {kind:'task_child',parent_conversation_id:'parent',parent_run_id:'parent-run',task_id:'task',child_conversation_id:'conversation'};
    const wrap = (overrides:Record<string,unknown>={}) => ({source_host_id:'host',conversation_id:'conversation',client_id:'client',goal_control:null,
        result:{...result,recovery:'observe',task_execution:task,...overrides}});
    assert.equal(parseRunObservation(wrap(),owner)?.task_execution?.task_id,'task');
    for (const overrides of [{can_submit:true},{recovery:'resume_required'},{submission_receipt:{}},
        {permissions:[{}]},{task_execution:{...task,child_conversation_id:'foreign'}},
        {task_execution:{...task,parent_conversation_id:'conversation'}}]) assert.throws(()=>parseRunObservation(wrap(overrides),owner));
});
