import { test } from 'node:test';
import assert from 'node:assert/strict';
import { localModelEditorSnapshot } from './localModelEditor.ts';

test('reload replaces both draft and revision without mutating the catalog', () => {
    const current = { id: 'model', installed: true, configurationRevision: 8, parameters: { temperature: .123456, topP: .9 } };
    const result = localModelEditorSnapshot([current], 'model');
    assert.equal(result.model.configurationRevision, 8);
    assert.deepEqual(result.parameters, current.parameters);
    result.parameters.temperature = 0;
    assert.equal(current.parameters.temperature, .123456);
});

test('missing, removed and invalid revision models cannot become a new editor baseline', () => {
    const current = { id: 'model', installed: true, configurationRevision: 8, parameters: { temperature: .7 } };
    assert.throws(() => localModelEditorSnapshot([], 'model'));
    assert.throws(() => localModelEditorSnapshot([current], 'other'));
    assert.throws(() => localModelEditorSnapshot([{ ...current, installed: false }], 'model'));
    for (const configurationRevision of [-1, NaN, .5, Number.MAX_SAFE_INTEGER + 1]) {
        assert.throws(() => localModelEditorSnapshot([{ ...current, configurationRevision }], 'model'));
    }
});
