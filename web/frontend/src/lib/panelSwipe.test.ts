import { test } from 'node:test';
import assert from 'node:assert/strict';
import { completesPanelSwipe, panelOutwardSign } from './panelSwipe.ts';

const start = {x: 200, y: 100, time: 1000};
test('drawer swipes mirror both sides in RTL and open in the opposite direction', () => {
    for (const rtl of [false, true]) for (const side of ['start', 'end'] as const) {
        const sign = panelOutwardSign(side, rtl);
        assert.equal(completesPanelSwipe(start, {x: 200 + 100 * sign, y: 110, time: 1300}, 390, sign), true);
        assert.equal(completesPanelSwipe(start, {x: 200 - 100 * sign, y: 110, time: 1300}, 390, sign), false);
        assert.equal(completesPanelSwipe(start, {x: 200 - 100 * sign, y: 110, time: 1300}, 390, -sign), true);
    }
});
test('scrolling, taps, diagonal drags, long selections and cancelled motion never dismiss drawers', () => {
    for (const end of [{x:220,y:100,time:1100}, {x:300,y:240,time:1200}, {x:300,y:100,time:1800}, {x:300,y:100,time:900}]) {
        assert.equal(completesPanelSwipe(start, end, 390, 1), false);
    }
    assert.equal(completesPanelSwipe(start, {x:300,y:100,time:1300}, 0, 1), false);
});
