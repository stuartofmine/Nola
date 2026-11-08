import { t, type Locale } from './i18n.ts';
import { resultTime } from './results.ts';

export type DashboardTask = {
    sourceId: string;
    name: string;
    format: string;
    enabled: boolean;
    scheduleFrequency: string;
    scheduledTimeISO?: string | null;
    scheduledWeekday?: number | null;
    nextScheduledAt?: string | null;
    latestResult?: { resultId: string; status: string; createdAt: string } | null;
};

export type DashboardSnapshot = {
    tasks: DashboardTask[];
    stats: { totalTasks: number; successCount: number; failedCount: number };
    backend: string;
    localFirst: boolean;
};

export function activeTask(task: DashboardTask) {
    return ['running', 'queued'].includes(task.latestResult?.status ?? '');
}

export function dashboardFocus(tasks: DashboardTask[], now = Date.now()): { kind: 'attention' | 'active' | 'next' | 'idle'; task?: DashboardTask } {
    const failed = tasks.find(task => ['failed', 'partial'].includes(task.latestResult?.status ?? ''));
    if (failed) return { kind: 'attention', task: failed };
    const active = tasks.find(activeTask);
    if (active) return { kind: 'active', task: active };
    const next = tasks.filter(task => task.enabled && task.nextScheduledAt && Date.parse(task.nextScheduledAt) >= now)
        .sort((a, b) => Date.parse(a.nextScheduledAt!) - Date.parse(b.nextScheduledAt!))[0];
    return next ? { kind: 'next', task: next } : { kind: 'idle' };
}

export function scheduleLabel(task: DashboardTask, locale: Locale = 'en') {
    const frequency = ['once', 'hourly', 'daily', 'weekly'].includes(task.scheduleFrequency)
        ? t(locale, `activity.frequency.${task.scheduleFrequency}`)
        : task.scheduleFrequency || t(locale, 'activity.frequency.unset');
    // Recurring wall-clock values have no date or zone; preserve them verbatim.
    const scheduled = task.scheduledTimeISO;
    const time = scheduled && /^\d{4}-\d{2}-\d{2}T/.test(scheduled) && Number.isFinite(Date.parse(scheduled))
        ? resultTime(scheduled, locale) : scheduled;
    return [frequency, time].filter(Boolean).join(' · ');
}

// A failed or malformed read is not an empty dashboard or a successful health check.
export function dashboardSnapshot(value: unknown, locale: Locale = 'en'): DashboardSnapshot {
    if (!value || typeof value !== 'object') throw new Error(t(locale, 'activity.invalid_dashboard'));
    const data = value as DashboardSnapshot;
    if (!Array.isArray(data.tasks) || !data.stats || typeof data.backend !== 'string' || typeof data.localFirst !== 'boolean'
        || ![data.stats.totalTasks, data.stats.successCount, data.stats.failedCount].every(count => Number.isSafeInteger(count) && count >= 0)
        || !data.tasks.every(task => task && typeof task.sourceId === 'string' && typeof task.name === 'string'
            && typeof task.enabled === 'boolean' && typeof task.scheduleFrequency === 'string'
            && (!task.latestResult || (typeof task.latestResult.resultId === 'string' && typeof task.latestResult.status === 'string' && typeof task.latestResult.createdAt === 'string')))) {
        throw new Error(t(locale, 'activity.invalid_dashboard'));
    }
    return data;
}
