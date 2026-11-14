import test from 'node:test';
import assert from 'node:assert/strict';
import { readUiDraftState, saveUiDraftState, draftStateFromAttempt } from './uiDraftSession.ts';

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

test('cold reload retains submitted edits after an unacknowledged attempt is rejected', () => {
    const attempt = {input: {identity: {owner: {authority_id: 'host', conversation_id: 'conversation',
        participant_id: 'assistant', context_id: 'conversation', originating_run_id: 'run',
        message_id: 'message', part_id: 'part'}, surface_id: 'form', generation: 1},
        expected_surface_revision: 4, action: null, edits: [{type: 'set' as const,
            instance_id: 'name', binding: 'value', value: '保留填写值 👩🏽‍💻'}]},
        requestId: 'same-request', submittedAtMs: 123};
    const restored = draftStateFromAttempt(attempt);
    restored.unresolved = null; // A definitive rejection permits editing again.
    assert.equal(restored.revision, 4);
    assert.deepEqual(restored.drafts[JSON.stringify(['name', 'value'])], attempt.input.edits[0]);
    assert.equal(attempt.requestId, 'same-request');
    attempt.input.edits[0].value = 'changed elsewhere';
    assert.equal((restored.drafts[JSON.stringify(['name', 'value'])] as {value: string}).value, '保留填写值 👩🏽‍💻');
});
