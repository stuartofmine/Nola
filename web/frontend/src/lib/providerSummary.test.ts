import test from 'node:test';
import assert from 'node:assert/strict';
import { providerSummary } from './providerSummary.ts';
import { supportedLocales, t } from './i18n.ts';

const profile = { id: 'a/b', displayName: 'Model', selected: true, endpoint: { id: 'endpoint', baseURL: 'private' }, apiKey: 'private' };
test('provider summary counts unique configured endpoints without retaining private catalog fields', () => {
    assert.deepEqual(providerSummary({ profiles: [profile, { ...profile, id: 'disabled', selected: false, enabled: false }] }), { endpoints: 1, models: 2, selected: { id: 'a/b', name: 'Model' } });
    assert.deepEqual(providerSummary({ profiles: [] }), { endpoints: 0, models: 0, selected: null });
    assert.ok(!JSON.stringify(providerSummary({ profiles: [profile] })).includes('private'));
});
test('malformed or ambiguous catalogs never become a zero or guessed selection', () => {
    for (const value of [null, {}, { profiles: null }, { profiles: [profile, profile] }, { profiles: [profile, { ...profile, id: 'b' }] }, ...[{ selected: 'true' }, { endpoint: {} }, { displayName: '' }, { id: null }].map(patch => ({ profiles: [{ ...profile, ...patch }] }))]) {
        assert.throws(() => providerSummary(value));
    }
});
test('provider and data settings labels are translated in all supported languages', () => {
    for (const language of supportedLocales) for (const key of ['category.providers', 'category.data', 'providers.detail', 'providers.loading', 'providers.failed', 'providers.endpoints', 'providers.models', 'providers.selected', 'providers.none', 'providers.manage', 'data.detail', 'data.open', 'data.managementUnavailable']) {
        assert.notEqual(t(language, `settings.${key}`), `settings.${key}`);
    }
});
