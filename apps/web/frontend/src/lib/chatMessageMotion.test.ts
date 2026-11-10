import test from 'node:test';
import assert from 'node:assert/strict';
import { appendedMessageIds } from './chatMessageMotion.ts';

test('only animates newly appended messages after an established timeline', () => {
    assert.deepEqual(appendedMessageIds(['a', 'b'], ['a', 'b', 'c', 'd']), ['c', 'd']);
    assert.deepEqual(appendedMessageIds([], ['first-user-message'], false, true), ['first-user-message']);
});

test('does not animate hydration, history prepend, replacement, or removal', () => {
    assert.deepEqual(appendedMessageIds([], ['a', 'b']), []);
    assert.deepEqual(appendedMessageIds(['b', 'c'], ['a', 'b', 'c'], true), []);
    assert.deepEqual(appendedMessageIds(['a', 'b'], ['x', 'a', 'b', 'c']), []);
    assert.deepEqual(appendedMessageIds(['a', 'b'], ['a']), []);
});
