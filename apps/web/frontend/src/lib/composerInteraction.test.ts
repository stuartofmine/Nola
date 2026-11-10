import test from 'node:test';
import assert from 'node:assert/strict';
import { composerEnterSubmits } from './composerInteraction.ts';
const enter = { key: 'Enter', shiftKey: false, isComposing: false, keyCode: 13, repeat: false };
test('Enter submits once; newline and IME confirmation stay in the editor', () => {
    assert.equal(composerEnterSubmits(enter, false), true);
    for (const event of [{ ...enter, shiftKey: true }, { ...enter, isComposing: true },
        { ...enter, keyCode: 229 }, { ...enter, repeat: true }, { ...enter, key: 'a' }]) {
        assert.equal(composerEnterSubmits(event, false), false);
    }
    assert.equal(composerEnterSubmits(enter, true), false);
});
