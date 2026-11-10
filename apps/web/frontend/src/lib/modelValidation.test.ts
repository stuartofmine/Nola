import { test } from 'node:test';
import assert from 'node:assert/strict';
import { modelValidationMessages } from './locales/modelValidation.ts';
import { ModelValidationError, modelValidationText, type ModelValidationIssue } from './modelValidation.ts';
import { reasoningValidationError, type ReasoningConfiguration, type RemoteProfile } from './modelProfiles.ts';
import { endpointSnapshot, endpointValidationError, endpointRequest } from './modelEndpoints.ts';
import { localModelEditorSnapshot } from './localModelEditor.ts';

test('reasoning checks return stable codes independently of language', () => {
    const base: ReasoningConfiguration = { control: 'effort', efforts: ['low'], budgets: [] };
    const cases: [Partial<ReasoningConfiguration>, string, string][] = [
        [{}, 'invalid', 'protocol'], [{ efforts: ['unknown'] }, 'responses', 'efforts'],
        [{ budgets: [-1], control: 'automatic' }, 'responses', 'budgets'],
        [{ budgets: [NaN] }, 'responses', 'budgets'], [{}, 'anthropic', 'control'],
        [{ control: 'budget' }, 'responses', 'control'],
        [{ control: 'budget', budgets: [1024, 1024, 2048] }, 'anthropic', 'budgetTiers'],
        [{ efforts: [] }, 'responses', 'effortRequired'], [{}, 'chat_completions', 'effortTiers'],
        [{ defaultPreference: 'high' }, 'responses', 'default'],
    ];
    for (const [change, protocol, code] of cases) assert.equal(reasoningValidationError({ ...base, ...change }, protocol, 16384)?.code, `modelValidation.${code}`);
    assert.deepEqual(reasoningValidationError({ ...base, control: 'budget' }, 'anthropic', 16384)?.params, { minimum: 1024 });
});

test('endpoint and local model failures retain typed issues rather than translated exceptions', () => {
    const model: RemoteProfile = { id: 'model', displayName: 'Name', modelName: 'model', revision: 1, enabled: true, selected: false, priority: 0, order: 0, capabilities: [], parameters: {}, endpoint: { id: 'endpoint', name: 'Name', baseURL: 'https://example.com', wireProtocol: 'responses', enabled: true, order: 0 } };
    const snapshot = endpointSnapshot([model], 'endpoint');
    for (const [draft, code] of [
        [{ name: '' }, 'nameRequired'], [{ name: '界'.repeat(171) }, 'nameLength'],
        [{ baseURL: 'https://u:p@example.com' }, 'url'], [{ wireProtocol: 'unknown' }, 'selectProtocol'],
    ] as const) assert.equal(endpointValidationError(snapshot, { ...snapshot.draft, ...draft })?.code, `modelValidation.${code}`);
    assert.deepEqual(endpointValidationError(snapshot, { ...snapshot.draft, name: 'x'.repeat(513) })?.params, { maximum: 512 });
    const throws = (fn: () => unknown, code: string) => assert.throws(fn, error => error instanceof ModelValidationError && error.issue.code === `modelValidation.${code}` && error.message === error.issue.code);
    throws(() => endpointSnapshot([], 'endpoint'), 'endpointMissing');
    throws(() => endpointSnapshot([{ ...model, revision: -1 }], 'endpoint'), 'endpointRevision');
    throws(() => endpointRequest(snapshot, { ...snapshot.draft, name: '' }), 'nameRequired');
    throws(() => localModelEditorSnapshot([], 'model'), 'localMissing');
    throws(() => localModelEditorSnapshot([{ id: 'model', installed: true, configurationRevision: -1, parameters: {} }], 'model'), 'localRevision');
});

test('all seven validation catalogs are complete and preserve placeholders', () => {
    const keys = Object.keys(modelValidationMessages.en).sort();
    const placeholders = (text: string) => [...text.matchAll(/\{\w+\}/g)].map(match => match[0]).sort();
    assert.equal(Object.keys(modelValidationMessages).length, 7);
    for (const messages of Object.values(modelValidationMessages)) {
        assert.deepEqual(Object.keys(messages).sort(), keys);
        for (const key of keys as (keyof typeof messages)[]) {
            assert.ok(messages[key].trim());
            assert.deepEqual(placeholders(messages[key]), placeholders(modelValidationMessages.en[key]));
        }
    }
});

test('stored nested issues retranslate with locale and interpolate parameters only once', () => {
    const issue: ModelValidationIssue = { code: 'modelValidation.endpointReasoning', params: { model: '<model>{reason}' }, cause: { code: 'modelValidation.budgetTiers', params: { minimum: 1024 } } };
    for (const language of Object.keys(modelValidationMessages)) {
        const text = modelValidationText(language, issue);
        assert.ok(text.includes('<model>{reason}'));
        assert.ok(text.includes('1024'));
        assert.ok(!text.includes('{minimum}'));
        assert.ok(!text.includes('{model}'));
    }
    assert.notEqual(modelValidationText('en', issue), modelValidationText('ja', issue));
    assert.equal(modelValidationText('unknown', issue), modelValidationText('en', issue));
    assert.equal(modelValidationText('de', 'opaque server message'), 'opaque server message');
    assert.ok(modelValidationText('en', { code: 'modelValidation.nameLength', params: { maximum: 512 } }).includes('512'));
});
