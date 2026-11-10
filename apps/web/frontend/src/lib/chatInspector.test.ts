import test from 'node:test';
import assert from 'node:assert/strict';
import { supportedLocales, t } from './i18n.ts';

test('all inspector actions and empty plan states have seven-language labels', () => {
    for (const language of supportedLocales) {
        for (const action of ['progress', 'resources', 'settings', 'close', 'requirements', 'noPlan']) {
            const key = `chatInspector.${action}`;
            assert.notEqual(t(language, key), key);
            assert.ok(t(language, key).trim());
        }
    }
});
