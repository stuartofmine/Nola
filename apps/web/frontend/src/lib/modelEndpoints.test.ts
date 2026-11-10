import { test } from 'node:test';
import assert from 'node:assert/strict';
import { endpointSnapshot, endpointDirty, endpointValidationError, endpointRequest } from './modelEndpoints.ts';
import type { RemoteProfile } from './modelProfiles.ts';

function model(id: string, endpoint = 'provider/中文'): RemoteProfile {
    return { id, revision: 4, modelName: id, displayName: id, enabled: true, selected: false, priority: 50, order: 0,
        endpoint: { id: endpoint, name: '提供商', baseURL: 'https://example.com/v1', wireProtocol: 'responses', enabled: true, order: 0 },
        capabilities: ['chat'], parameters: { maxOutputTokens: 16384 }, reasoning: { control: 'effort', efforts: ['low', 'high'], budgets: [] } };
}

test('endpoint edits include every member revision and no other endpoint or credentials', () => {
    const snapshot = endpointSnapshot([model('a'), model('b'), model('foreign', 'other')], 'provider/中文');
    const draft = { ...snapshot.draft, name: ' 新名称 ', enabled: false };
    assert.equal(endpointDirty(snapshot, draft), true);
    assert.deepEqual(endpointRequest(snapshot, draft), {
        expectedRevisions: { a: 4, b: 4 }, name: '新名称', baseURL: 'https://example.com/v1', wireProtocol: 'responses', enabled: false,
    });
    assert.equal(snapshot.draft.name, '提供商');
    assert.equal(endpointDirty(snapshot, { ...snapshot.draft }), false);
});

test('endpoint editing holds a stable model contract until confirmed reload', () => {
    const original = model('a');
    const snapshot = endpointSnapshot([original], 'provider/中文');
    original.reasoning!.efforts!.push('medium');
    original.parameters.maxOutputTokens = 512;
    original.revision = 5;
    assert.deepEqual(snapshot.profiles[0].reasoning!.efforts, ['low', 'high']);
    assert.equal(snapshot.profiles[0].parameters.maxOutputTokens, 16384);
    assert.equal(snapshot.expectedRevisions.a, 4);
    assert.deepEqual(endpointValidationError(snapshot, { ...snapshot.draft, wireProtocol: 'anthropic' }), { code: 'modelValidation.endpointReasoning', params: { model: 'a' }, cause: { code: 'modelValidation.control' } });
    assert.equal(endpointValidationError(snapshot, { ...snapshot.draft, wireProtocol: 'chat_completions' })?.cause?.code, 'modelValidation.effortTiers');
});

test('invalid names, URL credentials, protocols and stale identities cannot be submitted', () => {
    const snapshot = endpointSnapshot([model('a')], 'provider/中文');
    for (const baseURL of ['file:///tmp/test', 'javascript:alert(1)', 'https://user:secret@example.com', 'https://example.com/#fragment', 'https://example.com/#', 'invalid']) {
        assert.ok(endpointValidationError(snapshot, { ...snapshot.draft, baseURL }), baseURL);
    }
    assert.ok(endpointValidationError(snapshot, { ...snapshot.draft, name: ' ' }));
    assert.ok(endpointValidationError(snapshot, { ...snapshot.draft, name: '模型'.repeat(100) }));
    assert.ok(endpointValidationError(snapshot, { ...snapshot.draft, wireProtocol: 'unknown' }));
    assert.throws(() => endpointSnapshot([model('a')], 'missing'));
    assert.throws(() => endpointSnapshot([{ ...model('a'), revision: NaN }], 'provider/中文'));
    assert.throws(() => endpointRequest(snapshot, { ...snapshot.draft, name: '' }));
});

test('opaque model IDs remain data, even for object prototype property names', () => {
    const snapshot = endpointSnapshot([model('__proto__'), model('constructor')], 'provider/中文');
    const payload = JSON.parse(JSON.stringify(endpointRequest(snapshot, snapshot.draft)));
    assert.equal(payload.expectedRevisions.__proto__, 4);
    assert.equal(payload.expectedRevisions.constructor, 4);
});
