import { test } from 'node:test';
import assert from 'node:assert/strict';
import { dashboardFocus, dashboardSnapshot, scheduleLabel, activeTask } from './dashboard.ts';
import type { DashboardTask } from './dashboard.ts';
import { supportedLocales, t } from './i18n.ts';

function task(sourceId: string, changes: Partial<DashboardTask> = {}): DashboardTask {
    return { sourceId, name: sourceId, format: 'script', enabled: true, scheduleFrequency: 'daily', ...changes };
}

test('schedule labels localize known frequencies and dates without rewriting custom schedules', () => {
    const timestamp = '2026-09-22T13:45:12Z';
    for (const locale of supportedLocales) {
        for (const frequency of ['once', 'hourly', 'daily', 'weekly']) {
            assert.equal(scheduleLabel(task('one', { scheduleFrequency: frequency, scheduledTimeISO: '10:30' }), locale), `${t(locale, `activity.frequency.${frequency}`)} · 10:30`);
        }
        assert.equal(scheduleLabel(task('one', { scheduledTimeISO: timestamp }), locale), `${t(locale, 'activity.frequency.daily')} · ${new Date(timestamp).toLocaleString(locale)}`);
        assert.equal(scheduleLabel(task('one', { scheduleFrequency: 'custom', scheduledTimeISO: 'opaque' }), locale), 'custom · opaque');
        assert.equal(scheduleLabel(task('one', { scheduleFrequency: '' }), locale), t(locale, 'activity.frequency.unset'));
        assert.throws(() => dashboardSnapshot(null, locale), { message: t(locale, 'activity.invalid_dashboard') });
    }
});

test('overview focuses failed/partial outcomes before active tasks without mutating server order', () => {
    const tasks = [task('running', { latestResult: { resultId: 'r', status: 'running', createdAt: '2026-09-22' } }), task('partial', { latestResult: { resultId: 'p', status: 'partial', createdAt: '2026-09-22' } })];
    assert.equal(dashboardFocus(tasks).task?.sourceId, 'partial');
    assert.equal(dashboardFocus(tasks.slice(0, 1)).kind, 'active');
    assert.equal(activeTask(tasks[0]), true);
    assert.deepEqual(tasks.map(task => task.sourceId), ['running', 'partial']);
});

test('next schedule ignores disabled, invalid and past dates rather than inventing a healthy task', () => {
    const tasks = [
        task('disabled', { enabled: false, nextScheduledAt: '2026-09-22T12:00:00Z' }),
        task('invalid', { nextScheduledAt: 'not-a-date' }),
        task('past', { nextScheduledAt: '2026-09-20T12:00:00Z' }),
        task('later', { nextScheduledAt: '2026-09-24T12:00:00Z' }),
        task('next', { nextScheduledAt: '2026-09-23T12:00:00Z' }),
    ];
    assert.equal(dashboardFocus(tasks, Date.parse('2026-09-22T00:00:00Z')).task?.sourceId, 'next');
    assert.deepEqual(dashboardFocus(tasks.slice(0, 3), Date.parse('2026-09-22T00:00:00Z')), { kind: 'idle' });
    assert.deepEqual(dashboardFocus([]), { kind: 'idle' });
    assert.equal(tasks[3].sourceId, 'later');
});

test('malformed stats cannot turn a failed read into a zero or healthy dashboard', () => {
    const value = { tasks: [task('one')], stats: { totalTasks: 1, successCount: 0, failedCount: 0 }, backend: 'cli-rust', localFirst: true };
    assert.equal(dashboardSnapshot(value), value);
    for (const invalid of [null, {}, { ...value, tasks: [null] }, { ...value, stats: { ...value.stats, successCount: -1 } }, { ...value, stats: {} }]) {
        assert.throws(() => dashboardSnapshot(invalid));
    }
    assert.equal(scheduleLabel(task('one', { scheduledTimeISO: '10:30' }), 'zh-Hans'), '每天 · 10:30');
    assert.equal(scheduleLabel(task('one', { scheduleFrequency: 'custom' })), 'custom');
});
