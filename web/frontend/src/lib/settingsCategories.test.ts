import test from 'node:test';
import assert from 'node:assert/strict';
import { settingsCategories, resolveSettingsCategory } from './settingsCategories.ts';

test('settings destinations preserve existing notification links after consolidation', () => {
    for (const category of settingsCategories) assert.equal(resolveSettingsCategory(category), category);
    assert.equal(resolveSettingsCategory('notifications'), 'general');
    assert.equal(resolveSettingsCategory(null), null);
    assert.equal(resolveSettingsCategory('unknown'), null);
});
