import {test} from 'node:test';
import assert from 'node:assert/strict';
import {isDialogBackdrop} from './dialogBackdrop.ts';

test('drawer backdrop excludes child controls and empty space inside panel', () => {
    const bounds = {left: 100, right: 400, top: 0, bottom: 800};
    assert.equal(isDialogBackdrop(true, 50, 300, bounds), true);
    assert.equal(isDialogBackdrop(false, 50, 300, bounds), false);
    assert.equal(isDialogBackdrop(true, 150, 300, bounds), false);
    assert.equal(isDialogBackdrop(true, 100, 0, bounds), false);
    assert.equal(isDialogBackdrop(true, 150, 801, bounds), true);
});
