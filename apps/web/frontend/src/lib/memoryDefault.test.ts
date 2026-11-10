import test from 'node:test';
import assert from 'node:assert/strict';
import { memoryDefault, memoryDefaultReceipt } from './memoryDefault.ts';
import { supportedLocales, t } from './i18n.ts';
test('memory defaults distinguish stored intent from host authority and require exact receipts', () => {
    assert.deepEqual(memoryDefault({ enabled: true, hostAllowed: false, token: 'private' }), { enabled: true, hostAllowed: false });
    assert.deepEqual(memoryDefaultReceipt({ enabled: false, hostAllowed: false }, false), { enabled: false, hostAllowed: false });
    assert.throws(() => memoryDefaultReceipt({ enabled: true, hostAllowed: true }, false));
    for (const value of [null, {}, { enabled: true }, { enabled: 'false', hostAllowed: true }, { enabled: false, hostAllowed: 'true' }]) assert.throws(() => memoryDefault(value));
});
test('memory default copy covers every locale', () => {
    for (const locale of supportedLocales) for (const key of ['title', 'enabled', 'scope', 'denied']) assert.notEqual(t(locale, `settings.memory.${key}`), `settings.memory.${key}`);
});
