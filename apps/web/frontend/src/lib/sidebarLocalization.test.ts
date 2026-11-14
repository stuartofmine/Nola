import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'vite';
import { ApiError } from './api.ts';
import { supportedLocales, t } from './i18n.ts';

import { conversationErrorKey } from './conversationErrors.ts';

test('production list, resources and language controls render their current localized contracts', async () => {
    const server = await createServer({server: {middlewareMode: true, ws: false}, appType: 'custom'});
    try {
        const {default: ListNavigation} = await server.ssrLoadModule('/src/components/WorkspaceListNavigation.svelte');
        const {default: ResourceNavigation} = await server.ssrLoadModule('/src/components/WorkspaceResourceNavigation.svelte');
        const {default: LanguagePreference} = await server.ssrLoadModule('/src/components/LanguagePreference.svelte');
        const {render} = await server.ssrLoadModule('svelte/server');
        const {locale, localePreference, supportedLocales: languages, t} = await server.ssrLoadModule('/src/lib/i18n.ts');
        for (const language of languages) {
            localePreference.set('system'); locale.set(language);
            for (const selected of ['conversations', 'resources']) {
                const list = render(ListNavigation, {props: {category: selected, onchange: () => {}}}).body;
                assert.equal((list.match(/aria-pressed="true"/g) ?? []).length, 1);
                assert.equal((list.match(/<button /g) ?? []).length, 2);
                for (const category of ['conversations', 'resources']) assert.ok(list.includes(t(language, `chatInspector.list.${category}`)));
            }
            const resources = render(ResourceNavigation, {props: {selectedPath: '/data-sources?tab=memory', onopen: () => {}}}).body;
            for (const key of ['shell.nav.data', 'shell.nav.models', 'shell.nav.capabilities']) assert.ok(resources.includes(t(language, key).replaceAll('&', '&amp;')));
            assert.equal((resources.match(/aria-current="page"/g) ?? []).length, 1);
            assert.match(resources, /href="\/data-sources" aria-current="page"/);
            assert.equal((resources.match(/<a /g) ?? []).length, 3);
            assert.ok(!resources.includes('href="/activity"'));
            assert.doesNotMatch(resources, /conversation-manager|new-conversation/);
            const preferences = render(LanguagePreference).body;
            assert.ok(preferences.includes(t(language, 'shell.language')));
            assert.ok(preferences.includes(t(language, 'shell.language.system')));
            for (const choice of languages) assert.ok(preferences.includes(`value="${choice}"`));
        }
    } finally { await server.close(); }
});

test('production conversation manager stores safe error keys that retranslate without retaining server response bodies', () => {
    const secret = 'SENSITIVE_PROVIDER_BODY';
    const cases = [
        [new ApiError(500, secret), 'load', 'shell.conversations.load_error'],
        [new Error(secret), 'create', 'shell.conversations.create_error'],
        [new ApiError(401, secret), 'load', 'shell.conversations.auth_error'],
        [new ApiError(403, secret), 'create', 'shell.conversations.auth_error'],
    ] as const;
    for (const [error, operation, expected] of cases) {
        const storedKey = conversationErrorKey(error, operation);
        assert.equal(storedKey, expected);
        for (const language of supportedLocales) {
            assert.notEqual(t(language, storedKey), storedKey);
            assert.ok(!t(language, storedKey).includes(secret));
        }
        assert.notEqual(t('en', storedKey), t('zh-Hans', storedKey));
    }
});

test('WeChat provider display labels exist in all locales without changing transport identifiers', () => {
    for (const kind of ['wechat_official', 'we_com']) {
        for (const language of supportedLocales) assert.notEqual(t(language, `messaging.provider.${kind}`), `messaging.provider.${kind}`);
    }
    assert.equal(t('zh-Hans', 'messaging.provider.wechat_official'), '微信公众号');
    assert.equal(t('zh-Hans', 'messaging.provider.we_com'), '企业微信');
});
