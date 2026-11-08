import test from 'node:test';
import assert from 'node:assert/strict';
import { readUiDraftState, saveUiDraftState } from './uiDraftSession.ts';

test('UI draft session isolates owners and survives renderer remount without aliasing', () => {
    const drafts = {field: {type: 'set' as const, instance_id: 'field', binding: 'value', value: '未提交'}};
    saveUiDraftState('authority-a/form', {drafts, revision: 2, unresolved: null});
    drafts.field.value = 'later mutation';
    assert.equal(readUiDraftState('authority-a/form')?.drafts.field.type, 'set');
    assert.deepEqual(readUiDraftState('authority-a/form')?.drafts.field, {
        type: 'set', instance_id: 'field', binding: 'value', value: '未提交',
    });
    assert.equal(readUiDraftState('authority-b/form'), undefined);
    saveUiDraftState('authority-a/form', {drafts: {}, revision: null, unresolved: null});
    assert.equal(readUiDraftState('authority-a/form'), undefined);
});
