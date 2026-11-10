import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { supportedLocales, t, messageCatalogs } from './i18n.ts';
import { resultTime } from './results.ts';
import { modelDisplayMessages } from './locales/modelDisplay.ts';

test('model display labels and every supported effort are registered in all seven languages', () => {
    for (const language of supportedLocales) {
        for (const [key, value] of Object.entries(modelDisplayMessages[language])) {
            assert.ok(value.trim());
            assert.equal(t(language, key), value);
            assert.equal(messageCatalogs.filter(catalog => Object.hasOwn(catalog[language], key)).length, 1, key);
        }
        for (const effort of ['none', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max']) {
            assert.notEqual(t(language, `modelDisplay.effort.${effort}`), `modelDisplay.effort.${effort}`);
        }
    }
    assert.equal(t('zh-Hans', 'modelDisplay.effort.high'), '高');
    assert.equal(t('ja', 'modelDisplay.localTimezone'), '現地時間');
});

test('conversation timestamp formatter follows current locale and safely hides malformed dates', () => {
    const value = '2026-09-23T15:04:05Z';
    for (const language of supportedLocales) {
        assert.equal(resultTime(value, language), new Date(value).toLocaleString(language));
        for (const invalid of [null, undefined, '', 'not a timestamp', '<script>bad</script>']) assert.equal(resultTime(invalid, language), '—');
    }
    assert.notEqual(resultTime(value, 'en'), resultTime(value, 'de'));
});

test('component bindings localize labels without changing model wire values or timezone identity', () => {
    const remote = readFileSync(new URL('../pages/RemoteProfiles.svelte', import.meta.url), 'utf8');
    assert.match(remote, /value=\{effort\} bind:group=\{thinking.efforts\}/);
    assert.match(remote, /protocolEfforts.includes\(effort\) \? thinkingTitle\(effort\)/);
    assert.match(remote, /<option value=\{option\}>\{thinkingTitle\(option\)\}/);
    for (const page of ['ModelCenter', 'RemoteProfiles']) {
        const source = readFileSync(new URL(`../pages/${page}.svelte`, import.meta.url), 'utf8');
        assert.ok(!source.includes("label: 'Top P'"));
        assert.ok(source.includes("label: 'modelDisplay.topP'"));
    }
    const conversations = readFileSync(new URL('../components/ChatConversationManager.svelte', import.meta.url), 'utf8');
    assert.ok(conversations.includes('conversationTime(conversation.updatedAt)'));
    assert.match(conversations, /new Intl\.DateTimeFormat\(\$locale,/);
    const automations = readFileSync(new URL('../pages/Automations.svelte', import.meta.url), 'utf8');
    assert.ok(automations.includes("action.timezone === 'local' ? t($locale, 'modelDisplay.localTimezone') : action.timezone"));
});
