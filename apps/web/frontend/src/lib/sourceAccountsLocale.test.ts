import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { supportedLocales, t } from './i18n.ts';
import { sourceAccountMessages, sourceAccountActionKey, sourceAccountStateKey } from './locales/sourceAccounts.ts';

test('source-account catalog covers seven languages with matching literal placeholders', () => {
    const keys = Object.keys(sourceAccountMessages.en).sort();
    for (const language of supportedLocales) {
        assert.deepEqual(Object.keys(sourceAccountMessages[language]).sort(), keys);
        for (const key of keys) {
            const value = sourceAccountMessages[language][key];
            assert.ok(value?.trim(), `${language}:${key}`);
            assert.notEqual(t(language, key), key);
            assert.deepEqual(value.match(/\{\w+\}/g)?.sort() ?? [], sourceAccountMessages.en[key].match(/\{\w+\}/g)?.sort() ?? []);
        }
    }
});

test('source status and action wire values are localized without treating authentication as inference', () => {
    for (const language of supportedLocales) {
        for (const state of ['starting', 'pending', 'authenticated_source_only', 'authenticated', 'cancelled', 'failed',
            'expired', 'revoked', 'unauthenticated', 'denied', 'unconfirmed', 'unknown', 'login_required',
            'present_but_unsupported', 'invalid', 'unavailable', 'disconnected', 'stopped_locally', 'source_callback_completed']) {
            assert.notEqual(t(language, sourceAccountStateKey(state)), state);
        }
        assert.equal(sourceAccountStateKey('<untrusted>'), 'sourceAccounts.state.unknown');
        assert.equal(sourceAccountActionKey('unrecognized'), 'sourceAccounts.readAction');
        assert.equal(sourceAccountActionKey('login_in_codex_then_refresh'), 'sourceAccounts.action.login_in_codex_then_refresh');
        assert.ok(t(language, 'sourceAccounts.disabled'));
    }
    assert.equal(t('en', 'sourceAccounts.userCode', {code: '<tag>{other}</tag>'}), 'User code: <tag>{other}</tag>');
});

test('source-account components use localized status and retain owner/cancellation explanations', () => {
    for (const file of ['SourceAccounts', 'CodexSourceLogin', 'KimiSourceLogin', 'OpenCodeSourceLogin']) {
        const source = readFileSync(new URL(`../components/${file}.svelte`, import.meta.url), 'utf8');
        assert.ok(source.includes('sourceAccountStateKey'));
        assert.ok(source.includes('locale, t'));
        assert.ok(!source.includes('{snapshot.state}'));
        assert.ok(!source.includes('{status.login_state}'));
    }
    const codex = readFileSync(new URL('../components/CodexSourceLogin.svelte', import.meta.url), 'utf8');
    const kimi = readFileSync(new URL('../components/KimiSourceLogin.svelte', import.meta.url), 'utf8');
    assert.ok(codex.includes('sourceAccounts.codexCancelLimits'));
    assert.ok(kimi.includes('sourceAccounts.cancelConsent'));
    assert.ok(kimi.includes('sourceAccounts.stopObservation'));
});
