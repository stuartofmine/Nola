import test from 'node:test';
import assert from 'node:assert/strict';
import { freezeHostToolPlan, hostToolPlanReference } from './hostToolPlan.ts';

const fixture = (now = Date.now()) => ({ source_host_id: 'linux-host', result: {
    schema_version: 1, reference: { manifest_id: 'host-issued', host_id: 'linux-host', client_id: 'browser', session_id: 'conversation', catalog_revision: 'a'.repeat(64) },
    issued_at_ms: now - 1, expires_at_ms: now + 60_000,
    tools: [{tool_id: 'mcp_custom', route: 'agent_host', schema: {type: 'function', function: {name: 'mcp_custom', description: 'Full Host schema', parameters: {type: 'object', required: ['query']}}}}],
} });

test('preserves Host reference without client platform inference and rejects wrong owners', () => {
    const value = fixture(1000);
    assert.deepEqual(hostToolPlanReference(value, 'linux-host', 'browser', 'conversation'), value.result.reference);
    for (const owner of [['other', 'browser', 'conversation'], ['linux-host', 'other', 'conversation'], ['linux-host', 'browser', 'other']])
        assert.throws(() => hostToolPlanReference(value, owner[0], owner[1], owner[2]));
    const duplicate = fixture(1000); duplicate.result.tools.push(duplicate.result.tools[0]);
    assert.throws(() => hostToolPlanReference(duplicate, 'linux-host', 'browser', 'conversation'));
    const changed = fixture(1000); changed.result.tools[0].schema.function.name = 'other';
    assert.throws(() => hostToolPlanReference(changed, 'linux-host', 'browser', 'conversation'));
});

test('freezes issued reference into the request before submission hashing without changing device advertisement', async () => {
    const request = { expected_host_id: 'linux-host', client_id: 'browser', conversationId: 'conversation', messages: [], device_bridge: {protocol_version: 1} };
    const value = fixture();
    const frozen = await freezeHostToolPlan(request, async (path, options) => {
        assert.equal(path, '/api/agent/tool-plan/prepare');
        assert.deepEqual(JSON.parse(String(options.body)), {expected_host_id: 'linux-host', client_id: 'browser', session_id: 'conversation'});
        return value;
    });
    assert.deepEqual(frozen, {...request, tool_plan_ref: value.result.reference});
    assert.equal(Object.hasOwn(request, 'tool_plan_ref'), false);
    await assert.rejects(freezeHostToolPlan(frozen, async () => value));
});

test('rejects numeric coercion, blank or oversized identity and invalid catalog time bounds', () => {
    for (const patch of [{legacy: true}, {schema_version: '1'}, {schema_version: true}, {issued_at_ms: '999'}, {expires_at_ms: '61000'},
        {issued_at_ms: 0}, {issued_at_ms: 61000}, {expires_at_ms: 999}, {expires_at_ms: 301000}, {expires_at_ms: Number.MAX_SAFE_INTEGER + 1}]) {
        const value = fixture(1000);
        assert.throws(() => hostToolPlanReference({...value, result: {...value.result, ...patch}}, 'linux-host', 'browser', 'conversation'));
    }
    for (const manifest_id of [' \n', 'x'.repeat(513), 'id\u0000suffix', 'id\u0085suffix', 123]) {
        const value = fixture(1000);
        assert.throws(() => hostToolPlanReference({...value, result: {...value.result, reference: {...value.result.reference, manifest_id}}}, 'linux-host', 'browser', 'conversation'));
    }
    const blank = fixture(1000); blank.result.tools[0].tool_id = ' '; blank.result.tools[0].schema.function.name = ' ';
    assert.throws(() => hostToolPlanReference(blank, 'linux-host', 'browser', 'conversation'));
    const unknown = fixture(1000);
    assert.throws(() => hostToolPlanReference({...unknown, result: {...unknown.result, reference: {...unknown.result.reference, legacy: true}}}, 'linux-host', 'browser', 'conversation'));
    assert.throws(() => hostToolPlanReference({...unknown, result: {...unknown.result, tools: [{...unknown.result.tools[0], legacy: true}]}}, 'linux-host', 'browser', 'conversation'));
});

test('in-flight preparation cannot bind another conversation or mutated draft to the original reference', async () => {
    const request = {expected_host_id: 'linux-host', client_id: 'browser', conversationId: 'conversation',
        messages: [{role: 'user', content: 'original'}], configuration: {model: 'original-model'}};
    const original = structuredClone(request);
    const value = fixture();
    let resolve!: (value: unknown) => void;
    const response = new Promise<unknown>(done => { resolve = done; });
    const pending = freezeHostToolPlan(request, async () => response);
    request.conversationId = 'another-conversation';
    request.messages[0].content = 'changed';
    request.configuration.model = 'changed-model';
    resolve(value);
    const frozen = await pending;
    assert.deepEqual(frozen, {...original, tool_plan_ref: value.result.reference});
    request.messages.push({role: 'user', content: 'later'});
    assert.deepEqual(frozen.messages, original.messages);
});

test('remote catalog structure does not depend on the browser clock', () => {
    for (const hostTime of [1_000, 4_000_000_000_000]) {
        const value = fixture(hostTime);
        assert.deepEqual(hostToolPlanReference(value, 'linux-host', 'browser', 'conversation'), value.result.reference);
    }
});
