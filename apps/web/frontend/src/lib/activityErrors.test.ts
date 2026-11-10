import { test } from 'node:test';
import assert from 'node:assert/strict';
import { ApiError } from './api.ts';
import { activityErrorKey } from './activityErrors.ts';
import { supportedLocales, t } from './i18n.ts';

test('activity errors are safe keys that retranslate without exposing response bodies', () => {
    for (const error of [new ApiError(500, 'private_token_and_path'), new Error('private_token_and_path'), null]) {
        const key = activityErrorKey(error);
        assert.equal(key, 'activity.errorLoad');
        for (const language of supportedLocales) {
            assert.notEqual(t(language, key), key);
            assert.ok(!t(language, key).includes('private_token_and_path'));
        }
    }
    for (const status of [401, 403]) assert.equal(activityErrorKey(new ApiError(status, 'secret')), 'activity.errorAccess');
    assert.notEqual(t('en', 'activity.errorAccess'), t('zh-Hans', 'activity.errorAccess'));
});
