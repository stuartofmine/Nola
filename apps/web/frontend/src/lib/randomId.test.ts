import {test} from 'node:test';
import assert from 'node:assert/strict';
import {randomId} from './randomId.ts';

test('resource IDs work on LAN HTTP without secure-context-only randomUUID', () => {
    const random = {getRandomValues: crypto.getRandomValues.bind(crypto)};
    const ids = Array.from({length: 30}, () => randomId(random));
    assert.equal(new Set(ids).size, ids.length);
    for (const id of ids) assert.match(id, /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
});

test('missing CSPRNG is an error rather than a weak identifier fallback', () => {
    assert.throws(() => randomId({getRandomValues() { throw new Error('CSPRNG unavailable'); }}));
});
