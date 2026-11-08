import { test } from 'node:test';
import assert from 'node:assert/strict';
import { activeResult, filterResults, resultLink, resultTime, resultStatus, resultFormat, sourceResults } from './results.ts';
import { supportedLocales, t } from './i18n.ts';
import type { ResultSummary } from './results.ts';
import { executionConversationLink } from './results.ts';

test('execution links preserve exact conversation and Run identities', () => {
    assert.equal(executionConversationLink('会话 /?#', 'run /?'), '/chat?conversation=%E4%BC%9A%E8%AF%9D%20%2F%3F%23&run=run%20%2F%3F');
    assert.equal(executionConversationLink('conversation', undefined), undefined);
    assert.equal(executionConversationLink('', 'run'), undefined);
    assert.equal(executionConversationLink(null, null), undefined, 'A launch failure has an occurrence but no admitted Run');
});

test('task history scopes active and completed results by exact opaque source identity', () => {
    const rows = ['task', 'task-2', '任务 /?x=1', 'task'].map((sourceId, index) => ({ sourceId, resultId: String(index) } as ResultSummary));
    assert.deepEqual(sourceResults(rows, 'task').map(row => row.resultId), ['0', '3']);
    assert.deepEqual(sourceResults(rows, '任务 /?x=1'), [rows[2]]);
    assert.deepEqual(sourceResults(rows, 'missing'), []);
    assert.deepEqual(sourceResults(rows, ''), []);
    assert.equal(sourceResults(rows, null), rows);
});

test('missing timestamps remain unavailable instead of becoming the Unix epoch', () => {
    for (const value of [null, undefined, '', 'invalid']) assert.equal(resultTime(value), '—');
});

test('result presentation follows every selected locale and preserves unknown protocol values', () => {
    const timestamp = '2026-09-22T13:45:12Z';
    for (const locale of supportedLocales) {
        assert.equal(resultTime(timestamp, locale), new Date(timestamp).toLocaleString(locale));
        for (const invalid of [null, undefined, '', 'invalid']) assert.equal(resultTime(invalid, locale), '—');
        for (const status of ['answered', 'action_completed', 'delegation_accepted', 'incomplete', 'blocked', 'cancelled', 'failed', 'running', 'waiting_input', 'queued']) {
            assert.equal(resultStatus(status, locale), t(locale, `activity.status.${status}`));
        }
        assert.equal(resultStatus('future_status', locale), 'future_status');
        assert.equal(resultFormat('custom_format', locale), 'custom_format');
        assert.equal(resultFormat('rss', locale), 'RSS');
        assert.equal(resultFormat('json', locale), 'JSON');
    }
    assert.notEqual(resultTime(timestamp, 'en'), resultTime(timestamp, 'de'));
});

test('result search uses the displayed localized format without translating backend content', () => {
    const result = { resultId: 'opaque', status: 'answered', sourceName: 'Original 中文', sourceId: 'task', sourceUrl: '', previewText: 'Résumé', sourceFormat: 'file' } as ResultSummary;
    const original = { ...result };
    for (const locale of supportedLocales) {
        assert.deepEqual(filterResults([result], resultFormat('file', locale), 'all', locale), [result]);
        assert.deepEqual(filterResults([result], '中文', 'all', locale), [result]);
        assert.deepEqual(filterResults([result], 'Résumé', 'all', locale), [result]);
    }
    assert.deepEqual(result, original);
});

test('result status and search filtering retain server order and unknown states', () => {
    const results = ['answered', 'action_completed', 'delegation_accepted', 'incomplete', 'blocked', 'cancelled', 'failed', 'running', 'waiting_input', 'queued', 'future'].map((status, index) => ({
        resultId: String(index), status, sourceName: '分析 Report', sourceId: 'task', sourceUrl: '', previewText: 'Résumé', sourceFormat: 'file',
    } as ResultSummary));
    assert.deepEqual(results.filter(activeResult).map(result => result.status), ['running', 'waiting_input', 'queued']);
    assert.deepEqual(filterResults(results, 'REPORT', 'attention').map(result => result.status), ['incomplete', 'blocked', 'failed']);
    assert.equal(filterResults(results, '文件', 'all', 'zh-Hans').length, 11);
    assert.equal(filterResults(results, 'Résumé', 'success').length, 2);
    assert.equal(filterResults(results, '', 'all').at(-1)?.status, 'future');
    assert.equal(results[0].resultId, '0');
});

test('result links cannot execute scripts expose embedded credentials or open local paths', () => {
    for (const value of ['javascript:alert(1)', 'data:text/html,test', 'file:///tmp/result', '/tmp/report', '//evil.invalid', 'https://user:password@fixture.invalid', null, {}]) {
        assert.equal(resultLink(value), undefined);
    }
    assert.equal(resultLink('https://fixture.invalid/report?q=1#part'), 'https://fixture.invalid/report?q=1#part');
    assert.equal(resultTime('not a date'), '—');
});
