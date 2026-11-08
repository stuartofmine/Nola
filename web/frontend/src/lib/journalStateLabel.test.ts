import test from 'node:test';
import assert from 'node:assert/strict';
import { journalStateLabelKey, journalPartStateLabelKey } from './journalStateLabel.ts';
import { chatPageMessages } from './locales/chatPage.ts';

test('all Task lifecycle states have seven-language labels and sealed is not success', () => {
    for (const state of ['pending', 'queued', 'running', 'waiting', 'waiting_for_input', 'waiting_for_permission',
        'suspended', 'retry_waiting', 'sealed', 'completed', 'failed', 'cancelled']) {
        const key = journalStateLabelKey(state)!;
        assert.notEqual(key, 'chatPage.stateUnknown');
        for (const messages of Object.values(chatPageMessages)) {
            assert.ok(messages[key]?.trim());
            assert.notEqual(messages[key], key);
        }
    }
    assert.notEqual(journalStateLabelKey('sealed'), journalStateLabelKey('completed'));
    assert.equal(journalStateLabelKey('sealed'), 'chatPage.stateSealed');
});

test('typed Error Parts remain failures without a separate activity state', () => {
    assert.equal(journalPartStateLabelKey('error', undefined), 'chatPage.toolFailed');
    assert.equal(journalPartStateLabelKey('error', 'completed'), 'chatPage.toolFailed');
    assert.equal(journalPartStateLabelKey('step', undefined), null);
    assert.equal(journalPartStateLabelKey('step', 'sealed'), 'chatPage.stateSealed');
    assert.equal(journalPartStateLabelKey('future_kind', 'future_state'), 'chatPage.stateUnknown');
});

test('missing and unknown states do not become completed or expose raw protocol labels', () => {
    for (const value of [undefined, null, 1, {}, '']) assert.equal(journalStateLabelKey(value), null);
    for (const value of ['future_state', '__proto__', 'constructor'])
        assert.equal(journalStateLabelKey(value), 'chatPage.stateUnknown');
});
