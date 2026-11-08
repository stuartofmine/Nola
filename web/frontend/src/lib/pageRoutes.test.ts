import { test } from 'node:test';
import assert from 'node:assert/strict';
import { pageLoader } from './pageRoutes.ts';

test('lazy page routing accepts only explicit migrated pages, never arbitrary module paths', () => {
    for (const path of ['/models', '/model-test', '/login', '/capabilities', '/results', '/dashboard', '/data-sources', '/settings', '/scheduled-actions', '/workspace-review', '/today', '/conversations', '/chat']) {
        assert.equal(typeof pageLoader(path), 'function');
    }
    for (const path of ['/chat-next', '/unknown', '__proto__', 'constructor', '/models/../../secrets', 'https://evil.invalid/Models.svelte']) {
        assert.equal(pageLoader(path), undefined);
    }
});
