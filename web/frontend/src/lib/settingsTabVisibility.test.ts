import test from 'node:test';
import assert from 'node:assert/strict';
import { visibleTabScrollLeft } from './settingsTabVisibility.ts';

test('settings tab rail scrolls only enough to reveal a clipped tab', () => {
    assert.equal(visibleTabScrollLeft(0, 16, 374, 40, 120), 0);
    assert.equal(visibleTabScrollLeft(300, 16, 374, 8, 90), 292);
    assert.equal(visibleTabScrollLeft(280, 16, 374, 350, 420), 326);
    assert.equal(visibleTabScrollLeft(5, 16, 374, 0, 100), 0);
});
