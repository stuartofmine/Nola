import test from 'node:test';
import assert from 'node:assert/strict';
import { ChatScrollOwnership, chatNearBottom } from './chatScroll.ts';

const metrics = (scrollTop: number, scrollHeight = 1000, clientHeight = 200) => ({ scrollTop, scrollHeight, clientHeight });

test('gesture end preserves last user observation across delayed reflow', () => {
    const scroll = new ChatScrollOwnership(); scroll.reset('a');
    scroll.beginUserScroll(scroll.token);
    scroll.observe(scroll.token, metrics(800), 'user');
    scroll.endUserScroll(scroll.token, metrics(800, 1600));
    assert.equal(scroll.followsLatest, true);
    assert.equal(scroll.contentChanged(scroll.token, metrics(800, 1600)), 1400);
    scroll.beginUserScroll(scroll.token);
    scroll.observe(scroll.token, metrics(300, 1600), 'user');
    scroll.endUserScroll(scroll.token, metrics(300, 500));
    assert.equal(scroll.followsLatest, false);
    scroll.returnToLatest(scroll.token);
    scroll.endUserScroll(scroll.token, metrics(300, 1600));
    assert.equal(scroll.followsLatest, true);
});

test('captured gesture timeout cannot end a later gesture or change a switched session', () => {
    const scroll = new ChatScrollOwnership(); scroll.reset('a');
    scroll.beginUserScroll(scroll.token);
    const timeout = scroll.token;
    scroll.observe(scroll.token, metrics(300), 'user');
    scroll.endUserScroll(timeout, metrics(800));
    assert.equal(scroll.followsLatest, false);
    assert.equal(scroll.contentChanged(scroll.token, metrics(800)), null);
    scroll.reset('b');
    scroll.endUserScroll(timeout, metrics(300));
    assert.equal(scroll.followsLatest, true);
    assert.equal(scroll.contentChanged(scroll.token, metrics(300)), 800);
});

test('new content follows owned latest even when layout moves the bottom away', () => {
    const scroll = new ChatScrollOwnership();
    const token = scroll.reset('a');
    scroll.observe(token, metrics(800, 1500), 'programmatic');
    assert.equal(scroll.followsLatest, true);
    assert.equal(scroll.contentChanged(token, metrics(800, 1500)), 1300);
});

test('user scroll away releases follow; programmatic bottom never reacquires it', () => {
    const scroll = new ChatScrollOwnership(); scroll.reset('a');
    scroll.observe(scroll.token, metrics(300), 'user');
    assert.equal(scroll.contentChanged(scroll.token, metrics(300, 1500)), null);
    scroll.observe(scroll.token, metrics(1300, 1500), 'programmatic');
    assert.equal(scroll.followsLatest, false);
    scroll.observe(scroll.token, metrics(1280, 1500), 'user');
    assert.equal(scroll.followsLatest, true);
    assert.equal(scroll.contentChanged(scroll.token, metrics(1280, 1600)), 1400);
});

test('active user gestures invalidate pending layout and suppress follow until ended', () => {
    const scroll = new ChatScrollOwnership(); const queued = scroll.reset('a');
    assert.equal(scroll.beginUserScroll(queued), true);
    assert.equal(scroll.contentChanged(queued, metrics(800)), null);
    scroll.observe(scroll.token, metrics(800), 'user');
    assert.equal(scroll.contentChanged(scroll.token, metrics(800, 1200)), null);
    scroll.endUserScroll(scroll.token, metrics(800));
    assert.equal(scroll.contentChanged(scroll.token, metrics(800, 1200)), 1000);
});

test('history prepend restores a stable visible anchor, excluding concurrent appended height', () => {
    const scroll = new ChatScrollOwnership(); scroll.reset('a');
    const anchor = scroll.beginHistoryPrepend(scroll.token, metrics(300), { id: 'message', contentTop: 280 })!;
    assert.equal(scroll.contentChanged(scroll.token, metrics(300, 2000)), null);
    assert.equal(scroll.restoreHistoryAnchor(anchor, metrics(300, 2000), { id: 'message', contentTop: 680 }), 700);
    assert.equal(scroll.restoreHistoryAnchor(anchor, metrics(300, 2000), { id: 'message', contentTop: 680 }), null);
    assert.equal(scroll.followsLatest, false);
});

