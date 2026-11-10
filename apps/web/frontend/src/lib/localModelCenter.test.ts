import { test } from 'node:test';
import assert from 'node:assert/strict';
import { localModelCenterMessages } from './locales/localModelCenter.ts';
import { supportedLocales, t } from './i18n.ts';

test('local model recommendation, runtime and storage labels cover every UI locale', () => {
    const keys = Object.keys(localModelCenterMessages.en);
    for (const language of supportedLocales) {
        assert.deepEqual(Object.keys(localModelCenterMessages[language]).sort(), [...keys].sort());
        for (const key of keys) {
            assert.notEqual(t(language, key), key);
            assert.deepEqual([...t(language, key).matchAll(/\{\w+\}/g)].map(match => match[0]).sort(),
                [...localModelCenterMessages.en[key].matchAll(/\{\w+\}/g)].map(match => match[0]).sort());
        }
    }
});
