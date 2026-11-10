import assert from 'node:assert/strict';
import { test } from 'node:test';
import { parseChatBrowserConnection, parseChatToolCatalog } from './chatTools.ts';

test('tool inspector uses catalog identities without guessing availability from names', () => {
    assert.deepEqual(parseChatToolCatalog({ tools: [{ name: 'bash' }, { name: 'bash' }, { name: 'read_file' }] }), {
        names: ['bash', 'read_file'],
    });
    assert.throws(() => parseChatToolCatalog({ tools: [{ description: 'shell' }] }));
});

test('browser connection comes from native host status', () => {
    assert.deepEqual(parseChatBrowserConnection({ connected: true, hosts: [{ id: 'a' }] }), { connected: true, hostCount: 1 });
    assert.deepEqual(parseChatBrowserConnection({ connected: false, hosts: [] }), { connected: false, hostCount: 0 });
    assert.throws(() => parseChatBrowserConnection({ connected: true }));
});