test('user action wins over pending history restore or earlier explicit latest command', () => {
    const scroll = new ChatScrollOwnership(); scroll.reset('a');
    const anchor = scroll.beginHistoryPrepend(scroll.token, metrics(300), { id: 'message', contentTop: 280 })!;
    const latest = scroll.returnToLatest(scroll.token)!;
    assert.equal(scroll.restoreHistoryAnchor(anchor, metrics(300), { id: 'message', contentTop: 280 }), null);
    scroll.observe(scroll.token, metrics(100), 'user');
    assert.equal(scroll.contentChanged(latest, metrics(100)), null);
});

test('conversation changes and same-ID reopen reject stale measurements and gestures', () => {
    const scroll = new ChatScrollOwnership(); const old = scroll.reset('a');
    scroll.reset('b'); scroll.reset('a');
    assert.equal(scroll.beginUserScroll(old), false);
    scroll.observe(old, metrics(100), 'user');
    assert.equal(scroll.returnToLatest(old), null);
    assert.equal(scroll.contentChanged(old, metrics(100)), null);
    assert.equal(scroll.followsLatest, true);
});

test('missing history anchor does not invent a bottom target', () => {
    const scroll = new ChatScrollOwnership(); scroll.reset('a');
    const anchor = scroll.beginHistoryPrepend(scroll.token, metrics(300), { id: 'message', contentTop: 280 })!;
    assert.equal(scroll.restoreHistoryAnchor(anchor, metrics(300), null), null);
    assert.equal(scroll.followsLatest, false);
});

test('threshold, short content, overscroll, and invalid geometry are handled safely', () => {
    assert.equal(chatNearBottom(metrics(776)), true);
    assert.equal(chatNearBottom(metrics(775)), false);
    assert.equal(chatNearBottom(metrics(-20, 100)), true);
    assert.equal(chatNearBottom(metrics(820)), true);
    assert.equal(chatNearBottom(metrics(NaN)), false);
    assert.equal(chatNearBottom(metrics(0, -1)), false);
    assert.throws(() => new ChatScrollOwnership(Infinity));
    const scroll = new ChatScrollOwnership(); scroll.reset('a');
    assert.equal(scroll.contentChanged(scroll.token, metrics(0, Infinity)), null);
    scroll.observe(scroll.token, metrics(NaN), 'user');
    assert.equal(scroll.followsLatest, true);
    assert.equal(scroll.contentChanged(scroll.token, metrics(0, 100)), 0);
});

test('conversation reading restoration uses a stable anchor and rejects a later user gesture', () => {
    const scroll = new ChatScrollOwnership();
    const position = {followsLatest: false, scrollTop: 300, anchorId: 'message-A', viewportOffset: 20};
    const token = scroll.restoreIntent(scroll.reset('A'), position)!;
    assert.equal(scroll.followsLatest, false);
    assert.equal(scroll.restoredPosition(token, position, {scrollTop: 0, scrollHeight: 2200, clientHeight: 500}, {id: 'message-A', contentTop: 920}), 900);
    scroll.beginUserScroll(token);
    assert.equal(scroll.restoredPosition(token, position, {scrollTop: 0, scrollHeight: 2200, clientHeight: 500}, null), null);
    const other = scroll.reset('B');
    assert.equal(scroll.restoredPosition(token, position, {scrollTop: 0, scrollHeight: 2200, clientHeight: 500}, null), null);
    const latest = {...position, followsLatest: true};
    const restored = scroll.restoreIntent(other, latest)!;
    assert.equal(scroll.restoredPosition(restored, latest, {scrollTop: 0, scrollHeight: 2200, clientHeight: 500}, null), 1700);
});
