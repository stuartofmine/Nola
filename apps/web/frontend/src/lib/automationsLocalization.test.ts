import { test } from 'node:test';
import assert from 'node:assert/strict';
import { automationMessages } from './locales/automations.ts';

test('automation catalogs cover all seven locales and preserve confirmation placeholders', () => {
    const locales = ['zh-Hans', 'en', 'de', 'es', 'fr', 'ja', 'ru'] as const;
    const expected = Object.keys(automationMessages.en).sort();
    for (const locale of locales) {
        const messages = automationMessages[locale];
        assert.deepEqual(Object.keys(messages).sort(), expected);
        for (const key of expected) {
            assert.ok(messages[key].trim(), `${locale}: ${key}`);
            assert.deepEqual(messages[key].match(/\{[A-Za-z][A-Za-z0-9_]*\}/g) ?? [], automationMessages.en[key].match(/\{[A-Za-z][A-Za-z0-9_]*\}/g) ?? [], `${locale}: ${key}`);
        }
        for (const key of ['runConfirmBody', 'approveBody', 'deleteBody']) assert.ok(messages[`automation.${key}`].includes('{name}'));
    }
});

test('automation copy distinguishes saved schedules, manual completion, and source-task scope', () => {
    assert.match(automationMessages.en['automation.hostScheduleHint'], /manually only/);
    assert.match(automationMessages.en['automation.hostScheduleHint'], /do not trigger execution/);
    assert.match(automationMessages.en['automation.runSucceeded'], /completed/);
    assert.match(automationMessages.en['automation.runFailed'], /failed/);
    assert.equal(automationMessages.en['automation.prompt'], 'Agent prompt');
    assert.match(automationMessages.en['automation.scriptUnavailable'], /preview is not sufficient/);
    for (const format of ['agent', 'html', 'search', 'rss', 'json', 'icalendar', 'file', 'folder', 'script', 'reminder']) {
        assert.ok(automationMessages.en[`automation.format.${format}`]);
    }
});
