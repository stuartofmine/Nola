import test from 'node:test';
import assert from 'node:assert/strict';
import { retrievalDefault, retrievalDefaultReceipt } from './retrievalDefault.ts';
import { supportedLocales, t } from './i18n.ts';
test('retrieval default requires a canonical boolean and exact saved value', () => {
    assert.deepEqual(retrievalDefault({ enabled: true, token: 'private' }), { enabled: true });
    assert.deepEqual(retrievalDefaultReceipt({ enabled: false }, false), { enabled: false });
    assert.throws(() => retrievalDefaultReceipt({ enabled: true }, false));
    for (const value of [null, {}, { enabled: 'false' }, { enabled: 1 }, { enabled: null }]) assert.throws(() => retrievalDefault(value));
    for (const locale of supportedLocales) for (const key of ['enabled', 'scope']) assert.notEqual(t(locale, `settings.retrieval.${key}`), `settings.retrieval.${key}`);
});
