const test = require('node:test');
const assert = require('node:assert/strict');
const { explicitProfile, successfulCalls, sealedOutcome, retryableObservationRace } = require('./a2ui_long_task_oracle.cjs');
test('model selection requires explicit enabled chat/tools profile', () => {
    const profile = { id: 'qwen', enabled: true, endpoint: { enabled: true }, capabilities: ['chat'], executionCapabilityStatus: 'tools_enabled' };
    assert.deepEqual(explicitProfile([profile], 'qwen'), { selectedModelSource: 'cloud', selectedModelId: 'qwen' });
    for (const change of [{ enabled: false }, { endpoint: { enabled: false } }, { executionCapabilityStatus: 'text_only' }, { capabilities: [] }]) {
        assert.throws(() => explicitProfile([{ ...profile, ...change }], 'qwen'));
    }
    assert.throws(() => explicitProfile([profile], 'unknown'));
});
test('successful publication counts exclude failures and other Runs', () => {
    const part = (id, state) => ({ kind: 'tool', capability_id: 'render_ui', part_id: id, state });
    assert.deepEqual(successfulCalls({ messages: [
        { message_id: 'run:assistant', parts: [part('ok', 'completed'), part('schema-retry', 'failed'), part('pending', 'running')] },
        { message_id: 'other:assistant', parts: [part('foreign', 'completed')] },
    ] }, 'run'), ['ok']);
});
test('UI completion cannot replace authoritative exact Run terminal seal', () => {
    assert.equal(sealedOutcome({ status: 'sealed' }, 'cid', 'run'), null);
    const observation = { status: 'sealed', terminal_seal: { identity: { conversation_id: 'cid', run_id: 'run', assistant_message_id: 'run:assistant' }, result: { conversation_id: 'cid', run_id: 'run', value: { kind: 'cancelled' } } } };
    assert.equal(sealedOutcome(observation, 'cid', 'run'), 'cancelled');
    assert.throws(() => sealedOutcome(observation, 'cid', 'other'));
    assert.throws(() => sealedOutcome({ ...observation, status: 'running' }, 'cid', 'run'));
});

test('only explicit read-only observation races can be retried with the same identity', () => {
    const body = {code:'observation_unavailable',error:'execution changed during observation'};
    assert.equal(retryableObservationRace('/api/chat/agent/v2/observe','POST',409,body),true);
    for (const path of ['/api/chat/agent/v2/resume','/api/chat/agent/v2/cancel','/api/chat/agent/v2/stream'])
        assert.equal(retryableObservationRace(path,'POST',409,body),false);
    for (const error of ['observation identity mismatch','root assistant mismatch','Run has no source submission owner for remote recovery','observation response exceeds budget'])
        assert.equal(retryableObservationRace('/api/chat/agent/v2/observe','POST',409,{...body,error}),false);
    assert.equal(retryableObservationRace('/api/chat/agent/v2/observe','POST',500,body),false);
});
